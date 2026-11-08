//! Classic as a `Mode` (port of `night_watch_game.gd`): the walkable security room, the building the cameras look into, the
//! hunters and the HUD data, driven by `ClassicSim`.
//!
//! * The player walks the office and USES stations with the interact key (the monitor, the DOOR and LIGHT buttons, the
//!   lamp, the chair) or the quick keys (Z / X doors, hold G / H door lights, Y lamp, Space monitor, R mutes Zellia).
//! * The monitor shows one of eight live cameras: the building is drawn from the Godot camera mounts into the feed target,
//!   with the hunters standing, working and WALKING through it (`Placement`: A* on the nav graph, idle stare / task).
//! * The sim never reads a hunter's position: where she walks, stares or works changes no timer.
//!
//! The pure state (everything but the GPU objects) is built by `ClassicMode::new`, so most of this file is tested without a
//! window; `start` builds the office, the building, the props and the hunters' actors.
#![allow(dead_code)]

use super::placement::Placement;
use super::sim::{ClassicSim, MeetResult, Over, SimEvent, SimInput};
use super::stalker::door_name;
use super::view::{self, Pose, PoseIn, View};
use super::world::Props;
use crate::audio::cue_names;
use crate::core::{
    blackout_ms, building, call_levels, cam_rooms, camera_action, cast, hour_label, AdminEntry, CallEvent, CallScript, Cue, EndCard, Mode,
    Outcome, Session, ShiftCtx, CAM_NEXT, CAM_PREV, DOOR_LEFT, DOOR_RIGHT, INTERACT, LAMP, LIGHT_LEFT, LIGHT_RIGHT, MEET, MONITOR, MUTE,
};
use crate::gfx::building::{classic_layout, Building};
use crate::gfx::office::{Office, OfficeConfig, SPAWN};
use crate::gfx::LightSet;
use crate::hunters::{yaw_toward, Actor, ActorDriver, BehaviourEvent, PoseState};
use macroquad::prelude::{vec3, Vec3};
use std::collections::BTreeMap;

/// Who is driving an actor's clips right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Owner {
    Fresh,
    /// `set_pose_state` (the sim's pose).
    Sim,
    /// The `ActorDriver` (walk, sprint, idle behaviour).
    Drive,
}

struct Slot {
    id: String,
    actor: Actor,
    driver: ActorDriver,
    owner: Owner,
}

/// The GPU side of the shift (built in `start`).
struct Gx {
    office: Office,
    building: Building,
    props: Props,
    slots: Vec<Slot>,
}

/// Something the scene must do for an event or an admin row (kept as data so the rules stay testable without a window).
#[derive(Clone, Debug)]
enum Fx {
    Cues(Vec<Cue>),
    CallText(String),
    Flash,
    Note(String),
    OpenMonitor(String),
    MeetEyes,
    PlayCall,
    SkipCall,
}

pub struct ClassicMode {
    view: View,
    night: i32,
    is_custom: bool,
    seed: u32,
    pub(super) sim: ClassicSim,
    pub(super) placement: Placement,
    /// Zellia's clock (the shift clock the call script reads).
    elapsed_ms: f64,
    pub(super) door_shut: [bool; 2],
    pub(super) light_held: [bool; 2],
    pub(super) monitor_up: bool,
    pub(super) monitor_room: String,
    lamp_on: bool,
    hour_seen: i32,
    call_shown: bool,
    flinch_left: BTreeMap<String, f32>,
    retreat_left: BTreeMap<String, f32>,
    attacker: String,
    finished: bool,
    pending_subtitle: Option<String>,
    mode_text: String,
    gx: Option<Gx>,
    /// The panel buttons as last drawn: doorL_door, doorR_door, doorL_light, doorR_light.
    btn: [Option<bool>; 4],
    dirty: bool,
    // admin switches
    admin_frozen: bool,
    admin_no_scare: bool,
    admin_reveal: bool,
    admin_noclip: bool,
    admin_light: [bool; 2],
    pending_admin: Vec<String>,
}

impl ClassicMode {
    /// The pure state of a shift of this session.
    pub fn new(session: &Session) -> ClassicMode {
        let seed = session.seed_or_fresh();
        let sim = ClassicSim::new(session.night, seed, &session.custom_levels, session.is_custom);
        ClassicMode {
            view: View::embedded(),
            night: session.night,
            is_custom: session.is_custom,
            seed,
            sim,
            placement: Placement::new(seed),
            elapsed_ms: 0.0,
            door_shut: [false, false],
            light_held: [false, false],
            monitor_up: false,
            monitor_room: "stage".to_string(),
            lamp_on: true,
            hour_seen: 0,
            call_shown: false,
            flinch_left: BTreeMap::new(),
            retreat_left: BTreeMap::new(),
            attacker: String::new(),
            finished: false,
            pending_subtitle: None,
            mode_text: String::new(),
            gx: None,
            btn: [None; 4],
            dirty: false,
            admin_frozen: false,
            admin_no_scare: false,
            admin_reveal: false,
            admin_noclip: false,
            admin_light: [false, false],
            pending_admin: Vec::new(),
        }
    }

    pub fn sim(&self) -> &ClassicSim {
        &self.sim
    }

    fn input(&self) -> SimInput {
        SimInput { door_shut: self.door_shut, light_held: self.light_held, cam: self.monitor_up, cam_room: self.monitor_room.clone() }
    }

    fn playing(&self) -> bool {
        self.sim.over == Over::Running
    }

    fn pan_of(room: &str) -> f32 {
        building().room(room).map(|r| r.pan as f32).unwrap_or(0.0)
    }

    fn cam_label(room: &str) -> String {
        match building().room(room) {
            Some(r) if r.cam > 0 => format!("CAM {}  {}", r.cam, r.label),
            _ => String::new(),
        }
    }

    fn hint_actions(&self) -> Vec<String> {
        let mut list = self.view.hint_actions.clone();
        if self.sim.kathy_night {
            list.push(MEET.to_string());
        }
        if crate::hud::admin::enabled() {
            list.push("admin".to_string());
        }
        list
    }

    // ---- the rules around the sim (pure) ------------------------------------------------------------------------------

    /// Toggle a door (index 0 left, 1 right). False when nothing changed (decided night, or the power is out).
    fn toggle_door(&mut self, i: usize) -> bool {
        if !self.playing() || self.sim.out {
            return false;
        }
        self.door_shut[i] = !self.door_shut[i];
        true
    }

