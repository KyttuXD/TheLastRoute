//! The admin tools (game/admin/): the in-run developer overlay on the 0 key (admin_menu.gd with
//! the route's rows from infinite_route_test.gd `_infinite_admin_rows`, DebugView's entries and
//! the AdminNpcPanel page), its "ADMIN ACTIVE" status badge, the effects its toggles have on
//! the run, and the hub/title ADMIN panel (hub_admin_panel.gd).
//!
//! Layout follows the original's canvas units: the 480x270 design canvas expands with the
//! window, the panel takes ~94% x 92% of it up to 420x300 units, and is zoomed so one unit is
//! never under 1.6 px (see `menu.rs`).
//!
//! Wiring (all in the app loop, see the integration notes in the port report):
//!   let mut admin = admin::Admin::new();          // once
//!   let esc = admin.input(g);                      // each run frame, before the Esc/pause check
//!   let dt = admin.game_dt(dt); admin.before_update(g); g.update(dt, ..); admin.after_update(g, &c, &content, dt);
//!   admin.draw(g, &assets, &content);              // after the HUD
//!   if let Some(r) = admin.take_request() { .. }   // RESTART / END ROUTE
//! and for the title ADMIN button: `admin::HubPanel` / `admin::draw_hub`.

mod actions;
mod effects;
mod hub;
mod look;
mod menu;
mod prefs;

pub use hub::{draw_hub, HubPanel};

/// The game's admin owners, the same list the server grants owner rights to
/// (`ADMIN_OWNER_USERNAMES` in src/bin/lr_server/accounts/account_service.rs). Client gates use this
/// list; matching is case-insensitive.
pub const ADMIN_OWNERS: [&str; 2] = ["KYTTU", "AYTTU"];

/// Whether a signed-in account name is one of the admin owners (KYTTU or AYTTU, any case).
pub fn is_admin_owner(username: &str) -> bool {
    ADMIN_OWNERS.iter().any(|owner| owner.eq_ignore_ascii_case(username))
}

#[cfg(test)]
mod owner_gate_tests {
    use super::*;

    #[test]
    fn admin_owners_pass_the_gate_in_any_case() {
        for name in ["KYTTU", "kyttu", "AYTTU", "ayttu", "Ayttu"] {
            assert!(is_admin_owner(name), "{name}");
        }
    }

    #[test]
    fn normal_names_do_not_pass_the_gate() {
        for name in ["ALICE", "", "KYTTU2", " KYTTU", "AYTT"] {
            assert!(!is_admin_owner(name), "{name:?}");
        }
    }
}

use crate::app::assets::Assets;
use crate::content::Content;
use crate::game::{Controls, Game};
use macroquad::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;

/// What the menu asks the app to do with the run (owner route controls).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    /// RESTART ROUTE // NEW SEED
    RestartNewSeed,
    /// RESTART ROUTE // SAME SEED
    RestartSameSeed,
    /// END ROUTE
    EndRoute,
}

/// DebugView.Mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Off,
    Fullbright,
    Wireframe,
}

pub use crate::shared::admin_flags::{FULLBRIGHT, GOD, NOCLIP, NO_TARGET, RUN_SPEED, WIREFRAME};

pub const TABS: [&str; 10] = [
    "STATUS",
    "GENERAL",
    "PLAYER",
    "HUNTERS",
    "NPCS",
    "WORLD",
    "STORY",
    "VISUAL",
    "DEBUG VIEW",
    "FIELDS",
];

/// An open OptionButton list.
pub(crate) struct Popup {
    pub key: &'static str,
    pub anchor: Rect,
    pub items: Vec<String>,
    pub scroll: f32,
}

/// What the run looked like just before `Game::update` (god mode / noclip / run speed work
/// from the difference).
#[derive(Default)]
struct Pre {
    pos: (f32, f32),
    report: bool,
    escape: bool,
    flashlight: bool,
    valid: bool,
}

