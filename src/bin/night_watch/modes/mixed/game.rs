//! Mixed Shift and Endless (mode `mixed`), the 3D shift scene (port of `mixed_game.gd` and `mixed_admin.gd`): the transit hub
//! (`world.rs`) around the security room with three permanent vents, a vent run above the building (CAM 9), a drawn roster
//! of threats run by a pacing director (`sim.rs`), and no captions: every warning is a sound, a light, a figure or a pulsing
//! camera button. You use the stations (the monitor, the DOOR buttons, the three vent hatches held to seal, the desk lamp, the
//! chair) or the quick keys: Z / X doors, V / B / N hold to seal the left / top / right vent, tap G / Y / H the vent light,
//! hold R to wind the music box, Space the monitor, 1-8 / arrows / 9 the cameras.
//!
//! The hunters physically walk (contract 9.1): the sim decides WHEN and WHERE (`Presence::sync` turns every threat's state into a
//! target), and the `Crowd` carries her there along the nav graph (`data/nav/mixed.json`), so she is visible crossing a feed.
//! Between moves a hunter stares into the camera, does a task or waits (`data/behaviour.json`); the attack timers never wait for it.
#![allow(dead_code)]

use super::data::{endless_clock, hours_text, tun, Kind, Side};
use super::sim::{Auto, Ev, EvType, MixedSim, Over, Threat};
#[allow(unused_imports)] // some of these are used by the tests only
use super::skin::{self, SkinBag, CAMS};
use super::world::{self, HubWorld};
use crate::audio::cue_names;
#[allow(unused_imports)] // some of these are used by the tests only
use crate::core::{fnv1a, AdminEntry, CallScript, EndCard, Fnv, Headless, Mode, Outcome, Session, ShiftCtx};
use crate::gfx::office::{SEAT, SPAWN};
use crate::gfx::{Office, OfficeConfig, OfficeVariant};
use crate::hud::admin::{AdminAction, ModeAdmin};
use crate::hunters::{Actor, ActorDriver, BehaviourData, BehaviourEvent, Crowd, HunterLib, Pace, PoseState};
use macroquad::prelude::{clear_background, vec3, Color, Vec3};
use std::collections::HashMap;
use std::rc::Rc;

/// Godot walks a hop in 2 s, which sprints the 20-40 m of this hub; the pace here is route length / this budget (clamped to
/// the walker's 1.2..6.0 m/s). Only the looks change: the sim's timers decide when she is at a door.
pub const WALK_BUDGET_S: f32 = 6.0;
/// The scheduled length of the static flash (seconds).
const FLASH_S: f32 = 0.14;
/// The cameras 1-8 (the vent run, CAM 9, is picked with 9 or its map button).
const BUILDING_CAMS: usize = 8;

// ------------------------------------------------------------------------------------------------ where the hunters are

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// Standing in a camera room (the walker's route).
    Hub,
    /// Crawling the vent run.
    Run,
    /// At an office door, creeping closer.
    Door,
    /// Her face in an office vent.
    VentStand,
    /// At the camera's lens (the Counter).
    Lens,
    /// Inside the office (the monitor-gated one).
    Office,
}

/// What the rules say about one threat, as a place to be: Godot's `_target`.
#[derive(Clone, Debug)]
pub struct Target {
    pub at: Where,
    /// The sim room for `Hub`, else a short key ("run", "doorL", "ventT", "lens", "office").
    pub key: String,
    pub pos: Vec3,
    pub look: Vec3,
    pub visible: bool,
    /// Placed at once (no walk): the vent mouth, the lens, the office.
    pub snap: bool,
}

/// What the world looks like from the player's side.
#[derive(Clone, Debug)]
pub struct Env {
    pub eye: Vec3,
    pub monitor_up: bool,
}

fn hidden(look: Vec3) -> Target {
    Target { at: Where::Hub, key: "away".to_string(), pos: Vec3::ZERO, look, visible: false, snap: false }
}

/// Where a threat is and what she is doing, from the rules.
pub fn target_of(sim: &MixedSim, th: &Threat, slot: usize, env: &Env) -> Target {
    let t = tun();
    let c = &sim.c;
    let face = env.eye;
    if th.armed {
        let Some(side) = th.side else {
            // the Counter: her face at the lens
            let lens = world::camera_transform(&c.mon.room).0;
            return Target { at: Where::Lens, key: "lens".to_string(), pos: world::stand_position(&c.mon.room, 1), look: lens, visible: env.monitor_up, snap: true };
        };
        if th.kind == Kind::Vent {
            return Target {
                at: Where::VentStand,
                key: side.vent_room().to_string(),
                pos: world::vent_stand(side),
                look: face,
                visible: !c.vents[side.idx()].shut,
                snap: true,
            };
        }
        let door = side.door().unwrap_or("doorL");
        let progress = th.armed_ms / th.tell_need.max(1.0);
        return Target { at: Where::Door, key: door.to_string(), pos: Office::door_stand(door, 0, progress as f32), look: face, visible: true, snap: false };
    }
    match th.kind {
        Kind::Monitor => {
            if th.present {
                let sx = if th.side == Some(Side::L) { -2.2 } else { 2.2 };
                return Target { at: Where::Office, key: "office".to_string(), pos: vec3(sx, 0.0, 0.9 - 0.3 * th.steps as f32), look: face, visible: true, snap: true };
            }
        }
        Kind::Counter => {
            if th.c >= t.timing_of(Kind::Counter).cue_at && env.monitor_up {
                let lens = world::camera_transform(&c.mon.room).0;
                return Target { at: Where::Lens, key: "lens".to_string(), pos: world::stand_position(&c.mon.room, 1), look: lens, visible: true, snap: true };
            }
        }
        _ => {
            if let Some(room) = th.room() {
                if th.kind == Kind::Vent {
                    let spot = super::sim::Core::vent_spot_of(th);
                    return Target { at: Where::Run, key: "run".to_string(), pos: world::vent_spot_pos(spot), look: world::camera_transform("vent").0, visible: true, snap: false };
                }
                return Target {
                    at: Where::Hub,
                    key: room.to_string(),
                    pos: world::stand_position(room, slot.min(1)),
                    look: world::camera_transform(room).0,
                    visible: true,
                    snap: false,
                };
            }
        }
    }
    hidden(face)
}

#[derive(Clone, Debug)]
struct Placed {
    key: String,
    slot: usize,
    pos: Vec3,
}

/// The crowd and the bookkeeping that turns sim states into walks. Pure (no GL): the mode, the headless run and the tests
/// share it.
pub struct Presence {
    pub crowd: Crowd,
    placed: HashMap<String, Placed>,
    /// Manual clock: hunters are put on their spots at once.
    pub snap: bool,
    /// How many times a hunter was put somewhere at once (not walked).
    pub teleports: u32,
}

impl Presence {
    pub fn new(seed: u32, snap: bool) -> Presence {
        let crowd = Crowd::new(Rc::new(world::nav()), BehaviourData::embedded(), seed ^ 0x4d49_5844);
        Presence { crowd, placed: HashMap::new(), snap, teleports: 0 }
    }

    /// Advance the walkers and their idle behaviour by `dt` seconds.
    pub fn tick(&mut self, dt: f32) -> Vec<BehaviourEvent> {
        if self.snap {
            return Vec::new();
        }
        self.crowd.tick(dt)
    }

    /// A new night: the first stare can fire again.
    pub fn new_night(&mut self) {
        self.crowd.new_night();
    }

    pub fn clear(&mut self) {
        let ids: Vec<String> = self.placed.keys().cloned().collect();
        for id in ids {
            self.crowd.remove(&id);
        }
        self.placed.clear();
    }

    fn put(&mut self, id: &str, room: &str, pos: Vec3, look: Vec3) {
        self.teleports += 1;
        self.crowd.add_at(id, room, pos, Some(look));
    }

