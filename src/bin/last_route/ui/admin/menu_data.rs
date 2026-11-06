//! The admin menu's entries and labels (data/admin/menu.json, `lr.admin_menu/1`): the GENERAL
//! item and teleport and speed lists, the PLAYER / HUNTERS / WORLD / STORY / VISUAL action rows,
//! the NPC op grids, the hunter picker's verbs and emotes, and the FIELDS page. Read once from the
//! embedded file; the owner gate and the menu's layout stay in code (see `menu.rs`).
//!
//! Strings are leaked once at first use, so the lists hand out `&'static str` ids the way the old
//! constants did and `Cmd` keeps its shape.

use crate::content::embedded;
use serde_json::Value;
use std::sync::OnceLock;

/// The parsed menu. Every list keeps the order of the JSON file, which is the order the rows show.
pub(super) struct MenuData {
    pub items: Vec<(&'static str, &'static str)>,
    pub teleports: Vec<(&'static str, &'static str)>,
    pub run_speeds: Vec<(f32, &'static str)>,
    pub game_speeds: Vec<(f32, &'static str)>,
    pub stories: Vec<(&'static str, &'static str)>,
    pub hunter_do: Vec<&'static str>,
    pub emotes: Vec<&'static str>,
    pub act_general: Vec<(&'static str, &'static str)>,
    pub act_player: Vec<(&'static str, &'static str)>,
    pub act_hunters: Vec<(&'static str, &'static str)>,
    pub act_world: Vec<(&'static str, &'static str)>,
    pub act_story: Vec<(&'static str, &'static str)>,
    pub act_visual: Vec<(&'static str, &'static str)>,
    pub npc_ops: Vec<(&'static str, &'static str)>,
    pub npc_all: Vec<(&'static str, &'static str)>,
    /// The NPC command ids this build runs (the rest are `npc_pending`).
    pub npc_live: Vec<&'static str>,
    /// The NPC commands that cannot act yet, each with the reason its tooltip shows.
    pub npc_pending: Vec<(&'static str, &'static str)>,
    /// Shown when an NPC button is gray because there is no NPC to act on.
    pub npc_no_npcs: &'static str,
    /// Put before a pending command's reason.
    pub npc_pending_prefix: &'static str,
    pub npc_smarts: Vec<i32>,
    /// The most NPCs one SPAWN NPC press makes (the count stepper stops here).
    pub npc_spawn_max: i32,
    pub fields: Vec<(&'static str, &'static str)>,
}

/// The admin menu data, parsed once from the embedded data/admin/menu.json. The file is compiled
/// in, and a test parses it, so a broken copy is a failed test rather than a silent fallback.
pub(super) fn menu() -> &'static MenuData {
    static MENU: OnceLock<MenuData> = OnceLock::new();
    MENU.get_or_init(|| {
        let (path, text) = embedded::data_under("admin/menu.json")
            .next()
            .expect("data/admin/menu.json is embedded");
        let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{path}: {e}"));
        parse(&v).unwrap_or_else(|e| panic!("{path}: {e}"))
    })
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// The `{id, label}` list at `key`, or the `{label, placeholder}` list when `a` is "label".
fn pairs(v: &Value, key: &str, a: &str, b: &str) -> Result<Vec<(&'static str, &'static str)>, String> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} is not a list"))?;
    arr.iter()
        .map(|e| -> Result<(&'static str, &'static str), String> {
            let x = e
                .get(a)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{key}: an entry has no {a}"))?;
            let y = e
                .get(b)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{key}: an entry has no {b}"))?;
            Ok((leak(x), leak(y)))
        })
        .collect()
}

fn strings(v: &Value, key: &str) -> Result<Vec<&'static str>, String> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} is not a list"))?;
    arr.iter()
        .map(|e| -> Result<&'static str, String> {
            e.as_str()
                .map(leak)
                .ok_or_else(|| format!("{key}: an entry is not a string"))
        })
        .collect()
}

fn speeds(v: &Value, key: &str) -> Result<Vec<(f32, &'static str)>, String> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} is not a list"))?;
    arr.iter()
        .map(|e| -> Result<(f32, &'static str), String> {
            let value = e
                .get("value")
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("{key}: an entry has no value"))?;
            let label = e
                .get("label")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{key}: an entry has no label"))?;
            Ok((value as f32, leak(label)))
        })
        .collect()
}

