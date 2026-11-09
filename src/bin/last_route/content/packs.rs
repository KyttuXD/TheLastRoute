//! Mod packs in the original's format (examples/mods/*): a folder with `pack.json`
//! (`lr.pack/1`: id, name, version, author, description, overrides) and content folders such
//! as `hunters/*.json` (`lr.hunter/1`) and `loot/*.json` (`lr.loot/1`). A pack adds new ids;
//! it may replace a base id only when that id is listed in `overrides`. Packs load in folder
//! name order from `mods/` beside the executable, `mods/` in the working folder, and the
//! per-user folder (%APPDATA%/KYTTU/TheLastRoute/mods). Pack hunters may bring a `.bbmodel` model with
//! its atlas and a portrait PNG; a model in another format falls back to the base body.
//!
//! The browser build has no folders: its packs are imported through the MODS tab and kept in the page's storage
//! (`pack_import`). They load through the same code below, from memory instead of from a folder (`Source::Memory`).

use super::pack_import::{self, ImportedPack};
use super::{hunters, Content};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The pack type lives in `crate::packs` (standalone, no game modules); re-exported so paths do not change.
pub use crate::packs::Pack;

// ------------------------------------------------------------------------------------------- limits

/// The size and count limits of a pack (data/packs/limits.json, `lr.pack_limits/1`). The numbers equal the pack index's
/// `MOD_LIMITS` in `lr_server`, so a pack the index lists is never refused here for its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackLimits {
    /// Files in one pack.
    pub max_files: usize,
    /// Bytes in one file.
    pub max_file_bytes: usize,
    /// Bytes in all files of one pack.
    pub max_pack_bytes: usize,
    /// Bytes in every imported pack together (what the browser keeps).
    pub max_total_bytes: usize,
    /// Entries read from one zip, and files looked at in one chosen folder.
    pub max_listed: usize,
}

impl Default for PackLimits {
    /// Used for a key the file lacks (or when the file is unreadable): the index's `MOD_LIMITS`.
    fn default() -> Self {
        PackLimits { max_files: 512, max_file_bytes: 16 << 20, max_pack_bytes: 128 << 20, max_total_bytes: 256 << 20, max_listed: 4096 }
    }
}

impl PackLimits {
    /// Reads a limits document. A key that is missing keeps its default; a key that is not a positive whole number is an error.
    pub fn parse(text: &str) -> Result<PackLimits, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("limits.json: {e}"))?;
        let mut out = PackLimits::default();
        let read = |key: &str, slot: &mut usize| -> Result<(), String> {
            match v.get(key) {
                None => Ok(()),
                Some(x) => match x.as_u64().filter(|n| *n > 0).and_then(|n| usize::try_from(n).ok()) {
                    Some(n) => {
                        *slot = n;
                        Ok(())
                    }
                    None => Err(format!("limits.json: \"{key}\" must be a positive whole number")),
                },
            }
        };
        read("max_files", &mut out.max_files)?;
        read("max_file_bytes", &mut out.max_file_bytes)?;
        read("max_pack_bytes", &mut out.max_pack_bytes)?;
        read("max_total_bytes", &mut out.max_total_bytes)?;
        read("max_listed", &mut out.max_listed)?;
        Ok(out)
    }
}

/// The limits in force: data/packs/limits.json, or the defaults when the embedded file cannot be read.
pub fn limits() -> &'static PackLimits {
    static LIMITS: OnceLock<PackLimits> = OnceLock::new();
    LIMITS.get_or_init(|| {
        super::embedded::data_under("packs/limits.json")
            .next()
            .and_then(|(_, text)| PackLimits::parse(text).ok())
            .unwrap_or_default()
    })
}