    fn set_door(&mut self, i: usize, shut: bool) {
        if self.playing() && !self.sim.out {
            self.door_shut[i] = shut;
        }
    }

    fn set_light(&mut self, i: usize, held: bool) {
        if self.playing() {
            self.light_held[i] = held && !self.sim.out;
        }
    }

    /// What an event does to the scene: timers, sounds, notes, static.
    fn on_event(&mut self, e: &SimEvent) -> Vec<Fx> {
        let mut fx = Vec::new();
        match e {
            SimEvent::Flinch { id, room } => {
                self.flinch_left.insert(id.clone(), self.view.flinch_s);
                fx.push(Fx::Cues(cue_names::hunter_event("flinch", ClassicMode::pan_of(room))));
            }
            SimEvent::Out => {
                self.door_shut = [false, false];
                self.light_held = [false, false];
                fx.push(Fx::CallText(self.view.power_out_call.clone()));
                fx.push(Fx::Cues(cue_names::power_out()));
            }
            SimEvent::Hum { door, muffled, .. } => {
                let pan = if door == "doorL" { -self.view.hum_pan } else { self.view.hum_pan };
                let vol = if *muffled { self.view.hum_muffled_volume } else { self.view.hum_volume };
                fx.push(Fx::Cues(cue_names::office("breath", pan, vol)));
            }
            SimEvent::Moved { id, room, from } | SimEvent::Arrived { id, room, from } | SimEvent::Repelled { id, room, from } | SimEvent::Pushed { id, room, from } => {
                let kind = e.kind();
                if kind == "repelled" || kind == "pushed" {
                    self.retreat_left.insert(id.clone(), self.view.retreat_s);
                }
                if !(id == "kathy" && kind == "repelled") {
                    // her fading away makes no sound; a hunter on the move is only heard in the halls
                    if kind != "moved" || room == "westhall" || room == "easthall" {
                        fx.push(Fx::Cues(cue_names::hunter_event(kind, ClassicMode::pan_of(room))));
                    }
                }
                if self.monitor_up && (*room == self.monitor_room || *from == self.monitor_room) {
                    fx.push(Fx::Flash);
                }
            }
            SimEvent::Attack { .. } | SimEvent::Caught { .. } | SimEvent::Win => {}
        }
        fx
    }

    /// Run the pure side of an admin row; the rest comes back as effects.
    fn admin_apply(&mut self, id: &str) -> Vec<Fx> {
        let mut fx = Vec::new();
        match id {
            "door_l_shut" => self.set_door(0, true),
            "door_l_open" => self.set_door(0, false),
            "door_r_shut" => self.set_door(1, true),
            "door_r_open" => self.set_door(1, false),
            "light_l_on" => {
                self.admin_light[0] = true;
                self.set_light(0, true);
            }
            "light_l_off" => {
                self.admin_light[0] = false;
                self.set_light(0, false);
            }
            "light_r_on" => {
                self.admin_light[1] = true;
                self.set_light(1, true);
            }
            "light_r_off" => {
                self.admin_light[1] = false;
                self.set_light(1, false);
            }
            "all_to_doors" => {
                self.sim.admin_all_to_doors();
                self.dirty = true;
            }
            "kathy_meet" => {
                if self.sim.kathy_night {
                    self.sim.admin_send("kathy", "westhall");
                    self.dirty = true;
                    fx.push(Fx::OpenMonitor("westhall".to_string()));
                    fx.push(Fx::MeetEyes);
                }
            }
            "freeze" => self.admin_frozen = !self.admin_frozen,
            "inf_power" => self.sim.infinite_power = !self.sim.infinite_power,
            "god" => self.sim.god = !self.sim.god,
            "no_scare" => self.admin_no_scare = !self.admin_no_scare,
            "noclip" => self.admin_noclip = !self.admin_noclip,
            "reveal" => self.admin_reveal = !self.admin_reveal,
            "play_call" => fx.push(Fx::PlayCall),
            "skip_call" => fx.push(Fx::SkipCall),
            other => {
                if let Some(v) = other.strip_prefix("speed_") {
                    if let Ok(m) = v.parse::<f64>() {
                        self.sim.hunter_speed = m.clamp(0.1, 8.0);
                    }
                } else if let Some(v) = other.strip_prefix("power_") {
                    if let Ok(p) = v.parse::<f64>() {
                        for e in self.sim.admin_set_power(p) {
                            fx.extend(self.on_event(&e));
                        }
                    }
                } else if let Some(v) = other.strip_prefix("hour_") {
                    if let Ok(h) = v.parse::<i32>() {
                        self.sim.admin_set_hour(h);
                        self.hour_seen = self.sim.hour();
                        self.elapsed_ms = self.sim.elapsed;
                    }
                } else if let Some(v) = other.strip_prefix("ai_up:") {
                    let level = self.sim.hunter(v).map(|h| h.base).unwrap_or(0);
                    self.sim.admin_set_level(v, (level + 1).min(crate::core::MAX_AI));
                    self.dirty = true;
                } else if let Some(v) = other.strip_prefix("ai_down:") {
                    let level = self.sim.hunter(v).map(|h| h.base).unwrap_or(0);
                    self.sim.admin_set_level(v, (level - 1).max(0));
                    self.dirty = true;
                } else if let Some(v) = other.strip_prefix("all_to:") {
                    let ids: Vec<String> = self.sim.hunters.iter().map(|h| h.id.clone()).collect();
                    for hid in ids {
                        self.sim.admin_send(&hid, v);
                    }
                    self.dirty = true;
                }
            }
        }
        fx
    }

    // ---- effects needing the context ---------------------------------------------------------------------------------

