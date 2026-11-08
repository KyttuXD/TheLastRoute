//! The Mode trait (render-facing half of a shift), the Headless trait (window-less half), the per-frame ShiftCtx,
//! and HudState: the plain data the HUD draws. Plus a small FNV-1a hasher for `Headless::digest`.
#![allow(dead_code)]

use super::calls::CallScript;
use super::controls::Bindings;
use super::data::ModesData;
use super::input::InputFrame;
use super::save::Save;
use super::session::Session;
use macroquad::prelude::Vec2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Running,
    Won,
    Lost,
}

#[derive(Clone, Debug)]
pub struct EndCard {
    pub title: String,
    pub body: String,
    pub can_next: bool,
}

/// Everything a mode may read or change while a shift runs. Built per frame by main.rs.
pub struct ShiftCtx<'a> {
    pub session: &'a Session,
    pub modes: &'a ModesData,
    pub input: &'a InputFrame,
    pub bindings: &'a Bindings,
    pub save: &'a mut Save,
    pub snd: &'a mut crate::audio::Sounds,
    /// Pure data the Hud draws (status, prompt, note, call, monitor state).
    pub hud: &'a mut HudState,
    /// Shaders, textures, feed target.
    pub gfx: &'a mut crate::gfx::Gfx,
    pub hunters: &'a mut crate::hunters::HunterLib,
    pub player: &'a mut crate::player::Player,
    /// Tests and --shot: snap animations, no walkers moving.
    pub manual_clock: bool,
    pub paused: bool,
}

/// A free text block in the 480x270 space (task lists, shop lines).
#[derive(Clone, Debug, Default)]
pub struct HudPanel {
    pub x: f32,
    pub y: f32,
    /// (text, colour, size)
    pub lines: Vec<(String, [f32; 3], f32)>,
}

/// The camera map's data: rooms at (x, z) world spots, the walkable edges and which rooms have a camera.
#[derive(Clone, Debug, Default)]
pub struct CamLayout {
    pub rooms: Vec<(String, Vec2)>,
    pub edges: Vec<(String, String)>,
    pub entrances: Vec<String>,
    pub cams: Vec<String>,
    pub labels: Vec<(String, String)>,
    pub vents: Vec<String>,
    pub office: Option<Vec2>,
}

#[derive(Clone, Default)]
pub struct HudState {
    pub hour: i32,
    pub power: f32,
    pub usage: i32,
    pub night_label: String,
    pub mode_text: String,
    /// Override the default "Power left: n%" / "Usage: |||" lines.
    pub power_text: Option<String>,
    pub usage_text: Option<String>,
    /// Overrides hour_label (descent, mixed endless).
    pub clock_text: Option<String>,
    pub prompt: String,
    pub note: Option<(String, f32)>,
    pub hint: String,
    pub monitor_available: bool,
    pub monitor_up: bool,
    pub monitor_room: String,
    pub cam_label: String,
    pub cam_flash_s: f32,
    pub cam_map: Option<CamLayout>,
    pub cam_blips: Vec<String>,
    pub cam_pulse: Vec<String>,
    pub call: CallScript,
    pub call_text: String,
    pub zellia_muted: bool,
    /// Small labelled bars (chill, noise, beacon), drawn under the status block.
    pub bars: Vec<(String, f32, [f32; 3])>,
    pub panels: Vec<HudPanel>,
    pub show_crosshair: bool,
}

impl HudState {
    /// Show a note for `seconds`.
    pub fn set_note(&mut self, text: &str, seconds: f32) {
        self.note = Some((text.to_string(), seconds));
    }
    /// Count the note down; it disappears at zero.
    pub fn tick_note(&mut self, dt_s: f32) {
        if let Some((_, left)) = &mut self.note {
            *left -= dt_s;
            if *left <= 0.0 {
                self.note = None;
            }
        }
    }
}

/// One line of the dev overlay: a heading or a button the mode runs in `admin_do(id)`.
#[derive(Clone, Debug)]
pub struct AdminEntry {
    pub heading: bool,
    pub label: String,
    pub id: String,
}

impl AdminEntry {
    pub fn heading(label: &str) -> AdminEntry {
        AdminEntry { heading: true, label: label.to_string(), id: String::new() }
    }
    pub fn button(label: &str, id: &str) -> AdminEntry {
        AdminEntry { heading: false, label: label.to_string(), id: id.to_string() }
    }
}

