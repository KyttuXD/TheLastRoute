//! The Descent's admin panel (`descent_admin.gd` plus the generic hooks `descent_game.gd` offers): the elevator's depth (which
//! stage is up), solving or dealing the stage again, the wrong-attempt count, Elise's closeness and window, the door hold,
//! and the generic switches (god, freeze, no scare, reveal, her speed and AI level, place, Zellia's call). `@win` and
//! `@lose` come from the shared rows. Opens only for an admin account (`hud::admin::AdminGate`); a used row marks the run
//! DEBUG, so nothing is saved.
#![allow(dead_code)]

use super::game::DescentMode;
use super::sim::Over;
use crate::core::AdminEntry;
use crate::hud::admin::{AdminAction, ModeAdmin};

/// The panel registered with `AdminRegistry` (`reg.register(Box::new(DescentAdmin));`).
pub struct DescentAdmin;

impl ModeAdmin for DescentAdmin {
    fn mode_id(&self) -> &'static str {
        "descent"
    }

    fn title(&self) -> String {
        "THE DESCENT SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        vec![
            AdminAction::new("SOLVE THIS STAGE", "solve"),
            AdminAction::new("DEAL THIS PUZZLE AGAIN", "redeal"),
            AdminAction::new("WRONG ATTEMPTS: ADD ONE", "wrong_add"),
            AdminAction::new("WRONG ATTEMPTS: CLEAR THE COUNT", "wrong_clear"),
            AdminAction::new("ELISE'S CLOSENESS 0% (OUT OF SIGHT)", "close_0"),
            AdminAction::new("ELISE'S CLOSENESS 30%", "close_30"),
            AdminAction::new("ELISE'S CLOSENESS 60% (RED BAR)", "close_60"),
            AdminAction::new("ELISE'S CLOSENESS 90% (AT THE GLASS)", "close_90"),
            AdminAction::new("HOLD THE DOOR (LATCH ON/OFF)", "hold"),
        ]
    }
}

fn onoff(b: bool) -> &'static str {
    if b {
        "ON"
    } else {
        "OFF"
    }
}

/// The dynamic rows: the status, the floors of this descent, and the generic switches.
pub fn entries(m: &DescentMode) -> Vec<AdminEntry> {
    let s = &m.sim;
    let mut out = Vec::new();
    out.push(AdminEntry::heading(&format!(
        "DESCENT {}{}  FLOOR {} / {}  {}",
        m.descent,
        if m.custom { " (CUSTOM)" } else { "" },
        (s.stage_idx + 1).min(s.stages.len()),
        s.stages.len(),
        match s.over {
            Over::None => s.kind().map(|k| k.id().to_uppercase()).unwrap_or_default(),
            Over::Win => "WON".to_string(),
            Over::Caught => "CAUGHT".to_string(),
        }
    )));
    out.push(AdminEntry::heading(&format!(
        "ELISE  AI {}  CLOSENESS {}%  WINDOW {}  WRONG {}",
        s.level,
        s.threat.round() as i32,
        s.side.id(),
        s.wrong
    )));
    out.push(AdminEntry::heading("ELEVATOR DEPTH"));
    for (i, st) in s.stages.iter().enumerate() {
        out.push(AdminEntry::button(&format!("FLOOR {}: {}", i + 1, st.name()), &format!("floor_{}", i + 1)));
    }
    out.push(AdminEntry::heading("SWITCHES"));
    out.push(AdminEntry::button(&format!("GOD MODE: {}", onoff(s.god)), "god"));
    out.push(AdminEntry::button(&format!("FREEZE THE CLOCK: {}", onoff(m.frozen)), "freeze"));
    out.push(AdminEntry::button(&format!("NO SCARE: {}", onoff(m.no_scare)), "noscare"));
    out.push(AdminEntry::button(&format!("REVEAL ELISE: {}", onoff(m.reveal)), "reveal"));
    out.push(AdminEntry::heading(&format!("ELISE'S SPEED  {:.1}X", s.speed_mult)));
    for (label, id) in [("0.5X", "speed_0.5"), ("1X", "speed_1"), ("2X", "speed_2"), ("4X", "speed_4")] {
        out.push(AdminEntry::button(label, id));
    }
    out.push(AdminEntry::heading("ELISE'S AI LEVEL"));
    for (label, id) in [("0", "ai_0"), ("5", "ai_5"), ("10", "ai_10"), ("15", "ai_15"), ("20", "ai_20")] {
        out.push(AdminEntry::button(label, id));
    }
    out.push(AdminEntry::heading("ELISE'S PLACE"));
    for (label, id) in [("FAR", "place_far"), ("STEP 2", "place_step2"), ("STEP 1", "place_step1"), ("AT THE EDGE", "place_edge")] {
        out.push(AdminEntry::button(label, id));
    }
    out.push(AdminEntry::heading("ZELLIA"));
    out.push(AdminEntry::button("PLAY THIS DESCENT'S CALL", "call_play"));
    out.push(AdminEntry::button("SKIP THE CALL", "call_skip"));
    out
}

