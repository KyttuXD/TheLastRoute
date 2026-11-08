//! Salvage Run (mode `dayshift`), the 3D shift scene (`dayshift_game.gd`): first the day (the shop: spend the week's budget on
//! upgrades), then the night in the cargo bay, which has a workbench with a four-slot rack, a window and the Classic doors. Hold
//! USE on a piece to take it (salvaging is loud, noise makes the door visitors keener), shut the doors / hold the door lights as
//! in Classic, and keep quiet while Ysolde is at the window. The rules are `DayshiftSim` (sim.rs); the hunters walk the ship's
//! navigation graph (stage.rs) and are seen on the security cameras of the monitor (contract 9.1). `manual_clock` (--shot, tests)
//! skips the shop and puts everyone on their spot without walking.
#![allow(dead_code)]

use super::shop::{draw_shop, ShopAction, ShopState};
use super::sim::{bot_basket, carry_from, name_of, pan_of, tun, DayEvent, DayInput, DayOpts, DayshiftSim, Over, CAST_IDS, VISITORS};
use super::stage::{shift_setup, Stage};
use super::world::{bar_segments, begin_feed, cam_layout, day_lights, feed_lights, map, set_slot, Scene, SKY};
use crate::audio::cue_names;
use crate::core::{
    camera_action, hour_index, rating_of, CallEvent, CallScript, EndCard, Mode, Mulberry32, Outcome, Session, ShiftCtx, CAM_NEXT, CAM_PREV,
    DOOR_LEFT, DOOR_RIGHT, INTERACT, LIGHT_LEFT, LIGHT_RIGHT, MAX_AI, MONITOR, POINTS_PER_LEVEL,
};
use crate::gfx::office::SPAWN;
use crate::gfx::{Gfx, LightSet};
use crate::hunters::behaviour::{ActorDriver, BehaviourEvent};
use crate::hunters::Actor;
use macroquad::prelude::*;

const HINT: &str =
    "WASD move   hold E on a PIECE to salvage   E on DOOR buttons   hold E on LIGHT   Z / X doors   G / H lights   Space cameras   P pause";

/// The sat-at-the-desk spot and the standing-up spot.
const SEAT: Vec3 = Vec3::new(0.0, 0.0, -1.05);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Shop,
    Playing,
    Over,
}

/// How the shift ended (for the end card).
#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    None,
    Win { money: i32, need: i32, score: i32 },
    Short { money: i32, need: i32 },
    Caught { by: String },
}

/// The end card of a shift, with the Godot strings.
pub fn end_card_for(ending: &Ending, week: i32, custom: bool, win_carry: i32) -> Option<EndCard> {
    match ending {
        Ending::None => None,
        Ending::Win { money, need, score } => {
            let more = !custom && week < 5;
            let mut detail = format!("Quota met: ${money} of ${need}.");
            if win_carry > 0 && more {
                detail.push_str(&format!(" ${win_carry} carries over to next week's shop."));
            }
            if custom {
                let max = CAST_IDS.len() as i32 * MAX_AI * POINTS_PER_LEVEL;
                Some(EndCard {
                    title: "CUSTOM NIGHT COMPLETE".to_string(),
                    body: format!("6:00 AM\n{detail}\nScore {score} of {max}: {}", rating_of(*score, max)),
                    can_next: false,
                })
            } else {
                let tail = if more { format!("\nWeek {} is open.", week + 1) } else { "\nYou cleared every week.".to_string() };
                Some(EndCard { title: format!("WEEK {week} COMPLETE"), body: format!("6:00 AM\n{detail}{tail}"), can_next: more })
            }
        }
        Ending::Short { money, need } => Some(EndCard {
            title: "QUOTA MISSED".to_string(),
            body: format!("${money} of ${need} at 6 AM. The week is not cleared."),
            can_next: false,
        }),
        Ending::Caught { by } => Some(EndCard {
            title: "CAUGHT".to_string(),
            body: format!("{} reached you in week {week}.", name_of(by)),
            can_next: false,
        }),
    }
}

/// The label of a rack piece's kind ("Scrap").
pub fn kind_label(kind: &str) -> String {
    tun().item_kinds.iter().find(|k| k.id == kind).map(|k| k.label.clone()).unwrap_or_else(|| kind.to_string())
}