    /// Bring the crowd in line with the rules: new targets walk, unchanged ones are left to their idle behaviour.
    pub fn sync(&mut self, sim: &MixedSim, env: &Env) {
        let mut slots: HashMap<String, usize> = HashMap::new();
        let mut present: Vec<String> = Vec::new();
        for th in &sim.threats {
            present.push(th.id.clone());
            let key = th.room().unwrap_or("-").to_string();
            let n = slots.entry(key).or_insert(0);
            let slot = *n;
            *n += 1;
            let tgt = target_of(sim, th, slot, env);
            if !tgt.visible {
                self.crowd.remove(&th.id);
                self.placed.remove(&th.id);
                continue;
            }
            self.place(th, &tgt, slot);
        }
        let gone: Vec<String> = self.placed.keys().filter(|k| !present.contains(k)).cloned().collect();
        for id in gone {
            self.crowd.remove(&id);
            self.placed.remove(&id);
        }
    }

    fn place(&mut self, th: &Threat, t: &Target, slot: usize) {
        let id = th.id.as_str();
        let slot_use = slot.min(1);
        let prev = self.placed.get(id).cloned();
        let first = prev.is_none();
        let key_changed = prev.as_ref().map_or(true, |p| p.key != t.key);
        match t.at {
            Where::Hub => {
                let slot_changed = prev.as_ref().map_or(true, |p| p.slot != slot_use);
                if first || key_changed || slot_changed {
                    if first || self.snap {
                        if self.crowd.add(id, &t.key, slot_use) {
                            self.teleports += 1;
                        }
                    } else {
                        self.crowd.send(id, &t.key, slot_use, Pace::Budget(WALK_BUDGET_S));
                    }
                }
            }
            Where::Run => {
                let moved = prev.as_ref().map_or(true, |p| p.pos.distance(t.pos) > 0.01);
                if first || key_changed || self.snap {
                    self.put(id, "vent", t.pos, t.look);
                } else if moved {
                    self.crowd.send_to(id, "vent", t.pos, Some(t.look), Pace::Walk);
                }
            }
            Where::Door => {
                // from the office floor (off the nav graph) she is put at the door; elsewhere she walks there
                let from_office = prev.as_ref().map_or(false, |p| p.key == "office");
                if first || self.snap || from_office {
                    self.put(id, &t.key, t.pos, t.look);
                } else if key_changed {
                    self.crowd.send_to(id, &t.key, t.pos, Some(t.look), Pace::Budget(WALK_BUDGET_S));
                } else {
                    self.crowd.set_final(id, t.pos, Some(t.look));
                }
            }
            Where::VentStand | Where::Lens | Where::Office => {
                let moved = prev.as_ref().map_or(true, |p| p.pos.distance(t.pos) > 0.01);
                if first || key_changed || moved {
                    self.put(id, &t.key, t.pos, t.look);
                }
            }
        }
        // idle behaviour only while she is just walking her route
        let walker = matches!(th.kind, Kind::Watched | Kind::Path | Kind::Winder);
        self.crowd.set_enabled(id, walker && th.active && !th.armed && t.at == Where::Hub);
        self.placed.insert(id.to_string(), Placed { key: t.key.clone(), slot: slot_use, pos: t.pos });
    }

    /// Hunters in a camera room's view, by position.
    pub fn visible_in(&self, sim_room: &str) -> Vec<String> {
        self.crowd.visible_in(sim_room).iter().map(|a| a.id.clone()).collect()
    }

    /// A hash of where everybody stands (for the headless digest).
    pub fn positions_hash(&self) -> u64 {
        let mut h = Fnv::new();
        for a in self.crowd.agents() {
            h.str(&a.id).i32((a.walker.pos.x * 100.0).round() as i32).i32((a.walker.pos.z * 100.0).round() as i32);
        }
        h.finish()
    }
}

// ------------------------------------------------------------------------------------------------ the headless run

/// The window-less half of the mode: the sim fed by the seeded autoplayer, with the hunters walking the nav graph.
pub struct WalkingHeadless {
    pub sim: MixedSim,
    pub pres: Presence,
    auto: Auto,
    env: Env,
}

impl WalkingHeadless {
    pub fn new(seed: u32, level: i32) -> WalkingHeadless {
        let sim = MixedSim::new(seed, level - 1);
        let mut pres = Presence::new(seed, false);
        let env = Env { eye: vec3(0.0, 1.7, 0.9), monitor_up: false };
        pres.sync(&sim, &env);
        WalkingHeadless { sim, pres, auto: Auto::new(seed), env }
    }
}

impl Headless for WalkingHeadless {
    fn id(&self) -> &'static str {
        "mixed"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        self.auto.drive(&mut self.sim, dt_ms);
        let events = self.sim.step(dt_ms);
        self.env.monitor_up = self.sim.c.mon.up;
        self.pres.sync(&self.sim, &self.env);
        self.pres.tick((dt_ms / 1000.0) as f32);
        events.iter().map(|e| if e.id.is_empty() { e.ty.name().to_string() } else { format!("{}:{}", e.ty.name(), e.id) }).collect()
    }

    fn outcome(&self) -> Outcome {
        match self.sim.c.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.c.elapsed
    }

    fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.sim.digest()).u64(self.pres.positions_hash());
        h.finish()
    }
}

// ------------------------------------------------------------------------------------------------ the scene

struct Slot {
    actor: Actor,
    driver: ActorDriver,
}

/// Everything that needs a GL context.
struct Gx {
    office: Office,
    hub: HubWorld,
    slots: HashMap<String, Slot>,
}

struct Death {
    by: String,
    name: String,
    hours: i32,
    advice: String,
}

pub struct MixedGame {
    seed: u32,
    level: i32,
    sim: MixedSim,
    skin: usize,
    bag: SkinBag,
    pres: Presence,
    gx: Option<Gx>,
    monitor_up: bool,
    seal_held: [bool; 3],
    wind_held: bool,
    hour_seen: i32,
    finished: bool,
    death: Option<Death>,
    pending_skin: Option<usize>,
    pending_note: Option<String>,
    mode_text: String,
    /// A presentation clock (seconds), for blinks and flickers.
    time_s: f32,
    dirty: bool,
    // admin
    admin_frozen: bool,
    admin_god: bool,
    admin_no_scare: bool,
    admin_reveal: bool,
    admin_infinite: bool,
    admin_noclip: bool,
}

fn pan_for_side(side: Option<Side>) -> f32 {
    match side {
        Some(Side::L) => -1.0,
        Some(Side::R) => 1.0,
        _ => 0.0,
    }
}

/// The pan of footsteps in a sim room: left to right across the hub.
fn pan_of_room(room: &str) -> f32 {
    (world::centre_of(room).x / 20.0).clamp(-1.0, 1.0)
}

