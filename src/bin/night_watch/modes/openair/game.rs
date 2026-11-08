//! The Open Air shift scene (mode `extra`): three outdoor watches, each a walkable 3D set (`world.rs`) with the shared player,
//! prompt, crosshair, pause and end cards, driven by `OpenAirSim` (`sim.rs`); the hunters walk the set's nav graph and stare at
//! its cameras (`cast.rs`). The watch is the level (1 ROOFTOP, 2 BUS STOP, 3 CAMPFIRE). A Custom Night runs the watches whose
//! hunters are on one after another: the sliders feed the sim, a held watch offers NEXT WATCH, and only the last win is recorded.
//!
//! Controls: Z / X bar the stair door and the hatch (rooftop); hold G / H the torch left / right; hold F relight the lantern
//! (rooftop) or fetch firewood (campfire); Y the heat lamp (bus stop) or a log on the fire (campfire); T the tent. Everything
//! also works physically: aim at a station and press use (E), or hold use on a torch, the lantern or the firewood. SPACE opens
//! the security cameras (new in the 3D port).
#![allow(dead_code)]

use super::cast::Cast;
use super::data::{fill_prompt, map_of, name_of, pan_of, watch_label, WatchMap, MODE_ID};
use super::sim::{OpenAirSim, Over, Side, SimEvent, SimInput};
use super::world::{feed_position, OpenAirWorld, SceneState};
use crate::audio::cue_names;
use crate::core::{
    camera_action, hour_index, hour_label, AdminEntry, CallEvent, CallScript, EndCard, Mode, Outcome, Session, ShiftCtx, CAM_NEXT, CAM_PREV, DOOR_LEFT,
    DOOR_RIGHT, INTERACT, LAMP, LIGHT_LEFT, LIGHT_RIGHT, MONITOR, MUTE,
};
use crate::gfx::StationDef;
use crate::hunters::behaviour::BehaviourEvent;
use crate::hunters::{Actor, ActorDriver, PoseState};
use macroquad::prelude::*;

/// The key of the F action: relight the lantern (rooftop), fetch firewood (campfire).
pub const FETCH: &str = "nw_oa_fetch";
pub const TENT: &str = "nw_oa_tent";
/// How long a camera cut shows static.
const FLASH_S: f32 = 0.14;
const SKY: Color = Color::new(0.02, 0.03, 0.06, 1.0);

struct Slot {
    id: &'static str,
    actor: Actor,
    driver: ActorDriver,
}

pub struct OpenAirMode {
    pub watch: usize,
    pub is_custom: bool,
    pub has_more: bool,
    scored: usize,
    level: i32,
    seed: u32,
    map: &'static WatchMap,
    pub sim: OpenAirSim,
    pub cast: Cast,
    world: Option<OpenAirWorld>,
    stations: Vec<StationDef>,
    actors: Vec<Slot>,
    /// What is held this step (the bars stay as they were toggled).
    input: SimInput,
    aimed: Option<usize>,
    monitor_up: bool,
    hour_seen: i32,
    clock_s: f32,
    end_done: bool,
    catcher: Option<&'static str>,
    // admin
    admin_frozen: bool,
    admin_reveal: bool,
    admin_no_scare: bool,
}

impl OpenAirMode {
    /// Builds the rules and the cast (no GL yet; `start` builds the set).
    pub fn new(session: &Session, watch: usize, step_index: usize, has_more: bool, scored: usize) -> OpenAirMode {
        let watch = watch.min(2);
        let map = map_of(watch);
        let seed = session.seed_or_fresh();
        let sim = super::make_sim(session, watch, step_index);
        let look = map.spawn + vec3(0.0, 1.7, 0.0);
        let mut cast = Cast::new(watch, seed.wrapping_add(step_index as u32), look);
        cast.place_all(&sim);
        OpenAirMode {
            watch,
            is_custom: session.is_custom,
            has_more,
            scored,
            level: session.level(),
            seed,
            map,
            sim,
            cast,
            world: None,
            stations: Vec::new(),
            actors: Vec::new(),
            input: SimInput::default(),
            aimed: None,
            monitor_up: false,
            hour_seen: 0,
            clock_s: 0.0,
            end_done: false,
            catcher: None,
            admin_frozen: false,
            admin_reveal: false,
            admin_no_scare: false,
        }
    }

