#![allow(dead_code)]
//! Masked Hours (mode `mask`), the shift scene (port of `mask_game.gd`): the night guard's small office with no doors, in the
//! dark, and a wing of rooms the monitor looks into. You walk the office and USE stations with the interact key: the MASK on
//! the desk (put it on / take it off), the MONITOR (cameras 1-8; the music box is CAM 1), the MUSIC BOX (hold to wind: the
//! monitor jumps to CAM 1), the two VENT grates and the HALL LIGHT (hold for the light), the desk lamp, the chair. Quick keys
//! (on keys the shared map leaves free): mask (Z), monitor (Space / U), hold G / H the left / right vent light, hold X the
//! hall light, hold R to wind, Y the lamp; in the monitor 1-8 or the arrows pick a camera.
//!
//! The rules are `MaskSim` (seeded like the 2D game) behind `MaskControls` (the mask and the monitor are one exclusive pair).
//! The hunters physically walk the wing's nav graph (`Stage`); the monitor renders each security camera live into the feed
//! target at the Godot mounts. `ctx.manual_clock` snaps the walkers so a screenshot or test is repeatable.

use super::controls::{CtlEvent, CtlKind, MaskControls, Panel};
use super::data::{map, rules, VENT_HUNTERS};
use super::sim::{Ev, HallState, MaskSim, Over, RushState, SimInput, VentState};
use super::walk::Stage;
use super::world::{feed_lights, office_lights, MaskWorld};
use crate::audio::cue_names;
use crate::core::{
    call_levels, camera_action, display_name, hour_index, rating_of, AdminEntry, CallEvent, CallScript, Cue, EndCard, Mode, Outcome, Session,
    ShiftCtx, CAM_NEXT, CAM_PREV, DOOR_LEFT, HOUR_MS, LAMP, MAX_AI, MONITOR, NIGHT_MS, POINTS_PER_LEVEL,
};
use crate::gfx::{Gfx, LightSet};
use crate::hunters::{Actor, ActorDriver, BehaviourEvent};
use macroquad::prelude::*;
use std::collections::BTreeMap;

pub const WIN_TEXT: &str = "You held the wing until 6 AM.";
/// (held control, station that holds it, quick-key action).
const HOLDS: [(&str, &str, &str); 4] = [
    ("lightL", "ventL", "nw_light_left"),
    ("lightR", "ventR", "nw_light_right"),
    ("lightH", "hall_light", "nw_hall_light"),
    ("winPtr", "musicbox", ""),
];
const STATIC_S: f32 = 0.14;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Playing,
    Won,
    Lost,
}

pub struct MaskMode {
    night: i32,
    seed: u32,
    custom: bool,
    pub sim: MaskSim,
    pub ctl: MaskControls,
    pub stage: Stage,
    world: Option<MaskWorld>,
    actors: BTreeMap<String, Actor>,
    drivers: BTreeMap<String, ActorDriver>,
    state: State,
    hour_seen: i32,
    monitor_shown: bool,
    aimed: Option<usize>,
    call_left: f32,
    fan_t: f32,
    mask_cover: f32,
    pending_ctl: Vec<CtlEvent>,
    pending_ev: Vec<Ev>,
    pending_call: bool,
    pending_skip: bool,
    pub admin_frozen: bool,
    pub admin_no_scare: bool,
    pub admin_reveal: bool,
    pub admin_noclip: bool,
    pub admin_god: bool,
}