/// GDScript's `capitalize()`: the first letter of every word in upper case.
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl MixedGame {
    pub fn new(session: &Session) -> MixedGame {
        let seed = session.seed_or_fresh();
        let level = session.level().clamp(1, 5);
        let sim = MixedSim::new(seed, level - 1);
        let mut bag = SkinBag::new();
        let skin = bag.pick(seed);
        let name = sim.c.diff.name.clone();
        MixedGame {
            seed,
            level,
            sim,
            skin,
            bag,
            pres: Presence::new(seed, false),
            gx: None,
            monitor_up: false,
            seal_held: [false; 3],
            wind_held: false,
            hour_seen: 0,
            finished: false,
            death: None,
            pending_skin: None,
            pending_note: None,
            mode_text: format!("9. Mixed Shift - {name}"),
            time_s: 0.0,
            dirty: false,
            admin_frozen: false,
            admin_god: false,
            admin_no_scare: false,
            admin_reveal: false,
            admin_infinite: false,
            admin_noclip: false,
        }
    }

    pub fn sim(&self) -> &MixedSim {
        &self.sim
    }

    pub fn skin_index(&self) -> usize {
        self.skin
    }

    fn playing(&self) -> bool {
        !self.finished && self.sim.c.over == Over::None
    }

    fn monitor_map_room(&self) -> String {
        world::map_for(&self.sim.c.mon.room)
    }

    fn flash(&self, ctx: &mut ShiftCtx) {
        ctx.hud.cam_flash_s = FLASH_S;
    }

    fn cues(&self, ctx: &mut ShiftCtx, list: Vec<crate::core::Cue>) {
        ctx.snd.cues(&list);
    }

    fn env(&self, ctx: &ShiftCtx) -> Env {
        Env { eye: ctx.player.eye(), monitor_up: self.monitor_up }
    }

    // ---- the hint and the camera title

    fn hint(&self, ctx: &ShiftCtx, monitor_up: bool) -> String {
        let b = ctx.bindings;
        let k = |a: &str| b.key_text(a);
        let mut s = if monitor_up {
            format!(
                "{} / {} or 1-8 camera   9 vent run   {} or {} close   hold {} wind (on the box's camera)",
                k("nw_cam_prev"),
                k("nw_cam_next"),
                k("nw_monitor"),
                k("interact"),
                k("nw_wind")
            )
        } else {
            format!(
                "WASD move   {} use   {} / {} doors   hold {} / {} / {} seal vents L / T / R   tap {} / {} / {} vent light   {} monitor   hold {} wind   P pause",
                k("interact"),
                k("nw_door_left"),
                k("nw_door_right"),
                k("nw_vent_l"),
                k("nw_vent_t"),
                k("nw_vent_r"),
                k("nw_vlight_l"),
                k("nw_vlight_t"),
                k("nw_vlight_r"),
                k("nw_monitor"),
                k("nw_wind")
            )
        };
        if crate::hud::admin::enabled() {
            s.push_str("   0 admin");
        }
        s
    }

    fn set_cam(&self, ctx: &mut ShiftCtx) {
        let map_room = self.monitor_map_room();
        ctx.hud.monitor_room = map_room.clone();
        let mut label = skin::skin(self.skin).cam_title(&map_room);
        let sim_room = self.sim.c.mon.room.as_str();
        if let Some(w) = self.sim.threats.iter().find(|t| t.kind == Kind::Winder && t.box_room == sim_room) {
            label.push_str(&format!(
                "    TUNE {}%{}    HOLD {} TO WIND",
                (100.0 * w.tune).round() as i32,
                if w.stopped { " STOPPED" } else { "" },
                ctx.bindings.key_text("nw_wind")
            ));
        }
        ctx.hud.cam_label = label;
    }

    // ---- the player's actions

    fn toggle_door(&mut self, ctx: &mut ShiftCtx, side: Side) {
        if !self.playing() {
            return;
        }
        let want = !self.sim.c.doors[side.idx()].want;
        let shut = self.sim.set_door(side, want);
        if (want && shut) || !want {
            self.cues(ctx, cue_names::door(shut));
        }
    }

    fn set_seal(&mut self, side: Side, on: bool) {
        if self.playing() {
            self.sim.set_vent(side, on);
            self.seal_held[side.idx()] = on;
        }
    }

    fn flash_vent(&mut self, ctx: &mut ShiftCtx, side: Side) {
        if !self.playing() {
            return;
        }
        let res = self.sim.light_vent(side);
        if res.ok {
            self.cues(ctx, cue_names::panel_button());
        }
    }

    fn set_wind(&mut self, ctx: &mut ShiftCtx, on: bool) {
        if self.playing() {
            self.sim.set_wind(on);
            self.wind_held = on;
            self.sync_feed(ctx);
        }
    }

    fn open_monitor(&mut self, ctx: &mut ShiftCtx, room: Option<&str>) {
        if !self.playing() || self.monitor_up {
            return;
        }
        self.monitor_up = true;
        self.sim.set_monitor(true);
        if let Some(r) = room {
            self.sim.set_room(r);
        }
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.hud.monitor_up = true;
        ctx.hud.hint = self.hint(ctx, true);
        self.set_cam(ctx);
        self.flash(ctx);
        self.cues(ctx, cue_names::monitor_toggle());
    }

    fn close_monitor(&mut self, ctx: &mut ShiftCtx) {
        if !self.monitor_up {
            return;
        }
        self.monitor_up = false;
        self.sim.set_monitor(false);
        ctx.hud.monitor_up = false;
        ctx.player.frozen = self.finished;
        ctx.player.look_frozen = self.finished;
        ctx.hud.hint = self.hint(ctx, false);
        self.cues(ctx, cue_names::monitor_toggle());
    }

    fn toggle_monitor(&mut self, ctx: &mut ShiftCtx) {
        if self.monitor_up {
            self.close_monitor(ctx);
        } else {
            self.open_monitor(ctx, None);
        }
    }

    fn select_camera(&mut self, ctx: &mut ShiftCtx, room: &str) {
        if !self.playing() || !self.monitor_up || !tun().cam_rooms.iter().any(|r| r == room) {
            return;
        }
        self.sim.set_room(room);
        self.set_cam(ctx);
        self.flash(ctx);
        self.cues(ctx, cue_names::camera_switch());
    }

    fn select_camera_number(&mut self, ctx: &mut ShiftCtx, n: usize) {
        if n >= 1 && n <= tun().cam_rooms.len() {
            let room = tun().cam_rooms[n - 1].clone();
            self.select_camera(ctx, &room);
        }
    }

    /// Left / right through the eight building cameras (the vent run is picked with 9 or its map button).
    fn cycle_camera(&mut self, ctx: &mut ShiftCtx, step: i32) {
        let rooms = &tun().cam_rooms[..BUILDING_CAMS];
        let cur = self.sim.c.mon.room.clone();
        let next = match rooms.iter().position(|r| *r == cur) {
            None => rooms[if step > 0 { 0 } else { rooms.len() - 1 }].clone(),
            Some(i) => rooms[(i as i32 + step).rem_euclid(rooms.len() as i32) as usize].clone(),
        };
        self.select_camera(ctx, &next);
    }

    /// A winding hold jumps the monitor to the music box's camera (the rules do it): follow it on screen.
    fn sync_feed(&mut self, ctx: &mut ShiftCtx) {
        if self.monitor_up && self.monitor_map_room() != ctx.hud.monitor_room {
            self.set_cam(ctx);
            self.flash(ctx);
        }
    }

    fn use_station(&mut self, ctx: &mut ShiftCtx, id: &str) {
        match id {
            "monitor" => self.toggle_monitor(ctx),
            "doorL_door" => {
                self.toggle_door(ctx, Side::L);
                self.cues(ctx, cue_names::panel_button());
            }
            "doorR_door" => {
                self.toggle_door(ctx, Side::R);
                self.cues(ctx, cue_names::panel_button());
            }
            "lamp" => {
                if let Some(g) = self.gx.as_mut() {
                    let on = g.office.state.lamp_on;
                    g.office.set_lamp(!on);
                }
            }
            "chair" => ctx.player.teleport(SEAT, 0.0, -0.1),
            _ => {}
        }
    }

    // ---- the events

    fn on_event(&mut self, ctx: &mut ShiftCtx, e: &Ev) {
        let side_pan = pan_for_side(e.side);
        match e.ty {
            EvType::Step | EvType::Turned => {
                self.cues(ctx, cue_names::office("footsteps", pan_of_room(&e.room), 0.8));
                if self.monitor_up && e.room == self.sim.c.mon.room {
                    self.flash(ctx);
                }
            }
            EvType::Arrive => self.cues(ctx, cue_names::office("crimson_door", side_pan, 1.0)),
            EvType::Tell => self.cues(ctx, cue_names::attack_warning(side_pan)),
            EvType::Pulse => self.cues(ctx, cue_names::office("presence", side_pan, 0.7)),
            EvType::Echo => self.cues(ctx, cue_names::office("camera", 0.0, 0.8)),
            EvType::Stop => self.cues(ctx, cue_names::attack_warning(0.0)),
            EvType::Wound => self.cues(ctx, cue_names::office("control", 0.0, 0.8)),
            EvType::Hum => self.cues(ctx, cue_names::office("control", 0.0, 0.3)),
            EvType::Answered | EvType::Gone => self.cues(ctx, cue_names::office("scrape", side_pan, 0.8)),
            EvType::Strain | EvType::VentStrain => self.cues(ctx, cue_names::office("scrape", 0.0, 1.0)),
            EvType::DoorShut | EvType::VentShut | EvType::VentOpen => self.cues(ctx, cue_names::panel_button()),
            EvType::Feint => {
                let kind = if e.kind == "step" { "footsteps" } else { "presence" };
                self.cues(ctx, cue_names::office(kind, side_pan, 0.3));
            }
            EvType::Spike => {
                if e.phase == "warn" {
                    self.cues(ctx, cue_names::office("drain", 0.0, 0.6));
                }
            }
            EvType::Chaos => {
                if e.phase == "start" {
                    self.cues(ctx, cue_names::office("static", 0.0, 0.8));
                }
            }
            EvType::Rotate => {
                self.cues(ctx, cue_names::hour_change());
                if self.sim.c.endless {
                    // ENDLESS keeps cycling: the roster re-draws and the map goes on to the next skin
                    self.pending_skin = Some(self.bag.pick(self.seed.wrapping_add(e.gen as u32)));
                    self.flash(ctx);
                }
            }
            _ => {}
        }
    }

    // ---- what you see

    fn sync_visuals(&mut self, ctx: &mut ShiftCtx) {
        let c = &self.sim.c;
        let doors = c.doors;
        let reveal: Vec<String> = if self.admin_reveal {
            self.sim.occupancy().iter().filter(|(_, v)| !v.is_empty()).map(|(r, _)| world::map_for(r)).collect()
        } else {
            Vec::new()
        };
        ctx.hud.cam_blips = reveal;
        let devices = doors.iter().filter(|d| d.shut).count() as i32 + self.monitor_up as i32;
        ctx.hud.hour = c.hour_of() % 6;
        ctx.hud.power = (100.0 * c.seal / c.seal_max) as f32;
        ctx.hud.usage = devices;
        ctx.hud.power_text = Some(format!("DOOR SEAL {}%", (100.0 * c.seal / c.seal_max).round() as i32));
        ctx.hud.usage_text = Some(format!("VENT SEAL {}%", (100.0 * c.vent_seal / tun().vent_seal_max).round() as i32));
        ctx.hud.clock_text = if c.endless { Some(endless_clock(c.elapsed)) } else { None };
        ctx.hud.mode_text = self.mode_text.clone();
        let shut = [doors[0].shut, doors[1].shut];
        let pulsing: [bool; 2] = {
            let mut p = [false; 2];
            for (i, side) in [Side::L, Side::R].iter().enumerate() {
                for th in &self.sim.threats {
                    if th.armed && th.tell_on && th.side == Some(*side) && th.kind != Kind::Vent {
                        p[i] = ((c.elapsed / 300.0) as i64) % 2 == 0;
                    }
                }
            }
            p
        };
        let vents = c.vents;
        let box_at = self.sim.threats.iter().find(|t| t.kind == Kind::Winder).map(|t| t.box_room.clone());
        let manual = ctx.manual_clock;
        if let Some(g) = self.gx.as_mut() {
            if manual {
                g.office.snap_slabs(shut);
            }
            for (i, door) in ["doorL", "doorR"].iter().enumerate() {
                g.office.set_button(&format!("{door}_door"), shut[i]);
                g.office.set_light(door, pulsing[i]);
            }
            for side in Side::ALL {
                g.office.world.set_visible(&format!("VentShutter{}", side.key()), vents[side.idx()].shut);
                g.office.world.set_visible(&format!("VentFlash{}", side.key()), vents[side.idx()].flash > 0.0);
            }
            match &box_at {
                Some(room) => g.hub.show_box(Some(world::stand_position(room, 0) + vec3(0.9, 0.0, -0.4))),
                None => g.hub.show_box(None),
            }
        }
        let env = self.env(ctx);
        self.pres.snap = ctx.manual_clock;
        self.pres.sync(&self.sim, &env);
        if self.monitor_up {
            self.set_cam(ctx);
        }
    }

    fn post_step(&mut self, ctx: &mut ShiftCtx) {
        let hour = self.sim.hour_of();
        if hour != self.hour_seen {
            self.hour_seen = hour;
            self.cues(ctx, cue_names::hour_change());
        }
        self.sync_feed(ctx);
        self.sync_visuals(ctx);
        self.finish_if_decided(ctx);
    }

    /// What would have stopped it, for the CAUGHT card.
    fn advice_for(&self, id: &str) -> String {
        let t = tun();
        match t.kind_of(id) {
            Some(k) => format!("{} ({}): {}.", t.name_of(id), t.text(k).label, title_case(&t.text(k).answer)),
            None => String::new(),
        }
    }

    fn freeze_player(&mut self, ctx: &mut ShiftCtx) {
        if self.monitor_up {
            self.close_monitor(ctx);
        }
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
    }

    fn finish_if_decided(&mut self, ctx: &mut ShiftCtx) {
        if self.finished {
            return;
        }
        match self.sim.c.over {
            Over::None => {}
            Over::Win => {
                self.finished = true;
                self.freeze_player(ctx);
                self.cues(ctx, cue_names::win());
            }
            Over::Caught => {
                self.finished = true;
                let by = self.sim.c.by.clone();
                let hours = self.sim.hour_reached();
                if self.sim.c.endless {
                    ctx.save.record_best("mixed", hours);
                }
                self.freeze_player(ctx);
                let short = ctx.save.short_death_scenes();
                self.cues(ctx, cue_names::caught(short));
                self.death = Some(Death { name: tun().name_of(&by), advice: self.advice_for(&by), by, hours });
            }
        }
    }

    /// God mode: whoever would catch you is answered (sent home), the tells stop, the seals come back.
    fn god_rescue(&mut self) {
        self.sim.c.over = Over::None;
        self.sim.c.by.clear();
        let ids = self.sim.roster();
        for id in ids {
            self.sim.admin_send_home(&id);
        }
        self.sim.c.lock.clear();
        self.sim.c.cool = tun().cooldown_ms;
        self.sim.c.seal = self.sim.c.seal.max(8000.0);
        self.sim.c.vent_seal = self.sim.c.vent_seal.max(4000.0);
    }

    fn has_next_level(&self, ctx: &ShiftCtx) -> bool {
        let scored = ctx.modes.get("mixed").map(|m| m.scored as i32).unwrap_or(4);
        self.level < scored
    }

    // ---- admin

    fn apply_admin(&mut self, id: &str) {
        let t = tun();
        let (head, tail) = match id.split_once(':') {
            Some((h, r)) => (h, r),
            None => (id, ""),
        };
        match head {
            "skin" => self.pending_skin = Some(skin::index_of(tail)),
            "skin_next" => {
                let n = self.bag.pick(self.seed.wrapping_add(self.sim.c.elapsed as u32));
                self.pending_skin = Some(n);
            }
            "hour" => {
                let h: i32 = tail.parse().unwrap_or(0);
                self.sim.admin_set_hour(if self.sim.c.endless { h } else { h.min(5) });
                self.hour_seen = h;
            }
            "redraw" => {
                self.sim.admin_redraw_roster();
            }
            "spike" => {
                self.sim.admin_force_spike();
            }
            "chaos" => {
                self.sim.admin_force_chaos(tail);
            }
            "door" => {
                if let Some((s, how)) = tail.split_once('_') {
                    if let Some(side) = Side::parse(s) {
                        self.sim.set_door(side, how == "shut");
                    }
                }
            }
            "seal" => {
                if let Some((s, how)) = tail.split_once('_') {
                    if let Some(side) = Side::parse(s) {
                        self.set_seal(side, how == "on");
                    }
                }
            }
            "vent_run" => {
                self.sim.admin_add_threat("verrine");
                self.sim.admin_place("verrine", "vent");
            }
            "vent_left" => {
                self.sim.admin_add_threat("verrine");
                self.sim.admin_place("verrine", "ventL");
            }
            "winder_stop" => {
                self.sim.admin_add_threat("glyra");
                if let Some(th) = self.sim.threat_mut("glyra") {
                    th.tune = 0.0006;
                }
            }
            "all_home" => {
                for id in self.sim.roster() {
                    self.sim.admin_send_home(&id);
                }
            }
            "t" => match tail {
                "freeze" => self.admin_frozen = !self.admin_frozen,
                "god" => self.admin_god = !self.admin_god,
                "reveal" => self.admin_reveal = !self.admin_reveal,
                "infinite" => self.admin_infinite = !self.admin_infinite,
                "noscare" => self.admin_no_scare = !self.admin_no_scare,
                "noclip" => self.admin_noclip = !self.admin_noclip,
                _ => {}
            },
            "speed" => {
                let m: f64 = tail.parse().unwrap_or(1.0);
                self.sim.c.hunter_speed = m.clamp(0.1, 8.0);
            }
            _ => {}
        }
        let _ = t;
        self.dirty = true;
    }

    fn rebuild_hub(&mut self, ctx: &mut ShiftCtx, skin_idx: usize) {
        let next = skin_idx.min(skin::SKINS.len() - 1);
        self.skin = next;
        if let Some(g) = self.gx.as_mut() {
            g.hub = HubWorld::build(&ctx.gfx.tex, next);
        }
        ctx.hud.cam_map = Some(world::layout_for(next));
        // the hunters stand where the rules put them in the new scene
        self.pres.clear();
        self.set_cam(ctx);
        self.dirty = true;
    }

    fn ensure_slot(g: &mut Gx, hunters: &mut HunterLib, id: &str) -> bool {
        if g.slots.contains_key(id) {
            return true;
        }
        match hunters.actor(id) {
            Some(actor) => {
                g.slots.insert(id.to_string(), Slot { actor, driver: ActorDriver::new() });
                true
            }
            None => false,
        }
    }

    fn update_actors(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        let MixedGame { gx, pres, sim, .. } = self;
        let Some(g) = gx.as_mut() else { return };
        for slot in g.slots.values_mut() {
            slot.actor.visible = false;
        }
        for a in pres.crowd.agents() {
            if !MixedGame::ensure_slot(g, ctx.hunters, &a.id) {
                continue;
            }
            let th = sim.threat(&a.id);
            let Some(slot) = g.slots.get_mut(&a.id) else { continue };
            slot.actor.visible = true;
            slot.actor.pos = a.walker.pos;
            slot.actor.yaw = a.walker.yaw;
            let kind = th.map(|t| t.kind);
            if kind == Some(Kind::Vent) {
                let in_shaft = th.map(|t| t.active || t.armed).unwrap_or(false);
                if slot.actor.update_vent(in_shaft, a.walker.moving, dt_s) {
                    slot.actor.animate(dt_s);
                    continue;
                }
            }
            let at_door = th.map(|t| t.armed && t.tell_on && t.kind != Kind::Vent && t.side.map_or(false, |s| s != Side::T)).unwrap_or(false);
            if at_door && !a.walker.moving {
                slot.actor.set_pose_state(PoseState::AtDoor);
                slot.driver.reset();
            } else {
                slot.driver.drive(a, &mut slot.actor);
            }
            slot.actor.animate(dt_s);
        }
    }
}