pub struct Admin {
    /// Build.tools_enabled(): the badge and the 0 key exist only while this is on.
    pub enabled: bool,
    open: bool,
    tab: usize,
    tab_offset: usize,
    scroll: [f32; TABS.len()],
    content_h: [f32; TABS.len()],
    sel: HashMap<&'static str, usize>,
    popup: Option<Popup>,
    /// A dropdown to open on the next draw (tests and screenshots).
    pending_popup: Option<&'static str>,
    /// Keyboard focus (a widget key); the first action control gets it when the menu opens.
    focus: Option<String>,
    fields: [String; 3],
    field_focus: Option<usize>,
    prefs: prefs::Prefs,

    // PLAYER
    no_target: bool,
    god_mode: bool,
    noclip: bool,
    run_speed: f32,
    // WORLD
    game_speed: f32,
    step_frames: u32,
    lights_all_on: bool,
    // VISUAL
    show_traps: bool,
    light_sight: bool,
    map_reveal: bool,
    region_map: bool,
    show_collision: bool,
    hunter_reveal: bool,
    perf_overlay: bool,
    view: View,

    /// GameState.debug_run: any tool used this run (nothing from it should be recorded).
    debug_run: bool,
    /// Hunter ids summoned by the admin this run (REMOVE SUMMONED takes these back out).
    summoned: Vec<String>,
    request: Option<Request>,
    npc_count: i32,

    run_key: Option<u32>,
    last_time: f32,
    pre: Pre,
    pending_landing: bool,
    frame_ms: VecDeque<f32>,
    clock: f32,
}

impl Default for Admin {
    fn default() -> Self {
        Admin::new()
    }
}

