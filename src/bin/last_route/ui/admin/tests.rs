//! Admin menu logic that needs no window: the label lists, the NPC commands, the speed rows, RESET ALL,
//! the gate closing the menu, tab stepping and the spawn count limits.

use super::actions::npc_op_live;
use super::menu::{hunter_list, labels, step_count, Cmd};
use super::*;
use crate::config::Difficulty;
use crate::game::pdist;
use crate::world::catalog::Catalog;

fn game() -> (Game, Content) {
    let content = Content::load();
    let catalog = Catalog::load();
    let g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
    (g, content)
}

/// A game with `n` admin NPCs next to the runner.
fn game_with_npcs(n: i32) -> (Game, Content, Admin) {
    let (mut g, content) = game();
    let p = g.p.pos;
    assert_eq!(g.npc_spawn(n, 50, (p.0, 1.6, p.1), (0.0, 1.0, 0.0)), n as usize);
    (g, content, Admin::new())
}

fn npc(a: &mut Admin, g: &mut Game, content: &Content, op: &'static str) {
    a.run_cmd(Cmd::Npc(op), g, content, &hunter_list(content));
}

#[test]
fn action_dropdowns_list_labels_not_ids() {
    let m = menu();
    assert_eq!(labels(&m.act_hunters)[0], "STUN ALL HUNTERS");
    assert_eq!(labels(&m.act_general)[1], "RESTART ROUTE // NEW SEED");
    assert_eq!(labels(&m.act_visual)[0], "TELEPORT TO WAYPOINT");
    for list in [&m.act_general, &m.act_player, &m.act_hunters, &m.act_world, &m.act_story, &m.act_visual] {
        for l in labels(list) {
            assert!(!l.contains('_'), "{l:?} is an id");
        }
    }
}

#[test]
fn the_live_npc_commands_are_real_ids_and_the_rest_are_listed() {
    let m = menu();
    let ids: Vec<&str> = m.npc_ops.iter().chain(m.npc_all.iter()).map(|(id, _)| *id).collect();
    for live in ["teleport", "follow", "unfollow", "smart", "remove", "all_teleport", "all_follow", "all_unfollow", "all_smart"] {
        assert!(ids.contains(&live), "{live} is not in menu.json");
        assert!(npc_op_live(live));
    }
    assert!(!npc_op_live("spectate") && !npc_op_live("freeze") && !npc_op_live("Teleport To Me"));
}

#[test]
fn npc_follow_and_stop_following_move_the_follow_flag() {
    let (mut g, content, mut a) = game_with_npcs(2);
    assert!(g.npcs.iter().all(|n| n.joined), "admin NPCs follow from the start");
    a.sel.insert("npc_pick", 1);
    npc(&mut a, &mut g, &content, "unfollow");
    assert!(g.npcs[0].joined && !g.npcs[1].joined, "only the picked NPC stops");
    npc(&mut a, &mut g, &content, "follow");
    assert!(g.npcs[1].joined);
    npc(&mut a, &mut g, &content, "all_unfollow");
    assert!(g.npcs.iter().all(|n| !n.joined));
    npc(&mut a, &mut g, &content, "all_follow");
    assert!(g.npcs.iter().all(|n| n.joined));
    assert!(g.message.contains("FOLLOW YOU"), "{}", g.message);
}

#[test]
fn npc_smart_takes_the_dropdown_value() {
    let (mut g, content, mut a) = game_with_npcs(2);
    a.sel.insert("npc_smart", 3);
    npc(&mut a, &mut g, &content, "all_smart");
    assert!(g.npcs.iter().all(|n| n.smart == 90));
    a.sel.insert("npc_smart", 0);
    a.sel.insert("npc_pick", 0);
    npc(&mut a, &mut g, &content, "smart");
    assert_eq!((g.npcs[0].smart, g.npcs[1].smart), (30, 90));
}

#[test]
fn npc_teleport_and_remove_act_on_the_picked_npc() {
    let (mut g, content, mut a) = game_with_npcs(3);
    let far = (g.npcs[2].at.0 + 3.0, g.npcs[2].at.1);
    g.npcs[2].at = far;
    a.sel.insert("npc_pick", 2);
    npc(&mut a, &mut g, &content, "teleport");
    assert!(pdist(g.npcs[2].at, g.p.pos) < 3.0, "lands beside the runner");
    let name = g.npcs[2].name.clone();
    npc(&mut a, &mut g, &content, "remove");
    assert_eq!(g.npcs.len(), 2);
    assert!(g.npcs.iter().all(|n| n.name != name));
    npc(&mut a, &mut g, &content, "all_teleport");
    assert!(g.message.contains("2 NPCS TELEPORTED"), "{}", g.message);
}

#[test]
fn npc_commands_with_no_npcs_say_so() {
    let (mut g, content) = game();
    let mut a = Admin::new();
    for op in ["teleport", "follow", "unfollow", "smart", "remove", "all_follow", "all_teleport"] {
        npc(&mut a, &mut g, &content, op);
        assert_eq!(g.message, "ADMIN // NO NPCS", "{op}");
    }
}

#[test]
fn a_command_that_is_not_built_yet_names_itself() {
    let (mut g, content, mut a) = game_with_npcs(1);
    npc(&mut a, &mut g, &content, "spectate");
    assert_eq!(g.message, "ADMIN // SPECTATE IS NOT IN THIS BUILD YET");
    npc(&mut a, &mut g, &content, "all_freeze");
    assert_eq!(g.message, "ADMIN // ALL FREEZE IS NOT IN THIS BUILD YET");
    assert_eq!(g.npcs.len(), 1, "nothing was changed");
}

