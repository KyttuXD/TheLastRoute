//! `lr.loot/1` tables (data/loot/route.json and data/loot/sewer.json). The finite route places
//! one of every item deterministically, so these weights matter for packs and the Infinite
//! route; they are loaded and validated so packs can override them.

use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Loot {
    pub id: String,
    pub base: Vec<String>,
    /// (kind, chance)
    pub rolls: Vec<(String, f32)>,
    /// (item, weight)
    pub tools: Vec<(String, f32)>,
    pub backpacks: Vec<String>,
    /// (item, chance)
    pub aid: Vec<(String, f32)>,
}

impl Loot {
    pub fn parse(v: &Value) -> Loot {
        let strs = |k: &str| {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let pairs = |k: &str, name: &str, num: &str| -> Vec<(String, f32)> {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            Some((e[name].as_str()?.to_string(), e[num].as_f64()? as f32))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Loot {
            id: v["id"].as_str().unwrap_or("route").to_string(),
            base: strs("base"),
            rolls: pairs("rolls", "kind", "chance"),
            tools: pairs("tools", "item", "weight"),
            backpacks: strs("backpacks"),
            aid: pairs("aid", "item", "chance"),
        }
    }
}

/// The loaded tables, after `LootTables.table_for` (loot_tables.gd): the sewer region rolls the
/// `sewer` table, every other region the `route` table.
#[derive(Clone, Debug, Default)]
pub struct LootTables {
    pub route: Loot,
    pub sewer: Loot,
}

impl LootTables {
    pub fn table_for(&self, region: &str) -> &Loot {
        if region == "sewer" {
            &self.sewer
        } else {
            &self.route
        }
    }

    /// The table a file with this `id` replaces, if it names one.
    pub fn by_id_mut(&mut self, id: &str) -> Option<&mut Loot> {
        match id {
            "route" => Some(&mut self.route),
            "sewer" => Some(&mut self.sewer),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_tables() -> LootTables {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("bin").join("last_route").join("data").join("loot");
        let mut t = LootTables::default();
        for file in ["route.json", "sewer.json"] {
            let text = std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
            let v: Value = serde_json::from_str(&text).expect("loot json");
            let id = v["id"].as_str().unwrap().to_string();
            *t.by_id_mut(&id).unwrap_or_else(|| panic!("{file}: no slot for {id}")) = Loot::parse(&v);
        }
        t
    }

    #[test]
    fn table_for_picks_sewer_only_for_the_sewer_region() {
        let t = data_tables();
        let sewer = t.table_for("sewer");
        assert_eq!(sewer.id, "sewer");
        assert_eq!(sewer.rolls, vec![("tool".to_string(), 0.45)]);
        assert!(sewer.backpacks.is_empty() && sewer.aid.is_empty());
        assert_eq!(t.table_for("apartment").id, "route");
        assert_eq!(t.table_for("").id, "route");
        assert_eq!(t.table_for("apartment").rolls.len(), 4);
    }

    #[test]
    fn content_load_has_both_tables() {
        let c = crate::content::Content::load();
        assert_eq!(c.loot.route.id, "route");
        assert_eq!(c.loot.sewer.id, "sewer");
    }

    #[test]
    fn by_id_mut_only_knows_route_and_sewer() {
        let mut t = LootTables::default();
        assert!(t.by_id_mut("route").is_some());
        assert!(t.by_id_mut("sewer").is_some());
        assert!(t.by_id_mut("maze").is_none());
    }
}