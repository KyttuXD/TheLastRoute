//! The content registry: the base game's JSON (data/, compiled in) with mod packs layered on
//! top, after game/config/content_registry.gd. Packs add new ids or override existing ones.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod embedded;
pub mod hunters;
pub mod loot;
pub mod npcs;
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
    /// The region catalogue (data/regions and packs' regions/), `lr.region/1`.
    pub regions: RegionTable,
    pub packs: Vec<Pack>,
    /// Problems found while loading (shown on the MODS settings tab).
    pub warnings: Vec<String>,
}

impl Content {
    pub fn load() -> Content {
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
        for (path, text) in embedded::data_under("hunters/").filter(|(p, _)| !p.ends_with("sabotage.json") && !p.ends_with("nightmare_pounce.json") && *p != "hunters/targets.json" && *p != "hunters/emotes.json") {
            // hunters/search.json is the search rules (hunters/brain.rs), not a hunter definition
            if path == "hunters/search.json" {
                continue;
            }
            // hunters/director.json is the squad rules (hunters/director.rs), not a hunter definition
            if path == "hunters/director.json" {
                continue;
            }
            // hunters/charge.json is the per-hunter charge table (hunters/gimmicks/charge.rs), not a hunter definition
            if path == crate::hunters::gimmicks::charge::CHARGE_PATH {
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
        packs::load_all(&mut c);
        c.regions = regions::load(&c.packs, &mut c.warnings);
        regions::install(c.regions.clone());
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