    pub fn label(&self) -> String {
        watch_label(self.watch)
    }

    /// The next watch of this mode: the next level, or, in a Custom Night, the next watch whose hunters are on.
    pub fn has_next_level(&self) -> bool {
        self.has_more || (!self.is_custom && (self.level as usize) < self.scored)
    }

    fn in_tent(&self, feet: Vec3) -> bool {
        self.watch == 2 && self.map.tent.as_ref().map(|t| t.contains(feet.x, feet.z)).unwrap_or(false)
    }

    fn key_text(ctx: &ShiftCtx, action: &str) -> String {
        ctx.bindings.key_text(action).to_uppercase()
    }

    fn play(ctx: &mut ShiftCtx, cues: Vec<crate::core::Cue>) {
        ctx.snd.cues(&cues);
    }

    // -------------------------------------------------------------------------------------------- player actions

    fn toggle_bar(&mut self, ctx: &mut ShiftCtx, side: Side) {
        if self.sim.over != Over::Running || self.watch != 0 {
            return;
        }
        let shut = match side {
            Side::L => {
                self.input.bar_l = !self.input.bar_l;
                self.input.bar_l
            }
            Side::R => {
                self.input.bar_r = !self.input.bar_r;
                self.input.bar_r
            }
        };
        OpenAirMode::play(ctx, cue_names::door(shut));
    }

    fn after_act(&mut self, ctx: &mut ShiftCtx, did: bool) {
        if did {
            OpenAirMode::play(ctx, cue_names::panel_button());
        } else {
            OpenAirMode::play(ctx, cue_names::menu("deny"));
        }
    }

    pub fn press_lamp(&mut self, ctx: &mut ShiftCtx) {
        if self.watch == 1 && self.sim.over == Over::Running {
            let did = self.sim.act_lamp();
            self.after_act(ctx, did);
        }
    }

    pub fn pick_can(&mut self, ctx: &mut ShiftCtx, i: usize) {
        if self.watch == 1 && self.sim.over == Over::Running {
            let did = self.sim.act_can(i);
            self.after_act(ctx, did);
        }
    }

    pub fn throw_log(&mut self, ctx: &mut ShiftCtx) {
        if self.watch == 2 && self.sim.over == Over::Running {
            let did = self.sim.act_log();
            self.after_act(ctx, did);
        }
    }

    /// T: into the tent when out, out when in (the tent is where the player stands).
    pub fn toggle_tent(&mut self, ctx: &mut ShiftCtx) {
        if self.watch != 2 || self.sim.over != Over::Running {
            return;
        }
        let Some(t) = self.map.tent.clone() else { return };
        let inside = self.in_tent(ctx.player.feet);
        let to = if inside { t.outside } else { t.inside };
        let (yaw, pitch) = (ctx.player.cam.yaw, ctx.player.cam.pitch);
        ctx.player.teleport(to, yaw, pitch);
        self.input.tent = !inside;
    }

    fn activate(&mut self, ctx: &mut ShiftCtx, id: &str) {
        match (self.watch, id) {
            (0, "barL") => self.toggle_bar(ctx, Side::L),
            (0, "barR") => self.toggle_bar(ctx, Side::R),
            (1, "lamp") => self.press_lamp(ctx),
            (1, "can0") => self.pick_can(ctx, 0),
            (1, "can1") => self.pick_can(ctx, 1),
            (1, "can2") => self.pick_can(ctx, 2),
            (2, "fire") => self.throw_log(ctx),
            (2, "tent") => self.toggle_tent(ctx),
            _ => {}
        }
    }

    // -------------------------------------------------------------------------------------------- monitor

    fn cam_ids(&self) -> Vec<String> {
        self.map.camera_ids()
    }

    fn cam_index(&self, ctx: &ShiftCtx) -> usize {
        self.map.cameras.iter().position(|c| c.id == ctx.hud.monitor_room).unwrap_or(0)
    }

