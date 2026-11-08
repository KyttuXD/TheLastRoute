//! Dead Air (mode `lure`), the shift (port of lure_game.gd, lure_panel.gd and lure_sounds.gd): the broadcast booth (first person,
//! the shared player, prompt and crosshair), with the two DOOR buttons sealing the vent openings (the slabs are the vent covers),
//! the LIGHT buttons switching the heater and the fan, and the monitor on the desk looking into the radio station. Rules: `sim.rs`.
//!
//! In the monitor: 1-8 pick a camera, L lures Xivvy to the room on screen, Q / W / E / R / T reboot audio / cameras / vents /
//! heater / fan (or click the buttons). Quick keys outside it: Z / X seal the left / right vent, G / H heater / fan, Space the
//! monitor, Y the lamp, R mutes Zellia.
//!
//! The hunters WALK (`presence.rs`, nav graph `data/nav/lure.json`): the sim says where Xivvy is, the crowd carries her there,
//! and she is visible crossing the feeds. Idle behaviour (stare at a camera, a task, waiting) runs between moves.
#![allow(dead_code)]

use super::data::{is_vent_room, tuning, ROOMS};
use super::map::{map, SPAWN};
use super::presence::{booth_focus, visitor_pos, Presence};
use super::sim::{Over, Sim, SimEvent, SimOpts, SysState};
use super::world::{booth_ambient, booth_lights, feed_lights, Booth, Hall, PropKit};
use crate::audio::cue_names;
use crate::core::*;
use crate::gfx::{Gfx, LightSet};
use crate::hunters::behaviour::{ActorDriver, BehaviourEvent};
use crate::hunters::{yaw_toward, Actor};
use crate::ui::theme::fill_rect;
use crate::ui::{text as ui_text, ButtonStyle, Ui};
use macroquad::prelude::*;
use std::collections::BTreeMap;

/// Seconds the static flash lasts (the HUD's `STATIC_SECONDS`).
const FLASH_S: f32 = 0.14;
/// Seconds a message ("Cameras is down...") stays in the call box.
const MESSAGE_S: f32 = 4.0;

