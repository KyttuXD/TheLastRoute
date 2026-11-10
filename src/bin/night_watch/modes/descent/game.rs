//! The Descent shift (`descent_game.gd`): a fixed view from inside a service elevator, driven by `Sim`. Click the service
//! panel (or press 1-6) to repair the fuse boxes, the signal pads and the pipes; hold SPACE (or the panel's button) to keep
//! the door shut on the last stage. Elise walks the sill outside the caged window on her side (a real hunter on the shaft's
//! nav graph, `EliseCtl`) and comes to the glass as the closeness bar fills. Not walkable: the cursor is free, the shared
//! pause, call box and end cards are used. There is no monitor in this mode (as in Godot), so the fixed view is the camera.
#![allow(dead_code)]

use super::admin;
use super::elise::{EliseCtl, ELISE};
use super::panel::{self, PanelAction};
use super::sim::{from_session, Act, Over, Sim, SimEvent, Stage};
use super::world::DescentWorld;
use crate::audio::cue_names;
use crate::core::{AdminEntry, CallEvent, CallScript, EndCard, Mode, Mulberry32, Outcome, Session, ShiftCtx};
use crate::gfx::FpCamera;
use crate::hunters::behaviour::BehaviourEvent;
use crate::hunters::{Actor, ActorDriver};
use crate::ui::theme;
use macroquad::prelude::*;

pub const HINT: &str = "click the panel   1-6 press a button   hold SPACE to hold the door   R radio   P pause";

pub struct DescentMode {
    pub(super) sim: Sim,
    pub(super) descent: i32,
    pub(super) custom: bool,
    seed: u32,
    elise: EliseCtl,
    driver: ActorDriver,
    actor: Option<Actor>,
    world: Option<DescentWorld>,
    pub(super) hold_down: bool,
    /// Admin: the door latch (holds the door without a key).
    pub(super) hold_latch: bool,
    pub(super) frozen: bool,
    pub(super) no_scare: bool,
    pub(super) reveal: bool,
    /// Events from admin actions, played (sounds) on the next frame.
    pub(super) pending: Vec<SimEvent>,
    /// Admin: Some(true) plays this descent's call, Some(false) skips it.
    pub(super) pending_call: Option<bool>,
    pres: Mulberry32,
    stare: f32,
}

impl DescentMode {
    pub fn new(session: &Session) -> DescentMode {
        let seed = session.seed_or_fresh();
        let sim = from_session(session, seed);
        let descent = sim.descent;
        DescentMode {
            sim,
            descent,
            custom: session.is_custom,
            seed,
            elise: EliseCtl::new(seed),
            driver: ActorDriver::new(),
            actor: None,
            world: None,
            hold_down: false,
            hold_latch: false,
            frozen: false,
            no_scare: false,
            reveal: false,
            pending: Vec::new(),
            pending_call: None,
            pres: Mulberry32::new(seed ^ 0x5052_4553),
            stare: 0.0,
        }
    }

    pub fn has_next(&self) -> bool {
        !self.custom && self.descent < self.sim.tun.descents
    }

    /// Play the sounds of sim events.
    fn events(&mut self, ctx: &mut ShiftCtx, events: Vec<SimEvent>) {
        for e in events {
            let cues = match e {
                SimEvent::Step => cue_names::panel_button(),
                SimEvent::Wrong => cue_names::menu("deny"),
                SimEvent::Stage(_) => cue_names::menu("pick"),
                SimEvent::Ride(_) => {
                    let mut c = cue_names::door(false);
                    c.extend(cue_names::hour_change());
                    c
                }
                SimEvent::Win => cue_names::win(),
                SimEvent::Caught => cue_names::caught(ctx.save.short_death_scenes()),
            };
            ctx.snd.cues(&cues);
        }
    }