    fn select_camera(&mut self, ctx: &mut ShiftCtx, index: usize) {
        let ids = self.cam_ids();
        if ids.is_empty() {
            return;
        }
        let i = index.min(ids.len() - 1);
        if ids[i] != ctx.hud.monitor_room {
            ctx.hud.monitor_room = ids[i].clone();
            ctx.hud.cam_flash_s = FLASH_S;
            OpenAirMode::play(ctx, cue_names::camera_switch());
        }
    }

    fn toggle_monitor(&mut self, ctx: &mut ShiftCtx) {
        let up = !ctx.hud.monitor_up;
        ctx.hud.monitor_up = up;
        if up {
            if !self.map.cameras.iter().any(|c| c.id == ctx.hud.monitor_room) {
                if let Some(first) = self.map.cameras.first() {
                    ctx.hud.monitor_room = first.id.clone();
                }
            }
            ctx.hud.cam_flash_s = FLASH_S;
        }
        OpenAirMode::play(ctx, cue_names::monitor_toggle());
    }

    // -------------------------------------------------------------------------------------------- events and the end

    fn on_event(&mut self, ctx: &mut ShiftCtx, e: &SimEvent) {
        let cues = match e {
            SimEvent::Close { id, dist, side } => {
                let pan = if self.watch == 0 { side_pan(id, *side) } else { pan_of(id) };
                cue_names::office("footsteps", pan, 0.15 + (3 - *dist) as f32 * 0.12)
            }
            SimEvent::Repelled { id, side, .. } => {
                let pan = if self.watch == 0 { side_pan(id, *side) } else { pan_of(id) };
                cue_names::office("scrape", pan, 0.8)
            }
            SimEvent::Gust | SimEvent::Weather { .. } => cue_names::office("breath", 0.0, 0.5),
            SimEvent::Relit | SimEvent::TentIn | SimEvent::TentOut => cue_names::panel_button(),
            SimEvent::Can | SimEvent::Gathered => vec![cue_names::cue("pick", 0.0, 0.8)],
            SimEvent::Dry => cue_names::menu("deny"),
            SimEvent::Bus => cue_names::hour_change(),
            SimEvent::Creep => cue_names::office("presence", pan_of("ysolde"), 0.5),
            SimEvent::Stolen => cue_names::office("scrape", pan_of("ysolde"), 0.7),
            SimEvent::Backoff { .. } => cue_names::office("footsteps", pan_of("ysolde"), 0.3),
            _ => Vec::new(),
        };
        if !cues.is_empty() {
            OpenAirMode::play(ctx, cues);
        }
    }

    /// The outcome was decided: freeze the player, the win or caught sound, the catcher lunges.
    fn settle_end(&mut self, ctx: &mut ShiftCtx) {
        if self.sim.over == Over::Running || self.end_done {
            return;
        }
        self.end_done = true;
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.hud.prompt.clear();
        ctx.hud.show_crosshair = false;
        if self.sim.over == Over::Win {
            OpenAirMode::play(ctx, cue_names::win());
        } else {
            let short = ctx.save.short_death_scenes();
            OpenAirMode::play(ctx, cue_names::caught(short));
            if self.sim.cause == "edge" {
                let by = self.sim.by;
                self.catcher = Some(by);
                for s in self.actors.iter_mut().filter(|s| s.id == by) {
                    s.actor.set_pose_state(PoseState::Attack);
                }
            }
        }
    }

