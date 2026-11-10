//! Reads story packs into one `Pack` and checks it. The core pack is the game's own content; mod packs add to it.
//! Every problem found is collected, so one run reports them all.
//!
//! A pack is a folder with a `pack.json` (`lr.pack/1`, the same manifest as last_route and the Godot mod kit):
//!
//! ```text
//! pack.json          {"schema": "lr.pack/1", "id", "name"?, "version"?, "author"?, "description"?,
//!                     "overrides"?: ["scenes/<id>", "items/<id>", "maps/<id>", "props/<id>", "skins/<id>", "start"],
//!                     "start"?: {"map", "spawn"}, "start_scene"?,
//!                     "story"?: {"title"?, "era", "order"?, "player"?: {"name"?, "role"?, "speed"?, "body"?,
//!                                "start_items"?: [item id], "start_flags"?: [flag]}}}
//!
//! A pack with a "story" block is a story: its id is the story id, and its own "start" is where that story begins
//! (no override needed). The era is one of sample, past, lastroute, simulora, future. The core pack is the sample story
//! whether or not it has a block. Only the core pack sets the default start (the sample story starts there). A mod
//! pack never moves it: a mod that lists "start" in its overrides is refused with an error, and so is a mod without a
//! story block that has a "start" (a story mod just needs its own "start").
//!
//! Optional fields are strict: a field of the wrong type (an op's "value" or "once" that is not a boolean, a "height"
//! or "speed" that is not a number, a "player" that is not an object, an item "name" that is not text, ...) is an error
//! naming the field, never a silent default. A container of the wrong shape ("lines", "choices", "scenes", "items",
//! "legend") is an error too, and so is a scene that is a bare string. File extensions match in any case (.JSON, .PNG,
//! .BBMODEL). PNG sizes are read from the file's own header before the picture is decoded.
//! items/icons/<n>.png  an item's picture, at most 64x64; an item names it with "icon": "<n>.png" and may set
//!                      "stack": 1..=99 (default 1, or 9 when it has a "use")
//! items/*.json       {"schema": "lr.story.items/1", "items": {id: {"name", "description", "use"?: {"heal": N} or
//!                     {"damage": N}, "consumable"?}}}
//! battles/*.json     {"schema": "lr.story.battles/1", "battles": {id: {"name"?, "boss"?, "max_hp"?, "defense"?,
//!                     "move_speed"?, "spawn": map spawn name, "skin"?: skin name,
//!                     "attacks": [{"name", "kind"?: melee|charge|shot|slam, "damage"?, "range"?, "windup"?,
//!                     "cooldown"?, "weight"?, "min_phase"?, "radius"?, "speed"?, "text"?}],
//!                     "phases"?: [66, 33] (1..99, high to low), "intro"?, "win"?: scene, "lose"?: scene}}}
//! quests/*.json      {"schema": "lr.story.quests/1", "quests": {id: {"title"?, "description"?, "show_if"?: cond,
//!                     "steps": [{"text", "done_if": cond}]}}}
//!                    Ops: {"op": "battle", "battle": id}, {"op": "heal", "amount": N}. There is no XP and no levels:
//!                    an "xp" op or battle field is a load error.
//!                    Story player: "stats": {"max_hp"?, "attack"?, "defense"?, "speed"?}.
//! scenes/*.json      {"schema": "lr.story.scenes/1", "scenes": {id: scene}}
//! maps/*.json        {"schema": "lr.story.map/1", "id"?, "name", "art", "mask"?, "legend": {hex: kind},
//!                     "outdoor"?: bool (no ceiling, a sky), "ceiling"?: {"height"?: 1-16 m, "color"?: hex},
//!                     "sky"?: {"top"?: hex, "bottom"?: hex} (outdoor only), "walls"?: lab|brick|concrete|plaster,
//!                     "floor"?: blocks|asphalt|paving|tiles}
//!                    Also on a map: "weather"?: {"kind": clear|rain|snow|fog|storm|leaves, "intensity"?: 0-1 (0.6),
//!                    "changes"?: [{"kind", "weight"}], "every_secs"?: seconds (90)} (outdoor maps only).
//!                    Op: {"op": "weather", "kind", "intensity"?}. A prop legend entry takes "use": {"action": sit|lean|
//!                    work|draw_water|read|sleep|warm_hands|look, "text"?, "scene"?, "seat_height"?: 0-2 m (0.45)};
//!                    an npc entry takes "routine": <routine id>.
//! routines/*.json    {"schema": "lr.story.routines/1", "routines": {id: {"loop"?: bool (true), "steps": [{"go"?: spawn
//!                    name or "prop:<x>,<y>", "do"?: idle|sit|lean|work|draw_water|read|sleep|warm_hands|look|wave|chat|
//!                    wander, "secs"?: above 0 (5), "with"?: npc name (chat), "radius"?: metres (wander, 3)}],
//!                    "shelter"?: spawn name (rain/snow), "flee"?: spawn name (fight)}}}. Every go/shelter/flee target
//!                    must exist on each map an NPC with the routine stands on; "with" must be an NPC on that map.
//!                    Ids follow the same prefix and "routines/<id>" override rules as scenes.
//! maps/<art>.png     the floor picture (any size, a whole multiple of the mask)
//! maps/<mask>.png    the mask: one pixel per tile, coloured by the legend ("mask" defaults to the art)
//! props/<id>.bbmodel a prop model (Blockbench); its atlas PNG sits beside it or is embedded. A map legend entry
//!                    {"kind": "prop", "prop": "<id>", "yaw"?: degrees} places it. The id is the file name.
//! skins/<name>.png   a mannequin atlas, 64x64 like the default one. An npc legend entry {"skin": "<name>"} paints its
//!                    figure with it (default: the default atlas). The name is the file name; a mod's names start
//!                    with "<pack id>_".
//! ```
//!
//! Ids a mod adds must start with `<pack id>_`, so they never collide. A mod may replace another pack's id only when
//! it lists `"<kind>/<id>"` in its overrides, and an earlier pack (the core, or a mod with a smaller id) must define
//! that id; an override of an id nobody defines earlier is an error, and does not exempt the id from the prefix rule.
//! The folder name must equal the pack id.
//!
//! Within one pack, a scene id may repeat only when the pack lists `scenes/<id>` in its overrides and the later copy is
//! in a scene file whose name contains `overrides` (those files are read last). That copy replaces the first. A repeat
//! in any other file is an error.

use crate::assets_vfs as vfs;
use crate::model::*;
use macroquad::prelude::{Image, ImageFormat};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_PACK: &str = "lr.pack/1";
pub const SCHEMA_ITEMS: &str = "lr.story.items/1";
pub const SCHEMA_SCENES: &str = "lr.story.scenes/1";
pub const SCHEMA_BATTLES: &str = "lr.story.battles/1";
pub const SCHEMA_QUESTS: &str = "lr.story.quests/1";
pub const SCHEMA_ROUTINES: &str = "lr.story.routines/1";
pub const SCHEMA_MAP: &str = "lr.story.map/1";
const DEFAULT_LOCKED: &str = "It won't open.";
/// A mask bigger than this is almost certainly a mistake (the grid is walked and drawn every frame).
const MAX_MASK_SIDE: usize = 512;
/// A floor picture bigger than this on a side is refused before it is decoded (it would be a texture of that size).
const MAX_ART_SIDE: usize = 8192;
/// The side of a skin (a mannequin atlas): the same size as the default atlas, so its UVs line up.
pub const SKIN_SIDE: u16 = 64;

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
    /// The pack's story block, when it has one (the core pack has none unless it lists one).
    story: Option<StoryBlock>,
    items: BTreeMap<String, Item>,
    scenes: BTreeMap<String, Scene>,
    battles: BTreeMap<String, Battle>,
    quests: BTreeMap<String, Quest>,
    routines: BTreeMap<String, Routine>,
    maps: BTreeMap<String, MapDef>,
    props: BTreeMap<String, PathBuf>,
    skins: BTreeMap<String, PathBuf>,
    /// Slots (`scenes/<id>`) the pack replaced with a later copy of its own (see the module doc).
    replaced: BTreeSet<String>,
}

impl Contribution {
    /// Whether this pack itself defines `id` of `kind` ("scenes", "items", ...).
    fn defines(&self, kind: &str, id: &str) -> bool {
        match kind {
            "items" => self.items.contains_key(id),
            "scenes" => self.scenes.contains_key(id),
            "battles" => self.battles.contains_key(id),
            "quests" => self.quests.contains_key(id),
            "routines" => self.routines.contains_key(id),
            "maps" => self.maps.contains_key(id),
            "props" => self.props.contains_key(id),
            "skins" => self.skins.contains_key(id),
            _ => false,
        }
    }
}

/// A pack's "story" block: how the story is titled and sorted, and who the player is.
#[derive(Clone, Debug)]
struct StoryBlock {
    title: String,
    era: Era,
    order: f64,
    player: Player,
    /// `"multiplayer": true`: rooms may play this story (the story server checks the same flag).
    multiplayer: bool,
}

impl StoryBlock {
    /// The sample story the core pack is when it has no block.
    fn sample(name: &str) -> StoryBlock {
        StoryBlock {
            title: name.to_string(),
            era: Era::Sample,
            order: 0.0,
            player: Player::default(),
            multiplayer: false,
        }
    }
}

/// The highest player speed a pack may ask for (a multiplier on walking and sprinting).
const MAX_SPEED: f64 = 10.0;