/// Checks a pack folder against `lim` without reading file contents: how many files, how big each is, how big all are.
/// The walk stops at the first limit passed, so a huge folder is refused instead of listed whole.
pub fn check_disk_limits(dir: &Path, lim: &PackLimits) -> Result<(), String> {
    let (mut files, mut total) = (0usize, 0usize);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            files += 1;
            if files > lim.max_files {
                return Err(format!("more than {} files; a pack may hold at most {} (TOO_MANY_FILES)", lim.max_files, lim.max_files));
            }
            let size = usize::try_from(meta.len()).unwrap_or(usize::MAX);
            let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string();
            if size > lim.max_file_bytes {
                return Err(format!("{rel}: the file is over the {} MB limit (FILE_TOO_LARGE)", lim.max_file_bytes >> 20));
            }
            total = total.saturating_add(size);
            if total > lim.max_pack_bytes {
                return Err(format!("the pack is over the {} MB limit (PACK_TOO_LARGE)", lim.max_pack_bytes >> 20));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------- download check

/// Checks one downloaded pack file against its manifest entry (`size` and `sha256` of the pack index) before it is written
/// or loaded. The size is checked first (cheap, and it stops a huge body being hashed); the hash is compared in lower case.
/// Err names the file and which check failed; the caller drops the file and the pack. (The sha256 is the shared
/// `shared::skins::sha256_hex`.)
pub fn verify_download(path: &str, bytes: &[u8], want_size: u64, want_sha256: &str) -> Result<(), String> {
    if bytes.len() as u64 != want_size {
        return Err(format!("{path}: downloaded {} bytes but the pack index lists {want_size} (SIZE_MISMATCH)", bytes.len()));
    }
    let got = crate::shared::skins::sha256_hex(bytes);
    if !got.eq_ignore_ascii_case(want_sha256.trim()) {
        return Err(format!("{path}: the sha256 does not match the pack index (HASH_MISMATCH)"));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------- overrides

/// Whether `overrides` lists content `id` of `kind` ("hunters", "props", "loot", "regions"). Both forms count: the index's
/// `<kind>/<id>` (`hunters/mimic`) and the bare id (`mimic`) the loader used to compare.
pub fn listed(overrides: &[String], kind: &str, id: &str) -> bool {
    overrides.iter().any(|o| o == id || o.split_once('/').is_some_and(|(k, rest)| k == kind && rest == id))
}

// ------------------------------------------------------------------------------------------- region loot

struct RegionLootStore {
    /// (pack id, region id, table) in load order.
    entries: Vec<(String, String, super::loot::Loot)>,
    /// The pack ids that are in the loaded game; None until `activate_region_loot` runs.
    active: Option<Vec<String>>,
}

static REGION_LOOT: Mutex<RegionLootStore> = Mutex::new(RegionLootStore { entries: Vec::new(), active: None });

/// The loot table a loaded pack gave `region` (a loot file with a `"region"` key), or None: the region then rolls the
/// game's own table (`LootTables::table_for`). The last pack that names the region wins. Nothing is returned until the
/// game has finished loading its packs, so a pack that was only test-loaded (a browser import check) never changes loot.
pub fn region_loot(region: &str) -> Option<super::loot::Loot> {
    let store = REGION_LOOT.lock().ok()?;
    let active = store.active.as_ref()?;
    store.entries.iter().rev().find(|(pack, r, _)| r == region && active.contains(pack)).map(|(_, _, l)| l.clone())
}

/// Marks the packs in `c` as the ones whose region loot counts. `load_all` calls it once every pack is loaded.
pub fn activate_region_loot(c: &Content) {
    if let Ok(mut store) = REGION_LOOT.lock() {
        let active = store.active.get_or_insert_with(Vec::new);
        for p in &c.packs {
            if !active.contains(&p.id) {
                active.push(p.id.clone());
            }
        }
    }
}

/// The region a loot document is for: its `"region"` key when it is a plain id.
fn loot_region(v: &serde_json::Value) -> Option<String> {
    let r = v.get("region")?.as_str()?;
    (!r.is_empty() && r.len() <= 64 && r.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')).then(|| r.to_string())
}

// ------------------------------------------------------------------------------------------- validator

/// Checks the pack folder `dir` the way the game would load it, without a window: the size limits, `pack.json`, every
/// hunter, prop, loot and region file. Returns every problem found (empty when the pack is good) and what the pack adds.
pub fn validate_dir(dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut errors = vec![];
    if !dir.join("pack.json").is_file() {
        return (vec![format!("{}: no pack.json in this folder", dir.display())], vec![]);
    }
    if let Err(e) = check_disk_limits(dir, limits()) {
        return (vec![e], vec![]);
    }
    let mut c = Content::base();
    let base_warnings = c.warnings.len();
    load_source(&mut c, &Source::Disk(dir));
    let mut warnings = c.warnings.split_off(base_warnings);
    let Some(pack) = c.packs.first().cloned() else {
        return (warnings, vec![]);
    };
    let mut provides = pack.provides.clone();
    // an optional "levels" section in pack.json (data/levels.json's entry shape) must extend the shipped levels
    if let Some(section) = std::fs::read_to_string(dir.join("pack.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("levels").cloned())
    {
        if let Err(why) = crate::world::levels::check_section(&section) {
            errors.push(format!("{}: pack.json \"levels\": {why}", pack.id));
        }
    }
    let table = crate::packs::regions::load(&c.packs, &mut warnings);
    // "loaded, but no level places it" is a note for a new region id, not a fault of the pack
    warnings.retain(|w| !w.contains("no level places it"));
    // a region the pack adds or replaces is listed by id
    provides.extend(table.defs.iter().filter(|d| d.source == pack.id).map(|d| format!("region:{}", d.id)));
    errors.extend(warnings);
    if errors.is_empty() && provides.is_empty() {
        errors.push("the pack adds nothing the game reads: put hunters/<name>.json, props/<name>.json, regions/<name>.json or a loot file in it".to_string());
    }
    (errors, provides)
}

/// `--validate-pack DIR`: prints the result and returns the process exit code (0 good, 1 problems).
pub fn validate_cli(dir: &str) -> i32 {
    let (errors, provides) = validate_dir(Path::new(dir));
    if errors.is_empty() {
        println!("OK: {dir} adds {} item(s): {}", provides.len(), provides.join(", "));
        0
    } else {
        for e in &errors {
            println!("ERROR: {e}");
        }
        println!("{} problem(s) in {dir}", errors.len());
        1
    }
}

pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.join("mods"));
        }
    }
    dirs.push(PathBuf::from("mods"));
    if let Some(user) = crate::shared::paths::user_dir() {
        dirs.push(user.join("mods"));
    }
    let mut seen = vec![];
    dirs.retain(|d| {
        let key = d.canonicalize().unwrap_or(d.clone());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    dirs
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Where a pack's files are read from: a folder on disk, or the memory of an imported pack (browser build).
pub enum Source<'a> {
    Disk(&'a Path),
    Memory(&'a ImportedPack),
}

impl Source<'_> {
    /// The `Pack.dir` the pack is known by: its folder, or the virtual folder `read_pack_file` serves from memory.
    fn dir(&self) -> PathBuf {
        match self {
            Source::Disk(dir) => dir.to_path_buf(),
            Source::Memory(pack) => pack_import::virtual_dir(&pack.id),
        }
    }

    /// The name used in messages: the folder name, or the pack's id.
    fn label(&self) -> String {
        match self {
            Source::Disk(dir) => dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            Source::Memory(pack) => pack.id.clone(),
        }
    }

    fn read_text(&self, rel: &str) -> Option<String> {
        match self {
            Source::Disk(dir) => std::fs::read_to_string(dir.join(rel)).ok(),
            Source::Memory(pack) => pack.text(rel),
        }
    }

    /// The `*.json` files directly inside the pack folder `sub`, in name order: the name to show in a message and the
    /// text (or why it could not be read).
    fn json_texts(&self, sub: &str) -> Vec<(String, Result<String, String>)> {
        match self {
            Source::Disk(dir) => json_files(&dir.join(sub))
                .into_iter()
                .map(|f| {
                    let text = std::fs::read_to_string(&f).map_err(|e| e.to_string());
                    (f.display().to_string(), text)
                })
                .collect(),
            Source::Memory(pack) => pack.json_in(sub),
        }
    }
}

/// Reads one file of a pack by its pack-relative path.
pub fn read_pack_file(pack_dir: &Path, rel: &str) -> Option<Vec<u8>> {
    // an imported pack (browser build) is served from memory
    if let Some(answer) = pack_import::read_virtual(pack_dir, rel) {
        return answer;
    }
    let p = pack_dir.join(rel);
    let canon = p.canonicalize().ok()?;
    // stay inside the pack folder
    if !canon.starts_with(pack_dir.canonicalize().ok()?) {
        return None;
    }
    std::fs::read(canon).ok()
}

pub fn load_all(c: &mut Content) {
    for root in search_dirs() {
        let mut packs: Vec<PathBuf> = std::fs::read_dir(&root)
            .map(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.join("pack.json").is_file())
                    .collect()
            })
            .unwrap_or_default();
        packs.sort();
        for dir in packs {
            load_pack(c, &dir);
        }
    }
    // the browser build: the packs the player imported (kept in the page's storage), after any others
    if cfg!(target_arch = "wasm32") {
        pack_import::load_stored_into(c);
    }
    activate_region_loot(c);
    activate_pack_levels(c);
}

/// Registers the `levels` section of each loaded pack with the level table (`world::levels::register_pack_section`), so
/// the levels a pack adds are played. This is the play-activation point: `load_all` runs it once the packs are loaded
/// for the game, while `validate_dir` and the browser import check only validate a section (`levels::check_section`)
/// and never register it. A section the table refuses goes to `c.warnings` and its levels are not added.
fn activate_pack_levels(c: &mut Content) {
    let packs: Vec<(String, PathBuf)> = c.packs.iter().map(|p| (p.id.clone(), p.dir.clone())).collect();
    for (id, dir) in packs {
        // a pack already registered by an earlier load of the content keeps its levels (a second load adds nothing)
        let Ok(mut done) = LEVEL_PACKS.lock() else {
            return;
        };
        if done.contains(&id) {
            continue;
        }
        let Some(text) = read_pack_file(&dir, "pack.json").and_then(|b| String::from_utf8(b).ok()) else {
            continue;
        };
        let Some(section) = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("levels").cloned())
        else {
            continue;
        };
        match crate::world::levels::register_pack_section(&section) {
            Ok(()) => done.push(id),
            Err(why) => c.warnings.push(format!("{id}: pack.json \"levels\" {why}")),
        }
    }
}

/// The packs whose `levels` section is in the level table (`activate_pack_levels`).
static LEVEL_PACKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Forgets which packs already registered their levels (tests that reset the level table).
#[cfg(test)]
pub(crate) fn forget_level_packs() {
    if let Ok(mut done) = LEVEL_PACKS.lock() {
        done.clear();
    }
}

fn load_pack(c: &mut Content, dir: &Path) {
    load_source(c, &Source::Disk(dir));
}

/// Loads one pack from its source into the content registry. Problems go to `c.warnings`; the pack is skipped when its
/// `pack.json` is unusable.
pub fn load_source(c: &mut Content, src: &Source) {
    let label = src.label();
    let dir = src.dir();
    if let Source::Disk(d) = src {
        if let Err(why) = check_disk_limits(d, limits()) {
            c.warnings.push(format!("{label}: {why}"));
            return;
        }
    }
    let manifest: serde_json::Value = match src
        .read_text("pack.json")
        .and_then(|t| serde_json::from_str(&t).ok())
    {
        Some(v) => v,
        None => {
            c.warnings
                .push(format!("{label}: pack.json is not valid JSON"));
            return;
        }
    };
    if manifest["schema"]
        .as_str()
        .map(|s| s != "lr.pack/1")
        .unwrap_or(false)
    {
        c.warnings.push(format!("{label}: unknown pack schema"));
        return;
    }
    let id = manifest["id"].as_str().unwrap_or("").to_string();
    if id.is_empty() || c.packs.iter().any(|p| p.id == id) {
        c.warnings
            .push(format!("{label}: missing or duplicate pack id"));
        return;
    }
    let overrides: Vec<String> = manifest["overrides"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut pack = Pack {
        id: id.clone(),
        name: manifest["name"].as_str().unwrap_or(&id).to_string(),
        version: manifest["version"].as_str().unwrap_or("").to_string(),
        author: manifest["author"].as_str().unwrap_or("").to_string(),
        description: manifest["description"].as_str().unwrap_or("").to_string(),
        dir,
        provides: vec![],
    };
    for (shown, text) in src.json_texts("hunters") {
        let parsed = text
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()));
        let v = match parsed {
            Ok(v) => v,
            Err(e) => {
                c.warnings.push(format!("{id}/{shown}: {e}"));
                continue;
            }
        };
        let hid = v["id"].as_str().unwrap_or("").to_string();
        let existing = c.hunter_index(&hid);
        if existing.is_some() && !listed(&overrides, "hunters", &hid) {
            c.warnings.push(format!(
                "{id}: {hid} already exists; list it in \"overrides\" to replace it"
            ));
            continue;
        }
        match hunters::parse(&v, existing.map(|i| &c.hunters[i]), &id) {
            Ok(h) => {
                pack.provides.push(h.id.clone());
                match existing {
                    Some(i) => c.hunters[i] = h,
                    None => c.hunters.push(h),
                }
            }
            Err(e) => c.warnings.push(format!("{id}: {e}")),
        }
    }
    // props: a pack may restyle a catalogue type it lists in "overrides" (models are pack-relative paths)
    for (_, text) in src.json_texts("props") {
        if let Some(v) = text.ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            for mut p in super::props::parse_file(&v) {
                if c.props.contains_key(&p.id) && !listed(&overrides, "props", &p.id) {
                    c.warnings.push(format!(
                        "{id}: prop {} already exists; list it in \"overrides\" to replace it",
                        p.id
                    ));
                    continue;
                }
                p.models = p.models.iter().map(|m| format!("pack:{id}:{m}")).collect();
                pack.provides.push(format!("prop:{}", p.id));
                c.props.insert(p.id.clone(), p);
            }
        }
    }
    for (_, text) in src.json_texts("loot") {
        if let Some(v) = text.ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            let lid = v["id"].as_str().unwrap_or("");
            if let Some(region) = loot_region(&v) {
                // a table for one region: it needs no override, it adds a table the region did not have
                if let Ok(mut store) = REGION_LOOT.lock() {
                    store.entries.retain(|(p, r, _)| !(p == &id && r == &region));
                    store.entries.push((id.clone(), region.clone(), super::loot::Loot::parse(&v)));
                }
                pack.provides.push(format!("region_loot:{region}"));
            } else if listed(&overrides, "loot", lid) {
                if let Some(table) = c.loot.by_id_mut(lid) {
                    *table = super::loot::Loot::parse(&v);
                    pack.provides.push(format!("loot:{lid}"));
                }
            }
        }
    }
    c.packs.push(pack);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded::base64_decode;
    use crate::content::loot::LootTables;
    use crate::content::npcs::NpcRules;
    use crate::content::regions::RegionTable;
    use crate::render::bbmodel;
    use std::collections::HashMap;

    /// A 1x1 PNG.
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    fn empty_content() -> Content {
        Content {
            hunters: vec![],
            props: HashMap::new(),
            loot: LootTables::default(),
            npcs: NpcRules::default(),
            regions: RegionTable::default(),
            packs: vec![],
            warnings: vec![],
        }
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn tiny_bbmodel(texture: &str) -> String {
        format!(
            r#"{{"meta":{{"format_version":"4.10","model_format":"free","box_uv":false}},"name":"lamp","resolution":{{"width":16,"height":16}},"elements":[{{"name":"base","type":"cube","from":[0,0,0],"to":[8,8,8],"origin":[0,0,0],"faces":{{"north":{{"uv":[0,0,8,8],"texture":0}}}}}}],"textures":[{texture}]}}"#
        )
    }

    /// A pack bbmodel prop: the prop is registered with its pack-qualified model path, and the model and its texture
    /// read back through the same pack-file reader load_models uses (an embedded data URI, or a pack png the model names).
    #[test]
    fn a_pack_bbmodel_prop_reads_its_own_texture() {
        let dir = std::env::temp_dir().join(format!("lr-pack-bbmodel-{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "pack.json", r#"{"schema":"lr.pack/1","id":"lamp_pack","name":"Lamps","version":"1.0.0","author":"t"}"#);
        write(
            &dir,
            "props/lamps.json",
            r#"{"props":[{"id":"mod_lamp","models":["assets/props/lamp.bbmodel","assets/props/lamp2.bbmodel"]}]}"#,
        );
        write(
            &dir,
            "assets/props/lamp.bbmodel",
            &tiny_bbmodel(&format!(r#"{{"name":"lamp.png","source":"data:image/png;base64,{PNG_B64}"}}"#)),
        );
        write(&dir, "assets/props/lamp2.bbmodel", &tiny_bbmodel(r#"{"name":"lamp2.png","relative_path":"lamp2.png"}"#));
        std::fs::write(dir.join("assets/props/lamp2.png"), base64_decode(PNG_B64)).unwrap();
        let mut c = empty_content();
        load_pack(&mut c, &dir);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        let def = &c.props["mod_lamp"];
        assert_eq!(def.models, ["pack:lamp_pack:assets/props/lamp.bbmodel", "pack:lamp_pack:assets/props/lamp2.bbmodel"]);
        for (rel, from_file) in [("assets/props/lamp.bbmodel", false), ("assets/props/lamp2.bbmodel", true)] {
            let text = String::from_utf8(read_pack_file(&dir, rel).expect(rel)).unwrap();
            let sibling = |file: &str| read_pack_file(&dir, &format!("assets/props/{file}"));
            let j: serde_json::Value = serde_json::from_str(&text).unwrap();
            let (found, bytes) = bbmodel::texture_bytes(&j, &sibling).expect(rel);
            assert_eq!(found.is_some(), from_file, "{rel}");
            assert_eq!(bytes, base64_decode(PNG_B64), "{rel}");
        }
        // a pack file path cannot climb out of the pack
        assert!(read_pack_file(&dir, "../outside.bbmodel").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- overrides, limits, download check, region loot, validator

    fn temp_pack(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr-pack-{name}-{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn pack_json(id: &str, overrides: &str) -> String {
        format!(r#"{{"schema":"lr.pack/1","id":"{id}","name":"T","version":"1","author":"t","overrides":{overrides}}}"#)
    }

    #[test]
    fn listed_accepts_the_index_form_and_the_bare_id_but_only_for_the_right_kind() {
        let o = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(listed(&o(&["hunters/aki"]), "hunters", "aki"));
        assert!(listed(&o(&["aki"]), "hunters", "aki"));
        assert!(!listed(&o(&["props/aki"]), "hunters", "aki"), "a prop override does not replace a hunter");
        assert!(!listed(&o(&["hunters/akira"]), "hunters", "aki"));
        assert!(!listed(&[], "hunters", "aki"));
        assert!(listed(&o(&["loot/route"]), "loot", "route"));
    }

    #[test]
    fn a_hunter_override_works_in_both_forms_and_is_refused_without_one() {
        let doc = r#"{"schema":"lr.hunter/1","id":"aki","speed":1.37}"#;
        for (overrides, replaced) in [(r#"["hunters/aki"]"#, true), (r#"["aki"]"#, true), ("[]", false), (r#"["props/aki"]"#, false)] {
            let dir = temp_pack("ovr");
            write(&dir, "pack.json", &pack_json("ovr_pack", overrides));
            write(&dir, "hunters/aki.json", doc);
            let mut c = Content::base();
            let before = c.warnings.len();
            let base_speed = c.hunters[c.hunter_index("aki").expect("aki is a base hunter")].speed;
            load_pack(&mut c, &dir);
            let h = &c.hunters[c.hunter_index("aki").unwrap()];
            assert_eq!(h.is_pack(), replaced, "{overrides}");
            assert_eq!(c.warnings.len() > before, !replaced, "{overrides}: {:?}", c.warnings);
            assert_eq!(h.speed != base_speed, replaced, "{overrides}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn the_limits_file_matches_the_index_limits_and_a_changed_value_changes_behaviour() {
        assert_eq!(*limits(), PackLimits::default(), "data/packs/limits.json equals the lr_server MOD_LIMITS");
        let small = PackLimits::parse(r#"{"max_files":2,"max_file_bytes":10,"max_pack_bytes":15}"#).unwrap();
        assert_eq!((small.max_files, small.max_file_bytes, small.max_pack_bytes), (2, 10, 15));
        assert_eq!(small.max_listed, PackLimits::default().max_listed, "a missing key keeps its default");
        assert!(PackLimits::parse(r#"{"max_files":0}"#).is_err());
        assert!(PackLimits::parse(r#"{"max_files":"many"}"#).is_err());
        assert!(PackLimits::parse("not json").is_err());

        let dir = temp_pack("limits");
        write(&dir, "pack.json", "{}");
        write(&dir, "a/one.json", "12345");
        assert!(check_disk_limits(&dir, &small).is_ok());
        write(&dir, "a/two.json", "12345678");
        assert!(check_disk_limits(&dir, &small).is_err(), "three files exceed max_files 2");
        let few = PackLimits { max_files: 10, ..small };
        assert!(check_disk_limits(&dir, &few).is_ok());
        write(&dir, "a/three.json", "123456789012");
        assert!(check_disk_limits(&dir, &few).unwrap_err().contains("FILE_TOO_LARGE"));
        let roomy = PackLimits { max_file_bytes: 100, ..few };
        assert!(check_disk_limits(&dir, &roomy).unwrap_err().contains("PACK_TOO_LARGE"));
        assert!(check_disk_limits(&dir, &PackLimits { max_pack_bytes: 1000, ..roomy }).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The SHA-256 test vectors for "abc" and "" (FIPS 180-2), and the download check built on them.
    #[test]
    fn sha256_known_vectors_and_download_mismatches_are_rejected() {
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(crate::shared::skins::sha256_hex(b"abc"), abc);
        assert_eq!(crate::shared::skins::sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert!(verify_download("hunters/a.json", b"abc", 3, abc).is_ok());
        assert!(verify_download("hunters/a.json", b"abc", 3, &abc.to_uppercase()).is_ok(), "the hash compares in lower case");
        let wrong_size = verify_download("hunters/a.json", b"abc", 4, abc).unwrap_err();
        assert!(wrong_size.contains("SIZE_MISMATCH") && wrong_size.contains("hunters/a.json"), "{wrong_size}");
        let wrong_hash = verify_download("hunters/a.json", b"abd", 3, abc).unwrap_err();
        assert!(wrong_hash.contains("HASH_MISMATCH"), "{wrong_hash}");
        assert!(verify_download("x", b"abc", 3, "").is_err());
    }

    #[test]
    fn a_pack_loot_file_with_a_region_adds_that_regions_table() {
        let dir = temp_pack("regloot");
        write(&dir, "pack.json", &pack_json("regloot_pack", "[]"));
        write(
            &dir,
            "loot/mall.json",
            r#"{"schema":"lr.loot/1","id":"mall_loot","region":"regloot_mall","base":["Medkit"],"rolls":[{"kind":"tools","chance":0.5}],"tools":[{"item":"Flashlight","weight":2}]}"#,
        );
        // not a plain region id: ignored by the region path
        write(&dir, "loot/bad.json", r#"{"id":"bad_loot","region":"../x","base":["X"]}"#);
        let mut c = Content::base();
        load_pack(&mut c, &dir);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        assert_eq!(c.packs[0].provides, ["region_loot:regloot_mall"]);
        activate_region_loot(&c);
        let t = region_loot("regloot_mall").expect("the pack's table is active");
        assert_eq!(t.base, ["Medkit"]);
        assert_eq!(t.rolls, [("tools".to_string(), 0.5)]);
        assert_eq!(t.tools, [("Flashlight".to_string(), 2.0)]);
        assert!(region_loot("regloot_other").is_none(), "a region the pack did not name keeps the game's table");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_validator_passes_a_good_pack_and_reports_every_problem_of_a_bad_one() {
        let good = temp_pack("val-good");
        write(&good, "pack.json", &pack_json("val_good", "[]"));
        write(&good, "hunters/new.json", r#"{"schema":"lr.hunter/1","id":"val_new"}"#);
        write(
            &good,
            "regions/crypt.json",
            r#"{"schema":"lr.region/1","id":"val_crypt","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[{"type":"crate","size":[1,1,1]}]}"#,
        );
        let (errors, provides) = validate_dir(&good);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(provides.contains(&"val_new".to_string()) && provides.contains(&"region:val_crypt".to_string()), "{provides:?}");
        assert_eq!(validate_cli(good.to_str().unwrap()), 0);

        let bad = temp_pack("val-bad");
        write(&bad, "pack.json", &pack_json("val_bad", "[]"));
        write(&bad, "hunters/dup.json", r#"{"schema":"lr.hunter/1","id":"aki"}"#);
        write(&bad, "hunters/broken.json", "{ not json");
        write(&bad, "regions/oops.json", r#"{"schema":"lr.region/1","id":"val_oops"}"#);
        let (errors, _) = validate_dir(&bad);
        assert!(errors.len() >= 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("already exists")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("broken.json")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("oops.json")), "{errors:?}");
        assert_eq!(validate_cli(bad.to_str().unwrap()), 1);

        let empty = temp_pack("val-empty");
        write(&empty, "pack.json", &pack_json("val_empty", "[]"));
        assert!(validate_dir(&empty).0.iter().any(|e| e.contains("adds nothing")));
        let missing = temp_pack("val-missing");
        assert!(validate_dir(&missing).0[0].contains("no pack.json"));
        for d in [good, bad, empty] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