    pub fn end_title_and_body(&self) -> (String, String) {
        match self.sim.over {
            Over::Win => {
                let label = self.label();
                let title = if self.has_more {
                    format!("{label} HELD")
                } else if self.is_custom {
                    "CUSTOM NIGHT COMPLETE".to_string()
                } else {
                    format!("WATCH {} COMPLETE", self.watch + 1)
                };
                let lower = label.to_lowercase();
                let mut chars = lower.chars();
                let nice = match chars.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                };
                (title, format!("DAWN\n{nice} held until morning."))
            }
            _ => {
                let who = self.sim.by;
                let name = name_of(who);
                let what = if who == "wynelle" {
                    "The cold took the stop".to_string()
                } else if self.watch == 2 && who == "ysolde" && self.sim.hidden {
                    "Ysolde looked in the tent".to_string()
                } else {
                    format!("{name} got to you")
                };
                ("CAUGHT".to_string(), format!("{what} at {}.", hour_label(hour_index(self.sim.elapsed))))
            }
        }
    }

    // -------------------------------------------------------------------------------------------- what you see

    fn scene_state(&self, feet: Vec3) -> SceneState {
        SceneState {
            lantern: self.sim.lantern,
            lamp_on: self.sim.lamp && self.sim.fuel > 0.0,
            fire: self.sim.fire,
            weather_on: self.sim.weather.on,
            torch_side: self.sim.torch_side,
            feet,
            time_s: self.clock_s,
        }
    }

    /// The HUD text and the set's state, from the rules.
    fn sync_visuals(&mut self, ctx: &mut ShiftCtx, snap: bool) {
        let hour = hour_index(self.sim.elapsed);
        let fetch = OpenAirMode::key_text(ctx, FETCH);
        self.monitor_up = ctx.hud.monitor_up;
        let hud = &mut *ctx.hud;
        hud.hour = hour;
        hud.night_label = format!("8. OPEN AIR - {}{}", if self.is_custom { "CUSTOM - " } else { "" }, self.label());
        hud.show_crosshair = !hud.monitor_up && self.sim.over == Over::Running;
        hud.bars.clear();
        match self.watch {
            0 => {
                hud.power_text = Some(format!("OIL {}%  TORCH {}%", self.sim.oil.round() as i32, self.sim.torch.round() as i32));
                hud.usage_text = Some(if self.sim.lantern { "LANTERN ON".to_string() } else { format!("LANTERN OUT  (hold {fetch})") });
            }
            1 => {
                hud.power_text = Some(format!("WARM {}%  FUEL {}%", self.sim.warm.round() as i32, self.sim.fuel.round() as i32));
                hud.usage_text = Some(format!("CHILL {}%{}", self.sim.chill.round() as i32, if self.sim.bus > 0.0 { "  BUS" } else { "" }));
                let col = if self.sim.chill > 30.0 { [1.0, 0.353, 0.353] } else { [0.498, 0.878, 1.0] };
                hud.bars.push(("CHILL".to_string(), (self.sim.chill / 100.0).clamp(0.0, 1.0) as f32, col));
            }
            _ => {
                hud.power_text = Some(format!("FIRE {}%  LOGS {}  TORCH {}%", self.sim.fire.round() as i32, self.sim.logs, self.sim.torch.round() as i32));
                let mut y_text = String::new();
                if self.sim.y.dist < 2 {
                    y_text = format!("  YSOLDE {}", if self.sim.y.dist == 1 { "IS OUT OF THE TREES" } else { "IS AT THE LOGS" });
                }
                let tent = if self.sim.hidden { format!("IN THE TENT {:.1}s", self.sim.tent_ms / 1000.0) } else { "TENT: OUT".to_string() };
                hud.usage_text = Some(format!("{tent}{y_text}"));
            }
        }
        // cameras: the ring on the map where somebody stares into one
        let pulses: Vec<String> = self.map.cameras.iter().filter(|c| self.cast.staring_at(&c.rooms)).map(|c| c.id.clone()).collect();
        hud.cam_pulse = pulses;
        if hud.monitor_up {
            let idx = self.map.cameras.iter().position(|c| c.id == hud.monitor_room).unwrap_or(0);
            if let Some(c) = self.map.cameras.get(idx) {
                let want = self.cast.stare_pulse(&c.rooms) * FLASH_S;
                if want > hud.cam_flash_s {
                    hud.cam_flash_s = want;
                }
            }
        }
        // the set
        if let Some(w) = self.world.as_mut() {
            let dt = 0.0;
            w.animate([self.input.bar_l && self.sim.oil > 0.0, self.input.bar_r && self.sim.oil > 0.0], dt, snap);
            w.set_lantern(self.sim.lantern);
            w.set_lamp(self.sim.lamp && self.sim.fuel > 0.0);
            w.set_fire(self.sim.fire, self.clock_s);
            w.set_pile(self.sim.logs);
            w.set_cans(&self.sim.cans);
            w.set_bus(self.sim.bus);
        }
        for (i, name) in ["can0", "can1", "can2"].iter().enumerate() {
            let present = self.sim.cans.contains(&i);
            for st in self.stations.iter_mut().filter(|s| s.id == *name) {
                st.disabled = !present;
            }
        }
        self.sync_actors();
    }

    fn sync_actors(&mut self) {
        for slot in self.actors.iter_mut() {
            slot.actor.visible = self.sim.hunter_on(slot.id) || self.admin_reveal;
            let Some(agent) = self.cast.agent(slot.id) else { continue };
            if self.catcher == Some(slot.id) {
                slot.actor.pos = agent.walker.pos;
                slot.actor.yaw = agent.walker.yaw;
                slot.actor.hidden = agent.walker.hidden;
                continue;
            }
            slot.driver.drive(agent, &mut slot.actor);
        }
    }

    /// Put the hunters where the rules have them (a restart, an admin jump).
    fn snap_cast(&mut self) {
        self.cast.place_all(&self.sim);
        self.sync_actors();
    }

    fn render_set(&self, ctx: &ShiftCtx, cam_pos: Vec3, feed: bool) {
        let Some(world) = self.world.as_ref() else { return };
        let st = self.scene_state(ctx.player.feet);
        let lights = if feed { world.feed_lights(&st) } else { world.lights(&st) };
        let weather = if self.sim.weather.on { Some(self.clock_s) } else { None };
        world.draw(&ctx.gfx, &lights, cam_pos, weather);
        for slot in &self.actors {
            slot.actor.draw(&ctx.gfx, &lights, cam_pos);
        }
    }
}