/// The pack folders to read as mods: subfolders of each root that hold a `pack.json`, then the named folders. The core
/// folder is skipped, and a folder reached twice is read once.
pub fn discover(cfg: &Config) -> Vec<PathBuf> {
    let core = canonical(&cfg.core);
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in &cfg.mod_roots {
        let Ok(read) = vfs::read_dir(root) else { continue };
        let mut subs: Vec<PathBuf> = read.into_iter().filter(|p| vfs::is_file(&p.join("pack.json"))).collect();
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

#[cfg(not(target_arch = "wasm32"))]
fn canonical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(target_arch = "wasm32")]
fn canonical(p: &Path) -> PathBuf {
    p.to_path_buf()
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
        battles: BTreeMap::new(),
        quests: BTreeMap::new(),
        routines: BTreeMap::new(),
        maps: BTreeMap::new(),
        props: BTreeMap::new(),
        skins: BTreeMap::new(),
        packs: Vec::new(),
        stories: Vec::new(),
    };
    let mut all = vec![core];
    all.extend(mods);
    // A pack id is its folder name, so one id in two folders means the same pack is in two places (for example a copy
    // in the program's mods folder and another beside the working folder). The merge would take both, so it is refused.
    for (i, c) in all.iter().enumerate() {
        if let Some(earlier) = all[..i].iter().find(|o| o.id == c.id) {
            errs.push(format!("pack id '{}' is in two folders: {} and {}", c.id, earlier.dir.display(), c.dir.display()));
        }
    }
    check_overrides(&all, &mut errs);
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
    put(&mut pack.battles, "battles", &c.battles, c, errs);
    put(&mut pack.quests, "quests", &c.quests, c, errs);
    put(&mut pack.routines, "routines", &c.routines, c, errs);
    put(&mut pack.maps, "maps", &c.maps, c, errs);
    put(&mut pack.props, "props", &c.props, c, errs);
    put(&mut pack.skins, "skins", &c.skins, c, errs);
    if let Some((map, spawn)) = &c.start {
        if c.core {
            pack.start_map = map.clone();
            pack.start_spawn = spawn.clone();
            if c.start_scene.is_some() {
                pack.start_scene = c.start_scene.clone();
            }
        } else if c.story.is_none() {
            errs.push(format!("pack '{}' sets \"start\"; only the core pack or a story pack (a pack with a \"story\" block) may", c.id));
        }
        // A story (its block, or the core pack's sample story) starts at its own "start".
        if let Some(block) = c.story.clone().or_else(|| c.core.then(|| StoryBlock::sample(&c.name))) {
            pack.stories.push(StoryInfo {
                id: c.id.clone(),
                title: block.title,
                era: block.era,
                order: block.order,
                start_map: map.clone(),
                start_spawn: spawn.clone(),
                start_scene: c.start_scene.clone(),
                player: block.player,
                multiplayer: block.multiplayer,
            });
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
}

/// Checks every override entry against the packs in merge order (core first, then mods by id): the id must be defined
/// by an earlier pack (or replaced by the pack's own later copy), and a mod's override does not exempt an id from the
/// prefix rule unless it really replaces an earlier pack's id. "start" is never a valid override for a mod.
fn check_overrides(all: &[Contribution], errs: &mut Vec<String>) {
    for (i, c) in all.iter().enumerate() {
        let prefix = format!("{}_", c.id);
        for o in &c.overrides {
            if o == "start" {
                if !c.core {
                    errs.push(format!(
                        "pack '{}' lists \"start\" in its overrides; a mod cannot move the start (a story pack begins at its own \"start\")",
                        c.id
                    ));
                }
                continue;
            }
            let Some((kind, id)) = o.split_once('/') else { continue };
            if !["scenes", "items", "maps", "props", "skins", "battles", "quests", "routines"].contains(&kind) || id.is_empty() {
                continue; // reported where the manifest is read
            }
            let earlier = all[..i].iter().any(|p| p.defines(kind, id));
            let own_replace = c.replaced.contains(o);
            if earlier {
                continue;
            }
            if let Some(later) = all[i + 1..].iter().find(|p| p.defines(kind, id)) {
                errs.push(format!(
                    "pack '{}' overrides \"{o}\", which is defined by a later pack '{}'; packs merge in id order, so the override must come from a pack after it",
                    c.id, later.id
                ));
            } else if c.defines(kind, id) && !own_replace {
                errs.push(format!("pack '{}' overrides \"{o}\", but no earlier pack defines it", c.id));
            } else if !c.defines(kind, id) {
                errs.push(format!("pack '{}' overrides \"{o}\", which no pack defines", c.id));
            }
            // The override named an id that is not an earlier pack's: it earns no exemption from the prefix rule.
            if !c.core && c.defines(kind, id) && !id.starts_with(&prefix) {
                errs.push(format!("{} id '{id}' must start with '{prefix}' (an override only exempts an earlier pack's id)", kind));
            }
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
    let overrides: Vec<String> = match str_list(&root, "overrides") {
        Ok(list) => list,
        Err(e) => {
            errs.push(format!("{ctx}: {e}"));
            Vec::new()
        }
    };
    for o in &overrides {
        let known = o == "start" || ["scenes/", "items/", "maps/", "props/", "skins/", "battles/", "quests/", "routines/"].iter().any(|k| o.starts_with(k) && o.len() > k.len());
        if !known {
            errs.push(format!(
                "{ctx}: override \"{o}\" must be \"scenes/<id>\", \"items/<id>\", \"maps/<id>\", \"props/<id>\", \"skins/<id>\", \"battles/<id>\", \"quests/<id>\", \"routines/<id>\" or \"start\""
            ));
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
    // A story block makes this pack a story. A block that does not parse is reported, and the sample story stands in.
    let story = match root.get("story") {
        None | Some(Value::Null) => None,
        Some(block) => match parse_story(block, &name) {
            Ok(story) => Some(story),
            Err(e) => {
                errs.push(format!("{ctx}: \"story\": {e}"));
                Some(StoryBlock::sample(&name))
            }
        },
    };
    if story.is_some() && !core && start.is_none() {
        errs.push(format!("{ctx}: a story pack needs \"start\": {{\"map\", \"spawn\"}}, the place its story begins"));
    }

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
                let entries = match container(&v, "items") {
                    Ok(e) => e,
                    Err(e) => {
                        errs.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                for (key, def) in entries {
                    if !check_id("items", &key, errs) {
                        continue;
                    }
                    match parse_item(def, &key) {
                        Ok(mut item) => match resolve_item_icon(&mut item, dir) {
                            Ok(()) => insert_unique(&mut items, key, item, "item", errs),
                            Err(e) => errs.push(format!("{ctx}: item '{key}': {e}")),
                        },
                        Err(e) => errs.push(format!("{ctx}: item '{key}': {e}")),
                    }
                }
            }
            Err(e) => errs.push(e),
        }
    }

    // Scene files named "*overrides*" are read last (json_files sorts by name, then the stable sort moves them back), so
    // a copy in one of them replaces the pack's own copy of a scene the pack lists in its overrides (see the module doc).
    let mut scene_files = json_files(&dir.join("scenes"));
    scene_files.sort_by_key(|f| is_override_file(f));
    let mut scenes = BTreeMap::new();
    let mut replaced = BTreeSet::new();
    for file in scene_files {
        let from_override_file = is_override_file(&file);
        match read_json(&file, SCHEMA_SCENES) {
            Ok(v) => {
                let entries = match container(&v, "scenes") {
                    Ok(e) => e,
                    Err(e) => {
                        errs.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                for (key, def) in entries {
                    if !check_id("scenes", &key, errs) {
                        continue;
                    }
                    match parse_scene(def) {
                        Ok(scene) => {
                            let replaces = from_override_file && scenes.contains_key(&key) && overrides.contains(&format!("scenes/{key}"));
                            if replaces {
                                replaced.insert(format!("scenes/{key}"));
                                scenes.insert(key, scene);
                            } else {
                                insert_unique(&mut scenes, key, scene, "scene", errs);
                            }
                        }
                        Err(e) => errs.push(format!("scene '{key}': {e}")),
                    }
                }
            }
            Err(e) => errs.push(e),
        }
    }

    let mut battles = BTreeMap::new();
    for file in json_files(&dir.join("battles")) {
        match read_json(&file, SCHEMA_BATTLES) {
            Ok(v) => {
                let entries = match container(&v, "battles") {
                    Ok(e) => e,
                    Err(e) => {
                        errs.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                for (key, def) in entries {
                    if !check_id("battles", &key, errs) {
                        continue;
                    }
                    match parse_battle(def, &key) {
                        Ok(b) => insert_unique(&mut battles, key, b, "battle", errs),
                        Err(e) => errs.push(format!("{ctx}: battle '{key}': {e}")),
                    }
                }
            }
            Err(e) => errs.push(e),
        }
    }

    let mut quests = BTreeMap::new();
    for file in json_files(&dir.join("quests")) {
        match read_json(&file, SCHEMA_QUESTS) {
            Ok(v) => {
                let entries = match container(&v, "quests") {
                    Ok(e) => e,
                    Err(e) => {
                        errs.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                for (key, def) in entries {
                    if !check_id("quests", &key, errs) {
                        continue;
                    }
                    match parse_quest(def, &key) {
                        Ok(q) => insert_unique(&mut quests, key, q, "quest", errs),
                        Err(e) => errs.push(format!("{ctx}: quest '{key}': {e}")),
                    }
                }
            }
            Err(e) => errs.push(e),
        }
    }

    let mut routines = BTreeMap::new();
    for file in json_files(&dir.join("routines")) {
        match read_json(&file, SCHEMA_ROUTINES) {
            Ok(v) => {
                let entries = match container(&v, "routines") {
                    Ok(e) => e,
                    Err(e) => {
                        errs.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                for (key, def) in entries {
                    if !check_id("routines", &key, errs) {
                        continue;
                    }
                    match parse_routine(def) {
                        Ok(r) => insert_unique(&mut routines, key, r, "routine", errs),
                        Err(e) => errs.push(format!("{ctx}: routine '{key}': {e}")),
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

    // A prop is named by its file (props/<id>.bbmodel). The model itself is read when the game starts, not here.
    let mut props = BTreeMap::new();
    for file in bbmodel_files(&dir.join("props")) {
        let Some(id) = file.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
        if check_id("props", &id, errs) {
            props.insert(id, file);
        }
    }

    // A skin is named by its file (skins/<name>.png), a mannequin atlas of SKIN_SIDE pixels square. It is decoded here, so
    // a broken or wrong-sized file is reported with the pack's other problems.
    let mut skins = BTreeMap::new();
    for file in files_with_extension(&dir.join("skins"), "png") {
        let Some(name) = file.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
        if !is_plain_name(&name) {
            errs.push(format!("{ctx}: skin file name '{name}' must be letters, digits, '_' or '-' only (it is the skin's name)"));
            continue;
        }
        if !check_id("skins", &name, errs) {
            continue;
        }
        match read_png_bytes(&file) {
            Ok(png) if png.w == SKIN_SIDE as u32 && png.h == SKIN_SIDE as u32 => match decode_png(&png, &file) {
                Ok(_) => {
                    skins.insert(name, file);
                }
                Err(e) => errs.push(format!("{ctx}: skin '{name}': {e}")),
            },
            Ok(png) => errs.push(format!(
                "{ctx}: skin '{name}' is {}x{}; a skin must be {SKIN_SIDE}x{SKIN_SIDE}",
                png.w, png.h
            )),
            Err(e) => errs.push(format!("{ctx}: skin '{name}': {e}")),
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
        story,
        items,
        scenes,
        battles,
        quests,
        routines,
        maps,
        props,
        skins,
        replaced,
    })
}

/// Whether `name` is a plain id: letters, digits, '_' and '-' only (the rule skin references already follow).
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The entries of the object under `key` (the "items", "scenes" containers). A missing or null one is empty; anything
/// but an object is an error.
fn container<'a>(v: &'a Value, key: &str) -> Result<Vec<(String, &'a Value)>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(o)) => Ok(o.iter().map(|(k, v)| (k.clone(), v)).collect()),
        Some(_) => Err(format!("\"{key}\" must be an object of id: definition")),
    }
}

/// One item: an object with an optional text "name" (default: the id) and "description".
fn parse_item(def: &Value, key: &str) -> Result<Item, String> {
    if !def.is_object() {
        return Err("an item must be an object with \"name\" and \"description\"".to_string());
    }
    let use_effect = match def.get("use") {
        None | Some(Value::Null) => None,
        Some(u) if u.is_object() => {
            let heal = opt_int(u, "heal")?;
            let damage = opt_int(u, "damage")?;
            match (heal, damage) {
                (Some(h), None) if h > 0 => Some(ItemUse::Heal(h)),
                (None, Some(d)) if d > 0 => Some(ItemUse::Damage(d)),
                (Some(_), None) | (None, Some(_)) => return Err("\"use\" amounts must be above 0".to_string()),
                _ => return Err("\"use\" needs exactly one of \"heal\" or \"damage\"".to_string()),
            }
        }
        Some(_) => return Err("\"use\" must be an object like {\"heal\": 10}".to_string()),
    };
    let default_stack = if use_effect.is_some() { 9 } else { 1 };
    let stack = match opt_int(def, "stack")? {
        None => default_stack,
        Some(n) if (1..=MAX_ITEM_STACK as i32).contains(&n) => n as u32,
        Some(_) => return Err(format!("\"stack\" must be between 1 and {MAX_ITEM_STACK}")),
    };
    let icon = match opt_str(def, "icon")? {
        None => None,
        Some(name) if is_icon_file_name(&name) => Some(PathBuf::from(name)),
        Some(name) => {
            return Err(format!(
                "\"icon\" '{name}' must be a plain PNG file name in items/icons/ (letters, digits, '_' or '-', then \".png\")"
            ))
        }
    };
    Ok(Item {
        name: opt_str(def, "name")?.unwrap_or_else(|| key.to_string()),
        description: opt_str(def, "description")?.unwrap_or_default(),
        consumable: opt_bool(def, "consumable", use_effect.is_some())?,
        use_effect,
        stack,
        icon,
    })
}

/// The most an item's "stack" may be.
const MAX_ITEM_STACK: u32 = 99;
/// The biggest an item icon may be, in pixels per side.
const ICON_MAX_SIDE: u32 = 64;

/// Whether `name` is `<plain name>.png`: no folders, no dots besides the extension.
fn is_icon_file_name(name: &str) -> bool {
    name.strip_suffix(".png").is_some_and(is_plain_name)
}

/// Checks an item's icon file (`<pack>/items/icons/<name>`): it must exist, be a PNG by its header, and be at most
/// ICON_MAX_SIDE square. On success the item's icon becomes the full path.
fn resolve_item_icon(item: &mut Item, dir: &Path) -> Result<(), String> {
    let Some(name) = item.icon.take() else { return Ok(()) };
    let path = dir.join("items").join("icons").join(&name);
    let png = read_png_bytes(&path).map_err(|e| format!("icon '{}': {e}", name.display()))?;
    if png.w == 0 || png.h == 0 || png.w > ICON_MAX_SIDE || png.h > ICON_MAX_SIDE {
        return Err(format!(
            "icon '{}' is {}x{}; an icon must be at most {ICON_MAX_SIDE}x{ICON_MAX_SIDE}",
            name.display(),
            png.w,
            png.h
        ));
    }
    item.icon = Some(path);
    Ok(())
}

/// An optional whole number field; anything else (text, a fraction) is an error naming the field.
fn opt_int(v: &Value, key: &str) -> Result<Option<i32>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("\"{key}\" must be a whole number")),
    }
}

/// A whole number field with a default and a minimum.
fn int_min(v: &Value, key: &str, default: i32, min: i32) -> Result<i32, String> {
    let n = opt_int(v, key)?.unwrap_or(default);
    if n < min {
        return Err(format!("\"{key}\" must be at least {min}"));
    }
    Ok(n)
}

/// An optional finite number field with a default, checked against a minimum (inclusive).
fn f32_min(v: &Value, key: &str, default: f32, min: f32) -> Result<f32, String> {
    let n = opt_f64(v, key)?.map(|n| n as f32).unwrap_or(default);
    if !n.is_finite() || n < min {
        return Err(format!("\"{key}\" must be a number of at least {min}"));
    }
    Ok(n)
}

/// One enemy attack (see `EnemyAttack`). Defaults by kind: melee range 2, charge range 8 speed 10, shot range 12
/// speed 8 radius 0.3, slam range 6 radius 2; windup 0.6 s, cooldown 1.5 s, weight 1, damage 5, min_phase 0.
fn parse_attack(m: &Value, phase_marks: usize) -> Result<EnemyAttack, String> {
    if !m.is_object() {
        return Err("an attack must be an object with \"name\"".to_string());
    }
    let name = req_str(m, "name")?;
    let ctx = |e: String| format!("attack '{name}': {e}");
    let kind = match opt_str(m, "kind").map_err(&ctx)? {
        None => AttackKind::Melee,
        Some(k) => AttackKind::parse(&k).map_err(&ctx)?,
    };
    let (range, speed, radius) = match kind {
        AttackKind::Melee => (2.0, 0.0, 0.0),
        AttackKind::Charge => (8.0, 10.0, 0.0),
        AttackKind::Shot => (12.0, 8.0, 0.3),
        AttackKind::Slam => (6.0, 0.0, 2.0),
    };
    let damage = int_min(m, "damage", 5, 1).map_err(&ctx)?;
    let weight = opt_int(m, "weight").map_err(&ctx)?.unwrap_or(1);
    if weight < 1 {
        return Err(ctx("\"weight\" must be above 0".to_string()));
    }
    let windup = f32_min(m, "windup", 0.6, 0.0).map_err(&ctx)?;
    if windup <= 0.0 {
        return Err(ctx("\"windup\" must be above 0".to_string()));
    }
    let min_phase = int_min(m, "min_phase", 0, 0).map_err(&ctx)? as usize;
    if min_phase > phase_marks {
        return Err(ctx(format!("\"min_phase\" {min_phase} is more than the battle's {phase_marks} phase mark(s)")));
    }
    Ok(EnemyAttack {
        kind,
        damage,
        range: f32_min(m, "range", range, 0.0).map_err(&ctx)?,
        windup,
        cooldown: f32_min(m, "cooldown", 1.5, 0.0).map_err(&ctx)?,
        weight: weight as u32,
        min_phase,
        radius: f32_min(m, "radius", radius, 0.0).map_err(&ctx)?,
        speed: f32_min(m, "speed", speed, 0.0).map_err(&ctx)?,
        text: opt_str(m, "text").map_err(&ctx)?.unwrap_or_default(),
        name,
    })
}

/// One battle (see `Battle`). Defaults: name = id, not a boss, 20 HP, defense 1, move_speed 3 m/s. "spawn" is
/// required; "attacks" needs at least one entry; "phases" are percent marks from high to low.
fn parse_battle(def: &Value, id: &str) -> Result<Battle, String> {
    if !def.is_object() {
        return Err("a battle must be an object".to_string());
    }
    let mut phases: Vec<i32> = Vec::new();
    match def.get("phases") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for p in list {
                let n = p.as_i64().ok_or_else(|| "\"phases\" must be a list of whole numbers".to_string())?;
                if !(1..=99).contains(&n) {
                    return Err(format!("phase {n} must be between 1 and 99 (a percent of the enemy's HP)"));
                }
                if phases.last().is_some_and(|last| n as i32 >= *last) {
                    return Err("\"phases\" must go from high to low, each mark below the one before".to_string());
                }
                phases.push(n as i32);
            }
        }
        Some(_) => return Err("\"phases\" must be a list".to_string()),
    }
    let mut attacks = Vec::new();
    match def.get("attacks") {
        Some(Value::Array(list)) => {
            for m in list {
                attacks.push(parse_attack(m, phases.len())?);
            }
        }
        None | Some(Value::Null) => {}
        Some(_) => return Err("\"attacks\" must be a list".to_string()),
    }
    if attacks.is_empty() {
        return Err("a battle needs at least one attack".to_string());
    }
    if def.get("xp").is_some() {
        return Err(XP_REMOVED.to_string());
    }
    let spawn = req_str(def, "spawn")?;
    if spawn.is_empty() {
        return Err("\"spawn\" must name a spawn on a map".to_string());
    }
    Ok(Battle {
        id: id.to_string(),
        name: opt_str(def, "name")?.unwrap_or_else(|| id.to_string()),
        boss: opt_bool(def, "boss", false)?,
        max_hp: int_min(def, "max_hp", 20, 1)?,
        defense: int_min(def, "defense", 1, 0)?,
        move_speed: f32_min(def, "move_speed", 3.0, 0.0)?,
        skin: opt_str(def, "skin")?,
        spawn,
        attacks,
        phases,
        intro: opt_str(def, "intro")?.unwrap_or_default(),
        win: opt_str(def, "win")?,
        lose: opt_str(def, "lose")?,
    })
}

/// One quest: "title" (default: the id), "description", optional "show_if" and "steps": [{"text", "done_if"}].
fn parse_quest(def: &Value, id: &str) -> Result<Quest, String> {
    if !def.is_object() {
        return Err("a quest must be an object".to_string());
    }
    let mut steps = Vec::new();
    match def.get("steps") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for (i, s) in list.iter().enumerate() {
                if !s.is_object() {
                    return Err(format!("step {} must be an object with \"text\" and \"done_if\"", i + 1));
                }
                let cond = s.get("done_if").ok_or_else(|| format!("step {}: missing \"done_if\"", i + 1))?;
                steps.push(QuestStep {
                    text: req_str(s, "text").map_err(|e| format!("step {}: {e}", i + 1))?,
                    done_if: parse_cond(cond).map_err(|e| format!("step {}: {e}", i + 1))?,
                });
            }
        }
        Some(_) => return Err("\"steps\" must be a list".to_string()),
    }
    if steps.is_empty() {
        return Err("a quest needs at least one step".to_string());
    }
    let show_if = match def.get("show_if") {
        None | Some(Value::Null) => None,
        Some(c) => Some(parse_cond(c).map_err(|e| format!("show_if: {e}"))?),
    };
    Ok(Quest {
        id: id.to_string(),
        title: opt_str(def, "title")?.unwrap_or_else(|| id.to_string()),
        description: opt_str(def, "description")?.unwrap_or_default(),
        show_if,
        steps,
    })
}

/// The player's "stats" object: every field optional; max_hp must be above 0, the others at least 0.
fn parse_stats(v: &Value) -> Result<Stats, String> {
    if v.is_null() {
        return Ok(Stats::default());
    }
    if !v.is_object() {
        return Err("player \"stats\" must be an object".to_string());
    }
    let d = Stats::default();
    let field = |k: &str, default: i32, min: i32| int_min(v, k, default, min).map_err(|e| format!("player stats {e}"));
    Ok(Stats {
        max_hp: field("max_hp", d.max_hp, 1)?,
        attack: field("attack", d.attack, 0)?,
        defense: field("defense", d.defense, 0)?,
        speed: field("speed", d.speed, 0)?,
    })
}

/// The "story" block: `title` (default: the pack name), `era` (required), `order` (default 0) and `player`.
fn parse_story(v: &Value, pack_name: &str) -> Result<StoryBlock, String> {
    if !v.is_object() {
        return Err("must be an object".to_string());
    }
    let title = opt_str(v, "title")?.unwrap_or_else(|| pack_name.to_string());
    let era_name = req_str(v, "era")?;
    let era = Era::parse(&era_name).ok_or_else(|| format!("unknown era '{era_name}'; use one of {}", Era::NAMES.join(", ")))?;
    let order = opt_f64(v, "order")?.unwrap_or(0.0);
    let p = &v["player"];
    let player = if p.is_null() {
        Player::default()
    } else {
        if !p.is_object() {
            return Err("\"player\" must be an object".to_string());
        }
        let name = opt_str(p, "name")?.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| Player::default().name);
        let speed = match p.get("speed") {
            None | Some(Value::Null) => 1.0,
            Some(s) => s
                .as_f64()
                .filter(|s| *s > 0.0 && *s <= MAX_SPEED)
                .ok_or_else(|| format!("player \"speed\" must be a number above 0 and at most {MAX_SPEED}"))? as f32,
        };
        Player {
            name,
            role: opt_str(p, "role")?.unwrap_or_default(),
            speed,
            body: parse_body(p)?,
            start_items: str_list(p, "start_items")?,
            start_flags: str_list(p, "start_flags")?,
            stats: parse_stats(&p["stats"])?,
        }
    };
    let multiplayer = opt_bool(v, "multiplayer", false)?;
    Ok(StoryBlock { title, era, order, player, multiplayer })
}

/// A list of text under `key` in `v`; a missing or null list is empty.
fn str_list(v: &Value, key: &str) -> Result<Vec<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("\"{key}\" must be a list of text")))
            .collect(),
        Some(_) => Err(format!("\"{key}\" must be a list")),
    }
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
                Op::Battle(b) => {
                    if !pack.battles.contains_key(b) {
                        errs.push(format!("{ctx}: unknown battle '{b}'"));
                    }
                }
                Op::SetFlag { .. } | Op::Toast(_) | Op::Heal(_) | Op::Weather { .. } => {}
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

    let mut story_ids = BTreeSet::new();
    for story in &pack.stories {
        let ctx = format!("story '{}'", story.id);
        if !story_ids.insert(story.id.as_str()) {
            errs.push(format!("{ctx} is defined more than once"));
        }
        match pack.maps.get(&story.start_map) {
            None => errs.push(format!("{ctx}: start map '{}' does not exist", story.start_map)),
            Some(m) if !m.spawns.contains_key(&story.start_spawn) => {
                errs.push(format!("{ctx}: start map '{}' has no spawn '{}'", story.start_map, story.start_spawn))
            }
            Some(_) => {}
        }
        if let Some(s) = &story.start_scene {
            scene_ref(s, &format!("{ctx} start_scene"), &mut errs);
        }
        for item in &story.player.start_items {
            items(item, &format!("{ctx} player start_items"), &mut errs);
        }
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

    for (id, b) in &pack.battles {
        let ctx = format!("battle '{id}'");
        for (what, s) in [("win", &b.win), ("lose", &b.lose)] {
            if let Some(s) = s {
                scene_ref(s, &format!("{ctx} {what}"), &mut errs);
            }
        }
        if !pack.maps.values().any(|m| m.spawns.contains_key(&b.spawn)) {
            errs.push(format!("{ctx}: spawn '{}' is not on any map", b.spawn));
        }
        if let Some(skin) = &b.skin {
            if !pack.skins.contains_key(skin) {
                errs.push(format!("{ctx}: unknown skin '{skin}' (no pack has skins/{skin}.png)"));
            }
        }
    }
    for (id, q) in &pack.quests {
        let ctx = format!("quest '{id}'");
        if let Some(c) = &q.show_if {
            conds(c, &ctx, &mut errs);
        }
        for (i, s) in q.steps.iter().enumerate() {
            conds(&s.done_if, &format!("{ctx} step {}", i + 1), &mut errs);
        }
    }

    for (id, map) in &pack.maps {
        for kind in map.legend.values() {
            let ctx = format!("map '{id}'");
            match kind {
                Kind::Door { open_if, .. } => conds(open_if, &ctx, &mut errs),
                Kind::Item { item, .. } => items(item, &ctx, &mut errs),
                Kind::Npc { scene, skin, routine, .. } => {
                    scene_ref(scene, &ctx, &mut errs);
                    if let Some(r) = routine {
                        if !pack.routines.contains_key(r) {
                            errs.push(format!("{ctx}: unknown routine '{r}' (no pack has it in routines/*.json)"));
                        }
                    }
                    if let Some(name) = skin {
                        if !pack.skins.contains_key(name) {
                            errs.push(format!("{ctx}: unknown skin '{name}' (no pack has skins/{name}.png)"));
                        }
                    }
                }
                Kind::Trigger { scene, .. } => scene_ref(scene, &ctx, &mut errs),
                Kind::Exit { map: m, spawn } => spawn_ref(m, spawn, &ctx, &mut errs),
                Kind::Prop { prop, use_, .. } => {
                    if !pack.props.contains_key(prop) {
                        errs.push(format!("{ctx}: unknown prop '{prop}' (no pack has props/{prop}.bbmodel)"));
                    }
                    if let Some(PropUse { scene: Some(s), .. }) = use_ {
                        scene_ref(s, &format!("{ctx}: prop '{prop}' use"), &mut errs);
                    }
                }
                Kind::Floor | Kind::Wall { .. } | Kind::Spawn { .. } => {}
            }
        }
    }
    validate_routines(pack, &mut errs);
    errs
}

/// Checks each routine against every map an NPC using it stands on: "go", "shelter" and "flee" targets must exist on
/// that map (a spawn name, or a `prop:<x>,<y>` tile that holds a prop), and a "with" must name an NPC on the same map.
fn validate_routines(pack: &Pack, errs: &mut Vec<String>) {
    for (map_id, map) in &pack.maps {
        let mut used: BTreeSet<&str> = BTreeSet::new();
        for kind in &map.grid.cells {
            if let Kind::Npc { routine: Some(r), .. } = kind {
                used.insert(r.as_str());
            }
        }
        for rid in used {
            let Some(routine) = pack.routines.get(rid) else { continue }; // reported with the legend
            let ctx = format!("routine '{rid}' on map '{map_id}'");
            for (what, target) in [("shelter", &routine.shelter), ("flee", &routine.flee)] {
                if let Some(t) = target {
                    if !map.spawns.contains_key(t) {
                        errs.push(format!("{ctx}: \"{what}\" spawn '{t}' is not on this map"));
                    }
                }
            }
            for (i, step) in routine.steps.iter().enumerate() {
                let sctx = format!("{ctx}, step {}", i + 1);
                if let Some(go) = &step.go {
                    match parse_prop_target(go) {
                        None => {
                            if !map.spawns.contains_key(go) {
                                errs.push(format!("{sctx}: \"go\" spawn '{go}' is not on this map"));
                            }
                        }
                        Some(Err(e)) => errs.push(format!("{sctx}: \"go\" {e}")),
                        Some(Ok((x, y))) => {
                            if !matches!(map.grid.at(x, y), Some(Kind::Prop { .. })) {
                                errs.push(format!("{sctx}: \"go\" target prop:{x},{y} holds no prop on this map"));
                            }
                        }
                    }
                }
                if let Some(with) = &step.with {
                    let there = map.grid.cells.iter().any(|k| matches!(k, Kind::Npc { name, .. } if name == with));
                    if !there {
                        errs.push(format!("{sctx}: \"with\" NPC '{with}' is not on this map"));
                    }
                }
            }
        }
    }
}

/// One routine: `{"loop"?: bool (default true), "steps": [...], "shelter"?, "flee"?}`. A step is `{"go"?, "do"?: action
/// (default idle), "secs"?: above 0 (default 5), "with"?: NPC name (chat needs it), "radius"?: above 0 (wander, 3)}`.
fn parse_routine(def: &Value) -> Result<Routine, String> {
    if !def.is_object() {
        return Err("a routine must be an object with \"steps\"".to_string());
    }
    let steps_json = match def.get("steps") {
        Some(Value::Array(a)) if !a.is_empty() => a,
        Some(Value::Array(_)) | None | Some(Value::Null) => return Err("\"steps\" needs at least one step".to_string()),
        Some(_) => return Err("\"steps\" must be a list".to_string()),
    };
    let mut steps = Vec::new();
    for (i, s) in steps_json.iter().enumerate() {
        steps.push(parse_routine_step(s).map_err(|e| format!("step {}: {e}", i + 1))?);
    }
    Ok(Routine { looping: opt_bool(def, "loop", true)?, steps, shelter: opt_str(def, "shelter")?, flee: opt_str(def, "flee")? })
}

fn parse_routine_step(s: &Value) -> Result<RoutineStep, String> {
    if !s.is_object() {
        return Err("a step must be an object".to_string());
    }
    let do_ = match opt_str(s, "do")? {
        None => NpcDo::Idle,
        Some(name) => NpcDo::parse(&name)?,
    };
    let secs = opt_f64(s, "secs")?.unwrap_or(DEFAULT_STEP_SECS as f64);
    if secs <= 0.0 {
        return Err(format!("\"secs\" must be above 0, got {secs}"));
    }
    let radius = opt_f64(s, "radius")?.unwrap_or(DEFAULT_WANDER_RADIUS as f64);
    if radius <= 0.0 {
        return Err(format!("\"radius\" must be above 0, got {radius}"));
    }
    let with = opt_str(s, "with")?;
    if do_ == NpcDo::Chat && with.is_none() {
        return Err("a chat step needs \"with\" (the NPC to talk to)".to_string());
    }
    Ok(RoutineStep { go: opt_str(s, "go")?, do_, secs: secs as f32, with, radius: radius as f32 })
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
    // The pictures live in the pack's maps folder: a plain file name only, so a map cannot read another pack's files.
    for (field, file) in [("art", &art_name), ("mask", &mask_name)] {
        let plain = !file.is_empty() && !file.contains(['/', '\\', ':']) && file != "." && file != "..";
        if !plain {
            errs.push(format!("{ctx}: \"{field}\" must be a file name in the maps folder, not a path: '{file}'"));
            return None;
        }
    }
    let art_path = dir.join("maps").join(&art_name);
    let mask_path = dir.join("maps").join(&mask_name);

    let mut legend = BTreeMap::new();
    let legend_entries = match container(&v, "legend") {
        Ok(e) => e,
        Err(e) => {
            errs.push(format!("{ctx}: {e}"));
            return None;
        }
    };
    let mut spawn_facing = BTreeMap::new();
    for (hex, def) in legend_entries {
        let Some(key) = normalise_colour(&hex) else {
            errs.push(format!("{ctx}: legend key '{hex}' is not a 6-digit hex colour or \"clear\""));
            continue;
        };
        match parse_kind(def) {
            Ok(kind) => {
                if let Kind::Spawn { name } = &kind {
                    match parse_facing(def) {
                        Ok(Some(yaw)) => {
                            spawn_facing.insert(name.clone(), yaw);
                        }
                        Ok(None) => {}
                        Err(e) => errs.push(format!("{ctx}: legend '{hex}': {e}")),
                    }
                }
                if legend.insert(key.clone(), kind).is_some() {
                    errs.push(format!("{ctx}: legend colour '{key}' is listed twice (keys are not case or '#' sensitive)"));
                }
            }
            Err(e) => errs.push(format!("{ctx}: legend '{hex}': {e}")),
        }
    }

    // The sizes come from the PNG headers, before anything is decoded: macroquad keeps a side in a u16, so a huge
    // picture could pass a check made on the decoded image.
    let art_png = match read_png_bytes(&art_path) {
        Ok(p) => p,
        Err(e) => {
            errs.push(format!("{ctx}: {e}"));
            return None;
        }
    };
    let mask_png = if mask_path == art_path {
        None
    } else {
        match read_png_bytes(&mask_path) {
            Ok(p) => Some(p),
            Err(e) => {
                errs.push(format!("{ctx}: {e}"));
                return None;
            }
        }
    };
    let (mw, mh) = {
        let m = mask_png.as_ref().unwrap_or(&art_png);
        (m.w as usize, m.h as usize)
    };
    let (aw, ah) = (art_png.w as usize, art_png.h as usize);
    if mw == 0 || mh == 0 || mw > MAX_MASK_SIDE || mh > MAX_MASK_SIDE {
        errs.push(format!("{ctx}: mask is {mw}x{mh}; each side must be 1-{MAX_MASK_SIDE} pixels"));
        return None;
    }
    if aw > MAX_ART_SIDE || ah > MAX_ART_SIDE {
        errs.push(format!("{ctx}: art is {aw}x{ah}; each side must be at most {MAX_ART_SIDE} pixels"));
        return None;
    }
    let decode = |png: &PngBytes, path: &Path, errs: &mut Vec<String>| match decode_png(png, path) {
        Ok(img) => Some(img),
        Err(e) => {
            errs.push(format!("{ctx}: {e}"));
            None
        }
    };
    let art = decode(&art_png, &art_path, errs)?;
    let mask = match &mask_png {
        Some(p) => Some(decode(p, &mask_path, errs)?),
        None => None,
    };
    let mask = mask.as_ref().unwrap_or(&art);
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
    let env = match parse_map_env(&v, &legend) {
        Ok(env) => env,
        Err(e) => {
            errs.push(format!("{ctx}: {e}"));
            return None;
        }
    };
    Some(MapDef {
        id,
        name,
        art: art_path,
        legend,
        grid: Grid { w: mw, h: mh, cells },
        spawns,
        spawn_facing,
        outdoor: env.outdoor,
        ceiling_height: env.ceiling_height,
        ceiling_color: env.ceiling_color,
        sky: env.sky,
        wall_style: env.wall_style,
        floor_style: env.floor_style,
        weather: env.weather,
    })
}

/// A spawn's optional "facing": the way the body looks on arriving there. Degrees clockwise from north (north is up the
/// map picture, east is right), or one of "north", "east", "south", "west". Returns the body yaw in radians (the body's
/// forward is (-sin yaw, -cos yaw), so north is 0 and east is -90 degrees). None when the spawn names no facing.
fn parse_facing(def: &Value) -> Result<Option<f32>, String> {
    let degrees = match def.get("facing") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => match s.as_str() {
            "north" => 0.0,
            "east" => 90.0,
            "south" => 180.0,
            "west" => 270.0,
            other => return Err(format!("spawn \"facing\" '{other}' must be north, east, south, west or a number of degrees")),
        },
        Some(v) => v
            .as_f64()
            .filter(|d| d.is_finite())
            .ok_or("spawn \"facing\" must be north, east, south, west or a number of degrees")? as f32,
    };
    Ok(Some(-degrees.to_radians()))
}

/// The look fields of a map (see `MapDef`), parsed apart from the pictures so they can be tested alone.
#[derive(Debug, PartialEq)]
struct MapEnv {
    outdoor: bool,
    ceiling_height: f32,
    ceiling_color: Option<Rgb>,
    sky: Option<(Rgb, Rgb)>,
    wall_style: String,
    floor_style: String,
    weather: Option<WeatherDef>,
}

/// A hex colour in `field` of `v` (with or without '#'), or None when absent.
fn opt_hex(v: &Value, field: &str, what: &str) -> Result<Option<Rgb>, String> {
    let Some(raw) = opt_str(v, field).map_err(|e| format!("{what}: {e}"))? else { return Ok(None) };
    let key = normalise_colour(&raw)
        .filter(|k| k != "clear")
        .ok_or_else(|| format!("{what}: bad \"{field}\" colour '{raw}' (6-digit hex)"))?;
    let n = u32::from_str_radix(&key, 16).map_err(|e| e.to_string())?;
    Ok(Some([(n >> 16) as u8, (n >> 8) as u8, n as u8]))
}

/// A style name from `allowed`, or the first of them when the field is absent.
fn parse_style(v: &Value, field: &str, allowed: &[&str]) -> Result<String, String> {
    match opt_str(v, field)? {
        None => Ok(allowed[0].to_string()),
        Some(s) if allowed.contains(&s.as_str()) => Ok(s),
        Some(s) => Err(format!("\"{field}\" style '{s}' is unknown (use one of: {})", allowed.join(", "))),
    }
}

/// An optional object field: absent or null is None, an object is Some, anything else is an error.
fn opt_object<'a>(v: &'a Value, field: &str) -> Result<Option<&'a Value>, String> {
    match v.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(o) if o.is_object() => Ok(Some(o)),
        Some(_) => Err(format!("\"{field}\" must be an object")),
    }
}

/// Outdoor, ceiling, sky and style fields of a map file. An outdoor map has no ceiling (so "ceiling" is an error
/// there) and a sky; an indoor map has a ceiling and no sky (so "sky" is an error there).
fn parse_map_env(v: &Value, legend: &BTreeMap<String, Kind>) -> Result<MapEnv, String> {
    let outdoor = opt_bool(v, "outdoor", false)?;
    let tallest = legend
        .values()
        .filter_map(|k| if let Kind::Wall { height, .. } = k { Some(*height) } else { None })
        .fold(None, |m: Option<f32>, h| Some(m.map_or(h, |m| m.max(h))));
    let mut ceiling_height = tallest.map_or(DEFAULT_CEILING_HEIGHT, |h| h.clamp(1.0, 16.0));
    let mut ceiling_color = None;
    if let Some(c) = opt_object(v, "ceiling")? {
        if outdoor {
            return Err("an outdoor map has no \"ceiling\" (remove it or set \"outdoor\" to false)".to_string());
        }
        if let Some(h) = opt_f64(c, "height").map_err(|e| format!("ceiling: {e}"))? {
            if !(1.0..=16.0).contains(&h) {
                return Err(format!("ceiling: \"height\" must be between 1 and 16 metres, got {h}"));
            }
            ceiling_height = h as f32;
        }
        ceiling_color = opt_hex(c, "color", "ceiling")?;
    }
    let mut sky = None;
    if let Some(s) = opt_object(v, "sky")? {
        if !outdoor {
            return Err("\"sky\" needs \"outdoor\": true (an indoor map has a ceiling)".to_string());
        }
        sky = Some((
            opt_hex(s, "top", "sky")?.unwrap_or(DEFAULT_SKY.0),
            opt_hex(s, "bottom", "sky")?.unwrap_or(DEFAULT_SKY.1),
        ));
    } else if outdoor {
        sky = Some(DEFAULT_SKY);
    }
    Ok(MapEnv {
        outdoor,
        ceiling_height,
        ceiling_color,
        sky,
        wall_style: parse_style(v, "walls", &WALL_STYLES)?,
        floor_style: parse_style(v, "floor", &FLOOR_STYLES)?,
        weather: parse_weather(v, outdoor)?,
    })
}

/// A map's optional "weather": `{"kind", "intensity"?: 0..1 (default 0.6), "changes"?: [{"kind", "weight"}] (weight
/// above 0), "every_secs"?: above 0 (default 90)}`. Outdoor maps only.
fn parse_weather(v: &Value, outdoor: bool) -> Result<Option<WeatherDef>, String> {
    let Some(w) = opt_object(v, "weather")? else { return Ok(None) };
    if !outdoor {
        return Err("\"weather\" needs \"outdoor\": true (an indoor map has no sky)".to_string());
    }
    let kind = WeatherKind::parse(&req_str(w, "kind").map_err(|e| format!("weather: {e}"))?).map_err(|e| format!("weather: {e}"))?;
    let intensity = parse_intensity(w).map_err(|e| format!("weather: {e}"))?;
    let every_secs = opt_f64(w, "every_secs").map_err(|e| format!("weather: {e}"))?.unwrap_or(DEFAULT_WEATHER_EVERY_SECS as f64);
    if every_secs <= 0.0 {
        return Err(format!("weather: \"every_secs\" must be above 0, got {every_secs}"));
    }
    let mut changes = Vec::new();
    match w.get("changes") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for (i, c) in list.iter().enumerate() {
                let ctx = format!("weather change {}", i + 1);
                if !c.is_object() {
                    return Err(format!("{ctx}: must be an object with \"kind\" and \"weight\""));
                }
                let k = WeatherKind::parse(&req_str(c, "kind").map_err(|e| format!("{ctx}: {e}"))?).map_err(|e| format!("{ctx}: {e}"))?;
                let weight = opt_f64(c, "weight").map_err(|e| format!("{ctx}: {e}"))?.ok_or_else(|| format!("{ctx}: missing \"weight\""))?;
                if weight <= 0.0 {
                    return Err(format!("{ctx}: \"weight\" must be above 0, got {weight}"));
                }
                changes.push((k, weight as f32));
            }
        }
        Some(_) => return Err("weather: \"changes\" must be a list of {\"kind\", \"weight\"}".to_string()),
    }
    Ok(Some(WeatherDef { kind, intensity, changes, every_secs: every_secs as f32 }))
}