fn capitalize_word(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// Live switches the admin hooks set (the Godot `_admin_*` fields).
#[derive(Clone, Copy, Debug, Default)]
pub struct AdminFlags {
    pub frozen: bool,
    pub no_scare: bool,
    pub reveal: bool,
    pub noclip: bool,
}

pub struct LureMode {
    pub night: i32,
    pub is_custom: bool,
    pub seed: u32,
    pub score: i32,
    pub sim: Sim,
    presence: Presence,
    booth: Option<Booth>,
    hall: Option<Hall>,
    actors: BTreeMap<String, Actor>,
    drivers: BTreeMap<String, ActorDriver>,
    vent_owned: BTreeMap<String, bool>,
    /// The room the monitor shows (the rules' id).
    pub monitor_room: String,
    pub monitor_up: bool,
    pub lamp_on: bool,
    pub admin: AdminFlags,
    aim: Option<usize>,
    hour_seen: i32,
    end_done: bool,
    msg: Option<(String, f32)>,
    teleport_pending: bool,
    fx: Mulberry32,
    frame: u32,
    started: bool,
}

pub fn new_mode(session: &Session) -> LureMode {
    let mut opts = SimOpts::from_session(session);
    opts.seed = session.seed_or_fresh();
    let score = session.custom_levels.values().map(|v| (*v).clamp(0, MAX_AI)).sum::<i32>() * POINTS_PER_LEVEL;
    LureMode {
        night: if session.is_custom { 5 } else { session.night.clamp(1, 6) },
        is_custom: session.is_custom,
        seed: opts.seed,
        score,
        sim: Sim::new(opts.clone()),
        presence: Presence::new(opts.seed, false),
        booth: None,
        hall: None,
        actors: BTreeMap::new(),
        drivers: BTreeMap::new(),
        vent_owned: BTreeMap::new(),
        monitor_room: "storage".to_string(),
        monitor_up: false,
        lamp_on: true,
        admin: AdminFlags::default(),
        aim: None,
        hour_seen: 0,
        end_done: false,
        msg: None,
        teleport_pending: false,
        fx: Mulberry32::new(opts.seed ^ 0x6c75_7265),
        frame: 0,
        started: false,
    }
}

impl LureMode {
    // --- the monitor ----------------------------------------------------------------------------------------------------

    fn watching(&self) -> String {
        if self.monitor_up && self.sim.online("cams") {
            self.monitor_room.clone()
        } else {
            String::new()
        }
    }

    pub fn open_monitor(&mut self, ctx: &mut ShiftCtx) {
        if self.sim.over != Over::None || self.monitor_up {
            return;
        }
        self.monitor_up = true;
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.hud.cam_flash_s = FLASH_S;
        ctx.snd.cues(&cue_names::monitor_toggle());
    }

    pub fn close_monitor(&mut self, ctx: &mut ShiftCtx) {
        if !self.monitor_up {
            return;
        }
        self.monitor_up = false;
        ctx.player.frozen = self.sim.over != Over::None;
        ctx.player.look_frozen = self.sim.over != Over::None;
        ctx.snd.cues(&cue_names::monitor_toggle());
    }

    /// The room a `--monitor ROOM` request names, as a sim room id (accepts a map id or a sim id).
    fn requested_sim_room(room: &str) -> Option<String> {
        if ROOMS.contains(&room) {
            return Some(room.to_string());
        }
        let m = map();
        m.map_to_sim(room).filter(|r| ROOMS.contains(r)).map(|r| r.to_string())
    }

    /// `--monitor ROOM` makes the shell raise the HUD's monitor flag right after `start`, but this mode keeps its own monitor
    /// state and rewrites the HUD from it every frame, so the request was lost. On the first frame, take it over: open the
    /// monitor and point it at that camera.
    fn adopt_requested_monitor(&mut self, ctx: &mut ShiftCtx) {
        if self.frame != 0 || self.monitor_up || !ctx.hud.monitor_up {
            return;
        }
        if let Some(room) = Self::requested_sim_room(&ctx.hud.monitor_room) {
            self.monitor_room = room;
        }
        self.open_monitor(ctx);
    }

    fn toggle_monitor(&mut self, ctx: &mut ShiftCtx) {
        if self.monitor_up {
            self.close_monitor(ctx);
        } else {
            self.open_monitor(ctx);
        }
    }

    pub fn select_camera(&mut self, ctx: &mut ShiftCtx, room: &str) {
        if self.sim.over != Over::None || !ROOMS.contains(&room) {
            return;
        }
        if room != self.monitor_room {
            ctx.hud.cam_flash_s = FLASH_S;
            ctx.snd.cues(&cue_names::camera_switch());
        }
        self.monitor_room = room.to_string();
    }

    fn select_number(&mut self, ctx: &mut ShiftCtx, n: i32) {
        if n >= 1 && (n as usize) <= ROOMS.len() {
            self.select_camera(ctx, ROOMS[n as usize - 1]);
        }
    }

    fn cycle_camera(&mut self, ctx: &mut ShiftCtx, step: i32) {
        let i = ROOMS.iter().position(|r| *r == self.monitor_room).unwrap_or(0) as i32;
        let next = (i + step).rem_euclid(ROOMS.len() as i32) as usize;
        self.select_camera(ctx, ROOMS[next]);
    }

    // --- player actions (also what the tests and the panel press) -----------------------------------------------------

    fn deny(ctx: &mut ShiftCtx) {
        ctx.snd.cues(&cue_names::office("control", 0.0, 0.3));
    }

    pub fn toggle_seal(&mut self, ctx: &mut ShiftCtx, side: char) {
        if self.sim.over != Over::None {
            return;
        }
        if self.sim.seal(side, None) {
            ctx.snd.cues(&cue_names::door(self.sim.seals[if side == 'L' { 0 } else { 1 }]));
        }
    }

    /// Returns the message to show ("" when it worked).
    pub fn toggle_appliance(&mut self, ctx: &mut ShiftCtx, name: &str) -> String {
        if self.sim.over != Over::None {
            return String::new();
        }
        if !self.sim.has_climate || (name == "fan" && !self.sim.has_fan) {
            Self::deny(ctx);
            return format!("NOTHING TO {} TONIGHT", if name == "heater" { "HEAT" } else { "COOL" });
        }
        if !self.sim.toggle(name) {
            Self::deny(ctx);
            return format!("{} IS DOWN // REBOOT IT ON THE MONITOR", name.to_uppercase());
        }
        ctx.snd.cues(&cue_names::panel_button());
        String::new()
    }

    pub fn do_lure(&mut self, ctx: &mut ShiftCtx) -> bool {
        if self.sim.over != Over::None || !self.monitor_up {
            return false;
        }
        let room = self.monitor_room.clone();
        if self.sim.lure(&room) {
            ctx.snd.cues(&cue_names::office("presence", tuning().pan(&room), 0.8));
            true
        } else {
            Self::deny(ctx);
            false
        }
    }

    pub fn do_reboot(&mut self, ctx: &mut ShiftCtx, system: &str) -> bool {
        if self.sim.over != Over::None {
            return false;
        }
        if self.sim.reboot(system) {
            ctx.snd.cues(&cue_names::monitor_toggle());
            true
        } else {
            Self::deny(ctx);
            false
        }
    }

    fn toggle_lamp(&mut self) {
        self.lamp_on = !self.lamp_on;
        if let Some(b) = &mut self.booth {
            b.set_lamp(self.lamp_on);
        }
    }

    fn note(ctx: &mut ShiftCtx, message: &str) {
        if !message.is_empty() {
            ctx.hud.set_note(message, 2.5);
        }
    }

    fn say(&mut self, ctx: &mut ShiftCtx, text: &str) {
        ctx.hud.call_text = text.to_string();
        self.msg = Some((text.to_string(), MESSAGE_S));
    }

    fn use_station(&mut self, ctx: &mut ShiftCtx, id: &str) {
        match id {
            "monitor" => self.toggle_monitor(ctx),
            "doorL_door" => {
                self.toggle_seal(ctx, 'L');
                ctx.snd.cues(&cue_names::panel_button());
            }
            "doorR_door" => {
                self.toggle_seal(ctx, 'R');
                ctx.snd.cues(&cue_names::panel_button());
            }
            "doorL_light" => {
                let m = self.toggle_appliance(ctx, "heater");
                Self::note(ctx, &m);
            }
            "doorR_light" => {
                let m = self.toggle_appliance(ctx, "fan");
                Self::note(ctx, &m);
            }
            "lamp" => self.toggle_lamp(),
            "chair" => {
                ctx.player.teleport(vec3(0.0, 0.0, -0.4), 0.0, -0.15);
                Self::note(ctx, "YOU SIT AT THE CONSOLE");
            }
            _ => {}
        }
    }

    // --- events -----------------------------------------------------------------------------------------------------------

    fn on_event(&mut self, ctx: &mut ShiftCtx, e: &SimEvent) {
        let t = tuning();
        match e {
            SimEvent::Moved { room, from } => {
                let kind = if room.starts_with("vent") { "scrape" } else { "footsteps" };
                ctx.snd.cues(&cue_names::office(kind, t.pan(room), 0.8));
                if self.monitor_up && (*room == self.monitor_room || *from == self.monitor_room) {
                    ctx.hud.cam_flash_s = FLASH_S;
                }
            }
            SimEvent::AtOffice { room, from } => {
                let pan = t.pan(room);
                ctx.snd.cues(&cue_names::office("crimson_door", pan, 1.0));
                ctx.snd.cues(&cue_names::attack_warning(pan));
                if self.monitor_up && *from == self.monitor_room {
                    ctx.hud.cam_flash_s = FLASH_S;
                }
            }
            SimEvent::Retreat { side } => {
                ctx.snd.cues(&cue_names::office("scrape", if *side == 'L' { -0.6 } else { 0.6 }, 0.9));
            }
            SimEvent::Phantom { .. } => {
                ctx.snd.cues(&cue_names::office("static", 0.0, 0.9));
                if self.monitor_up {
                    ctx.hud.cam_flash_s = FLASH_S;
                }
            }
            SimEvent::Broken { system, .. } => {
                ctx.snd.cues(&cue_names::office("drain", 0.0, 1.0));
                let label = t.system(system).map(|s| s.label.clone()).unwrap_or_default();
                let text = format!("{} is down. Reboot it from the monitor.", capitalize_word(&label));
                self.say(ctx, &text);
            }
            SimEvent::Fixed { .. } => {
                ctx.snd.cues(&cue_names::office("control", 0.0, 0.9));
            }
            SimEvent::Caught { .. } | SimEvent::Win => {}
        }
    }

    /// The first frame an outcome is decided: close the monitor, freeze the player, play the sting.
    fn finish_if_needed(&mut self, ctx: &mut ShiftCtx) {
        if self.sim.over == Over::None || self.end_done {
            return;
        }
        self.end_done = true;
        self.close_monitor(ctx);
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        match self.sim.over {
            Over::Win => ctx.snd.cues(&cue_names::win()),
            Over::Caught => {
                let short = ctx.save.short_death_scenes();
                ctx.snd.cues(&cue_names::caught(short));
            }
            Over::None => {}
        }
    }

    // --- what the HUD shows -----------------------------------------------------------------------------------------------

    fn hint(&self, bindings: &Bindings) -> String {
        let mut active: Vec<&str> = vec!["nw_door_left", "nw_door_right", "nw_light_left", "nw_light_right", "nw_lamp", "nw_monitor"];
        if crate::hud::admin::enabled() {
            active.push("admin");
        }
        if self.monitor_up {
            let k = |a: &str| bindings.key_text(a);
            format!(
                "{} / {} or 1-8 camera | {} close | {} lure | {}{}{}{}{} reboot",
                k(CAM_PREV),
                k(CAM_NEXT),
                k(MONITOR),
                k("nw_lure"),
                k("nw_reboot_audio"),
                k("nw_reboot_cams"),
                k("nw_reboot_vent"),
                k("nw_reboot_heater"),
                k("nw_reboot_fan"),
            )
        } else {
            bindings.hint_line(false, &active)
        }
    }

    fn sync_hud(&self, hud: &mut HudState, bindings: &Bindings) {
        let s = &self.sim;
        hud.hour = hour_index(s.elapsed);
        hud.power = 100.0;
        hud.usage = 0;
        hud.night_label = if self.is_custom { "CUSTOM NIGHT".to_string() } else { format!("NIGHT {}", self.night) };
        hud.mode_text.clear();
        hud.power_text = Some(format!("AIR {}%", s.air_left()));
        hud.usage_text = Some(if s.has_climate {
            format!("TEMP {} F", s.temp.round() as i32)
        } else {
            format!("SEALS {}{}", if s.seals[0] { "L" } else { "-" }, if s.seals[1] { "R" } else { "-" })
        });
        hud.monitor_available = s.over == Over::None;
        hud.monitor_up = self.monitor_up;
        let m = map();
        hud.monitor_room = m.sim_to_map(&self.monitor_room).unwrap_or("").to_string();
        hud.cam_label.clear();
        if hud.cam_map.is_none() {
            hud.cam_map = Some(m.layout());
        }
        hud.show_crosshair = !self.monitor_up && s.over == Over::None;
        hud.hint = self.hint(bindings);
        hud.cam_blips = if self.admin.reveal {
            let mut v = Vec::new();
            if let Some(id) = m.sim_to_map(&s.killer.room) {
                v.push(id.to_string());
            }
            if let Some(p) = &s.phantom {
                if let Some(id) = m.sim_to_map(&p.room) {
                    v.push(id.to_string());
                }
            }
            v
        } else {
            Vec::new()
        };
        hud.cam_pulse = ROOMS
            .iter()
            .filter(|r| self.presence.crowd.staring_at(r))
            .filter_map(|r| m.sim_to_map(r).map(|s| s.to_string()))
            .collect();
    }

    fn sync_visuals(&mut self, manual: bool) {
        let shut = self.sim.seals;
        let Some(b) = &mut self.booth else { return };
        if manual {
            b.snap_slabs(shut);
        }
        b.set_button("doorL_door", shut[0]);
        b.set_button("doorR_door", shut[1]);
        b.set_button("doorL_light", self.sim.heater_on);
        b.set_button("doorR_light", self.sim.fan_on);
    }

    /// Put the hunters' actors where the crowd has them.
    fn drive_actors(&mut self, dt: f32, player_feet: Vec3) {
        let ids: Vec<String> = self.actors.keys().cloned().collect();
        let visitors = self.sim.climate_visitors();
        for id in ids {
            let Some(actor) = self.actors.get_mut(&id) else { continue };
            match self.presence.crowd.agent(&id) {
                Some(a) => {
                    let shown = id != "xivvy" || self.sim.lvl > 0;
                    actor.visible = shown;
                    if shown {
                        let owns = if id == "xivvy" { actor.update_vent(is_vent_room(a.walker.room()), a.walker.moving, dt) } else { false };
                        let driver = self.drivers.entry(id.clone()).or_default();
                        let was = self.vent_owned.get(&id).copied().unwrap_or(false);
                        if owns {
                            actor.pos = a.walker.pos;
                            actor.yaw = a.walker.yaw;
                            actor.hidden = a.walker.hidden;
                        } else {
                            if was {
                                driver.reset();
                            }
                            driver.drive(a, actor);
                        }
                        self.vent_owned.insert(id.clone(), owns);
                    }
                }
                None => match visitors.iter().find(|(v, _)| *v == id) {
                    Some((_, t)) => {
                        actor.visible = true;
                        actor.pos = visitor_pos(&id, *t);
                        actor.yaw = yaw_toward(actor.pos, player_feet);
                        actor.set_locomotion(false, false);
                    }
                    None => actor.visible = false,
                },
            }
            actor.animate(dt);
        }
    }

    // --- admin (the ids ModeAdmin lists; see admin.rs) ------------------------------------------------------------------

    fn run_admin(&mut self, id: &str) {
        let parts: Vec<&str> = id.split(':').collect();
        match parts.as_slice() {
            ["lure", room] => self.sim.force_lure(room),
            ["lure_reset"] => self.sim.lure_cool = 0.0,
            ["place", room] => {
                self.sim.place_killer(room);
                self.teleport_pending = true;
            }
            ["sys", name, "break"] => self.sim.set_system(name, true),
            ["sys", name, "fix"] => self.sim.set_system(name, false),
            ["seal", side, on] => {
                let s = if *side == "L" { 'L' } else { 'R' };
                self.sim.seals[if s == 'L' { 0 } else { 1 }] = *on == "on";
            }
            ["temp", n] => {
                if let Ok(v) = n.parse::<f64>() {
                    self.sim.temp = v.clamp(20.0, 130.0);
                }
            }
            ["app", name, on] => self.sim.set_appliance(name, *on == "on"),
            ["clear_climate"] => self.sim.clear_climate(),
            ["freeze"] => self.admin.frozen = !self.admin.frozen,
            ["god"] => self.sim.god = !self.sim.god,
            ["inf_air"] => self.sim.infinite_stress = !self.sim.infinite_stress,
            ["no_scare"] => self.admin.no_scare = !self.admin.no_scare,
            ["reveal"] => self.admin.reveal = !self.admin.reveal,
            ["noclip"] => self.admin.noclip = !self.admin.noclip,
            ["speed", x] => {
                if let Ok(v) = x.parse::<f64>() {
                    self.sim.hunter_speed = v.clamp(0.1, 8.0);
                }
            }
            ["air", n] => {
                if let Ok(v) = n.parse::<f64>() {
                    self.sim.stress = (100.0 - v).clamp(0.0, 100.0);
                }
            }
            ["hour", h] => {
                if let Ok(v) = h.parse::<f64>() {
                    self.sim.elapsed = (v * HOUR_MS).clamp(0.0, NIGHT_MS - 1000.0);
                    self.hour_seen = hour_index(self.sim.elapsed);
                }
            }
            ["level", who, n] => {
                if let Ok(v) = n.parse::<i32>() {
                    self.sim.set_level(who, v);
                }
            }
            ["phantom", who, room] => {
                if ROOMS.contains(room) && super::data::PHANTOM_IDS.contains(who) {
                    self.sim.phantom = Some(super::sim::Phantom { id: who.to_string(), room: room.to_string(), t: 0.0 });
                }
            }
            _ => {}
        }
    }

    fn static_over_feed(&mut self, ctx: &ShiftCtx) {
        let pulse = self.presence.crowd.stare_pulse(&self.monitor_room);
        if pulse <= 0.0 || ctx.save.reduced_motion() || !self.sim.online("cams") {
            return;
        }
        let pic = crate::hud::monitor::picture_rect();
        fill_rect(pic, Color::new(0.8, 0.85, 0.82, pulse * 0.18));
        for _ in 0..8 {
            let y = pic.y + self.fx.next() as f32 * pic.h;
            let h = 0.5 + self.fx.next() as f32 * 1.5;
            fill_rect(Rect::new(pic.x, y, pic.w, h), Color::new(0.9, 0.93, 0.9, pulse * 0.7));
        }
    }

    /// The side panel of the monitor: LURE, one row per system with REBOOT, the temperature line, SYSTEM ERROR.
    fn panel(&mut self, ctx: &mut ShiftCtx) {
        let live = !ctx.paused && self.sim.over == Over::None;
        let t = tuning();
        let mut ui = Ui::begin(ctx.input);
        let x = 332.0;
        let rows: Vec<&super::data::SystemInfo> = t
            .systems
            .iter()
            .filter(|s| !s.climate || (if s.id == "fan" { self.sim.has_fan } else { self.sim.has_climate }))
            .collect();
        let height = 14.0 + rows.len() as f32 * 12.0 + if self.sim.has_climate { 10.0 } else { 0.0 };
        fill_rect(Rect::new(x - 4.0, 31.0, 128.0, height + 4.0), Color::new(0.0, 0.0, 0.0, 0.55));
        // LURE
        let room = self.monitor_room.clone();
        let can_lure = self.sim.online("audio") && self.sim.lure_cool <= 0.0 && !room.starts_with("vent");
        let label = if self.sim.lure_cool > 0.0 { format!("LURE READY IN {}s", (self.sim.lure_cool / 1000.0).ceil() as i32) } else { "LURE HERE  L".to_string() };
        let lure = ui.button(Rect::new(x, 34.0, 120.0, 11.0), &label, 6.0, ButtonStyle::Normal, can_lure);
        if lure.clicked && live {
            self.do_lure(ctx);
        } else if lure.denied && live {
            Self::deny(ctx);
        }
        let mut reboot: Option<String> = None;
        for (i, s) in rows.iter().enumerate() {
            let y = 47.0 + i as f32 * 12.0;
            ui_text::text(&s.label, x, y + 2.0, 6.0, Color::new(0.9, 0.88, 0.8, 1.0));
            let (txt, col) = match self.sim.state_of(&s.id) {
                SysState::Ok => ("ONLINE".to_string(), Color::new(0.6, 1.0, 0.7, 1.0)),
                SysState::Broken => ("BROKEN".to_string(), Color::new(1.0, 0.4, 0.35, 1.0)),
                SysState::Reboot => {
                    let left = ((t.t.reboot_ms - self.sim.reboot_t(&s.id)) / 1000.0).ceil().max(0.0) as i32;
                    (format!("REBOOT {left}s"), Color::new(1.0, 0.85, 0.4, 1.0))
                }
            };
            ui_text::text(&txt, x + 38.0, y + 2.0, 6.0, col);
            let broken = self.sim.state_of(&s.id) == SysState::Broken;
            let r = ui.button(Rect::new(x + 84.0, y, 36.0, 10.0), &format!("REBOOT {}", s.key), 5.0, ButtonStyle::Normal, broken);
            if r.clicked && live {
                reboot = Some(s.id.clone());
            } else if r.denied && live {
                Self::deny(ctx);
            }
        }
        if let Some(sys) = reboot {
            self.do_reboot(ctx, &sys);
        }
        if self.sim.has_climate {
            let y = 47.0 + rows.len() as f32 * 12.0 + 1.0;
            let line = format!("TEMP {} F   COLD {}%  HEAT {}%", self.sim.temp.round() as i32, self.sim.cold as i32, self.sim.heat as i32);
            ui_text::text(&line, x, y, 6.0, Color::new(0.88, 0.69, 0.29, 1.0));
        }
        if !self.sim.online("cams") {
            ui_text::text_center("SYSTEM ERROR", 240.0, 126.0, 14.0, Color::new(1.0, 0.35, 0.3, 1.0));
        }
    }
}

impl Mode for LureMode {
    fn id(&self) -> &'static str {
        "lure"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        if self.started {
            return;
        }
        self.started = true;
        ctx.player.teleport(SPAWN, 0.0, -0.1);
        ctx.player.footprint = Some((-2.5, -2.0, 2.5, 2.0));
        ctx.player.frozen = false;
        ctx.player.look_frozen = false;
        let kit = PropKit::new();
        let gfx: &Gfx = ctx.gfx;
        self.booth = Some(Booth::new(&gfx.tex, &kit));
        self.hall = Some(Hall::new(&gfx.tex, &kit));
        for id in &tuning().cast {
            if let Some(a) = ctx.hunters.actor(id) {
                self.actors.insert(id.clone(), a);
            }
        }
        self.presence = Presence::new(self.seed, ctx.manual_clock);
        ctx.hud.call = if self.is_custom { CallScript::empty() } else { CallScript::for_level("lure", self.night) };
        ctx.hud.cam_map = Some(map().layout());
        self.sync_visuals(true);
        self.sync_hud(ctx.hud, ctx.bindings);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        if self.sim.over != Over::None {
            return;
        }
        let input = ctx.input;
        self.adopt_requested_monitor(ctx);
        ctx.player.frozen = self.monitor_up;
        ctx.player.look_frozen = self.monitor_up;
        ctx.player.noclip = self.admin.noclip;
        let solids = self.booth.as_ref().map(|b| b.all_solids()).unwrap_or_default();
        ctx.player.update(input, ctx.bindings, &solids, ctx.save.mouse_sens);
        self.aim = None;
        ctx.hud.prompt.clear();
        if !self.monitor_up {
            if let Some(b) = &self.booth {
                self.aim = ctx.player.aim(&b.stations, &solids);
                if let Some(i) = self.aim {
                    ctx.hud.prompt = b.stations[i].prompt.clone();
                }
            }
        }
        // Zellia
        if !self.monitor_up && input.just(MUTE) && ctx.hud.call.mute() {
            ctx.hud.zellia_muted = true;
        }
        if self.monitor_up {
            if input.just(MONITOR) {
                self.close_monitor(ctx);
                return;
            }
            if input.just(CAM_PREV) {
                self.cycle_camera(ctx, -1);
            }
            if input.just(CAM_NEXT) {
                self.cycle_camera(ctx, 1);
            }
            for n in 1..=8 {
                if input.just(camera_action(n)) {
                    self.select_number(ctx, n);
                }
            }
            if input.just("nw_lure") {
                self.do_lure(ctx);
            }
            for (action, system) in [("nw_reboot_audio", "audio"), ("nw_reboot_cams", "cams"), ("nw_reboot_vent", "vent"), ("nw_reboot_heater", "heater"), ("nw_reboot_fan", "fan")] {
                if input.just(action) {
                    self.do_reboot(ctx, system);
                }
            }
            return;
        }
        if input.just(MONITOR) {
            self.open_monitor(ctx);
            return;
        }
        if input.just(DOOR_LEFT) {
            self.toggle_seal(ctx, 'L');
        }
        if input.just(DOOR_RIGHT) {
            self.toggle_seal(ctx, 'R');
        }
        if input.just(LIGHT_LEFT) {
            let m = self.toggle_appliance(ctx, "heater");
            Self::note(ctx, &m);
        }
        if input.just(LIGHT_RIGHT) {
            let m = self.toggle_appliance(ctx, "fan");
            Self::note(ctx, &m);
        }
        if input.just(LAMP) {
            self.toggle_lamp();
        }
        if input.just(INTERACT) {
            if let (Some(i), Some(b)) = (self.aim, &self.booth) {
                let id = b.stations[i].id.clone();
                self.use_station(ctx, &id);
            }
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.sim.over != Over::None || self.admin.frozen {
            return;
        }
        let watch = self.watching();
        let events = self.sim.step(dt_ms, &watch);
        for ev in ctx.hud.call.update(self.sim.elapsed, dt_ms) {
            match ev {
                CallEvent::Ring => ctx.snd.cues(&cue_names::call_ring()),
                CallEvent::Talk => ctx.snd.cues(&cue_names::call_line()),
                _ => {}
            }
        }
        let hour = hour_index(self.sim.elapsed);
        if hour != self.hour_seen {
            self.hour_seen = hour;
            ctx.snd.cues(&cue_names::hour_change());
        }
        for e in &events {
            self.on_event(ctx, e);
        }
        if self.teleport_pending {
            self.teleport_pending = false;
            self.presence.teleport_xivvy(&self.sim, ctx.player.eye());
        }
        for be in self.presence.tick(&self.sim, (dt_ms / 1000.0) as f32, ctx.player.eye()) {
            if let BehaviourEvent::FirstStare { subtitle, .. } = be {
                self.say(ctx, &subtitle);
            }
        }
        if self.sim.over != Over::None && self.monitor_up {
            self.close_monitor(ctx);
        }
        self.sync_visuals(ctx.manual_clock);
        self.sync_hud(ctx.hud, ctx.bindings);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        self.frame = self.frame.wrapping_add(1);
        self.finish_if_needed(ctx);
        ctx.hud.cam_flash_s = (ctx.hud.cam_flash_s - dt_s).max(0.0);
        if let Some((text, left)) = &mut self.msg {
            *left -= dt_s;
            if *left <= 0.0 {
                if ctx.hud.call_text == *text {
                    ctx.hud.call_text.clear();
                }
                self.msg = None;
            }
        }
        if ctx.paused {
            self.sync_hud(ctx.hud, ctx.bindings);
            return;
        }
        let shut = self.sim.seals;
        if let Some(b) = &mut self.booth {
            if ctx.manual_clock {
                b.snap_slabs(shut);
            } else {
                b.animate(shut, dt_s);
            }
        }
        self.sync_visuals(false);
        let feet = ctx.player.feet;
        self.drive_actors(dt_s, feet);
        self.sync_hud(ctx.hud, ctx.bindings);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
        ctx.player.cam.apply();
        let cam_pos = ctx.player.cam.pos;
        let s = &self.sim;
        let amb = booth_ambient(s.has_climate, s.cold, s.heat, s.temp, s.stress);
        let lights: LightSet = booth_lights(self.lamp_on, amb, ctx.player.flashlight_spot());
        if let Some(b) = &self.booth {
            b.draw(ctx.gfx, &lights, cam_pos);
        }
        // hunters in the hatches and the booth: Xivvy in her duct, the climate pair creeping in
        let nav = self.presence.crowd.nav();
        for (id, a) in &self.actors {
            if !a.visible {
                continue;
            }
            let near_booth = a.pos.x.abs() < 10.0 && a.pos.z.abs() < 4.0 && nav.room_at(a.pos).map(|r| r.starts_with("office")).unwrap_or(true);
            if near_booth {
                a.draw(ctx.gfx, &lights, cam_pos);
            }
            let _ = id;
        }
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let m = map();
        let sim_room = m.map_to_sim(room).unwrap_or(&self.monitor_room).to_string();
        let (pos, target) = m.camera_transform(&sim_room);
        ctx.gfx.feed.begin(pos, target, Vec3::Y);
        if self.sim.online("cams") {
            if let Some(h) = &self.hall {
                let lights = feed_lights(&sim_room);
                h.draw(ctx.gfx, &lights, pos);
                for agent in self.presence.crowd.visible_in(&sim_room) {
                    if let Some(a) = self.actors.get(&agent.id) {
                        if a.visible {
                            a.draw_in(ctx.gfx, &lights, pos, Actor::is_duct_room(&sim_room));
                        }
                    }
                }
            }
        }
        ctx.gfx.feed.end();
    }

    fn draw_overlay(&mut self, ctx: &mut ShiftCtx) {
        if !self.monitor_up || self.sim.over != Over::None {
            return;
        }
        self.static_over_feed(ctx);
        self.panel(ctx);
    }

    fn outcome(&self) -> Outcome {
        match self.sim.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn end_card(&self, ctx: &ShiftCtx) -> Option<EndCard> {
        match self.sim.over {
            Over::None => None,
            Over::Win => {
                let more = ctx.session.has_next(ctx.modes);
                let mut body = format!("6:00 AM\nThe air held until morning. Air left: {}%.", self.sim.air_left());
                if self.is_custom {
                    body += &format!("\nScore {}  {}  (best {})", self.score, rating_of(self.score, 400), ctx.save.best_score("lure"));
                } else if more {
                    body += &format!("\nNight {} is open.", self.night + 1);
                } else {
                    body += "\nYou cleared every night.";
                }
                let title = if self.is_custom { "CUSTOM NIGHT COMPLETE".to_string() } else { format!("NIGHT {} COMPLETE", self.night) };
                Some(EndCard { title, body, can_next: more })
            }
            Over::Caught => {
                let h = hour_label(hour_index(self.sim.elapsed));
                let name = tuning().name(&self.sim.by);
                let line = match self.sim.by.as_str() {
                    "mavrine" => format!("The air gave out at {h}."),
                    "wynelle" => format!("The cold took the office at {h}."),
                    "solyth" => format!("The heat took the office at {h}."),
                    _ => format!("{} got in through the vent at {h}.", if name.is_empty() { "Xivvy".to_string() } else { name }),
                };
                Some(EndCard { title: "CAUGHT".to_string(), body: line, can_next: false })
            }
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.sim.over != Over::Caught {
            return None;
        }
        let cause = if self.sim.by == "wynelle" { "cold" } else { "" };
        Some((self.sim.by.clone(), cause.to_string()))
    }

    fn wants_mouse_free(&self) -> bool {
        self.monitor_up
    }

    fn admin_do(&mut self, id: &str) {
        self.run_admin(id);
    }

    fn debug_win(&mut self) {
        if self.sim.over == Over::None {
            self.sim.over = Over::Win;
        }
    }

    fn debug_lose(&mut self) {
        if self.sim.over == Over::None {
            self.sim.over = Over::Caught;
            self.sim.by = "xivvy".to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monitor_request_resolves_sim_and_map_ids_and_rejects_others() {
        assert_eq!(LureMode::requested_sim_room("ventr").as_deref(), Some("ventr"));
        assert_eq!(LureMode::requested_sim_room("storage").as_deref(), Some("storage"));
        assert!(LureMode::requested_sim_room("nowhere").is_none());
        assert!(LureMode::requested_sim_room("").is_none());
    }

    fn session(night: i32) -> Session {
        let mut s = Session::new();
        s.start_mode("lure", night, BTreeMap::new(), Some(4242));
        s
    }

    #[test]
    fn a_story_night_starts_at_the_nights_tables() {
        let m = new_mode(&session(3));
        assert_eq!((m.night, m.is_custom), (3, false));
        assert_eq!(m.sim.n, 2);
        assert_eq!(m.sim.lvl, 5);
        assert_eq!(m.outcome(), Outcome::Running);
        assert_eq!(m.id(), "lure");
        assert!(!m.wants_mouse_free());
    }

    #[test]
    fn a_custom_night_plays_at_night_five_with_the_sliders() {
        let mut s = Session::new();
        let mut c = BTreeMap::new();
        c.insert("xivvy".to_string(), 7);
        c.insert("ferryn".to_string(), 3);
        s.start_mode("lure", 1, c, Some(9));
        let m = new_mode(&s);
        assert!(m.is_custom && m.night == 5);
        assert_eq!(m.sim.lvl, 7);
        assert_eq!(m.score, 100);
    }

    #[test]
    fn a_seed_of_zero_still_gets_a_seed() {
        let mut s = Session::new();
        s.start_mode("lure", 2, BTreeMap::new(), Some(0));
        s.seed_value = 0;
        let m = new_mode(&s);
        assert!(m.seed <= 0x7fff_ffff);
    }

    #[test]
    fn the_end_card_strings_follow_the_godot_text() {
        // end_card needs a ShiftCtx, which needs a GL context: check the pieces it builds from
        assert_eq!(capitalize_word("CAMERAS"), "Cameras");
        assert_eq!(capitalize_word("vents"), "Vents");
        assert_eq!(capitalize_word(""), "");
        assert_eq!(rating_of(400, 400), "Unbeatable!");
        assert_eq!(hour_label(hour_index(5.0 * HOUR_MS)), "5 AM");
    }

    #[test]
    fn debug_win_and_lose_decide_the_shift() {
        let mut m = new_mode(&session(2));
        m.debug_win();
        assert_eq!(m.outcome(), Outcome::Won);
        m.debug_lose();
        assert_eq!(m.outcome(), Outcome::Won, "a decided shift stays decided");
        let mut l = new_mode(&session(2));
        l.debug_lose();
        assert_eq!(l.outcome(), Outcome::Lost);
        assert_eq!(l.killer(), Some(("xivvy".to_string(), String::new())));
    }

    #[test]
    fn the_cold_kills_through_the_cold_line() {
        let mut m = new_mode(&session(4));
        m.sim.over = Over::Caught;
        m.sim.by = "wynelle".to_string();
        assert_eq!(m.killer(), Some(("wynelle".to_string(), "cold".to_string())));
    }

    #[test]
    fn admin_rows_change_the_game() {
        let mut m = new_mode(&session(4));
        m.admin_do("seal:L:on");
        assert!(m.sim.seals[0] && !m.sim.seals[1]);
        m.admin_do("seal:L:off");
        assert!(!m.sim.seals[0]);
        m.admin_do("sys:cams:break");
        assert!(!m.sim.online("cams"));
        m.admin_do("sys:cams:fix");
        assert!(m.sim.online("cams"));
        m.admin_do("temp:40");
        assert_eq!(m.sim.temp, 40.0);
        m.admin_do("temp:500");
        assert_eq!(m.sim.temp, 130.0);
        m.admin_do("app:heater:on");
        assert!(m.sim.heater_on);
        m.admin_do("app:fan:on");
        assert!(m.sim.fan_on && !m.sim.heater_on);
        m.admin_do("air:30");
        assert_eq!(m.sim.stress, 70.0);
        m.admin_do("hour:3");
        assert_eq!(m.sim.elapsed, 3.0 * HOUR_MS);
        m.admin_do("hour:99");
        assert_eq!(m.sim.elapsed, NIGHT_MS - 1000.0);
        m.admin_do("lure:prize");
        assert_eq!(m.sim.killer.target, "prize");
        assert!(m.sim.killer.lure_left > 0.0);
        m.admin_do("lure:ventl");
        assert_eq!(m.sim.killer.target, "prize", "a crawlspace is no lure target");
        m.admin_do("place:officeL");
        assert_eq!(m.sim.killer.room, "officeL");
        assert!(m.teleport_pending);
        m.admin_do("god");
        assert!(m.sim.god);
        m.admin_do("freeze");
        assert!(m.admin.frozen);
        m.admin_do("speed:100");
        assert_eq!(m.sim.hunter_speed, 8.0);
        m.admin_do("phantom:ferryn:lobby");
        assert_eq!(m.sim.phantom.as_ref().map(|p| p.room.clone()), Some("lobby".to_string()));
        m.admin_do("level:xivvy:12");
        assert_eq!(m.sim.lvl, 12);
        m.admin_do("lure_reset");
        assert_eq!(m.sim.lure_cool, 0.0);
        m.admin_do("clear_climate");
        assert_eq!(m.sim.temp, 70.0);
        m.admin_do("nonsense");
    }
}