// ------------------------------------------------------------------------------------------------ the Mode

impl Mode for MixedGame {
    fn id(&self) -> &'static str {
        "mixed"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        let mut cfg = OfficeConfig::default();
        cfg.variant = OfficeVariant::Hub;
        cfg.drop_hall_end_walls = true;
        cfg.extra = world::office_extras();
        let mut office = Office::build(cfg, &ctx.gfx.tex);
        world::tweak_office(&mut office);
        let hub = HubWorld::build(&ctx.gfx.tex, self.skin);
        let mut gx = Gx { office, hub, slots: HashMap::new() };
        for id in self.sim.roster() {
            if let Some(mut actor) = ctx.hunters.actor(&id) {
                actor.visible = false;
                gx.slots.insert(id, Slot { actor, driver: ActorDriver::new() });
            }
        }
        self.gx = Some(gx);

        ctx.player.footprint = Some((-2.5, -2.0, 2.5, 2.0));
        ctx.player.teleport(SPAWN, 0.0, -0.1);

        let h = &mut *ctx.hud;
        h.cam_map = Some(world::layout_for(self.skin));
        h.monitor_available = true;
        h.monitor_up = false;
        h.show_crosshair = true;
        h.zellia_muted = false;
        h.call = CallScript::empty();
        h.mode_text = self.mode_text.clone();
        self.pres.snap = ctx.manual_clock;
        self.set_cam(ctx);
        ctx.hud.hint = self.hint(ctx, false);
        self.hour_seen = 0;
        self.sync_visuals(ctx);
        self.update_actors(ctx, 0.0);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        let input = ctx.input;
        let Some(g) = self.gx.as_ref() else { return };
        ctx.player.noclip = self.admin_noclip;
        ctx.player.frozen = self.monitor_up;
        ctx.player.look_frozen = self.monitor_up;
        let solids = g.office.all_solids();
        ctx.player.update(input, ctx.bindings, &solids, ctx.save.mouse_sens);
        let aimed: Option<String> = if self.monitor_up { None } else { ctx.player.aim(&g.office.stations, &solids).map(|i| g.office.stations[i].id.clone()) };
        ctx.hud.prompt = match &aimed {
            Some(id) => g.office.station(id).map(|s| s.prompt.clone()).unwrap_or_default(),
            None => String::new(),
        };
        ctx.hud.show_crosshair = !self.monitor_up;
        if !self.playing() {
            return;
        }