/// The optional "intensity" (0..=1, default `DEFAULT_WEATHER_INTENSITY`) of a weather block or op.
fn parse_intensity(v: &Value) -> Result<f32, String> {
    match opt_f64(v, "intensity")? {
        None => Ok(DEFAULT_WEATHER_INTENSITY),
        Some(i) if (0.0..=1.0).contains(&i) => Ok(i as f32),
        Some(i) => Err(format!("\"intensity\" must be between 0 and 1, got {i}")),
    }
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
    let h = opt_f64(v, "height")?.unwrap_or(3.0);
    if h > 0.0 && h <= 16.0 {
        Ok(h as f32)
    } else {
        Err(format!("\"height\" must be above 0 and at most 16 metres, got {h}"))
    }
}

/// The optional "color" field as RGB, or the default.
fn parse_rgb(v: &Value, default: Rgb) -> Result<Rgb, String> {
    let Some(raw) = opt_str(v, "color")? else { return Ok(default) };
    let key = normalise_colour(&raw).filter(|k| k != "clear").ok_or_else(|| format!("bad \"color\" '{raw}'"))?;
    let n = u32::from_str_radix(&key, 16).map_err(|e| e.to_string())?;
    Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

const WALL_COLOUR: Rgb = [0x5a, 0x63, 0x78];
const DOOR_COLOUR: Rgb = [0xb0, 0x70, 0x30];
const ITEM_COLOUR: Rgb = [0xff, 0xd8, 0x4a];
const NPC_COLOUR: Rgb = [0x6f, 0xbf, 0x8a];
const PROP_COLOUR: Rgb = [0x9a, 0x9a, 0x9a];

fn parse_kind(v: &Value) -> Result<Kind, String> {
    if !v.is_object() {
        return Err("a legend entry must be an object with a \"kind\"".to_string());
    }
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
                locked: opt_str(v, "locked")?.unwrap_or_else(|| DEFAULT_LOCKED.to_string()),
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
            body: parse_body(v)?,
            skin: parse_skin(v)?,
            routine: parse_routine_ref(v)?,
        },
        "trigger" => Kind::Trigger {
            scene: req_str(v, "scene")?,
            once: opt_bool(v, "once", true)?,
        },
        "exit" => Kind::Exit {
            map: req_str(v, "map")?,
            spawn: req_str(v, "spawn")?,
        },
        "prop" => Kind::Prop {
            prop: req_str(v, "prop")?,
            yaw: parse_yaw(v)?,
            color: parse_rgb(v, PROP_COLOUR)?,
            use_: parse_prop_use(v)?,
        },
        other => return Err(format!("unknown kind '{other}'")),
    })
}

