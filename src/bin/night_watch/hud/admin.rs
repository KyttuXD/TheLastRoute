//! The admin overlay (key `0`), port of `night_watch_admin.gd`. Opens only for an account the server marks `admin`
//! (`AdminGate`); there is no env flag or debug-build gate. It lists the entries a mode gives (`Mode::admin_entries`) under the common ones and
//! returns the id of a clicked row; the caller routes it (ids starting with `@` are common: `@win`, `@lose`) and marks the
//! run DEBUG (`Save::debug_run = true`) on any use, so nothing is saved.
#![allow(dead_code)]

use super::status::*;
use crate::core::{AdminEntry, InputFrame, Mode};
use std::sync::atomic::{AtomicBool, Ordering};
use macroquad::prelude::*;

pub const ROW_H: f32 = 9.0;
const PANEL_RECT: Rect = Rect { x: 6.0, y: 40.0, w: 190.0, h: 224.0 };
const HEAD_H: f32 = 24.0;

/// Who may open the overlay: only an account the server marks `admin` (contract section 9). No env flag, no debug gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AdminGate {
    pub enabled: bool,
}

impl AdminGate {
    pub const OFF: AdminGate = AdminGate { enabled: false };
    /// Tests and `--check`.
    pub fn force_admin() -> AdminGate {
        AdminGate { enabled: true }
    }
    /// From the signed-in account (guests and signed-out are not admin).
    pub fn from_account(acct: Option<&crate::net::account::Account>) -> AdminGate {
        AdminGate { enabled: acct.map(|a| a.admin).unwrap_or(false) }
    }
    /// Read the current account (call after sign-in, sign-out and session restore).
    pub fn current() -> AdminGate {
        AdminGate::from_account(crate::net::account::current().as_ref())
    }
    /// Publish this gate for `enabled()` (the one main.rs reads).
    pub fn publish(self) {
        GATE.store(self.enabled, Ordering::Relaxed);
    }
}

static GATE: AtomicBool = AtomicBool::new(false);

/// The published gate: true only for a signed-in admin (or after `AdminGate::force_admin().publish()`).
pub fn enabled() -> bool {
    GATE.load(Ordering::Relaxed)
}

/// A panel row a mode contributes: label and id, run by `ModeAdmin::apply`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminAction {
    pub label: String,
    pub id: String,
}

impl AdminAction {
    pub fn new(label: &str, id: &str) -> AdminAction {
        AdminAction { label: label.to_string(), id: id.to_string() }
    }
}

/// Port of one `*_admin.gd` provider. A mode implements it in its own file and registers it (see `AdminRegistry`).
/// `apply` changes the mode through the `Mode` trait; the default forwards to `Mode::admin_do`.
pub trait ModeAdmin {
    /// The mode id this panel belongs to (`Mode::id`).
    fn mode_id(&self) -> &'static str;
    /// Heading of the panel (Godot: "<TITLE> SPECIFIC").
    fn title(&self) -> String;
    /// Rows of the panel.
    fn actions(&self) -> Vec<AdminAction>;
    /// Run the row with this id. Returns false when the id is not this panel's.
    fn apply(&self, id: &str, mode: &mut dyn Mode) -> bool {
        if self.actions().iter().any(|a| a.id == id) {
            mode.admin_do(id);
            true
        } else {
            false
        }
    }
}

/// Panels by mode id. Each mode registers in one `register_all` call, e.g. `reg.register(Box::new(ClassicAdmin));`
#[derive(Default)]
pub struct AdminRegistry {
    panels: Vec<Box<dyn ModeAdmin>>,
}

impl AdminRegistry {
    pub fn new() -> AdminRegistry {
        AdminRegistry { panels: Vec::new() }
    }
    pub fn register(&mut self, p: Box<dyn ModeAdmin>) {
        self.panels.retain(|q| q.mode_id() != p.mode_id());
        self.panels.push(p);
    }
    pub fn panel(&self, mode_id: &str) -> Option<&dyn ModeAdmin> {
        self.panels.iter().find(|p| p.mode_id() == mode_id).map(|b| b.as_ref())
    }
    /// Common rows, then the mode's registered panel, then whatever `Mode::admin_entries` adds.
    pub fn entries(&self, mode: &mut dyn Mode) -> Vec<AdminEntry> {
        let mut out = common_entries();
        if let Some(p) = self.panel(mode.id()) {
            out.push(AdminEntry::heading(&p.title()));
            for a in p.actions() {
                out.push(AdminEntry::button(&a.label, &a.id));
            }
        }
        out.extend(mode.admin_entries());
        out
    }
    /// Run a clicked id: `@win` and `@lose` are common, then the panel, then `Mode::admin_do`.
    pub fn run(&self, id: &str, mode: &mut dyn Mode) {
        match id {
            "@win" => mode.debug_win(),
            "@lose" => mode.debug_lose(),
            other => {
                let handled = self.panel(mode.id()).map(|p| p.apply(other, mode)).unwrap_or(false);
                if !handled {
                    mode.admin_do(other);
                }
            }
        }
    }
}

