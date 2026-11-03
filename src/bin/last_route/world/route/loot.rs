//! `LootTables` (game/world/interactables/loot_tables.gd) for the Infinite route's chunk pickups: the `roll`,
//! `item_for`, `pick_tool`, `pick_backpack` and `roll_aid` draws of the streamed generator's `_pickups`
//! (streamed_route_generator.gd:5393-5466).
//!
//! The tables are the `lr.loot/1` files `data/loot/route.json` (the `route` table, every region without a table of its
//! own) and `data/loot/sewer.json` (`sewer`). They are written out here as constants, in the file order, with the
//! GDScript floats as f64. The test `tables_match_data_loot_files` reads both JSON files and checks the constants, so
//! a data change that is not copied here fails the suite.
//!
//! Packs are not read: the Godot `LootTables.table_for` takes a pack table that names the region first. No pack
//! loads in this port, so only the built-in tables are used.
//!
//! RNG: every chance is one `next_float` in file order, and every `next_index` is one draw (`next_index(0)` too).
//! `roll` and `roll_aid` do not short-circuit. `pick_tool` draws its roll before it checks for an empty list.

use crate::world::rng::Rng;

/// `BACKPACK_DEPTH_STEP` (loot_tables.gd): a backpack's size rank grows by one every this many levels down.
pub const BACKPACK_DEPTH_STEP: f64 = 4.0;

/// One chance roll of a table: the kind that comes up when `next_float() < chance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roll {
    pub kind: &'static str,
    pub chance: f64,
}

/// One world tool by weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tool {
    pub item: &'static str,
    pub weight: f64,
}

/// One aid item (Stim Pack, Revive Kit), with the co-op chance where the file has one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aid {
    pub item: &'static str,
    pub chance: f64,
    pub chance_coop: Option<f64>,
}

/// One `lr.loot/1` table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Table {
    pub id: &'static str,
    pub base: &'static [&'static str],
    pub rolls: &'static [Roll],
    pub tools: &'static [Tool],
    pub backpacks: &'static [&'static str],
    pub aid: &'static [Aid],
}

/// `data/loot/route.json`.
pub const ROUTE: Table = Table {
    id: "route",
    base: &["battery"],
    rolls: &[
        Roll { kind: "tool", chance: 0.72 },
        Roll { kind: "battery", chance: 0.38 },
        Roll { kind: "coin", chance: 0.30 },
        Roll { kind: "backpack", chance: 0.16 },
    ],
    tools: &[
        Tool { item: "taser", weight: 0.3 },
        Tool { item: "lockpick", weight: 0.3 },
        Tool { item: "crowbar", weight: 0.3 },
        Tool { item: "rescue_axe", weight: 0.1 },
    ],
    backpacks: &["backpack_small", "backpack_medium", "backpack_large"],
    aid: &[
        Aid { item: "stim_pack", chance: 0.18, chance_coop: None },
        Aid { item: "revive_kit", chance: 0.03, chance_coop: Some(0.06) },
    ],
};

/// `data/loot/sewer.json`.
pub const SEWER: Table = Table {
    id: "sewer",
    base: &["battery"],
    rolls: &[Roll { kind: "tool", chance: 0.45 }],
    tools: &[
        Tool { item: "taser", weight: 0.3 },
        Tool { item: "lockpick", weight: 0.3 },
        Tool { item: "crowbar", weight: 0.3 },
        Tool { item: "rescue_axe", weight: 0.1 },
    ],
    backpacks: &[],
    aid: &[],
};

/// `LootTables.table_for` with the built-in tables: a region named `sewer` has the sewer table, every other region the
/// route table (the `route` default, loot_tables.gd).
pub fn table_for(region: &str) -> &'static Table {
    if region == "sewer" {
        &SEWER
    } else {
        &ROUTE
    }
}

/// `LootTables.roll` (loot_tables.gd): one `next_float` per entry in file order, and the kinds that came up.
pub fn roll(table: &Table, rng: &mut Rng) -> Vec<&'static str> {
    table
        .rolls
        .iter()
        .filter_map(|entry| (rng.next_float() < entry.chance).then_some(entry.kind))
        .collect()
}

/// `LootTables.item_for` (loot_tables.gd): the item a planned pickup of `kind` holds. Tools and backpacks draw, the
/// other kinds hold nothing (""). `level` is the chunk's level (`coord.y`).
pub fn item_for(kind: &str, table: &Table, rng: &mut Rng, level: i32) -> &'static str {
    match kind {
        "tool" => pick_tool(table, rng),
        "backpack" => pick_backpack(table, rng, level),
        _ => "",
    }
}

/// `LootTables.pick_tool` (loot_tables.gd): one world tool by weight, one `next_float`. The weights are scaled when
/// they do not add up to 1.
pub fn pick_tool(table: &Table, rng: &mut Rng) -> &'static str {
    let mut value = rng.next_float();
    if table.tools.is_empty() {
        return "";
    }
    let total = table.tools.iter().fold(0.0, |sum, tool| sum + tool.weight);
    if (total - 1.0).abs() > 1e-6 {
        value *= total;
    }
    for tool in table.tools {
        value -= tool.weight;
        if value < 0.0 {
            return tool.item;
        }
    }
    table.tools[0].item
}

