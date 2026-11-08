//! Bedtime's admin panel (bedroom_admin.gd plus the generic hooks BedroomGame offered the shared admin menu). Opens only through
//! the `AdminGate` (an account the server marks admin). Every row runs `BedroomMode::admin_do(id)`, which really changes the game.
#![allow(dead_code)]

use crate::hud::admin::{AdminAction, ModeAdmin};

/// Register with `admin_reg.register(Box::new(crate::modes::bedroom::BedroomAdmin));`.
pub struct BedroomAdmin;

impl ModeAdmin for BedroomAdmin {
    fn mode_id(&self) -> &'static str {
        "bedroom"
    }

    fn title(&self) -> String {
        "BEDTIME SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        let mut out = Vec::new();
        // which hand is down (one at a time)
        out.push(AdminAction::new("HOLD THE CLOSET DOOR", "hand_hold"));
        out.push(AdminAction::new("LIGHT ON THE HALL", "hand_hall"));
        out.push(AdminAction::new("LIGHT ON THE BED", "hand_bed"));
        out.push(AdminAction::new("NO HAND DOWN (DARK)", "hand_none"));
        // Haelith's creep (the sleep meter)
        for (label, v) in [("0% (FAR OFF)", 0), ("25%", 25), ("50% (SHE STIRS)", 50), ("75%", 75), ("99% (AT THE BED)", 99)] {
            out.push(AdminAction::new(&format!("CREEP {label}"), &format!("creep_{v}")));
        }
        out.push(AdminAction::new("MAKE THE HOUSE RESTLESS NOW", "restless"));
        // the clock and the battery
        for h in 0..6 {
            out.push(AdminAction::new(&format!("SET HOUR {}", crate::core::hour_label(h)), &format!("hour_{h}")));
        }
        for b in [0, 25, 50, 100] {
            out.push(AdminAction::new(&format!("BATTERY {b}%"), &format!("battery_{b}")));
        }
        // switches
        out.push(AdminAction::new("TOGGLE FREEZE THE CLOCK", "freeze"));
        out.push(AdminAction::new("TOGGLE INFINITE BATTERY", "infinite_battery"));
        out.push(AdminAction::new("TOGGLE GOD MODE", "god"));
        out.push(AdminAction::new("TOGGLE NO SCARE", "no_scare"));
        out.push(AdminAction::new("TOGGLE REVEAL (LIGHT THE ROOM)", "reveal"));
        for s in ["0.5", "1", "2", "4"] {
            out.push(AdminAction::new(&format!("HUNTER SPEED {s}X"), &format!("speed_{s}")));
        }
        // who stands where
        for (h, name) in [("loxelle", "LOXELLE"), ("javelle", "JAVELLE"), ("haelith", "HAELITH")] {
            for (st, label) in [("away", "AWAY"), ("breath", "BREATH"), ("near", "NEAR")] {
                out.push(AdminAction::new(&format!("{name} {label}"), &format!("place_{h}_{st}")));
            }
        }
        out.push(AdminAction::new("PLAY ZELLIA'S CALL", "call_play"));
        out.push(AdminAction::new("SKIP ZELLIA'S CALL", "call_skip"));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Mode, Session};
    use super::super::game::BedroomMode;
    use std::collections::BTreeMap;

    #[test]
    fn every_row_has_a_unique_id_and_label() {
        let a = BedroomAdmin.actions();
        assert!(a.len() >= 40, "{}", a.len());
        let mut ids: Vec<&str> = a.iter().map(|x| x.id.as_str()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate ids");
        assert!(a.iter().all(|x| !x.label.is_empty()));
        assert_eq!(BedroomAdmin.mode_id(), "bedroom");
        assert_eq!(BedroomAdmin.title(), "BEDTIME SPECIFIC");
    }

    #[test]
    fn the_panel_rows_of_bedroom_admin_gd_are_there() {
        let a = BedroomAdmin.actions();
        for id in ["hand_hold", "hand_hall", "hand_bed", "hand_none", "creep_0", "creep_25", "creep_50", "creep_75", "creep_99", "restless"] {
            assert!(a.iter().any(|x| x.id == id), "{id}");
        }
    }

    #[test]
    fn applying_a_row_changes_the_mode() {
        let mut s = Session::new();
        s.start_mode("bedroom", 3, BTreeMap::new(), Some(5));
        let mut m = BedroomMode::new(&s);
        let panel = BedroomAdmin;
        assert!(panel.apply("hand_hold", &mut m));
        assert_eq!(m.held(), [true, false, false]);
        assert!(panel.apply("creep_50", &mut m));
        assert_eq!(m.sim().logic.haelith.p, 50.0);
        assert!(!panel.apply("not_a_row", &mut m));
    }

    #[test]
    fn every_row_id_is_understood_by_the_mode() {
        let mut s = Session::new();
        s.start_mode("bedroom", 4, BTreeMap::new(), Some(9));
        for row in BedroomAdmin.actions() {
            let mut m = BedroomMode::new(&s);
            m.admin_do(&row.id);
        }
    }
}