        // the monitor and its cameras
        if input.just("nw_monitor") {
            self.toggle_monitor(ctx);
        } else if self.monitor_up {
            if input.just("interact") {
                self.close_monitor(ctx);
            } else if input.just("nw_cam_prev") {
                self.cycle_camera(ctx, -1);
            } else if input.just("nw_cam_next") {
                self.cycle_camera(ctx, 1);
            } else {
                let mut n = input.digit.map(|d| d as usize).filter(|d| (1..=9).contains(d));
                if n.is_none() {
                    const CAM_ACTIONS: [&str; 9] = ["nw_cam_1", "nw_cam_2", "nw_cam_3", "nw_cam_4", "nw_cam_5", "nw_cam_6", "nw_cam_7", "nw_cam_8", "nw_cam_9"];
                    n = CAM_ACTIONS.iter().position(|a| input.just(a)).map(|i| i + 1);
                }
                if let Some(n) = n {
                    self.select_camera_number(ctx, n);
                }
            }
        } else if input.just("interact") {
            if let Some(id) = &aimed {
                let id = id.clone();
                self.use_station(ctx, &id);
            }
        }
        // the doors work from the monitor too
        if input.just("nw_door_left") {
            self.toggle_door(ctx, Side::L);
        } else if input.just("nw_door_right") {
            self.toggle_door(ctx, Side::R);
        }