/// Where a step is heard on the rooftop: on the side she is coming from, at her own distance from centre.
pub fn side_pan(id: &str, side: Side) -> f32 {
    let p = pan_of(id).abs();
    match side {
        Side::L => -p,
        Side::R => p,
    }
}

impl Mode for OpenAirMode {
    fn id(&self) -> &'static str {
        MODE_ID
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        let map = self.map;
        ctx.player.teleport(map.spawn, map.spawn_yaw, map.spawn_pitch);
        ctx.player.footprint = Some(map.footprint);
        ctx.player.frozen = false;
        ctx.player.look_frozen = false;
        ctx.player.noclip = false;
        ctx.player.crouch = false;
        ctx.player.flashlight = false;
        self.world = Some(OpenAirWorld::build(self.watch, &ctx.gfx.tex));
        // the stations, with the bound keys in their prompts
        let key = |a: &str| ctx.bindings.key_text(a);
        self.stations = map
            .stations
            .iter()
            .map(|s| StationDef {
                id: s.id.clone(),
                prompt: fill_prompt(&s.prompt, &key),
                pos: s.pos,
                focus: s.focus,
                radius: s.radius,
                disabled: s.id.starts_with("can"),
            })
            .collect();
        // the hunters
        self.actors.clear();
        for id in self.cast.ids().to_vec() {
            if let Some(mut actor) = ctx.hunters.actor(id) {
                actor.visible = false;
                self.actors.push(Slot { id, actor, driver: ActorDriver::new() });
            }
        }
        // the hud
        let h = &mut *ctx.hud;
        h.monitor_available = true;
        h.monitor_up = false;
        h.cam_map = Some(map.layout.clone());
        h.monitor_room = map.cameras.first().map(|c| c.id.clone()).unwrap_or_default();
        h.cam_blips.clear();
        h.show_crosshair = true;
        h.call = if self.is_custom { CallScript::empty() } else { CallScript::for_level(MODE_ID, self.watch as i32 + 1) };
        self.cast.look = ctx.player.eye();
        self.snap_cast();
        self.sync_visuals(ctx, true);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        let inp = ctx.input;
        if self.sim.over != Over::Running {
            return;
        }
        // the cameras
        if inp.just(MONITOR) {
            self.toggle_monitor(ctx);
        } else if ctx.hud.monitor_up && inp.just(INTERACT) {
            self.toggle_monitor(ctx);
        }
        if ctx.hud.monitor_up {
            let count = self.map.cameras.len();
            let now = self.cam_index(ctx);
            for n in 1..=(count.min(8) as i32) {
                if inp.just(camera_action(n)) {
                    self.select_camera(ctx, (n - 1) as usize);
                }
            }
            if inp.just(CAM_NEXT) && count > 0 {
                self.select_camera(ctx, (now + 1) % count);
            }
            if inp.just(CAM_PREV) && count > 0 {
                self.select_camera(ctx, (now + count - 1) % count);
            }
        }
        // the player (the cameras hold him in his chair)
        let up = ctx.hud.monitor_up;
        ctx.player.frozen = up;
        ctx.player.look_frozen = up;
        if let Some(w) = self.world.as_ref() {
            let sens = ctx.save.mouse_sens;
            ctx.player.update(inp, ctx.bindings, w.solids(), sens);
            self.aimed = if up { None } else { ctx.player.aim(&self.stations, w.solids()) };
        }
        self.cast.look = ctx.player.eye();
        let aimed_id = self.aimed.and_then(|i| self.stations.get(i)).map(|s| s.id.clone()).unwrap_or_default();
        ctx.hud.prompt = self.aimed.and_then(|i| self.stations.get(i)).map(|s| s.prompt.clone()).unwrap_or_default();
        // quick keys
        if inp.just(DOOR_LEFT) {
            self.toggle_bar(ctx, Side::L);
        }
        if inp.just(DOOR_RIGHT) {
            self.toggle_bar(ctx, Side::R);
        }
        if inp.just(LAMP) {
            if self.watch == 1 {
                self.press_lamp(ctx);
            } else if self.watch == 2 {
                self.throw_log(ctx);
            }
        }
        if inp.just(TENT) {
            self.toggle_tent(ctx);
        }
        if inp.just(MUTE) && ctx.hud.call.mute() {
            ctx.hud.zellia_muted = true;
        }
        if inp.just(INTERACT) && !up && !aimed_id.is_empty() {
            self.activate(ctx, &aimed_id);
        }
        // held things
        let interact = inp.is_down(INTERACT) && !up;
        let on = |id: &str| interact && aimed_id == id;
        self.input.torch_l = inp.is_down(LIGHT_LEFT) || on("torchL");
        self.input.torch_r = inp.is_down(LIGHT_RIGHT) || on("torchR");
        self.input.relight = inp.is_down(FETCH) || on("lantern");
        self.input.gather = self.watch == 2 && (inp.is_down(FETCH) || on("wood"));
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.admin_frozen || self.sim.over != Over::Running {
            return;
        }
        if self.watch == 2 {
            self.input.tent = self.in_tent(ctx.player.feet);
        }
        let events = self.sim.step(dt_ms, &self.input);
        for ce in ctx.hud.call.update(self.sim.elapsed, dt_ms) {
            match ce {
                CallEvent::Ring => OpenAirMode::play(ctx, cue_names::call_ring()),
                CallEvent::Talk => OpenAirMode::play(ctx, cue_names::call_line()),
                _ => {}
            }
        }
        let hour = hour_index(self.sim.elapsed);
        if hour != self.hour_seen {
            self.hour_seen = hour;
            OpenAirMode::play(ctx, cue_names::hour_change());
        }
        for e in &events {
            self.on_event(ctx, e);
        }
        self.cast.look = ctx.player.eye();
        self.cast.sync(&self.sim, ctx.manual_clock);
        self.settle_end(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        if !ctx.paused {
            self.clock_s += dt_s;
            if ctx.hud.cam_flash_s > 0.0 {
                ctx.hud.cam_flash_s = (ctx.hud.cam_flash_s - dt_s).max(0.0);
            }
            if !ctx.manual_clock {
                let events = self.cast.tick(dt_s);
                for e in &events {
                    match e {
                        BehaviourEvent::FirstStare { subtitle, .. } => {
                            if !subtitle.is_empty() {
                                ctx.hud.set_note(subtitle, 4.0);
                            }
                            OpenAirMode::play(ctx, cue_names::office("static", 0.0, 0.35));
                        }
                        _ => {}
                    }
                }
            }
            for slot in self.actors.iter_mut() {
                slot.actor.animate(dt_s);
            }
            if let Some(w) = self.world.as_mut() {
                w.animate([self.input.bar_l && self.sim.oil > 0.0, self.input.bar_r && self.sim.oil > 0.0], dt_s, ctx.manual_clock);
            }
        }
        self.settle_end(ctx);
        self.sync_visuals_live(ctx);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        if ctx.hud.monitor_up {
            return;
        }
        let cam = ctx.player.cam;
        set_camera(&cam.camera3d());
        clear_background(SKY);
        self.render_set(ctx, cam.pos, false);
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let map = self.map;
        let cam = match map.camera(room).or_else(|| map.cameras.first()) {
            Some(c) => c,
            None => return,
        };
        let pos = feed_position(cam);
        set_camera(&Camera3D {
            position: pos,
            target: cam.target,
            up: Vec3::Y,
            fovy: cam.fov.to_radians(),
            aspect: Some(16.0 / 9.0),
            render_target: Some(ctx.gfx.feed.rt.clone()),
            z_near: 0.05,
            z_far: 80.0,
            ..Default::default()
        });
        clear_background(SKY);
        self.render_set(ctx, pos, true);
        set_default_camera();
    }