/// Parses data/admin/menu.json (`lr.admin_menu/1`). Any missing list is an error.
pub(super) fn parse(v: &Value) -> Result<MenuData, String> {
    let npc_smarts = v
        .get("npc_smarts")
        .and_then(Value::as_array)
        .ok_or("npc_smarts is not a list")?
        .iter()
        .map(|e| {
            e.as_i64()
                .map(|n| n as i32)
                .ok_or_else(|| "npc_smarts: an entry is not a whole number".to_string())
        })
        .collect::<Result<Vec<i32>, String>>()?;
    let npc_spawn_max = v
        .get("npc_spawn_max")
        .and_then(Value::as_i64)
        .filter(|n| *n >= 1)
        .ok_or("npc_spawn_max is not a whole number of at least 1")? as i32;
    if npc_smarts.len() != 4 {
        return Err("npc_smarts needs exactly 4 entries (the SMART dropdown)".to_string());
    }
    let npc_ops = pairs(v, "npc_ops", "id", "label")?;
    let npc_all = pairs(v, "npc_all", "id", "label")?;
    let npc_live = strings(v, "npc_live")?;
    let npc_pending = pairs(v, "npc_pending", "id", "reason")?;
    for (id, reason) in &npc_pending {
        if reason.trim().is_empty() {
            return Err(format!("npc_pending: {id} has an empty reason"));
        }
    }
    for (id, _) in npc_ops.iter().chain(npc_all.iter()) {
        let live = npc_live.contains(id);
        let pending = npc_pending.iter().any(|(p, _)| p == id);
        if live == pending {
            return Err(format!("npc command {id} must be in exactly one of npc_live and npc_pending"));
        }
    }
    for id in npc_live.iter().copied().chain(npc_pending.iter().map(|(id, _)| *id)) {
        if !npc_ops.iter().chain(npc_all.iter()).any(|(o, _)| *o == id) {
            return Err(format!("npc_live / npc_pending names {id}, which is not a button"));
        }
    }
    let text = |key: &str| {
        v.get("npc_text")
            .and_then(|t| t.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(leak)
            .ok_or_else(|| format!("npc_text.{key} is missing"))
    };
    let npc_no_npcs = text("no_npcs")?;
    let npc_pending_prefix = text("pending")?;
    if v.get("run_speeds").and_then(Value::as_array).map_or(0, Vec::len) < 1
        || v.get("game_speeds").and_then(Value::as_array).map_or(0, Vec::len) < 1
    {
        return Err("run_speeds and game_speeds need at least one entry".to_string());
    }
    Ok(MenuData {
        items: pairs(v, "items", "id", "label")?,
        teleports: pairs(v, "teleports", "id", "label")?,
        run_speeds: speeds(v, "run_speeds")?,
        game_speeds: speeds(v, "game_speeds")?,
        stories: pairs(v, "stories", "id", "label")?,
        hunter_do: strings(v, "hunter_do")?,
        emotes: strings(v, "emotes")?,
        act_general: pairs(v, "act_general", "id", "label")?,
        act_player: pairs(v, "act_player", "id", "label")?,
        act_hunters: pairs(v, "act_hunters", "id", "label")?,
        act_world: pairs(v, "act_world", "id", "label")?,
        act_story: pairs(v, "act_story", "id", "label")?,
        act_visual: pairs(v, "act_visual", "id", "label")?,
        npc_ops,
        npc_all,
        npc_live,
        npc_pending,
        npc_no_npcs,
        npc_pending_prefix,
        npc_smarts,
        npc_spawn_max,
        fields: pairs(v, "fields", "label", "placeholder")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The Standard lists: the exact entries and order the menu had before the data move.
    #[test]
    fn embedded_menu_matches_the_standard_lists() {
        let m = menu();
        assert_eq!(m.items.len(), 11);
        assert_eq!(m.items[0], ("flashlight", "FLASHLIGHT"));
        assert_eq!(m.items[10], ("backpack_large", "LARGE BACKPACK"));
        assert_eq!(m.teleports.len(), 8);
        assert_eq!(m.teleports[7], ("checkpoint", "CHECKPOINT"));
        assert_eq!(m.run_speeds, vec![(1.0, "1X"), (1.5, "1.5X"), (2.0, "2X"), (3.0, "3X")]);
        assert_eq!(m.game_speeds.len(), 6);
        assert_eq!(m.game_speeds[0], (0.0, "PAUSE"));
        assert_eq!(m.game_speeds[5], (-1.0, "STEP 1 FRAME"));
        assert_eq!(m.stories.len(), 3);
        assert_eq!(m.stories[1], ("first_signal", "FIRST SIGNAL"));
        assert_eq!(m.hunter_do.len(), 4);
        assert_eq!(m.hunter_do[2], "TOGGLE (SUMMON / REMOVE ALL MATCHES)");
        assert_eq!(m.emotes.len(), 14);
        assert_eq!(m.emotes[13], "VICTORY");
        assert_eq!(m.act_general.len(), 7);
        assert_eq!(m.act_general[0], ("roster_refresh", "REFRESH ADMIN ROSTER"));
        assert_eq!(m.act_player.len(), 3);
        assert_eq!(m.act_hunters.len(), 7);
        assert_eq!(m.act_world.len(), 6);
        assert_eq!(m.act_story.len(), 2);
        assert_eq!(m.act_visual.len(), 4);
        assert_eq!(m.npc_ops.len(), 17);
        assert_eq!(m.npc_ops[10], ("smart", "SMART = SPAWN SMART"));
        assert_eq!(m.npc_all.len(), 11);
        assert_eq!(m.npc_all[10], ("all_smart", "ALL SMART"));
        assert_eq!(m.npc_smarts, vec![30, 50, 70, 90]);
        assert_eq!(m.npc_spawn_max, 32);
        assert_eq!(m.fields.len(), 3);
        assert_eq!(m.fields[0], ("SAFE TELEPORT", "X,Z or X,Z,LEVEL"));
        assert_eq!(m.fields[2], ("REMOVE LIMITED ADMIN", "USERNAME"));
    }

    /// Changing a value in the data changes what the menu holds (the labels, ids and numbers).
    #[test]
    fn changing_the_data_changes_the_menu() {
        let mut v: Value = serde_json::from_str(embedded::data_under("admin/menu.json").next().unwrap().1).unwrap();
        v["act_general"][0]["label"] = json!("REFRESH THE ROSTER");
        v["npc_smarts"] = json!([10, 20, 40, 80]);
        v["run_speeds"][3]["value"] = json!(5.0);
        let m = parse(&v).unwrap();
        assert_eq!(m.act_general[0].1, "REFRESH THE ROSTER");
        assert_eq!(m.act_general[0].0, "roster_refresh");
        assert_eq!(m.npc_smarts, vec![10, 20, 40, 80]);
        assert_eq!(m.run_speeds[3].0, 5.0);
    }

    /// Every id is a bare lowercase word and every label is shown capitalised or lowercase text the
    /// player can read: the lists are (id, label), so a swapped order shows ids on the buttons.
    #[test]
    fn ids_are_ids_and_labels_are_labels() {
        let m = menu();
        let lists = [&m.items, &m.teleports, &m.stories, &m.act_general, &m.act_player, &m.act_hunters,
            &m.act_world, &m.act_story, &m.act_visual, &m.npc_ops, &m.npc_all];
        for list in lists {
            for (id, label) in list.iter() {
                assert!(id.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "id {id:?}");
                assert!(label.chars().any(|c| c.is_ascii_uppercase()), "label {label:?} reads as an id");
                assert_ne!(id, label);
            }
        }
    }

    #[test]
    fn ids_are_unique_within_a_list() {
        let m = menu();
        for list in [&m.items, &m.teleports, &m.act_general, &m.act_hunters, &m.act_world, &m.npc_ops, &m.npc_all] {
            let mut ids: Vec<&str> = list.iter().map(|(id, _)| *id).collect();
            ids.sort();
            let n = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), n);
        }
    }

    #[test]
    fn bad_limits_in_the_data_are_errors() {
        let base: Value = serde_json::from_str(embedded::data_under("admin/menu.json").next().unwrap().1).unwrap();
        let mut v = base.clone();
        v["npc_spawn_max"] = json!(0);
        assert!(parse(&v).is_err());
        let mut v = base.clone();
        v["npc_smarts"] = json!([30, 50]);
        assert!(parse(&v).is_err());
        let mut v = base;
        v["game_speeds"] = json!([]);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn every_npc_button_is_live_or_pending_with_a_reason() {
        let m = menu();
        let all: Vec<&str> = m.npc_ops.iter().chain(m.npc_all.iter()).map(|(id, _)| *id).collect();
        assert_eq!(m.npc_live.len() + m.npc_pending.len(), all.len());
        for id in &all {
            let live = m.npc_live.contains(id);
            let reason = m.npc_pending.iter().find(|(p, _)| p == id).map(|(_, r)| *r);
            assert!(live != reason.is_some(), "{id}");
            if let Some(r) = reason {
                assert!(r.len() > 12, "{id}: {r:?}");
            }
        }
        assert!(m.npc_no_npcs.contains("SPAWN NPC"));
    }

    #[test]
    fn an_npc_command_in_both_lists_or_neither_is_an_error() {
        let base: Value = serde_json::from_str(embedded::data_under("admin/menu.json").next().unwrap().1).unwrap();
        let mut v = base.clone();
        v["npc_live"].as_array_mut().unwrap().push(json!("goto"));
        assert!(parse(&v).is_err(), "goto is pending");
        let mut v = base.clone();
        v["npc_live"].as_array_mut().unwrap().retain(|e| e != "smart");
        assert!(parse(&v).is_err(), "smart is in neither list");
        let mut v = base.clone();
        v["npc_live"].as_array_mut().unwrap().push(json!("dance"));
        assert!(parse(&v).is_err(), "dance is not a button");
        let mut v = base;
        v["npc_pending"][0]["reason"] = json!("  ");
        assert!(parse(&v).is_err(), "a pending command needs its reason");
    }

    #[test]
    fn a_list_missing_from_the_data_is_an_error() {
        let mut v: Value = serde_json::from_str(embedded::data_under("admin/menu.json").next().unwrap().1).unwrap();
        v.as_object_mut().unwrap().remove("emotes");
        assert!(parse(&v).is_err());
    }
}