/// The optional "body" field (of an NPC, or of a story's "player" block): a mannequin body name, default "blocky".
fn parse_body(v: &Value) -> Result<BodyChoice, String> {
    match v.get("body") {
        None | Some(Value::Null) => Ok(BodyChoice::default()),
        Some(Value::String(name)) => BodyChoice::parse(name),
        Some(_) => Err(format!("\"body\" must be a name: one of {}", BodyChoice::NAMES.join(", "))),
    }
}

/// The optional "skin" field of an NPC: the name of a skin in a pack's skins folder (the file's name without ".png").
/// Letters, digits, "_" and "-" only, so the name is a plain id. None (the default) is the default atlas.
fn parse_skin(v: &Value) -> Result<Option<String>, String> {
    match v.get("skin") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => {
            if is_plain_name(name) {
                Ok(Some(name.clone()))
            } else {
                Err(format!("\"skin\" must be the name of a skin (letters, digits, '_' or '-'), got '{name}'"))
            }
        }
        Some(_) => Err("\"skin\" must be the name of a skin in the pack's skins folder".to_string()),
    }
}

/// The optional "routine" field of an NPC: a routine id (letters, digits, '_' or '-').
fn parse_routine_ref(v: &Value) -> Result<Option<String>, String> {
    match opt_str(v, "routine")? {
        None => Ok(None),
        Some(r) if is_plain_name(&r) => Ok(Some(r)),
        Some(r) => Err(format!("\"routine\" must be a routine id (letters, digits, '_' or '-'), got '{r}'")),
    }
}

/// The optional "use" object of a prop: `{"action", "text"?, "scene"?, "seat_height"?: 0..=2 metres}`.
fn parse_prop_use(v: &Value) -> Result<Option<PropUse>, String> {
    let Some(u) = opt_object(v, "use")? else { return Ok(None) };
    let action = PropAction::parse(&req_str(u, "action").map_err(|e| format!("use: {e}"))?).map_err(|e| format!("use: {e}"))?;
    let text = opt_str(u, "text").map_err(|e| format!("use: {e}"))?.unwrap_or_else(|| action.default_text().to_string());
    if text.trim().is_empty() {
        return Err("use: \"text\" must not be empty".to_string());
    }
    let seat_height = match opt_f64(u, "seat_height").map_err(|e| format!("use: {e}"))? {
        None => DEFAULT_SEAT_HEIGHT,
        Some(h) if (0.0..=MAX_SEAT_HEIGHT as f64).contains(&h) => h as f32,
        Some(h) => return Err(format!("use: \"seat_height\" must be between 0 and {MAX_SEAT_HEIGHT} metres, got {h}")),
    };
    Ok(Some(PropUse { action, text, scene: opt_str(u, "scene").map_err(|e| format!("use: {e}"))?, seat_height }))
}

/// The optional "yaw" field: a prop's turn about the vertical axis in degrees, default 0. It must be a finite number.
fn parse_yaw(v: &Value) -> Result<f32, String> {
    match v.get("yaw") {
        None | Some(Value::Null) => Ok(0.0),
        Some(n) => n
            .as_f64()
            .filter(|d| d.is_finite())
            .map(|d| d as f32)
            .ok_or_else(|| "\"yaw\" must be a number of degrees".to_string()),
    }
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
    // An empty list would make "any" never true and "all" always true, with nothing to show why.
    if v.as_array().is_some_and(|a| a.is_empty()) {
        return Err("\"all\" and \"any\" need at least one condition".to_string());
    }
    v.as_array()
        .ok_or("\"all\" and \"any\" take a list")?
        .iter()
        .map(parse_cond)
        .collect()
}

/// The error for the removed XP system ("op": "xp" and a battle's "xp").
const XP_REMOVED: &str = "XP was removed: KYTTU - LRS has no XP and no levels (delete the \"xp\" field or op)";

fn parse_op(v: &Value) -> Result<Op, String> {
    Ok(match req_str(v, "op")?.as_str() {
        "set_flag" => Op::SetFlag {
            flag: req_str(v, "flag")?,
            value: opt_bool(v, "value", true)?,
        },
        "give" => Op::Give(req_str(v, "item")?),
        "take" => Op::Take(req_str(v, "item")?),
        "exit" => Op::Exit {
            map: req_str(v, "map")?,
            spawn: req_str(v, "spawn")?,
        },
        "toast" => Op::Toast(req_str(v, "text")?),
        "battle" => Op::Battle(req_str(v, "battle")?),
        "heal" => Op::Heal(op_amount(v)?),
        "weather" => Op::Weather {
            kind: WeatherKind::parse(&req_str(v, "kind")?)?,
            intensity: match opt_f64(v, "intensity")? {
                None => None,
                Some(_) => Some(parse_intensity(v)?),
            },
        },
        "xp" => return Err(XP_REMOVED.to_string()),
        other => return Err(format!("unknown op '{other}'")),
    })
}

/// The "amount" of a heal op: a whole number above 0.
fn op_amount(v: &Value) -> Result<i32, String> {
    match opt_int(v, "amount")? {
        Some(n) if n > 0 => Ok(n),
        Some(_) => Err("\"amount\" must be above 0".to_string()),
        None => Err("missing \"amount\"".to_string()),
    }
}

fn parse_ops(v: &Value) -> Result<Vec<Op>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::Array(list) => list.iter().map(parse_op).collect(),
        _ => Err("\"do\" takes a list of ops".to_string()),
    }
}

fn parse_scene(v: &Value) -> Result<Scene, String> {
    if !v.is_object() {
        return Err("a scene must be an object (with \"lines\", \"choices\", \"do\", \"next\")".to_string());
    }
    let mut lines = Vec::new();
    match v.get("lines") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for l in list {
                lines.push(match l {
                    Value::String(text) => Line {
                        speaker: String::new(),
                        text: text.clone(),
                    },
                    Value::Object(_) => Line {
                        speaker: opt_str(l, "speaker")?.unwrap_or_default(),
                        text: req_str(l, "text")?,
                    },
                    _ => return Err("a line must be text or an object with \"text\"".to_string()),
                });
            }
        }
        Some(_) => return Err("\"lines\" must be a list".to_string()),
    }
    let mut choices = Vec::new();
    match v.get("choices") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for c in list {
                choices.push(Choice {
                    text: req_str(c, "text")?,
                    cond: match c.get("if") {
                        Some(cond) => Some(parse_cond(cond)?),
                        None => None,
                    },
                    ops: parse_ops(&c["do"])?,
                    next: opt_str(c, "next")?,
                });
            }
        }
        Some(_) => return Err("\"choices\" must be a list".to_string()),
    }
    Ok(Scene {
        once: opt_bool(v, "once", false)?,
        lines,
        choices,
        ops: parse_ops(&v["do"])?,
        next: opt_str(v, "next")?,
    })
}

fn read_json(path: &Path, schema: &str) -> Result<Value, String> {
    let name = path.display();
    let text = vfs::read_to_string(path).map_err(|e| format!("{name}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
    match str_field(&v, "schema") {
        Some(s) if s == schema => Ok(v),
        Some(s) => Err(format!("{name}: schema is '{s}', expected '{schema}'")),
        None => Err(format!("{name}: missing \"schema\": \"{schema}\"")),
    }
}

/// A PNG file's bytes with the size its header declares.
struct PngBytes {
    bytes: Vec<u8>,
    w: u32,
    h: u32,
}

/// The width and height in a PNG's IHDR chunk (the first chunk, right after the 8-byte signature). Pure.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

fn read_png_bytes(path: &Path) -> Result<PngBytes, String> {
    let bytes = vfs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = png_size(&bytes).ok_or_else(|| format!("{}: not a PNG (no valid header)", path.display()))?;
    Ok(PngBytes { bytes, w, h })
}

fn decode_png(png: &PngBytes, path: &Path) -> Result<Image, String> {
    Image::from_file_with_format(&png.bytes, Some(ImageFormat::Png)).map_err(|e| format!("{}: not a PNG ({e})", path.display()))
}

/// Sorted `*.json` files in a folder. A missing folder is empty, not an error.
/// A scene file whose name contains "overrides". Such files are read after a pack's other scene files.
fn is_override_file(path: &Path) -> bool {
    path.file_name().map_or(false, |n| n.to_string_lossy().contains("overrides"))
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    files_with_extension(dir, "json")
}

/// Sorted `*.bbmodel` files in a folder (a pack's props). A missing folder is empty, not an error.
fn bbmodel_files(dir: &Path) -> Vec<PathBuf> {
    files_with_extension(dir, "bbmodel")
}

fn files_with_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(read) = vfs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = read.into_iter().filter(|p| p.extension().map_or(false, |x| x.eq_ignore_ascii_case(ext))).collect();
    files.sort();
    files
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

/// An optional text field: absent or null is None, text is Some, anything else is an error naming the field.
fn opt_str(v: &Value, key: &str) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("\"{key}\" must be text")),
    }
}

/// An optional boolean field with a default; a value that is not true or false is an error naming the field.
fn opt_bool(v: &Value, key: &str, default: bool) -> Result<bool, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("\"{key}\" must be true or false")),
    }
}

/// An optional finite number field; a value that is not a number is an error naming the field.
fn opt_f64(v: &Value, key: &str) -> Result<Option<f64>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| format!("\"{key}\" must be a number")),
    }
}

