//! Dead Air's admin panel (port of lure_admin.gd) for the dev overlay: lure Xivvy now, force her to a room or a vent opening,
//! break or fix a system, seal or open a vent, set the temperature and the appliances, refill the air. The generic controls the
//! Godot game offers through its `admin_*` hooks (jump to a time, freeze, air as power, god, no scare, reveal, hunter speed and
//! levels, show a phantom) are rows here too. Every row is an id that `LureMode::admin_do` runs.
#![allow(dead_code)]

use super::data::{tuning, PHANTOM_IDS, ROOMS};
use crate::hud::admin::{AdminAction, ModeAdmin};

pub struct LureAdmin;

/// Every row of the panel as (label, id).
pub fn rows() -> Vec<(String, String)> {
    let t = tuning();
    let mut v: Vec<(String, String)> = Vec::new();
    let mut add = |label: String, id: String| v.push((label, id));
    for room in ROOMS {
        if !room.starts_with("vent") {
            add(format!("LURE XIVVY TO {}", t.label(room)), format!("lure:{room}"));
        }
    }
    add("RESET THE LURE COOLDOWN".into(), "lure_reset".into());
    for (label, room) in [
        ("XIVVY TO THE LEFT CRAWLSPACE", "ventl"),
        ("XIVVY TO THE RIGHT CRAWLSPACE", "ventr"),
        ("XIVVY AT THE LEFT VENT OPENING", "officeL"),
        ("XIVVY AT THE RIGHT VENT OPENING", "officeR"),
        ("XIVVY BACK TO RECEPTION", "lobby"),
    ] {
        add(label.into(), format!("place:{room}"));
    }
    for s in &t.systems {
        add(format!("BREAK {}", s.label), format!("sys:{}:break", s.id));
        add(format!("FIX {}", s.label), format!("sys:{}:fix", s.id));
    }
    for (label, id) in [
        ("SEAL LEFT VENT", "seal:L:on"),
        ("OPEN LEFT VENT", "seal:L:off"),
        ("SEAL RIGHT VENT", "seal:R:on"),
        ("OPEN RIGHT VENT", "seal:R:off"),
    ] {
        add(label.into(), id.into());
    }
    for (label, n) in [("40 F (FREEZING)", 40), ("58 F (COLD LINE)", 58), ("70 F", 70), ("96 F (HEAT LINE)", 96), ("115 F (ROASTING)", 115)] {
        add(label.into(), format!("temp:{n}"));
    }
    for (label, id) in [("HEATER ON", "app:heater:on"), ("HEATER OFF", "app:heater:off"), ("FAN ON", "app:fan:on"), ("FAN OFF", "app:fan:off")] {
        add(label.into(), id.into());
    }
    add("CLEAR THE COLD AND THE HEAT (70 F)".into(), "clear_climate".into());
    // the generic hooks
    for (label, id) in [
        ("FREEZE / RESUME THE CLOCK", "freeze"),
        ("GOD MODE ON / OFF", "god"),
        ("INFINITE AIR ON / OFF", "inf_air"),
        ("NO JUMPSCARE ON / OFF", "no_scare"),
        ("REVEAL THE HUNTERS ON / OFF", "reveal"),
        ("NOCLIP ON / OFF", "noclip"),
        ("AIR TO 100%", "air:100"),
        ("AIR TO 50%", "air:50"),
        ("HUNTER SPEED 0.5X", "speed:0.5"),
        ("HUNTER SPEED 1X", "speed:1"),
        ("HUNTER SPEED 2X", "speed:2"),
        ("HUNTER SPEED 4X", "speed:4"),
    ] {
        add(label.into(), id.into());
    }
    for h in 0..6 {
        add(format!("JUMP TO {}", crate::core::hour_label(h)), format!("hour:{h}"));
    }
    for who in ["xivvy", "ferryn", "briselle", "mavrine", "wynelle", "solyth"] {
        for n in [0, 5, 10, 20] {
            add(format!("{} AI {n}", who.to_uppercase()), format!("level:{who}:{n}"));
        }
    }
    for who in PHANTOM_IDS {
        add(format!("SHOW {} IN RECEPTION", who.to_uppercase()), format!("phantom:{who}:lobby"));
        add(format!("SHOW {} IN THE LIVE ROOM", who.to_uppercase()), format!("phantom:{who}:ballpit"));
    }
    v
}

impl ModeAdmin for LureAdmin {
    fn mode_id(&self) -> &'static str {
        "lure"
    }
    fn title(&self) -> String {
        "DEAD AIR SPECIFIC".to_string()
    }
    fn actions(&self) -> Vec<AdminAction> {
        rows().into_iter().map(|(l, i)| AdminAction::new(&l, &i)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::game::new_mode;
    use super::*;
    use crate::core::{Mode, Session};
    use std::collections::BTreeMap;

    #[test]
    fn the_panel_lists_unique_ids_and_a_title() {
        let a = LureAdmin;
        assert_eq!(a.mode_id(), "lure");
        assert_eq!(a.title(), "DEAD AIR SPECIFIC");
        let acts = a.actions();
        assert!(acts.len() > 60);
        let mut ids: Vec<&str> = acts.iter().map(|x| x.id.as_str()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "every row has its own id");
        assert!(acts.iter().any(|x| x.id == "lure:storage" && x.label == "LURE XIVVY TO TRANSMITTER ROOM"));
        assert!(!acts.iter().any(|x| x.id == "lure:ventl"), "a crawlspace is no lure target");
    }

    #[test]
    fn every_row_runs_without_trouble_and_the_registry_forwards_it() {
        let mut s = Session::new();
        s.start_mode("lure", 6, BTreeMap::new(), Some(31));
        let mut m = new_mode(&s);
        for a in LureAdmin.actions() {
            m.admin_do(&a.id);
        }
        // the default ModeAdmin::apply forwards an id it lists to Mode::admin_do and refuses a stranger
        let mut m2 = new_mode(&s);
        assert!(LureAdmin.apply("seal:R:on", &mut m2));
        assert!(!LureAdmin.apply("door_l_shut", &mut m2));
    }
}
