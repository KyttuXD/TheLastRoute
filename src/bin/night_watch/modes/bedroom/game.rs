//! Bedtime (mode `bedroom`), the shift: you lie in bed in a dark room with no cameras and a flashlight on a battery. Hold A to
//! keep the closet door shut (Loxelle breathes behind it, never seen until she gets in), D to put the light down the hall
//! (Javelle waits where it ends), F to light the foot of the bed (Haelith creeps in while it is dark). One hand, so one thing
//! at a time. The mouse looks around from the pillow (the arrow keys too). Rules: `sim.rs`; the house: `world.rs`.
//!
//! Bedroom_game.gd has no monitor (`set_monitor_available(false)`), so there are no cameras to render; the hunters are
//! heard, and Javelle is glimpsed in the hall, walking on the nav graph.
#![allow(dead_code)]

use super::map::{map, BedroomMap};
use super::sim::{hunter_visible, rules, CueClock, Hunter, Light, Over, Sim, SimEvent, SimInput, SimOpts, Stage};
use super::world::{lights_for, BedroomWorld, Closet};
use crate::audio::cue_names;
use crate::core::{
    hour_index, hour_label, AdminEntry, CallEvent, CallScript, Cue, EndCard, Mode, ModesData, Outcome, Session, ShiftCtx, HOUR_MS,
    NIGHT_MS,
};
use crate::gfx::FpCamera;
use crate::hunters::{Actor, ActorDriver};
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Playing,
    Won,
    Lost,
}

/// The hint line (bedroom_game.gd).
pub const HINT: &str = "Hold  A closet door   D hall light   F bed light   mouse / arrows look   R radio   P pause";

/// The admin call buttons need the HUD, which only a frame callback has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CallAdmin {
    Play,
    Skip,
}

pub struct BedroomMode {
    night: i32,
    custom: bool,
    sim: Sim,
    cues: CueClock,
    world: Option<BedroomWorld>,
    cam: FpCamera,
    actors: [Option<Actor>; 3],
    drivers: [ActorDriver; 3],
    /// Which hand is down: [closet door, hall light, bed light].
    held: [bool; 3],
    state: State,
    hour_seen: i32,
    frozen: bool,
    no_scare: bool,
    reveal: bool,
    scare_light: Option<Light>,
    pending_call: Option<CallAdmin>,
}