/// The rack prompt of a piece: "SCRAP $10 // HOLD TO SALVAGE".
pub fn rack_prompt(kind: &str, value: i32) -> String {
    format!("{} ${} // HOLD TO SALVAGE", kind_label(kind).to_uppercase(), value)
}

struct Hunter {
    id: &'static str,
    actor: Actor,
    driver: ActorDriver,
}

pub struct DayshiftMode {
    phase: Phase,
    week: i32,
    custom: bool,
    seed: u32,
    opts: DayOpts,
    shop: ShopState,
    sim: DayshiftSim,
    stage: Stage,
    scene: Option<Scene>,
    hunters: Vec<Hunter>,
    door_shut: [bool; 2],
    light_held: [bool; 2],
    work_slot: i32,
    sitting: bool,
    /// Seconds of static left after a camera switch.
    flash_s: f32,
    /// Seconds a power message stays in the call box.
    msg_s: f32,
    hour_seen: i32,
    win_carry: i32,
    ending: Ending,
    /// What each rack slot shows now (kind, bar segments), so a group is only rebuilt when it changes.
    slot_view: [Option<(String, usize)>; 4],
    pending: Vec<String>,
    frozen: bool,
    god: bool,
    no_scare: bool,
    dirty: bool,
    /// Whether the monitor was up at the last frame (`wants_mouse_free` has no context to read it from).
    monitor_seen: bool,
}

impl DayshiftMode {
    pub fn new(session: &Session) -> DayshiftMode {
        let (week, opts, seed) = shift_setup(session);
        let custom = session.is_custom;
        let pending = super::take_pending_carry(week);
        let carry = if custom { 0 } else { pending };
        let sim = DayshiftSim::new(week, Mulberry32::new(seed), opts.clone());
        let stage = Stage::new(&sim, seed);
        DayshiftMode {
            phase: Phase::Shop,
            week,
            custom,
            seed,
            opts,
            shop: ShopState::new(week, carry, custom),
            sim,
            stage,
            scene: None,
            hunters: Vec::new(),
            door_shut: [false; 2],
            light_held: [false; 2],
            work_slot: -1,
            sitting: false,
            flash_s: 0.0,
            msg_s: 0.0,
            hour_seen: 0,
            win_carry: 0,
            ending: Ending::None,
            slot_view: [None, None, None, None],
            pending: Vec::new(),
            frozen: false,
            god: false,
            no_scare: false,
            dirty: false,
            monitor_seen: false,
        }
    }