    /// What the HUD and the set show for the sim's state now.
    fn sync(&mut self, ctx: &mut ShiftCtx) {
        let s = &self.sim;
        let cue = s.meter_cue();
        let shown = (s.stage_idx + 1).min(s.stages.len());
        ctx.hud.clock_text = Some(format!("FLOOR {} / {}", shown, s.stages.len()));
        ctx.hud.night_label = if self.custom {
            "THE DESCENT \u{b7} CUSTOM NIGHT".to_string()
        } else {
            format!("THE DESCENT \u{b7} DESCENT {}", self.descent)
        };
        ctx.hud.power_text = Some(format!("{} {}%", cue.label.to_uppercase(), s.threat.round() as i32));
        ctx.hud.usage_text = Some(format!("WRONG {}", s.wrong));
        let colour = if cue.warn {
            [1.0, 0.353, 0.353]
        } else if cue.slow {
            [0.498, 0.878, 1.0]
        } else {
            [0.498, 0.831, 0.604]
        };
        ctx.hud.bars = vec![("OUTSIDE".to_string(), (s.threat / 100.0).clamp(0.0, 1.0) as f32, colour)];
        let ride = if s.is_riding() { 1.0 - (s.ride / s.tun.ride_ms) as f32 } else { 0.0 };
        if let Some(w) = self.world.as_mut() {
            w.set_ride(ride);
        }
    }

    /// A press on the panel (also what the tests press).
    pub fn press(&mut self, ctx: &mut ShiftCtx, a: Act) {
        if self.sim.over != Over::None {
            return;
        }
        let ev = self.sim.act(a);
        self.events(ctx, ev);
        self.sync(ctx);
    }

    fn closeness_shown(&self) -> f32 {
        if self.sim.level > 0 {
            self.sim.threat as f32
        } else {
            0.0
        }
    }

    fn play_call(&mut self, ctx: &mut ShiftCtx, play: bool) {
        if play {
            let lines = crate::core::call_levels("descent")
                .into_iter()
                .find(|c| c.level == self.descent)
                .map(|c| c.lines)
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| vec!["Zellia, on the headset.".to_string()]);
            ctx.hud.call = CallScript::from_lines(lines, 0.0);
            ctx.hud.zellia_muted = false;
            ctx.hud.call.update(0.0, 0.0);
        } else {
            ctx.hud.call.mute();
            ctx.hud.call_text.clear();
        }
    }
}