/// Closeness of each place (`admin_place_hunter`).
pub fn place_percent(id: &str) -> Option<f64> {
    match id {
        "place_far" => Some(0.0),
        "place_step2" => Some(40.0),
        "place_step1" => Some(70.0),
        "place_edge" => Some(97.0),
        _ => None,
    }
}

/// Run a row by its id. Events of the sim are queued so the shift plays their sounds on its next frame.
pub fn run(m: &mut DescentMode, id: &str) {
    if let Some(n) = id.strip_prefix("floor_").and_then(|n| n.parse::<i32>().ok()) {
        m.sim.admin_goto_stage(n);
        return;
    }
    if let Some(n) = id.strip_prefix("close_").and_then(|n| n.parse::<f64>().ok()) {
        m.sim.admin_set_threat(n);
        return;
    }
    if let Some(n) = id.strip_prefix("speed_").and_then(|n| n.parse::<f64>().ok()) {
        m.sim.speed_mult = n;
        return;
    }
    if let Some(n) = id.strip_prefix("ai_").and_then(|n| n.parse::<i32>().ok()) {
        m.sim.admin_set_level(n);
        return;
    }
    if let Some(p) = place_percent(id) {
        m.sim.admin_set_threat(p);
        return;
    }
    match id {
        "solve" => {
            let ev = m.sim.admin_solve();
            m.pending.extend(ev);
        }
        "redeal" => m.sim.admin_redeal(),
        "wrong_add" => m.sim.wrong += 1,
        "wrong_clear" => m.sim.wrong = 0,
        "hold" => m.hold_latch = !m.hold_latch,
        "god" => {
            m.sim.god = !m.sim.god;
            if m.sim.god {
                m.sim.threat = m.sim.threat.min(99.0);
            }
        }
        "freeze" => m.frozen = !m.frozen,
        "noscare" => m.no_scare = !m.no_scare,
        "reveal" => m.reveal = !m.reveal,
        "call_play" => m.pending_call = Some(true),
        "call_skip" => m.pending_call = Some(false),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Mode, Outcome, Session};
    use std::collections::BTreeMap;

    fn mode(level: i32) -> DescentMode {
        let mut s = Session::new();
        s.start_mode("descent", level, BTreeMap::new(), Some(31));
        DescentMode::new(&s)
    }

    #[test]
    fn the_panel_lists_the_godot_rows() {
        let p = DescentAdmin;
        assert_eq!(p.mode_id(), "descent");
        let ids: Vec<String> = p.actions().into_iter().map(|a| a.id).collect();
        for want in ["solve", "redeal", "wrong_add", "wrong_clear", "close_0", "close_30", "close_60", "close_90", "hold"] {
            assert!(ids.contains(&want.to_string()), "{want}");
        }
    }

    #[test]
    fn the_depth_rows_follow_the_stages_of_the_descent() {
        let m = mode(4);
        let rows = entries(&m);
        let floors: Vec<&AdminEntry> = rows.iter().filter(|r| r.id.starts_with("floor_")).collect();
        assert_eq!(floors.len(), 4);
        assert!(floors[3].label.ends_with("DOOR"), "the door is the last floor: {}", floors[3].label);
        assert!(rows.iter().any(|r| r.id == "god") && rows.iter().any(|r| r.id == "call_play"));
    }

    #[test]
    fn rows_change_the_run() {
        let mut m = mode(3);
        m.sim.step(1800.0, false);
        run(&mut m, "floor_3");
        assert_eq!(m.sim.stage_idx, 2);
        run(&mut m, "close_60");
        assert_eq!(m.sim.threat, 60.0);
        run(&mut m, "wrong_add");
        run(&mut m, "wrong_add");
        assert_eq!(m.sim.wrong, 2);
        run(&mut m, "wrong_clear");
        assert_eq!(m.sim.wrong, 0);
        run(&mut m, "ai_20");
        assert_eq!(m.sim.level, 20);
        run(&mut m, "speed_2");
        assert_eq!(m.sim.speed_mult, 2.0);
        run(&mut m, "place_edge");
        assert_eq!(m.sim.threat, 97.0);
        run(&mut m, "god");
        assert!(m.sim.god && m.sim.threat <= 99.0);
        run(&mut m, "hold");
        assert!(m.hold_latch);
        run(&mut m, "freeze");
        assert!(m.frozen);
        run(&mut m, "solve");
        assert_eq!(m.sim.over, Over::Win, "solving the last stage ends the descent");
        assert_eq!(m.pending.len(), 2, "stage and win events wait for their sounds");
    }

    #[test]
    fn a_lost_shift_has_a_killer_unless_scares_are_off() {
        let mut m = mode(2);
        m.debug_lose();
        assert_eq!(m.outcome(), Outcome::Lost);
        assert_eq!(m.killer(), Some(("elise".to_string(), "shaft".to_string())));
        run(&mut m, "noscare");
        assert_eq!(m.killer(), None);
    }

    #[test]
    fn the_places_are_the_godot_ones() {
        assert_eq!(place_percent("place_far"), Some(0.0));
        assert_eq!(place_percent("place_step2"), Some(40.0));
        assert_eq!(place_percent("place_step1"), Some(70.0));
        assert_eq!(place_percent("place_edge"), Some(97.0));
        assert_eq!(place_percent("nope"), None);
    }
}
