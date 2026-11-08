//! The Facility's shift scene (`story_game.gd`, `story_panel.gd`, `story_sounds.gd`, `story_admin.gd`, `story_controls.gd`): you
//! walk the five rooms with the shared first-person player (WASD, mouse, E to use) and do the night's task list at each room's
//! station. Walking through a door is the rules' `move`: it takes 1.8 s (3.6 crawling, hold C) and the player is carried through
//! the doorway on foot while the rules count it. Hold F for the beacon, H to hide under the desk in Control, hold E on the lift's
//! call button. Opalie (gallery), Porcelle (workshop, auditorium) and Quenelle (the lift) walk the map's nav graph and
//! idle between the rules' moves (stare at a camera, do a task). The six security cameras are watched from the monitor wall in
//! Control (Space).
//!
//! The rules are `sim::StoryLogic`; this file feeds it input, plays its events and draws.
#![allow(dead_code)]

use super::sim::{rules, Interact, LiftState, Options, Over, Puzzle, SimEvent, SimInput, StoryLogic, Who};
use super::world::{arrival, creeper_spot, door_point, layout, project, StoryWorld, BEHAVIOUR_JSON, NAV_JSON};
use crate::audio::cue_names as sfx;
use crate::core::{
    camera_action, rating_of, AdminEntry, CallEvent, CallScript, CamLayout, Cue, EndCard, HudPanel, Mode, Mulberry32, Outcome, Session, ShiftCtx,
};
use crate::gfx::{Gfx, LightSet, Spot};
use crate::hud::admin::{AdminAction, ModeAdmin};
use crate::hud::monitor::picture_rect;
use crate::hud::status::{c3, fill, stroke, tw, vtext_center, AMBER, DIM};
use crate::hunters::behaviour::{ActorDriver, BehaviourData, BehaviourEvent, Crowd};
use crate::hunters::nav::{NavMap, Pace};
use crate::hunters::place::yaw_toward;
use crate::hunters::pose_state::PoseState;
use crate::hunters::rig::{Actor, FeedView};
use crate::ui::theme::{k, ox, oy};
use crate::ui::{ButtonStyle, Ui};
use macroquad::prelude::*;
use std::rc::Rc;

/// The task list (virtual 480x270): small text in the clear band right of the NOISE / BEACON bars, above the monitor banks.
const TASK_X: f32 = 84.0;
const TASK_Y: f32 = 20.0;
const TASK_TEXT: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Playing,
    Won,
    Lost,
}

/// A walk through a door, as the player sees it: to the doorway first, then in.
#[derive(Clone, Debug)]
struct Glide {
    from: Vec3,
    mid: Vec3,
    to: Vec3,
    dur: f64,
    dir: char,
    room: String,
}

struct Slot {
    who: Who,
    actor: Option<Actor>,
    driver: ActorDriver,
}

/// Where a creeper was last sent: the same target is not sent twice.
#[derive(Clone, Debug, PartialEq)]
struct Track {
    room: String,
    dist: i32,
    side_neg: bool,
}

pub struct StoryGame {
    sim: StoryLogic,
    night: i32,
    is_custom: bool,
    seed: u32,
    state: State,
    world: Option<StoryWorld>,
    crowd: Option<Crowd>,
    slots: Vec<Slot>,
    held: SimInput,
    keypad_up: bool,
    monitor_open: bool,
    glide: Option<Glide>,
    /// Validators: crawl through the next door.
    crawl_override: bool,
    /// Which way each creeper's far end lies from where you came in (-1 or 1): Opalie, Porcelle.
    side: [f32; 2],
    porcelle_room: String,
    tracks: [Option<Track>; 2],
    enabled: [bool; 2],
    q_state: LiftState,
    /// Quenelle is walking in or out of the car (the lift opening or leaving).
    q_trip: bool,
    aim: Option<usize>,
    cam_idx: usize,
    frame: u32,
    pending_goto: Option<String>,
    admin_frozen: bool,
    admin_no_scare: bool,
    admin_noclip: bool,
}

/// The dim backing rectangle (x, y, w, h) under the task panels, or none when they are empty.
fn task_backing(panels: &[HudPanel]) -> Option<(f32, f32, f32, f32)> {
    let mut w = 0.0f32;
    let mut h = 0.0f32;
    for p in panels {
        for (s, _, size) in &p.lines {
            w = w.max(s.chars().count() as f32 * size * 0.45);
            h += size + 3.0;
        }
    }
    (w > 0.0).then(|| (TASK_X - 4.0, TASK_Y - 2.0, w + 8.0, h + 6.0))
}

