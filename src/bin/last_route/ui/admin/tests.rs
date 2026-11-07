//! Admin menu logic that needs no window: the label lists, the NPC commands, the speed rows, RESET ALL,
//! the gate closing the menu, tab stepping and the spawn count limits.

use super::actions::{npc_button, npc_op_live, NpcButton};
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

/// Every id of the two NPC grids, with its label.
fn npc_buttons() -> Vec<(&'static str, &'static str)> {
    let m = menu();
    m.npc_ops.iter().chain(m.npc_all.iter()).copied().collect()
}

#[test]
fn every_working_npc_button_is_enabled_when_the_run_has_an_npc() {
    for (op, label) in npc_buttons() {
        let state = npc_button(op, 1);
        if npc_op_live(op) {
            assert_eq!(state, NpcButton::Ready, "{label} is gray with an NPC on the floor");
            assert_eq!(state.tip(), None, "{label}: an enabled button has no gray-out reason");
            assert_eq!(npc_button(op, 5), NpcButton::Ready, "{label}");
        } else {
            let tip = state.tip().unwrap_or_else(|| panic!("{label} is gray and says nothing"));
            assert!(tip.starts_with(menu().npc_pending_prefix), "{label}: {tip}");
            assert!(tip.len() > menu().npc_pending_prefix.len() + 10, "{label} needs a real reason: {tip}");
        }
    }
}

#[test]
fn with_no_npc_a_working_button_is_gray_and_says_to_spawn_one() {
    for (op, label) in npc_buttons().into_iter().filter(|(op, _)| npc_op_live(op)) {
        let state = npc_button(op, 0);
        assert_eq!(state, NpcButton::NoNpcs, "{label}");
        assert!(state.tip().unwrap().contains("SPAWN NPC"), "{label}");
    }
}

#[test]
fn a_not_built_button_stays_gray_with_its_reason_even_with_npcs() {
    let pending: Vec<&str> = npc_buttons().into_iter().map(|(op, _)| op).filter(|op| !npc_op_live(op)).collect();
    assert!(pending.contains(&"spectate") && pending.contains(&"kill") && pending.contains(&"goto"));
    assert!(pending.contains(&"freeze") && pending.contains(&"all_resume"));
    for op in pending {
        for n in [0, 3] {
            assert!(matches!(npc_button(op, n), NpcButton::Pending(r) if !r.is_empty()), "{op} with {n} NPCs");
        }
    }
}

#[test]
fn every_working_npc_button_changes_the_game_as_labelled() {
    // (op, what it must change) on a game with three NPCs; the second one is the picked NPC
    let setup = || {
        let (mut g, content, mut a) = game_with_npcs(3);
        a.sel.insert("npc_pick", 1);
        a.sel.insert("npc_smart", 0); // SMART 30
        for n in g.npcs.iter_mut() {
            n.at = (n.at.0 + 4.0, n.at.1);
        }
        (g, content, a)
    };
    for (op, label) in npc_buttons().into_iter().filter(|(op, _)| npc_op_live(op)) {
        let (mut g, content, mut a) = setup();
        if op == "follow" || op == "all_follow" {
            for n in g.npcs.iter_mut() {
                n.joined = false;
            }
        }
        let before: Vec<_> = g.npcs.iter().map(|n| (n.at, n.joined, n.smart)).collect();
        let ids: Vec<u32> = g.npcs.iter().map(|n| n.id).collect();
        npc(&mut a, &mut g, &content, op);
        assert!(!g.message.contains("NOT IN THIS BUILD"), "{label}: {}", g.message);
        assert!(!g.message.contains("NO NPCS"), "{label}: {}", g.message);
        match op {
            "teleport" => {
                assert_ne!(g.npcs[1].at, before[1].0, "{label}: the picked NPC did not move");
                assert_eq!((g.npcs[0].at, g.npcs[2].at), (before[0].0, before[2].0), "{label}: only the picked one");
                assert!(pdist(g.npcs[1].at, g.p.pos) < 3.0, "{label}");
            }
            "all_teleport" => {
                for (i, n) in g.npcs.iter().enumerate() {
                    assert_ne!(n.at, before[i].0, "{label}: NPC {i} stayed");
                    assert!(pdist(n.at, g.p.pos) < 3.0, "{label}");
                }
            }
            "follow" => assert_eq!(g.npcs.iter().map(|n| n.joined).collect::<Vec<_>>(), [false, true, false], "{label}"),
            "all_follow" => assert!(g.npcs.iter().all(|n| n.joined), "{label}"),
            "unfollow" => assert_eq!(g.npcs.iter().map(|n| n.joined).collect::<Vec<_>>(), [true, false, true], "{label}"),
            "all_unfollow" => assert!(g.npcs.iter().all(|n| !n.joined), "{label}"),
            "smart" => assert_eq!(g.npcs.iter().map(|n| n.smart).collect::<Vec<_>>(), [50, 30, 50], "{label}"),
            "all_smart" => assert!(g.npcs.iter().all(|n| n.smart == 30), "{label}"),
            "remove" => {
                assert_eq!(g.npcs.len(), 2, "{label}");
                assert!(g.npcs.iter().all(|n| n.id != ids[1]), "{label}: the picked NPC is still there");
            }
            other => panic!("{other} is live but this test does not know what it changes"),
        }
    }
}