fn req_str(v: &Value, key: &str) -> Result<String, String> {
    str_field(v, key).ok_or_else(|| format!("missing \"{key}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::Color;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_story_load_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes pack `id` under `root`: a pack.json with `extra` (the fields after "name"), and one 1x1 map `<id>_map`
    /// whose only pixel is the spawn "here". Returns the pack folder.
    fn write_pack(root: &Path, id: &str, extra: &str) -> PathBuf {
        let dir = root.join(id);
        fs::create_dir_all(dir.join("maps")).unwrap();
        fs::write(
            dir.join("pack.json"),
            format!(r#"{{"schema": "lr.pack/1", "id": "{id}", "name": "{id} name", {extra}}}"#),
        )
        .unwrap();
        let map = format!("{id}_map");
        fs::write(
            dir.join("maps").join(format!("{map}.json")),
            format!(
                r#"{{"schema": "lr.story.map/1", "id": "{map}", "name": "Map", "art": "{map}.png", "legend": {{"00ff00": {{"kind": "spawn", "name": "here"}}}}}}"#
            ),
        )
        .unwrap();
        let art = Image::gen_image_color(1, 1, Color::new(0.0, 1.0, 0.0, 1.0));
        art.export_png(&dir.join("maps").join(format!("{map}.png")).to_string_lossy());
        dir
    }

    /// Writes an items file for `id` holding the item `<id>_key`.
    fn write_key_item(dir: &Path, id: &str) {
        fs::create_dir_all(dir.join("items")).unwrap();
        fs::write(
            dir.join("items").join("items.json"),
            format!(r#"{{"schema": "lr.story.items/1", "items": {{"{id}_key": {{"name": "Key"}}}}}}"#),
        )
        .unwrap();
    }

    fn start_at(id: &str) -> String {
        format!(r#""start": {{"map": "{id}_map", "spawn": "here"}}"#)
    }

    #[test]
    fn story_packs_add_their_own_stories_and_the_core_pack_is_the_sample() {
        let root = scratch("stories");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let alpha = write_pack(
            &root,
            "alpha_story",
            &format!(
                r#"{}, "story": {{"title": "Alpha", "era": "lastroute", "order": 2, "player": {{"name": "Kit", "role": "Hunter", "speed": 1.25, "start_items": ["alpha_story_key"], "start_flags": ["briefed"]}}}}"#,
                start_at("alpha_story")
            ),
        );
        write_key_item(&alpha, "alpha_story");
        let beta = write_pack(
            &root,
            "beta_story",
            &format!(r#"{}, "story": {{"title": "Beta", "era": "past", "order": 2}}"#, start_at("beta_story")),
        );
        let cfg = Config { core, mod_dirs: vec![alpha, beta], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));

        let ids: Vec<&str> = pack.stories.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["core_pack", "alpha_story", "beta_story"], "core first, then the mods by id");

        let sample = pack.story("core_pack").unwrap();
        assert_eq!((sample.era, sample.order), (Era::Sample, 0.0));
        assert_eq!(sample.title, "core_pack name");
        assert_eq!(sample.player, Player::default());
        assert_eq!(sample.start_map, "core_pack_map");

        let alpha = pack.story("alpha_story").unwrap();
        assert_eq!(alpha.title, "Alpha");
        assert_eq!(alpha.era, Era::LastRoute);
        assert_eq!(alpha.start_map, "alpha_story_map");
        assert_eq!(alpha.player.name, "Kit");
        assert_eq!(alpha.player.role, "Hunter");
        assert_eq!(alpha.player.speed, 1.25);
        assert_eq!(alpha.player.start_items, vec!["alpha_story_key".to_string()]);
        assert_eq!(alpha.player.start_flags, vec!["briefed".to_string()]);

        let beta = pack.story("beta_story").unwrap();
        assert_eq!(beta.era, Era::Past);
        assert_eq!(beta.player, Player::default(), "a story without a player block gets the default player");

        // Menu order: by order, then by title (Alpha and Beta share order 2).
        let menu: Vec<&str> = pack.stories_in_menu_order().into_iter().map(|s| s.id.as_str()).collect();
        assert_eq!(menu, vec!["core_pack", "alpha_story", "beta_story"]);
        let _ = fs::remove_dir_all(&root);
    }

    /// A map picture must be a plain file name, a legend colour may appear once, and "any"/"all" need a condition.
    #[test]
    fn map_paths_duplicate_colours_and_empty_condition_lists_are_errors() {
        let root = scratch("map_rules");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let map_file = core.join("maps").join("core_pack_map.json");
        let write_map = |art: &str, legend: &str| {
            fs::write(&map_file, format!(r#"{{"schema": "lr.story.map/1", "id": "core_pack_map", "name": "Map", "art": "{art}", "legend": {{{legend}}}}}"#)).unwrap();
        };
        let spawn = r#""00ff00": {"kind": "spawn", "name": "here"}"#;
        let cfg = || Config { core: core.clone(), mod_dirs: Vec::new(), mod_roots: Vec::new() };

        write_map("../core_pack_map.png", spawn);
        let errs = load(&cfg()).err().expect("a path is refused");
        assert!(errs.iter().any(|e| e.contains("must be a file name in the maps folder")), "{errs:?}");

        write_map("core_pack_map.png", &format!(r##"{spawn}, "#00FF00": {{"kind": "floor"}}"##));
        let errs = load(&cfg()).err().expect("a colour twice is refused");
        assert!(errs.iter().any(|e| e.contains("listed twice")), "{errs:?}");

        write_map("core_pack_map.png", &format!(r#"{spawn}, "ff0000": {{"kind": "door", "if": {{"any": []}}}}"#));
        let errs = load(&cfg()).err().expect("an empty any is refused");
        assert!(errs.iter().any(|e| e.contains("at least one condition")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    /// A spawn's "facing" is a compass name or degrees clockwise from north, turned into the body's yaw.
    #[test]
    fn spawn_facing_reads_names_and_degrees() {
        use serde_json::json;
        let yaw = |v: Value| parse_facing(&v).unwrap().unwrap();
        assert_eq!(parse_facing(&json!({"kind": "spawn"})).unwrap(), None);
        assert!((yaw(json!({"facing": "north"})) - 0.0).abs() < 1e-6);
        assert!((yaw(json!({"facing": "east"})) + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((yaw(json!({"facing": 180})) + std::f32::consts::PI).abs() < 1e-6);
        // east really faces +x with the body's forward (-sin yaw, -cos yaw)
        let e = yaw(json!({"facing": "east"}));
        assert!((-e.sin() - 1.0).abs() < 1e-6 && (-e.cos()).abs() < 1e-6);
        assert!(parse_facing(&json!({"facing": "up"})).is_err());
        assert!(parse_facing(&json!({"facing": true})).is_err());
    }

    /// One pack id in two folders is refused, and the two folders are both named in the message.
    #[test]
    fn one_pack_id_in_two_folders_is_an_error() {
        let root = scratch("twin_ids");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let first = root.join("first_root");
        let second = root.join("second_root");
        let a = write_pack(&first, "twin_story", &format!(r#""story": {{"era": "future"}}, {}"#, start_at("twin_story")));
        let b = write_pack(&second, "twin_story", &format!(r#""story": {{"era": "future"}}, {}"#, start_at("twin_story")));
        let cfg = Config { core, mod_dirs: vec![a, b], mod_roots: Vec::new() };
        let errs = load(&cfg).err().expect("the same id twice is a problem");
        assert!(errs.iter().any(|e| e.contains("pack id 'twin_story' is in two folders")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    /// A pack that names a start scene but defines no scenes is refused at load (the cross-check in `validate`), even
    /// though a missing `scenes` container alone reads as empty.
    #[test]
    fn a_pack_with_a_start_scene_but_no_scenes_is_refused() {
        let root = scratch("startscene");
        let core = write_pack(&root, "core_pack", &format!(r#"{}, "start_scene": "opening""#, start_at("core_pack")));
        let cfg = Config { core, mod_dirs: Vec::new(), mod_roots: Vec::new() };
        let errs = load(&cfg).err().expect("a start scene that does not exist is refused");
        assert!(errs.iter().any(|e| e.contains("unknown scene 'opening'")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_story_pack_without_a_start_is_an_error() {
        let root = scratch("nostart");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let lost = write_pack(&root, "lost_story", r#""story": {"era": "future"}"#);
        let cfg = Config { core, mod_dirs: vec![lost], mod_roots: Vec::new() };
        let errs = load(&cfg).err().expect("the pack has a problem");
        assert!(errs.iter().any(|e| e.contains("needs \"start\"")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bad_era_speed_or_item_is_reported() {
        let root = scratch("badfields");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let era = write_pack(&root, "era_story", &format!(r#"{}, "story": {{"era": "nowhere"}}"#, start_at("era_story")));
        let speed = write_pack(
            &root,
            "speed_story",
            &format!(r#"{}, "story": {{"era": "future", "player": {{"speed": 0}}}}"#, start_at("speed_story")),
        );
        let item = write_pack(
            &root,
            "item_story",
            &format!(
                r#"{}, "story": {{"era": "future", "player": {{"start_items": ["ghost"]}}}}"#,
                start_at("item_story")
            ),
        );
        let cfg = Config { core, mod_dirs: vec![era, speed, item], mod_roots: Vec::new() };
        let errs = load(&cfg).err().expect("the pack has problems");
        assert!(errs.iter().any(|e| e.contains("unknown era 'nowhere'")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("\"speed\" must be a number above 0")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("unknown item 'ghost'")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_story_start_that_is_not_on_a_map_is_an_error() {
        let root = scratch("badstart");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let lost = write_pack(
            &root,
            "lost_story",
            r#""start": {"map": "nowhere_map", "spawn": "here"}, "story": {"era": "future"}"#,
        );
        let cfg = Config { core, mod_dirs: vec![lost], mod_roots: Vec::new() };
        let errs = load(&cfg).err().expect("the pack has a problem");
        assert!(errs.iter().any(|e| e.contains("story 'lost_story': start map 'nowhere_map' does not exist")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    /// Writes a prop file `props/<id>.bbmodel` under `dir`. Its text is not read at load time, so it is a stub.
    fn write_prop(dir: &Path, id: &str) {
        fs::create_dir_all(dir.join("props")).unwrap();
        fs::write(dir.join("props").join(format!("{id}.bbmodel")), "{}").unwrap();
    }

    /// Adds map `<id>_props` to `dir`: a 2x1 picture, its right pixel is the legend entry `prop_json` (red).
    fn write_prop_map(dir: &Path, id: &str, prop_json: &str) {
        let map = format!("{id}_props");
        let mut art = Image::gen_image_color(2, 1, Color::new(0.0, 1.0, 0.0, 1.0));
        art.get_image_data_mut()[1] = [255, 0, 0, 255];
        art.export_png(&dir.join("maps").join(format!("{map}.png")).to_string_lossy());
        fs::write(
            dir.join("maps").join(format!("{map}.json")),
            format!(
                r#"{{"schema": "lr.story.map/1", "id": "{map}", "name": "Props", "art": "{map}.png", "legend": {{"00ff00": {{"kind": "spawn", "name": "here"}}, "ff0000": {prop_json}}}}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_legend_prop_a_pack_ships_validates_and_its_yaw_is_read() {
        let root = scratch("props_ok");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let mod_dir = write_pack(&root, "crate_story", &format!(r#"{}, "story": {{"era": "future"}}"#, start_at("crate_story")));
        write_prop(&mod_dir, "crate_story_crate");
        write_prop_map(&mod_dir, "crate_story", r#"{"kind": "prop", "prop": "crate_story_crate", "yaw": 90}"#);
        let cfg = Config { core, mod_dirs: vec![mod_dir.clone()], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));

        assert_eq!(pack.props.get("crate_story_crate"), Some(&mod_dir.join("props").join("crate_story_crate.bbmodel")));
        let map = &pack.maps["crate_story_props"];
        assert!(
            map.legend.values().any(|k| matches!(k, Kind::Prop { prop, yaw, .. } if prop == "crate_story_crate" && *yaw == 90.0)),
            "{:?}",
            map.legend
        );
        assert!(matches!(map.grid.at(1, 0), Some(Kind::Prop { .. })), "the red pixel is the prop");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_prop_or_a_prop_without_the_pack_prefix_is_an_error() {
        let root = scratch("props_bad");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let mod_dir = write_pack(&root, "gone_story", &format!(r#"{}, "story": {{"era": "future"}}"#, start_at("gone_story")));
        write_prop_map(&mod_dir, "gone_story", r#"{"kind": "prop", "prop": "gone_story_ghost"}"#);
        write_prop(&mod_dir, "crate");
        let cfg = Config { core, mod_dirs: vec![mod_dir], mod_roots: Vec::new() };
        let errs = load(&cfg).err().expect("the pack has problems");
        assert!(errs.iter().any(|e| e.contains("map 'gone_story_props': unknown prop 'gone_story_ghost'")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("props id 'crate' must start with 'gone_story_'")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_prop_kind_takes_an_optional_yaw_in_degrees_and_colour() {
        use serde_json::json;
        let plain = parse_kind(&json!({"kind": "prop", "prop": "p"})).unwrap();
        assert_eq!(plain, Kind::Prop { prop: "p".into(), yaw: 0.0, color: PROP_COLOUR, use_: None });
        let turned = parse_kind(&json!({"kind": "prop", "prop": "p", "yaw": -45.5, "color": "#112233"})).unwrap();
        assert_eq!(turned, Kind::Prop { prop: "p".into(), yaw: -45.5, color: [0x11, 0x22, 0x33], use_: None });
        assert!(parse_kind(&json!({"kind": "prop", "prop": "p", "yaw": "north"})).unwrap_err().contains("\"yaw\""));
        assert!(parse_kind(&json!({"kind": "prop"})).unwrap_err().contains("\"prop\""));
    }

    /// Adds map `core_pack_world` (5x1) to the core pack `core`: spawn "here" (0,0), NPC Ann with routine "walker"
    /// (1,0), the usable prop "bench" (2,0), NPC Bob (3,0), spawn "well" (4,0). `routines_body` is the inside of the
    /// routines object; `map_extra` goes before the legend (for example `"outdoor": true,`); `use_scene` is the scene
    /// the bench runs for the player.
    fn write_routine_world(core: &Path, routines_body: &str, map_extra: &str, use_scene: &str) {
        fs::create_dir_all(core.join("scenes")).unwrap();
        fs::write(
            core.join("scenes").join("s.json"),
            r#"{"schema": "lr.story.scenes/1", "scenes": {"talk": {"lines": ["Hi."]}}}"#,
        )
        .unwrap();
        write_prop(core, "bench");
        fs::create_dir_all(core.join("routines")).unwrap();
        fs::write(
            core.join("routines").join("r.json"),
            format!(r#"{{"schema": "lr.story.routines/1", "routines": {{{routines_body}}}}}"#),
        )
        .unwrap();
        let mut art = Image::gen_image_color(5, 1, Color::new(0.0, 1.0, 0.0, 1.0));
        {
            let d = art.get_image_data_mut();
            d[1] = [255, 0, 0, 255];
            d[2] = [0, 0, 255, 255];
            d[3] = [255, 255, 0, 255];
            d[4] = [255, 255, 255, 255];
        }
        art.export_png(&core.join("maps").join("core_pack_world.png").to_string_lossy());
        let map = r#"{"schema": "lr.story.map/1", "id": "core_pack_world", "name": "World", "art": "core_pack_world.png", EXTRA "legend": {
            "00ff00": {"kind": "spawn", "name": "here"},
            "ff0000": {"kind": "npc", "name": "Ann", "scene": "talk", "routine": "walker"},
            "0000ff": {"kind": "prop", "prop": "bench", "use": {"action": "sit", "scene": "SCENE"}},
            "ffff00": {"kind": "npc", "name": "Bob", "scene": "talk"},
            "ffffff": {"kind": "spawn", "name": "well"}}}"#
            .replace("EXTRA", map_extra)
            .replace("SCENE", use_scene);
        fs::write(core.join("maps").join("core_pack_world.json"), map).unwrap();
    }

    const GOOD_ROUTINE: &str = r#""walker": {"steps": [
        {"go": "well", "do": "idle", "secs": 4},
        {"go": "prop:2,0", "do": "sit", "secs": 10},
        {"do": "chat", "with": "Bob", "secs": 3},
        {"do": "wander", "radius": 2}], "shelter": "here", "flee": "well"}"#;

    #[test]
    fn routines_props_and_weather_load_from_a_pack() {
        let root = scratch("routine_ok");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        write_routine_world(&core, GOOD_ROUTINE, r#""outdoor": true, "weather": {"kind": "rain"},"#, "talk");
        let pack = load(&alone(&core)).unwrap_or_else(|errs| panic!("{errs:?}"));
        let r = &pack.routines["walker"];
        assert!(r.looping, "loop defaults to true");
        assert_eq!((r.shelter.as_deref(), r.flee.as_deref()), (Some("here"), Some("well")));
        assert_eq!(r.steps.len(), 4);
        assert_eq!(r.steps[0], RoutineStep { go: Some("well".into()), do_: NpcDo::Idle, secs: 4.0, with: None, radius: DEFAULT_WANDER_RADIUS });
        assert_eq!((r.steps[1].go.as_deref(), r.steps[1].do_), (Some("prop:2,0"), NpcDo::Sit));
        assert_eq!((r.steps[2].do_, r.steps[2].with.as_deref(), r.steps[2].secs), (NpcDo::Chat, Some("Bob"), 3.0));
        assert_eq!((r.steps[3].go.clone(), r.steps[3].do_, r.steps[3].radius, r.steps[3].secs), (None, NpcDo::Wander, 2.0, DEFAULT_STEP_SECS));
        let map = &pack.maps["core_pack_world"];
        assert!(matches!(map.grid.at(1, 0), Some(Kind::Npc { routine: Some(r), .. }) if r == "walker"));
        assert!(matches!(map.grid.at(3, 0), Some(Kind::Npc { routine: None, .. })));
        match map.grid.at(2, 0) {
            Some(Kind::Prop { use_: Some(u), .. }) => {
                assert_eq!(u.action, PropAction::Sit);
                assert_eq!(u.text, "Sit");
                assert_eq!(u.scene.as_deref(), Some("talk"));
                assert_eq!(u.seat_height, DEFAULT_SEAT_HEIGHT);
            }
            other => panic!("{other:?}"),
        }
        let w = map.weather.as_ref().expect("the map has weather");
        assert_eq!((w.kind, w.intensity, w.every_secs), (WeatherKind::Rain, DEFAULT_WEATHER_INTENSITY, DEFAULT_WEATHER_EVERY_SECS));
        assert!(w.changes.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn routine_targets_and_references_are_validated_against_the_npcs_map() {
        let cases: [(&str, &str, &str, &str); 8] = [
            (r#""walker": {"steps": [{"go": "nowhere"}]}"#, "", "talk", "routine 'walker' on map 'core_pack_world', step 1: \"go\" spawn 'nowhere' is not on this map"),
            (r#""walker": {"steps": [{"go": "prop:1,0"}]}"#, "", "talk", "step 1: \"go\" target prop:1,0 holds no prop"),
            (r#""walker": {"steps": [{"go": "prop:9,9"}]}"#, "", "talk", "step 1: \"go\" target prop:9,9 holds no prop"),
            (r#""walker": {"steps": [{"go": "prop:x,y"}]}"#, "", "talk", "is not a prop target"),
            (r#""walker": {"steps": [{"do": "chat", "with": "Zed"}]}"#, "", "talk", "\"with\" NPC 'Zed' is not on this map"),
            (r#""walker": {"steps": [{}], "shelter": "nowhere"}"#, "", "talk", "\"shelter\" spawn 'nowhere' is not on this map"),
            (r#""walker": {"steps": [{}], "flee": "nowhere"}"#, "", "talk", "\"flee\" spawn 'nowhere' is not on this map"),
            (r#""other": {"steps": [{}]}"#, "", "talk", "unknown routine 'walker'"),
        ];
        for (i, (body, extra, scene, want)) in cases.iter().enumerate() {
            let root = scratch(&format!("routine_bad_{i}"));
            let core = write_pack(&root, "core_pack", &start_at("core_pack"));
            write_routine_world(&core, body, extra, scene);
            let errs = load(&alone(&core)).err().unwrap_or_else(|| panic!("case {i} should fail"));
            assert!(errs.iter().any(|e| e.contains(want)), "case {i}: {errs:?}");
            let _ = fs::remove_dir_all(&root);
        }
        // A prop's use scene must exist too.
        let root = scratch("routine_bad_use_scene");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        write_routine_world(&core, GOOD_ROUTINE, "", "ghost");
        let errs = load(&alone(&core)).err().expect("a missing use scene is an error");
        assert!(errs.iter().any(|e| e.contains("prop 'bench' use: unknown scene 'ghost'")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
        // Weather on an indoor map is a load error.
        let root = scratch("routine_indoor_weather");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        write_routine_world(&core, GOOD_ROUTINE, r#""weather": {"kind": "rain"},"#, "talk");
        let errs = load(&alone(&core)).err().expect("indoor weather is an error");
        assert!(errs.iter().any(|e| e.contains("\"weather\" needs \"outdoor\": true")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mods_routines_need_the_pack_prefix_unless_they_override() {
        let root = scratch("routine_mod");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        write_routine_world(&core, GOOD_ROUTINE, "", "talk");
        let m = write_pack(&root, "mod_a", r#""overrides": ["routines/walker"]"#);
        fs::create_dir_all(m.join("routines")).unwrap();
        fs::write(
            m.join("routines").join("r.json"),
            r#"{"schema": "lr.story.routines/1", "routines": {"walker": {"steps": [{"do": "look"}]}, "mod_a_idle": {"loop": false, "steps": [{}]}}}"#,
        )
        .unwrap();
        let cfg = Config { core: core.clone(), mod_dirs: vec![m.clone()], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert_eq!(pack.routines["walker"].steps.len(), 1, "the override replaced the core routine");
        assert!(!pack.routines["mod_a_idle"].looping);

        fs::write(m.join("pack.json"), r#"{"schema": "lr.pack/1", "id": "mod_a", "name": "x"}"#).unwrap();
        let errs = load(&cfg).err().expect("without the override the ids are errors");
        assert!(errs.iter().any(|e| e.contains("routines id 'walker' must start with 'mod_a_'")), "{errs:?}");
        fs::write(m.join("pack.json"), r#"{"schema": "lr.pack/1", "id": "mod_a", "name": "x", "overrides": ["routines/ghost"]}"#).unwrap();
        let errs = load(&cfg).err().expect("an override of nothing is refused");
        assert!(errs.iter().any(|e| e.contains("overrides \"routines/ghost\"")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn routine_definitions_reject_bad_data() {
        use serde_json::json;
        let ok = parse_routine(&json!({"steps": [{}]})).unwrap();
        assert_eq!(ok.steps[0], RoutineStep { go: None, do_: NpcDo::Idle, secs: DEFAULT_STEP_SECS, with: None, radius: DEFAULT_WANDER_RADIUS });
        assert!(ok.looping && ok.shelter.is_none() && ok.flee.is_none());
        for (bad, want) in [
            (json!({}), "at least one step"),
            (json!({"steps": []}), "at least one step"),
            (json!({"steps": "walk"}), "\"steps\" must be a list"),
            (json!("walk"), "must be an object"),
            (json!({"steps": ["walk"]}), "step 1: a step must be an object"),
            (json!({"steps": [{"secs": 0}]}), "\"secs\" must be above 0"),
            (json!({"steps": [{}, {"secs": -2}]}), "step 2: \"secs\" must be above 0"),
            (json!({"steps": [{"secs": "long"}]}), "\"secs\" must be a number"),
            (json!({"steps": [{"do": "dance"}]}), "unknown \"do\" action 'dance'"),
            (json!({"steps": [{"do": "chat"}]}), "chat step needs \"with\""),
            (json!({"steps": [{"radius": 0}]}), "\"radius\" must be above 0"),
            (json!({"steps": [{"go": 3}]}), "\"go\" must be text"),
            (json!({"loop": "yes", "steps": [{}]}), "\"loop\" must be true or false"),
            (json!({"steps": [{}], "shelter": 4}), "\"shelter\" must be text"),
        ] {
            let e = parse_routine(&bad).unwrap_err();
            assert!(e.contains(want), "{bad}: {e}");
        }
        for name in NpcDo::NAMES {
            assert!(NpcDo::parse(name).is_ok(), "{name}");
        }
        assert_eq!(parse_prop_target("well"), None);
        assert_eq!(parse_prop_target("prop:3, 4"), Some(Ok((3, 4))));
        assert!(matches!(parse_prop_target("prop:3"), Some(Err(_))));
    }

    #[test]
    fn prop_use_and_npc_routine_fields_parse() {
        use serde_json::json;
        let draw = parse_kind(&json!({"kind": "prop", "prop": "p", "use": {"action": "draw_water"}})).unwrap();
        let Kind::Prop { use_: Some(u), .. } = &draw else { panic!("{draw:?}") };
        assert_eq!(*u, PropUse { action: PropAction::DrawWater, text: "Draw water".into(), scene: None, seat_height: DEFAULT_SEAT_HEIGHT });
        let full = parse_kind(&json!({"kind": "prop", "prop": "p", "use": {"action": "sleep", "text": "Rest", "scene": "nap", "seat_height": 0.3}})).unwrap();
        let Kind::Prop { use_: Some(u), .. } = &full else { panic!("{full:?}") };
        assert_eq!(*u, PropUse { action: PropAction::Sleep, text: "Rest".into(), scene: Some("nap".into()), seat_height: 0.3 });
        for name in PropAction::NAMES {
            let a = PropAction::parse(name).unwrap();
            assert_eq!(a.name(), name);
            assert!(!a.default_text().is_empty());
        }
        for (bad, want) in [
            (json!({"action": "dance"}), "unknown prop action 'dance'"),
            (json!({}), "use: missing \"action\""),
            (json!({"action": "sit", "text": 3}), "\"text\" must be text"),
            (json!({"action": "sit", "text": " "}), "must not be empty"),
            (json!({"action": "sit", "scene": 3}), "\"scene\" must be text"),
            (json!({"action": "sit", "seat_height": -1}), "\"seat_height\" must be between"),
            (json!({"action": "sit", "seat_height": 3}), "\"seat_height\" must be between"),
            (json!({"action": "sit", "seat_height": "low"}), "\"seat_height\" must be a number"),
        ] {
            let e = parse_kind(&json!({"kind": "prop", "prop": "p", "use": bad})).unwrap_err();
            assert!(e.contains(want), "{bad}: {e}");
        }
        assert!(parse_kind(&json!({"kind": "prop", "prop": "p", "use": "sit"})).unwrap_err().contains("\"use\" must be an object"));

        let npc = parse_kind(&json!({"kind": "npc", "name": "A", "scene": "a", "routine": "baker_day"})).unwrap();
        assert!(matches!(npc, Kind::Npc { routine: Some(r), .. } if r == "baker_day"));
        for bad in [json!("../x"), json!(""), json!(4)] {
            let e = parse_kind(&json!({"kind": "npc", "name": "A", "scene": "a", "routine": bad})).unwrap_err();
            assert!(e.contains("\"routine\""), "{bad}: {e}");
        }
    }

    #[test]
    fn map_weather_parses_defaults_changes_and_rejects_bad_data() {
        use serde_json::json;
        let none = parse_map_env(&json!({"outdoor": true}), &walls(3.0)).unwrap();
        assert!(none.weather.is_none());
        let basic = parse_map_env(&json!({"outdoor": true, "weather": {"kind": "snow"}}), &walls(3.0)).unwrap();
        assert_eq!(
            basic.weather,
            Some(WeatherDef { kind: WeatherKind::Snow, intensity: 0.6, changes: Vec::new(), every_secs: 90.0 })
        );
        let full = parse_map_env(
            &json!({"outdoor": true, "weather": {"kind": "clear", "intensity": 0.25, "every_secs": 30,
                "changes": [{"kind": "rain", "weight": 2}, {"kind": "fog", "weight": 0.5}]}}),
            &walls(3.0),
        )
        .unwrap();
        assert_eq!(
            full.weather,
            Some(WeatherDef {
                kind: WeatherKind::Clear,
                intensity: 0.25,
                changes: vec![(WeatherKind::Rain, 2.0), (WeatherKind::Fog, 0.5)],
                every_secs: 30.0,
            })
        );
        for name in WeatherKind::NAMES {
            assert_eq!(WeatherKind::parse(name).unwrap().name(), name);
        }
        for (bad, want) in [
            (json!({"weather": {"kind": "rain"}}), "needs \"outdoor\": true"),
            (json!({"outdoor": true, "weather": "rain"}), "\"weather\" must be an object"),
            (json!({"outdoor": true, "weather": {}}), "weather: missing \"kind\""),
            (json!({"outdoor": true, "weather": {"kind": "hail"}}), "unknown weather kind 'hail'"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "intensity": 1.5}}), "\"intensity\" must be between 0 and 1"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "intensity": -0.1}}), "\"intensity\" must be between 0 and 1"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "intensity": "high"}}), "\"intensity\" must be a number"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "every_secs": 0}}), "\"every_secs\" must be above 0"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "changes": "snow"}}), "\"changes\" must be a list"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "changes": ["snow"]}}), "weather change 1: must be an object"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "changes": [{"kind": "snow"}]}}), "missing \"weight\""),
            (json!({"outdoor": true, "weather": {"kind": "rain", "changes": [{"kind": "snow", "weight": 0}]}}), "\"weight\" must be above 0"),
            (json!({"outdoor": true, "weather": {"kind": "rain", "changes": [{"kind": "hail", "weight": 1}]}}), "unknown weather kind 'hail'"),
        ] {
            let e = parse_map_env(&bad, &walls(3.0)).unwrap_err();
            assert!(e.contains(want), "{bad}: {e}");
        }
    }

    #[test]
    fn the_weather_op_parses_and_validates_intensity() {
        use serde_json::json;
        assert_eq!(
            parse_op(&json!({"op": "weather", "kind": "storm"})).unwrap(),
            Op::Weather { kind: WeatherKind::Storm, intensity: None }
        );
        assert_eq!(
            parse_op(&json!({"op": "weather", "kind": "leaves", "intensity": 0.9})).unwrap(),
            Op::Weather { kind: WeatherKind::Leaves, intensity: Some(0.9) }
        );
        for bad in [
            json!({"op": "weather"}),
            json!({"op": "weather", "kind": "hail"}),
            json!({"op": "weather", "kind": "rain", "intensity": 2}),
            json!({"op": "weather", "kind": "rain", "intensity": "much"}),
        ] {
            assert!(parse_op(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_npc_takes_an_optional_body_and_skin() {
        use serde_json::json;
        let plain = parse_kind(&json!({"kind": "npc", "name": "Felix", "scene": "felix"})).unwrap();
        assert_eq!(
            plain,
            Kind::Npc {
                name: "Felix".into(),
                scene: "felix".into(),
                color: NPC_COLOUR,
                body: BodyChoice::Blocky,
                skin: None,
                routine: None,
            }
        );
        let slim = parse_kind(&json!({
            "kind": "npc", "name": "Renata", "scene": "renata", "body": "blocky_slim", "skin": "renata_atlas"
        }))
        .unwrap();
        assert_eq!(
            slim,
            Kind::Npc {
                name: "Renata".into(),
                scene: "renata".into(),
                color: NPC_COLOUR,
                body: BodyChoice::BlockySlim,
                skin: Some("renata_atlas".into()),
                routine: None,
            }
        );
        let bad_body = parse_kind(&json!({"kind": "npc", "name": "X", "scene": "x", "body": "robot"})).unwrap_err();
        assert!(bad_body.contains("unknown body 'robot'"), "{bad_body}");
        for skin in ["../x", "/abs/x", "x.png", "a\\b", "C:x", "a/b", ""] {
            let e = parse_kind(&json!({"kind": "npc", "name": "X", "scene": "x", "skin": skin})).unwrap_err();
            assert!(e.contains("\"skin\""), "{skin}: {e}");
        }
        assert!(parse_kind(&json!({"kind": "npc", "name": "X", "scene": "x", "skin": 3})).unwrap_err().contains("\"skin\""));
        assert!(parse_kind(&json!({"kind": "npc", "name": "X", "scene": "x", "body": 3})).unwrap_err().contains("\"body\""));
    }

    #[test]
    fn a_story_player_takes_an_optional_body() {
        use serde_json::json;
        let plain = parse_story(&json!({"era": "future"}), "p").unwrap();
        assert_eq!(plain.player.body, BodyChoice::Blocky);
        let slim = parse_story(&json!({"era": "future", "player": {"body": "blocky_slim"}}), "p").unwrap();
        assert_eq!(slim.player.body, BodyChoice::BlockySlim);
        let bad = parse_story(&json!({"era": "future", "player": {"body": "robot"}}), "p").unwrap_err();
        assert!(bad.contains("unknown body 'robot'"), "{bad}");
    }

    /// The pack set with only `core` in it.
    fn alone(core: &Path) -> Config {
        Config { core: core.to_path_buf(), mod_dirs: Vec::new(), mod_roots: Vec::new() }
    }

    /// Writes a `side` x `side` skin, `skins/<name>.png`, in the pack folder `dir`.
    fn write_skin(dir: &Path, name: &str, side: u16) {
        fs::create_dir_all(dir.join("skins")).unwrap();
        let img = Image::gen_image_color(side, side, Color::new(0.5, 0.4, 0.3, 1.0));
        img.export_png(&dir.join("skins").join(format!("{name}.png")).to_string_lossy());
    }

    #[test]
    fn a_mod_skin_name_carries_its_pack_prefix() {
        let root = scratch("skin_prefix");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let good = write_pack(&root, "beta_mod", r#""version": "0.1.0""#);
        write_skin(&good, "beta_mod_linda", SKIN_SIDE);
        let cfg = Config { core: core.clone(), mod_dirs: vec![good], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert!(pack.skins.contains_key("beta_mod_linda"), "{:?}", pack.skins.keys().collect::<Vec<_>>());

        let bare = write_pack(&root, "gamma_mod", r#""version": "0.1.0""#);
        write_skin(&bare, "linda", SKIN_SIDE);
        let cfg = Config { core, mod_dirs: vec![bare], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("skins id 'linda' must start with 'gamma_mod_'")), "{errs:?}");
    }

    #[test]
    fn a_skin_must_be_a_64_by_64_png() {
        let root = scratch("skin_size");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));

        write_skin(&core, "small", 32);
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("skin 'small' is 32x32; a skin must be 64x64")), "{errs:?}");
        fs::remove_file(core.join("skins").join("small.png")).unwrap();

        fs::write(core.join("skins").join("broken.png"), b"not a png").unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("skin 'broken'")), "{errs:?}");
        fs::remove_file(core.join("skins").join("broken.png")).unwrap();

        write_skin(&core, "tall", SKIN_SIDE);
        let pack = load(&alone(&core)).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert!(pack.skins.contains_key("tall"));
    }

    #[test]
    fn an_npc_naming_an_unknown_skin_is_an_error() {
        let root = scratch("npc_skin");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        fs::create_dir_all(core.join("scenes")).unwrap();
        fs::write(
            core.join("scenes").join("talk.json"),
            r#"{"schema": "lr.story.scenes/1", "scenes": {"talk": {"lines": ["Hi."]}}}"#,
        )
        .unwrap();
        fs::write(
            core.join("maps").join("core_pack_crowd.json"),
            r#"{"schema": "lr.story.map/1", "name": "Crowd", "art": "core_pack_map.png", "legend": {"00ff00": {"kind": "npc", "name": "Ghost", "scene": "talk", "skin": "ghost"}}}"#,
        )
        .unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unknown skin 'ghost'")), "{errs:?}");

        write_skin(&core, "ghost", SKIN_SIDE);
        let pack = load(&alone(&core)).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert!(pack.skins.contains_key("ghost"));
    }

    // ---- strict loader rules ----

    #[test]
    fn wrong_typed_optional_fields_are_errors_naming_the_field() {
        use serde_json::json;
        let e = parse_op(&json!({"op": "set_flag", "flag": "f", "value": "false"})).unwrap_err();
        assert!(e.contains("\"value\""), "{e}");
        assert_eq!(parse_op(&json!({"op": "set_flag", "flag": "f", "value": false})).unwrap(), Op::SetFlag { flag: "f".into(), value: false });
        assert_eq!(parse_op(&json!({"op": "set_flag", "flag": "f"})).unwrap(), Op::SetFlag { flag: "f".into(), value: true });

        let e = parse_scene(&json!({"once": "yes", "lines": []})).unwrap_err();
        assert!(e.contains("\"once\""), "{e}");
        let e = parse_scene(&json!({"next": 3})).unwrap_err();
        assert!(e.contains("\"next\""), "{e}");
        let e = parse_scene(&json!({"lines": [{"speaker": 4, "text": "x"}]})).unwrap_err();
        assert!(e.contains("\"speaker\""), "{e}");
        let e = parse_scene(&json!({"choices": [{"text": "x", "next": 1}]})).unwrap_err();
        assert!(e.contains("\"next\""), "{e}");

        let e = parse_kind(&json!({"kind": "trigger", "scene": "s", "once": "no"})).unwrap_err();
        assert!(e.contains("\"once\""), "{e}");
        let e = parse_kind(&json!({"kind": "wall", "height": "tall"})).unwrap_err();
        assert!(e.contains("\"height\""), "{e}");
        let e = parse_kind(&json!({"kind": "door", "if": {"flag": "f"}, "locked": 5})).unwrap_err();
        assert!(e.contains("\"locked\""), "{e}");
        let e = parse_kind(&json!({"kind": "wall", "color": 5})).unwrap_err();
        assert!(e.contains("\"color\""), "{e}");
        assert!(parse_kind(&json!("wall")).is_err());

        let e = parse_story(&json!({"era": "future", "player": {"speed": "fast"}}), "p").unwrap_err();
        assert!(e.contains("\"speed\""), "{e}");
        let e = parse_story(&json!({"era": "future", "player": "Kit"}), "p").unwrap_err();
        assert!(e.contains("\"player\""), "{e}");
        let e = parse_story(&json!({"era": "future", "player": {"name": 3}}), "p").unwrap_err();
        assert!(e.contains("\"name\""), "{e}");
        let e = parse_story(&json!({"era": "future", "player": {"role": []}}), "p").unwrap_err();
        assert!(e.contains("\"role\""), "{e}");
        let e = parse_story(&json!({"era": "future", "order": "1"}), "p").unwrap_err();
        assert!(e.contains("\"order\""), "{e}");
        let e = parse_story(&json!({"era": "future", "multiplayer": "yes"}), "p").unwrap_err();
        assert!(e.contains("\"multiplayer\""), "{e}");
        let e = parse_story(&json!({"era": "future", "title": 3}), "p").unwrap_err();
        assert!(e.contains("\"title\""), "{e}");

        let e = parse_item(&json!({"name": 7}), "k").unwrap_err();
        assert!(e.contains("\"name\""), "{e}");
        let e = parse_item(&json!({"description": []}), "k").unwrap_err();
        assert!(e.contains("\"description\""), "{e}");
        assert!(parse_item(&json!("Key"), "k").is_err());
        assert_eq!(parse_item(&json!({}), "k").unwrap(), Item::plain("k", ""));
    }

    #[test]
    fn wrong_shaped_containers_and_bare_string_scenes_are_errors() {
        use serde_json::json;
        assert!(parse_scene(&json!({"lines": "Hello"})).unwrap_err().contains("\"lines\""));
        assert!(parse_scene(&json!({"lines": {"text": "x"}})).unwrap_err().contains("\"lines\""));
        assert!(parse_scene(&json!({"lines": [3]})).unwrap_err().contains("line"));
        assert!(parse_scene(&json!({"choices": {"text": "x"}})).unwrap_err().contains("\"choices\""));
        assert!(parse_scene(&json!("Hello")).unwrap_err().contains("scene must be an object"));
        assert!(parse_scene(&json!({"lines": ["a", {"speaker": "B", "text": "b"}]})).is_ok());
        assert!(container(&json!({"scenes": []}), "scenes").is_err());
        assert!(container(&json!({"items": "x"}), "items").is_err());
        assert!(container(&json!({"legend": 3}), "legend").is_err());
        assert_eq!(container(&json!({}), "scenes").unwrap().len(), 0);
        assert_eq!(container(&json!({"scenes": {"a": {}}}), "scenes").unwrap().len(), 1);
    }

    #[test]
    fn wrong_shaped_files_in_a_pack_are_reported() {
        let root = scratch("shapes");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        fs::create_dir_all(core.join("scenes")).unwrap();
        fs::write(core.join("scenes").join("a.json"), r#"{"schema": "lr.story.scenes/1", "scenes": []}"#).unwrap();
        fs::write(core.join("scenes").join("b.json"), r#"{"schema": "lr.story.scenes/1", "scenes": {"b": "Hello"}}"#).unwrap();
        fs::create_dir_all(core.join("items")).unwrap();
        fs::write(core.join("items").join("i.json"), r#"{"schema": "lr.story.items/1", "items": {"k": {"name": 1}}}"#).unwrap();
        fs::write(
            core.join("maps").join("core_pack_bad.json"),
            r#"{"schema": "lr.story.map/1", "name": "Bad", "art": "core_pack_map.png", "legend": []}"#,
        )
        .unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("a.json") && e.contains("\"scenes\" must be an object")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("scene 'b'") && e.contains("must be an object")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("item 'k'") && e.contains("\"name\"")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("\"legend\" must be an object")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_extensions_match_in_any_case() {
        let root = scratch("ext_case");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        fs::create_dir_all(core.join("scenes")).unwrap();
        fs::write(core.join("scenes").join("TALK.JSON"), r#"{"schema": "lr.story.scenes/1", "scenes": {"talk": {"lines": ["Hi."]}}}"#).unwrap();
        write_prop(&core, "core_pack_crate");
        fs::rename(core.join("props").join("core_pack_crate.bbmodel"), core.join("props").join("core_pack_crate.BBMODEL")).unwrap();
        write_skin(&core, "core_pack_skin", SKIN_SIDE);
        fs::rename(core.join("skins").join("core_pack_skin.png"), core.join("skins").join("core_pack_skin.PNG")).unwrap();
        let pack = load(&alone(&core)).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert!(pack.scenes.contains_key("talk"));
        assert!(pack.props.contains_key("core_pack_crate"));
        assert!(pack.skins.contains_key("core_pack_skin"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mod_cannot_move_the_start_with_an_override() {
        let root = scratch("start_override");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let plain = write_pack(&root, "plain_mod", &format!(r#""overrides": ["start"], {}"#, start_at("plain_mod")));
        let story = write_pack(
            &root,
            "story_mod",
            &format!(r#""overrides": ["start"], {}, "story": {{"era": "future"}}"#, start_at("story_mod")),
        );
        let cfg = Config { core, mod_dirs: vec![plain, story], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("pack 'plain_mod' lists \"start\" in its overrides")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("pack 'story_mod' lists \"start\" in its overrides")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("pack 'plain_mod' sets \"start\"")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    fn write_item(dir: &Path, file: &str, id: &str) {
        fs::create_dir_all(dir.join("items")).unwrap();
        fs::write(
            dir.join("items").join(file),
            format!(r#"{{"schema": "lr.story.items/1", "items": {{"{id}": {{"name": "N"}}}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn overriding_an_earlier_packs_id_works_and_overriding_nothing_does_not() {
        let root = scratch("override_rules");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        write_item(&core, "items.json", "lamp");

        // A real override of the core's item: loads, and needs no prefix.
        let good = write_pack(&root, "good_mod", r#""overrides": ["items/lamp"]"#);
        write_item(&good, "items.json", "lamp");
        let cfg = Config { core: core.clone(), mod_dirs: vec![good], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert_eq!(pack.items["lamp"].name, "N");

        // An unprefixed id with an override that replaces nothing: both the override and the prefix rule fire.
        let bad = write_pack(&root, "bad_mod", r#""overrides": ["items/torch"]"#);
        write_item(&bad, "items.json", "torch");
        let cfg = Config { core: core.clone(), mod_dirs: vec![bad], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("overrides \"items/torch\", but no earlier pack defines it")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("items id 'torch' must start with 'bad_mod_'")), "{errs:?}");

        // A prefixed id the pack only defines itself is also not an override.
        let own = write_pack(&root, "own_mod", r#""overrides": ["items/own_mod_key"]"#);
        write_item(&own, "items.json", "own_mod_key");
        let cfg = Config { core: core.clone(), mod_dirs: vec![own], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("but no earlier pack defines it")), "{errs:?}");
        assert!(!errs.iter().any(|e| e.contains("must start with")), "{errs:?}");

        // Nobody defines it at all.
        let none = write_pack(&root, "none_mod", r#""overrides": ["items/ghost"]"#);
        let cfg = Config { core: core.clone(), mod_dirs: vec![none], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("\"items/ghost\", which no pack defines")), "{errs:?}");

        // A later pack defines it: the message says so.
        let early = write_pack(&root, "aaa_mod", r#""overrides": ["items/zzz_mod_key"]"#);
        let late = write_pack(&root, "zzz_mod", r#""version": "0.1.0""#);
        write_item(&late, "items.json", "zzz_mod_key");
        let cfg = Config { core, mod_dirs: vec![late, early], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("defined by a later pack 'zzz_mod'; packs merge in id order")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pack_replacing_its_own_scene_from_an_overrides_file_is_a_valid_override() {
        let root = scratch("own_replace");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let m = write_pack(&root, "own_mod", r#""overrides": ["scenes/own_mod_hi"]"#);
        fs::create_dir_all(m.join("scenes")).unwrap();
        fs::write(m.join("scenes").join("a.json"), r#"{"schema": "lr.story.scenes/1", "scenes": {"own_mod_hi": {"lines": ["one"]}}}"#).unwrap();
        fs::write(m.join("scenes").join("a_overrides.json"), r#"{"schema": "lr.story.scenes/1", "scenes": {"own_mod_hi": {"lines": ["two"]}}}"#).unwrap();
        let cfg = Config { core, mod_dirs: vec![m], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert_eq!(pack.scenes["own_mod_hi"].lines[0].text, "two");
        let _ = fs::remove_dir_all(&root);
    }

    /// Bytes that start like a PNG of the declared size (signature and IHDR) and stop there: enough for the header
    /// check, and not decodable.
    fn png_header_only(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn png_sizes_come_from_the_header() {
        assert_eq!(png_size(&png_header_only(70000, 3)), Some((70000, 3)));
        assert_eq!(png_size(&png_header_only(65537, 1)), Some((65537, 1)));
        assert_eq!(png_size(b"not a png at all, really not"), None);
        assert_eq!(png_size(&png_header_only(1, 1)[..20]), None);
        let mut wrong = png_header_only(1, 1);
        wrong[12] = b'X';
        assert_eq!(png_size(&wrong), None);
    }

    #[test]
    fn huge_pictures_are_refused_by_their_real_header_size() {
        let root = scratch("png_huge");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let maps = core.join("maps");

        // A mask 65537 wide would wrap to 1 in a u16; the header size is what counts.
        fs::write(maps.join("core_pack_wide-mask.png"), png_header_only(65537, 1)).unwrap();
        fs::write(
            maps.join("core_pack_wide.json"),
            r#"{"schema": "lr.story.map/1", "name": "Wide", "art": "core_pack_map.png", "mask": "core_pack_wide-mask.png", "legend": {}}"#,
        )
        .unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("mask is 65537x1; each side must be 1-512")), "{errs:?}");
        fs::remove_file(maps.join("core_pack_wide.json")).unwrap();

        // The same for the floor picture.
        fs::write(maps.join("core_pack_big.png"), png_header_only(65537, 1)).unwrap();
        fs::write(
            maps.join("core_pack_big.json"),
            r#"{"schema": "lr.story.map/1", "name": "Big", "art": "core_pack_big.png", "mask": "core_pack_map.png", "legend": {}}"#,
        )
        .unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("art is 65537x1; each side must be at most")), "{errs:?}");
        fs::remove_file(maps.join("core_pack_big.json")).unwrap();

        // A skin whose header says 64 wide and very tall.
        fs::create_dir_all(core.join("skins")).unwrap();
        fs::write(core.join("skins").join("core_pack_tall.png"), png_header_only(64, 65600)).unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("skin 'core_pack_tall' is 64x65600; a skin must be 64x64")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_skin_file_name_must_be_a_plain_name() {
        assert!(is_plain_name("renata_atlas-2"));
        for bad in ["", "a b", "a.b", "a:b"] {
            assert!(!is_plain_name(bad), "{bad}");
        }
        let root = scratch("skin_name");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        write_skin(&core, "has space", SKIN_SIDE);
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("skin file name 'has space'")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn items_can_have_a_use_and_a_consumable_flag() {
        use serde_json::json;
        let i = parse_item(&json!({"use": {"heal": 12}}), "potion").unwrap();
        assert_eq!((i.use_effect, i.consumable), (Some(ItemUse::Heal(12)), true), "consumable by default when used");
        let i = parse_item(&json!({"use": {"damage": 4}, "consumable": false}), "bomb").unwrap();
        assert_eq!((i.use_effect, i.consumable), (Some(ItemUse::Damage(4)), false));
        let i = parse_item(&json!({"name": "Key"}), "key").unwrap();
        assert_eq!((i.use_effect, i.consumable), (None, false));
        for bad in [
            json!({"use": 5}),
            json!({"use": {}}),
            json!({"use": {"heal": 1, "damage": 1}}),
            json!({"use": {"heal": 0}}),
            json!({"use": {"heal": "x"}}),
            json!({"use": {"heal": 2}, "consumable": 1}),
        ] {
            assert!(parse_item(&bad, "x").is_err(), "{bad}");
        }
    }

    #[test]
    fn item_stacks_default_and_validate() {
        use serde_json::json;
        assert_eq!(parse_item(&json!({}), "k").unwrap().stack, 1);
        assert_eq!(parse_item(&json!({"use": {"heal": 5}}), "k").unwrap().stack, 9);
        assert_eq!(parse_item(&json!({"use": {"heal": 5}, "stack": 3}), "k").unwrap().stack, 3);
        assert_eq!(parse_item(&json!({"stack": 99}), "k").unwrap().stack, 99);
        for bad in [json!({"stack": 0}), json!({"stack": 100}), json!({"stack": -1}), json!({"stack": "9"}), json!({"stack": 1.5})] {
            let e = parse_item(&bad, "k").unwrap_err();
            assert!(e.contains("\"stack\""), "{bad}: {e}");
        }
    }

    #[test]
    fn item_icons_must_be_plain_png_names() {
        use serde_json::json;
        let i = parse_item(&json!({"icon": "lamp-1.png"}), "k").unwrap();
        assert_eq!(i.icon, Some(PathBuf::from("lamp-1.png")));
        assert_eq!(parse_item(&json!({}), "k").unwrap().icon, None);
        for bad in ["../x.png", "a/b.png", "a\\b.png", "x", "x.jpg", ".png", "a.b.png", "", "x.PNG"] {
            let e = parse_item(&json!({ "icon": bad }), "k").unwrap_err();
            assert!(e.contains("\"icon\""), "{bad}: {e}");
        }
        assert!(parse_item(&json!({"icon": 5}), "k").is_err());
    }

    #[test]
    fn item_icons_are_checked_against_the_pack_folder() {
        let root = scratch("item_icons");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        let m = write_pack(&root, "icn_mod", r#""overrides": []"#);
        let icons = m.join("items").join("icons");
        fs::create_dir_all(&icons).unwrap();
        fs::write(icons.join("ok.png"), png_header_only(64, 64)).unwrap();
        fs::write(icons.join("big.png"), png_header_only(65, 64)).unwrap();
        fs::write(icons.join("fake.png"), b"not a png at all, really not").unwrap();
        fs::write(
            m.join("items").join("items.json"),
            r#"{"schema": "lr.story.items/1", "items": {
                "icn_mod_ok": {"icon": "ok.png"},
                "icn_mod_big": {"icon": "big.png"},
                "icn_mod_fake": {"icon": "fake.png"},
                "icn_mod_gone": {"icon": "gone.png"}}}"#,
        )
        .unwrap();
        let cfg = Config { core: core.clone(), mod_dirs: vec![m.clone()], mod_roots: Vec::new() };
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("item 'icn_mod_big'") && e.contains("at most 64x64")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("item 'icn_mod_fake'") && e.contains("not a PNG")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("item 'icn_mod_gone'") && e.contains("icon 'gone.png'")), "{errs:?}");
        assert!(!errs.iter().any(|e| e.contains("icn_mod_ok")), "{errs:?}");

        fs::write(
            m.join("items").join("items.json"),
            r#"{"schema": "lr.story.items/1", "items": {"icn_mod_ok": {"icon": "ok.png"}}}"#,
        )
        .unwrap();
        let cfg = Config { core, mod_dirs: vec![m.clone()], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|errs| panic!("{errs:?}"));
        assert_eq!(pack.items["icn_mod_ok"].icon, Some(icons.join("ok.png")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn player_stats_default_and_validate() {
        use serde_json::json;
        let s = parse_story(&json!({"era": "past"}), "p").unwrap();
        assert_eq!(s.player.stats, Stats::default());
        let s = parse_story(&json!({"era": "past", "player": {"stats": {"max_hp": 50, "attack": 8}}}), "p").unwrap();
        assert_eq!(s.player.stats, Stats { max_hp: 50, attack: 8, defense: 2, speed: 5 });
        assert!(parse_story(&json!({"era": "past", "player": {"stats": {"max_hp": 0}}}), "p").is_err());
        assert!(parse_story(&json!({"era": "past", "player": {"stats": {"attack": -1}}}), "p").is_err());
        assert!(parse_story(&json!({"era": "past", "player": {"stats": 3}}), "p").is_err());
        assert!(parse_story(&json!({"era": "past", "player": {"stats": {"speed": "fast"}}}), "p").is_err());
    }

    #[test]
    fn battle_and_rpg_ops_parse_and_validate_amounts() {
        use serde_json::json;
        assert_eq!(parse_op(&json!({"op": "battle", "battle": "b"})).unwrap(), Op::Battle("b".into()));
        assert_eq!(parse_op(&json!({"op": "heal", "amount": 5})).unwrap(), Op::Heal(5));
        let removed = parse_op(&json!({"op": "xp", "amount": 9})).unwrap_err();
        assert!(removed.contains("XP was removed"), "{removed}");
        assert!(parse_op(&json!({"op": "battle"})).is_err());
        for bad in [json!({"op": "heal"}), json!({"op": "heal", "amount": 0}), json!({"op": "heal", "amount": -2}), json!({"op": "heal", "amount": "3"})] {
            assert!(parse_op(&bad).is_err(), "{bad}");
        }
    }

    fn walls(h: f32) -> BTreeMap<String, Kind> {
        BTreeMap::from([("ffffff".to_string(), Kind::Wall { height: h, color: WALL_COLOUR })])
    }

    #[test]
    fn map_env_defaults() {
        use serde_json::json;
        let e = parse_map_env(&json!({}), &BTreeMap::new()).unwrap();
        assert_eq!((e.outdoor, e.ceiling_height, e.ceiling_color, e.sky), (false, 3.0, None, None));
        assert_eq!((e.wall_style.as_str(), e.floor_style.as_str()), ("lab", "blocks"));
        // The default ceiling is the tallest wall in the legend.
        let mut legend = walls(4.5);
        legend.insert("000000".into(), Kind::Wall { height: 2.0, color: WALL_COLOUR });
        assert_eq!(parse_map_env(&json!({}), &legend).unwrap().ceiling_height, 4.5);
        // A short wall never gives a ceiling under 1 m.
        assert_eq!(parse_map_env(&json!({}), &walls(0.5)).unwrap().ceiling_height, 1.0);
    }

    #[test]
    fn map_env_ceiling_fields() {
        use serde_json::json;
        let e = parse_map_env(&json!({"ceiling": {"height": 5.5, "color": "#aabbcc"}}), &walls(3.0)).unwrap();
        assert_eq!((e.ceiling_height, e.ceiling_color), (5.5, Some([0xaa, 0xbb, 0xcc])));
        let e = parse_map_env(&json!({"ceiling": {"color": "102030"}}), &walls(3.0)).unwrap();
        assert_eq!((e.ceiling_height, e.ceiling_color), (3.0, Some([0x10, 0x20, 0x30])));
        for bad in [
            json!({"ceiling": {"height": 0.5}}),
            json!({"ceiling": {"height": 17}}),
            json!({"ceiling": {"height": "tall"}}),
            json!({"ceiling": {"color": "red"}}),
            json!({"ceiling": {"color": "clear"}}),
            json!({"ceiling": 3}),
            json!({"outdoor": true, "ceiling": {"height": 3}}),
            json!({"outdoor": "yes"}),
        ] {
            assert!(parse_map_env(&bad, &walls(3.0)).is_err(), "{bad}");
        }
        assert!(parse_map_env(&json!({"ceiling": {"height": 1}}), &walls(3.0)).is_ok());
        assert!(parse_map_env(&json!({"ceiling": {"height": 16}}), &walls(3.0)).is_ok());
    }

    #[test]
    fn map_env_outdoor_and_sky() {
        use serde_json::json;
        let e = parse_map_env(&json!({"outdoor": true}), &walls(3.0)).unwrap();
        assert!(e.outdoor);
        assert_eq!(e.sky, Some(DEFAULT_SKY));
        let e = parse_map_env(&json!({"outdoor": true, "sky": {"top": "000011", "bottom": "#ffeedd"}}), &walls(3.0)).unwrap();
        assert_eq!(e.sky, Some(([0, 0, 0x11], [0xff, 0xee, 0xdd])));
        let e = parse_map_env(&json!({"outdoor": true, "sky": {"top": "000011"}}), &walls(3.0)).unwrap();
        assert_eq!(e.sky, Some(([0, 0, 0x11], DEFAULT_SKY.1)));
        assert!(parse_map_env(&json!({"sky": {"top": "000011"}}), &walls(3.0)).is_err(), "sky needs outdoor");
        assert!(parse_map_env(&json!({"outdoor": true, "sky": {"top": "nope"}}), &walls(3.0)).is_err());
        assert!(parse_map_env(&json!({"outdoor": true, "sky": "blue"}), &walls(3.0)).is_err());
    }

    #[test]
    fn map_env_styles() {
        use serde_json::json;
        for w in WALL_STYLES {
            assert_eq!(parse_map_env(&json!({"walls": w}), &walls(3.0)).unwrap().wall_style, w);
        }
        for f in FLOOR_STYLES {
            assert_eq!(parse_map_env(&json!({"floor": f}), &walls(3.0)).unwrap().floor_style, f);
        }
        let err = parse_map_env(&json!({"walls": "marble"}), &walls(3.0)).unwrap_err();
        assert!(err.contains("marble") && err.contains("brick"), "{err}");
        assert!(parse_map_env(&json!({"floor": "lava"}), &walls(3.0)).is_err());
        assert!(parse_map_env(&json!({"walls": 3}), &walls(3.0)).is_err());
        assert!(parse_map_env(&json!({"floor": "Tiles"}), &walls(3.0)).is_err(), "names are lowercase");
    }

    #[test]
    fn battles_parse_with_defaults_and_reject_bad_data() {
        use serde_json::json;
        let b = parse_battle(&json!({"spawn": "here", "attacks": [{"name": "Bite"}]}), "wolf").unwrap();
        assert_eq!((b.name.as_str(), b.boss, b.max_hp, b.spawn.as_str()), ("wolf", false, 20, "here"));
        assert_eq!((b.defense, b.move_speed, b.skin.clone()), (1, 3.0, None));
        let a = &b.attacks[0];
        assert_eq!((a.kind, a.damage, a.weight, a.min_phase), (AttackKind::Melee, 5, 1, 0));
        assert_eq!((a.range, a.windup, a.cooldown), (2.0, 0.6, 1.5));
        let b = parse_battle(
            &json!({"name": "King", "boss": true, "max_hp": 100, "defense": 3, "move_speed": 4.5,
                    "spawn": "throne", "skin": "king",
                    "attacks": [
                        {"name": "Slam", "kind": "slam", "damage": 20, "weight": 3, "radius": 3.0, "windup": 1.0, "text": "Boom."},
                        {"name": "Bolt", "kind": "shot", "damage": 8, "speed": 12, "min_phase": 2, "range": 15, "cooldown": 0}
                    ],
                    "phases": [66, 33], "intro": "Hi", "win": "w", "lose": "l"}),
            "king",
        )
        .unwrap();
        assert!(b.boss);
        assert_eq!((b.phases.clone(), b.win.as_deref(), b.lose.as_deref()), (vec![66, 33], Some("w"), Some("l")));
        assert_eq!((b.skin.as_deref(), b.move_speed), (Some("king"), 4.5));
        assert_eq!((b.attacks[0].kind, b.attacks[0].radius, b.attacks[0].range, b.attacks[0].text.as_str()), (AttackKind::Slam, 3.0, 6.0, "Boom."));
        assert_eq!((b.attacks[1].kind, b.attacks[1].speed, b.attacks[1].radius, b.attacks[1].min_phase, b.attacks[1].cooldown), (AttackKind::Shot, 12.0, 0.3, 2, 0.0));
        assert_eq!(parse_battle(&json!({"spawn": "s", "attacks": [{"name": "c", "kind": "charge"}]}), "c").unwrap().attacks[0].speed, 10.0);

        let m = json!([{"name": "a"}]);
        let bad = |v: Value| parse_battle(&v, "x").is_err();
        assert!(bad(json!({"spawn": "s"})), "no attacks");
        assert!(bad(json!({"spawn": "s", "attacks": []})));
        assert!(bad(json!({"attacks": m})), "no spawn");
        assert!(bad(json!({"spawn": "", "attacks": m})));
        assert!(bad(json!({"spawn": "s", "attacks": "bite"})));
        assert!(bad(json!({"spawn": "s", "attacks": m, "max_hp": 0})));
        assert!(bad(json!({"spawn": "s", "attacks": m, "move_speed": -1})));
        assert!(bad(json!({"spawn": "s", "attacks": m, "move_speed": "fast"})));
        for xp in [json!(-1), json!(0), json!(5)] {
            let err = parse_battle(&json!({"spawn": "s", "attacks": m, "xp": xp}), "x").unwrap_err();
            assert!(err.contains("XP was removed"), "{err}");
        }
        assert!(bad(json!({"spawn": "s", "attacks": m, "skin": 4})));
        for p in [0, 100, -5] {
            assert!(bad(json!({"spawn": "s", "attacks": m, "phases": [p]})), "phase {p}");
        }
        assert!(bad(json!({"spawn": "s", "attacks": m, "phases": "half"})));
        assert!(bad(json!({"spawn": "s", "attacks": m, "phases": [33, 66]})), "phases go high to low");
        assert!(bad(json!({"spawn": "s", "attacks": m, "phases": [50, 50]})));
        assert!(!bad(json!({"spawn": "s", "attacks": m, "phases": [75, 50, 25]})));
        assert!(parse_battle(&json!("wolf"), "x").is_err());
    }

    #[test]
    fn attack_fields_are_validated() {
        use serde_json::json;
        let one = |a: Value| parse_battle(&json!({"spawn": "s", "phases": [50], "attacks": [a]}), "x");
        assert!(one(json!({"name": "a", "min_phase": 1})).is_ok());
        assert!(one(json!({"name": "a", "min_phase": 2})).is_err(), "min_phase beyond the phase marks");
        assert!(one(json!({"name": "a", "min_phase": -1})).is_err());
        assert!(one(json!({})).is_err(), "no name");
        assert!(one(json!("bite")).is_err());
        assert!(one(json!({"name": "a", "kind": "laser"})).is_err());
        assert!(one(json!({"name": "a", "kind": 3})).is_err());
        assert!(one(json!({"name": "a", "weight": 0})).is_err());
        assert!(one(json!({"name": "a", "damage": 0})).is_err());
        assert!(one(json!({"name": "a", "damage": 1.5})).is_err());
        assert!(one(json!({"name": "a", "windup": 0})).is_err(), "windup must be above 0");
        assert!(one(json!({"name": "a", "windup": -1})).is_err());
        assert!(one(json!({"name": "a", "windup": "slow"})).is_err());
        for field in ["range", "cooldown", "radius", "speed"] {
            assert!(one(json!({"name": "a", field: -0.5})).is_err(), "{field} below 0");
            assert!(one(json!({"name": "a", field: "x"})).is_err(), "{field} text");
            assert!(one(json!({"name": "a", field: 0})).is_ok(), "{field} may be 0");
        }
        let e = one(json!({"name": "Zap", "kind": "laser"})).unwrap_err();
        assert!(e.contains("attack 'Zap'") && e.contains("melee, charge, shot, slam"), "{e}");
    }

    #[test]
    fn quests_parse_and_reject_bad_data() {
        use serde_json::json;
        let q = parse_quest(
            &json!({"title": "Find it", "description": "d", "show_if": {"flag": "met"},
                    "steps": [{"text": "Get key", "done_if": {"has": "k"}}, {"text": "Open", "done_if": {"flag": "open"}}]}),
            "find",
        )
        .unwrap();
        assert_eq!((q.title.as_str(), q.steps.len()), ("Find it", 2));
        assert_eq!(q.show_if, Some(Cond::Flag("met".into())));
        assert_eq!(q.steps[0].done_if, Cond::Has("k".into()));
        assert_eq!(parse_quest(&json!({"steps": [{"text": "a", "done_if": {"flag": "f"}}]}), "id").unwrap().title, "id");
        assert!(parse_quest(&json!({}), "q").is_err(), "no steps");
        assert!(parse_quest(&json!({"steps": [{"text": "a"}]}), "q").is_err(), "missing done_if");
        assert!(parse_quest(&json!({"steps": [{"text": "a", "done_if": {"bogus": 1}}]}), "q").is_err(), "bad condition");
        assert!(parse_quest(&json!({"steps": [{"done_if": {"flag": "f"}}]}), "q").is_err(), "missing text");
        assert!(parse_quest(&json!({"show_if": 3, "steps": [{"text": "a", "done_if": {"flag": "f"}}]}), "q").is_err());
    }

    #[test]
    fn battle_and_quest_files_load_and_cross_references_are_checked() {
        let root = scratch("rpg_files");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        fs::create_dir_all(core.join("battles")).unwrap();
        fs::create_dir_all(core.join("quests")).unwrap();
        fs::create_dir_all(core.join("scenes")).unwrap();
        fs::write(
            core.join("battles").join("b.json"),
            r#"{"schema": "lr.story.battles/1", "battles": {"wolf": {"spawn": "here", "attacks": [{"name": "Bite"}], "win": "nope", "lose": "also_nope"}}}"#,
        )
        .unwrap();
        fs::write(
            core.join("quests").join("q.json"),
            r#"{"schema": "lr.story.quests/1", "quests": {"hunt": {"steps": [{"text": "a", "done_if": {"has": "ghost_item"}}], "show_if": {"has": "other_ghost"}}}}"#,
        )
        .unwrap();
        fs::write(
            core.join("scenes").join("s.json"),
            r#"{"schema": "lr.story.scenes/1", "scenes": {"start": {"do": [{"op": "battle", "battle": "missing"}, {"op": "battle", "battle": "wolf"}]}}}"#,
        )
        .unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        for want in ["unknown scene 'nope'", "unknown scene 'also_nope'", "unknown item 'ghost_item'", "unknown item 'other_ghost'", "unknown battle 'missing'"] {
            assert!(errs.iter().any(|e| e.contains(want)), "{want}: {errs:?}");
        }
        assert!(!errs.iter().any(|e| e.contains("unknown battle 'wolf'")), "{errs:?}");

        // A spawn that is on no map and a skin nobody ships are errors.
        fs::write(
            core.join("battles").join("b.json"),
            r#"{"schema": "lr.story.battles/1", "battles": {"wolf": {"spawn": "nowhere", "skin": "ghost_skin", "attacks": [{"name": "Bite"}]}}}"#,
        )
        .unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("battle 'wolf': spawn 'nowhere' is not on any map")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("battle 'wolf': unknown skin 'ghost_skin'")), "{errs:?}");
        // A battle with no attacks is reported by the loader.
        fs::write(core.join("battles").join("b.json"), r#"{"schema": "lr.story.battles/1", "battles": {"wolf": {"spawn": "here"}}}"#).unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("battle 'wolf': a battle needs at least one attack")), "{errs:?}");

        // Fix every reference: the files load.
        fs::write(
            core.join("battles").join("b.json"),
            r#"{"schema": "lr.story.battles/1", "battles": {"wolf": {"spawn": "here", "attacks": [{"name": "Bite"}], "win": "start"}}}"#,
        )
        .unwrap();
        fs::write(core.join("quests").join("q.json"), r#"{"schema": "lr.story.quests/1", "quests": {"hunt": {"steps": [{"text": "a", "done_if": {"flag": "f"}}]}}}"#).unwrap();
        fs::write(
            core.join("scenes").join("s.json"),
            r#"{"schema": "lr.story.scenes/1", "scenes": {"start": {"do": [{"op": "battle", "battle": "wolf"}]}}}"#,
        )
        .unwrap();
        let pack = load(&alone(&core)).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(pack.battles["wolf"].id, "wolf");
        assert_eq!(pack.quests["hunt"].steps.len(), 1);

        // A wrong schema and a wrong container are errors.
        fs::write(core.join("quests").join("q.json"), r#"{"schema": "lr.story.items/1", "quests": {}}"#).unwrap();
        assert!(load(&alone(&core)).is_err());
        fs::write(core.join("quests").join("q.json"), r#"{"schema": "lr.story.quests/1", "quests": []}"#).unwrap();
        let errs = load(&alone(&core)).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("\"quests\" must be an object")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mod_battles_and_quests_need_the_pack_prefix_unless_overridden() {
        let root = scratch("rpg_mods");
        let core = write_pack(&root, "core_pack", &start_at("core_pack"));
        fs::create_dir_all(core.join("battles")).unwrap();
        fs::create_dir_all(core.join("quests")).unwrap();
        fs::write(core.join("battles").join("b.json"), r#"{"schema": "lr.story.battles/1", "battles": {"wolf": {"spawn": "here", "attacks": [{"name": "Bite"}]}}}"#).unwrap();
        fs::write(core.join("quests").join("q.json"), r#"{"schema": "lr.story.quests/1", "quests": {"hunt": {"steps": [{"text": "a", "done_if": {"flag": "f"}}]}}}"#).unwrap();
        let m = write_pack(&root, "mod_a", r#""overrides": ["battles/wolf", "quests/hunt"]"#);
        fs::create_dir_all(m.join("battles")).unwrap();
        fs::create_dir_all(m.join("quests")).unwrap();
        fs::write(
            m.join("battles").join("b.json"),
            r#"{"schema": "lr.story.battles/1", "battles": {"wolf": {"max_hp": 77, "spawn": "here", "attacks": [{"name": "Bite"}]}, "mod_a_bat": {"spawn": "here", "attacks": [{"name": "Screech"}]}}}"#,
        )
        .unwrap();
        fs::write(
            m.join("quests").join("q.json"),
            r#"{"schema": "lr.story.quests/1", "quests": {"hunt": {"title": "Better", "steps": [{"text": "a", "done_if": {"flag": "f"}}]}}}"#,
        )
        .unwrap();
        let cfg = Config { core: core.clone(), mod_dirs: vec![m.clone()], mod_roots: Vec::new() };
        let pack = load(&cfg).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(pack.battles["wolf"].max_hp, 77, "the override replaced the core battle");
        assert_eq!(pack.quests["hunt"].title, "Better");
        assert!(pack.battles.contains_key("mod_a_bat"));

        // Without the override, the unprefixed ids are errors.
        fs::write(m.join("pack.json"), r#"{"schema": "lr.pack/1", "id": "mod_a", "name": "x", "overrides": []}"#).unwrap();
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("battles id 'wolf' must start with 'mod_a_'")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("quests id 'hunt' must start with 'mod_a_'")), "{errs:?}");

        // An override of a battle nobody defines is refused.
        fs::write(m.join("pack.json"), r#"{"schema": "lr.pack/1", "id": "mod_a", "name": "x", "overrides": ["battles/ghost"]}"#).unwrap();
        let errs = load(&cfg).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("overrides \"battles/ghost\"")), "{errs:?}");
        let _ = fs::remove_dir_all(&root);
    }
}