fn title_case(s: &str) -> String {
    s.to_lowercase()
        .split(' ')
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

fn cam_layout() -> CamLayout {
    let l = layout();
    let base = CamLayout {
        rooms: l.map_pos.iter().map(|(r, p)| (r.clone(), *p)).collect(),
        edges: l.map_edges.clone(),
        entrances: l.entrances.clone(),
        cams: l.cameras.iter().map(|c| c.room.clone()).collect(),
        labels: l.cameras.iter().map(|c| (c.room.clone(), c.label.clone())).collect(),
        vents: Vec::new(),
        office: None,
    };
    crate::hud::cam_map::from_nav(&base, super::world::NAV_JSON, &[])
}

fn play(ctx: &mut ShiftCtx, list: Vec<Cue>) {
    ctx.snd.cues(&list);
}

impl StoryGame {
    pub fn new(session: &Session) -> StoryGame {
        let seed = session.seed_or_fresh();
        let mut s = session.clone();
        s.seed_value = seed;
        let l = layout();
        StoryGame {
            sim: StoryLogic::new(Options::from_session(&s)),
            night: if s.is_custom { 5 } else { s.night.clamp(1, 6) },
            is_custom: s.is_custom,
            seed,
            state: State::Playing,
            world: None,
            crowd: None,
            slots: Vec::new(),
            held: SimInput::default(),
            keypad_up: false,
            monitor_open: false,
            glide: None,
            crawl_override: false,
            side: [l.default_side(Who::Opalie), l.default_side(Who::Porcelle)],
            porcelle_room: "workshop".to_string(),
            tracks: [None, None],
            enabled: [false, false],
            q_state: LiftState::Away,
            q_trip: false,
            aim: None,
            cam_idx: 0,
            frame: 0,
            pending_goto: None,
            admin_frozen: false,
            admin_no_scare: false,
            admin_noclip: false,
        }
    }

    pub fn sim(&self) -> &StoryLogic {
        &self.sim
    }

    // ---- the hunters

    fn spawn_hunters(&mut self, ctx: &mut ShiftCtx) {
        let l = layout();
        let nav = Rc::new(NavMap::from_json(NAV_JSON).unwrap_or_else(|_| NavMap::new("story")));
        let data = BehaviourData::parse(BEHAVIOUR_JSON).map(Rc::new).unwrap_or_else(|_| BehaviourData::embedded());
        let mut crowd = Crowd::new(nav, data, self.seed);
        let look = ctx.player.feet;
        for who in Who::ALL {
            let (room, pos) = match who {
                Who::Opalie => ("gallery".to_string(), creeper_spot(who, "gallery", self.sim.opalie.dist, self.side[0])),
                Who::Porcelle => (self.porcelle_room.clone(), creeper_spot(who, &self.porcelle_room, self.sim.porcelle.dist, self.side[1])),
                Who::Quenelle => ("lift".to_string(), l.in_car),
            };
            crowd.add_at(who.id(), &room, pos, Some(look));
            let mut actor = ctx.hunters.actor_in(who.id(), "story");
            if let Some(a) = actor.as_mut() {
                a.visible = false;
                a.pos = pos;
            }
            self.slots.push(Slot { who, actor, driver: ActorDriver::new() });
        }
        self.crowd = Some(crowd);
    }

    /// Send the creepers to their spots, run Quenelle's lift trips, switch idle behaviour on and off, tick the crowd and pose the actors.
    fn step_hunters(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        let l = layout();
        let look = ctx.player.feet;
        let playing = self.state == State::Playing;
        let manual = ctx.manual_clock;
        let mut notes: Vec<String> = Vec::new();
        if let Some(crowd) = self.crowd.as_mut() {
            if playing {
                for (idx, who) in [Who::Opalie, Who::Porcelle].into_iter().enumerate() {
                    let room = if who == Who::Opalie { "gallery".to_string() } else { self.porcelle_room.clone() };
                    let dist = self.sim.creeper(who).dist;
                    let side = self.side[idx];
                    let spot = creeper_spot(who, &room, dist, side);
                    let on = self.sim.base_level(who) > 0;
                    let present = self.sim.room == room;
                    let enabled = on && !present && !manual && !self.admin_frozen;
                    if manual {
                        crowd.add_at(who.id(), &room, spot, Some(look));
                        continue;
                    }
                    let want = Track { room: room.clone(), dist, side_neg: side < 0.0 };
                    let was_enabled = self.enabled[idx];
                    let changed = self.tracks[idx].as_ref() != Some(&want) || (was_enabled && !enabled);
                    if changed {
                        let here = crowd.agent(who.id()).map(|a| a.walker.room().to_string()).unwrap_or_default();
                        if here != room {
                            crowd.send_to(who.id(), &room, spot, Some(look), Pace::Budget(2.5));
                        } else if let Some(a) = crowd.agent_mut(who.id()) {
                            a.brain.moved();
                            a.walker.go_points(&[spot], &room, spot, Some(look), Pace::Budget(1.2));
                        }
                        self.tracks[idx] = Some(want);
                    } else if !enabled {
                        crowd.set_final(who.id(), spot, Some(look));
                    }
                    crowd.set_enabled(who.id(), enabled);
                    self.enabled[idx] = enabled;
                }
                // Quenelle: out of the lift when it opens, back in when it leaves
                let state_now = self.sim.quenelle.state;
                let goal = if state_now == LiftState::Open { l.at_door } else { l.in_car };
                let q_look = l.at_door + vec3(0.0, 0.0, 3.0);
                if manual {
                    crowd.add_at(Who::Quenelle.id(), "lift", goal, Some(q_look));
                } else if state_now != self.q_state {
                    if let Some(a) = crowd.agent_mut(Who::Quenelle.id()) {
                        a.brain.moved();
                        a.walker.go_points(&[goal], "lift", goal, Some(q_look), Pace::Budget(l.q_trip_s));
                    }
                    self.q_trip = true;
                }
                self.q_state = state_now;
                if self.q_trip && crowd.agent(Who::Quenelle.id()).map_or(true, |a| a.walker.arrived()) {
                    self.q_trip = false;
                }
                let q_on = self.sim.base_level(Who::Quenelle) > 0 && state_now == LiftState::Away && !self.q_trip && !manual && !self.admin_frozen;
                crowd.set_enabled(Who::Quenelle.id(), q_on);
                if !ctx.paused && !manual && !self.admin_frozen {
                    for e in crowd.tick(dt_s) {
                        if let BehaviourEvent::FirstStare { subtitle, .. } = e {
                            notes.push(subtitle);
                        }
                    }
                }
            }
            // pose and place the actors
            for slot in &mut self.slots {
                let Some(actor) = slot.actor.as_mut() else { continue };
                if playing {
                    if let Some(agent) = crowd.agent(slot.who.id()) {
                        slot.driver.drive(agent, actor);
                    }
                }
                actor.animate(dt_s);
            }
        }
        for n in notes {
            ctx.hud.set_note(&n, 3.0);
        }
    }

    /// Which hunters are drawn: the player's view hides Quenelle in her car; a camera sees into it.
    fn draw_hunters(&mut self, gfx: &Gfx, lights: &LightSet, cam: Vec3, player_pass: bool) {
        let shown: Vec<bool> = self.slots.iter().map(|s| self.hunter_shown(s.who, player_pass)).collect();
        for (slot, on) in self.slots.iter_mut().zip(shown) {
            let Some(actor) = slot.actor.as_mut() else { continue };
            actor.visible = on;
            actor.draw(gfx, lights, cam);
        }
    }

    /// Is she drawn at all: on tonight's floor (a camera sees into the lift car; the player's view hides Quenelle in it), or the killer.
    fn hunter_shown(&self, who: Who, player_pass: bool) -> bool {
        let open = self.sim.quenelle.state == LiftState::Open;
        let base = self.sim.base_level(who) > 0;
        let killer = self.state == State::Lost && self.sim.by == Some(who);
        let on_floor = match who {
            Who::Opalie | Who::Porcelle => base,
            Who::Quenelle => base && (!player_pass || open || self.q_trip),
        };
        on_floor || killer
    }

    /// The view of a room's feed camera (what `draw_feed` renders and `feed_report` checks).
    fn feed_view(room: &str) -> Option<FeedView> {
        layout().camera(room).map(|c| FeedView::new(c.pos, c.target, c.fov))
    }

    /// Submit the hunters to the feed that is begun: the shown ones inside the view. Returns the ids submitted.
    fn draw_feed_hunters(&self, gfx: &Gfx, lights: &LightSet, view: &FeedView, room: &str) -> Vec<String> {
        let duct = Actor::is_duct_room(room);
        let mut out = Vec::new();
        for slot in &self.slots {
            let Some(actor) = slot.actor.as_ref() else { continue };
            if actor.draw_feed(gfx, lights, view, self.hunter_shown(slot.who, false), duct) {
                out.push(slot.who.id().to_string());
            }
        }
        out
    }

    // ---- what the player does

    fn deny(&self, ctx: &mut ShiftCtx) {
        play(ctx, sfx::office("control", 0.0, 0.3));
    }

    /// What pressing use on a station does. The text is shown as a note.
    fn station_pressed(&mut self, ctx: &mut ShiftCtx, id: &str) -> String {
        if self.state != State::Playing || self.keypad_up || self.glide.is_some() {
            return String::new();
        }
        if let Some(rest) = id.strip_prefix("door_") {
            let mut it = rest.rsplitn(2, '_');
            let dir = it.next().and_then(|d| d.chars().next());
            let room = it.next().unwrap_or("");
            if room == self.sim.room {
                if let Some(d) = dir {
                    self.try_move(ctx, d);
                }
            }
            return String::new();
        }
        if id == "terminal" {
            if self.sim.room == "control" && matches!(self.sim.puzzle, Puzzle::Code { .. }) {
                self.open_keypad();
            } else {
                self.deny(ctx);
                return "NOTHING TO KEY IN".to_string();
            }
            return String::new();
        }
        if id.starts_with("dial_") || id.starts_with("part_") || id.starts_with("switch_") {
            let i = id.split('_').nth(1).and_then(|s| s.parse::<i32>().ok()).unwrap_or(-1);
            return self.do_interact(ctx, Interact::index(i));
        }
        if id == "crank" {
            return self.do_interact(ctx, Interact::default());
        }
        if id == "monitor" {
            return match self.sim.task() {
                Some(t) => format!("NEXT: {} ({})", t.label, title_case(&rules().label(&t.room))),
                None => String::new(),
            };
        }
        String::new()
    }

    /// Start walking through a door (L, R or C): the rules count the walk, the player is carried through it.
    fn try_move(&mut self, ctx: &mut ShiftCtx, dir: char) -> bool {
        if self.state != State::Playing || self.sim.moving.is_some() {
            return false;
        }
        let crawl = self.crawl_override || (!ctx.manual_clock && ctx.input.is_down("crouch"));
        self.sim.set_crawl(crawl);
        let from = self.sim.room.clone();
        if !self.sim.move_dir(dir) {
            self.deny(ctx);
            return false;
        }
        let (to, dur) = match &self.sim.moving {
            Some(m) => (m.to.clone(), m.left),
            None => return false,
        };
        self.glide = Some(Glide { from: ctx.player.feet, mid: door_point(&from, dir), to: arrival(&to, dir), dur, dir, room: to });
        play(ctx, sfx::door(true));
        true
    }

    /// One action at the current task's station.
    fn do_interact(&mut self, ctx: &mut ShiftCtx, p: Interact) -> String {
        if self.state != State::Playing {
            return String::new();
        }
        let events = self.sim.interact(p);
        let mut msg = String::new();
        for e in &events {
            match e {
                SimEvent::Step(_) => play(ctx, sfx::panel_button()),
                SimEvent::Wrong(_) => {
                    play(ctx, sfx::office("warning", 0.0, 0.5));
                    msg = "WRONG // IT STARTS OVER".to_string();
                }
                SimEvent::Deny => self.deny(ctx),
                SimEvent::Task(_) => play(ctx, sfx::office("shift", 0.0, 0.8)),
                _ => {}
            }
        }
        self.check_over(ctx);
        msg
    }

    fn open_keypad(&mut self) {
        self.keypad_up = true;
    }

    fn close_keypad(&mut self) {
        self.keypad_up = false;
    }

    fn keypad_digit(&mut self, ctx: &mut ShiftCtx, d: u8) {
        if !self.keypad_up || d > 9 {
            return;
        }
        self.do_interact(ctx, Interact::digit((b'0' + d) as char));
        if !matches!(self.sim.puzzle, Puzzle::Code { .. }) || self.sim.over != Over::Running {
            self.close_keypad();
        }
    }

    fn set_monitor(&mut self, ctx: &mut ShiftCtx, on: bool) {
        if ctx.hud.monitor_up == on {
            return;
        }
        ctx.hud.monitor_up = on;
        self.monitor_open = on;
        ctx.hud.cam_flash_s = 0.14;
        play(ctx, sfx::monitor_toggle());
        if on {
            self.select_camera(ctx, self.cam_idx, false);
        }
    }

    fn select_camera(&mut self, ctx: &mut ShiftCtx, idx: usize, blip: bool) {
        let l = layout();
        let Some(cam) = l.cameras.get(idx) else { return };
        self.cam_idx = idx;
        ctx.hud.monitor_room = cam.room.clone();
        ctx.hud.cam_label = format!("CAM {}  {}", idx + 1, cam.label);
        ctx.hud.cam_flash_s = 0.14;
        if blip {
            play(ctx, sfx::camera_switch());
        }
    }

    fn handle_monitor(&mut self, ctx: &mut ShiftCtx) {
        let input = ctx.input;
        let l = layout();
        let available = self.state == State::Playing
            && self.sim.room == "control"
            && self.sim.moving.is_none()
            && !self.sim.hidden
            && !self.keypad_up
            && self.glide.is_none();
        ctx.hud.monitor_available = available;
        self.monitor_open = ctx.hud.monitor_up;
        if !available {
            if ctx.hud.monitor_up {
                self.set_monitor(ctx, false);
            }
            return;
        }
        if !ctx.hud.monitor_up {
            if input.just("nw_monitor") {
                self.set_monitor(ctx, true);
            }
            return;
        }
        // monitor up: the camera it shows follows the HUD's room (the --monitor flag sets it)
        if let Some(i) = l.camera_index(&ctx.hud.monitor_room) {
            if i != self.cam_idx || ctx.hud.cam_label.is_empty() {
                self.select_camera(ctx, i, false);
            }
        }
        if input.just("nw_monitor") || input.just("interact") {
            self.set_monitor(ctx, false);
            return;
        }
        let n = l.cameras.len().max(1);
        let mut idx = self.cam_idx;
        if input.just("nw_cam_next") {
            idx = (idx + 1) % n;
        }
        if input.just("nw_cam_prev") {
            idx = (idx + n - 1) % n;
        }
        for i in 0..n {
            if input.just(camera_action(i as i32 + 1)) {
                idx = i;
            }
        }
        if idx != self.cam_idx {
            self.select_camera(ctx, idx, true);
        } else if ctx.hud.cam_label.is_empty() {
            self.select_camera(ctx, idx, false);
        }
    }

    // ---- the end of a night

    fn check_over(&mut self, ctx: &mut ShiftCtx) {
        if self.state != State::Playing {
            return;
        }
        match self.sim.over {
            Over::Win => self.finish_win(ctx),
            Over::Caught => self.finish_caught(ctx),
            Over::Running => {}
        }
    }

    fn freeze_player(&mut self, ctx: &mut ShiftCtx) {
        self.keypad_up = false;
        ctx.hud.monitor_up = false;
        ctx.hud.monitor_available = false;
        self.monitor_open = false;
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        ctx.player.crouch = false;
    }

    fn finish_win(&mut self, ctx: &mut ShiftCtx) {
        self.state = State::Won;
        self.freeze_player(ctx);
        play(ctx, sfx::win());
    }

    fn finish_caught(&mut self, ctx: &mut ShiftCtx) {
        self.state = State::Lost;
        self.freeze_player(ctx);
        let short = ctx.save.short_death_scenes();
        play(ctx, sfx::caught(short));
        let who = self.sim.by.unwrap_or(Who::Opalie);
        let f = ctx.player.forward();
        let flat = vec3(f.x, 0.0, f.z).normalize_or_zero();
        let flat = if flat == Vec3::ZERO { vec3(0.0, 0.0, -1.0) } else { flat };
        let feet = ctx.player.feet;
        let pos = feet + flat * 1.1;
        let yaw = yaw_toward(pos, feet);
        for slot in &mut self.slots {
            if slot.who == who {
                if let Some(a) = slot.actor.as_mut() {
                    a.pos = pos;
                    a.yaw = yaw;
                    a.visible = true;
                    a.set_pose_state(PoseState::Attack);
                }
                slot.driver.reset();
            }
        }
    }

    // ---- events

    fn on_event(&mut self, ctx: &mut ShiftCtx, e: &SimEvent) {
        match e {
            SimEvent::Arrive(_) => {
                if let Some(g) = self.glide.take() {
                    let (yaw, pitch) = (ctx.player.cam.yaw, ctx.player.cam.pitch);
                    ctx.player.teleport(g.to, yaw, pitch);
                    self.arrived(g.dir, &g.room);
                }
                play(ctx, sfx::office("footsteps", 0.0, 0.4));
            }
            SimEvent::Close { id, away, .. } => play(ctx, sfx::office("footsteps", id.pan(), if *away { 0.5 } else { 0.9 })),
            SimEvent::Back { id, .. } => play(ctx, sfx::office("scrape", id.pan(), 0.6)),
            SimEvent::Pushed(id) => play(ctx, sfx::office("scrape", id.pan(), 0.9)),
            SimEvent::Alert(_) => play(ctx, sfx::office("presence", 0.8, 0.7)),
            SimEvent::Bell => {
                play(ctx, sfx::office("warning", 0.0, 1.0));
                ctx.hud.set_note("THE LIFT BELL", 2.5);
            }
            SimEvent::Open => play(ctx, sfx::office("crimson_door", 0.0, 1.0)),
            SimEvent::Leave => play(ctx, sfx::office("shift", 0.0, 0.6)),
            SimEvent::Task(_) => play(ctx, sfx::office("shift", 0.0, 0.8)),
            _ => {}
        }
    }

    /// You came into `room` through the door `dir` from the next room: which end of it each hunter comes from.
    fn arrived(&mut self, dir: char, room: &str) {
        if dir == 'C' {
            return;
        }
        let side = if dir == 'L' { -1.0 } else { 1.0 };
        if room == "gallery" {
            self.side[0] = side;
        } else if rules().porcelle_rooms.iter().any(|r| r == room) {
            self.side[1] = side;
            self.porcelle_room = room.to_string();
        }
    }

    // ---- what you see

    fn task_panels(&self) -> Vec<HudPanel> {
        let sim = &self.sim;
        let mut lines: Vec<(String, [f32; 3], f32)> = Vec::new();
        for (i, t) in sim.tasks.iter().enumerate() {
            let mark = if i < sim.task_idx {
                "x"
            } else if i == sim.task_idx {
                ">"
            } else {
                "-"
            };
            let col = if i < sim.task_idx {
                [0.54, 0.52, 0.45]
            } else if i == sim.task_idx {
                [0.96, 0.72, 0.28]
            } else {
                [0.93, 0.89, 0.8]
            };
            lines.push((format!("{} {}  ({})", mark, t.label, title_case(&rules().label(&t.room))), col, TASK_TEXT));
        }
        let mut tag = if sim.moving.is_some() { "WALKING".to_string() } else { rules().label(&sim.room) };
        if sim.hidden {
            tag = "HIDING".to_string();
        } else if sim.light {
            tag = "BEACON ON".to_string();
        } else if sim.working {
            tag = "HOLDING THE BUTTON".to_string();
        }
        let list_h = lines.len() as f32 * (TASK_TEXT + 3.0);
        vec![
            HudPanel { x: TASK_X, y: TASK_Y, lines },
            HudPanel { x: TASK_X, y: TASK_Y + list_h + 3.0, lines: vec![(tag, [0.8, 0.9, 1.0], TASK_TEXT)] },
        ]
    }

    /// Everything the shared HUD shows, from the rules; the puzzle on its station; the player carried through a door.
    fn sync(&mut self, ctx: &mut ShiftCtx) {
        let l = layout();
        if let Some(w) = self.world.as_mut() {
            w.show_puzzle(&self.sim.puzzle);
            w.set_lift_open(self.sim.quenelle.state == LiftState::Open);
        }
        let beacon_key = ctx.bindings.key_text("nw_fac_beacon");
        let hide_key = ctx.bindings.key_text("nw_fac_hide");
        let crawl_key = "C";
        let panels = self.task_panels();
        let staring: Vec<String> = match self.crowd.as_ref() {
            Some(c) => l.cameras.iter().filter(|cam| c.staring_at(&cam.room)).map(|cam| cam.room.clone()).collect(),
            None => Vec::new(),
        };
        let h = &mut *ctx.hud;
        h.hour = 0;
        h.power = 100.0;
        h.usage = 0;
        h.power_text = Some(String::new());
        h.usage_text = Some(String::new());
        h.clock_text = Some(String::new());
        h.night_label = if self.is_custom { "CUSTOM NIGHT".to_string() } else { format!("NIGHT {}", self.night) };
        h.show_crosshair = self.state == State::Playing && !h.monitor_up && !self.keypad_up;
        h.bars = vec![
            ("NOISE".to_string(), (self.sim.noise / 100.0) as f32, [0.9, 0.5, 0.2]),
            (format!("BEACON  {beacon_key}"), (self.sim.charge / 100.0) as f32, [1.0, 0.9, 0.5]),
        ];
        // the list sits in the clear band above the monitor banks, and never over the monitor screen itself
        h.panels = if h.monitor_up { Vec::new() } else { panels };
        h.cam_map = Some(cam_layout());
        h.cam_pulse = staring;
        h.hint = if h.monitor_up {
            "LEFT / RIGHT OR 1-6 SWITCH CAMERA   SPACE OR E CLOSES THE MONITOR".to_string()
        } else {
            format!("WASD move   E use (hold E on the lift button)   hold {beacon_key} beacon   hold {hide_key} hide in Control   hold {crawl_key} through a door to crawl   P pause")
        };
        if self.state != State::Playing || self.keypad_up || self.glide.is_some() || h.monitor_up {
            h.prompt.clear();
        }
        // while the rules count a walk through a door, the player is carried through it
        if let (Some(g), Some(m)) = (&self.glide, &self.sim.moving) {
            let t = (1.0 - m.left / g.dur.max(1.0)).clamp(0.0, 1.0) as f32;
            let pos = if t < 0.45 { g.from.lerp(g.mid, t / 0.45) } else { g.mid.lerp(g.to, (t - 0.45) / 0.55) };
            let p = vec3(pos.x, ctx.player.feet.y, pos.z);
            let (yaw, pitch) = (ctx.player.cam.yaw, ctx.player.cam.pitch);
            ctx.player.teleport(p, yaw, pitch);
        }
    }

    /// The beacon's light: Godot's SpotLight3D at (0.2, 1.5, -0.2) from the body, pointing where it looks.
    fn beacon_spot(&self, ctx: &ShiftCtx) -> Option<Spot> {
        if !self.sim.light {
            return None;
        }
        let l = layout();
        let f = ctx.player.forward();
        let flat = vec3(f.x, 0.0, f.z).normalize_or_zero();
        let pos = ctx.player.feet + vec3(0.0, 1.5, 0.0) + ctx.player.right() * l.beacon_offset.x + flat * l.beacon_offset.z;
        Some(Spot {
            pos,
            dir: f,
            color: l.beacon_color,
            energy: l.beacon_energy,
            range: l.beacon_range,
            cos_outer: l.beacon_half_deg.to_radians().cos(),
        })
    }

    /// The note's code on the desk and the breakers' numbers, drawn where they are in the view.
    fn draw_world_labels(&self, ctx: &ShiftCtx) {
        let l = layout();
        let cam = ctx.player.cam;
        let aspect = screen_width() / screen_height().max(1.0);
        let to_virtual = |s: Vec2| vec2((s.x - ox()) / k(), (s.y - oy()) / k());
        if let Puzzle::Code { code, .. } = &self.sim.puzzle {
            let p = l.centre("control") + vec3(-0.2, 0.84, 1.9);
            let dist = cam.pos.distance(p);
            if dist < 3.6 {
                if let Some(s) = project(&cam, aspect, p) {
                    let v = to_virtual(s);
                    let text = code.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
                    let size = (14.0 / dist.max(0.6)).clamp(5.0, 18.0);
                    let w = tw(&text, size);
                    fill(Rect::new(v.x - w * 0.5 - 2.0, v.y - size * 0.5 - 1.0, w + 4.0, size + 2.0), Color::new(0.95, 0.93, 0.82, 0.92));
                    vtext_center(&text, v.x, v.y - size * 0.5, size, Color::new(0.1, 0.08, 0.05, 1.0));
                }
            }
        }
        if let Puzzle::Switches { n, order, .. } = &self.sim.puzzle {
            let c = l.centre("breaker");
            for i in 0..*n {
                let p = c + vec3(-1.9 + i as f32 * 0.76, 1.95, -l.room_d * 0.5 + 0.35);
                let dist = cam.pos.distance(p);
                if dist > 6.0 {
                    continue;
                }
                if let Some(s) = project(&cam, aspect, p) {
                    let v = to_virtual(s);
                    let rank = order.iter().position(|o| *o == i).unwrap_or(0) + 1;
                    let size = (12.0 / dist.max(0.8)).clamp(5.0, 14.0);
                    vtext_center(&rank.to_string(), v.x, v.y - size * 0.5, size, AMBER);
                }
            }
        }
    }

    fn draw_keypad(&mut self, ctx: &mut ShiftCtx) {
        let panel = Rect::new(195.0, 66.0, 90.0, 126.0);
        fill(panel, Color::new(0.05, 0.04, 0.07, 0.94));
        stroke(panel, AMBER, 1.0);
        let shown = match &self.sim.puzzle {
            Puzzle::Code { n, entered, .. } => format!("{}{}", entered, "_".repeat((*n as usize).saturating_sub(entered.chars().count()))),
            _ => String::new(),
        };
        vtext_center(&shown, 240.0, 72.0, 12.0, Color::new(0.6, 1.0, 0.7, 1.0));
        let mut ui = Ui::begin(ctx.input);
        let rows = [["1", "2", "3"], ["4", "5", "6"], ["7", "8", "9"], ["", "0", ""]];
        let mut pressed: Option<u8> = None;
        for (r, row) in rows.iter().enumerate() {
            for (c, label) in row.iter().enumerate() {
                if label.is_empty() {
                    continue;
                }
                let rect = Rect::new(201.0 + c as f32 * 27.0, 92.0 + r as f32 * 19.0, 24.0, 16.0);
                if ui.button(rect, label, 8.0, ButtonStyle::Normal, true).clicked {
                    pressed = label.parse::<u8>().ok();
                }
            }
        }
        vtext_center("KEY IT IN  /  E LEAVES", 240.0, 176.0, 5.0, DIM);
        if let Some(d) = pressed {
            self.keypad_digit(ctx, d);
        }
    }
}

impl Mode for StoryGame {
    fn id(&self) -> &'static str {
        "story"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        let l = layout();
        self.world = Some(StoryWorld::new(&ctx.gfx.tex));
        ctx.hud.monitor_available = false;
        ctx.hud.show_crosshair = true;
        ctx.hud.cam_map = Some(cam_layout());
        ctx.hud.call = if self.is_custom { CallScript::empty() } else { CallScript::for_level("story", ctx.session.night) };
        ctx.hud.call_text.clear();
        ctx.player.footprint = None;
        ctx.player.teleport(l.player_start, l.player_yaw, l.player_pitch);
        self.spawn_hunters(ctx);
        self.select_camera(ctx, 0, false);
        ctx.hud.cam_flash_s = 0.0;
        self.sync(ctx);
        if let Some(w) = self.world.as_mut() {
            w.snap();
        }
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        self.frame = self.frame.wrapping_add(1);
        let input = ctx.input;
        if self.state != State::Playing {
            ctx.player.frozen = true;
            ctx.player.look_frozen = true;
            return;
        }
        // admin: go to a room
        if let Some(room) = self.pending_goto.take() {
            self.sim.room = room.clone();
            self.sim.moving = None;
            self.glide = None;
            self.sim.hidden = false;
            let dir = if room == "lift" || room == "control" { 'C' } else { 'L' };
            let (yaw, pitch) = (ctx.player.cam.yaw, ctx.player.cam.pitch);
            ctx.player.teleport(arrival(&room, dir), yaw, pitch);
        }
        if input.just("office_radio") && ctx.hud.call.mute() {
            ctx.hud.zellia_muted = true;
        }
        let keypad_was_up = self.keypad_up;
        self.held.light = input.is_down("nw_fac_beacon");
        self.held.hide = input.is_down("nw_fac_hide");
        if self.keypad_up {
            if let Some(d) = input.digit {
                self.keypad_digit(ctx, d);
            }
            if input.just("interact") {
                self.close_keypad();
            }
        }
        let monitor_was_up = ctx.hud.monitor_up;
        self.handle_monitor(ctx);
        // the player: frozen while a door carries them, at the keypad, at the monitor, or hiding
        let walking_free = !self.keypad_up && self.glide.is_none() && !ctx.hud.monitor_up && !self.held.hide;
        ctx.player.frozen = !walking_free;
        ctx.player.look_frozen = self.keypad_up;
        ctx.player.crouch = self.sim.hidden;
        ctx.player.noclip = self.admin_noclip;
        if let Some(w) = self.world.as_ref() {
            ctx.player.update(input, ctx.bindings, &w.solids, ctx.save.mouse_sens);
        }
        // what is under the crosshair
        self.aim = None;
        let mut prompt = String::new();
        let mut aimed_id: Option<String> = None;
        if !self.keypad_up && self.glide.is_none() && !ctx.hud.monitor_up {
            if let Some(w) = self.world.as_ref() {
                self.aim = ctx.player.aim(&w.stations, &w.world.solids);
                if let Some(s) = self.aim.and_then(|i| w.stations.get(i)) {
                    prompt = s.prompt.clone();
                    aimed_id = Some(s.id.clone());
                }
            }
        }
        ctx.hud.prompt = prompt;
        self.held.work = input.is_down("interact") && aimed_id.as_deref() == Some("lift_button");
        if input.just("interact") && !monitor_was_up && !keypad_was_up {
            if let Some(id) = aimed_id {
                let msg = self.station_pressed(ctx, &id);
                if !msg.is_empty() {
                    ctx.hud.set_note(&msg, 2.5);
                }
            }
        }
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.state != State::Playing || self.admin_frozen {
            return;
        }
        let events = self.sim.step(dt_ms, self.held);
        for e in ctx.hud.call.update(self.sim.elapsed, dt_ms) {
            match e {
                CallEvent::Ring => play(ctx, sfx::call_ring()),
                CallEvent::Talk => play(ctx, sfx::call_line()),
                CallEvent::Open | CallEvent::Close => {}
            }
        }
        for e in &events {
            self.on_event(ctx, e);
        }
        self.check_over(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        if let Some(w) = self.world.as_mut() {
            if ctx.manual_clock {
                w.snap();
            } else {
                w.animate(dt_s);
            }
        }
        if ctx.hud.cam_flash_s > 0.0 {
            ctx.hud.cam_flash_s = (ctx.hud.cam_flash_s - dt_s).max(0.0);
        }
        self.sync(ctx);
        self.step_hunters(ctx, dt_s);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
        let cam = ctx.player.cam;
        cam.apply();
        let beacon = self.beacon_spot(ctx);
        let lights = match self.world.as_ref() {
            Some(w) => {
                let lights = w.lights_at(cam.pos, beacon);
                w.draw(&*ctx.gfx, &lights, cam.pos);
                lights
            }
            None => return,
        };
        self.draw_hunters(&*ctx.gfx, &lights, cam.pos, true);
        // a dim backing under the task list so it reads over the bright monitors (the HUD text is drawn after this)
        set_default_camera();
        if !ctx.hud.monitor_up {
            if let Some((x, y, w, h)) = task_backing(&self.task_panels()) {
                fill(Rect::new(x, y, w, h), Color::new(0.02, 0.03, 0.05, 0.72));
            }
        }
    }

    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {
        let l = layout();
        let Some(cam) = l.camera(room) else { return };
        set_camera(&Camera3D {
            position: cam.pos,
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
        let lights = LightSet::feed();
        if let Some(w) = self.world.as_ref() {
            w.draw(&*ctx.gfx, &lights, cam.pos);
        }
        let view = FeedView::new(cam.pos, cam.target, cam.fov);
        self.draw_feed_hunters(&*ctx.gfx, &lights, &view, room);
        set_default_camera();
    }

    fn feed_report(&self, room: &str) -> Option<(Vec<String>, Vec<String>)> {
        let crowd = self.crowd.as_ref()?;
        let view = Self::feed_view(room)?;
        let duct = Actor::is_duct_room(room);
        // in the room's view and on tonight's floor; "drawn" is exactly what `draw_feed_hunters` submits
        let inside: Vec<String> = crowd
            .visible_in(room)
            .iter()
            .map(|a| a.id.clone())
            .filter(|id| self.slots.iter().any(|s| s.who.id() == id.as_str() && self.hunter_shown(s.who, false)))
            .collect();
        let drawn = inside
            .iter()
            .filter(|id| {
                self.slots.iter().any(|s| s.who.id() == id.as_str() && s.actor.as_ref().map_or(false, |a| a.feed_drawn(self.hunter_shown(s.who, false), duct, &view)))
            })
            .cloned()
            .collect();
        Some((inside, drawn))
    }

    fn tour_protect(&mut self) {
        // the early nights keep part of the cast off the floor: the tour needs all three on it
        for who in Who::ALL {
            if self.sim.base_level(who) <= 0 {
                self.sim.levels.insert(who.id().to_string(), 6);
            }
        }
        self.sim.god = true;
        self.sim.infinite_beacon = true;
        self.admin_no_scare = true;
        self.sim.elapsed = crate::core::tour_clock_wrap(self.sim.elapsed);
    }

    fn draw_overlay(&mut self, ctx: &mut ShiftCtx) {
        if self.state == State::Playing && !ctx.hud.monitor_up {
            self.draw_world_labels(ctx);
        }
        // Opalie's hearing line on the noise bar (the HUD draws the bar: x 8..78, y 50..54) and the lift's lamp under the night label
        let hear = self.sim.tune.hear as f32;
        fill(Rect::new(8.0 + 70.0 * (hear / 100.0), 48.0, 1.0, 8.0), Color::new(1.0, 0.2, 0.2, 1.0));
        let lamp = match self.sim.quenelle.state {
            LiftState::Bell => c3([1.0, 0.8, 0.2]),
            LiftState::Open => c3([1.0, 0.3, 0.2]),
            LiftState::Away => c3([0.2, 0.2, 0.2]),
        };
        if self.sim.base_level(Who::Quenelle) > 0 {
            fill(Rect::new(8.0, 40.0, 10.0, 3.0), lamp);
        }
        // a stare into the feed: a soft static pulse over the picture
        if ctx.hud.monitor_up {
            if let Some(c) = self.crowd.as_ref() {
                let pulse = c.stare_pulse(&ctx.hud.monitor_room);
                if pulse > 0.01 && !ctx.save.reduced_motion() {
                    let pic = picture_rect();
                    let mut rng = Mulberry32::new(self.frame.wrapping_mul(2_654_435_761));
                    fill(pic, Color::new(0.75, 0.8, 0.76, pulse * 0.55));
                    for _ in 0..6 {
                        let y = pic.y + rng.next() as f32 * pic.h;
                        fill(Rect::new(pic.x, y, pic.w, 0.6 + rng.next() as f32 * 1.6), Color::new(0.9, 0.92, 0.9, pulse * 0.7));
                    }
                }
            }
        }
        if self.keypad_up && self.state == State::Playing {
            self.draw_keypad(ctx);
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
                let more = ctx.session.has_next(ctx.modes);
                let mut body = "Every task on the list is done.".to_string();
                if self.is_custom {
                    let score: i32 = ctx.session.custom_levels.values().map(|v| (*v).clamp(0, 20)).sum::<i32>() * 10;
                    body.push_str(&format!("\nScore {}  {}  (best {})", score, rating_of(score, 400), ctx.save.best_score("story")));
                } else if more {
                    body.push_str(&format!("\nNight {} is open.", self.night + 1));
                } else {
                    body.push_str("\nYou cleared every night.");
                }
                let title = if self.is_custom { "CUSTOM NIGHT COMPLETE".to_string() } else { format!("NIGHT {} COMPLETE", self.night) };
                Some(EndCard { title, body, can_next: more })
            }
            State::Lost => {
                let who = self.sim.by.unwrap_or(Who::Opalie);
                let room = rules().label(&self.sim.room).to_lowercase();
                let line = if who == Who::Quenelle {
                    "Quenelle saw you when the lift opened.".to_string()
                } else {
                    format!("{} got to you in the {}.", who.name(), room)
                };
                Some(EndCard { title: "CAUGHT".to_string(), body: line, can_next: false })
            }
        }
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.state == State::Lost {
            self.sim.by.map(|w| (w.id().to_string(), String::new()))
        } else {
            None
        }
    }

    fn wants_mouse_free(&self) -> bool {
        self.keypad_up || self.monitor_open
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        let mut out = vec![AdminEntry::heading("JUMP TO TASK")];
        for (i, t) in self.sim.tasks.iter().enumerate() {
            out.push(AdminEntry::button(&format!("TASK {}: {}", i + 1, t.label.to_uppercase()), &format!("task_{i}")));
        }
        out
    }

    fn admin_do(&mut self, id: &str) {
        let sim = &mut self.sim;
        match id {
            "noise_0" => sim.noise = 0.0,
            "noise_25" => sim.noise = 25.0,
            "noise_50" => sim.noise = 50.0,
            "noise_80" => sim.noise = 80.0,
            "noise_100" => sim.noise = 100.0,
            "force_task" => {
                if sim.over == Over::Running && sim.task_idx < sim.tasks.len() {
                    let mut ev = Vec::new();
                    sim.complete_task(&mut ev);
                    if sim.over == Over::Win {
                        self.state = State::Won;
                    }
                }
            }
            "lift_away" | "lift_bell" | "lift_open" => {
                let q = &mut sim.quenelle;
                q.state = match id {
                    "lift_bell" => LiftState::Bell,
                    "lift_open" => LiftState::Open,
                    _ => LiftState::Away,
                };
                q.t = 0.0;
                q.expose = 0.0;
                q.clock = 0.0;
                q.gap = -1.0;
            }
            "toggle_frozen" => self.admin_frozen = !self.admin_frozen,
            "toggle_god" => sim.god = !sim.god,
            "toggle_inf_beacon" => sim.infinite_beacon = !sim.infinite_beacon,
            "toggle_no_scare" => self.admin_no_scare = !self.admin_no_scare,
            "toggle_noclip" => self.admin_noclip = !self.admin_noclip,
            "speed_0.5" => sim.hunter_speed = 0.5,
            "speed_1" => sim.hunter_speed = 1.0,
            "speed_2" => sim.hunter_speed = 2.0,
            "speed_4" => sim.hunter_speed = 4.0,
            "beacon_full" => sim.charge = 100.0,
            "beacon_empty" => sim.charge = 0.0,
            other => {
                if let Some(i) = other.strip_prefix("task_").and_then(|s| s.parse::<usize>().ok()) {
                    sim.jump_to_task(i);
                } else if let Some(room) = other.strip_prefix("room_") {
                    if rules().room(room).is_some() {
                        self.pending_goto = Some(room.to_string());
                    }
                } else if let Some((name, steps)) = other.split_once("_steps_") {
                    if let (Some(who), Ok(d)) = (Who::from_id(name), steps.parse::<i32>()) {
                        if who != Who::Quenelle {
                            let h = sim.creeper_mut(who);
                            h.dist = d.clamp(0, 3);
                            h.strike = 0.0;
                        }
                    }
                }
            }
        }
    }

    fn debug_win(&mut self) {
        if self.state == State::Playing {
            self.sim.over = Over::Win;
            self.state = State::Won;
        }
    }

    fn debug_lose(&mut self) {
        if self.state == State::Playing {
            self.sim.over = Over::Caught;
            self.sim.by = Some(Who::Opalie);
            self.state = State::Lost;
        }
    }
}

/// The Facility's admin panel (`story_admin.gd` and the generic hooks `StoryGame` offered): the noise meter, the current
/// task, go to any room, the lift, god mode, hunter speed, the beacon, the creepers' steps. Ids are handled by `StoryGame::admin_do`
/// (the jump-to-task rows come from `Mode::admin_entries`, because they carry the night's real task names).
pub struct StoryAdmin;

impl ModeAdmin for StoryAdmin {
    fn mode_id(&self) -> &'static str {
        "story"
    }

    fn title(&self) -> String {
        "THE FACILITY SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        let mut out = vec![
            AdminAction::new("NOISE: SILENT", "noise_0"),
            AdminAction::new("NOISE: 25 (A DIAL CLICK)", "noise_25"),
            AdminAction::new("NOISE: 50", "noise_50"),
            AdminAction::new("NOISE: 80 (LOUD)", "noise_80"),
            AdminAction::new("NOISE: 100 (EVERYONE HEARS)", "noise_100"),
            AdminAction::new("FORCE THE CURRENT TASK DONE", "force_task"),
        ];
        for r in &rules().rooms {
            out.push(AdminAction::new(&format!("GO TO {}", r.label), &format!("room_{}", r.id)));
        }
        out.extend([
            AdminAction::new("SEND THE LIFT AWAY", "lift_away"),
            AdminAction::new("RING THE BELL", "lift_bell"),
            AdminAction::new("OPEN THE LIFT (QUENELLE)", "lift_open"),
            AdminAction::new("TOGGLE: FREEZE THE NIGHT", "toggle_frozen"),
            AdminAction::new("TOGGLE: GOD MODE", "toggle_god"),
            AdminAction::new("TOGGLE: INFINITE BEACON", "toggle_inf_beacon"),
            AdminAction::new("TOGGLE: NO JUMPSCARE", "toggle_no_scare"),
            AdminAction::new("TOGGLE: NOCLIP", "toggle_noclip"),
            AdminAction::new("HUNTER SPEED 0.5X", "speed_0.5"),
            AdminAction::new("HUNTER SPEED 1X", "speed_1"),
            AdminAction::new("HUNTER SPEED 2X", "speed_2"),
            AdminAction::new("HUNTER SPEED 4X", "speed_4"),
            AdminAction::new("BEACON FULL", "beacon_full"),
            AdminAction::new("BEACON EMPTY", "beacon_empty"),
        ]);
        for who in ["opalie", "porcelle"] {
            for d in 0..4 {
                out.push(AdminAction::new(&format!("{} {} STEPS", who.to_uppercase(), d), &format!("{who}_steps_{d}")));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(night: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("story", night, Default::default(), Some(seed));
        s
    }

    #[test]
    fn a_hunter_at_a_stand_spot_is_in_that_cameras_feed() {
        let l = layout();
        let mut a = crate::hunters::rig::tests::test_actor();
        let mut spots = vec![("gallery", creeper_spot(Who::Opalie, "gallery", 3, -1.0)), ("lift", l.in_car), ("lift", l.at_door)];
        for room in rules().porcelle_rooms.iter() {
            spots.push((room.as_str(), creeper_spot(Who::Porcelle, room, 3, 1.0)));
        }
        for (room, pos) in spots {
            let Some(view) = StoryGame::feed_view(room) else { continue };
            a.pos = pos;
            assert!(a.feed_drawn(true, false, &view), "{room} at {pos:?}");
        }
        assert!(StoryGame::feed_view("workshop").is_some());
    }

    #[test]
    fn the_tour_puts_all_three_on_the_floor() {
        let mut g = StoryGame::new(&session(1, 5));
        g.tour_protect();
        for who in Who::ALL {
            assert!(g.sim.base_level(who) > 0, "{who:?}");
        }
    }

    #[test]
    fn a_game_builds_without_a_window() {
        let g = StoryGame::new(&session(3, 5));
        assert_eq!(g.id(), "story");
        assert_eq!(g.outcome(), Outcome::Running);
        assert_eq!(g.night, 3);
        assert!(!g.wants_mouse_free());
        assert!(g.killer().is_none());
        assert_eq!(g.side, [-1.0, 1.0]);
    }

    #[test]
    fn a_custom_night_is_night_five_with_the_sliders() {
        let mut s = Session::new();
        let mut sliders = std::collections::BTreeMap::new();
        sliders.insert("opalie".to_string(), 12);
        sliders.insert("porcelle".to_string(), 0);
        sliders.insert("quenelle".to_string(), 0);
        s.start_mode("story", 1, sliders, Some(9));
        let g = StoryGame::new(&s);
        assert!(g.is_custom);
        assert_eq!(g.night, 5);
        assert_eq!(g.sim.base_level(Who::Opalie), 12);
        assert_eq!(g.sim.tasks.len(), 7);
    }

    #[test]
    fn debug_win_and_lose_decide_the_outcome() {
        let mut g = StoryGame::new(&session(2, 5));
        g.debug_win();
        assert_eq!(g.outcome(), Outcome::Won);
        let mut g = StoryGame::new(&session(2, 5));
        g.debug_lose();
        assert_eq!(g.outcome(), Outcome::Lost);
        assert_eq!(g.killer(), Some(("opalie".to_string(), String::new())));
    }

    #[test]
    fn admin_ids_change_the_rules() {
        let mut g = StoryGame::new(&session(4, 5));
        g.admin_do("noise_80");
        assert_eq!(g.sim.noise, 80.0);
        g.admin_do("lift_open");
        assert_eq!(g.sim.quenelle.state, LiftState::Open);
        g.admin_do("lift_away");
        assert_eq!(g.sim.quenelle.state, LiftState::Away);
        g.admin_do("opalie_steps_1");
        assert_eq!(g.sim.opalie.dist, 1);
        g.admin_do("porcelle_steps_0");
        assert_eq!(g.sim.porcelle.dist, 0);
        g.admin_do("task_2");
        assert_eq!(g.sim.task_idx, 2);
        g.admin_do("toggle_god");
        assert!(g.sim.god);
        g.admin_do("speed_2");
        assert_eq!(g.sim.hunter_speed, 2.0);
        g.admin_do("room_workshop");
        assert_eq!(g.pending_goto.as_deref(), Some("workshop"));
        g.admin_do("room_nowhere");
        assert_eq!(g.pending_goto.as_deref(), Some("workshop"), "an unknown room changes nothing");
        g.admin_do("force_task");
        assert_eq!(g.sim.task_idx, 3);
    }

    #[test]
    fn the_panel_lists_the_rows_the_game_handles() {
        let mut g = StoryGame::new(&session(5, 5));
        let panel = StoryAdmin.actions();
        assert!(panel.len() > 30);
        assert!(panel.iter().any(|a| a.id == "room_lift"));
        assert!(panel.iter().any(|a| a.id == "opalie_steps_3"));
        // every row runs without trouble
        for a in &panel {
            let mut fresh = StoryGame::new(&session(5, 5));
            fresh.admin_do(&a.id);
        }
        let entries = g.admin_entries();
        assert_eq!(entries.len(), 1 + g.sim.tasks.len());
        assert!(entries[0].heading);
        assert!(entries[1].label.starts_with("TASK 1: LOG IN"));
        g.admin_do(&entries[3].id);
        assert_eq!(g.sim.task_idx, 2);
    }

    #[test]
    fn the_panel_registers_and_runs_through_the_registry() {
        let mut reg = crate::hud::admin::AdminRegistry::new();
        reg.register(Box::new(StoryAdmin));
        let mut g = StoryGame::new(&session(2, 5));
        let rows = reg.entries(&mut g);
        assert!(rows.iter().any(|e| e.heading && e.label == "THE FACILITY SPECIFIC"));
        assert!(rows.iter().any(|e| e.id == "task_0"));
        reg.run("noise_50", &mut g);
        assert_eq!(g.sim.noise, 50.0);
        reg.run("@win", &mut g);
        assert_eq!(g.outcome(), Outcome::Won);
    }

    #[test]
    fn title_case_reads_like_godots_capitalize() {
        assert_eq!(title_case("BREAKER ROOM"), "Breaker Room");
        assert_eq!(title_case("CONTROL"), "Control");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn the_camera_map_has_six_cameras_in_key_order() {
        let c = cam_layout();
        assert_eq!(c.cams, vec!["breaker", "gallery", "control", "workshop", "auditorium", "lift"]);
        assert_eq!(c.rooms.len(), 6);
        assert_eq!(c.edges.len(), 5);
        assert!(c.labels.iter().any(|(r, l)| r == "lift" && l == "LIFT"));
        assert_eq!(c.entrances, vec!["gallery", "workshop", "lift"]);
    }

    #[test]
    fn the_task_panel_marks_done_current_and_to_do() {
        let mut g = StoryGame::new(&session(1, 3));
        g.sim.task_idx = 1;
        let panels = g.task_panels();
        let lines = &panels[0].lines;
        assert_eq!(lines.len(), 4);
        assert!(lines[0].0.starts_with("x Log in  (Control)"));
        assert!(lines[1].0.starts_with("> Calibrate the lamps  (Gallery)"));
        assert!(lines[2].0.starts_with("- Collect the parts  (Workshop)"));
        assert_eq!(panels[1].lines[0].0, "CONTROL");
        g.sim.hidden = true;
        assert_eq!(g.task_panels()[1].lines[0].0, "HIDING");
    }

    #[test]
    fn the_task_list_sits_clear_of_the_bars_and_the_monitor_banks() {
        let g = StoryGame::new(&session(1, 3));
        let panels = g.task_panels();
        let (x, y, w, h) = task_backing(&panels).expect("a backing");
        assert!(x >= 78.0, "right of the 70 wide NOISE / BEACON bars (x 8..78): {x}");
        assert!(y + h <= 71.0, "the four-task night stays above the banks (virtual y 71): {}", y + h);
        assert!(x + w < 480.0 && y >= 0.0);
        assert!(panels[1].lines[0].2 == TASK_TEXT && panels[1].y > panels[0].y);
        assert!(task_backing(&[]).is_none());
    }

    #[test]
    fn the_end_card_pieces_follow_the_godot_strings() {
        // end_card needs a ShiftCtx (GL objects), so the parts it is made of are checked here
        let modes = crate::core::ModesData::load();
        assert!(session(2, 5).has_next(&modes), "night 2 has a night 3 after it");
        assert!(!session(6, 5).has_next(&modes), "night 6 is the last");
        assert_eq!(modes.get("story").map(|m| m.scored), Some(6));
        assert_eq!(rating_of(400, 400), "Unbeatable!");
        assert_eq!(rules().label("gallery").to_lowercase(), "gallery");
        assert_eq!(StoryGame::new(&session(2, 5)).night + 1, 3);
    }
}