    /// The week's carry shown in the shop.
    pub fn carry(&self) -> i32 {
        self.shop.carry
    }

    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Shop => "shop",
            Phase::Playing => "playing",
            Phase::Over => "over",
        }
    }

    /// The shop's START SHIFT: the upgrades are bought, the night begins.
    fn begin_shift(&mut self, ctx: &mut ShiftCtx, basket: Vec<String>) {
        if self.phase != Phase::Shop {
            return;
        }
        self.opts.upgrades = basket;
        self.sim = DayshiftSim::new(self.week, Mulberry32::new(self.seed), self.opts.clone());
        self.stage = Stage::new(&self.sim, self.seed);
        let face = ctx.player.feet;
        self.stage.begin(&self.sim, face, ctx.manual_clock);
        self.phase = Phase::Playing;
        self.door_shut = [false; 2];
        self.light_held = [false; 2];
        self.work_slot = -1;
        self.hour_seen = 0;
        ctx.player.frozen = false;
        ctx.player.look_frozen = false;
        ctx.hud.monitor_available = true;
        ctx.hud.show_crosshair = true;
        ctx.hud.call = if self.custom { CallScript::empty() } else { CallScript::for_level("dayshift", self.week) };
        for h in self.hunters.iter_mut() {
            h.actor.visible = self.stage.is_active(h.id);
            h.driver.reset();
        }
        self.slot_view = [None, None, None, None];
        self.sync_visuals(ctx);
        self.drive_actors(0.0);
    }

    fn current_input(&self) -> DayInput {
        DayInput {
            work: self.work_slot >= 0,
            target: self.work_slot,
            door_l: self.door_shut[0],
            door_r: self.door_shut[1],
            light_l: self.light_held[0],
            light_r: self.light_held[1],
        }
    }

    // ---- player actions

    fn toggle_door(&mut self, ctx: &mut ShiftCtx, i: usize) {
        if self.phase != Phase::Playing || self.sim.out {
            return;
        }
        self.door_shut[i] = !self.door_shut[i];
        ctx.snd.cues(&cue_names::door(self.door_shut[i]));
    }

    fn open_monitor(&mut self, ctx: &mut ShiftCtx) {
        if self.phase != Phase::Playing {
            return;
        }
        if map().camera_number(&ctx.hud.monitor_room) == 0 {
            if let Some(c) = map().cameras.first() {
                ctx.hud.monitor_room = c.room.clone();
            }
        }
        ctx.hud.monitor_up = true;
        ctx.hud.prompt.clear();
        self.flash_s = 0.14;
        ctx.snd.cues(&cue_names::monitor_toggle());
    }

    fn close_monitor(&mut self, ctx: &mut ShiftCtx) {
        if ctx.hud.monitor_up {
            ctx.hud.monitor_up = false;
            ctx.snd.cues(&cue_names::monitor_toggle());
        }
    }

    fn select_cam(&mut self, ctx: &mut ShiftCtx, idx: usize) {
        let rooms = map().room_ids();
        if rooms.is_empty() {
            return;
        }
        let room = rooms[idx % rooms.len()].clone();
        if ctx.hud.monitor_room != room {
            ctx.hud.monitor_room = room;
            self.flash_s = 0.14;
            ctx.snd.cues(&cue_names::camera_switch());
        }
    }

    fn step_cam(&mut self, ctx: &mut ShiftCtx, delta: i32) {
        let n = map().cameras.len() as i32;
        if n == 0 {
            return;
        }
        let cur = map().camera_number(&ctx.hud.monitor_room) as i32 - 1;
        let next = (cur.max(0) + delta).rem_euclid(n);
        self.select_cam(ctx, next as usize);
    }

    fn sit(&mut self, ctx: &mut ShiftCtx) {
        self.sitting = true;
        ctx.player.teleport(SEAT, 0.0, -0.1);
        ctx.player.frozen = true;
    }

    fn stand(&mut self, ctx: &mut ShiftCtx) {
        self.sitting = false;
        let (yaw, pitch) = (ctx.player.cam.yaw, ctx.player.cam.pitch);
        ctx.player.teleport(SPAWN, yaw, pitch);
        ctx.player.frozen = false;
    }

    fn activate(&mut self, ctx: &mut ShiftCtx, id: &str) {
        match id {
            "doorL_door" => {
                self.toggle_door(ctx, 0);
                ctx.snd.cues(&cue_names::panel_button());
            }
            "doorR_door" => {
                self.toggle_door(ctx, 1);
                ctx.snd.cues(&cue_names::panel_button());
            }
            "lamp" => {
                if let Some(s) = self.scene.as_mut() {
                    let on = !s.office.state.lamp_on;
                    s.office.set_lamp(on);
                }
            }
            "chair" => self.sit(ctx),
            "monitor" => self.open_monitor(ctx),
            "breaker" | "ledger" => {
                let prompt = self.scene.as_ref().and_then(|s| s.office.station(id)).map(|s| s.prompt.clone()).unwrap_or_default();
                if !prompt.is_empty() {
                    ctx.hud.set_note(&prompt, 2.5);
                }
            }
            _ => {}
        }
    }

    // ---- what you see

    fn sync_visuals(&mut self, ctx: &mut ShiftCtx) {
        let dev = self.sim.dev;
        let sim = &self.sim;
        let devices = [dev.door_l, dev.door_r, dev.light_l, dev.light_r].iter().filter(|b| **b).count() as i32;
        ctx.hud.hour = hour_index(sim.elapsed);
        ctx.hud.power = sim.power as f32;
        ctx.hud.usage = devices;
        ctx.hud.night_label = format!("Week {}", self.week);
        let bars = ((sim.noise / 10.0) as i32).clamp(0, 10) as usize;
        ctx.hud.usage_text = Some(format!("${} / ${}   NOISE {}", sim.money, sim.need, "|".repeat(bars)));
        let Some(scene) = self.scene.as_mut() else { return };
        let o = &mut scene.office;
        o.set_light("doorL", dev.light_l);
        o.set_light("doorR", dev.light_r);
        if ctx.manual_clock {
            o.snap_slabs([dev.door_l, dev.door_r]);
        }
        o.set_button("doorL_door", dev.door_l);
        o.set_button("doorR_door", dev.door_r);
        o.set_button("doorL_light", dev.light_l);
        o.set_button("doorR_light", dev.light_r);
        if let Some(s) = o.station_mut("ledger") {
            s.prompt = format!("BUDGET LEDGER // ${} OF ${}", sim.money, sim.need);
        }
        for i in 0..4 {
            let item = sim.items.iter().find(|it| it.slot == i);
            let progress = item.map(|it| (it.progress / it.need.max(1.0)) as f32).unwrap_or(0.0);
            let view = item.map(|it| (it.kind.clone(), bar_segments(progress)));
            if let Some(s) = o.station_mut(&format!("rack{i}")) {
                match item {
                    Some(it) => {
                        s.disabled = false;
                        s.prompt = rack_prompt(&it.kind, it.value);
                    }
                    None => {
                        s.disabled = true;
                        s.prompt.clear();
                    }
                }
            }
            if view != self.slot_view[i] {
                set_slot(o, i, item.map(|it| (it.kind.as_str(), progress)));
                self.slot_view[i] = view;
            }
        }
    }

    /// Hunters: sim position and idle behaviour onto the actors.
    fn drive_actors(&mut self, dt_s: f32) {
        for h in self.hunters.iter_mut() {
            if let Some(agent) = self.stage.crowd.agent(h.id) {
                h.driver.drive(agent, &mut h.actor);
            }
            if dt_s > 0.0 {
                h.actor.animate(dt_s);
            }
        }
    }

    fn freeze_player(&mut self, ctx: &mut ShiftCtx) {
        self.work_slot = -1;
        self.sitting = false;
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.hud.monitor_up = false;
        ctx.hud.prompt.clear();
        ctx.hud.show_crosshair = false;
    }

    // ---- the end of a night

    fn finish_if_over(&mut self, ctx: &mut ShiftCtx) {
        if self.phase != Phase::Playing {
            return;
        }
        match self.sim.over {
            Over::None => {}
            Over::Win => self.finish_win(ctx),
            Over::Short => self.finish_short(ctx),
            Over::Caught => self.finish_caught(ctx),
        }
    }

    fn finish_win(&mut self, ctx: &mut ShiftCtx) {
        self.phase = Phase::Over;
        self.freeze_player(ctx);
        ctx.snd.cues(&cue_names::win());
        let money = self.sim.money;
        let need = self.sim.need;
        self.win_carry = if self.custom { 0 } else { carry_from(money, need) };
        if !self.custom && self.week < 5 {
            super::store_win_carry(self.week + 1, self.win_carry);
        }
        let score: i32 = ctx.session.custom_levels.values().map(|v| (*v).clamp(0, MAX_AI)).sum::<i32>() * POINTS_PER_LEVEL;
        self.ending = Ending::Win { money, need, score };
    }

    fn finish_short(&mut self, ctx: &mut ShiftCtx) {
        self.phase = Phase::Over;
        self.freeze_player(ctx);
        ctx.snd.cues(&cue_names::caught(ctx.save.short_death_scenes()));
        self.ending = Ending::Short { money: self.sim.money, need: self.sim.need };
    }

    fn finish_caught(&mut self, ctx: &mut ShiftCtx) {
        self.phase = Phase::Over;
        self.freeze_player(ctx);
        ctx.snd.cues(&cue_names::caught(ctx.save.short_death_scenes()));
        self.ending = Ending::Caught { by: self.sim.by.clone() };
    }

    // ---- sim events

    fn on_event(&mut self, ctx: &mut ShiftCtx, e: &DayEvent) {
        match e {
            DayEvent::Item { .. } => ctx.snd.cues(&cue_names::panel_button()),
            DayEvent::Salvage { value, .. } => {
                ctx.snd.cues(&cue_names::win());
                ctx.hud.set_note(&format!("+${value}"), 1.2);
            }
            DayEvent::Close { idx, dist } => {
                let id = VISITORS[(*idx).min(1)];
                ctx.snd.cues(&cue_names::office("footsteps", pan_of(id), 0.6 + 0.2 * (3 - *dist) as f32));
            }
            DayEvent::Repelled { idx } => ctx.snd.cues(&cue_names::office("scrape", pan_of(VISITORS[(*idx).min(1)]), 0.9)),
            DayEvent::Warned { idx } => ctx.snd.cues(&cue_names::office("presence", pan_of(VISITORS[(*idx).min(1)]), 0.8)),
            DayEvent::Knock => ctx.snd.cues(&cue_names::office("crimson_door", 0.0, 0.9)),
            DayEvent::Window => ctx.snd.cues(&cue_names::attack_warning(0.0)),
            DayEvent::Gone => ctx.snd.cues(&cue_names::office("scrape", 0.0, 0.6)),
            DayEvent::Out => {
                ctx.hud.call_text = "The power is out.".to_string();
                self.msg_s = 4.0;
                ctx.snd.cues(&cue_names::power_out());
            }
            _ => {}
        }
    }

    // ---- admin

    fn run_admin(&mut self, ctx: &mut ShiftCtx, id: &str) {
        let playing = self.phase == Phase::Playing;
        match id {
            "@win" => {
                if playing {
                    self.sim.over = Over::Win;
                    self.finish_if_over(ctx);
                }
                return;
            }
            "@lose" => {
                if playing {
                    self.sim.over = Over::Caught;
                    self.sim.by = "ravelle".to_string();
                    self.finish_if_over(ctx);
                }
                return;
            }
            _ => {}
        }
        if self.phase == Phase::Shop {
            return;
        }
        match id {
            "money_0" => self.sim.money = 0,
            "money_100" => self.sim.money = 100,
            "money_250" => self.sim.money = 250,
            "money_quota" => self.sim.money = self.sim.need,
            "quota_100" => self.sim.need = 100,
            "quota_210" => self.sim.need = 210,
            "quota_340" => self.sim.need = 340,
            "quota_9999" => self.sim.need = 9999,
            "rack_scrap" => {
                self.sim.add_piece("scrap");
            }
            "rack_parts" => {
                self.sim.add_piece("parts");
            }
            "rack_prize" => {
                self.sim.add_piece("prize");
            }
            "rack_clear" => self.sim.items.clear(),
            "upg_all" => self.sim.upgrades = tun().upgrades.iter().map(|u| u.id.clone()).collect(),
            "upg_none" => self.sim.upgrades.clear(),
            "noise_0" => self.sim.noise = 0.0,
            "noise_30" => self.sim.noise = 30.0,
            "noise_50" => self.sim.noise = 50.0,
            "noise_100" => self.sim.noise = 100.0,
            "doorL_shut" | "doorL_open" | "doorR_shut" | "doorR_open" => {
                if playing && !self.sim.out {
                    self.door_shut[if id.starts_with("doorL") { 0 } else { 1 }] = id.ends_with("_shut");
                }
            }
            "lightL_on" | "lightL_off" | "lightR_on" | "lightR_off" => {
                if playing {
                    self.light_held[if id.starts_with("lightL") { 0 } else { 1 }] = id.ends_with("_on") && !self.sim.out;
                }
            }
            "ysolde_near" => self.sim.place_ysolde("near"),
            "ysolde_window" => self.sim.place_ysolde("window"),
            "ysolde_far" => self.sim.place_ysolde("far"),
            "all_to_doors" => {
                self.sim.place_visitor(0, "near");
                self.sim.place_visitor(1, "near");
            }
            "toggle_freeze" => self.frozen = !self.frozen,
            "toggle_god" => self.god = !self.god,
            "toggle_infinite_power" => self.sim.infinite_power = !self.sim.infinite_power,
            "toggle_no_scare" => self.no_scare = !self.no_scare,
            "power_100" => {
                self.sim.power = 100.0;
                self.sim.out = false;
                self.sim.out_ms = 0.0;
            }
            "power_0" => self.sim.power = 0.0,
            "hour_3" => self.sim.set_hour(3),
            "hour_5" => self.sim.set_hour(5),
            "speed_1" => self.sim.hunter_speed = 1.0,
            "speed_2" => self.sim.hunter_speed = 2.0,
            _ => {}
        }
        self.dirty = true;
    }

    fn process_pending(&mut self, ctx: &mut ShiftCtx) {
        if self.pending.is_empty() && !self.dirty {
            return;
        }
        let ids = std::mem::take(&mut self.pending);
        for id in ids {
            self.run_admin(ctx, &id);
        }
        if self.dirty && self.phase != Phase::Shop {
            self.dirty = false;
            let face = ctx.player.feet;
            self.stage.after_step(&self.sim, face, ctx.manual_clock);
            self.sync_visuals(ctx);
        }
    }

    fn lights(&self, ctx: &ShiftCtx) -> Option<LightSet> {
        self.scene.as_ref().map(|s| day_lights(&s.office, ctx.player.flashlight_spot()))
    }
}

