//! Salvage Run's admin panel (`dayshift_admin.gd`) as a `ModeAdmin`. The money and the quota, the rack's pieces, the upgrades,
//! the noise, the doors and their lights, the window (Ysolde's knock) and the door visitors on demand, plus the generic
//! switches the Godot game offered through its admin hooks (freeze the clock, god mode, infinite power, no scare, the hour,
//! the visitors' speed, the power). The mode's `admin_do` runs the ids.
//!
//! Registering (hub hook): `admin_reg.register(Box::new(crate::modes::dayshift::DayshiftAdmin));`
#![allow(dead_code)]

use crate::hud::admin::{AdminAction, ModeAdmin};

pub struct DayshiftAdmin;

/// The ids the mode understands, in panel order.
pub const IDS: &[(&str, &str)] = &[
    ("MONEY $0", "money_0"),
    ("MONEY $100", "money_100"),
    ("MONEY $250", "money_250"),
    ("MONEY = THE QUOTA", "money_quota"),
    ("QUOTA $100", "quota_100"),
    ("QUOTA $210 (WEEK 2)", "quota_210"),
    ("QUOTA $340 (WEEK 4)", "quota_340"),
    ("QUOTA $9999 (UNREACHABLE)", "quota_9999"),
    ("RACK: ADD SCRAP ($10)", "rack_scrap"),
    ("RACK: ADD PARTS ($25)", "rack_parts"),
    ("RACK: ADD A PRIZE ($60)", "rack_prize"),
    ("RACK: CLEAR THE RACK", "rack_clear"),
    ("UPGRADES: GRANT ALL FIVE", "upg_all"),
    ("UPGRADES: TAKE THEM ALL BACK", "upg_none"),
    ("NOISE: SILENT", "noise_0"),
    ("NOISE: UNEASY (30)", "noise_30"),
    ("NOISE: LOUD (50)", "noise_50"),
    ("NOISE: 100", "noise_100"),
    ("SHUT LEFT DOOR", "doorL_shut"),
    ("OPEN LEFT DOOR", "doorL_open"),
    ("SHUT RIGHT DOOR", "doorR_shut"),
    ("OPEN RIGHT DOOR", "doorR_open"),
    ("LEFT LIGHT ON", "lightL_on"),
    ("LEFT LIGHT OFF", "lightL_off"),
    ("RIGHT LIGHT ON", "lightR_on"),
    ("RIGHT LIGHT OFF", "lightR_off"),
    ("YSOLDE KNOCKS AT THE WINDOW NOW", "ysolde_near"),
    ("YSOLDE AT THE GLASS (BE QUIET)", "ysolde_window"),
    ("SEND YSOLDE AWAY", "ysolde_far"),
    ("BOTH VISITORS ONE STEP FROM THEIR DOORS", "all_to_doors"),
    ("FREEZE / UNFREEZE THE CLOCK", "toggle_freeze"),
    ("GOD MODE ON / OFF", "toggle_god"),
    ("INFINITE POWER ON / OFF", "toggle_infinite_power"),
    ("NO SCARE ON / OFF", "toggle_no_scare"),
    ("POWER 100%", "power_100"),
    ("POWER 0%", "power_0"),
    ("JUMP TO 3 AM", "hour_3"),
    ("JUMP TO 5 AM", "hour_5"),
    ("VISITORS' SPEED x1", "speed_1"),
    ("VISITORS' SPEED x2", "speed_2"),
];

impl ModeAdmin for DayshiftAdmin {
    fn mode_id(&self) -> &'static str {
        "dayshift"
    }

    fn title(&self) -> String {
        "SALVAGE RUN SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        IDS.iter().map(|(label, id)| AdminAction::new(label, id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_lists_every_row_once() {
        let a = DayshiftAdmin.actions();
        assert_eq!(a.len(), IDS.len());
        let mut ids: Vec<&str> = a.iter().map(|x| x.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), IDS.len(), "an id is used twice");
        assert_eq!(DayshiftAdmin.mode_id(), "dayshift");
        assert!(DayshiftAdmin.title().contains("SALVAGE RUN"));
        for want in ["money_quota", "rack_prize", "upg_all", "doorR_shut", "lightL_on", "ysolde_window", "all_to_doors"] {
            assert!(a.iter().any(|x| x.id == want), "{want}");
        }
    }
}