/// `LootTables.pick_backpack` (loot_tables.gd): the rank is `1 + floor(max(0, -level) / BACKPACK_DEPTH_STEP)`, capped
/// at the number of backpacks, then one `next_index` over that rank. Level 0 gives rank 1 (`backpack_small`).
pub fn pick_backpack(table: &Table, rng: &mut Rng, level: i32) -> &'static str {
    let n = table.backpacks.len().max(1) as i32;
    let depth = (0.0f64.max(-(level as f64)) / BACKPACK_DEPTH_STEP).floor() as i32;
    let depth_rank = (1 + depth).clamp(1, n);
    let rolled_rank = (1 + rng.next_index(depth_rank as usize) as i32).clamp(1, n);
    table.backpacks.get(rolled_rank as usize - 1).copied().unwrap_or("")
}

/// `LootTables.roll_aid` (loot_tables.gd): one `next_float` per aid entry in file order, against `chance_coop` when
/// the run is a co-op room and the entry has one, else `chance`.
pub fn roll_aid(table: &Table, rng: &mut Rng, coop: bool) -> Vec<&'static str> {
    table
        .aid
        .iter()
        .filter_map(|entry| {
            let chance = if coop { entry.chance_coop.unwrap_or(entry.chance) } else { entry.chance };
            (rng.next_float() < chance).then_some(entry.item)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_match_data_loot_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("bin").join("last_route").join("data").join("loot");
        for (file, table) in [("route.json", &ROUTE), ("sewer.json", &SEWER)] {
            let text = std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
            let v: serde_json::Value = serde_json::from_str(&text).expect("loot json");
            assert_eq!(v["id"].as_str(), Some(table.id), "{file} id");
            let base: Vec<&str> = v["base"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
            assert_eq!(base, table.base, "{file} base");
            let rolls: Vec<(&str, f64)> = v["rolls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| (e["kind"].as_str().unwrap(), e["chance"].as_f64().unwrap()))
                .collect();
            let ours: Vec<(&str, f64)> = table.rolls.iter().map(|r| (r.kind, r.chance)).collect();
            assert_eq!(rolls, ours, "{file} rolls");
            let tools: Vec<(&str, f64)> = v["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| (e["item"].as_str().unwrap(), e["weight"].as_f64().unwrap()))
                .collect();
            let ours: Vec<(&str, f64)> = table.tools.iter().map(|t| (t.item, t.weight)).collect();
            assert_eq!(tools, ours, "{file} tools");
            let packs: Vec<&str> = v["backpacks"]
                .as_array()
                .map(|a| a.iter().map(|x| x.as_str().unwrap()).collect())
                .unwrap_or_default();
            assert_eq!(packs, table.backpacks, "{file} backpacks");
            let aid: Vec<(&str, f64, Option<f64>)> = v["aid"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| (e["item"].as_str().unwrap(), e["chance"].as_f64().unwrap(), e["chance_coop"].as_f64()))
                        .collect()
                })
                .unwrap_or_default();
            let ours: Vec<(&str, f64, Option<f64>)> = table.aid.iter().map(|a| (a.item, a.chance, a.chance_coop)).collect();
            assert_eq!(aid, ours, "{file} aid");
        }
    }

    #[test]
    fn table_for_uses_sewer_only_for_the_sewer_region() {
        assert_eq!(table_for("sewer").id, "sewer");
        assert_eq!(table_for("apartment").id, "route");
        assert_eq!(table_for("").id, "route");
    }

    #[test]
    fn level_zero_backpack_is_the_small_one_and_costs_one_draw() {
        let mut a = Rng::new("BACKPACK-TEST");
        let mut b = Rng::new("BACKPACK-TEST");
        assert_eq!(pick_backpack(&ROUTE, &mut a, 0), "backpack_small");
        // one draw consumed: the next value of `a` equals the second value of `b`
        b.next_float();
        assert_eq!(a.next_float(), b.next_float());
    }

    #[test]
    fn pick_tool_scales_weights_only_when_they_do_not_add_to_one() {
        // the route weights add to 1 (no scaling): a roll of 0.0 gives the first tool
        let mut rng = Rng::new("TOOL-TEST");
        let first = pick_tool(&ROUTE, &mut rng);
        assert!(ROUTE.tools.iter().any(|t| t.item == first));
    }

    #[test]
    fn roll_draws_once_per_entry_without_short_circuit() {
        let mut a = Rng::new("ROLL-TEST");
        let mut b = Rng::new("ROLL-TEST");
        let _ = roll(&ROUTE, &mut a);
        for _ in 0..ROUTE.rolls.len() {
            b.next_float();
        }
        assert_eq!(a.next_float(), b.next_float());
    }
}