/// Classic's panel (port of `classic_admin.gd`): doors, door lights, all hunters to the doors, Kathy's forced meeting.
/// Ids: `door_l_shut`, `door_l_open`, `door_r_shut`, `door_r_open`, `light_l_on`, `light_l_off`, `light_r_on`,
/// `light_r_off`, `all_to_doors`, `kathy_meet` (Classic's `admin_do` must handle them; show the Kathy row only on Night 7
/// by also returning it from `Mode::admin_entries` instead if preferred).
pub struct ClassicAdmin;

impl ModeAdmin for ClassicAdmin {
    fn mode_id(&self) -> &'static str {
        "classic"
    }
    fn title(&self) -> String {
        "CLASSIC SPECIFIC".to_string()
    }
    fn actions(&self) -> Vec<AdminAction> {
        vec![
            AdminAction::new("SHUT LEFT DOOR", "door_l_shut"),
            AdminAction::new("OPEN LEFT DOOR", "door_l_open"),
            AdminAction::new("SHUT RIGHT DOOR", "door_r_shut"),
            AdminAction::new("OPEN RIGHT DOOR", "door_r_open"),
            AdminAction::new("LEFT LIGHT ON", "light_l_on"),
            AdminAction::new("LEFT LIGHT OFF", "light_l_off"),
            AdminAction::new("RIGHT LIGHT ON", "light_r_on"),
            AdminAction::new("RIGHT LIGHT OFF", "light_r_off"),
            AdminAction::new("ALL HUNTERS TO THEIR DOORS", "all_to_doors"),
            AdminAction::new("KATHY: FORCE MEET HER EYES", "kathy_meet"),
        ]
    }
}

/// The common entries every mode gets (the caller maps `@win` to `Mode::debug_win`, `@lose` to `Mode::debug_lose`).
pub fn common_entries() -> Vec<AdminEntry> {
    vec![
        AdminEntry { heading: true, label: "SHIFT".into(), id: String::new() },
        AdminEntry { heading: false, label: "WIN THE SHIFT".into(), id: "@win".into() },
        AdminEntry { heading: false, label: "LOSE THE SHIFT".into(), id: "@lose".into() },
    ]
}

/// Rows that fit under the header.
pub fn visible_rows() -> usize {
    ((PANEL_RECT.h - HEAD_H - 4.0) / ROW_H).floor() as usize
}

/// Clamp a scroll offset (in rows) to the list.
pub fn clamp_scroll(scroll: i32, total: usize) -> usize {
    let max = total.saturating_sub(visible_rows()) as i32;
    scroll.clamp(0, max.max(0)) as usize
}

pub struct AdminOverlay {
    pub open: bool,
    scroll: i32,
    /// Set once any row was used: the run is DEBUG.
    pub used: bool,
    /// Overrides the published gate (tests, `--check`).
    gate: Option<AdminGate>,
}

impl AdminOverlay {
    pub fn new() -> AdminOverlay {
        AdminOverlay { open: false, scroll: 0, used: false, gate: None }
    }

    /// An overlay with its own gate instead of the published one.
    pub fn with_gate(gate: AdminGate) -> AdminOverlay {
        AdminOverlay { gate: Some(gate), ..AdminOverlay::new() }
    }

    pub fn allowed(&self) -> bool {
        self.gate.map(|g| g.enabled).unwrap_or_else(enabled)
    }

    /// Open or close; refused (and closed) for a non-admin.
    pub fn toggle(&mut self) {
        if self.allowed() {
            self.open = !self.open;
            self.scroll = 0;
        } else {
            self.open = false;
        }
    }

    /// While open the mouse is free and the game should ignore clicks.
    pub fn wants_mouse(&self) -> bool {
        self.open
    }

