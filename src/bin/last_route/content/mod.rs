//! The content registry: the base game's JSON (data/, compiled in) with mod packs layered on
//! top, after game/config/content_registry.gd. Packs add new ids or override existing ones.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod embedded;
pub mod hunters;
pub mod loot;
pub mod npcs;
pub mod pack_import;
pub mod packs;
pub mod props;
pub mod public_roster;
pub mod regions;

use hunters::HunterDef;
use loot::{Loot, LootTables};
use npcs::NpcRules;
use packs::Pack;
use props::PropDef;
use regions::RegionTable;
use std::collections::HashMap;

pub struct Content {
    pub hunters: Vec<HunterDef>,
    pub props: HashMap<String, PropDef>,
    pub loot: LootTables,
    /// How a normal run seeds NPC teammates (data/npcs.json).
    pub npcs: NpcRules,
    /// The region catalogue (src/bin/last_route/data/regions and packs' regions/), `lr.region/1`.
    pub regions: RegionTable,
    pub packs: Vec<Pack>,
    /// Problems found while loading (shown on the MODS settings tab).
    pub warnings: Vec<String>,
}

impl Content {
    /// The game's content with every mod pack layered on (the normal start-up load).
    pub fn load() -> Content {
        let mut c = Content::base();
        packs::load_all(&mut c);
        c.regions = regions::load(&c.packs, &mut c.warnings);
        regions::install(c.regions.clone());
        // the loaded packs' content hashes, for the room's pack check (net/world_sync.rs)
        *loaded_digests() = c.packs.iter().map(|p| (p.id.clone(), pack_digest(p))).collect();
        c
    }

    /// The base game's own data only (data/, compiled in): no mod pack, no region table installed. A mod pack is checked
    /// against this (plus the packs already imported) before the browser build keeps it (`pack_import`).
    pub fn base() -> Content {
        let mut c = Content {
            hunters: vec![],
            props: HashMap::new(),
            loot: LootTables::default(),
            npcs: NpcRules::default(),
            regions: RegionTable::default(),
            packs: vec![],
            warnings: vec![],
        };
        for (path, text) in embedded::data_under("props/") {
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(v) => {
                    for p in props::parse_file(&v) {
                        c.props.insert(p.id.clone(), p);
                    }
                }
                Err(e) => c.warnings.push(format!("{path}: {e}")),
            }
        }
        for (path, text) in embedded::data_under("hunters/").filter(|(p, _)| !p.ends_with("sabotage.json") && !p.ends_with("nightmare_pounce.json") && !p.ends_with("nyxie_wake.json") && *p != "hunters/targets.json" && *p != "hunters/emotes.json" && *p != "hunters/offscreen.json" && *p != "hunters/pathing.json") {
            // hunters/search.json is the search rules (hunters/brain.rs), not a hunter definition
            if path == "hunters/search.json" || path == crate::hunters::navigation::TUNING_PATH || path == crate::hunters::navigation::SEARCH_DIFFICULTY_PATH {
                continue;
            }
            // hunters/director.json is the squad rules (hunters/director.rs), not a hunter definition
            // hunters/draw_pool.json, activities_idle.json and knock_sound.json are rule tables too
            if matches!(path, "hunters/draw_pool.json" | "hunters/activities_idle.json" | "hunters/knock_sound.json") {
                continue;
            }
            if path == "hunters/director.json" {
                continue;
            }
            // hunters/water.json and hunters/noise_events.json are rule tables, not hunter definitions
            if path == crate::hunters::water::WATER_PATH
                || path == "hunters/noise_events.json"
                || path == crate::hunters::vent_nav::VENTS_PATH
                || path == crate::hunters::activities::PATH
                || path == crate::hunters::flight_edges::FLIGHTS_PATH
            {
                continue;
            }
            // hunters/mechanic_cues.json is the hunter mechanic animation timing table (hunters/mechanics.rs)
            if path == crate::hunters::mechanics::PATH {
                continue;
            }
            // hunters/charge.json is the per-hunter charge table (hunters/gimmicks/charge.rs), not a hunter definition
            if path == crate::hunters::gimmicks::charge::CHARGE_PATH {
                continue;
            }
            // hunters/secret_creatures.json is the secret wolf spiders' rule table (hunters/secret_creatures.rs)
            if path == crate::hunters::secret_creatures::SECRET_PATH {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(text)
                .map_err(|e| e.to_string())
                .and_then(|v| hunters::parse(&v, None, ""))
            {
                Ok(mut h) => {
                    h.model
                        .get_or_insert_with(|| format!("hunters/{}.bbmodel", h.id));
                    h.atlas
                        .get_or_insert_with(|| format!("hunters/{}_atlas.png", h.id));
                    h.portrait
                        .get_or_insert_with(|| format!("portraits/{}.png", h.id));
                    c.hunters.push(h);
                }
                Err(e) => c.warnings.push(format!("{path}: {e}")),
            }
        }
        for (file, table) in [("loot/route.json", &mut c.loot.route), ("loot/sewer.json", &mut c.loot.sewer)] {
            if let Some((_, text)) = embedded::data_under(file).next() {
                if let Ok(v) = serde_json::from_str(text) {
                    *table = Loot::parse(&v);
                }
            }
        }
        if let Some((path, text)) = embedded::data_under("npcs.json").next() {
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(v) => c.npcs = NpcRules::parse(&v),
                Err(e) => c.warnings.push(format!("{path}: {e}")),
            }
        }
        c
    }

    pub fn hunter(&self, id: &str) -> Option<&HunterDef> {
        self.hunters.iter().find(|h| h.id == id)
    }

    pub fn hunter_index(&self, id: &str) -> Option<usize> {
        self.hunters.iter().position(|h| h.id == id)
    }

    /// DRAW_IDS: IMPLEMENTED_IDS plus hunters added by loaded packs, in that order.
    pub fn draw_pool(&self) -> Vec<usize> {
        let mut out: Vec<usize> = hunters::IMPLEMENTED_IDS
            .iter()
            .filter_map(|id| self.hunter_index(id))
            .collect();
        for (i, h) in self.hunters.iter().enumerate() {
            if !h.pack.is_empty()
                && !hunters::IMPLEMENTED_IDS.contains(&h.id.as_str())
                && !hunters::SPECIAL_IDS.contains(&h.id.as_str())
                && h.category == "az"
                && !out.contains(&i)
            {
                out.push(i);
            }
        }
        out
    }
}