impl BedroomMode {
    pub fn new(session: &Session) -> BedroomMode {
        let o = SimOpts::from_session(session);
        let m = map();
        let mut cam = FpCamera::new(m.eye, 0.0, 0.0);
        cam.fov_deg = rules().look.fov;
        BedroomMode {
            night: o.night,
            custom: o.custom,
            sim: Sim::new(&o),
            cues: CueClock::new(),
            world: None,
            cam,
            actors: [None, None, None],
            drivers: [ActorDriver::new(), ActorDriver::new(), ActorDriver::new()],
            held: [false; 3],
            state: State::Playing,
            hour_seen: 0,
            frozen: false,
            no_scare: false,
            reveal: false,
            scare_light: None,
            pending_call: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn held(&self) -> [bool; 3] {
        self.held
    }

    // ---- the hands (also what the tests press)

    /// One hand: pressing one lets go of the others. True when a sound should play.
    pub fn press(&mut self, i: usize, on: bool) -> bool {
        if self.state != State::Playing || i > 2 {
            return false;
        }
        if on {
            self.held = [false; 3];
        }
        self.held[i] = on;
        on
    }

    pub fn current_input(&self) -> SimInput {
        let light = if self.held[0] {
            Light::Dark
        } else if self.held[1] {
            Light::Hall
        } else if self.held[2] {
            Light::Bed
        } else {
            Light::Dark
        };
        SimInput { hold: self.held[0], light }
    }

    // ---- the look

    pub fn apply_look(&mut self) {
        let l = &rules().look;
        self.cam.yaw = self.cam.yaw.clamp(-l.yaw, l.yaw);
        self.cam.pitch = self.cam.pitch.clamp(-l.pitch_up * l.pitch_down_factor, l.pitch_up);
    }

    /// Mouse motion in screen px, with the player's sensitivity setting (scaled so the default feels like the Godot 0.0022).
    pub fn look_by_mouse(&mut self, dx: f32, dy: f32, sens_setting: f32) {
        let k = rules().look.sense / 0.0075;
        self.cam.yaw -= dx * sens_setting * k;
        self.cam.pitch -= dy * sens_setting * k;
        self.apply_look();
    }

    /// The arrow keys: left/right turn, up/down tilt.
    pub fn look_by_keys(&mut self, left: f32, up: f32, dt: f32) {
        if left != 0.0 || up != 0.0 {
            let l = &rules().look;
            self.cam.yaw += left * l.key_yaw_rate * dt;
            self.cam.pitch += up * l.key_pitch_rate * dt;
            self.apply_look();
        }
    }

    pub fn camera(&self) -> &FpCamera {
        &self.cam
    }

    // ---- the lights

    /// The light shown now: the one the hand holds, or the scare's own once you are caught.
    pub fn light_now(&self) -> Light {
        if self.state == State::Lost {
            self.scare_light.unwrap_or(Light::Dark)
        } else {
            self.sim.logic.light
        }
    }

    // ---- what the player reads

    fn sync_hud(&self, ctx: &mut ShiftCtx) {
        let l = &self.sim.logic;
        let h = &mut *ctx.hud;
        h.hour = hour_index(l.elapsed);
        h.power = l.battery as f32;
        h.usage = 0;
        h.night_label = if self.custom { "CUSTOM NIGHT".to_string() } else { format!("NIGHT {}", self.night) };
        h.power_text = Some(format!("BATTERY {}%", l.battery.round() as i32));
        h.usage_text = Some(if l.restless { "RESTLESS".to_string() } else { String::new() });
        h.hint = if crate::hud::admin::enabled() { format!("{HINT}   0 admin") } else { HINT.to_string() };
        h.monitor_available = false;
        h.monitor_up = false;
        h.show_crosshair = false;
    }

    /// The end card text of a finished shift (pure so the tests can read it).
    pub fn end_card_for(&self, session: &Session, modes: &ModesData, best_score: i32) -> Option<EndCard> {
        let l = &self.sim.logic;
        match l.over {
            Over::Running => None,
            Over::Win => {
                let more = session.has_next(modes);
                let mut body = format!("6:00 AM\nMorning came. Battery left: {}%.", l.battery.round() as i32);
                if self.custom {
                    let total: i32 = session.custom_levels.values().map(|v| (*v).clamp(0, crate::core::MAX_AI)).sum::<i32>() * crate::core::POINTS_PER_LEVEL;
                    body += &format!("\nScore {}  {}  (best {})", total, crate::core::rating_of(total, rules().score_max), best_score);
                } else if more {
                    body += &format!("\nNight {} is open.", self.night + 1);
                } else {
                    body += "\nYou cleared every night.";
                }
                let title = if self.custom { "CUSTOM NIGHT COMPLETE".to_string() } else { format!("NIGHT {} COMPLETE", self.night) };
                Some(EndCard { title, body, can_next: more && !self.custom })
            }
            Over::Caught => {
                let who = l.by.map(|h| rules().name_of(h).to_string()).unwrap_or_else(|| "Someone".to_string());
                Some(EndCard {
                    title: "CAUGHT".to_string(),
                    body: format!("{} got to you at {}.", who, hour_label(hour_index(l.elapsed))),
                    can_next: false,
                })
            }
        }
    }

    // ---- turning the sim's events into sound

    fn play(ctx: &mut ShiftCtx, list: Vec<Cue>) {
        ctx.snd.cues(&list);
    }

    fn settle(&mut self, ctx: &mut ShiftCtx) {
        if self.state != State::Playing {
            return;
        }
        match self.sim.logic.over {
            Over::Running => {}
            Over::Win => {
                self.state = State::Won;
                Self::play(ctx, cue_names::win());
            }
            Over::Caught => {
                self.state = State::Lost;
                self.sim.settle_catch();
                let short = ctx.save.short_death_scenes();
                Self::play(ctx, cue_names::caught(short));
                // her scare: the one moment she is seen. The closet door drifts open on her, or the light finds the other two.
                if let Some(who) = self.sim.logic.by {
                    self.scare_light = Some(self.sim.scare_light(who));
                    if who == Hunter::Loxelle {
                        if let Some(w) = &mut self.world {
                            w.set_closet(Closet::Wide);
                        }
                    }
                }
            }
        }
    }

    fn sync_world(&mut self) {
        if self.state != State::Playing {
            return;
        }
        let held = self.sim.logic.hold;
        if let Some(w) = &mut self.world {
            w.set_closet(if held { Closet::Held } else { Closet::Ajar });
        }
    }

    // ---- the actors

    fn place_actors(&mut self, dt_s: f32) {
        for h in Hunter::ALL {
            let i = h.idx();
            let visible = hunter_visible(&self.sim, h, self.reveal);
            if let (Some(actor), Some(agent)) = (self.actors[i].as_mut(), self.sim.crowd().agent(h.id())) {
                self.drivers[i].drive(agent, actor);
                actor.visible = visible;
                actor.animate(dt_s);
            }
        }
    }

    // ---- the admin menu

    pub fn admin_status(&self) -> Vec<String> {
        let l = &self.sim.logic;
        let stage = |h: Hunter| -> String {
            let th = l.door(h).map(|d| (d.state.name(), d.t / 1000.0, d.need / 1000.0)).unwrap_or(("away", 0.0, 0.0));
            format!(
                "{}  {}  AI {}  stage {:.1}s  answered {:.1}s",
                h.id().to_uppercase(),
                th.0,
                l.level(h),
                th.1,
                th.2
            )
        };
        vec![
            format!(
                "BEDTIME  NIGHT {}  {}  BATTERY {}%",
                self.night,
                hour_label(hour_index(l.elapsed)),
                l.battery.round() as i32
            ),
            stage(Hunter::Loxelle),
            stage(Hunter::Javelle),
            format!("HAELITH  creep {}%  AI {}", l.haelith.p as i32, l.level(Hunter::Haelith)),
            format!(
                "HAND {}  LIGHT {}  IDLE {:.0}s{}",
                if l.hold { "closet" } else { "free" },
                if l.light == Light::Dark { "dark" } else { l.light.name() },
                l.idle / 1000.0,
                if l.restless { "  RESTLESS" } else { "" }
            ),
            format!(
                "FROZEN {}  GOD {}  NO SCARE {}  SPEED {:.1}X",
                self.frozen, l.god, self.no_scare, l.hunter_speed
            ),
        ]
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn no_scare(&self) -> bool {
        self.no_scare
    }

    pub fn reveal(&self) -> bool {
        self.reveal
    }

    fn set_hand(&mut self, hand: &str) {
        self.held = [false; 3];
        match hand {
            "hold" => self.held[0] = true,
            "hall" => self.held[1] = true,
            "bed" => self.held[2] = true,
            _ => {}
        }
    }
}

impl Mode for BedroomMode {
    fn id(&self) -> &'static str {
        "bedroom"
    }

    fn start(&mut self, ctx: &mut ShiftCtx) {
        let m: &BedroomMap = map();
        self.world = Some(BedroomWorld::new(ctx.gfx, m));
        for h in Hunter::ALL {
            self.actors[h.idx()] = ctx.hunters.actor_in(h.id(), "bedroom");
        }
        self.sim.set_snap(ctx.manual_clock);
        ctx.hud.call = if self.custom { CallScript::empty() } else { CallScript::for_level("bedroom", ctx.session.level()) };
        ctx.player.frozen = true;
        ctx.player.look_frozen = true;
        self.cam.pos = m.eye;
        self.sync_hud(ctx);
        self.sync_world();
        self.place_actors(0.0);
    }

    fn handle_input(&mut self, ctx: &mut ShiftCtx) {
        if self.state != State::Playing {
            return;
        }
        let inp = ctx.input;
        for (action, i) in [("nw_bed_hold", 0usize), ("nw_bed_hall", 1), ("nw_bed_light", 2)] {
            if inp.just(action) {
                if self.press(i, true) {
                    Self::play(ctx, if i == 0 { cue_names::door(true) } else { cue_names::panel_button() });
                }
            }
            if inp.up(action) {
                self.held[i] = false;
            }
        }
        if inp.just("office_radio") && ctx.hud.call.mute() {
            ctx.hud.zellia_muted = true;
        }
        self.look_by_mouse(inp.mouse_delta.0, inp.mouse_delta.1, ctx.save.mouse_sens);
        let axis = |plus: KeyCode, minus: KeyCode| -> f32 {
            (if is_key_down(plus) { 1.0 } else { 0.0 }) - (if is_key_down(minus) { 1.0 } else { 0.0 })
        };
        self.look_by_keys(axis(KeyCode::Left, KeyCode::Right), axis(KeyCode::Up, KeyCode::Down), inp.dt);
    }

    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64) {
        if self.state != State::Playing || self.frozen {
            return;
        }
        let events = self.sim.step(dt_ms, &self.current_input());
        let elapsed = self.sim.logic.elapsed;
        for ev in ctx.hud.call.update(elapsed, dt_ms) {
            match ev {
                CallEvent::Ring => Self::play(ctx, cue_names::call_ring()),
                CallEvent::Talk => Self::play(ctx, cue_names::call_line()),
                _ => {}
            }
        }
        let hour = hour_index(elapsed);
        if hour != self.hour_seen {
            self.hour_seen = hour;
            Self::play(ctx, cue_names::hour_change());
        }
        for e in &events {
            let list = self.cues.on_event(e, elapsed);
            Self::play(ctx, list);
            if *e == SimEvent::Restless {
                ctx.hud.set_note("THE HOUSE IS RESTLESS", 2.5);
            }
        }
        if self.sim.logic.over == Over::Running {
            let list = self.cues.breathing(&self.sim.logic);
            Self::play(ctx, list);
        }
        self.sync_world();
        self.sync_hud(ctx);
        self.settle(ctx);
    }

    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32) {
        // admin buttons that need the HUD
        match self.pending_call.take() {
            Some(CallAdmin::Play) => {
                let mut c = CallScript::for_level("bedroom", ctx.session.level());
                if c.done {
                    c = CallScript::from_lines(vec!["Zellia, on the headset.".to_string()], 0.0);
                }
                ctx.hud.call = c;
                ctx.hud.zellia_muted = false;
                ctx.hud.call.update(self.sim.logic.elapsed, 0.0);
            }
            Some(CallAdmin::Skip) => {
                ctx.hud.call.mute();
                ctx.hud.call_text.clear();
            }
            None => {}
        }
        // an admin win or lose set by hand
        self.settle(ctx);
        // the walkers keep walking whenever the sim is not stepping them (the night is over, or the admin froze it)
        if !ctx.paused && (self.state != State::Playing || self.frozen) {
            self.sim.tick_crowd(dt_s);
        }
        self.place_actors(dt_s);
        self.sync_hud(ctx);
    }