    fn apply(&mut self, ctx: &mut ShiftCtx, fx: Vec<Fx>) {
        for f in fx {
            match f {
                Fx::Cues(c) => ctx.snd.cues(&c),
                Fx::CallText(t) => ctx.hud.call_text = t,
                Fx::Flash => self.flash_static(ctx),
                Fx::Note(t) => ctx.hud.set_note(&t, self.view.note_s),
                Fx::OpenMonitor(room) => self.open_monitor(ctx, Some(&room)),
                Fx::MeetEyes => self.meet_her_eyes(ctx),
                Fx::PlayCall => {
                    let lines = call_levels("classic")
                        .into_iter()
                        .find(|c| c.level == self.night)
                        .map(|c| c.lines)
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| vec!["Zellia, on the headset.".to_string()]);
                    ctx.hud.call = CallScript::from_lines(lines, 0.0);
                    ctx.hud.zellia_muted = false;
                    self.call_shown = false;
                    self.elapsed_ms = 0.0;
                }
                Fx::SkipCall => {
                    ctx.hud.call.mute();
                    ctx.hud.call_text.clear();
                    self.call_shown = false;
                }
            }
        }
    }

    fn flash_static(&self, ctx: &mut ShiftCtx) {
        ctx.hud.cam_flash_s = self.view.static_s;
    }

    fn refresh_hint(&self, ctx: &mut ShiftCtx) {
        let acts = self.hint_actions();
        let refs: Vec<&str> = acts.iter().map(|s| s.as_str()).collect();
        ctx.hud.hint = ctx.bindings.hint_line(self.monitor_up, &refs);
    }

    fn set_cam(&self, ctx: &mut ShiftCtx) {
        ctx.hud.monitor_room = self.monitor_room.clone();
        ctx.hud.cam_label = ClassicMode::cam_label(&self.monitor_room);
    }

    fn open_monitor(&mut self, ctx: &mut ShiftCtx, room: Option<&str>) {
        if !self.playing() || self.sim.out {
            return;
        }
        if let Some(r) = room {
            if building().room(r).map_or(false, |x| x.cam > 0) {
                self.monitor_room = r.to_string();
            }
        }
        self.monitor_up = true;
        ctx.hud.monitor_up = true;
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        self.set_cam(ctx);
        self.refresh_hint(ctx);
        self.flash_static(ctx);
        ctx.snd.cues(&cue_names::monitor_toggle());
        if let Some(t) = self.pending_subtitle.take() {
            ctx.hud.set_note(&t, self.view.stare_note_s);
        }
    }

    fn close_monitor(&mut self, ctx: &mut ShiftCtx) {
        let was = self.monitor_up;
        self.monitor_up = false;
        ctx.hud.monitor_up = false;
        ctx.player.frozen = self.finished;
        ctx.player.look_frozen = self.finished;
        self.refresh_hint(ctx);
        if was {
            ctx.snd.cues(&cue_names::monitor_toggle());
        }
    }

    fn toggle_monitor(&mut self, ctx: &mut ShiftCtx) {
        if self.monitor_up {
            self.close_monitor(ctx);
        } else {
            self.open_monitor(ctx, None);
        }
    }

    fn select_camera(&mut self, ctx: &mut ShiftCtx, room: &str) {
        if !self.playing() || !building().room(room).map_or(false, |r| r.cam > 0) {
            return;
        }
        self.monitor_room = room.to_string();
        self.set_cam(ctx);
        self.flash_static(ctx);
        ctx.snd.cues(&cue_names::camera_switch());
    }

    fn cycle_camera(&mut self, ctx: &mut ShiftCtx, step: i32) {
        let rooms = cam_rooms();
        if rooms.is_empty() {
            return;
        }
        let i = rooms.iter().position(|r| *r == self.monitor_room).unwrap_or(0) as i32;
        let n = rooms.len() as i32;
        let next = rooms[(((i + step) % n + n) % n) as usize].clone();
        self.select_camera(ctx, &next);
    }

    fn toggle_lamp(&mut self) {
        self.lamp_on = !self.lamp_on;
        if let Some(g) = self.gx.as_mut() {
            g.office.set_lamp(self.lamp_on);
        }
    }

    /// Night 7: meet Kathy's eyes on the feed you are watching.
    fn meet_her_eyes(&mut self, ctx: &mut ShiftCtx) {
        if !self.playing() || !self.sim.kathy_night {
            return;
        }
        let (result, _events) = self.sim.meet(self.monitor_up, &self.monitor_room, false);
        match result {
            MeetResult::Met => {
                ctx.hud.set_note(&self.view.meet_met, self.view.note_s);
                self.flash_static(ctx);
                self.sync(ctx);
            }
            MeetResult::Miss => ctx.hud.set_note(&self.view.meet_miss, self.view.note_s),
            MeetResult::Locked => ctx.hud.set_note(&self.view.meet_locked, self.view.note_s),
            MeetResult::None => {}
        }
    }

    /// What pressing use on a station does.
    fn use_station(&mut self, ctx: &mut ShiftCtx, id: &str) {
        if !self.playing() {
            return;
        }
        match id {
            "monitor" => self.toggle_monitor(ctx),
            "doorL_door" | "doorR_door" => {
                let i = if id == "doorL_door" { 0 } else { 1 };
                if self.toggle_door(i) {
                    ctx.snd.cues(&cue_names::door(self.door_shut[i]));
                }
                ctx.snd.cues(&cue_names::panel_button());
            }
            "lamp" => self.toggle_lamp(),
            "chair" => ctx.player.teleport(self.view.seat, 0.0, self.view.pitch),
            "breaker" | "vent" => {
                let prompt = self.gx.as_ref().and_then(|g| g.office.station(id).map(|s| s.prompt.clone())).unwrap_or_default();
                if !prompt.is_empty() {
                    ctx.hud.set_note(&prompt, self.view.note_s);
                }
            }
            _ => {}
        }
    }

    // ---- syncing the scene to the sim -------------------------------------------------------------------------------------

    /// Status block, blips, office lights and buttons, and the hunters' placement.
    fn sync(&mut self, ctx: &mut ShiftCtx) {
        let input = self.input();
        let h = &mut *ctx.hud;
        h.hour = self.sim.hour();
        h.power = self.sim.power as f32;
        h.usage = input.usage();
        h.night_label = format!("1. Classic \u{b7} Night {}", self.night);
        h.mode_text = self.mode_text.clone();
        h.cam_blips = if self.admin_reveal { self.sim.hunters.iter().map(|x| x.room.clone()).collect() } else { Vec::new() };
        let feet = ctx.player.feet;
        self.sync_world(feet, ctx.manual_clock);
    }

    /// The part of `sync` that needs no HUD: the office and the walkers.
    fn sync_world(&mut self, player_pos: Vec3, snap: bool) {
        if let Some(g) = self.gx.as_mut() {
            // Night 7: her light dips (the only tell the light gives)
            for d in 0..2 {
                g.office.set_light(door_name(d), self.sim.light_on[d] && !self.sim.light_dip(d));
            }
            let want = [self.sim.door_shut[0], self.sim.door_shut[1], self.sim.light_on[0], self.sim.light_on[1]];
            let ids = ["doorL_door", "doorR_door", "doorL_light", "doorR_light"];
            for i in 0..4 {
                if self.btn[i] != Some(want[i]) {
                    g.office.set_button(ids[i], want[i]);
                    self.btn[i] = Some(want[i]);
                }
            }
            if snap {
                g.office.snap_slabs(self.sim.door_shut);
            }
        }
        let fl = &self.flinch_left;
        let rl = &self.retreat_left;
        let at = &self.attacker;
        self.placement.sync(&self.sim, player_pos, snap, &|id| fl.contains_key(id) || rl.contains_key(id) || at == id);
        self.dirty = false;
    }

    /// The Zellia clock: feed the script, play its sounds, show its text (the HUD's `_step_call`).
    fn update_call(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        for e in ctx.hud.call.update(self.elapsed_ms, dt_ms) {
            match e {
                CallEvent::Ring => ctx.snd.cues(&cue_names::call_ring()),
                CallEvent::Talk => ctx.snd.cues(&cue_names::call_line()),
                CallEvent::Open | CallEvent::Close => {}
            }
        }
        if ctx.hud.call.open {
            ctx.hud.call_text = ctx.hud.call.text.clone();
            self.call_shown = true;
        } else if self.call_shown {
            self.call_shown = false;
            ctx.hud.call_text.clear();
        }
    }

    /// The night is decided: close the monitor, freeze the player, play the sound.
    fn finish_if_decided(&mut self, ctx: &mut ShiftCtx) {
        if self.finished || self.sim.over == Over::Running {
            return;
        }
        self.finished = true;
        if self.monitor_up {
            self.close_monitor(ctx);
        }
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.hud.prompt.clear();
        match self.sim.over {
            Over::Win => ctx.snd.cues(&cue_names::win()),
            Over::Caught => {
                self.attacker = self.sim.by.clone();
                let short = ctx.save.short_death_scenes();
                ctx.snd.cues(&cue_names::caught(short));
            }
            Over::Running => {}
        }
        self.sync(ctx);
    }

    // ---- the actors -------------------------------------------------------------------------------------------------------

    fn update_actors(&mut self, ctx: &ShiftCtx, dt_s: f32) {
        let Some(g) = self.gx.as_mut() else { return };
        let lost = self.sim.over == Over::Caught;
        let feet = ctx.player.feet;
        for slot in &mut g.slots {
            if slot.id == "nyxie" {
                // only in the dark, creeping in from the left hall toward the desk over the blackout
                let show = self.sim.out && self.sim.nyxie_level > 0;
                if show {
                    let t = (self.sim.out_ms / blackout_ms(self.sim.nyxie_level)) as f32;
                    let from = Office::door_stand(&self.view.nyxie_door, 0, 0.0);
                    let pos = view::nyxie_at(&self.view, from, t);
                    if !slot.actor.visible {
                        slot.actor.stop_pose();
                        slot.actor.set_locomotion(false, false);
                    }
                    slot.actor.pos = pos;
                    slot.actor.yaw = yaw_toward(pos, feet);
                }
                slot.actor.visible = show;
                if show {
                    slot.actor.animate(dt_s);
                }
                continue;
            }
            let hunter = self.sim.hunter(&slot.id);
            let agent = self.placement.crowd.agent(&slot.id);
            let shown = hunter.map_or(false, |h| h.base > 0) && agent.is_some();
            slot.actor.visible = shown;
            let Some(agent) = agent else { continue };
            if !shown {
                continue;
            }
            slot.actor.pos = agent.walker.pos;
            slot.actor.yaw = agent.walker.yaw;
            let pose = view::pose_of(&PoseIn {
                hunter,
                shown,
                id: &slot.id,
                door_shut: self.sim.door_shut,
                monitor_up: self.monitor_up,
                monitor_room: &self.monitor_room,
                attacker: &self.attacker,
                lost,
                flinching: self.flinch_left.contains_key(&slot.id),
                retreating: self.retreat_left.contains_key(&slot.id),
                moving: agent.walker.moving,
            });
            let mut sim_state: Option<PoseState> = None;
            match pose {
                Pose::Sim(p) => sim_state = Some(p),
                Pose::Loco => {}
                Pose::Wait(p) => {
                    if agent.brain.view().pose.is_none() {
                        sim_state = Some(p);
                    }
                }
            }
            match sim_state {
                Some(p) => {
                    if slot.owner == Owner::Drive {
                        slot.actor.stop_pose();
                    }
                    slot.actor.set_pose_state(p);
                    slot.owner = Owner::Sim;
                }
                None => {
                    if slot.owner != Owner::Drive {
                        slot.driver.reset();
                        slot.owner = Owner::Drive;
                    }
                    slot.driver.drive(agent, &mut slot.actor);
                }
            }
            slot.actor.animate(dt_s);
        }
    }
}

