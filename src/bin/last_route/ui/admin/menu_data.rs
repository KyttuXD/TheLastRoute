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
    pub npc_smarts: Vec<i32>,
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
        npc_ops: pairs(v, "npc_ops", "id", "label")?,
        npc_all: pairs(v, "npc_all", "id", "label")?,
        npc_smarts,
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

    #[test]
    fn a_list_missing_from_the_data_is_an_error() {
        let mut v: Value = serde_json::from_str(embedded::data_under("admin/menu.json").next().unwrap().1).unwrap();
        v.as_object_mut().unwrap().remove("emotes");
        assert!(parse(&v).is_err());
    }
}