    fn draw_world(&mut self, ctx: &mut ShiftCtx) {
        let m = map();
        let bg = m.lights.background;
        clear_background(Color::new(bg[0], bg[1], bg[2], 1.0));
        let Some(world) = &self.world else { return };
        self.cam.apply();
        let lights = lights_for(m, self.light_now(), self.reveal);
        world.draw(&*ctx.gfx, &lights, self.cam.pos);
        for a in self.actors.iter().flatten() {
            a.draw(&*ctx.gfx, &lights, self.cam.pos);
        }
    }

    fn outcome(&self) -> Outcome {
        self.sim.outcome()
    }

    fn end_card(&self, ctx: &ShiftCtx) -> Option<EndCard> {
        self.end_card_for(ctx.session, ctx.modes, ctx.save.best_score("bedroom"))
    }

    fn killer(&self) -> Option<(String, String)> {
        if self.no_scare {
            return None;
        }
        match (self.sim.logic.over, self.sim.logic.by) {
            (Over::Caught, Some(h)) => Some((h.id().to_string(), String::new())),
            _ => None,
        }
    }

    fn admin_entries(&mut self) -> Vec<AdminEntry> {
        self.admin_status().iter().map(|s| AdminEntry::heading(s)).collect()
    }

    fn admin_do(&mut self, id: &str) {
        let tun = &rules().tun;
        match id {
            "hand_hold" => self.set_hand("hold"),
            "hand_hall" => self.set_hand("hall"),
            "hand_bed" => self.set_hand("bed"),
            "hand_none" => self.set_hand("none"),
            "restless" => self.sim.logic.idle = tun.afk_ms,
            "freeze" => self.frozen = !self.frozen,
            "infinite_battery" => self.sim.logic.infinite_battery = !self.sim.logic.infinite_battery,
            "god" => self.sim.logic.god = !self.sim.logic.god,
            "no_scare" => self.no_scare = !self.no_scare,
            "reveal" => self.reveal = !self.reveal,
            "call_play" => self.pending_call = Some(CallAdmin::Play),
            "call_skip" => self.pending_call = Some(CallAdmin::Skip),
            _ => {
                if let Some(p) = id.strip_prefix("creep_") {
                    if let Ok(v) = p.parse::<f64>() {
                        self.sim.logic.haelith.p = v.clamp(0.0, 99.0);
                    }
                } else if let Some(h) = id.strip_prefix("hour_") {
                    if let Ok(v) = h.parse::<i32>() {
                        self.sim.logic.elapsed = (v as f64 * HOUR_MS).clamp(0.0, NIGHT_MS - 1000.0);
                        self.hour_seen = hour_index(self.sim.logic.elapsed);
                    }
                } else if let Some(b) = id.strip_prefix("battery_") {
                    if let Ok(v) = b.parse::<f64>() {
                        self.sim.logic.battery = v.clamp(0.0, 100.0);
                    }
                } else if let Some(s) = id.strip_prefix("speed_") {
                    if let Ok(v) = s.parse::<f64>() {
                        self.sim.logic.hunter_speed = v.clamp(0.1, 8.0);
                    }
                } else if let Some(rest) = id.strip_prefix("place_") {
                    // place_<hunter>_<stage>
                    let mut it = rest.splitn(2, '_');
                    if let (Some(h), Some(s)) = (it.next(), it.next()) {
                        if let (Some(h), Some(s)) = (Hunter::from_id(h), Stage::from_name(s)) {
                            self.sim.admin_place(h, s);
                        }
                    }
                }
            }
        }
    }