impl Admin {
    pub fn new() -> Admin {
        let prefs = prefs::Prefs::load();
        Admin {
            enabled: true,
            open: false,
            tab: 0,
            tab_offset: 0,
            scroll: [0.0; TABS.len()],
            content_h: [0.0; TABS.len()],
            sel: HashMap::new(),
            popup: None,
            pending_popup: None,
            focus: None,
            fields: Default::default(),
            field_focus: None,
            no_target: false,
            god_mode: false,
            noclip: false,
            run_speed: 1.0,
            game_speed: 1.0,
            step_frames: 0,
            lights_all_on: true,
            show_traps: false,
            light_sight: true,
            map_reveal: false,
            region_map: false,
            show_collision: false,
            hunter_reveal: false,
            perf_overlay: prefs.get("perf_overlay"),
            view: View::Off,
            debug_run: false,
            summoned: vec![],
            request: None,
            npc_count: 1,
            run_key: None,
            last_time: 0.0,
            pre: Pre::default(),
            pending_landing: false,
            frame_ms: VecDeque::new(),
            clock: 0.0,
            prefs,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    #[allow(dead_code)]
    pub fn close(&mut self) {
        self.set_open(false);
    }

    #[allow(dead_code)]
    pub fn open_menu(&mut self) {
        self.set_open(true);
    }

    /// Opens a page's dropdown list by key (e.g. "act_general", "hunter"); for tests and screenshots.
    #[allow(dead_code)]
    pub fn show_dropdown(&mut self, key: &'static str) {
        self.pending_popup = Some(key);
    }

    /// Runs a menu row as if clicked: a dropdown row key ("teleport", "act_hunters", ..) with
    /// the chosen index, a toggle id with any index, or "hunter" with index = hunter * 4 + DO.
    /// For tests and scripted checks.
    #[allow(dead_code)]
    pub fn run_row_for_test(
        &mut self,
        key: &'static str,
        index: usize,
        g: &mut Game,
        content: &Content,
    ) {
        let hunters = menu::hunter_list(content);
        let cmd = if key == "hunter" {
            menu::Cmd::Hunter(index / 4, index % 4, 0)
        } else if self.is_toggle(key) {
            menu::Cmd::Toggle(key)
        } else {
            menu::Cmd::Run(key, index)
        };
        self.run_cmd(cmd, g, content, &hunters);
    }

    /// Opens the menu on a tab (0 STATUS .. 9 FIELDS); for tests and screenshots.
    #[allow(dead_code)]
    pub fn show_tab(&mut self, tab: usize) {
        self.set_open(true);
        self.tab = tab.min(TABS.len() - 1);
    }

    fn set_open(&mut self, shown: bool) {
        if shown && !self.enabled {
            return;
        }
        self.open = shown;
        self.popup = None;
        self.field_focus = None;
        // the first action control (GENERAL's ITEMS dropdown) takes focus on opening
        self.focus = if shown {
            Some("give_item".to_string())
        } else {
            None
        };
    }

    /// The debug view (FULLBRIGHT / WIREFRAME) for the renderer.
    #[allow(dead_code)]
    pub fn view(&self) -> View {
        self.view
    }

    #[allow(dead_code)]
    pub fn debug_run(&self) -> bool {
        self.debug_run
    }

    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    /// Keys, once per run frame and BEFORE the app's own Esc (pause) check. 0 / keypad 0
    /// toggles the menu, Esc closes it (an open dropdown list first), F6 toggles FULLBRIGHT and
    /// F8 the region map. Returns true when it used Esc this frame, so the pause menu must not.
    pub fn input(&mut self, g: &mut Game) -> bool {
        self.track_run(g);
        if !self.enabled {
            return false;
        }
        if is_key_pressed(KeyCode::F6) {
            self.view = if self.view == View::Fullbright {
                View::Off
            } else {
                View::Fullbright
            };
        }
        if is_key_pressed(KeyCode::F8) {
            self.toggle("region_map", g, None);
        }
        let typing = self.open && self.tab == 9 && self.field_focus.is_some();
        if self.open && is_key_pressed(KeyCode::Escape) {
            if self.popup.is_some() {
                self.popup = None;
            } else {
                self.set_open(false);
            }
            return true;
        }
        if self.open {
            self.keyboard_nav();
        }
        if !typing && (is_key_pressed(KeyCode::Key0) || is_key_pressed(KeyCode::Kp0)) {
            let next = !self.open;
            self.set_open(next);
        }
        false
    }

    /// Tab and Right move to the next tab, Shift+Tab and Left to the previous one (as a click
    /// does, this drops an open list and the field focus). On FIELDS, Up and Down move between
    /// the line edits while one has focus.
    fn keyboard_nav(&mut self) {
        let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        let step: isize = if is_key_pressed(KeyCode::Tab) {
            if shift {
                -1
            } else {
                1
            }
        } else if is_key_pressed(KeyCode::Right) {
            1
        } else if is_key_pressed(KeyCode::Left) {
            -1
        } else {
            0
        };
        if step != 0 {
            let next = (self.tab as isize + step).clamp(0, TABS.len() as isize - 1) as usize;
            if next != self.tab {
                self.tab = next;
                self.popup = None;
                self.field_focus = None;
            }
        }
        if self.tab == 9 {
            if let Some(i) = self.field_focus {
                if is_key_pressed(KeyCode::Up) {
                    self.field_focus = Some(i.saturating_sub(1));
                } else if is_key_pressed(KeyCode::Down) {
                    self.field_focus = Some((i + 1).min(self.fields.len() - 1));
                }
            }
        }
    }

    /// Engine.time_scale for the run: PAUSE, 0.25x .. 4x, and STEP 1 FRAME (one 60 Hz frame,
    /// then paused again).
    pub fn game_dt(&mut self, dt: f32) -> f32 {
        if self.step_frames > 0 {
            self.step_frames -= 1;
            return 1.0 / 60.0;
        }
        dt * self.game_speed
    }

    /// A new run (or a restart) applies the persisted admin preferences, as
    /// `_apply_persisted_admin_preferences` does when the route starts.
    fn track_run(&mut self, g: &mut Game) {
        let new_run = self.run_key != Some(g.run_seed) || g.time + 0.25 < self.last_time;
        self.last_time = g.time;
        if !new_run {
            return;
        }
        self.run_key = Some(g.run_seed);
        // the hub panel may have changed them since the last run
        self.prefs = prefs::Prefs::load();
        self.no_target = self.prefs.get("no_target");
        self.god_mode = self.prefs.get("god_mode");
        self.noclip = self.prefs.get("noclip");
        self.map_reveal = self.prefs.get("map_reveal");
        self.hunter_reveal = self.prefs.get("hunter_reveal");
        self.region_map = self.prefs.get("region_map");
        self.show_collision = self.prefs.get("show_collision");
        self.perf_overlay = self.prefs.get("perf_overlay");
        self.run_speed = 1.0;
        self.game_speed = 1.0;
        self.step_frames = 0;
        self.debug_run = false;
        self.summoned.clear();
        self.pending_landing = false;
        // leaving a run always turns the debug view off again (AdminMenu._exit_tree)
        self.view = View::Off;
        self.set_open(false);
    }

    /// Publishes the switches game code reads. Only the admin gets them: when the signed-in account is
    /// not the admin, every switch is published at its default, whatever admin.cfg says (a saved
    /// `god_mode=true` from the admin's own session must not reach a player).
    fn publish(&self) {
        let on = self.enabled;
        GOD.store(on && self.god_mode, Ordering::Relaxed);
        NO_TARGET.store(on && self.no_target, Ordering::Relaxed);
        NOCLIP.store(on && self.noclip, Ordering::Relaxed);
        FULLBRIGHT.store(on && self.view == View::Fullbright, Ordering::Relaxed);
        WIREFRAME.store(on && self.view == View::Wireframe, Ordering::Relaxed);
        let speed = if on { self.run_speed } else { 1.0 };
        RUN_SPEED.store(speed.to_bits(), Ordering::Relaxed);
    }

    /// Call right before `Game::update`.
    pub fn before_update(&mut self, g: &mut Game) {
        self.track_run(g);
        self.publish();
        // the maps read HUNTER REVEAL from the game (Game::map_hunters); the admin keeps it current here
        g.admin.hunter_reveal = self.enabled && self.hunter_reveal;
        self.pre = Pre {
            pos: g.p.pos,
            report: g.report.is_some(),
            escape: g.escape.is_some(),
            flashlight: g.p.flashlight_on,
            valid: true,
        };
        if self.enabled {
            self.no_target_before(g);
        }
    }

    /// Call right after `Game::update` with the frame's controls (zeroed while the menu is
    /// open) and the dt the run was stepped with.
    pub fn after_update(&mut self, g: &mut Game, c: &Controls, content: &Content, dt: f32) {
        if !self.enabled || !self.pre.valid {
            return;
        }
        self.pre.valid = false;
        self.apply_after(g, c, content, dt);
    }

    /// Everything the admin draws over the run: the debug overlays, the ADMIN ACTIVE badge and,
    /// when open, the menu (which also runs the clicked actions on the run).
    pub fn draw(&mut self, g: &mut Game, a: &Assets, content: &Content) {
        if !self.enabled {
            return;
        }
        let dt = get_frame_time();
        self.clock += dt;
        self.frame_ms.push_back(dt * 1000.0);
        while self.frame_ms.len() > 240 {
            self.frame_ms.pop_front();
        }
        self.lights_all_on = g.lights_on.iter().all(|l| *l);
        self.draw_overlays(g, a);
        self.draw_badge(a);
        if self.open {
            self.draw_menu(g, a, content);
        }
        self.publish();
    }

    /// _mark_debug: using a tool makes the run a DEBUG RUN (roster / read-only rows opt out).
    fn mark_debug(&mut self) {
        self.debug_run = true;
    }

    fn sel_of(&self, key: &'static str) -> usize {
        self.sel.get(key).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod publish_tests {
    use super::*;

    /// A player (not the admin) with a saved god mode in admin.cfg gets no god mode: the switches
    /// game code reads stay at their defaults until the admin account is signed in.
    #[test]
    fn a_player_who_is_not_the_admin_gets_no_switches() {
        let mut a = Admin::new();
        a.enabled = false;
        a.god_mode = true;
        a.no_target = true;
        a.noclip = true;
        a.run_speed = 4.0;
        a.publish();
        assert!(!GOD.load(Ordering::Relaxed));
        assert!(!NO_TARGET.load(Ordering::Relaxed));
        assert!(!NOCLIP.load(Ordering::Relaxed));
        assert_eq!(f32::from_bits(RUN_SPEED.load(Ordering::Relaxed)), 1.0);
        a.enabled = true;
        a.publish();
        assert!(GOD.load(Ordering::Relaxed));
        assert_eq!(f32::from_bits(RUN_SPEED.load(Ordering::Relaxed)), 4.0);
        a.enabled = false;
        a.publish();
    }
}