impl Mode for DayshiftMode {
    fn id(&self) -> &'static str {
        "dayshift"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        self.scene = Some(Scene::build(&ctx.gfx.tex));
        for id in CAST_IDS {
            if let Some(mut actor) = ctx.hunters.actor(id) {
                actor.visible = false;
                self.hunters.push(Hunter { id, actor, driver: ActorDriver::new() });
            }
        }
        ctx.player.teleport(SPAWN, 0.0, -0.1);
        ctx.player.footprint = Some((-2.5, -2.0, 2.5, 2.0));
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.hud.mode_text = format!("7. Salvage Run \u{b7} {}", if self.custom { "Custom Night".to_string() } else { format!("Week {}", self.week) });
        ctx.hud.hint = HINT.to_string();
        ctx.hud.night_label = format!("Week {}", self.week);
        ctx.hud.power = 100.0;
        ctx.hud.monitor_available = false;
        ctx.hud.cam_map = Some(cam_layout());
        ctx.hud.show_crosshair = false;
        ctx.hud.call = CallScript::empty();
        if let Some(first) = map().cameras.first() {
            ctx.hud.monitor_room = first.room.clone();
        }
        if ctx.manual_clock {
            let basket = bot_basket(self.week, self.shop.carry);
            self.begin_shift(ctx, basket);
        }
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        if self.phase != Phase::Playing {
            return;
        }
        let inp = ctx.input;
        // the monitor
        if inp.just(MONITOR) {
            if ctx.hud.monitor_up {
                self.close_monitor(ctx);
            } else {
                self.open_monitor(ctx);
            }
        } else if ctx.hud.monitor_up && inp.just(INTERACT) {
            self.close_monitor(ctx);
        }
        if ctx.hud.monitor_up {
            if inp.just(CAM_PREV) {
                self.step_cam(ctx, -1);
            } else if inp.just(CAM_NEXT) {
                self.step_cam(ctx, 1);
            }
            for n in 1..=8 {
                if inp.just(camera_action(n)) {
                    self.select_cam(ctx, n as usize - 1);
                }
            }
        }
        let monitor = ctx.hud.monitor_up;
        // the doors
        if inp.just(DOOR_LEFT) {
            self.toggle_door(ctx, 0);
        } else if inp.just(DOOR_RIGHT) {
            self.toggle_door(ctx, 1);
        }
        // the body
        ctx.player.frozen = monitor || self.sitting;
        ctx.player.look_frozen = monitor;
        let solids = self.scene.as_ref().map(|s| s.office.all_solids()).unwrap_or_default();
        ctx.player.update(inp, ctx.bindings, &solids, ctx.save.mouse_sens);
        if self.sitting && !monitor && (inp.is_down("move_forward") || inp.is_down("move_back") || inp.is_down("move_left") || inp.is_down("move_right")) {
            self.stand(ctx);
        }
        let aimed: Option<String> = match self.scene.as_ref() {
            Some(s) if !monitor => ctx.player.aim(&s.office.stations, &solids).map(|i| s.office.stations[i].id.clone()),
            _ => None,
        };
        ctx.hud.prompt = match (&aimed, self.scene.as_ref()) {
            (Some(id), Some(s)) => s.office.station(id).map(|st| st.prompt.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        if inp.just(INTERACT) && !monitor {
            if self.sitting {
                self.stand(ctx);
            } else if let Some(id) = aimed.clone() {
                self.activate(ctx, &id);
            }
        }
        // the lights are held (keys, or USE on a light button); the rack is held on a piece
        let down = inp.is_down(INTERACT);
        for i in 0..2 {
            let key = if i == 0 { LIGHT_LEFT } else { LIGHT_RIGHT };
            let button = if i == 0 { "doorL_light" } else { "doorR_light" };
            let want = inp.is_down(key) || (down && !monitor && aimed.as_deref() == Some(button));
            self.light_held[i] = want && !self.sim.out;
        }
        self.work_slot = -1;
        if down && !monitor && !self.sim.out {
            if let Some(id) = aimed.as_deref() {
                if let Some(n) = id.strip_prefix("rack") {
                    self.work_slot = n.parse::<i32>().unwrap_or(-1);
                }
            }
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.phase != Phase::Playing || self.frozen {
            return;
        }
        let input = self.current_input();
        let mut events = self.sim.step(dt_ms, &input);
        for ce in ctx.hud.call.update(self.sim.elapsed, dt_ms) {
            match ce {
                CallEvent::Ring => ctx.snd.cues(&cue_names::call_ring()),
                CallEvent::Talk => ctx.snd.cues(&cue_names::call_line()),
                _ => {}
            }
        }
        if self.god && self.sim.over != Over::None {
            self.sim.god_rescue();
            events.clear();
        }
        let hour = hour_index(self.sim.elapsed);
        if hour != self.hour_seen {
            self.hour_seen = hour;
            ctx.snd.cues(&cue_names::hour_change());
        }
        if self.sim.out {
            self.door_shut = [false; 2];
            self.light_held = [false; 2];
            self.work_slot = -1;
        }
        for e in &events {
            self.on_event(ctx, e);
        }
        let face = ctx.player.feet;
        self.stage.after_step(&self.sim, face, ctx.manual_clock);
        self.sync_visuals(ctx);
        self.finish_if_over(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        self.process_pending(ctx);
        self.monitor_seen = ctx.hud.monitor_up;
        if self.phase == Phase::Shop {
            return;
        }
        let snap = ctx.manual_clock;
        let dev = self.sim.dev;
        if !ctx.paused {
            self.flash_s = (self.flash_s - dt_s).max(0.0);
            if self.msg_s > 0.0 {
                self.msg_s -= dt_s;
                if self.msg_s <= 0.0 {
                    ctx.hud.call_text.clear();
                }
            }
            if !snap {
                for e in self.stage.tick(dt_s) {
                    if let BehaviourEvent::FirstStare { subtitle, .. } = e {
                        ctx.hud.set_note(&subtitle, 4.0);
                    }
                }
            }
            if let Some(scene) = self.scene.as_mut() {
                if snap {
                    scene.office.snap_slabs([dev.door_l, dev.door_r]);
                } else if self.phase == Phase::Playing {
                    scene.office.animate([dev.door_l, dev.door_r], dt_s);
                }
            }
        }
        // the soft static of a hunter staring into the camera that is up
        let pulse = if ctx.hud.monitor_up { self.stage.crowd.stare_pulse(&ctx.hud.monitor_room) } else { 0.0 };
        let soft = pulse / 0.35 * 0.14 * 0.28;
        ctx.hud.cam_flash_s = self.flash_s.max(soft);
        self.drive_actors(if ctx.paused { 0.0 } else { dt_s });
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        if ctx.hud.monitor_up {
            return;
        }
        let Some(lights) = self.lights(ctx) else { return };
        let gfx: &Gfx = &*ctx.gfx;
        ctx.player.cam.apply();
        clear_background(SKY);
        let cam = ctx.player.cam.pos;
        if let Some(scene) = self.scene.as_ref() {
            scene.draw_all(gfx, &lights, cam);
        }
        for h in &self.hunters {
            h.actor.draw(gfx, &lights, cam);
        }
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let m = map();
        let Some(cam) = m.camera(room).or_else(|| m.cameras.first()) else { return };
        let gfx: &Gfx = &*ctx.gfx;
        begin_feed(gfx, cam);
        let lights = feed_lights(room);
        if let Some(scene) = self.scene.as_ref() {
            scene.draw_all(gfx, &lights, cam.pos);
        }
        for h in &self.hunters {
            h.actor.draw_in(gfx, &lights, cam.pos, Actor::is_duct_room(room));
        }
        gfx.feed.end();
    }

    fn draw_overlay(&mut self, ctx: &mut ShiftCtx) {
        if self.phase != Phase::Shop {
            return;
        }
        match draw_shop(&mut self.shop, ctx.input, ctx.paused) {
            ShopAction::None => {}
            ShopAction::Toggled { .. } => ctx.snd.cues(&cue_names::menu("pick")),
            ShopAction::Denied => ctx.snd.cues(&cue_names::menu("deny")),
            ShopAction::Start => {
                ctx.snd.cues(&cue_names::menu("pick"));
                let basket = self.shop.owned.clone();
                self.begin_shift(ctx, basket);
            }
        }
    }

    fn outcome(&self) -> Outcome {
        if self.phase == Phase::Shop {
            Outcome::Running
        } else {
            self.sim.outcome()
        }
    }

    fn end_card(&self, _ctx: &ShiftCtx) -> Option<EndCard> {
        end_card_for(&self.ending, self.week, self.custom, self.win_carry)
    }

    fn killer(&self) -> Option<(String, String)> {
        match &self.ending {
            Ending::Caught { by } if !self.no_scare => Some((by.clone(), String::new())),
            _ => None,
        }
    }

    fn wants_mouse_free(&self) -> bool {
        self.phase == Phase::Shop || self.phase == Phase::Over || self.monitor_seen
    }

    fn admin_do(&mut self, id: &str) {
        self.pending.push(id.to_string());
    }

    fn debug_win(&mut self) {
        self.pending.push("@win".to_string());
    }

    fn debug_lose(&mut self) {
        self.pending.push("@lose".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_cards_use_the_godot_strings() {
        let win = end_card_for(&Ending::Win { money: 150, need: 140, score: 0 }, 2, false, 10).expect("card");
        assert_eq!(win.title, "WEEK 2 COMPLETE");
        assert_eq!(win.body, "6:00 AM\nQuota met: $150 of $140. $10 carries over to next week's shop.\nWeek 3 is open.");
        assert!(win.can_next);
        let last = end_card_for(&Ending::Win { money: 400, need: 380, score: 0 }, 5, false, 20).expect("card");
        assert_eq!(last.body, "6:00 AM\nQuota met: $400 of $380.\nYou cleared every week.");
        assert!(!last.can_next);
        let none = end_card_for(&Ending::Win { money: 140, need: 140, score: 0 }, 1, false, 0).expect("card");
        assert_eq!(none.body, "6:00 AM\nQuota met: $140 of $140.\nWeek 2 is open.");
        let custom = end_card_for(&Ending::Win { money: 340, need: 340, score: 300 }, 4, true, 0).expect("card");
        assert_eq!(custom.title, "CUSTOM NIGHT COMPLETE");
        assert_eq!(custom.body, format!("6:00 AM\nQuota met: $340 of $340.\nScore 300 of 600: {}", rating_of(300, 600)));
        assert!(!custom.can_next);
        let short = end_card_for(&Ending::Short { money: 120, need: 140 }, 1, false, 0).expect("card");
        assert_eq!(short.title, "QUOTA MISSED");
        assert_eq!(short.body, "$120 of $140 at 6 AM. The week is not cleared.");
        let caught = end_card_for(&Ending::Caught { by: "ysolde".into() }, 3, false, 0).expect("card");
        assert_eq!((caught.title.as_str(), caught.body.as_str()), ("CAUGHT", "Ysolde reached you in week 3."));
        assert!(end_card_for(&Ending::None, 1, false, 0).is_none());
    }

    #[test]
    fn rack_prompts_name_the_piece_and_its_worth() {
        assert_eq!(kind_label("scrap"), "Scrap");
        assert_eq!(rack_prompt("parts", 25), "PARTS $25 // HOLD TO SALVAGE");
        assert_eq!(rack_prompt("prize", 60), "PRIZE $60 // HOLD TO SALVAGE");
        assert_eq!(kind_label("odd"), "odd");
    }

    #[test]
    fn a_new_mode_starts_in_the_shop_with_the_weeks_carry() {
        let _g = super::super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Session::new();
        s.start_mode("dayshift", 3, Default::default(), Some(9));
        let m = DayshiftMode::new(&s);
        assert_eq!(m.phase_name(), "shop");
        assert_eq!(m.outcome(), Outcome::Running);
        assert_eq!(m.id(), "dayshift");
        assert_eq!(m.carry(), 0);
        assert!(m.wants_mouse_free());
        assert!(m.killer().is_none());
    }

    #[test]
    fn admin_requests_are_queued_for_the_next_frame() {
        let _g = super::super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Session::new();
        s.start_mode("dayshift", 1, Default::default(), Some(9));
        let mut m = DayshiftMode::new(&s);
        m.admin_do("money_quota");
        m.debug_win();
        m.debug_lose();
        assert_eq!(m.pending, vec!["money_quota".to_string(), "@win".to_string(), "@lose".to_string()]);
    }
}