    fn tour_protect(&mut self) {
        self.sim.logic.god = true;
        self.sim.logic.infinite_battery = true;
        self.no_scare = true;
        let e = crate::core::tour_clock_wrap(self.sim.logic.elapsed);
        if e != self.sim.logic.elapsed {
            self.sim.logic.elapsed = e;
            self.hour_seen = hour_index(e);
        }
    }

    fn debug_win(&mut self) {
        if self.state == State::Playing {
            self.sim.logic.over = Over::Win;
        }
    }

    fn debug_lose(&mut self) {
        if self.state == State::Playing {
            self.sim.logic.over = Over::Caught;
            self.sim.logic.by = Some(Hunter::Loxelle);
        }
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn session(night: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("bedroom", night, BTreeMap::new(), Some(seed));
        s
    }

    fn custom_session(levels: &[(&str, i32)]) -> Session {
        let mut s = Session::new();
        let c: BTreeMap<String, i32> = levels.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        s.start_mode("bedroom", 1, c, Some(3));
        s
    }

    #[test]
    fn one_hand_means_pressing_one_lets_go_of_the_others() {
        let mut m = BedroomMode::new(&session(2, 1));
        assert!(m.press(0, true));
        assert_eq!(m.held(), [true, false, false]);
        assert!(m.press(1, true));
        assert_eq!(m.held(), [false, true, false]);
        assert!(m.press(2, true));
        assert_eq!(m.held(), [false, false, true]);
        assert!(!m.press(2, false), "letting go plays nothing");
        assert_eq!(m.held(), [false, false, false]);
        assert!(!m.press(9, true));
    }

    #[test]
    fn the_hand_becomes_the_sim_input() {
        let mut m = BedroomMode::new(&session(2, 1));
        assert_eq!(m.current_input(), SimInput::NONE);
        m.press(0, true);
        assert_eq!(m.current_input(), SimInput { hold: true, light: Light::Dark });
        m.press(1, true);
        assert_eq!(m.current_input(), SimInput { hold: false, light: Light::Hall });
        m.press(2, true);
        assert_eq!(m.current_input(), SimInput { hold: false, light: Light::Bed });
    }

    #[test]
    fn the_look_is_clamped_to_the_pillow() {
        let mut m = BedroomMode::new(&session(1, 1));
        m.look_by_mouse(-100000.0, 0.0, 0.0075);
        assert!((m.camera().yaw - 1.25).abs() < 1e-6);
        m.look_by_mouse(100000.0, 0.0, 0.0075);
        assert!((m.camera().yaw + 1.25).abs() < 1e-6);
        m.look_by_mouse(0.0, -100000.0, 0.0075);
        assert!((m.camera().pitch - 0.7).abs() < 1e-6);
        m.look_by_mouse(0.0, 100000.0, 0.0075);
        assert!((m.camera().pitch + 0.42).abs() < 1e-6, "{}", m.camera().pitch);
        // the default sensitivity feels like the Godot 0.0022 per px
        let mut k = BedroomMode::new(&session(1, 1));
        k.look_by_mouse(100.0, 0.0, 0.0075);
        assert!((k.camera().yaw + 100.0 * 0.0022).abs() < 1e-5);
        // the arrow keys turn at 1.4 rad/s
        let mut a = BedroomMode::new(&session(1, 1));
        a.look_by_keys(1.0, 0.0, 0.5);
        assert!((a.camera().yaw - 0.7).abs() < 1e-6);
        assert_eq!(a.camera().fov_deg, 75.0);
        assert_eq!(a.camera().pos, vec3(0.0, 0.95, 1.9));
    }

    #[test]
    fn a_custom_night_uses_night_five_and_the_sliders() {
        let s = custom_session(&[("loxelle", 9), ("javelle", 0), ("haelith", 4)]);
        let m = BedroomMode::new(&s);
        assert!(m.custom);
        assert_eq!(m.night, 5);
        assert_eq!(m.sim.logic.base_level(Hunter::Loxelle), 9);
        assert_eq!(m.sim.logic.base_level(Hunter::Javelle), 0);
        assert_eq!(m.sim.logic.row.breath_ms, 3200.0, "custom keeps night one's timings");
    }

    #[test]
    fn the_end_cards_say_what_godot_says() {
        let modes = ModesData::load();
        let mut m = BedroomMode::new(&session(2, 1));
        assert!(m.end_card_for(&session(2, 1), &modes, 0).is_none());
        m.sim.logic.over = Over::Win;
        m.sim.logic.battery = 41.6;
        let c = m.end_card_for(&session(2, 1), &modes, 0).expect("win card");
        assert_eq!(c.title, "NIGHT 2 COMPLETE");
        assert_eq!(c.body, "6:00 AM\nMorning came. Battery left: 42%.\nNight 3 is open.");
        assert!(c.can_next);
        // the last night has no next
        let mut last = BedroomMode::new(&session(6, 1));
        last.sim.logic.over = Over::Win;
        let c = last.end_card_for(&session(6, 1), &modes, 0).expect("win card");
        assert_eq!(c.title, "NIGHT 6 COMPLETE");
        assert!(c.body.ends_with("You cleared every night."), "{}", c.body);
        assert!(!c.can_next);
        // a custom night shows the score line
        let cs = custom_session(&[("loxelle", 10), ("javelle", 10), ("haelith", 10)]);
        let mut cm = BedroomMode::new(&cs);
        cm.sim.logic.over = Over::Win;
        let c = cm.end_card_for(&cs, &modes, 300).expect("custom card");
        assert_eq!(c.title, "CUSTOM NIGHT COMPLETE");
        assert!(c.body.contains("Score 300  Stupendous!  (best 300)"), "{}", c.body);
        assert!(!c.can_next);
        // caught
        let mut lost = BedroomMode::new(&session(2, 1));
        lost.sim.logic.over = Over::Caught;
        lost.sim.logic.by = Some(Hunter::Javelle);
        lost.sim.logic.elapsed = 2.5 * 45000.0;
        let c = lost.end_card_for(&session(2, 1), &modes, 0).expect("lost card");
        assert_eq!((c.title.as_str(), c.body.as_str()), ("CAUGHT", "Javelle got to you at 2 AM."));
        assert!(!c.can_next);
        assert_eq!(lost.killer(), Some(("javelle".to_string(), String::new())));
        assert_eq!(lost.outcome(), Outcome::Lost);
    }

    #[test]
    fn the_admin_hands_hours_and_creep_really_change_the_game() {
        let mut m = BedroomMode::new(&session(3, 7));
        m.admin_do("hand_hall");
        assert_eq!(m.held(), [false, true, false]);
        m.admin_do("hand_none");
        assert_eq!(m.held(), [false; 3]);
        m.admin_do("creep_75");
        assert_eq!(m.sim.logic.haelith.p, 75.0);
        m.admin_do("creep_99");
        assert_eq!(m.sim.logic.haelith.p, 99.0);
        m.admin_do("hour_3");
        assert_eq!(m.sim.logic.elapsed, 135000.0);
        assert_eq!(m.hour_seen, 3);
        m.admin_do("hour_5");
        assert_eq!(m.sim.logic.elapsed, 225000.0);
        m.admin_do("battery_25");
        assert_eq!(m.sim.logic.battery, 25.0);
        m.admin_do("speed_4");
        assert_eq!(m.sim.logic.hunter_speed, 4.0);
        m.admin_do("restless");
        assert_eq!(m.sim.logic.idle, 14000.0);
        m.admin_do("god");
        m.admin_do("freeze");
        m.admin_do("no_scare");
        m.admin_do("reveal");
        m.admin_do("infinite_battery");
        assert!(m.sim.logic.god && m.is_frozen() && m.no_scare() && m.reveal() && m.sim.logic.infinite_battery);
        m.admin_do("god");
        assert!(!m.sim.logic.god);
        m.admin_do("place_loxelle_near");
        assert_eq!(m.sim.logic.doors[0].state, Stage::Near);
        m.admin_do("place_haelith_breath");
        assert_eq!(m.sim.logic.haelith.p, 50.0);
        m.admin_do("place_javelle_nonsense");
        assert_eq!(m.sim.logic.doors[1].state, Stage::Away);
        // the status block is readable
        let st = m.admin_status();
        assert!(st[0].starts_with("BEDTIME  NIGHT 3  5 AM"), "{}", st[0]);
        assert!(st.iter().any(|l| l.starts_with("LOXELLE  near")));
    }

    #[test]
    fn admin_win_and_lose_are_decided_outcomes() {
        let mut w = BedroomMode::new(&session(1, 1));
        w.debug_win();
        assert_eq!(w.outcome(), Outcome::Won);
        let mut l = BedroomMode::new(&session(1, 1));
        l.debug_lose();
        assert_eq!(l.outcome(), Outcome::Lost);
        assert_eq!(l.killer().map(|k| k.0), Some("loxelle".to_string()));
        l.admin_do("no_scare");
        assert_eq!(l.killer(), None, "no scare skips the death moment");
    }

    #[test]
    fn the_mode_reports_its_id_and_a_hint_with_the_three_keys() {
        let m = BedroomMode::new(&session(1, 1));
        assert_eq!(m.id(), "bedroom");
        assert!(HINT.contains("A closet door") && HINT.contains("D hall light") && HINT.contains("F bed light"));
        assert_eq!(m.state(), State::Playing);
        assert_eq!(m.light_now(), Light::Dark);
    }
}