    /// Handle key 0 (InputFrame.digit), scroll and clicks, then draw. `entries` is the full list (common + mode).
    /// Returns the id of the row clicked this frame.
    pub fn update_and_draw(&mut self, input: &InputFrame, entries: &[AdminEntry], status: &str) -> Option<String> {
        if !self.allowed() {
            self.open = false;
            return None;
        }
        if input.digit == Some(0) {
            self.toggle();
        }
        if !self.open {
            return None;
        }
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            self.scroll -= wheel.signum() as i32 * 2;
        }
        self.scroll = clamp_scroll(self.scroll, entries.len()) as i32;
        let mouse = mouse_v(input);
        fill(PANEL_RECT, Color::new(0.04, 0.03, 0.06, 0.92));
        stroke(PANEL_RECT, Color::new(1.0, 0.3, 0.3, 1.0), 1.0);
        vtext("ADMIN ACTIVE (NOT RECORDED)", PANEL_RECT.x + 5.0, PANEL_RECT.y + 4.0, 7.0, REC_RED);
        let first = status.lines().next().unwrap_or("");
        vtext(&crate::ui::text::glyph(first), PANEL_RECT.x + 5.0, PANEL_RECT.y + 13.0, 5.0, DIM);
        let mut picked = None;
        let start = self.scroll as usize;
        for (i, e) in entries.iter().enumerate().skip(start).take(visible_rows()) {
            let y = PANEL_RECT.y + HEAD_H + (i - start) as f32 * ROW_H;
            let row = Rect::new(PANEL_RECT.x + 3.0, y, PANEL_RECT.w - 6.0, ROW_H);
            let label = crate::ui::text::glyph(&e.label);
            if e.heading {
                vtext(&label, row.x + 2.0, y + 1.0, 6.0, AMBER);
                continue;
            }
            let hot = row.contains(mouse);
            if hot {
                fill(row, Color::new(0.2, 0.16, 0.1, 0.9));
                if input.click {
                    picked = Some(e.id.clone());
                }
            }
            vtext(&label, row.x + 6.0, y + 1.0, 6.0, if hot { AMBER } else { INK });
        }
        if picked.is_some() {
            self.used = true;
        }
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_entries_have_the_win_and_lose_ids() {
        let c = common_entries();
        assert!(c.iter().any(|e| e.id == "@win" && !e.heading));
        assert!(c.iter().any(|e| e.id == "@lose"));
        assert!(c[0].heading);
    }

    use crate::core::{EndCard, Outcome, ShiftCtx};

    #[derive(Default)]
    struct Fake {
        done: Vec<String>,
    }
    impl Mode for Fake {
        fn id(&self) -> &'static str {
            "classic"
        }
        fn start(&mut self, _: &mut ShiftCtx) {}
        fn handle_input(&mut self, _: &mut ShiftCtx) {}
        fn advance_ms(&mut self, _: &mut ShiftCtx, _: f64) {}
        fn animate(&mut self, _: &mut ShiftCtx, _: f32) {}
        fn draw_world(&mut self, _: &mut ShiftCtx) {}
        fn outcome(&self) -> Outcome {
            Outcome::Running
        }
        fn end_card(&self, _: &ShiftCtx) -> Option<EndCard> {
            None
        }
        fn admin_do(&mut self, id: &str) {
            self.done.push(id.to_string());
        }
        fn debug_win(&mut self) {
            self.done.push("win".into());
        }
        fn debug_lose(&mut self) {
            self.done.push("lose".into());
        }
    }

    #[test]
    fn overlay_refuses_for_non_admin() {
        let mut o = AdminOverlay::with_gate(AdminGate::OFF);
        o.toggle();
        assert!(!o.open && !o.wants_mouse());
        assert!(!AdminGate::from_account(None).enabled);
    }

    #[test]
    fn overlay_opens_for_admin_and_toggles() {
        let mut o = AdminOverlay::with_gate(AdminGate::force_admin());
        o.toggle();
        assert!(o.open);
        o.toggle();
        assert!(!o.open);
    }

    #[test]
    fn actions_apply_through_the_registry() {
        let mut reg = AdminRegistry::new();
        reg.register(Box::new(ClassicAdmin));
        let mut m = Fake::default();
        let e = reg.entries(&mut m);
        assert!(e.iter().any(|x| x.heading && x.label == "CLASSIC SPECIFIC"));
        assert!(e.iter().any(|x| x.id == "door_l_shut"));
        reg.run("door_l_shut", &mut m);
        reg.run("@win", &mut m);
        reg.run("@lose", &mut m);
        reg.run("unknown", &mut m);
        assert_eq!(m.done, vec!["door_l_shut", "win", "lose", "unknown"]);
        assert!(reg.panel("mask").is_none());
    }

    #[test]
    fn scroll_is_clamped_to_the_list() {
        assert_eq!(clamp_scroll(-5, 100), 0);
        assert_eq!(clamp_scroll(5, 3), 0);
        assert_eq!(clamp_scroll(1000, 100), 100 - visible_rows());
        assert!(visible_rows() >= 18);
    }
}