fn loaded_digests() -> std::sync::MutexGuard<'static, Vec<(String, String)>> {
    static D: std::sync::OnceLock<std::sync::Mutex<Vec<(String, String)>>> = std::sync::OnceLock::new();
    D.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Each mod pack the last `Content::load` loaded, as (pack id, hex hash of its content). The hash covers every file of
/// the pack in path order, each by its pack-relative path and its bytes, so two players with the same packs get the same
/// list. Empty before the first load and with no packs.
pub fn loaded_pack_digests() -> Vec<(String, String)> {
    loaded_digests().clone()
}

/// The content hash of one pack: the sha256 of its files' lines `<relative path> <sha256 of the bytes>`, in path order.
pub fn pack_digest(pack: &Pack) -> String {
    let files: Vec<(String, String)> = match pack_import::virtual_id(&pack.dir) {
        // an imported pack (browser build) is hashed from the memory it is served from
        Some(id) => pack_import::with_registry(|r| r.get(id))
            .map(|p| p.files.iter().map(|(rel, bytes)| (rel.clone(), crate::shared::skins::sha256_hex(bytes))).collect())
            .unwrap_or_default(),
        None => {
            let mut out = Vec::new();
            hash_tree(&pack.dir, "", &mut out);
            out
        }
    };
    digest_lines(files)
}

fn digest_lines(mut files: Vec<(String, String)>) -> String {
    files.sort();
    let mut text = String::new();
    for (rel, hash) in &files {
        text.push_str(rel);
        text.push(' ');
        text.push_str(hash);
        text.push('\n');
    }
    crate::shared::skins::sha256_hex(text.as_bytes())
}

/// The files under `dir` as (path relative to the pack, sha256), skipping the macOS metadata files the server also removes.
fn hash_tree(dir: &std::path::Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" || name.starts_with("._") {
            continue;
        }
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            hash_tree(&e.path(), &rel, out);
        } else if kind.is_file() {
            if let Ok(bytes) = std::fs::read(e.path()) {
                out.push((rel, crate::shared::skins::sha256_hex(&bytes)));
            }
        }
    }
}

#[cfg(test)]
mod digest_tests {
    use super::*;

    #[test]
    fn a_pack_digest_follows_the_files_not_their_order() {
        let a = digest_lines(vec![("pack.json".into(), "aa".into()), ("hunters/x.json".into(), "bb".into())]);
        let b = digest_lines(vec![("hunters/x.json".into(), "bb".into()), ("pack.json".into(), "aa".into())]);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, digest_lines(vec![("pack.json".into(), "aa".into()), ("hunters/x.json".into(), "bc".into())]));
        assert_ne!(a, digest_lines(vec![("pack.json".into(), "aa".into())]));
    }
}
