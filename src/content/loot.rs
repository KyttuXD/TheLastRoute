//! `lr.loot/1` tables (data/loot/route.json). The finite route places one of every item
//! deterministically, so these weights matter for packs and the Infinite route; they are
//! loaded and validated so packs can override them.

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