#[test]
fn the_spawn_count_stays_between_one_and_the_data_maximum() {
    let max = menu().npc_spawn_max;
    assert_eq!(step_count(1, false), 1);
    assert_eq!(step_count(1, true), 2);
    assert_eq!(step_count(max, true), max);
    assert_eq!(step_count(max + 50, false), max, "an out-of-range count comes back inside");
    assert_eq!(step_count(0, true), 1);
}

#[test]
fn the_speed_rows_start_on_the_speed_in_force() {
    let a = Admin::new();
    let m = menu();
    assert_eq!(m.run_speeds[a.sel_of("run_speed")].1, "1X");
    assert_eq!(m.game_speeds[a.sel_of("game_speed")].1, "1X", "GAME SPEED must not start on PAUSE");
    assert_eq!(a.current_of("run_speed").as_deref(), Some("1X"));
    assert_eq!(a.current_of("game_speed").as_deref(), Some("1X"));
    assert_eq!(a.current_of("teleport"), None);
}

#[test]
fn current_speed_follows_what_was_run() {
    let (mut g, content) = game();
    let mut a = Admin::new();
    a.run_row_for_test("run_speed", 3, &mut g, &content);
    assert_eq!(a.run_speed, 3.0);
    assert_eq!(a.current_of("run_speed").as_deref(), Some("3X"));
    a.run_row_for_test("game_speed", 0, &mut g, &content);
    assert_eq!(a.current_of("game_speed").as_deref(), Some("PAUSE"));
    assert_eq!(a.game_dt(0.1), 0.0);
    a.run_row_for_test("game_speed", 5, &mut g, &content);
    assert_eq!(a.game_dt(0.1), 1.0 / 60.0, "STEP 1 FRAME runs one 60 Hz frame");
    assert_eq!(a.game_dt(0.1), 0.0, "then stays paused");
    // an out-of-range index is clamped to the last entry, not a panic
    a.run_row_for_test("run_speed", 99, &mut g, &content);
    assert_eq!(a.run_speed, 3.0);
}

#[test]
fn reset_all_puts_every_switch_and_speed_back() {
    let mut a = Admin::new();
    a.god_mode = true;
    a.no_target = true;
    a.noclip = true;
    a.map_reveal = true;
    a.show_traps = true;
    a.light_sight = false;
    a.run_speed = 3.0;
    a.game_speed = 4.0;
    a.view = View::Wireframe;
    a.fields[0] = "10,20".to_string();
    a.sync_speed_rows();
    a.reset_all();
    assert!(!a.god_mode && !a.no_target && !a.noclip && !a.map_reveal && !a.show_traps);
    assert!(a.light_sight);
    assert_eq!((a.run_speed, a.game_speed), (1.0, 1.0));
    assert_eq!(a.view, View::Off);
    assert!(a.fields.iter().all(|f| f.is_empty()));
    assert_eq!(a.current_of("run_speed").as_deref(), Some("1X"));
    assert_eq!(menu().run_speeds[a.sel_of("run_speed")].1, "1X");
    assert!(!a.prefs.get("god_mode"));
}

#[test]
fn losing_the_admin_gate_closes_an_open_menu() {
    let (mut g, _) = game();
    let mut a = Admin::new();
    a.open_menu();
    assert!(a.is_open());
    a.enabled = false;
    assert!(!a.input(&mut g), "the pause menu may use Esc now");
    assert!(!a.is_open(), "a closed gate leaves no invisible menu holding the mouse free");
    a.open_menu();
    assert!(!a.is_open(), "and it does not open while the gate is closed");
}

#[test]
fn tab_stepping_stops_at_both_ends() {
    assert_eq!(step_tab(0, -1), 0);
    assert_eq!(step_tab(0, 1), 1);
    assert_eq!(step_tab(TABS.len() - 1, 1), TABS.len() - 1);
    assert_eq!(step_tab(5, -1), 4);
    assert_eq!(TABS[9], "FIELDS");
}

#[test]
fn show_tab_clamps_and_a_closed_menu_has_no_focus() {
    let mut a = Admin::new();
    a.show_tab(99);
    assert_eq!(a.tab, TABS.len() - 1);
    assert!(a.is_open());
    a.close();
    assert!(a.focus.is_none() && a.popup.is_none() && a.field_focus.is_none());
}

#[test]
fn every_toggle_row_is_a_known_toggle_with_a_state() {
    let a = Admin::new();
    for id in [
        "no_target", "god_mode", "noclip", "region_lights", "show_traps", "light_sight", "map_reveal",
        "region_map", "show_collision", "hunter_reveal", "perf_overlay", "fullbright", "wireframe",
    ] {
        assert!(a.is_toggle(id), "{id}");
    }
    assert!(!a.is_toggle("teleport"));
}

#[test]
fn only_switches_with_a_saved_preference_persist() {
    let (mut g, content) = game();
    let mut a = Admin::new();
    for id in ["no_target", "god_mode", "noclip", "map_reveal", "region_map", "show_collision", "hunter_reveal", "perf_overlay"] {
        assert!(prefs::KEYS.iter().any(|(k, _)| *k == id), "{id} has no saved preference");
        let before = a.state_of(id);
        a.run_row_for_test(Box::leak(id.to_string().into_boxed_str()), 0, &mut g, &content);
        assert_eq!(a.state_of(id), !before, "{id} toggles");
        assert_eq!(a.prefs.get(id), !before, "{id} is saved when toggled");
    }
}

#[test]
fn using_a_tool_marks_the_run_as_a_debug_run() {
    let (mut g, content) = game();
    let mut a = Admin::new();
    assert!(!a.debug_run());
    a.run_row_for_test("god_mode", 0, &mut g, &content);
    assert!(a.debug_run());
}