#[test]
fn the_remove_buttons_take_npcs_off_the_floor() {
    let (mut g, content, mut a) = game_with_npcs(2);
    npc(&mut a, &mut g, &content, "remove_nearest");
    assert_eq!(g.npcs.len(), 1, "REMOVE NEAREST takes one NPC");
    assert!(g.message.starts_with("ADMIN // NPC REMOVED"), "{}", g.message);
    npc(&mut a, &mut g, &content, "remove_all");
    assert!(g.npcs.is_empty(), "REMOVE ALL takes the rest");
    assert_eq!(g.message, "ADMIN // 1 NPCS REMOVED");
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
    for op in [
        "teleport", "follow", "unfollow", "smart", "remove", "all_follow", "all_teleport", "all_unfollow", "all_smart",
    ] {
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

/// A level-0 window with a stretch of open floor away from the start; returns the game and the stretch's middle cell.
fn open_game() -> (Game, Content, crate::world::Cell) {
    use crate::world::Cell;
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-OPEN-SUMMON", 0, &content, &catalog);
    let start = g.fac.start.center();
    let row = g
        .fac
        .floor_cells()
        .into_iter()
        .find(|c| {
            pdist(c.center(), start) > 25.0
                && (-1..=5).all(|k| g.fac.hunter_open(Cell::new(c.x + k, c.y)) && g.fac.hunter_open(Cell::new(c.x + k, c.y + 1)))
        })
        .expect("a roomy stretch of floor away from the start");
    g.hunters.clear();
    for level in g.resident_hunters.levels() {
        g.resident_hunters.list_mut(level).clear();
    }
    (g, content, Cell::new(row.x + 2, row.y))
}

fn summon_first(g: &mut Game, content: &Content) {
    let list = hunter_list(content);
    Admin::new().run_cmd(Cmd::Hunter(0, 0, 0), g, content, &list);
}

/// Summons one hunter, then stands it on the runner's cell and steps the hunters.
fn grab_attempt() -> Game {
    let (mut g, content, mid) = open_game();
    g.p.pos = mid.center();
    summon_first(&mut g, &content);
    assert_eq!(g.hunters.len(), 1);
    assert_eq!(g.hunters[0].y, 0.0, "a summon stands on the floor");
    g.hunters[0].pos = (g.p.pos.0 + 0.2, g.p.pos.1);
    let sounds = crate::audio::Sounds::default();
    let catalog = Catalog::load();
    for _ in 0..30 {
        g.step(1.0 / 60.0, &crate::game::Controls::default(), &content, &catalog, &sounds);
    }
    g
}

#[test]
fn a_summoned_hunter_at_the_runners_height_grabs() {
    let g = grab_attempt();
    assert!(g.escape.is_some() || g.report.is_some(), "the same height grabs");
}

#[test]
fn a_summon_on_the_level_below_stands_on_that_level() {
    use crate::world::route::sewer::core::LOWER_LEVEL;
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-LOWER-SUMMON", LOWER_LEVEL, &content, &catalog);
    g.hunters.clear();
    summon_first(&mut g, &content);
    let b = g.hunters.last().expect("a hunter was summoned");
    assert_eq!(b.level, g.runner_level());
    assert!((b.y - crate::game::level_stack::floor_y_of(g.runner_level())).abs() < 1e-4);
    assert_eq!(b.last_known_level, g.runner_level());
}

#[test]
fn a_summon_on_the_inferno_stands_on_the_terrain() {
    use crate::world::infinite::INFERNO_LEVEL;
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-INFERNO-SUMMON", INFERNO_LEVEL, &content, &catalog);
    g.hunters.clear();
    summon_first(&mut g, &content);
    let b = g.hunters.last().expect("a hunter was summoned");
    let want = crate::game::level_stack::floor_y_of(INFERNO_LEVEL) + g.fac.terrain().expect("terrain").floor_at(b.pos.0, b.pos.1);
    assert!((b.y - want).abs() < 1e-4, "feet {} over ground {want}", b.y);
}

// -- the secret wolf spiders (docs/SECRET_WOLF_SPIDER.md): the picker is their only listing --------------------------------

fn picker_index(content: &Content, id: &str) -> usize {
    hunter_list(content).iter().position(|(i, _)| i == id).unwrap_or_else(|| panic!("{id} is not in the picker"))
}

fn inferno_game(content: &Content) -> Game {
    Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-ADMIN-SPIDER", crate::world::infinite::INFERNO_LEVEL, content, &Catalog::load())
}

#[test]
fn the_picker_lists_the_wolf_spider() {
    use crate::hunters::secret_creatures as secret;
    let content = Content::load();
    let list = hunter_list(&content);
    assert!(list.iter().any(|(i, _)| i == secret::WHITE), "{}", secret::WHITE);
    assert_eq!(list.iter().filter(|(i, _)| i == secret::WHITE).count(), 1);
}

#[test]
fn a_wolf_spider_is_refused_off_its_home_level() {
    let (mut g, content) = game();
    let mut a = Admin::new();
    let list = hunter_list(&content);
    let id = "white_wolf_spider";
    a.run_cmd(Cmd::Hunter(picker_index(&content, id), 0, 0), &mut g, &content, &list);
    assert!(g.hunters.iter().all(|h| !h.def.id.ends_with("wolf_spider")), "{id} was summoned on level 0");
    assert!(g.message.contains("LIVES ON LEVEL -3"), "{id}: {}", g.message);
}

#[test]
fn the_summon_works_on_the_inferno_and_remove_takes_it_out_again() {
    let content = Content::load();
    let list = hunter_list(&content);
    let mut g = inferno_game(&content);
    let mut a = Admin::new();
    a.run_cmd(Cmd::Hunter(picker_index(&content, "white_wolf_spider"), 0, 0), &mut g, &content, &list);
    let spiders = g.hunters.iter().filter(|h| h.def.id == "white_wolf_spider").count();
    assert_eq!(spiders, 1, "{}", g.message);
    assert_eq!(g.hunters.iter().find(|h| h.def.id == "white_wolf_spider").unwrap().level, crate::world::infinite::INFERNO_LEVEL);
    // REMOVE takes it out again
    a.run_cmd(Cmd::Hunter(picker_index(&content, "white_wolf_spider"), 1, 0), &mut g, &content, &list);
    assert!(g.hunters.iter().all(|h| !h.def.id.ends_with("wolf_spider")));
}

#[test]
fn summon_all_leaves_the_wolf_spiders_to_their_own_level() {
    let (mut g, content) = game();
    let before = g.hunters.len();
    Admin::new().run_cmd(Cmd::Run("act_hunters", 5), &mut g, &content, &hunter_list(&content));
    assert!(g.hunters.len() > before);
    assert!(g.hunters.iter().all(|h| !h.def.id.ends_with("wolf_spider")), "summon all never brings them to level 0");
}

// ---- RESET PASSWORD (password.rs): the FIELDS action for full admin owners ----------------------------------------

use super::password::{canonical_username, failure_text, Notice, RESET_FIELD};
use crate::net::api::PasswordReset;
use crate::net::{BackendConfig, NetError, Pending};

fn http_error(status: u16, code: &str, error: &str) -> NetError {
    NetError::Http { status, code: code.into(), error: error.into(), body: serde_json::Value::Null }
}

fn temporary(name: &str) -> PasswordReset {
    PasswordReset { username: name.into(), temporary_password: Some("K7QF-2MXD-H9TA-WP4C".into()), sessions_revoked: 2 }
}

/// An admin that is a full owner, as the app sets it from the server's flag.
fn owner_admin() -> Admin {
    let mut a = Admin::new();
    a.enabled = true;
    a.owner = true;
    a
}

#[test]
fn the_reset_password_field_is_the_last_one_in_the_data() {
    assert_eq!(menu().fields[RESET_FIELD].0, "RESET PASSWORD");
    assert_eq!(RESET_FIELD, menu().fields.len() - 1, "hiding it must not move the other fields");
    assert_eq!(Admin::new().fields.len(), menu().fields.len());
}

#[test]
fn only_a_full_admin_owner_sees_the_reset_password_field() {
    let mut a = Admin::new();
    assert!(!a.owner, "nobody is an owner until the server says so");
    a.enabled = true;
    assert!(!a.field_visible(RESET_FIELD) && a.field_count() == menu().fields.len() - 1);
    assert!((0..RESET_FIELD).all(|i| a.field_visible(i)), "the other fields are for every admin");
    a.owner = true;
    assert!(a.field_visible(RESET_FIELD) && a.field_count() == menu().fields.len());
    // an owner flag without the admin gate gives nothing (signed out, or not the admin any more)
    a.enabled = false;
    assert!(!a.field_visible(RESET_FIELD));
}

#[test]
fn a_limited_admin_cannot_ask_for_a_reset() {
    let (mut g, content) = game();
    let mut a = Admin::new();
    a.enabled = true;
    a.fields[RESET_FIELD] = "runner".into();
    a.run_cmd(Cmd::Field(RESET_FIELD), &mut g, &content, &hunter_list(&content));
    assert!(g.message.contains("FULL ADMIN"), "{}", g.message);
    assert!(a.reset.request.is_none() && !a.reset_busy(), "nothing is queued for the app to send");
    assert_eq!(a.fields[RESET_FIELD], "runner", "the field is left alone");
}

#[test]
fn the_owner_queues_one_reset_for_a_valid_name_and_the_field_clears() {
    let (mut g, content) = game();
    let mut a = owner_admin();
    for bad in ["", "  ", "no", "has space", "semi;colon", &"A".repeat(25)] {
        a.fields[RESET_FIELD] = bad.to_string();
        a.run_cmd(Cmd::Field(RESET_FIELD), &mut g, &content, &hunter_list(&content));
        assert!(g.message.contains("TYPE THE ACCOUNT NAME"), "{bad:?}: {}", g.message);
        assert!(!a.reset_busy(), "{bad:?}");
    }
    a.fields[RESET_FIELD] = " runner_1 ".into();
    a.run_cmd(Cmd::Field(RESET_FIELD), &mut g, &content, &hunter_list(&content));
    assert!(g.message.contains("RESETTING THE PASSWORD OF RUNNER_1"), "{}", g.message);
    assert_eq!(a.reset.request.as_deref(), Some("RUNNER_1"));
    assert!(a.fields[RESET_FIELD].is_empty());
    // a second ask while the first is outstanding does nothing
    a.fields[RESET_FIELD] = "other".into();
    a.run_cmd(Cmd::Field(RESET_FIELD), &mut g, &content, &hunter_list(&content));
    assert!(g.message.contains("ALREADY RUNNING"), "{}", g.message);
    assert_eq!(a.reset.request.as_deref(), Some("RUNNER_1"));
    assert!(!g.message.contains("NEEDS THE PRIMARY SERVER"));
}

#[test]
fn the_other_fields_still_run_as_before() {
    let (mut g, content) = game();
    let mut a = owner_admin();
    a.fields[2] = "someone".into();
    a.run_cmd(Cmd::Field(2), &mut g, &content, &hunter_list(&content));
    assert!(g.message.contains("NEEDS THE PRIMARY SERVER"), "{}", g.message);
    assert!(!a.reset_busy());
}

#[test]
fn canonical_names_follow_the_server_rule() {
    assert_eq!(canonical_username(" ada_1 ").as_deref(), Some("ADA_1"));
    assert_eq!(canonical_username(&"a".repeat(24)).as_deref(), Some("A".repeat(24).as_str()));
    for bad in ["", "ab", "a b", "a-b", "über", &"a".repeat(25)] {
        assert_eq!(canonical_username(bad), None, "{bad:?}");
    }
}

#[test]
fn a_generated_password_opens_a_message_box_on_the_fields_tab() {
    let mut a = owner_admin();
    a.reset.pending = Some(("RUNNER".into(), Pending::done(Ok(temporary("RUNNER")))));
    assert!(!a.is_open());
    a.poll_password_reset();
    assert!(a.reset.pending.is_none());
    let notice = a.reset.notice.as_ref().expect("the answer opens the message box");
    assert_eq!(notice.password.as_deref(), Some("K7QF-2MXD-H9TA-WP4C"), "the COPY button copies exactly this");
    assert_eq!(notice.heading, "PASSWORD RESET");
    let text = notice.lines.join(" ");
    assert!(text.contains("RUNNER") && text.contains("SHOWN ONCE") && text.contains("2 OPEN SESSIONS"), "{text}");
    assert!(!text.contains("K7QF"), "the password is drawn in its own row, not in a line of text");
    assert!(a.is_open(), "the menu opens so the box can be clicked");
    assert_eq!(a.tab, 9);
    // CLOSE drops the box and the password with it
    a.close_notice();
    assert!(a.reset.notice.is_none());
}

#[test]
fn a_typed_password_is_not_shown_back_and_a_failure_says_why() {
    let typed = PasswordReset { username: "RUNNER".into(), temporary_password: None, sessions_revoked: 1 };
    let notice = Notice::success(&typed, "RUNNER");
    assert_eq!(notice.password, None);
    assert!(notice.lines.join(" ").contains("NEW PASSWORD"));
    let mut a = owner_admin();
    a.reset.pending = Some(("GHOST".into(), Pending::done(Err(http_error(404, "ACCOUNT_NOT_FOUND", "Account was not found")))));
    a.poll_password_reset();
    let notice = a.reset.notice.as_ref().unwrap();
    assert_eq!((notice.heading.as_str(), notice.password.as_ref()), ("RESET FAILED", None));
    assert_eq!(notice.lines, ["NO ACCOUNT NAMED GHOST"]);
}

#[test]
fn every_refusal_has_a_player_facing_line() {
    let line = |e: NetError| failure_text(&e, "RUNNER");
    assert!(line(NetError::Offline).starts_with("OFFLINE"));
    assert!(line(http_error(401, "NO_SESSION", "x")).contains("SIGN IN AGAIN"));
    assert!(line(http_error(403, "ADMIN_OWNER_REQUIRED", "x")).contains("FULL ADMIN"));
    assert_eq!(line(http_error(404, "ACCOUNT_NOT_FOUND", "x")), "NO ACCOUNT NAMED RUNNER");
    assert!(line(http_error(404, "UNKNOWN_ENDPOINT", "x")).contains("CANNOT RESET"));
    assert!(line(http_error(400, "USE_PASSWORD_CHANGE", "x")).contains("OWN ACCOUNT"));
    assert!(line(http_error(400, "INVALID_USERNAME", "x")).contains("NOT A VALID"));
    assert!(line(http_error(429, "RATE_LIMITED", "x")).contains("TOO MANY"));
    assert_eq!(line(http_error(500, "", "")), "THE SERVER ANSWERED 500");
    assert_eq!(line(http_error(502, "", "bad gateway")), "BAD GATEWAY");
    assert!(line(NetError::Timeout).contains("DID NOT ANSWER"));
    assert!(line(NetError::Transport("refused".into())).contains("COULD NOT REACH"));
    assert_eq!(line(NetError::Refused("Sign in first".into())), "SIGN IN FIRST");
}

/// Waits for a started request to land (the request runs on a worker thread).
fn settle(a: &mut Admin) {
    for _ in 0..400 {
        a.poll_password_reset();
        if a.reset.pending.is_none() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the reset never finished");
}

#[test]
fn the_app_sends_a_queued_reset_once_and_only_with_a_session() {
    // offline: the client refuses before sending anything
    let offline = BackendConfig { offline: true, ..BackendConfig::loopback(1) };
    let mut a = owner_admin();
    a.reset.request = Some("RUNNER".into());
    a.run_password_reset(&offline, "session-token");
    assert!(a.reset.request.is_none() && a.reset.pending.is_some(), "the request is on its way");
    a.run_password_reset(&offline, "session-token"); // nothing queued: no second request
    settle(&mut a);
    assert!(a.reset.notice.as_ref().unwrap().lines[0].starts_with("OFFLINE"));

    // no session token: refused locally, the server is never asked
    let mut b = owner_admin();
    b.reset.request = Some("RUNNER".into());
    b.run_password_reset(&BackendConfig::loopback(1), "");
    settle(&mut b);
    assert!(b.reset.notice.as_ref().unwrap().lines[0].contains("SIGN IN"));

    // with nothing queued the call does nothing at all
    let mut c = owner_admin();
    c.run_password_reset(&BackendConfig::loopback(1), "session-token");
    assert!(!c.reset_busy());
}

#[test]
fn signing_out_drops_a_password_waiting_in_the_message_box() {
    let (mut g, _) = game();
    let mut a = owner_admin();
    a.reset.notice = Some(Notice::success(&temporary("RUNNER"), "RUNNER"));
    a.reset.request = Some("OTHER".into());
    a.enabled = false;
    assert!(!a.input(&mut g));
    assert!(a.reset.notice.is_none() && !a.reset_busy(), "nothing of a reset outlives the admin's sign-in");
}

#[test]
fn reset_all_does_not_touch_a_reset_in_flight_and_clears_the_typed_name() {
    let mut a = owner_admin();
    a.fields[RESET_FIELD] = "runner".into();
    a.reset.request = Some("RUNNER".into());
    a.reset_all();
    assert!(a.fields.iter().all(|f| f.is_empty()));
    assert!(a.reset.request.is_some(), "RESET ALL SETTINGS is about switches, not a request already made");
}