        // vent seals (held), the vent light (a tap) and the music box (held)
        for (action, side) in [("nw_vent_l", Side::L), ("nw_vent_t", Side::T), ("nw_vent_r", Side::R)] {
            let by_station = input.is_down("interact") && aimed.as_deref() == Some(side_station(side));
            let want = input.is_down(action) || by_station;
            if want != self.seal_held[side.idx()] {
                self.set_seal(side, want);
            }
        }
        for (action, side) in [("nw_vlight_l", Side::L), ("nw_vlight_t", Side::T), ("nw_vlight_r", Side::R)] {
            if input.just(action) {
                self.flash_vent(ctx, side);
            }
        }
        let wind = input.is_down("nw_wind");
        if wind != self.wind_held {
            self.set_wind(ctx, wind);
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if !self.playing() || self.admin_frozen {
            return;
        }
        let mut left = dt_ms;
        while left > 0.0 && self.playing() {
            let dt = left.min(100.0);
            left -= dt;
            let mut events = self.sim.step(dt);
            if self.admin_infinite {
                self.sim.c.seal = self.sim.c.seal_max;
                self.sim.c.vent_seal = tun().vent_seal_max;
            }
            if self.admin_god && self.sim.c.over == Over::Caught {
                self.god_rescue();
                events.clear();
            }
            for e in &events {
                self.on_event(ctx, e);
            }
            self.post_step(ctx);
        }
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        if ctx.paused {
            return;
        }
        self.time_s += dt_s;
        if let Some(n) = self.pending_skin.take() {
            self.rebuild_hub(ctx, n);
        }
        if self.dirty {
            self.dirty = false;
            self.sync_visuals(ctx);
        }
        ctx.hud.cam_flash_s = (ctx.hud.cam_flash_s - dt_s).max(0.0);
        if !ctx.manual_clock {
            for ev in self.pres.tick(dt_s) {
                if let BehaviourEvent::FirstStare { subtitle, .. } = ev {
                    if self.monitor_up {
                        ctx.hud.set_note(&subtitle, 4.0);
                    } else {
                        self.pending_note = Some(subtitle);
                    }
                }
            }
            if self.playing() {
                let shut = [self.sim.c.doors[0].shut, self.sim.c.doors[1].shut];
                if let Some(g) = self.gx.as_mut() {
                    g.office.animate(shut, dt_s);
                }
            }
        }
        if self.monitor_up {
            if let Some(n) = self.pending_note.take() {
                ctx.hud.set_note(&n, 4.0);
            }
            // a hunter staring into the feed you watch: a soft static pulse
            let room = self.sim.c.mon.room.clone();
            let p = self.pres.crowd.stare_pulse(&room);
            if p > 0.0 {
                ctx.hud.cam_flash_s = ctx.hud.cam_flash_s.max(FLASH_S * p);
            }
            // CHAOS: a camera flickers
            if let Some(c) = &self.sim.c.chaos_s {
                if c.flicker > 0.0 && c.room == room && ((self.time_s * 2.5) as i32) % 3 == 0 {
                    ctx.hud.cam_flash_s = ctx.hud.cam_flash_s.max(FLASH_S);
                }
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
        let sk = skin::skin(self.skin);
        clear_background(Color::new(sk.sky[0], sk.sky[1], sk.sky[2], 1.0));
        let cam = ctx.player.cam;
        cam.apply();
        let lights = world::world_lights(g.office.lights(ctx.player.flashlight_spot()), self.skin);
        g.hub.draw(&ctx.gfx, &lights, cam.pos);
        g.office.world.draw(&ctx.gfx.shader, &lights, cam.pos);
        for s in g.slots.values() {
            if s.actor.visible {
                s.actor.draw(&ctx.gfx, &lights, cam.pos);
            }
        }
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let Some(g) = self.gx.as_ref() else { return };
        let sim_room = world::sim_for(room);
        let (pos, target) = world::camera_transform(&sim_room);
        ctx.gfx.feed.begin(pos, target, Vec3::Y);
        let blind = match &self.sim.c.chaos_s {
            Some(c) => c.flicker > 0.0 && c.room == sim_room && ((self.time_s * 2.5) as i32) % 3 == 0,
            None => false,
        };
        if !blind {
            let lights = world::feed_lights(self.skin, &sim_room);
            g.hub.draw(&ctx.gfx, &lights, pos);
            for s in g.slots.values() {
                if s.actor.visible && s.actor.pos.distance(pos) < 70.0 {
                    s.actor.draw(&ctx.gfx, &lights, pos);
                }
            }
        }
        ctx.gfx.feed.end();
    }

    fn outcome(&self) -> Outcome {
        match self.sim.c.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn end_card(&self, ctx: &ShiftCtx) -> Option<EndCard> {
        match self.sim.c.over {
            Over::None => None,
            Over::Win => {
                let more = self.has_next_level(ctx);
                let mut body = "6:00 AM\nYou held the building until 6 AM.".to_string();
                if more {
                    body.push_str(&format!("\n{} is open.", tun().shift(self.level).name));
                }
                Some(EndCard { title: format!("{} COMPLETE", self.sim.c.diff.name), body, can_next: more })
            }
            Over::Caught => {
                let (name, advice, hours) = match &self.death {
                    Some(d) => (d.name.clone(), d.advice.clone(), d.hours),
                    None => (tun().name_of(&self.sim.c.by), self.advice_for(&self.sim.c.by), self.sim.hour_reached()),
                };
                let name = if name.is_empty() { "A threat".to_string() } else { name };
                let mut body = format!("{name} reached you.\n{advice}");
                if self.sim.c.endless {
                    body.push_str(&format!("\nYou lasted {}. Best: {}.", hours_text(hours), hours_text(ctx.save.best_score("mixed"))));
                }
                Some(EndCard { title: "CAUGHT".to_string(), body, can_next: false })
            }
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.sim.c.over == Over::Caught && !self.admin_no_scare {
            let by = self.death.as_ref().map(|d| d.by.clone()).unwrap_or_else(|| self.sim.c.by.clone());
            Some((by, String::new()))
        } else {
            None
        }
    }

    fn wants_mouse_free(&self) -> bool {
        self.monitor_up
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        let c = &self.sim.c;
        let mut out = Vec::new();
        let clock = if c.endless { endless_clock(c.elapsed) } else { crate::core::hour_label(c.hour_of().min(5)) };
        out.push(AdminEntry::heading(&format!("MIXED SHIFT  {}  {}  SEED {}", c.diff.name, clock, self.seed)));
        out.push(AdminEntry::heading(&format!("DRESSING {}  ROSTER {}", skin::skin(self.skin).name, self.sim.roster().join(", ").to_uppercase())));
        out.push(AdminEntry::heading(&format!(
            "SEAL {}%  VENT SEAL {}%  BEAT {}  TENSION {:.2}",
            (100.0 * c.seal / c.seal_max).round() as i32,
            (100.0 * c.vent_seal / tun().vent_seal_max).round() as i32,
            c.beat.name().to_uppercase(),
            c.tension
        )));
        for th in &self.sim.threats {
            let where_ = th.room().map(|r| r.to_uppercase()).unwrap_or_else(|| "AWAY".to_string());
            let tell = if th.armed { format!("  TELL {:.1}/{:.1}s", th.armed_ms / 1000.0, th.tell_need / 1000.0) } else { String::new() };
            out.push(AdminEntry::heading(&format!("{}  {}  {}  {}{}", th.id.to_uppercase(), th.kind.name(), th.mood.name(), where_, tell)));
        }
        out.push(AdminEntry::heading(&format!(
            "FROZEN {}  GOD {}  NO SCARE {}  REVEAL {}  SPEED {:.1}X",
            self.admin_frozen, self.admin_god, self.admin_no_scare, self.admin_reveal, c.hunter_speed
        )));
        out
    }

    fn admin_do(&mut self, id: &str) {
        self.apply_admin(id);
    }

    fn debug_win(&mut self) {
        if self.sim.c.over == Over::None {
            self.sim.c.over = Over::Win;
        }
    }

    fn debug_lose(&mut self) {
        if self.sim.c.over == Over::None {
            self.sim.c.over = Over::Caught;
            self.sim.c.by = self.sim.threats.first().map(|t| t.id.clone()).unwrap_or_else(|| "haelith".to_string());
        }
    }
}

fn side_station(side: Side) -> &'static str {
    match side {
        Side::L => "ventL",
        Side::R => "ventR",
        Side::T => "ventT",
    }
}

// ------------------------------------------------------------------------------------------------ the admin panel

/// Mixed Shift's admin section (`mixed_admin.gd`): the rotation (play any of the nine maps, or the next one), ENDLESS's hour and
/// roster re-draw, the director, CHAOS events on demand, the doors and vents, the vent crawler's run, and threats on demand.
pub struct MixedAdmin;

impl ModeAdmin for MixedAdmin {
    fn mode_id(&self) -> &'static str {
        "mixed"
    }

    fn title(&self) -> String {
        "MIXED SHIFT SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        let mut out = Vec::new();
        for sk in skin::SKINS.iter() {
            out.push(AdminAction::new(&format!("MAP: {} ({})", sk.name, sk.mode.to_uppercase()), &format!("skin:{}", sk.id)));
        }
        out.push(AdminAction::new("MAP: NEXT IN THE ROTATION", "skin_next"));
        for h in [3, 6, 9, 12] {
            out.push(AdminAction::new(&format!("ENDLESS: SET HOUR {h}"), &format!("hour:{h}")));
        }
        out.push(AdminAction::new("ENDLESS: RE-DRAW THE ROSTER NOW", "redraw"));
        out.push(AdminAction::new("DIRECTOR: START THE NEXT SPIKE NOW", "spike"));
        for (k, label) in [
            ("mood", "A MOOD SWAPS"),
            ("wake", "THE VENT CRAWLER STIRS"),
            ("strain", "THE SEALS STRAIN"),
            ("flicker", "A CAMERA FLICKERS"),
            ("decoy", "A DECOY TELL"),
            ("boxfast", "THE BOX RUNS DOWN FAST"),
        ] {
            out.push(AdminAction::new(&format!("CHAOS: {label}"), &format!("chaos:{k}")));
        }
        for (id, label) in [("L_shut", "SHUT LEFT DOOR"), ("L_open", "OPEN LEFT DOOR"), ("R_shut", "SHUT RIGHT DOOR"), ("R_open", "OPEN RIGHT DOOR")] {
            out.push(AdminAction::new(label, &format!("door:{id}")));
        }
        for (id, label) in [("L_on", "SEAL LEFT VENT"), ("L_off", "RELEASE LEFT"), ("T_on", "SEAL TOP VENT"), ("T_off", "RELEASE TOP"), ("R_on", "SEAL RIGHT VENT"), ("R_off", "RELEASE RIGHT")] {
            out.push(AdminAction::new(label, &format!("seal:{id}")));
        }
        out.push(AdminAction::new("VENT CRAWLER INTO THE RUN (CAM 9)", "vent_run"));
        out.push(AdminAction::new("VENT CRAWLER AT THE LEFT VENT", "vent_left"));
        out.push(AdminAction::new("WINDER: STOP THE MUSIC BOX", "winder_stop"));
        out.push(AdminAction::new("ALL THREATS HOME", "all_home"));
        for (id, label) in [("freeze", "FREEZE THE SHIFT"), ("god", "GOD MODE"), ("reveal", "REVEAL HUNTERS ON THE MAP"), ("infinite", "INFINITE SEALS"), ("noscare", "NO DEATH SCENE"), ("noclip", "NOCLIP")] {
            out.push(AdminAction::new(&format!("TOGGLE: {label}"), &format!("t:{id}")));
        }
        for x in ["0.5", "1", "2", "4"] {
            out.push(AdminAction::new(&format!("HUNTER SPEED {x}X"), &format!("speed:{x}")));
        }
        out
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Outcome, Session};
    use std::collections::BTreeMap;

    fn session(level: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("mixed", level, BTreeMap::new(), Some(seed));
        s
    }

    fn env() -> Env {
        Env { eye: vec3(0.0, 1.7, 0.9), monitor_up: false }
    }

    fn solo(seed: u32, id: &str) -> MixedSim {
        let mut sim = MixedSim::new(seed, 1);
        sim.threats.clear();
        assert!(sim.admin_add_threat(id));
        sim.admin_set_hour(3);
        sim
    }

    #[test]
    fn a_new_game_matches_its_session() {
        let g = MixedGame::new(&session(4, 77));
        assert_eq!(g.level, 4);
        assert_eq!(g.sim.c.diff.name, "CHAOS");
        assert_eq!(g.id(), "mixed");
        assert_eq!(g.outcome(), Outcome::Running);
        assert!(g.skin < skin::SKINS.len());
        assert!(g.mode_text.contains("CHAOS"));
        // a level past the board is Endless
        assert_eq!(MixedGame::new(&session(9, 1)).sim.c.diff.name, "ENDLESS");
        // the same seed gives the same night and the same dressing
        let h = MixedGame::new(&session(4, 77));
        assert_eq!(g.sim.roster(), h.sim.roster());
        assert_eq!(g.skin, h.skin);
    }

    #[test]
    fn targets_follow_godots_target_function() {
        // a path walker in her route
        let mut sim = solo(5, "dovelle");
        assert!(sim.admin_place("dovelle", "westhall"));
        let t = target_of(&sim, &sim.threats[0], 0, &env());
        assert_eq!(t.at, Where::Hub);
        assert_eq!(t.key, "westhall");
        assert!(t.visible && !t.snap);
        assert_eq!(t.pos, world::stand_position("westhall", 0));
        assert_eq!(target_of(&sim, &sim.threats[0], 3, &env()).pos, world::stand_position("westhall", 1), "slots above 1 use slot 1");
        // at a door: creeping closer as the tell builds
        assert!(sim.admin_place("dovelle", "doorL"));
        let far = target_of(&sim, &sim.threats[0], 0, &env());
        assert_eq!(far.at, Where::Door);
        assert_eq!(far.key, "doorL");
        assert_eq!(far.pos, Office::door_stand("doorL", 0, 0.0));
        sim.threats[0].armed_ms = sim.threats[0].tell_need;
        let near = target_of(&sim, &sim.threats[0], 0, &env());
        assert!(near.pos.x > far.pos.x, "the left door stand moves toward the office: {:?} -> {:?}", far.pos, near.pos);
        // away
        sim.admin_send_home("dovelle");
        assert!(!target_of(&sim, &sim.threats[0], 0, &env()).visible);
    }

    #[test]
    fn the_vent_crawler_is_in_the_run_then_at_her_vent_until_it_is_sealed() {
        let mut sim = solo(5, "verrine");
        assert!(sim.admin_place("verrine", "vent"));
        let run = target_of(&sim, &sim.threats[0], 0, &env());
        assert_eq!(run.at, Where::Run);
        assert_eq!(run.pos, world::vent_spot_pos(spot_of(&sim.threats[0])));
        assert!(sim.admin_place("verrine", "ventR"));
        let at = target_of(&sim, &sim.threats[0], 0, &env());
        assert_eq!(at.at, Where::VentStand);
        assert_eq!(at.pos, vec3(2.3, 0.0, -1.7));
        assert!(at.snap && at.visible);
        sim.set_vent(Side::R, true);
        assert!(!target_of(&sim, &sim.threats[0], 0, &env()).visible, "a sealed vent hides her");
    }

    fn spot_of(th: &Threat) -> usize {
        super::super::sim::Core::vent_spot_of(th)
    }

    #[test]
    fn the_counter_and_the_monitor_gated_one_stand_where_the_rules_say() {
        let mut sim = solo(5, "haelith");
        sim.threats[0].c = 0.5;
        let hidden = target_of(&sim, &sim.threats[0], 0, &env());
        assert!(!hidden.visible, "no monitor, no face at the lens");
        let up = Env { eye: vec3(0.0, 1.7, 0.9), monitor_up: true };
        let lens = target_of(&sim, &sim.threats[0], 0, &up);
        assert_eq!(lens.at, Where::Lens);
        assert!(lens.visible && lens.snap);
        let mut sim = solo(5, "opalie");
        assert!(sim.admin_place("opalie", "office"));
        sim.threats[0].steps = 2;
        let t = target_of(&sim, &sim.threats[0], 0, &env());
        assert_eq!(t.at, Where::Office);
        assert!((t.pos.x + 2.2).abs() < 1e-5 && (t.pos.z - 0.3).abs() < 1e-5, "{:?}", t.pos);
    }

    #[test]
    fn a_threat_that_moves_walks_there_over_real_time() {
        let mut sim = solo(5, "ravelle");
        let mut pres = Presence::new(5, false);
        assert!(sim.admin_place("ravelle", "stage"));
        pres.sync(&sim, &env());
        let start = pres.crowd.agent("ravelle").unwrap().walker.pos;
        assert!(start.distance(world::stand_position("stage", 0)) < 0.01);
        // the rules move her to the dining area: she does not appear there
        assert!(sim.admin_place("ravelle", "dining"));
        pres.sync(&sim, &env());
        let a = pres.crowd.agent("ravelle").unwrap();
        assert!(a.walker.pos.distance(start) < 0.01, "still at the stage the instant the rules move her");
        assert!(a.walker.is_walking());
        let mut moved = 0.0;
        let mut last = a.walker.pos;
        for _ in 0..300 {
            pres.tick(0.1);
            let p = pres.crowd.agent("ravelle").unwrap().walker.pos;
            assert!(p.distance(last) <= 6.0 * 0.1 + 0.05, "no teleporting: {} m in a frame", p.distance(last));
            moved += p.distance(last);
            last = p;
        }
        assert!(moved > 15.0, "she walked {moved} m");
        let end = pres.crowd.agent("ravelle").unwrap();
        assert_eq!(pres.crowd.nav().room_at(end.walker.pos), Some("dining"), "she ended in the dining area at {:?}", end.walker.pos);
    }

    #[test]
    fn a_hunter_crossing_the_lane_is_seen_on_the_next_feed() {
        let mut sim = solo(5, "aki");
        let mut pres = Presence::new(5, false);
        assert!(sim.admin_place("aki", "stage"));
        pres.sync(&sim, &env());
        assert_eq!(pres.visible_in("stage"), vec!["aki".to_string()]);
        assert!(sim.admin_place("aki", "dining"));
        pres.sync(&sim, &env());
        let mut seen_in_dining_while_walking = false;
        for _ in 0..120 {
            pres.tick(0.1);
            let a = pres.crowd.agent("aki").unwrap();
            if a.walker.is_walking() && pres.visible_in("dining").contains(&"aki".to_string()) {
                seen_in_dining_while_walking = true;
            }
        }
        assert!(seen_in_dining_while_walking, "she can be watched entering the dining area");
    }

    #[test]
    fn a_door_walker_walks_up_the_hall_and_creeps_closer() {
        let mut sim = solo(7, "dovelle");
        let mut pres = Presence::new(7, false);
        assert!(sim.admin_place("dovelle", "westhall"));
        pres.sync(&sim, &env());
        assert!(sim.admin_place("dovelle", "doorL"));
        pres.sync(&sim, &env());
        for _ in 0..200 {
            pres.tick(0.1);
        }
        let a = pres.crowd.agent("dovelle").unwrap();
        assert_eq!(pres.crowd.nav().room_at(a.walker.pos), Some("doorL"));
        let first = a.walker.pos.x;
        sim.threats[0].armed_ms = sim.threats[0].tell_need;
        pres.sync(&sim, &env());
        for _ in 0..80 {
            pres.tick(0.1);
        }
        let later = pres.crowd.agent("dovelle").unwrap().walker.pos.x;
        assert!(later > first + 0.5, "{first} -> {later}");
        assert!(!pres.crowd.agent("dovelle").unwrap().brain.enabled(), "the idle behaviour is off while she is at the door");
    }

    #[test]
    fn idle_hunters_stare_work_or_wait_and_never_change_the_sim() {
        let mut sim = solo(9, "ilyra");
        let mut pres = Presence::new(9, false);
        assert!(sim.admin_place("ilyra", "kitchen"));
        pres.sync(&sim, &env());
        let digest = sim.digest();
        let mut activities = std::collections::BTreeSet::new();
        for _ in 0..3000 {
            pres.tick(0.1);
            activities.insert(format!("{:?}", pres.crowd.agent("ilyra").unwrap().brain.activity()));
        }
        assert_eq!(sim.digest(), digest, "idle behaviour never touches the rules");
        assert!(activities.len() >= 3, "{activities:?}");
        let a = pres.crowd.agent("ilyra").unwrap();
        assert_eq!(pres.crowd.nav().room_at(a.walker.pos), Some("kitchen"));
    }

    #[test]
    fn the_walking_headless_run_is_deterministic_and_reaches_an_outcome() {
        for lvl in 1..=4 {
            let mut a = WalkingHeadless::new(12345, lvl);
            let mut b = WalkingHeadless::new(12345, lvl);
            let mut t = 0.0;
            while t < 271_000.0 && a.outcome() == Outcome::Running {
                let ea = a.step_ms(100.0);
                let eb = b.step_ms(100.0);
                assert_eq!(ea, eb);
                t += 100.0;
            }
            assert_ne!(a.outcome(), Outcome::Running, "level {lvl}");
            assert_eq!(a.digest(), b.digest(), "level {lvl}");
        }
    }

    #[test]
    fn hunters_walk_between_camera_rooms_during_a_seeded_night() {
        let mut h = WalkingHeadless::new(4242, 4);
        let mut last: HashMap<String, Vec3> = HashMap::new();
        let mut walked = 0.0f32;
        let mut jumps = 0;
        let mut t = 0.0;
        while t < 200_000.0 && h.outcome() == Outcome::Running {
            let before = h.pres.teleports;
            h.step_ms(100.0);
            let placed_now = h.pres.teleports != before;
            for a in h.pres.crowd.agents() {
                let p = a.walker.pos;
                assert!(p.x.is_finite() && p.z.is_finite());
                if let Some(q) = last.get(&a.id) {
                    let d = p.distance(*q);
                    if d > 6.0 * 0.1 + 0.05 {
                        assert!(placed_now, "{} jumped {d} m without being placed", a.id);
                        jumps += 1;
                    } else {
                        walked += d;
                    }
                }
                last.insert(a.id.clone(), p);
            }
            t += 100.0;
        }
        assert!(walked > 30.0, "the hunters walked {walked} m ({jumps} placements)");
    }

    #[test]
    fn title_case_is_gdscripts_capitalize() {
        assert_eq!(title_case("switch camera, or lower the monitor"), "Switch Camera, Or Lower The Monitor");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn the_advice_names_the_threat_type_and_its_answer() {
        let g = MixedGame::new(&session(2, 3));
        let a = g.advice_for("haelith");
        assert!(a.starts_with("Haelith (Counter): "), "{a}");
        assert!(a.ends_with('.'));
        assert_eq!(g.advice_for("nobody"), "");
    }

    #[test]
    fn debug_win_and_lose_decide_the_shift() {
        let mut g = MixedGame::new(&session(2, 3));
        g.debug_lose();
        assert_eq!(g.outcome(), Outcome::Lost);
        assert!(g.killer().is_some());
        g.debug_win();
        assert_eq!(g.outcome(), Outcome::Lost, "a decided shift stays decided");
        let mut w = MixedGame::new(&session(2, 3));
        w.debug_win();
        assert_eq!(w.outcome(), Outcome::Won);
        assert!(w.killer().is_none());
    }

    #[test]
    fn the_admin_rows_change_the_game() {
        let mut g = MixedGame::new(&session(2, 3));
        g.admin_do("hour:3");
        assert_eq!(g.sim.hour_of(), 3);
        g.admin_do("door:L_shut");
        assert!(g.sim.c.doors[0].shut);
        g.admin_do("door:L_open");
        assert!(!g.sim.c.doors[0].shut);
        g.admin_do("seal:T_on");
        assert!(g.sim.c.vents[2].shut && g.seal_held[2]);
        g.admin_do("vent_left");
        assert!(g.sim.threat("verrine").map_or(false, |t| t.armed));
        g.admin_do("all_home");
        assert!(g.sim.threats.iter().all(|t| !t.armed));
        g.admin_do("speed:4");
        assert_eq!(g.sim.c.hunter_speed, 4.0);
        g.admin_do("t:god");
        assert!(g.admin_god);
        g.admin_do("skin:yard");
        assert_eq!(g.pending_skin, Some(6));
        g.admin_do("chaos:strain");
        assert!(g.sim.c.chaos_s.as_ref().map_or(false, |c| c.strain > 0.0));
    }

    #[test]
    fn every_admin_row_has_a_unique_id_the_game_understands() {
        let rows = MixedAdmin.actions();
        let mut ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert!(n >= 40);
        assert_eq!(MixedAdmin.mode_id(), "mixed");
        let mut g = MixedGame::new(&session(4, 3));
        for r in &rows {
            g.admin_do(&r.id);
            g.sim.c.over = Over::None;
        }
    }

    #[test]
    fn the_mode_text_and_clock_texts_follow_godot() {
        let g = MixedGame::new(&session(2, 3));
        assert_eq!(g.mode_text, "9. Mixed Shift - STEADY");
        assert_eq!(hours_text(2), "2 hours");
        assert!(endless_clock(0.0).contains("12 AM"));
    }

    #[test]
    fn the_hash_of_positions_is_stable_between_equal_crowds() {
        let a = WalkingHeadless::new(5, 2);
        let b = WalkingHeadless::new(5, 2);
        assert_eq!(a.pres.positions_hash(), b.pres.positions_hash());
        assert_eq!(fnv1a(b"mixed"), fnv1a(b"mixed"));
        let _ = CAMS;
    }
}