fn near_office(pos: Vec3, view: &View) -> bool {
    pos.z > view.office_view_z
}

impl Mode for ClassicMode {
    fn id(&self) -> &'static str {
        "classic"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        let office = Office::build(OfficeConfig::default(), &ctx.gfx.tex);
        let bld = Building::build(&ctx.gfx.tex);
        let props = Props::build();
        let mut slots = Vec::new();
        for def in cast().iter() {
            if let Some(mut actor) = ctx.hunters.actor(&def.id) {
                actor.visible = false;
                slots.push(Slot { id: def.id.clone(), actor, driver: ActorDriver::new(), owner: Owner::Fresh });
            }
        }
        self.gx = Some(Gx { office, building: bld, props, slots });

        ctx.player.footprint = Some(self.view.footprint);
        ctx.player.teleport(SPAWN, 0.0, self.view.pitch);

        let entry = ctx.modes.get("classic");
        let (num, title, level_name) = match entry {
            Some(e) => (e.num, e.title.clone(), e.levels.get((self.night.max(1) - 1) as usize).cloned().unwrap_or_default()),
            None => (1, "CLASSIC".to_string(), String::new()),
        };
        self.mode_text = view::mode_text(num, &title, &level_name, self.night);

        let h = &mut *ctx.hud;
        h.cam_map = Some(classic_layout());
        h.monitor_available = true;
        h.monitor_up = false;
        h.show_crosshair = true;
        h.zellia_muted = false;
        h.call = if self.is_custom { CallScript::empty() } else { CallScript::for_level("classic", self.night) };
        self.monitor_room = "stage".to_string();
        self.set_cam(ctx);
        self.refresh_hint(ctx);
        self.hour_seen = 0;
        // the initial placement snaps (nobody walks on the first frame)
        let feet = ctx.player.feet;
        self.sync_world(feet, true);
        self.sync(ctx);
        if let Some(g) = self.gx.as_mut() {
            g.office.snap_slabs([false, false]);
        }
        self.update_actors(ctx, 0.0);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        let input = ctx.input;
        let Some(g) = self.gx.as_ref() else { return };
        // walking and looking
        ctx.player.noclip = self.admin_noclip;
        ctx.player.frozen = self.monitor_up;
        ctx.player.look_frozen = self.monitor_up;
        let solids = g.office.all_solids();
        ctx.player.update(input, ctx.bindings, &solids, ctx.save.mouse_sens);
        // what the player aims at
        let aimed: Option<String> = if self.monitor_up { None } else { ctx.player.aim(&g.office.stations, &solids).map(|i| g.office.stations[i].id.clone()) };
        ctx.hud.prompt = match &aimed {
            Some(id) => g.office.station(id).map(|s| s.prompt.clone()).unwrap_or_default(),
            None => String::new(),
        };
        ctx.hud.show_crosshair = !self.monitor_up;