impl MaskMode {
    pub fn new(session: &Session) -> MaskMode {
        let custom = session.is_custom;
        let night = if custom { 5 } else { session.night.clamp(1, 6) };
        let seed = session.seed_or_fresh();
        MaskMode {
            night,
            seed,
            custom,
            sim: MaskSim::new(night, seed, session.custom_levels.clone(), custom),
            ctl: MaskControls::new(),
            stage: Stage::new(seed),
            world: None,
            actors: BTreeMap::new(),
            drivers: BTreeMap::new(),
            state: State::Playing,
            hour_seen: 0,
            monitor_shown: false,
            aimed: None,
            call_left: 0.0,
            fan_t: 0.0,
            mask_cover: 0.0,
            pending_ctl: Vec::new(),
            pending_ev: Vec::new(),
            pending_call: false,
            pending_skip: false,
            admin_frozen: false,
            admin_no_scare: false,
            admin_reveal: false,
            admin_noclip: false,
            admin_god: false,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn night(&self) -> i32 {
        self.night
    }

    fn has_next_level(&self, ctx: &ShiftCtx) -> bool {
        !self.custom && ctx.session.has_next(ctx.modes)
    }

    // --- text -------------------------------------------------------------------------------------------------------

    fn hint(&self, ctx: &ShiftCtx) -> String {
        let b = ctx.bindings;
        if self.ctl.monitor_wanted() {
            format!(
                "{} / {} or 1-8 camera   {} or {} close   hold {} wind (CAM 1)   hold {} / {} vent light on CAM 6 / 7",
                b.key_text("nw_cam_prev"),
                b.key_text("nw_cam_next"),
                b.key_text("nw_monitor"),
                b.key_text("interact"),
                b.key_text("nw_wind"),
                b.key_text("nw_light_left"),
                b.key_text("nw_light_right")
            )
        } else {
            format!(
                "WASD move   {} use   {} mask (no doors here)   {} monitor   {} / {} vent lights   {} hall light   {} wind   {} lamp   {} pause",
                b.key_text("interact"),
                b.key_text("nw_door_left"),
                b.key_text("nw_monitor"),
                b.key_text("nw_light_left"),
                b.key_text("nw_light_right"),
                b.key_text("nw_hall_light"),
                b.key_text("nw_wind"),
                b.key_text("nw_lamp"),
                b.key_text("pause")
            )
        }
    }

    fn mode_text(&self) -> String {
        format!("2. Masked Hours - {}", if self.custom { "Custom Night".to_string() } else { format!("Night {}", self.night) })
    }

    fn show_call(&mut self, ctx: &mut ShiftCtx, text: &str) {
        ctx.hud.call_text = text.to_string();
        self.call_left = 4.0;
    }

    fn flash_static(&self, ctx: &mut ShiftCtx) {
        ctx.hud.cam_flash_s = STATIC_S;
    }

    fn play(ctx: &mut ShiftCtx, cues: Vec<Cue>) {
        ctx.snd.cues(&cues);
    }

    // --- the controls -------------------------------------------------------------------------------------------------

    /// While a panel is wanted the player's hands are on it: no walking, no looking around.
    fn refresh_lock(&self, ctx: &mut ShiftCtx) {
        let frozen = self.ctl.monitor_busy() || self.state != State::Playing;
        ctx.player.frozen = frozen;
        ctx.player.look_frozen = frozen;
    }

    fn on_control_event(&mut self, ctx: &mut ShiftCtx, e: CtlEvent) {
        match e.kind {
            CtlKind::Raise => {
                if e.panel == Panel::Monitor {
                    Self::play(ctx, cue_names::monitor_toggle());
                } else {
                    Self::play(ctx, cue_names::panel_button());
                }
            }
            CtlKind::Up => {
                if e.panel == Panel::Monitor {
                    self.monitor_shown = true;
                    ctx.hud.monitor_up = true;
                    ctx.hud.monitor_room = self.ctl.cam.clone();
                    self.flash_static(ctx);
                }
            }
            CtlKind::Lower => {
                if e.panel == Panel::Monitor {
                    self.monitor_shown = false;
                    ctx.hud.monitor_up = false;
                    Self::play(ctx, cue_names::monitor_toggle());
                }
            }
            CtlKind::Down => {}
            CtlKind::Lamp => Self::play(ctx, cue_names::panel_button()),
        }
    }

    /// One control input (mask, monitor, lightL, lightR, lightH, winKey, winPtr, lamp, cam:<room>) through the exclusive-panel
    /// machine.
    pub fn press(&mut self, ctx: &mut ShiftCtx, action: &str, on: bool) {
        if self.state != State::Playing {
            return;
        }
        let events = self.ctl.press(action, on);
        for e in events {
            self.on_control_event(ctx, e);
        }
        if action.starts_with("cam:") && self.ctl.is_up(Panel::Monitor) {
            ctx.hud.monitor_room = self.ctl.cam.clone();
        }
        if (action == "winKey" || action == "winPtr") && on && self.ctl.is_up(Panel::Monitor) {
            ctx.hud.monitor_room = self.ctl.cam.clone();
        }
        self.refresh_lock(ctx);
    }

    /// The raise / lower animation ends at once (the player waits for it); no shift time passes.
    fn settle(&mut self, ctx: &mut ShiftCtx) {
        let events = self.ctl.step(500.0);
        for e in events {
            self.on_control_event(ctx, e);
        }
        self.refresh_lock(ctx);
    }

    pub fn select_camera(&mut self, ctx: &mut ShiftCtx, room: &str) {
        if self.state != State::Playing || !MaskControls::is_camera(room) {
            return;
        }
        if !self.ctl.monitor_wanted() {
            return;
        }
        if !self.ctl.is_up(Panel::Monitor) {
            self.settle(ctx);
        }
        self.press(ctx, &format!("cam:{room}"), true);
        self.flash_static(ctx);
        Self::play(ctx, cue_names::camera_switch());
    }

    fn drain_pending(&mut self, ctx: &mut ShiftCtx) {
        let ctl: Vec<CtlEvent> = std::mem::take(&mut self.pending_ctl);
        for e in ctl {
            self.on_control_event(ctx, e);
        }
        let evs: Vec<Ev> = std::mem::take(&mut self.pending_ev);
        for e in evs.iter() {
            self.on_event(ctx, e);
        }
        if self.pending_call {
            self.pending_call = false;
            let lines = call_levels("mask").into_iter().find(|c| c.level == self.night).map(|c| c.lines).unwrap_or_default();
            let lines = if lines.is_empty() { vec!["Zellia, on the headset.".to_string()] } else { lines };
            ctx.hud.call = CallScript::from_lines(lines, 0.0);
            ctx.hud.zellia_muted = false;
        }
        if self.pending_skip {
            self.pending_skip = false;
            ctx.hud.call.mute();
            ctx.hud.call_text.clear();
            self.call_left = 0.0;
        }
        self.process_over(ctx);
    }

    // --- events -------------------------------------------------------------------------------------------------------

    fn pan_of(e: &Ev) -> f32 {
        match e {
            Ev::Duct { side, .. } | Ev::Vent { side, .. } | Ev::Fetch { side, .. } | Ev::Creep { side, .. } | Ev::VentRetreat { side, .. } => {
                if *side == 'L' {
                    -1.0
                } else {
                    1.0
                }
            }
            Ev::Room { room, .. } => rules().pan_of(room),
            _ => 0.0,
        }
    }

    fn on_event(&mut self, ctx: &mut ShiftCtx, e: &Ev) {
        let pan = Self::pan_of(e);
        match e {
            Ev::Room { .. } => Self::play(ctx, cue_names::office("footsteps", pan, 0.5)),
            Ev::Hall { .. } => Self::play(ctx, cue_names::office("footsteps", 0.0, 0.7)),
            Ev::Step { step, .. } => Self::play(ctx, cue_names::office("footsteps", 0.0, 0.7 + 0.1 * *step as f32)),
            Ev::Office { .. } => Self::play(ctx, cue_names::office("crimson_door", 0.0, 1.0)),
            Ev::Leave { .. } | Ev::RushRetreat { .. } | Ev::VentRetreat { .. } => Self::play(ctx, cue_names::office("scrape", pan, 0.9)),
            Ev::Rush { .. } | Ev::Door { .. } => Self::play(ctx, cue_names::attack_warning(0.0)),
            Ev::FlashHit { .. } => Self::play(ctx, cue_names::office("presence", 0.0, 1.0)),
            Ev::Duct { .. } | Ev::Fetch { .. } | Ev::Creep { .. } => Self::play(ctx, cue_names::office("scrape", pan, 0.8)),
            Ev::Vent { .. } => Self::play(ctx, cue_names::attack_warning(pan)),
            Ev::Dark { .. } => {
                Self::play(ctx, cue_names::power_out());
                self.show_call(ctx, "The flashlight is dead.");
            }
            Ev::BoxEmpty => {
                Self::play(ctx, cue_names::attack_warning(0.0));
                self.show_call(ctx, "The music box has stopped.");
            }
            Ev::BoxSaved => Self::play(ctx, cue_names::panel_button()),
            _ => {}
        }
        let room = match e {
            Ev::Room { room, .. } | Ev::Duct { room, .. } => Some(room.as_str()),
            _ => None,
        };
        if self.monitor_shown && room == Some(self.ctl.cam.as_str()) {
            self.flash_static(ctx);
        }
    }

    // --- the end of a shift -----------------------------------------------------------------------------------------

    fn freeze_player(&mut self, ctx: &mut ShiftCtx) {
        self.ctl.cancel();
        self.monitor_shown = false;
        ctx.hud.monitor_up = false;
        self.mask_cover = 0.0;
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
    }

    fn process_over(&mut self, ctx: &mut ShiftCtx) {
        if self.state != State::Playing {
            return;
        }
        match self.sim.over {
            Over::Win => {
                self.state = State::Won;
                self.freeze_player(ctx);
                Self::play(ctx, cue_names::win());
            }
            Over::Caught => {
                self.state = State::Lost;
                self.freeze_player(ctx);
                let short = ctx.save.short_death_scenes();
                Self::play(ctx, cue_names::caught(short));
            }
            Over::None => {}
        }
    }

    // --- what you see -------------------------------------------------------------------------------------------------

    /// The camera rooms each hunter is in now (REVEAL puts a red dot on each).
    fn hunter_rooms(&self) -> Vec<String> {
        let mut rooms: Vec<String> = Vec::new();
        let mut add = |room: &str| {
            let cam = rules().room(room).map(|r| r.cam > 0).unwrap_or(false);
            if cam && !rooms.iter().any(|r| r == room) {
                rooms.push(room.to_string());
            }
        };
        add(&self.sim.hall.room);
        add(&self.sim.rush.room);
        for id in VENT_HUNTERS {
            add(&self.sim.vent_state(id).room);
        }
        rooms
    }

    fn sync_visuals(&mut self, ctx: &mut ShiftCtx) {
        let lit = self.sim.lit;
        let lamp = self.ctl.lamp && self.sim.battery > 0.0;
        let (glint_vl, glint_vr) = (self.sim.vent_glint('L'), self.sim.vent_glint('R'));
        let (glint_dl, glint_dr) = (self.sim.glint("ductL"), self.sim.glint("ductR"));
        let box_pct = self.sim.box_pct as f32;
        if let Some(w) = self.world.as_mut() {
            w.set_lamp(lamp);
            w.set_hall_button(lit.h);
            w.set_box(box_pct);
            w.set_glints(glint_vl, glint_vr, glint_dl, glint_dr);
        }
        self.mask_cover = self.ctl.cover(Panel::Mask);
        let devices = i32::from(lit.l || lit.r) + i32::from(lit.h) + i32::from(self.sim.lamp) + i32::from(self.monitor_shown);
        let hint = self.hint(ctx);
        let mode_text = self.mode_text();
        let prompt = match (self.monitor_shown, self.aimed, self.world.as_ref()) {
            (false, Some(i), Some(w)) => w.stations.get(i).map(|s| s.prompt.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        let mut label = rules().cam_label(&self.ctl.cam);
        if self.ctl.cam == "music" {
            label.push_str(&format!("    BOX {}%    HOLD TO WIND", box_pct.round() as i32));
        }
        let blips = if self.admin_reveal { self.hunter_rooms() } else { Vec::new() };
        let h = &mut *ctx.hud;
        h.hour = hour_index(self.sim.elapsed);
        h.power = self.sim.battery as f32;
        h.usage = devices;
        h.night_label = self.mode_text();
        h.mode_text = mode_text;
        h.power_text = Some(format!("BATTERY {}%", self.sim.battery.ceil() as i32));
        h.usage_text = Some(format!("MUSIC BOX {}%", self.sim.box_pct.round() as i32));
        h.monitor_available = self.state == State::Playing;
        h.cam_label = label;
        h.hint = hint;
        h.prompt = prompt;
        h.show_crosshair = !self.monitor_shown && self.state == State::Playing;
        h.cam_blips = blips;
        if self.monitor_shown {
            h.monitor_room = self.ctl.cam.clone();
        }
    }

    /// Place and animate the actors from the walkers. No GL.
    fn update_actors(&mut self, dt_s: f32) {
        for id in rules().cast.iter() {
            let (tgt, agent) = match (self.stage.target(id), self.stage.crowd.agent(id)) {
                (Some(t), Some(a)) => (t, a),
                _ => continue,
            };
            let actor = match self.actors.get_mut(id.as_str()) {
                Some(a) => a,
                None => continue,
            };
            let driver = self.drivers.entry(id.clone()).or_default();
            actor.visible = tgt.shown;
            actor.pos = agent.walker.pos;
            actor.yaw = agent.walker.yaw;
            actor.hidden = agent.walker.hidden;
            if tgt.driven {
                driver.reset();
                if id == "nyxie" {
                    actor.set_locomotion(false, false);
                } else {
                    actor.set_locomotion(true, true);
                }
            } else {
                let owns = VENT_HUNTERS.contains(&id.as_str()) && actor.update_vent(tgt.shaft, agent.walker.moving, dt_s);
                if owns {
                    driver.reset();
                } else {
                    driver.drive(agent, actor);
                }
            }
            actor.animate(dt_s);
        }
    }

    fn draw_actors(&self, gfx: &Gfx, lights: &LightSet, cam: Vec3) {
        for a in self.actors.values() {
            a.draw(gfx, lights, cam);
        }
    }

    /// The mask over the view: full-screen cream with the eye slits you see the wing through.
    fn draw_mask(cover: f32) {
        if cover <= 0.0 {
            return;
        }
        let (w, h) = (screen_width(), screen_height());
        let a = cover.clamp(0.0, 1.0);
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.88, 0.86, 0.80, 0.97 * a));
        let eye = vec2(w * 0.12, h * 0.05);
        for cx in [w * 0.34, w * 0.66] {
            draw_rectangle(cx - eye.x * 0.5, h * 0.42 - eye.y * 0.5, eye.x, eye.y, Color::new(0.05, 0.05, 0.08, 0.92 * a));
        }
        draw_rectangle(w * 0.46, h * 0.62, w * 0.08, h * 0.012, Color::new(0.35, 0.33, 0.30, a));
    }

    // --- admin ---------------------------------------------------------------------------------------------------------

    fn admin_mask(&mut self, on: bool) {
        let up = self.ctl.is_up(Panel::Mask);
        if up != on || (self.ctl.want == Panel::Mask) != on {
            let evs = self.ctl.press("mask", true);
            self.pending_ctl.extend(evs);
        }
        let evs = self.ctl.step(500.0);
        self.pending_ctl.extend(evs);
    }

    fn admin_press(&mut self, action: &str, on: bool) {
        let evs = self.ctl.press(action, on);
        self.pending_ctl.extend(evs);
    }

    /// A step of the rules with the controls as they are, edited by `f` (admin only).
    fn admin_step(&mut self, dt_ms: f64, f: &dyn Fn(&mut SimInput)) {
        let mut input = self.ctl.control_input();
        f(&mut input);
        let evs = self.sim.step(dt_ms, &input);
        self.pending_ev.extend(evs);
    }

    fn admin_status_rows(&self) -> Vec<String> {
        let s = &self.sim;
        let mut rows = Vec::new();
        rows.push(format!(
            "MASKED HOURS  NIGHT {}  {} (SEED {})",
            self.night,
            crate::core::hour_label(hour_index(s.elapsed)),
            self.seed
        ));
        rows.push(format!(
            "BATTERY {}%  MUSIC BOX {}%  MASK {}",
            s.battery.ceil() as i32,
            s.box_pct.round() as i32,
            if self.ctl.is_up(Panel::Mask) { "ON" } else { "OFF" }
        ));
        rows.push(format!(
            "FROZEN {}  GOD {}  NO SCARE {}  SPEED {:.1}X",
            self.admin_frozen, self.admin_god, self.admin_no_scare, s.hunter_speed
        ));
        rows.push(format!("THESSINE  {:?}  {}", s.hall.state, s.hall.room));
        rows.push(format!("UMBRIEL  {:?}  FLASHES {}/{}", s.rush.state, s.rush.flashes, s.rush.goal));
        for id in VENT_HUNTERS {
            let v = s.vent_state(id);
            rows.push(format!("{}  {:?}  {}", id.to_uppercase(), v.state, v.room));
        }
        rows
    }
}

impl Mode for MaskMode {
    fn id(&self) -> &'static str {
        "mask"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        let m = map();
        self.world = Some(MaskWorld::build(&ctx.gfx.tex));
        ctx.player.teleport(m.spawn, 0.0, -0.1);
        ctx.player.footprint = Some((-m.half_w, m.front_z, m.half_w, m.back_z));
        ctx.player.frozen = false;
        ctx.player.look_frozen = false;
        ctx.player.flashlight = false;
        for id in rules().cast.iter() {
            if let Some(mut a) = ctx.hunters.actor_in(id, "mask") {
                a.visible = false;
                self.actors.insert(id.clone(), a);
                self.drivers.insert(id.clone(), ActorDriver::new());
            }
        }
        ctx.hud.cam_map = Some(crate::hud::cam_map::from_nav(&m.cam_map, super::data::NAV_JSON, &[]));
        ctx.hud.monitor_available = true;
        ctx.hud.monitor_up = false;
        ctx.hud.monitor_room = "music".to_string();
        ctx.hud.show_crosshair = true;
        ctx.hud.call = if self.custom { CallScript::empty() } else { CallScript::for_level("mask", self.night) };
        self.stage.sync(&self.sim, ctx.player.eye(), true);
        self.update_actors(0.0);
        self.sync_visuals(ctx);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        if self.state != State::Playing {
            return;
        }
        let inp: &crate::core::InputFrame = ctx.input;
        // the player walks and looks (frozen while a panel holds the hands)
        if let Some(w) = self.world.as_ref() {
            let sens = ctx.save.mouse_sens;
            ctx.player.noclip = self.admin_noclip;
            ctx.player.update(inp, ctx.bindings, w.solids(), sens);
            self.aimed = ctx.player.aim(&w.stations, w.solids());
        }
        let aimed_id: Option<String> = match (self.aimed, self.world.as_ref()) {
            (Some(i), Some(w)) => w.stations.get(i).map(|s| s.id.clone()),
            _ => None,
        };

        if inp.just(DOOR_LEFT) {
            self.press(ctx, "mask", true);
        }
        if inp.just(LAMP) {
            self.press(ctx, "lamp", true);
        }
        if inp.just(MONITOR) {
            self.press(ctx, "monitor", true);
        }
        if inp.just("interact") {
            if self.ctl.monitor_wanted() {
                // the use key closes the monitor
                self.press(ctx, "monitor", true);
            } else if let Some(id) = aimed_id.as_deref() {
                match id {
                    "monitor" | "musicbox" => self.press(ctx, "monitor", true),
                    "mask" => self.press(ctx, "mask", true),
                    "lamp" => self.press(ctx, "lamp", true),
                    "chair" => {
                        ctx.player.teleport(vec3(0.0, 0.0, 0.45), 0.0, -0.1);
                    }
                    _ => {}
                }
            }
        }
        if self.ctl.monitor_wanted() {
            for n in 1..=8 {
                if inp.just(camera_action(n)) {
                    if let Some(room) = rules().room_for_cam(n) {
                        self.select_camera(ctx, &room);
                    }
                }
            }
            if inp.just(CAM_NEXT) {
                let room = self.ctl.cycle_cam(1);
                self.select_camera(ctx, &room);
            }
            if inp.just(CAM_PREV) {
                let room = self.ctl.cycle_cam(-1);
                self.select_camera(ctx, &room);
            }
        }
        // the held controls: a quick key, or the use key held on the matching station
        for (action, station, key) in HOLDS.iter() {
            let by_key = !key.is_empty() && inp.is_down(key);
            let by_station = inp.is_down("interact") && aimed_id.as_deref() == Some(*station);
            let want = by_key || by_station;
            if want != self.ctl.held(action) {
                self.press(ctx, action, want);
            }
        }
        let wind = inp.is_down("nw_wind");
        if wind != self.ctl.held("winKey") {
            self.press(ctx, "winKey", wind);
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.state != State::Playing || self.admin_frozen {
            return;
        }
        let events = self.ctl.step(dt_ms);
        for e in events {
            self.on_control_event(ctx, e);
        }
        self.refresh_lock(ctx);
        let input = self.ctl.control_input();
        let mut events = self.sim.step(dt_ms, &input);
        for ce in ctx.hud.call.update(self.sim.elapsed, dt_ms) {
            match ce {
                CallEvent::Ring => Self::play(ctx, cue_names::call_ring()),
                CallEvent::Talk => Self::play(ctx, cue_names::call_line()),
                _ => {}
            }
        }
        if self.admin_god && self.sim.over == Over::Caught {
            self.sim.god_rescue();
            events.clear();
        }
        let hour = hour_index(self.sim.elapsed);
        if hour != self.hour_seen {
            self.hour_seen = hour;
            Self::play(ctx, cue_names::hour_change());
        }
        for e in events.iter() {
            self.on_event(ctx, e);
        }
        self.process_over(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        self.drain_pending(ctx);
        ctx.hud.cam_flash_s = (ctx.hud.cam_flash_s - dt_s).max(0.0);
        if self.call_left > 0.0 && !ctx.paused {
            self.call_left -= dt_s;
            if self.call_left <= 0.0 {
                ctx.hud.call_text.clear();
            }
        }
        if !ctx.paused {
            // the desk fan's blades flicker
            self.fan_t += dt_s;
            if self.fan_t >= 0.06 {
                self.fan_t = 0.0;
                if let Some(w) = self.world.as_mut() {
                    w.flip_fan();
                }
            }
            self.stage.sync(&self.sim, ctx.player.eye(), ctx.manual_clock);
            if !ctx.manual_clock {
                for e in self.stage.tick(dt_s) {
                    if let BehaviourEvent::FirstStare { subtitle, .. } = e {
                        ctx.hud.set_note(&subtitle, 4.0);
                    }
                }
            }
            self.update_actors(dt_s);
            // a hunter staring into the camera you are watching: a soft static pulse on the feed
            if self.monitor_shown {
                let pulse = self.stage.crowd.stare_pulse(&self.ctl.cam);
                if pulse > 0.0 {
                    ctx.hud.cam_flash_s = ctx.hud.cam_flash_s.max(pulse * STATIC_S);
                }
            }
        }
        self.sync_visuals(ctx);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        clear_background(Color::new(0.015, 0.015, 0.025, 1.0));
        if let Some(w) = self.world.as_ref() {
            ctx.player.cam.apply();
            let lamp = self.ctl.lamp && self.sim.battery > 0.0;
            let lights = office_lights(self.sim.lit, lamp, ctx.player.flashlight_spot());
            let cam = ctx.player.cam.pos;
            w.draw(&*ctx.gfx, &lights, cam);
            self.draw_actors(&*ctx.gfx, &lights, cam);
        }
        // the mask over everything the 3D pass drew
        set_default_camera();
        Self::draw_mask(self.mask_cover);
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let cam = match map().camera(room) {
            Some(c) => c.clone(),
            None => return,
        };
        let w = match self.world.as_ref() {
            Some(w) => w,
            None => return,
        };
        set_camera(&Camera3D {
            position: cam.mount,
            target: cam.target,
            up: Vec3::Y,
            fovy: cam.fov.to_radians(),
            aspect: Some(16.0 / 9.0),
            render_target: Some(ctx.gfx.feed.rt.clone()),
            z_near: 0.05,
            z_far: 80.0,
            ..Default::default()
        });
        clear_background(Color::new(0.02, 0.03, 0.03, 1.0));
        let lights = feed_lights(room, self.sim.lit);
        w.draw(&*ctx.gfx, &lights, cam.mount);
        // a duct camera sees a crawling hunter; every other camera skips one inside a duct
        let duct = Actor::is_duct_room(room);
        for a in self.actors.values() {
            a.draw_in(&*ctx.gfx, &lights, cam.mount, duct);
        }
        ctx.gfx.feed.end();
    }

    fn feed_report(&self, room: &str) -> Option<(Vec<String>, Vec<String>)> {
        let inside: Vec<String> = self.stage.crowd.visible_in(room).iter().map(|a| a.id.clone()).collect();
        let drawn = inside.iter().filter(|id| self.actors.get(*id).map_or(false, |a| a.drawn_in_room(room))).cloned().collect();
        Some((inside, drawn))
    }

    fn tour_protect(&mut self) {
        self.admin_god = true;
        self.admin_no_scare = true;
        self.sim.infinite_battery = true;
        let e = crate::core::tour_clock_wrap(self.sim.elapsed);
        if e != self.sim.elapsed {
            self.sim.elapsed = e;
            self.hour_seen = hour_index(e);
        }
    }

    fn draw_overlay(&mut self, _ctx: &mut ShiftCtx) {
        if self.monitor_shown && !self.sim.frozen.is_empty() {
            crate::ui::text::text("HELD STILL", 30.0, 40.0, 9.0, Color::new(1.0, 0.85, 0.4, 1.0));
        }
    }

    fn outcome(&self) -> Outcome {
        match self.state {
            State::Playing => Outcome::Running,
            State::Won => Outcome::Won,
            State::Lost => Outcome::Lost,
        }
    }

    fn end_card(&self, ctx: &ShiftCtx) -> Option<EndCard> {
        match self.state {
            State::Playing => None,
            State::Won => {
                if self.custom {
                    let score: i32 = ctx.session.custom_levels.values().map(|v| (*v).clamp(0, MAX_AI)).sum::<i32>() * POINTS_PER_LEVEL;
                    let max = rules().cast.len() as i32 * MAX_AI * POINTS_PER_LEVEL;
                    Some(EndCard {
                        title: "CUSTOM NIGHT COMPLETE".to_string(),
                        body: format!("6:00 AM\n{}\nScore {} of {}: {}", WIN_TEXT, score, max, rating_of(score, max)),
                        can_next: false,
                    })
                } else {
                    let more = self.has_next_level(ctx);
                    let tail = if more { format!("\nNight {} is open.", self.night + 1) } else { "\nYou cleared every night.".to_string() };
                    Some(EndCard { title: format!("NIGHT {} COMPLETE", self.night), body: format!("6:00 AM\n{}{}", WIN_TEXT, tail), can_next: more })
                }
            }
            State::Lost => {
                let name = display_name(&self.sim.by);
                Some(EndCard { title: "CAUGHT".to_string(), body: format!("{} reached you on night {}.", name, self.night), can_next: false })
            }
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.state != State::Lost {
            return None;
        }
        let by = self.sim.by.clone();
        let cause = match by.as_str() {
            "nyxie" => "darkness",
            "glyra" => "music box",
            _ => "",
        };
        Some((by, cause.to_string()))
    }

    fn wants_mouse_free(&self) -> bool {
        self.ctl.monitor_busy()
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        let mut out = Vec::new();
        out.push(AdminEntry::heading("STATUS"));
        for row in self.admin_status_rows() {
            out.push(AdminEntry::heading(&row));
        }
        out.push(AdminEntry::heading("TIME"));
        for h in 0..6 {
            out.push(AdminEntry::button(&format!("JUMP TO {}", crate::core::hour_label(h)), &format!("hour_{h}")));
        }
        out.push(AdminEntry::heading("BATTERY"));
        out.push(AdminEntry::button("BATTERY 0%", "battery_0"));
        out.push(AdminEntry::button("BATTERY 20%", "battery_20"));
        out.push(AdminEntry::button("BATTERY 100%", "battery_100"));
        out.push(AdminEntry::button(if self.sim.infinite_battery { "INFINITE BATTERY: ON" } else { "INFINITE BATTERY: OFF" }, "toggle_infinite"));
        out.push(AdminEntry::heading("SHIFT"));
        out.push(AdminEntry::button(if self.admin_frozen { "FREEZE TIME: ON" } else { "FREEZE TIME: OFF" }, "toggle_freeze"));
        out.push(AdminEntry::button(if self.admin_god { "GOD MODE: ON" } else { "GOD MODE: OFF" }, "toggle_god"));
        out.push(AdminEntry::button(if self.admin_no_scare { "NO JUMPSCARE: ON" } else { "NO JUMPSCARE: OFF" }, "toggle_noscare"));
        out.push(AdminEntry::button(if self.admin_reveal { "REVEAL HUNTERS: ON" } else { "REVEAL HUNTERS: OFF" }, "toggle_reveal"));
        out.push(AdminEntry::button(if self.admin_noclip { "NOCLIP: ON" } else { "NOCLIP: OFF" }, "toggle_noclip"));
        out.push(AdminEntry::button("HUNTER SPEED 0.5X", "speed_0.5"));
        out.push(AdminEntry::button("HUNTER SPEED 1X", "speed_1"));
        out.push(AdminEntry::button("HUNTER SPEED 2X", "speed_2"));
        out.push(AdminEntry::button("HUNTER SPEED 4X", "speed_4"));
        out.push(AdminEntry::button("PLAY ZELLIA'S CALL", "call_play"));
        out.push(AdminEntry::button("SKIP THE CALL", "call_skip"));
        out
    }

    fn admin_do(&mut self, id: &str) {
        match id {
            "mask_on" => self.admin_mask(true),
            "mask_off" => self.admin_mask(false),
            "timer_0" | "timer_1000" | "timer_2000" | "timer_2400" => {
                let ms: f64 = id[6..].parse().unwrap_or(0.0);
                if self.sim.hall.state != HallState::Office {
                    self.sim.place_hunter("thessine", "office");
                }
                self.sim.hall.fooled = ms;
            }
            "box_100" => self.sim.box_pct = 100.0,
            "box_50" => self.sim.box_pct = 50.0,
            "box_10" => self.sim.box_pct = 10.0,
            "box_0" => self.sim.box_pct = 0.0,
            "lightL_on" => self.admin_press("lightL", true),
            "lightL_off" => self.admin_press("lightL", false),
            "lightR_on" => self.admin_press("lightR", true),
            "lightR_off" => self.admin_press("lightR", false),
            "hall_on" => self.admin_press("lightH", true),
            "hall_off" => self.admin_press("lightH", false),
            "flash_hall" => {
                // one quick flash of the hall light (what answers Umbriel); a dead flashlight gets a little charge first
                if self.sim.battery <= 0.0 {
                    self.sim.battery = 5.0;
                    self.sim.dark = 0.0;
                }
                self.admin_step(100.0, &|i| i.light_h = true);
                self.admin_step(100.0, &|i| i.light_h = false);
            }
            "lamp" => self.admin_press("lamp", true),
            "thessine_office" => self.sim.place_hunter("thessine", "office"),
            "umbriel_hall" => self.sim.place_hunter("umbriel", "hall"),
            "verrine_ventL" => self.sim.place_hunter("verrine", "ventL"),
            "all_home" => self.sim.all_home(),
            "battery_0" => self.sim.battery = 0.0,
            "battery_20" | "battery_100" => {
                self.sim.battery = if id == "battery_20" { 20.0 } else { 100.0 };
                self.sim.dark = 0.0;
            }
            "toggle_infinite" => self.sim.infinite_battery = !self.sim.infinite_battery,
            "toggle_freeze" => self.admin_frozen = !self.admin_frozen,
            "toggle_god" => self.admin_god = !self.admin_god,
            "toggle_noscare" => self.admin_no_scare = !self.admin_no_scare,
            "toggle_reveal" => self.admin_reveal = !self.admin_reveal,
            "toggle_noclip" => self.admin_noclip = !self.admin_noclip,
            "call_play" => self.pending_call = true,
            "call_skip" => self.pending_skip = true,
            _ => {
                if let Some(h) = id.strip_prefix("hour_") {
                    if let Ok(hour) = h.parse::<i32>() {
                        self.sim.elapsed = (hour as f64 * HOUR_MS).clamp(0.0, NIGHT_MS - 1000.0);
                        self.hour_seen = hour;
                    }
                } else if let Some(x) = id.strip_prefix("speed_") {
                    if let Ok(v) = x.parse::<f64>() {
                        self.sim.hunter_speed = v.clamp(0.1, 8.0);
                    }
                }
            }
        }
    }

    fn debug_win(&mut self) {
        if self.state == State::Playing {
            self.sim.over = Over::Win;
        }
    }

    fn debug_lose(&mut self) {
        if self.state == State::Playing {
            self.sim.over = Over::Caught;
            self.sim.by = "thessine".to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(night: i32, seed: u32) -> MaskMode {
        let mut s = Session::new();
        s.start_mode("mask", night, Default::default(), Some(seed));
        MaskMode::new(&s)
    }

    #[test]
    fn a_new_shift_starts_playing_with_the_night_table() {
        let m = mode(3, 5);
        assert_eq!(m.id(), "mask");
        assert_eq!(m.outcome(), Outcome::Running);
        assert_eq!(m.night(), 3);
        assert_eq!(m.sim.base_level("umbriel"), 10);
        assert!(!m.wants_mouse_free());
        assert!(m.killer().is_none());
    }

    #[test]
    fn a_custom_shift_is_night_five_with_the_sliders() {
        let mut s = Session::new();
        let mut lv = BTreeMap::new();
        lv.insert("thessine".to_string(), 17);
        s.start_mode("mask", 1, lv, Some(1));
        let m = MaskMode::new(&s);
        assert_eq!(m.night(), 5);
        assert!(m.custom);
        assert_eq!(m.sim.base_level("thessine"), 17);
    }

    #[test]
    fn the_admin_rows_change_the_rules() {
        let mut m = mode(1, 1);
        m.admin_do("thessine_office");
        assert_eq!(m.sim.hall.state, HallState::Office);
        m.admin_do("timer_2400");
        assert_eq!(m.sim.hall.fooled, 2400.0);
        m.admin_do("box_10");
        assert_eq!(m.sim.box_pct, 10.0);
        m.admin_do("hour_3");
        assert_eq!(m.sim.elapsed, 3.0 * HOUR_MS);
        m.admin_do("speed_2");
        assert_eq!(m.sim.hunter_speed, 2.0);
        m.admin_do("battery_20");
        assert_eq!(m.sim.battery, 20.0);
        m.admin_do("toggle_god");
        assert!(m.admin_god);
        m.admin_do("umbriel_hall");
        assert_eq!(m.sim.rush.state, RushState::Run);
        m.admin_do("verrine_ventL");
        assert_eq!(m.sim.verrine.state, VentState::Vent);
        m.admin_do("all_home");
        assert_eq!(m.sim.hall.state, HallState::Away);
        // the mask goes through the controls machine, with its events queued for the next frame
        m.admin_do("mask_on");
        assert!(m.ctl.is_up(Panel::Mask));
        assert!(!m.pending_ctl.is_empty());
        m.admin_do("mask_off");
        assert!(!m.ctl.is_up(Panel::Mask));
    }

    #[test]
    fn debug_win_and_lose_decide_the_shift() {
        let mut m = mode(1, 1);
        m.debug_win();
        assert_eq!(m.sim.over, Over::Win);
        let mut l = mode(1, 1);
        l.debug_lose();
        assert_eq!(l.sim.over, Over::Caught);
        assert_eq!(l.sim.by, "thessine");
    }

    #[test]
    fn a_flash_from_the_admin_panel_counts_as_one_flash() {
        let mut m = mode(3, 2);
        m.admin_do("umbriel_hall");
        m.sim.infinite_battery = true;
        m.admin_do("flash_hall");
        assert_eq!(m.sim.rush.flashes, 1);
        assert!(!m.pending_ev.is_empty());
    }

    #[test]
    fn the_admin_list_has_status_and_the_generic_rows() {
        let mut m = mode(1, 1);
        let rows = m.admin_entries();
        assert!(rows.iter().any(|r| r.heading && r.label.starts_with("MASKED HOURS")));
        for id in ["hour_0", "hour_5", "battery_0", "toggle_infinite", "toggle_freeze", "toggle_god", "speed_4", "call_play", "call_skip"] {
            assert!(rows.iter().any(|r| r.id == id), "{id}");
        }
        // every generic id is one admin_do understands
        for r in rows.iter().filter(|r| !r.heading) {
            m.admin_do(&r.id);
        }
    }
}