impl Mode for DescentMode {
    fn id(&self) -> &'static str {
        "descent"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        self.world = Some(DescentWorld::new(&ctx.gfx.tex));
        self.actor = ctx.hunters.actor(ELISE);
        if let Some(a) = self.actor.as_mut() {
            a.visible = false;
        }
        ctx.hud.monitor_available = false;
        ctx.hud.monitor_up = false;
        ctx.hud.show_crosshair = false;
        ctx.hud.cam_map = None;
        ctx.hud.hint = HINT.to_string();
        ctx.hud.prompt.clear();
        ctx.hud.call = if self.custom { CallScript::empty() } else { CallScript::for_level("descent", self.descent) };
        ctx.hud.zellia_muted = false;
        self.elise.new_night();
        self.sync(ctx);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        if ctx.paused {
            self.hold_down = false;
            return;
        }
        let input = ctx.input;
        if input.just("office_radio") {
            ctx.hud.call.mute();
            ctx.hud.zellia_muted = ctx.hud.call.muted;
        }
        if self.sim.over != Over::None {
            self.hold_down = false;
            return;
        }
        let mv = theme::to_virtual(input.mouse);
        let p = (mv.x, mv.y);
        let on_button = input.click_down && self.sim.kind() == Some(Stage::Hold) && panel::contains(panel::hold_button_rect(), p);
        self.hold_down = input.is_down("nw_hold") || on_button || self.hold_latch;
        if input.click {
            if let Some(PanelAction::Act(a)) = panel::hit(&self.sim, p) {
                self.press(ctx, a);
            }
        }
        if let Some(d) = input.digit {
            if (1..=6).contains(&d) {
                let n = (d - 1) as usize;
                match self.sim.kind() {
                    Some(Stage::Memory) => self.press(ctx, Act::Pad(n)),
                    Some(Stage::Wires) => self.press(ctx, Act::Wire(n)),
                    _ => {}
                }
            }
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.frozen || self.sim.over != Over::None {
            return;
        }
        let ev = self.sim.step(dt_ms, self.hold_down);
        self.events(ctx, ev);
        for e in ctx.hud.call.update(self.sim.elapsed, dt_ms) {
            match e {
                CallEvent::Ring => ctx.snd.cues(&cue_names::call_ring()),
                CallEvent::Talk => ctx.snd.cues(&cue_names::call_line()),
                _ => {}
            }
        }
        self.sync(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        if !self.pending.is_empty() {
            let p = std::mem::take(&mut self.pending);
            self.events(ctx, p);
        }
        if let Some(play) = self.pending_call.take() {
            self.play_call(ctx, play);
        }
        let active = self.sim.level > 0;
        let riding = self.sim.is_riding();
        if !ctx.paused {
            if ctx.manual_clock {
                if active {
                    self.elise.snap(self.sim.side, self.sim.threat);
                }
            } else {
                let evs = self.elise.update(dt_s, self.sim.side, self.sim.threat, riding, active);
                for e in evs {
                    if let BehaviourEvent::FirstStare { subtitle, .. } = e {
                        ctx.hud.set_note(&subtitle, 4.0);
                    }
                }
            }
            self.stare = if active { self.elise.stare_pulse() } else { 0.0 };
            // Elise: visible while the car stands still (and always to a revealing admin)
            let visible = active && ((!riding && self.sim.over != Over::Win) || self.reveal);
            let drove = match (self.actor.as_mut(), self.elise.agent()) {
                (Some(actor), Some(agent)) => {
                    self.driver.drive(agent, actor);
                    actor.visible = visible;
                    actor.animate(dt_s);
                    true
                }
                _ => false,
            };
            if !drove {
                if let Some(actor) = self.actor.as_mut() {
                    actor.visible = false;
                }
            }
        }
        self.sync(ctx);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        let w = match self.world.as_ref() {
            Some(w) => w,
            None => return,
        };
        let v = &w.view;
        clear_background(Color::new(v.background[0], v.background[1], v.background[2], 1.0));
        let cam = FpCamera { pos: v.cam_pos, yaw: 0.0, pitch: v.cam_pitch_deg.to_radians(), fov_deg: v.cam_fov_deg, near: 0.06, far: 100.0 };
        cam.apply();
        let lights = w.lights(self.closeness_shown());
        w.draw(&*ctx.gfx, &lights, cam.pos);
        if let Some(a) = self.actor.as_ref() {
            a.draw(&*ctx.gfx, &lights, cam.pos);
        }
    }

    fn draw_overlay(&mut self, ctx: &mut ShiftCtx) {
        let mv = theme::to_virtual(ctx.input.mouse);
        panel::draw(&self.sim, (mv.x, mv.y), self.hold_down);
        if self.stare > 0.02 {
            // the soft static of her stare
            let (w, h) = (screen_width(), screen_height());
            for _ in 0..28 {
                let y = self.pres.next() as f32 * h;
                let th = 1.0 + self.pres.next() as f32 * 2.0;
                let a = self.stare * 0.5 * (0.4 + 0.6 * self.pres.next() as f32);
                draw_rectangle(0.0, y, w, th, Color::new(0.8, 1.0, 0.9, a));
            }
        }
    }

    fn outcome(&self) -> Outcome {
        match self.sim.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn end_card(&self, _ctx: &ShiftCtx) -> Option<EndCard> {
        match self.sim.over {
            Over::None => None,
            Over::Win => {
                let title = if self.custom { "CUSTOM NIGHT COMPLETE".to_string() } else { format!("DESCENT {} COMPLETE", self.descent) };
                let n = self.sim.wrong;
                let mut body = format!("BOTTOM FLOOR\nReached the bottom with {} wrong attempt{}.", n, if n == 1 { "" } else { "s" });
                let more = self.has_next();
                if more {
                    body.push_str(&format!("\nDescent {} is open.", self.descent + 1));
                }
                Some(EndCard { title, body, can_next: more })
            }
            Over::Caught => Some(EndCard { title: "CAUGHT".to_string(), body: "Elise reached the car.".to_string(), can_next: false }),
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.sim.over == Over::Caught && !self.no_scare {
            Some((ELISE.to_string(), "shaft".to_string()))
        } else {
            None
        }
    }

    fn wants_mouse_free(&self) -> bool {
        true
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        admin::entries(self)
    }

    fn admin_do(&mut self, id: &str) {
        admin::run(self, id);
    }

    fn debug_win(&mut self) {
        if self.sim.over == Over::None {
            self.sim.over = Over::Win;
            self.pending.push(SimEvent::Win);
        }
    }

    fn debug_lose(&mut self) {
        if self.sim.over == Over::None {
            self.sim.over = Over::Caught;
            self.sim.by = ELISE.to_string();
            self.pending.push(SimEvent::Caught);
        }
    }
}