/// Render-facing half of a mode. Created by registry::make_mode; one per shift.
pub trait Mode {
    fn id(&self) -> &'static str;
    /// Build GPU resources (meshes, rigs). Called once, after the window exists.
    fn start(&mut self, ctx: &mut ShiftCtx);
    /// Per frame, BEFORE advance: read ctx.input, change sim input state, toggle doors, open the monitor, etc.
    fn handle_input(&mut self, ctx: &mut ShiftCtx);
    /// Advance the sim by dt_ms (already clamped to 100 per call; main calls repeatedly). Never called while paused
    /// or after the outcome is decided.
    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64);
    /// Per frame animation that runs even while paused or ended (slabs, walkers, flinch timers), real seconds.
    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32);
    /// Player view pass: set the camera and draw. The mode owns the 3D scene.
    fn draw_world(&mut self, ctx: &mut ShiftCtx);
    /// Draw the selected camera feed into ctx.gfx.feed. Only called while hud.monitor_up.
    fn draw_feed(&mut self, _ctx: &mut ShiftCtx, _room: &str) {}
    /// Feed diagnostics for the `--shots` tour: the hunter ids the sim puts in view of camera `room` (a HUD map room id), and
    /// the ids the feed would really draw. `None` for a mode without a crowd.
    fn feed_report(&self, _room: &str) -> Option<(Vec<String>, Vec<String>)> {
        None
    }
    /// Screen-space extras beyond the shared Hud (keypad, shop, descent panel, mask overlay).
    fn draw_overlay(&mut self, _ctx: &mut ShiftCtx) {}
    fn outcome(&self) -> Outcome;
    /// Some(card) once Won or Lost; after the death moment for losses.
    fn end_card(&self, ctx: &ShiftCtx) -> Option<EndCard>;
    /// Death line source: Some((hunter_id, cause_or_empty)) when Lost.
    fn killer(&self) -> Option<(String, String)> {
        None
    }
    /// Monitor, shop, keypad, descent panel.
    fn wants_mouse_free(&self) -> bool {
        false
    }
    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        Vec::new()
    }
    /// Run the dev overlay entry with this id.
    fn admin_do(&mut self, _id: &str) {}
    fn debug_win(&mut self);
    fn debug_lose(&mut self);
}

/// Pure, window-less half. Used by --check, tests and the Mode wrapper.
pub trait Headless {
    fn id(&self) -> &'static str;
    /// Scripted autoplay policy inside the sim (seeded); returns short event names.
    fn step_ms(&mut self, dt_ms: f64) -> Vec<String>;
    fn outcome(&self) -> Outcome;
    fn elapsed_ms(&self) -> f64;
    /// FNV-1a of key sim state (stable across runs).
    fn digest(&self) -> u64;
}

/// FNV-1a, 64 bit: the hash behind `Headless::digest`.
#[derive(Clone, Copy, Debug)]
pub struct Fnv(u64);

impl Fnv {
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Fnv {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self
    }
    pub fn str(&mut self, s: &str) -> &mut Fnv {
        self.bytes(s.as_bytes())
    }
    pub fn u64(&mut self, v: u64) -> &mut Fnv {
        self.bytes(&v.to_le_bytes())
    }
    pub fn i32(&mut self, v: i32) -> &mut Fnv {
        self.bytes(&v.to_le_bytes())
    }
    pub fn f64(&mut self, v: f64) -> &mut Fnv {
        self.bytes(&v.to_bits().to_le_bytes())
    }
    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Fnv::new()
    }
}

/// FNV-1a of a byte string.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    Fnv::new().bytes(bytes).finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_the_published_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn fnv_chaining_depends_on_order_and_value() {
        let a = Fnv::new().f64(1.0).i32(2).str("x").finish();
        let b = Fnv::new().f64(1.0).i32(2).str("x").finish();
        let c = Fnv::new().i32(2).f64(1.0).str("x").finish();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hud_state_defaults_are_empty() {
        let h = HudState::default();
        assert!(h.call.done);
        assert!(!h.monitor_up && !h.show_crosshair);
        assert!(h.note.is_none() && h.cam_map.is_none());
    }

    #[test]
    fn notes_count_down_and_clear() {
        let mut h = HudState::default();
        h.set_note("hello", 1.0);
        h.tick_note(0.5);
        assert!(h.note.is_some());
        h.tick_note(0.6);
        assert!(h.note.is_none());
    }

    #[test]
    fn admin_entries_build() {
        let h = AdminEntry::heading("DOORS");
        assert!(h.heading && h.id.is_empty());
        let b = AdminEntry::button("Win", "win");
        assert!(!b.heading && b.id == "win");
    }

    #[test]
    fn outcome_compares() {
        assert_ne!(Outcome::Running, Outcome::Won);
        assert_eq!(Outcome::Lost, Outcome::Lost);
    }
}
