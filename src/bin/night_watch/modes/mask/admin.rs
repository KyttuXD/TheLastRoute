#![allow(dead_code)]
//! Masked Hours' admin panel (port of `mask_admin.gd`): the mask on / off and Thessine's mask timer, the music box, the vent
//! lights and the hall light, the desk lamp, and the hall hunters and vent hunters on demand. It opens only for an account
//! the server marks `admin` (hud/admin.rs `AdminGate`); the rows are run by `MaskMode::admin_do`, and a used row marks the run
//! DEBUG so nothing is saved. The generic rows (jump to an hour, battery, god, freeze, reveal, speed, calls) come from
//! `MaskMode::admin_entries`.

use crate::hud::admin::{AdminAction, ModeAdmin};

pub struct MaskAdmin;

impl ModeAdmin for MaskAdmin {
    fn mode_id(&self) -> &'static str {
        "mask"
    }

    fn title(&self) -> String {
        "MASKED HOURS SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        vec![
            AdminAction::new("PUT THE MASK ON", "mask_on"),
            AdminAction::new("TAKE THE MASK OFF", "mask_off"),
            AdminAction::new("MASK TIMER 0 (JUST ARRIVED)", "timer_0"),
            AdminAction::new("MASK TIMER 1000", "timer_1000"),
            AdminAction::new("MASK TIMER 2000", "timer_2000"),
            AdminAction::new("MASK TIMER 2400 (ABOUT TO LEAVE)", "timer_2400"),
            AdminAction::new("MUSIC BOX WOUND (100%)", "box_100"),
            AdminAction::new("MUSIC BOX 50%", "box_50"),
            AdminAction::new("MUSIC BOX 10%", "box_10"),
            AdminAction::new("MUSIC BOX EMPTY (GLYRA COMES IN 6 S)", "box_0"),
            AdminAction::new("LEFT VENT LIGHT ON", "lightL_on"),
            AdminAction::new("LEFT VENT LIGHT OFF", "lightL_off"),
            AdminAction::new("RIGHT VENT LIGHT ON", "lightR_on"),
            AdminAction::new("RIGHT VENT LIGHT OFF", "lightR_off"),
            AdminAction::new("HALL LIGHT ON", "hall_on"),
            AdminAction::new("HALL LIGHT OFF", "hall_off"),
            AdminAction::new("FLASH THE HALL LIGHT ONCE", "flash_hall"),
            AdminAction::new("TOGGLE THE DESK LAMP", "lamp"),
            AdminAction::new("THESSINE INTO THE OFFICE", "thessine_office"),
            AdminAction::new("UMBRIEL RUNS THE MAIN HALL", "umbriel_hall"),
            AdminAction::new("VERRINE TO THE LEFT VENT", "verrine_ventL"),
            AdminAction::new("ALL HUNTERS HOME", "all_home"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Mode, Session};
    use super::super::game::MaskMode;

    #[test]
    fn the_panel_belongs_to_the_mask_mode_and_every_row_is_understood() {
        let p = MaskAdmin;
        assert_eq!(p.mode_id(), "mask");
        assert!(p.title().contains("MASKED HOURS"));
        let rows = p.actions();
        let mut ids: Vec<&str> = rows.iter().map(|a| a.id.as_str()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(n, ids.len(), "ids are unique");
        let mut s = Session::new();
        s.start_mode("mask", 4, Default::default(), Some(3));
        let mut m = MaskMode::new(&s);
        // every row runs without panicking, through the trait's default apply
        for a in &rows {
            assert!(p.apply(&a.id, &mut m as &mut dyn Mode), "{}", a.id);
        }
        assert!(!p.apply("not_a_row", &mut m as &mut dyn Mode));
    }
}