        // the lights: held with the quick key, or with use held on a LIGHT button (or forced by the admin overlay)
        for d in 0..2 {
            let action = if d == 0 { LIGHT_LEFT } else { LIGHT_RIGHT };
            let on_button = aimed.as_deref() == Some(if d == 0 { "doorL_light" } else { "doorR_light" });
            let want = input.is_down(action) || (input.is_down(INTERACT) && on_button) || self.admin_light[d];
            if want != self.light_held[d] {
                self.set_light(d, want);
            }
        }

        if input.just(MONITOR) {
            self.toggle_monitor(ctx);
        } else if self.monitor_up {
            if input.just(INTERACT) {
                self.close_monitor(ctx);
            } else if input.just(CAM_PREV) {
                self.cycle_camera(ctx, -1);
            } else if input.just(CAM_NEXT) {
                self.cycle_camera(ctx, 1);
            } else {
                let mut picked = false;
                for n in 1..=8 {
                    if input.just(camera_action(n)) {
                        let room = crate::core::room_for_cam(n);
                        self.select_camera(ctx, &room);
                        picked = true;
                        break;
                    }
                }
                if !picked {
                    if input.just(MEET) {
                        self.meet_her_eyes(ctx);
                    } else if input.just(MUTE) {
                        self.mute_zellia(ctx);
                    }
                }
            }
        } else {
            if input.just(DOOR_LEFT) && self.toggle_door(0) {
                ctx.snd.cues(&cue_names::door(self.door_shut[0]));
            }
            if input.just(DOOR_RIGHT) && self.toggle_door(1) {
                ctx.snd.cues(&cue_names::door(self.door_shut[1]));
            }
            if input.just(LAMP) {
                self.toggle_lamp();
            }
            if input.just(MUTE) {
                self.mute_zellia(ctx);
            }
            if input.just(INTERACT) {
                if let Some(id) = aimed {
                    self.use_station(ctx, &id);
                }
            }
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if !self.playing() || self.admin_frozen {
            return;
        }
        self.elapsed_ms += dt_ms;
        self.update_call(ctx, dt_ms);
        let input = self.input();
        let events = self.sim.step(dt_ms, &input);
        if self.sim.hour() != self.hour_seen {
            self.hour_seen = self.sim.hour();
            ctx.snd.cues(&cue_names::hour_change());
        }
        if self.sim.out && self.monitor_up {
            self.close_monitor(ctx);
        }
        for e in &events {
            let fx = self.on_event(e);
            self.apply(ctx, fx);
        }
        self.sync(ctx);
        self.finish_if_decided(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        if ctx.paused {
            return;
        }
        // admin rows that need the context
        if !self.pending_admin.is_empty() {
            let ids = std::mem::take(&mut self.pending_admin);
            for id in ids {
                let fx = self.admin_apply(&id);
                self.apply(ctx, fx);
                self.sync(ctx);
            }
        }
        if self.dirty {
            self.sync(ctx);
        }
        // timers
        for t in [&mut self.flinch_left, &mut self.retreat_left] {
            for v in t.values_mut() {
                *v -= dt_s;
            }
            t.retain(|_, v| *v > 0.0);
        }
        ctx.hud.cam_flash_s = (ctx.hud.cam_flash_s - dt_s).max(0.0);
        // the walkers and their idle behaviour (real time; the manual clock keeps them where the sim put them)
        if !ctx.manual_clock {
            for ev in self.placement.tick(dt_s) {
                if let BehaviourEvent::FirstStare { subtitle, .. } = ev {
                    if self.monitor_up {
                        ctx.hud.set_note(&subtitle, self.view.stare_note_s);
                    } else {
                        self.pending_subtitle = Some(subtitle);
                    }
                }
            }
            if self.playing() {
                let shut = self.sim.door_shut;
                if let Some(g) = self.gx.as_mut() {
                    g.office.animate(shut, dt_s);
                }
            }
        }
        // a hunter staring into the feed you watch: a soft static pulse
        if self.monitor_up {
            let p = self.placement.crowd.stare_pulse(&self.monitor_room);
            if p > 0.0 {
                ctx.hud.cam_flash_s = ctx.hud.cam_flash_s.max(p * self.view.stare_static_scale);
            }
        }
        self.update_actors(ctx, dt_s);
        self.finish_if_decided(ctx);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        if self.monitor_up {
            return; // the camera screen covers the world
        }
        let Some(g) = self.gx.as_ref() else { return };
        let cam = ctx.player.cam;
        cam.apply();
        let flash = ctx.player.flashlight_spot();
        g.office.draw(&ctx.gfx, cam.pos, flash);
        let lights = g.office.lights(flash);
        for s in &g.slots {
            if s.actor.visible && near_office(s.actor.pos, &self.view) {
                s.actor.draw(&ctx.gfx, &lights, cam.pos);
            }
        }
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let Some(g) = self.gx.as_ref() else { return };
        let pos = Building::begin_feed(&ctx.gfx, room);
        let lights = LightSet::feed();
        g.building.draw_feed_world(&ctx.gfx, pos);
        g.props.draw(&ctx.gfx, &lights, pos);
        for a in self.placement.crowd.visible_in(room) {
            if let Some(s) = g.slots.iter().find(|s| s.id == a.id) {
                if s.actor.visible {
                    s.actor.draw(&ctx.gfx, &lights, pos);
                }
            }
        }
        ctx.gfx.feed.end();
    }

    fn outcome(&self) -> Outcome {
        match self.sim.over {
            Over::Running => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn end_card(&self, _ctx: &ShiftCtx) -> Option<EndCard> {
        match self.sim.over {
            Over::Running => None,
            Over::Win => Some(self.view.win_card(self.night, self.is_custom)),
            Over::Caught => Some(self.view.caught_card(&self.sim.by, self.night)),
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.sim.over != Over::Caught || self.admin_no_scare {
            return None;
        }
        let cause = if self.sim.by == "nyxie" { "darkness" } else { "" };
        Some((self.sim.by.clone(), cause.to_string()))
    }

    fn wants_mouse_free(&self) -> bool {
        self.monitor_up
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        let mut e = Vec::new();
        let kind = if self.sim.kathy_night { "KATHY" } else { "CLASSIC" };
        e.push(AdminEntry::heading(&format!("{kind} LEVEL {}  {}  SEED {}", self.night, hour_label(self.sim.hour()), self.seed)));
        e.push(AdminEntry::heading(&format!(
            "POWER {}%  OUT {}  FROZEN {}  GOD {}  NO SCARE {}  SPEED {:.1}X",
            self.sim.power.ceil() as i32,
            self.sim.out,
            self.admin_frozen,
            self.sim.god,
            self.admin_no_scare,
            self.sim.hunter_speed
        )));
        for h in &self.sim.hunters {
            let walking = self.placement.crowd.agent(&h.id).map_or(false, |a| a.walker.moving);
            let door = if h.is_at_door() { format!("  AT DOOR {:.1}s", h.at_door_ms / 1000.0) } else { String::new() };
            e.push(AdminEntry::heading(&format!("{}  {}  AI {}{}{}", h.id.to_uppercase(), h.room, h.level, door, if walking { "  WALKING" } else { "" })));
        }
        e.push(AdminEntry::heading("CLASSIC TOOLS"));
        e.push(AdminEntry::button(&format!("FREEZE TIME: {}", on_off(self.admin_frozen)), "freeze"));
        e.push(AdminEntry::button(&format!("INFINITE POWER: {}", on_off(self.sim.infinite_power)), "inf_power"));
        e.push(AdminEntry::button(&format!("GOD MODE: {}", on_off(self.sim.god)), "god"));
        e.push(AdminEntry::button(&format!("NO SCARE: {}", on_off(self.admin_no_scare)), "no_scare"));
        e.push(AdminEntry::button(&format!("NOCLIP: {}", on_off(self.admin_noclip)), "noclip"));
        e.push(AdminEntry::button(&format!("REVEAL HUNTERS ON THE MAP: {}", on_off(self.admin_reveal)), "reveal"));
        for (label, id) in [("HUNTER SPEED 0.5X", "speed_0.5"), ("HUNTER SPEED 1X", "speed_1"), ("HUNTER SPEED 2X", "speed_2"), ("HUNTER SPEED 4X", "speed_4")] {
            e.push(AdminEntry::button(label, id));
        }
        for (label, id) in [("POWER 100%", "power_100"), ("POWER 50%", "power_50"), ("POWER 10%", "power_10"), ("POWER 0 (BLACKOUT)", "power_0")] {
            e.push(AdminEntry::button(label, id));
        }
        for h in 0..6 {
            e.push(AdminEntry::button(&format!("SET CLOCK TO {}", hour_label(h)), &format!("hour_{h}")));
        }
        e.push(AdminEntry::button("PLAY ZELLIA'S CALL", "play_call"));
        e.push(AdminEntry::button("SKIP CALL", "skip_call"));
        for h in &self.sim.hunters {
            e.push(AdminEntry::button(&format!("{} AI +1", h.id.to_uppercase()), &format!("ai_up:{}", h.id)));
            e.push(AdminEntry::button(&format!("{} AI -1", h.id.to_uppercase()), &format!("ai_down:{}", h.id)));
        }
        for room in cam_rooms() {
            e.push(AdminEntry::button(&format!("SEND ALL HUNTERS TO {}", room.to_uppercase()), &format!("all_to:{room}")));
        }
        e
    }

    fn admin_do(&mut self, id: &str) {
        self.pending_admin.push(id.to_string());
    }

    fn debug_win(&mut self) {
        if self.playing() {
            self.sim.over = Over::Win;
        }
    }

    fn debug_lose(&mut self) {
        if self.playing() {
            self.sim.over = Over::Caught;
            self.sim.by = self.sim.hunters.first().map(|h| h.id.clone()).unwrap_or_default();
        }
    }
}

impl ClassicMode {
    /// R: mute Zellia (ends the call; the key hides her box either way).
    fn mute_zellia(&mut self, ctx: &mut ShiftCtx) {
        ctx.hud.call.mute();
        ctx.hud.zellia_muted = true;
        ctx.hud.call_text.clear();
        self.call_shown = false;
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "ON"
    } else {
        "OFF"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{AdminEntry, Session};

    fn session(night: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("classic", night, BTreeMap::new(), Some(seed));
        s
    }

    fn mode(night: i32) -> ClassicMode {
        ClassicMode::new(&session(night, 4242))
    }

    fn names(fx: &[Fx]) -> Vec<&'static str> {
        let mut out = Vec::new();
        for f in fx {
            match f {
                Fx::Cues(c) => out.extend(c.iter().map(|c| c.name)),
                Fx::CallText(_) => out.push("call"),
                Fx::Flash => out.push("flash"),
                Fx::Note(_) => out.push("note"),
                Fx::OpenMonitor(_) => out.push("open"),
                Fx::MeetEyes => out.push("meet"),
                Fx::PlayCall => out.push("play"),
                Fx::SkipCall => out.push("skip"),
            }
        }
        out
    }

    #[test]
    fn a_new_shift_matches_its_session() {
        let m = mode(4);
        assert_eq!(m.id(), "classic");
        assert_eq!(m.night, 4);
        assert_eq!(m.seed, 4242);
        assert!(!m.sim.kathy_night);
        assert_eq!(m.sim.hunters.len(), 4);
        assert_eq!(m.outcome(), Outcome::Running);
        assert!(m.killer().is_none());
        assert_eq!(m.monitor_room, "stage");
        assert!(!m.wants_mouse_free());
        let k = mode(7);
        assert!(k.sim.kathy_night);
    }

    #[test]
    fn doors_toggle_only_while_the_power_is_on() {
        let mut m = mode(2);
        assert!(m.toggle_door(0));
        assert!(m.door_shut[0]);
        assert!(m.toggle_door(0));
        assert!(!m.door_shut[0]);
        m.sim.out = true;
        assert!(!m.toggle_door(1));
        m.set_door(1, true);
        assert!(!m.door_shut[1]);
        m.set_light(0, true);
        assert!(!m.light_held[0], "no light in the dark");
        m.sim.out = false;
        m.set_light(0, true);
        assert!(m.light_held[0]);
        m.sim.over = Over::Win;
        assert!(!m.toggle_door(0), "a decided night takes no input");
    }

    #[test]
    fn the_power_going_out_opens_everything_and_says_so() {
        let mut m = mode(2);
        m.door_shut = [true, true];
        m.light_held = [true, true];
        let fx = m.on_event(&SimEvent::Out);
        assert_eq!(m.door_shut, [false, false]);
        assert_eq!(m.light_held, [false, false]);
        assert_eq!(names(&fx), vec!["call", "power_out"]);
        match &fx[0] {
            Fx::CallText(t) => assert_eq!(t, "The power is out."),
            _ => panic!("the call text comes first"),
        }
    }

    #[test]
    fn a_flinch_starts_the_pose_timer_and_bursts() {
        let mut m = mode(2);
        let fx = m.on_event(&SimEvent::Flinch { id: "dovelle".into(), room: "doorL".into() });
        assert_eq!(m.flinch_left.get("dovelle"), Some(&0.7));
        assert_eq!(names(&fx), vec!["burst"]);
        match &fx[0] {
            Fx::Cues(c) => assert_eq!(c[0].pan, -1.0),
            _ => panic!("cues"),
        }
    }

    #[test]
    fn repelled_and_pushed_start_the_retreat_pose_and_arrivals_clank() {
        let mut m = mode(2);
        let fx = m.on_event(&SimEvent::Repelled { id: "dovelle".into(), room: "dining".into(), from: "doorL".into() });
        assert_eq!(m.retreat_left.get("dovelle"), Some(&1.4));
        assert_eq!(names(&fx), vec!["door", "step"]);
        let fx = m.on_event(&SimEvent::Pushed { id: "aki".into(), room: "closet".into(), from: "doorR".into() });
        assert_eq!(m.retreat_left.get("aki"), Some(&1.4));
        assert_eq!(names(&fx), vec!["door", "step"]);
        let fx = m.on_event(&SimEvent::Arrived { id: "dovelle".into(), room: "doorL".into(), from: "westhall".into() });
        assert_eq!(names(&fx), vec!["step", "clank"]);
    }

    #[test]
    fn a_hunter_on_the_move_is_only_heard_in_the_halls() {
        let mut m = mode(2);
        let hall = m.on_event(&SimEvent::Moved { id: "dovelle".into(), room: "westhall".into(), from: "dining".into() });
        assert_eq!(names(&hall), vec!["step"]);
        let far = m.on_event(&SimEvent::Moved { id: "dovelle".into(), room: "dining".into(), from: "stage".into() });
        assert!(names(&far).is_empty());
    }

    #[test]
    fn kathys_fading_away_makes_no_sound() {
        let mut m = mode(7);
        let fx = m.on_event(&SimEvent::Repelled { id: "kathy".into(), room: "stage".into(), from: "doorL".into() });
        assert!(names(&fx).is_empty());
        assert_eq!(m.retreat_left.get("kathy"), Some(&1.4), "the pose timer still runs");
    }

    #[test]
    fn a_move_on_the_watched_feed_flashes_static() {
        let mut m = mode(2);
        m.monitor_up = true;
        m.monitor_room = "stage".to_string();
        let to = m.on_event(&SimEvent::Moved { id: "dovelle".into(), room: "dining".into(), from: "stage".into() });
        assert!(names(&to).contains(&"flash"), "she left the feed you watch");
        let elsewhere = m.on_event(&SimEvent::Moved { id: "ravelle".into(), room: "corner".into(), from: "easthall".into() });
        assert!(!names(&elsewhere).contains(&"flash"));
        m.monitor_up = false;
        let closed = m.on_event(&SimEvent::Moved { id: "dovelle".into(), room: "dining".into(), from: "stage".into() });
        assert!(!names(&closed).contains(&"flash"));
    }

    #[test]
    fn kathys_hum_pans_to_her_door_and_a_shut_door_muffles_it() {
        let mut m = mode(7);
        let open = m.on_event(&SimEvent::Hum { id: "kathy".into(), door: "doorL".into(), muffled: false });
        let shut = m.on_event(&SimEvent::Hum { id: "kathy".into(), door: "doorR".into(), muffled: true });
        match (&open[0], &shut[0]) {
            (Fx::Cues(a), Fx::Cues(b)) => {
                assert_eq!(a[0].name, "breath");
                assert!(a[0].pan < 0.0 && b[0].pan > 0.0);
                assert!(a[0].intensity > b[0].intensity);
            }
            _ => panic!("cues"),
        }
    }

    #[test]
    fn a_caught_night_names_the_killer_and_the_darkness() {
        let mut m = mode(3);
        m.sim.over = Over::Caught;
        m.sim.by = "nyxie".to_string();
        assert_eq!(m.killer(), Some(("nyxie".to_string(), "darkness".to_string())));
        m.sim.by = "aki".to_string();
        assert_eq!(m.killer(), Some(("aki".to_string(), String::new())));
        assert_eq!(m.outcome(), Outcome::Lost);
        m.admin_no_scare = true;
        assert!(m.killer().is_none(), "no scare skips the death moment");
    }

    #[test]
    fn debug_win_and_lose_decide_the_night_once() {
        let mut m = mode(3);
        m.debug_win();
        assert_eq!(m.outcome(), Outcome::Won);
        m.debug_lose();
        assert_eq!(m.outcome(), Outcome::Won, "a decided night stays decided");
        let mut l = mode(3);
        l.debug_lose();
        assert_eq!(l.outcome(), Outcome::Lost);
        assert_eq!(l.sim.by, "dovelle");
        let mut k = mode(7);
        k.debug_lose();
        assert_eq!(k.sim.by, "kathy");
    }

    #[test]
    fn the_admin_rows_change_the_game() {
        let mut m = mode(7);
        m.admin_apply("door_l_shut");
        assert!(m.door_shut[0]);
        m.admin_apply("door_l_open");
        assert!(!m.door_shut[0]);
        m.admin_apply("light_r_on");
        assert!(m.light_held[1] && m.admin_light[1]);
        m.admin_apply("light_r_off");
        assert!(!m.light_held[1] && !m.admin_light[1]);
        m.admin_apply("god");
        assert!(m.sim.god);
        m.admin_apply("inf_power");
        assert!(m.sim.infinite_power);
        m.admin_apply("speed_4");
        assert_eq!(m.sim.hunter_speed, 4.0);
        m.admin_apply("hour_3");
        assert_eq!(m.sim.hour(), 3);
        assert_eq!(m.hour_seen, 3);
        assert_eq!(m.elapsed_ms, m.sim.elapsed);
        m.admin_apply("all_to_doors");
        assert!(m.sim.hunters.iter().all(|h| h.is_at_door()));
        assert!(m.dirty);
        let fx = m.admin_apply("kathy_meet");
        assert_eq!(names(&fx), vec!["open", "meet"]);
        assert_eq!(m.sim.hunter("kathy").map(|k| k.room.as_str()), Some("westhall"));
        let fx = m.admin_apply("power_0");
        assert!(names(&fx).contains(&"power_out"));
        assert!(m.sim.out);
        // a classic night has no Kathy to meet
        let mut c = mode(2);
        assert!(c.admin_apply("kathy_meet").is_empty());
        c.admin_apply("ai_up:dovelle");
        assert_eq!(c.sim.hunter("dovelle").map(|h| h.base), Some(5));
        c.admin_apply("ai_down:dovelle");
        c.admin_apply("ai_down:dovelle");
        assert_eq!(c.sim.hunter("dovelle").map(|h| h.base), Some(3));
        c.admin_apply("all_to:kitchen");
        assert!(c.sim.hunters.iter().all(|h| h.room == "kitchen"));
    }

    #[test]
    fn the_admin_rows_are_listed_with_state_and_ids() {
        let mut m = mode(3);
        let rows = m.admin_entries();
        assert!(rows.iter().any(|r| r.heading && r.label.starts_with("CLASSIC LEVEL 3")));
        assert!(rows.iter().any(|r| !r.heading && r.id == "freeze" && r.label == "FREEZE TIME: OFF"));
        assert!(rows.iter().any(|r| r.id == "hour_5"));
        assert!(rows.iter().any(|r| r.id == "all_to:corner"));
        assert!(rows.iter().any(|r| r.heading && r.label.starts_with("DOVELLE  stage  AI 6")));
        m.admin_do("god");
        assert_eq!(m.pending_admin, vec!["god".to_string()]);
        let ids: Vec<&AdminEntry> = rows.iter().filter(|r| !r.heading).collect();
        for (i, a) in ids.iter().enumerate() {
            assert!(ids.iter().skip(i + 1).all(|b| b.id != a.id), "duplicate row id {}", a.id);
        }
    }

    #[test]
    fn the_hint_line_lists_the_office_keys_and_kathys_meet_key() {
        let b = crate::core::Bindings::from_save(&{
            let mut s = crate::core::Save::default();
            s.debug_run = true;
            s
        });
        let m = mode(2);
        let acts = m.hint_actions();
        let refs: Vec<&str> = acts.iter().map(|s| s.as_str()).collect();
        let line = b.hint_line(false, &refs);
        assert!(line.contains("Z / X doors") && line.contains("G / H lights") && line.contains("Y lamp"), "{line}");
        let k = mode(7);
        assert!(k.hint_actions().iter().any(|a| a == MEET));
        let acts = k.hint_actions();
        let refs: Vec<&str> = acts.iter().map(|s| s.as_str()).collect();
        assert!(b.hint_line(true, &refs).contains("meet her eyes"));
    }

    #[test]
    fn the_end_cards_come_from_the_outcome() {
        let mut m = mode(3);
        m.sim.over = Over::Win;
        let c = m.view.win_card(m.night, m.is_custom);
        assert!(c.can_next && c.body.contains("Night 4 is open."));
        let mut l = mode(5);
        l.sim.over = Over::Caught;
        l.sim.by = "ilyra".to_string();
        let c = l.view.caught_card(&l.sim.by, l.night);
        assert_eq!(c.body, "Ilyra reached you on night 5.");
    }

    #[test]
    fn camera_labels_follow_the_building_table() {
        assert_eq!(ClassicMode::cam_label("stage"), "CAM 1  SHOW STAGE");
        assert_eq!(ClassicMode::cam_label("corner"), "CAM 8  EAST CORNER");
        assert_eq!(ClassicMode::cam_label("doorL"), "");
        assert_eq!(ClassicMode::cam_label("nowhere"), "");
        assert_eq!(ClassicMode::pan_of("westhall"), -0.8);
        assert_eq!(ClassicMode::pan_of("doorR"), 1.0);
    }

    #[test]
    fn the_scene_syncs_the_walkers_without_a_window() {
        let mut m = mode(3);
        m.sync_world(vec3(0.0, 0.0, 0.9), true);
        assert!(m.placement.crowd.agent("dovelle").is_some());
        m.sim.admin_send("dovelle", "dining");
        m.sync_world(vec3(0.0, 0.0, 0.9), false);
        assert!(m.placement.crowd.agent("dovelle").map_or(false, |a| a.walker.is_walking()));
        // a flinching hunter has her idle behaviour switched off
        m.flinch_left.insert("ravelle".to_string(), 0.7);
        m.sync_world(vec3(0.0, 0.0, 0.9), false);
        assert!(!m.placement.crowd.agent("ravelle").map_or(true, |a| a.brain.enabled()));
    }

    #[test]
    fn near_office_cuts_the_building_off() {
        let v = View::embedded();
        assert!(near_office(vec3(-4.9, 0.0, 0.3), &v));
        assert!(!near_office(vec3(0.0, 0.0, -30.0), &v));
    }

    #[test]
    fn the_nyxie_walk_uses_the_blackout_time() {
        let m = mode(2);
        let lvl = 5;
        assert_eq!(blackout_ms(lvl), 9500.0);
        let from = Office::door_stand(&m.view.nyxie_door, 0, 0.0);
        assert!((from.x + 4.9).abs() < 1e-4 && (from.z - 0.3).abs() < 1e-4);
        let _ = vec3(0.0, 0.0, 0.0);
    }
}
