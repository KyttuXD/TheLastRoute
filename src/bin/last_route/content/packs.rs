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

/// The pack type lives in `crate::packs` (standalone, no game modules); re-exported so paths do not change.
pub use crate::packs::Pack;

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
}

fn load_pack(c: &mut Content, dir: &Path) {
    load_source(c, &Source::Disk(dir));
}

/// Loads one pack from its source into the content registry. Problems go to `c.warnings`; the pack is skipped when its
/// `pack.json` is unusable.
pub fn load_source(c: &mut Content, src: &Source) {
    let label = src.label();
    let dir = src.dir();
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
        if existing.is_some() && !overrides.contains(&hid) {
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
                if c.props.contains_key(&p.id) && !overrides.contains(&p.id) {
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
            if overrides.iter().any(|o| o == lid) {
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
}