    fn outcome(&self) -> Outcome {
        self.sim.outcome()
    }

    fn end_card(&self, _ctx: &ShiftCtx) -> Option<EndCard> {
        match self.sim.over {
            Over::Running => None,
            Over::Win => {
                let (title, body) = self.end_title_and_body();
                Some(EndCard { title, body, can_next: self.has_next_level() })
            }
            Over::Caught => {
                let (title, body) = self.end_title_and_body();
                Some(EndCard { title, body, can_next: false })
            }
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.sim.over == Over::Caught && !self.admin_no_scare {
            Some((self.sim.by.to_string(), self.sim.cause.to_string()))
        } else {
            None
        }
    }

    fn wants_mouse_free(&self) -> bool {
        self.monitor_up
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        Vec::new()
    }

    fn admin_do(&mut self, id: &str) {
        super::admin::run(self, id);
    }

    fn debug_win(&mut self) {
        if self.sim.over == Over::Running {
            self.sim.over = Over::Win;
        }
    }

    fn debug_lose(&mut self) {
        if self.sim.over == Over::Running {
            self.sim.over = Over::Caught;
            self.sim.by = self.sim.hunter_ids().first().copied().unwrap_or("verrine");
            self.sim.cause = if self.watch == 1 { "cold" } else { "edge" };
        }
    }
}

impl OpenAirMode {
    /// The same as `sync_visuals` without the context-free parts that need `&mut ShiftCtx`: used each frame.
    fn sync_visuals_live(&mut self, ctx: &mut ShiftCtx) {
        let snap = ctx.manual_clock;
        self.sync_visuals(ctx, snap);
    }

    // admin hooks (see admin.rs) ---------------------------------------------------------------------------------

    pub fn admin_set_frozen(&mut self, on: bool) {
        self.admin_frozen = on;
    }
    pub fn admin_frozen(&self) -> bool {
        self.admin_frozen
    }
    pub fn admin_set_reveal(&mut self, on: bool) {
        self.admin_reveal = on;
    }
    pub fn admin_reveal(&self) -> bool {
        self.admin_reveal
    }
    pub fn admin_set_no_scare(&mut self, on: bool) {
        self.admin_no_scare = on;
    }
    pub fn admin_no_scare(&self) -> bool {
        self.admin_no_scare
    }
    /// After the admin changed the rules: the hunters stand where the rules have them.
    pub fn admin_resync(&mut self) {
        self.snap_cast();
    }
    pub fn seed(&self) -> u32 {
        self.seed
    }
    /// Jump the clock to a given hour (the admin panel).
    pub fn admin_set_hour(&mut self, hour: i32, elapsed_ms: f64) {
        self.sim.elapsed = elapsed_ms;
        self.hour_seen = hour;
    }
}
