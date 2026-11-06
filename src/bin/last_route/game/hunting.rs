//! Running the floor's hunters each frame and applying what they do (main_3d.gd): gimmick
//! events, captures and the escape struggle (mash [E] 6 times in 3.2 s), the hide check at a
//! locker, and the run report.

use super::{pdist, Admin, Escape, Game, Heater, Item, Report};
use crate::audio::Sounds;
use crate::config::*;
use super::level_stack::floor_y_of;
use super::resident_hunters::{frame_entry, Slot};
use crate::hunters::brain::{State, Target};
use crate::net::hunter_events;
use serde_json::{json, Value};
use crate::hunters::gimmicks::{nyxie::Wake, Ctx, Event, Gimmick};
use crate::hunters::noise_events::{self, NoiseEvent, NoiseKind};
use crate::hunters::water::WaterField;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What decides whether the hunters' water field must be rebuilt: the sewer chunk it was read from, the window origin,
/// the level, a time bucket while a pipe flow runs, and how many pipe cells and hunters there are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterKey {
    pub chunk: Option<(i32, i32, i32)>,
    pub origin: (i32, i32),
    pub level: i32,
    pub bucket: i64,
    pub pipe_cells: usize,
    pub hunters: usize,
}

/// The water field every hunter brain reads (`Brain::set_water`), the key it was built for, and the per-hunter state of
/// the wading splashes.
#[derive(Clone)]
pub struct HunterWater {
    pub key: WaterKey,
    pub field: Arc<WaterField>,
    /// Seconds to the next wading splash, per hunter (indexed as `Game::hunters`).
    pub splash_in: Vec<f32>,
    /// Each hunter's position last frame, to tell a wading hunter from a standing one.
    pub last_pos: Vec<(f32, f32)>,
}

/// The window cells a pipe flow reaches: `(cell, steps from the pipe)`, on outlet `outlet` of flow `id`.
#[derive(Clone, Debug, PartialEq)]
pub struct PipeCells {
    pub id: String,
    pub outlet: usize,
    pub cells: Vec<((i32, i32), i32)>,
}

/// Merges the sewer's water and the pipe floods into the field's two inputs: the deepest water per cell, and the cells
/// a pipe or gate has flooded to at least `flooded_depth` (the cells that change cost live and force a replan).
pub fn water_layers(
    sewer: &[((i32, i32), f32)],
    pipes: &[((i32, i32), f32)],
    flooded_depth: f32,
) -> (Vec<((i32, i32), f32)>, Vec<(i32, i32)>) {
    let mut depth: BTreeMap<(i32, i32), f32> = BTreeMap::new();
    for (c, d) in sewer.iter().chain(pipes.iter()) {
        if *d > 0.0 {
            let e = depth.entry(*c).or_insert(0.0);
            *e = e.max(*d);
        }
    }
    let mut flooded: Vec<(i32, i32)> = pipes.iter().filter(|(_, d)| *d >= flooded_depth).map(|(c, _)| *c).collect();
    flooded.sort();
    flooded.dedup();
    (depth.into_iter().collect(), flooded)
}

impl Game {
    /// The host's hunter frame (Godot `_collect_hunter_report`): one object per hunter of every list, with its id,
    /// position, facing, state name, active flag, stun, slow factor and disguise, and the `level` it stands on and its
    /// `presentation_y` (absolute feet). The active list comes first, indexed as `hunters` is; the resident hunters follow
    /// at `active.len() + k`, so an old guest's id check at that index skips them. Guests read it through
    /// [`Game::apply_hunter_frame`].
    pub fn hunter_frame(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self.hunters.iter().enumerate().map(|(index, h)| frame_entry(index, h)).collect();
        out.extend(self.resident_frame_entries());
        out
    }

    /// Guest: shows the host's hunters. Each entry finds its hunter by id (its `index` is only a hint), sets `level` and
    /// `presentation_y` (an old host sends neither: the hunter stands on the viewer's level at that floor's height), and a
    /// hunter whose level is not its list's moves to the list of that level. A field that is missing or not finite is left
    /// as it was. Only the display state is set here.
    pub fn apply_hunter_frame(&mut self, data: &[Value]) {
        let viewer = self.runner_level();
        let finite = |v: &Value| v.as_f64().map(|f| f as f32).filter(|f| f.is_finite());
        for entry in data {
            let Some(id) = entry["id"].as_str() else { continue };
            let index = entry["index"].as_u64().map_or(usize::MAX, |i| i as usize);
            let level = entry["level"].as_i64().and_then(|l| i32::try_from(l).ok());
            let spot = match self.locate_hunter(id, index, level) {
                Some(s) => s,
                None => match self.create_guest_hunter(id, level) {
                    Some(s) => s,
                    None => continue,
                },
            };
            let Some(h) = self.hunter_at_mut(spot) else { continue };
            if let (Some(x), Some(y)) = (finite(&entry["position"][0]), finite(&entry["position"][1])) {
                h.pos = (x, y);
            }
            if let (Some(x), Some(y)) = (finite(&entry["facing"][0]), finite(&entry["facing"][1])) {
                h.facing = (x, y);
            }
            if let Some(state) = entry["state"].as_str().and_then(hunter_events::state_from_name) {
                h.state = state;
            }
            if let Some(active) = entry["active"].as_bool() {
                h.active = active;
            }
            if let Some(stun) = finite(&entry["stun"]) {
                h.stun = stun;
            }
            if let Some(slow) = finite(&entry["slow_factor"]) {
                h.slow_factor = slow;
            }
            if entry.get("disguise").is_some() {
                h.disguise = entry["disguise"].as_str().map(str::to_string);
            }
            if let crate::hunters::gimmicks::Gimmick::WolfSpider(s) = &mut h.gimmick {
                if let (Some(phase), Some(bark)) = (entry["phase"].as_i64(), entry["bark_seq"].as_u64()) {
                    s.apply_network_state(phase, bark.min(u32::MAX as u64) as u32);
                }
            }
            h.level = level.unwrap_or(viewer);
            h.y = finite(&entry["presentation_y"]).unwrap_or_else(|| floor_y_of(h.level));
        }
        self.move_crossers();
    }

    pub(super) fn target(&self) -> Target {
        let mut t = self.runner_target();
        if self.admin_flags().no_target {
            // admin no_target: the runner reads as sheltered and silent to every hunter
            t.in_safe_zone = true;
            t.noise = 0.0;
            t.flashlight_on = false;
            t.sprinting = false;
        }
        t
    }

    fn runner_target(&self) -> Target {
        Target {
            pos: self.p.pos,
            facing: self.forward2(),
            hidden: self.p.hidden.is_some(),
            hiding_spot: self.p.hidden,
            in_safe_zone: self.in_safe_zone(),
            noise: self.p.noise * self.swim.noise_scale(&self.swim_cfg),
            flashlight_on: self.p.flashlight_on,
            lamp_charge: self.p.lamp,
            sprinting: self.p.sprinting && self.p.noise > 0.0,
            in_vent: self.p.in_vent,
            vent_time_in: self.p.vent_time,
            // a cold source on the runner keeps the reading under Solyth's pounce gate (specialty.rs)
            temperature: self.specialty.reading_for_hunters(self.meters.temperature),
            pesticide: self.p.pesticide > 0.0,
            map_open: self.map_open,
            level: self.runner_level(),
            // `y()` is floor + height and height already carries the surface under the feet (decks and the Inferno's
            // noisy floor), the same absolute y a hunter keeps through `Brain::follow_terrain`
            y: self.p.y(),
            cross: None,
        }
    }

    pub(super) fn update_hunters(&mut self, dt: f32, prof: &Profile, _sounds: &Sounds) {
        // this frame's noise events (plunge, landing, valve, burst, gate): drained here whoever simulates, so none piles up
        let noise = std::mem::take(&mut self.noise_events);
        // a guest's hunters only display: they never start emotes of their own
        let display_only = self.remote_hunters;
        for h in &mut self.hunters {
            h.display_only = display_only;
        }
        for h in self.resident_hunters.iter_all_mut() {
            h.display_only = display_only;
        }
        // a guest runs no hunter simulation: the host's frames set every hunter (apply_hunter_frame)
        if self.remote_hunters {
            return;
        }
        // this frame's route budget (hunters/path_scheduler.rs): last frame's queued searches are answered first
        crate::hunters::path_scheduler::begin_frame(&self.fac);
        // one navigation graph over the active and resident levels, given to every brain of every list
        self.sync_stack_nav();
        // the Inferno's secret creatures: the dwell clock on level -3 and their spawn (game/inferno_secrets.rs)
        self.update_inferno_secrets(dt);
        // _direct_hunters: the director runs every frame before the brains, in every mode
        // (relay rush on); the Infinite route then pushes crowded hunters apart
        let target = self.target();
        let admin = self.admin_flags();
        let runner = if self.report.is_none() && !admin.no_target {
            Some(self.p.pos)
        } else {
            None
        };
        let targets = vec![runner; self.hunters.len()];
        let remaining: Vec<crate::world::Cell> = self
            .fac
            .relays
            .iter()
            .zip(self.relays_taken.iter())
            .filter(|(_, t)| !**t)
            .map(|(c, _)| *c)
            .collect();
        self.director.relays_collected = self.relays_collected;
        self.director.relay_rush_enabled = true;
        self.director
            .tick(dt, &mut self.hunters, &targets, &self.fac, &remaining);
        // hunter sabotage (hunters/sabotage.rs): a dwell near an online relay may roll, and a hit blocks that relay
        // until its resecure time; off on every difficulty today
        let sabotaged = match self.sabotage.as_mut() {
            Some(tr) => tr.tick(dt, self.hunters.iter().map(|h| h.pos), &remaining).is_some(),
            None => false,
        };
        if sabotaged {
            self.announce("SIGNAL LOST // RELAY OFFLINE");
        }
        if self.inf.is_some() {
            crate::hunters::director::separate(dt, &mut self.hunters, &self.fac);
        }
        // the shared water field for every brain, the hunters' own wading splashes, and the sounds of this frame
        self.sync_hunter_water();
        self.hunters_hear_noise(&noise, prof.hunter_hearing);
        // a hunter's wading splash is heard by the bots only: hunters do not call each other
        let wading = self.hunter_wading_events(dt);
        self.npc_hear(&wading);
        // the NPC bots on a hunter's own level are candidate targets too (data/hunters/targets.json); the runner stays the
        // default target, and a bot is chased only where the rules allow it (hunters::brain::choose_target). A runner on
        // another level than the hunter carries the hunter's 3D sight and the adjacent hearing (`target_for`).
        let list_level = self.runner_level();
        let bot_targets = self.bot_targets();
        // the music box floor pickups still on the floor: Glyra guards the nearest (gimmicks/glyra.rs)
        let music_boxes: Vec<(f32, f32)> =
            self.pickups.iter().filter(|p| p.item == Item::MusicBox && !p.taken).map(|p| p.pos).collect();
        let mut all_events: Vec<(usize, Event)> = vec![];
        let mut caught: Option<usize> = None;
        for i in 0..self.hunters.len() {
            let (hunter_target, chose_bot) = self.target_for(&self.hunters[i], list_level, &target, &bot_targets);
            self.hunters[i].target_is_bot = chose_bot;
            // Nyxie's wake and dispel read the runner even while she chases a bot (nyxie::Nyxie::runner)
            if let Gimmick::Nyxie(n) = &mut self.hunters[i].gimmick {
                n.runner = chose_bot.then(|| target.clone());
            }
            let hunter_target = &hunter_target;
            if let Gimmick::Solyth(s) = &mut self.hunters[i].gimmick {
                s.heat_locked = self.meters.heat_locked;
            }
            // the temperature item's cold source slows Solyth while she is out (specialty.rs)
            self.specialty.deter(&mut self.hunters[i]);
            let heat_source = self.heat_source_for_hunters();
            let ctx = Ctx {
                fac: &self.fac,
                relays_taken: &self.relays_taken,
                batteries_taken: &self.batteries_taken,
                hiding_disabled: &self.hiding_disabled,
                vents: &self.fix.vents,
                // the temperature item's heat source is a heater to Wynelle (specialty.rs)
                heater_on: self.heater_running() || heat_source,
                heater_thawed: self.heater != Heater::Frozen || heat_source,
                diff: self.difficulty,
                music_box_left: self.music_box_left,
                music_boxes: &music_boxes,
                exposure: self.meters.exposure,
            };
            let held_before = ability_held(&self.hunters[i]);
            let (c, ev) =
                self.hunters[i].update(dt, &ctx, hunter_target, self.depth, self.relays_collected, prof);
            // the host tells the guests when a hunter starts a lunge or a pounce (the rising edge only)
            if let (None, Some((kind, ability))) = (held_before, ability_held(&self.hunters[i])) {
                let id = self.hunters[i].def.id.clone();
                self.hunter_out.emit(
                    hunter_events::rules(),
                    self.difficulty,
                    kind,
                    vec![("index", json!(i)), ("hunter_id", json!(id)), ("ability", json!(ability))],
                    crate::shared::clock::Instant::now(),
                );
            }
            // a capture ends the run only when the runner was the target: a bot caught is no run end
            if c && !chose_bot && caught.is_none() {
                caught = Some(i);
            }
            all_events.extend(ev.into_iter().map(|e| (i, e)));
        }
        for (i, e) in all_events {
            self.apply_event(i, e);
            if self.report.is_some() {
                return;
            }
        }
        // every resident floor simulates too (D5); a resident hunter that caught the runner joins the active list so the
        // struggle and the run report read it there
        if let Some((level, i)) = self.update_resident_hunters(dt, prof) {
            if caught.is_none() {
                caught = Some(self.adopt_resident(level, i));
            }
        }
        // a hunter that landed from a catwalk jump: the bots and the runner hear the thud, the other hunters do not
        let landings = self.hunter_landing_events();
        self.npc_hear(&landings);
        self.play_landing_thuds(&landings);
        // hunter footsteps, heartbeat, chase_start and Nyxie's wake cue: crate::audio::world::update (Game::update)
        if let Some(i) = caught {
            self.resolve_capture(i, &admin);
        }
        // a hunter whose derived level changed (it walked a flight or a ladder) moves to the list of that level
        self.move_crossers();
    }

    /// The runner is caught by active hunter `i`: the struggle starts, or the run ends when the hunter's capture cannot
    /// be escaped (or the escape is still cooling down).
    fn resolve_capture(&mut self, i: usize, admin: &Admin) {
        if self.escape_grace > 0.0 || admin.god || admin.no_target {
            return;
        }
        crate::audio::sound::capture();
        if self.hunters[i].gimmick.capture_inescapable() || self.escape_cooldown > 0.0 {
            let cause = if matches!(&self.hunters[i].gimmick, Gimmick::Nyxie(n) if n.wake == Wake::Empty) {
                "lamp"
            } else {
                "contact"
            };
            self.end_run(i, cause);
        } else {
            self.p.flashlight_on = false;
            // a runner pulled out of a locker is out of it: the grab view, not the hidden view, takes the camera
            self.hide_check = None;
            if let Some(li) = self.p.hidden {
                // the pulled-out runner cannot slip straight back in once the struggle is over
                self.locker_cooldown[li] = self.prof().hide_cooldown;
                let exit = capture_exit(&self.fac, li, self.hunters[i].pos);
                self.leave_locker(Some(exit));
            } else {
                self.leave_locker(None);
            }
            self.escape = Some(Escape {
                remaining: ESCAPE_DURATION,
                progress: 0,
                hunter: i,
            });
        }
    }

    pub(super) fn apply_event(&mut self, from: usize, e: Event) {
        match e {
            Event::Stun(s) => self.p.stun = self.p.stun.max(s),
            Event::FacilityAlert { at, except_self } => {
                for (j, h) in self.hunters.iter_mut().enumerate() {
                    if h.active && !(except_self && j == from) {
                        h.state = State::Investigate;
                        h.last_known = Some(at);
                    }
                }
            }
            Event::Eject { to } => {
                if self.p.hidden.is_some() {
                    self.leave_locker(Some(to));
                }
            }
            Event::Meter {
                exposure_add,
                noise_add,
                temperature_add,
            } => {
                self.meters.add_exposure(exposure_add);
                self.meters.add_noise(noise_add);
                if temperature_add != 0.0 {
                    self.meters.add_temperature(temperature_add);
                }
            }
            Event::SpotDisabled(i, v) => {
                if let Some(d) = self.hiding_disabled.get_mut(i) {
                    *d = v;
                }
            }
            Event::Mimic { rare: _ } => {}
            Event::TakeBattery(i) => {
                if let Some(t) = self.batteries_taken.get_mut(i) {
                    *t = true;
                }
            }
            Event::InescapableCapture => {
                let a = self.admin_flags();
                if !a.god && !a.no_target {
                    self.end_run(from, "cold")
                }
            }
            Event::Message(m) => self.say(&m),
            Event::SystemOutage(o) => self.begin_system_outage(o),
            Event::RadarCorrupt(r) => self.begin_radar_corrupt(r),
            Event::FakeFootsteps(f) => self.queue_fake_footsteps(&f),
        }
    }

    pub(super) fn update_escape(&mut self, dt: f32, c: &super::Controls, _sounds: &Sounds) {
        let prof = self.prof();
        let Some(esc) = self.escape.as_mut() else {
            return;
        };
        if c.interact {
            esc.progress += 1;
            crate::audio::sound::struggle(esc.progress as f32 / ESCAPE_PRESSES.max(1) as f32);
        }
        esc.remaining -= dt;
        let hi = esc.hunter;
        if esc.progress >= ESCAPE_PRESSES {
            self.escape = None;
            self.escape_cooldown_scale *= 0.95;
            self.escape_cooldown = ESCAPE_COOLDOWN * self.escape_cooldown_scale;
            self.escape_grace = ESCAPE_GRACE;
            let hp = self.hunters[hi].pos;
            let mut away = (self.p.pos.0 - hp.0, self.p.pos.1 - hp.1);
            let l = (away.0 * away.0 + away.1 * away.1).sqrt();
            away = if l < 0.01 {
                (1.0, 0.0)
            } else {
                (away.0 / l, away.1 / l)
            };
            // the shove stops at the first wall or railing on the way, never carries her through one
            let shoved = self.fac.clear_toward(
                self.p.pos,
                (self.p.pos.0 + away.0 * ESCAPE_KNOCKBACK, self.p.pos.1 + away.1 * ESCAPE_KNOCKBACK),
                PLAYER_RADIUS,
            );
            self.p.pos = self.fac.nearest_open_or(shoved.0, shoved.1, PLAYER_RADIUS);
            let pos = self.p.pos;
            self.hunters[hi].enter_search_from(pos, 1.8, &self.fac);
            self.hunters[hi].apply_stun(prof.escape_stun, false);
            crate::audio::sound::escape();
            self.say("You broke free!");
        } else if esc.remaining <= 0.0 {
            self.escape = None;
            self.end_run(hi, "contact");
        }
    }

    /// Seconds a hunter has spent checking hiding spot `spot`, while its check runs (the locker shakes for it,
    /// `net/hide_sync.rs` `search_tell_shake`); None when no check is on that spot.
    pub fn hide_check_elapsed(&self, spot: usize) -> Option<f32> {
        match self.hide_check {
            Some((s, _, left)) if s == spot => Some((self.prof().hide_tell - left).max(0.0)),
            _ => None,
        }
    }

    pub(super) fn update_hidden(
        &mut self,
        dt: f32,
        c: &super::Controls,
        prof: &Profile,
        _sounds: &Sounds,
    ) {
        let li = self.p.hidden.unwrap();
        // the look stays inside the locker window's cone (render/hiding_cam.rs); a rattling locker holds it still
        let shaking = matches!(self.hide_check, Some((spot, _, _)) if spot == li);
        if !crate::render::hiding_cam::look_locked(shaking) {
            let facing = self.fac.hiding.get(li).map_or((0.0, 1.0), |h| (h.facing.0 as f32, h.facing.1 as f32));
            let (yaw, pitch) = crate::render::hiding_cam::clamp_look(self.p.yaw, self.p.pitch, crate::render::hiding_cam::facing_yaw(facing));
            self.p.yaw = yaw;
            self.p.pitch = pitch;
        }
        self.p.moving = false;
        self.p.sprinting = false;
        self.p.noise = 0.0;
        self.p.flashlight_on = false;
        self.p.recover_stamina(dt);
        self.p.hide_lock = (self.p.hide_lock - dt).max(0.0);
        self.p.stun = (self.p.stun - dt).max(0.0);
        self.focus = None;
        let in_safe = self.in_safe_zone();
        self.meters.update(
            dt,
            &super::meters::Situation {
                flashlight_on: false,
                noise: 0.0,
                crouching: false,
                hidden: true,
                in_safe_zone: in_safe,
                heater_on: self.heater_running(),
                local: self.felt_climate(),
            },
        );
        if c.interact {
            if self.p.hide_lock > 0.0 {
                let s = self.p.hide_lock.ceil();
                self.say(&format!("Locked in hiding for {s:.0}s."));
            } else {
                self.leave_locker(None); // leaving a hiding spot is silent (route_interactable_3d.gd 795-808)
                return;
            }
        }
        // a hunter that tracked the runner here checks the locker after a short tell
        match self.hide_check {
            Some((spot, h, t)) if spot == li => {
                let t = t - dt;
                if t <= 0.0 {
                    self.hide_check = None;
                    self.locker_cooldown[li] = prof.hide_cooldown;
                    self.leave_locker(None);
                    let n = self.hunters[h].def.name.to_uppercase();
                    self.say(&format!("{n} PULLED THE DOOR OPEN"));
                    let id = self.hunters[h].def.id.clone();
                    self.hunter_out.emit(
                        hunter_events::rules(),
                        self.difficulty,
                        "hunter_door",
                        vec![("index", json!(h)), ("hunter_id", json!(id)), ("action", json!("open")), ("spot", json!(li))],
                        crate::shared::clock::Instant::now(),
                    );
                } else {
                    self.hide_check = Some((spot, h, t));
                }
            }
            _ => {
                let centre = self.fac.hiding[li].cell.center();
                if self.locker_cooldown[li] > 0.0 || self.remote_hunters {
                    return;
                }
                let found = self.hunters.iter().position(|h| {
                    h.active
                        && h.stun <= 0.0
                        && matches!(h.state, State::Chase | State::Search | State::Investigate)
                        && h.last_known
                            .map(|k| pdist(k, centre) <= prof.hide_trail)
                            .unwrap_or(false)
                        && pdist(h.pos, centre) <= HIDE_CHECK_REACH
                        && !h.gimmick.immune_to_capture(h)
                });
                if let Some(h) = found {
                    self.hide_check = Some((li, h, prof.hide_tell));
                    let id = self.hunters[h].def.id.clone();
                    self.hunter_out.emit(
                        hunter_events::rules(),
                        self.difficulty,
                        "hunter_locker",
                        vec![("index", json!(h)), ("hunter_id", json!(id)), ("spot", json!(li))],
                        crate::shared::clock::Instant::now(),
                    );
                    // her breath at the slats (hunter_view_3d.gd 1537-1558): within 3.5 m of the camera
                    let (eye, (_, right, _)) = (self.eye(), self.camera_basis());
                    let hp = self.hunters[h].pos;
                    let side = (hp.0 - eye.x) * right.x + (hp.1 - eye.z) * right.z;
                    let gap =
                        ((hp.0 - eye.x).powi(2) + (1.5 - eye.y).powi(2) + (hp.1 - eye.z).powi(2))
                            .sqrt();
                    crate::audio::sound::inspect_breath(
                        side,
                        gap,
                        self.hunters[h].def.id == "nyxie",
                    );
                    self.hunters[h].apply_stun(prof.hide_tell, false);
                    self.say("THE LOCKER RATTLES");
                }
            }
        }
    }

    pub(super) fn end_run(&mut self, hunter: usize, cause: &'static str) {
        let a = self.admin_flags();
        if a.god || a.no_target {
            // admin god mode / no target: nothing ends the run
            self.escape = None;
            return;
        }
        self.p.flashlight_on = false;
        // THE RUN SAVES ITSELF // DEATH ENDS IT
        crate::shared::checkpoint::clear();
        let h = &self.hunters[hunter];
        self.report = Some(Report {
            hunter: h.def.id.clone(),
            hunter_name: h.def.name.clone(),
            cause,
            depth: self.depth,
            score: self.score,
            relays: self.relays_collected,
            coins: self.coins,
            seed: self.run_seed,
            difficulty: self.difficulty,
        });
    }
}

/// The ability a hunter is using right now, for the host's world events: a Loxelle lunge (`hunter_charge`) or a Nightmare
/// pounce (`hunter_ability`, Nyxie and Solyth). None for every other hunter and while it is idle. The names are the
/// event kinds in `data/net/hunter_events.json`, so their difficulties and gaps come from the data.
pub(super) fn ability_held(b: &crate::hunters::brain::Brain) -> Option<(&'static str, &'static str)> {
    match &b.gimmick {
        Gimmick::Loxelle(_) if b.gimmick.holds_position(b) => Some(("hunter_charge", "lunge")),
        Gimmick::Nyxie(_) | Gimmick::Solyth(_) if b.gimmick.holds_position(b) => Some(("hunter_ability", "pounce")),
        _ => None,
    }
}

impl Game {
    /// The sewer water of the runner's level in window cells, from the chunk `water_cache` holds (the source the swim
    /// code reads): `(cell, metres of water)`. Empty off the sewer or with no chunk loaded.
    fn sewer_water_cells(&self) -> Vec<((i32, i32), f32)> {
        use crate::world::route::sewer::{core as sewer_core, water};
        let level = self.runner_level();
        let Some((key, Some(ch))) = &self.water_cache else { return vec![] };
        if key.1 != level {
            return vec![];
        }
        let o = self.window_origin();
        let mut out = vec![];
        for y in 0..sewer_core::CHUNK {
            for x in 0..sewer_core::CHUNK {
                let d = water::water_depth_at_level(&ch.sewer_grid, x, y, level);
                if d > 0.0 {
                    out.push(((key.0 * sewer_core::CHUNK + x - o.0, key.2 * sewer_core::CHUNK + y - o.1), d));
                }
            }
        }
        out
    }

    /// The window cells each running pipe flow reaches, from the sewer chunk's own flood cells and steps
    /// (`game/hunter_pipes.rs`).
    pub(super) fn hunter_pipe_cells(&self) -> Vec<PipeCells> {
        super::hunter_pipes::cells(self)
    }

    /// Gives the hunters of every resident list the dry field: the water field belongs to the level the runner's chunk
    /// was read for, and a hunter on any other level must not wade through it.
    fn sync_resident_water(&mut self) {
        let dry = self.resident_hunters.dry_field();
        for h in self.resident_hunters.iter_all_mut() {
            h.set_water(dry.clone());
        }
    }

    /// Builds the hunters' water field when something changed, and gives it to every brain (`Brain::set_water`).
    /// The sewer layer and the pipe floods are merged by `water_layers`; while a pipe flow runs the field is rebuilt
    /// every `water_rebuild_seconds`, otherwise only when the chunk, origin, level or hunter count changes. A guest
    /// runs no hunters and builds nothing.
    pub(super) fn sync_hunter_water(&mut self) {
        if self.remote_hunters {
            return;
        }
        let rules = noise_events::rules();
        let pipes = self.hunter_pipe_cells();
        let level = self.runner_level();
        let chunk = match &self.water_cache {
            Some((k, Some(_))) if k.1 == level => Some(*k),
            _ => None,
        };
        let bucket = if pipes.is_empty() { 0 } else { (self.time / rules.water_rebuild_seconds).floor() as i64 + 1 };
        let key = WaterKey {
            chunk,
            origin: self.window_origin(),
            level,
            bucket,
            pipe_cells: pipes.iter().map(|p| p.cells.len()).sum(),
            hunters: self.hunters.len(),
        };
        if self.hunter_water.as_ref().is_some_and(|w| w.key == key) {
            self.sync_resident_water();
            return;
        }
        let sewer = self.sewer_water_cells();
        let mut flood: Vec<((i32, i32), f32)> = vec![];
        for p in &pipes {
            for (cell, step) in &p.cells {
                let d = self.pipe_depth(&p.id, *step, p.outlet);
                if d > 0.0 {
                    flood.push((*cell, d));
                }
            }
        }
        let (depths, flooded) = water_layers(&sewer, &flood, rules.flooded_depth);
        let field = Arc::new(WaterField::from_depths(&depths, &flooded));
        for h in self.hunters.iter_mut() {
            h.set_water(field.clone());
        }
        let n = self.hunters.len();
        let (splash_in, last_pos) = match self.hunter_water.take() {
            Some(w) if w.splash_in.len() == n => (w.splash_in, w.last_pos),
            _ => (vec![0.0; n], self.hunters.iter().map(|h| h.pos).collect()),
        };
        self.hunter_water = Some(HunterWater { key, field, splash_in, last_pos });
        self.sync_resident_water();
    }

    /// The wading splashes of the hunters this frame: a hunter that moved while standing in at least
    /// `hunter_wade_min_depth` of water makes one every `hunter_wade_splash_every` seconds. Only the bots hear them.
    pub(super) fn hunter_wading_events(&mut self, dt: f32) -> Vec<NoiseEvent> {
        let rules = noise_events::rules();
        let level = self.runner_level();
        let Some(w) = self.hunter_water.as_mut() else { return vec![] };
        if w.splash_in.len() != self.hunters.len() {
            return vec![];
        }
        let mut out = vec![];
        for (i, h) in self.hunters.iter().enumerate() {
            let moved = pdist(h.pos, w.last_pos[i]) > 0.001;
            w.last_pos[i] = h.pos;
            let depth = { let c = crate::hunters::brain::cell_of(h.pos); w.field.depth_at((c.x, c.y)) };
            if !h.active || !moved || depth < rules.hunter_wade_min_depth {
                w.splash_in[i] = 0.0;
                continue;
            }
            w.splash_in[i] -= dt;
            if w.splash_in[i] <= 0.0 {
                w.splash_in[i] = rules.hunter_wade_splash_every;
                out.push(NoiseEvent::new(NoiseKind::WadingSplash, h.pos, level));
            }
        }
        out
    }

    /// One `HunterLanding` event for every hunter, active or resident, that landed from a catwalk jump this frame
    /// (`Brain::landed`, set once when the jump ends).
    ///
    /// The host takes the landing (`Option::take`): a resident that landed and then fell out of the nearest-within-cap
    /// set is not updated again, and its `landed` would otherwise raise the event every frame.
    pub(super) fn hunter_landing_events(&mut self) -> Vec<NoiseEvent> {
        let active = self.hunters.iter_mut().filter_map(|h| h.landed.take().map(|p| NoiseEvent::new(NoiseKind::HunterLanding, p, h.level)));
        let resident = self
            .resident_hunters
            .iter_all_mut()
            .filter_map(|h| h.landed.take().map(|p| NoiseEvent::new(NoiseKind::HunterLanding, p, h.level)));
        active.chain(resident).collect()
    }

    /// The runner hears a landing on her own level within the kind's radius, as a heavy step (the nearest hunter step's
    /// pan and falloff, scaled by `noise_events.json landing_thud_volume_scale`).
    pub(super) fn play_landing_thuds(&self, events: &[NoiseEvent]) {
        let level = self.runner_level();
        for e in events.iter().filter(|e| e.kind == NoiseKind::HunterLanding && e.level == level) {
            if let Some((pan, volume)) = self.landing_thud(e) {
                crate::audio::sound::hunter_footstep(pan, volume);
            }
        }
    }

    /// The pan and volume of the runner's thud for one landing: the footstep falloff of the hunter step cue at that
    /// offset, scaled by `landing_thud_volume_scale`; None beyond the kind's radius or past the footstep's range.
    pub(super) fn landing_thud(&self, e: &NoiseEvent) -> Option<(f32, f32)> {
        let rules = noise_events::rules();
        if pdist(e.pos, self.p.pos) > rules.radius_of(e, 1.0) {
            return None;
        }
        let offset = ((e.pos.0 - self.p.pos.0) as f64, (e.pos.1 - self.p.pos.1) as f64);
        let facing = self.forward2();
        let (pan, volume) = crate::audio::cues::rules::hunter_step_pan_volume(offset, (facing.0 as f64, facing.1 as f64))?;
        Some((pan as f32, volume as f32 * rules.landing_thud_volume_scale))
    }

    /// The host's hunters and the bots hear this frame's noise events (`hearing` is the difficulty's `hunter_hearing`):
    /// a hunter in range investigates or searches from the sound (`noise_events::react`), a bot turns toward it. Returns
    /// how many hunters reacted. A guest changes nothing: its hunters only display the host's frames. No event ever
    /// chases, raises the danger readout or starts the heartbeat or chase cue.
    pub(super) fn hunters_hear_noise(&mut self, events: &[NoiseEvent], hearing: f32) -> usize {
        if self.remote_hunters || events.is_empty() {
            return 0;
        }
        self.npc_hear(events);
        let level = self.runner_level();
        let mut reacted = noise_events::react(&mut self.hunters, events, level, hearing, &self.fac);
        // every resident floor hears too: its own level's sounds at the full radius, a storey away at the adjacent scale
        for l in self.resident_hunters.levels() {
            let (ids, mut brains): (Vec<String>, Vec<crate::hunters::brain::Brain>) =
                std::mem::take(self.resident_hunters.list_mut(l)).into_iter().map(|s| (s.id, s.brain)).unzip();
            if let Some((fac, _)) = self.level_parts(l) {
                reacted += noise_events::react(&mut brains, events, l, hearing, fac);
            }
            *self.resident_hunters.list_mut(l) =
                ids.into_iter().zip(brains).map(|(id, brain)| Slot { id, brain }).collect();
        }
        reacted
    }

    /// Producer accessor for the integrator: a noise event at `pos` on the runner's level, from the host's own
    /// simulation. A guest's runner never causes an event for the host's hunters (the host sees only its own). A sound
    /// that happens on another storey goes through `push_noise_at`.
    pub fn push_noise(&mut self, kind: NoiseKind, pos: (f32, f32)) {
        if self.remote_hunters {
            return;
        }
        let level = self.runner_level();
        self.push_noise_at(kind, pos, level);
    }
}

#[cfg(test)]
mod noise_react_tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::hunters::brain::Brain;
    use crate::world::catalog::Catalog;

    fn game() -> (Game, Content) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        g.hunters.clear();
        let mut h = Brain::new(content.hunters[0].clone(), (5.5, 5.5), "T", Difficulty::Standard);
        h.active = true;
        h.state = State::Wander;
        g.hunters.push(h);
        (g, content)
    }

    #[test]
    fn layers_take_the_deepest_water_and_flood_only_pipe_cells_past_the_threshold() {
        let (depths, flooded) = water_layers(&[((1, 1), 0.7), ((2, 1), 0.7)], &[((2, 1), 0.9), ((3, 1), 0.1), ((4, 1), 0.4)], 0.25);
        assert_eq!(depths, vec![((1, 1), 0.7), ((2, 1), 0.9), ((3, 1), 0.1), ((4, 1), 0.4)]);
        assert_eq!(flooded, vec![(2, 1), (4, 1)]);
        let (_, none) = water_layers(&[((1, 1), 0.7)], &[], 0.25);
        assert!(none.is_empty(), "the sewer's own water is not a flood");
        let (_, all) = water_layers(&[], &[((3, 1), 0.1)], 0.05);
        assert_eq!(all, vec![(3, 1)], "a changed threshold changes what is flooded");
    }

    #[test]
    fn the_water_field_is_built_once_and_rebuilt_when_the_hunters_change() {
        let (mut g, content) = game();
        g.sync_hunter_water();
        let first = g.hunter_water.as_ref().expect("a field was built").field.clone();
        g.sync_hunter_water();
        assert!(Arc::ptr_eq(&first, &g.hunter_water.as_ref().unwrap().field), "nothing changed: no rebuild");
        let mut h = Brain::new(content.hunters[0].clone(), (6.5, 5.5), "U", Difficulty::Standard);
        h.active = true;
        g.hunters.push(h);
        g.sync_hunter_water();
        assert!(!Arc::ptr_eq(&first, &g.hunter_water.as_ref().unwrap().field), "a new hunter gets the field");
    }

    #[test]
    fn a_plunge_splash_in_range_makes_the_host_hunter_search_the_point() {
        let (mut g, _) = game();
        g.push_noise(NoiseKind::PlungeSplash, (6.5, 5.5));
        let events = std::mem::take(&mut g.noise_events);
        assert_eq!(events.len(), 1);
        assert_eq!(g.hunters_hear_noise(&events, 1.0), 1);
        assert_eq!(g.hunters[0].state, State::Search);
        assert_eq!(g.hunters[0].last_known, Some((6.5, 5.5)));
    }

    #[test]
    fn a_splash_out_of_range_changes_nothing() {
        let (mut g, _) = game();
        g.push_noise(NoiseKind::PlungeSplash, (500.5, 5.5));
        let events = std::mem::take(&mut g.noise_events);
        assert_eq!(g.hunters_hear_noise(&events, 1.0), 0);
        assert_eq!(g.hunters[0].state, State::Wander);
    }

    #[test]
    fn a_catwalk_landing_is_an_event_for_the_bots_and_the_runner_and_never_queued_for_hunters() {
        let (mut g, content) = game();
        let mut other = Brain::new(content.hunters[1].clone(), (6.5, 5.5), "L", Difficulty::Standard);
        other.active = true;
        other.state = State::Wander;
        g.hunters.push(other);
        assert!(g.hunter_landing_events().is_empty(), "nothing landed");
        g.hunters[0].landed = Some((7.5, 5.5));
        let ev = g.hunter_landing_events();
        assert!(g.hunters[0].landed.is_none(), "the host took the landing");
        assert!(g.hunter_landing_events().is_empty(), "a landing is read once");
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].kind, ev[0].pos, ev[0].level), (NoiseKind::HunterLanding, (7.5, 5.5), g.hunters[0].level));
        // the hunters' own queue (what hunters_hear_noise reads) is untouched: only bots and the runner get the event
        assert!(g.noise_events.is_empty());
        g.play_landing_thuds(&ev); // the runner's thud: no panic with no audio engine
        // the computed level: the footstep falloff at that offset times the data scale, nothing past the radius
        let scale = noise_events::rules().landing_thud_volume_scale;
        let (here, near) = (g.p.pos, ev[0].clone());
        let at = |d: f32| NoiseEvent::new(NoiseKind::HunterLanding, (here.0 + d, here.1), near.level);
        let (_, v0) = g.landing_thud(&at(0.0)).expect("a landing on top of the runner is heard");
        let (_, v4) = g.landing_thud(&at(4.0)).expect("a landing 4 m away is heard");
        let step = |d: f64| crate::audio::cues::rules::hunter_step_pan_volume((d, 0.0), (0.0, 1.0)).unwrap().1 as f32;
        assert!((v0 - step(0.0) * scale).abs() < 1e-6 && (v4 - step(4.0) * scale).abs() < 1e-6);
        assert!(v4 < v0, "the thud falls off with distance");
        assert!(g.landing_thud(&at(1000.0)).is_none(), "beyond the radius nothing is heard");
        let rule = noise_events::rules().kind(NoiseKind::HunterLanding);
        assert_eq!(rule.response, noise_events::Response::Investigate);
        assert!(rule.radius > 0.0 && noise_events::rules().landing_thud_volume_scale > 0.0);
        assert_eq!(g.hunters[1].state, State::Wander, "the other hunter does not react");
        frames_one(&mut g);
        assert!(g.hunters[0].landed.is_none(), "the next frame clears it");
    }

    fn frames_one(g: &mut Game) {
        let prof = g.prof();
        g.update_hunters(1.0 / 60.0, &prof, &Sounds::default());
    }

    #[test]
    fn a_guest_never_changes_a_host_hunter_through_noise_events() {
        let (mut g, _) = game();
        g.remote_hunters = true;
        g.push_noise(NoiseKind::PipeBurst, (6.5, 5.5));
        assert!(g.noise_events.is_empty(), "a guest records no events");
        let ev = NoiseEvent::new(NoiseKind::PipeBurst, (6.5, 5.5), g.runner_level());
        assert_eq!(g.hunters_hear_noise(&[ev], 1.0), 0);
        assert_eq!(g.hunters[0].state, State::Wander);
        g.sync_hunter_water();
        assert!(g.hunter_water.is_none(), "a guest builds no hunter water field");
    }

    #[test]
    fn a_noise_event_never_raises_the_danger_readout_or_starts_a_chase() {
        let (mut g, _) = game();
        let danger = g.danger();
        for k in NoiseKind::ALL {
            g.hunters[0].state = State::Wander;
            let ev = NoiseEvent::new(k, (6.0, 5.5), g.runner_level());
            g.hunters_hear_noise(&[ev], 1.0);
            assert_ne!(g.hunters[0].state, State::Chase, "{}", k.id());
        }
        // A hunter that goes to look at a noise is Investigating, which the readout rates as before (base 0.4 plus
        // proximity); the events never put a hunter in Chase, which is what would push it past that.
        assert!(g.danger() >= danger);
        assert!(g.hunters.iter().all(|h| h.state != State::Chase));
    }

    #[test]
    fn a_data_change_to_the_radius_changes_who_reacts() {
        let (mut g, _) = game();
        let far = NoiseEvent::new(NoiseKind::ValveTurn, (5.5 + 8.0, 5.5), g.runner_level());
        let normal = noise_events::rules().radius_of(&far, g.hunters[0].hearing_scale);
        let reacts = normal >= 8.0;
        assert_eq!(g.hunters_hear_noise(&[far.clone()], 1.0) == 1, reacts);
        g.hunters[0].state = State::Wander;
        assert_eq!(g.hunters_hear_noise(&[far.with_loudness(0.0)], 1.0), 0, "a silent event is not heard");
    }
}

#[cfg(test)]
mod remote_hunter_tests {
    use super::super::Controls;
    use super::*;
    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use crate::shared::clock::Instant;
use std::time::Duration;

    fn run() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(
            crate::config::Difficulty::Standard,
            7,
            "ROOM-TEST-7",
            0,
            &content,
            &catalog,
        );
        // Standard activates route hunters after one relay (Godot GameConfig.streamed_hunter_population), so a
        // fresh run has none; the first relay admits the route hunter the frames below address
        let mut g = g;
        g.relays_collected = 1;
        g.ensure_hunters(&content);
        (g, content, catalog)
    }

    #[test]
    fn a_hunter_chasing_a_bot_raises_no_danger_and_one_chasing_the_runner_does() {
        let (mut g, _, _) = run();
        assert!(!g.hunters.is_empty());
        g.hunters[0].active = true;
        g.hunters[0].state = State::Chase;
        g.hunters[0].pos = (g.p.pos.0 + 2.0, g.p.pos.1);
        g.hunters[0].gimmick = crate::hunters::gimmicks::Gimmick::None;
        let audio = |g: &Game| -> (Option<f64>, bool) {
            let list: Vec<crate::audio::world::HunterAudio> = g
                .hunters
                .iter()
                .map(|h| crate::audio::world::HunterAudio {
                    id: &h.def.id, pos: h.pos, active: h.active, chasing: h.state == State::Chase, idle: false,
                    direct_visible: true, winded: false, stunned: false, in_vent: false,
                    nyxie_empty_wake: false, glyra_humming: false, targets_runner: !h.target_is_bot,
                })
                .collect();
            crate::audio::world::runner_danger_inputs(g.p.pos, &list)
        };
        g.hunters[0].target_is_bot = true;
        let (near, pursued) = audio(&g);
        assert!(near.is_none() && !pursued);
        assert_eq!(g.danger(), 0.0);
        g.hunters[0].target_is_bot = false;
        let (near, pursued) = audio(&g);
        assert!(near.is_some() && pursued);
        assert!(g.danger() > 0.75);
    }

    #[test]
    fn a_guest_applies_the_host_frame_to_the_hunter_at_its_index() {
        let (mut g, _, _) = run();
        assert!(!g.hunters.is_empty());
        let id = g.hunters[0].def.id.clone();
        let frame = vec![json!({
            "index": 0, "id": id, "position": [3.5, 4.5], "facing": [0.0, 1.0],
            "state": "CHASE", "active": true, "stun": 0.25, "slow_factor": 0.5, "disguise": null
        })];
        g.apply_hunter_frame(&frame);
        let h = &g.hunters[0];
        assert_eq!(h.pos, (3.5, 4.5));
        assert_eq!(h.facing, (0.0, 1.0));
        assert!(matches!(h.state, State::Chase));
        assert!(h.active);
        assert_eq!(h.stun, 0.25);
        assert_eq!(h.slow_factor, 0.5);
        assert_eq!(h.disguise, None);
    }

    #[test]
    fn a_mismatched_id_an_out_of_range_index_or_a_null_field_leaves_the_hunter_as_it_was() {
        let (mut g, _, _) = run();
        let id = g.hunters[0].def.id.clone();
        let before_pos = g.hunters[0].pos;
        let before_state = hunter_events::state_name(g.hunters[0].state);
        g.apply_hunter_frame(&[json!({"index": 0, "id": "not-a-hunter", "position": [9.0, 9.0], "state": "CHASE"})]);
        assert_eq!(g.hunters[0].pos, before_pos);
        assert_eq!(hunter_events::state_name(g.hunters[0].state), before_state);
        g.apply_hunter_frame(&[json!({"index": 0, "id": id, "position": null})]);
        assert_eq!(g.hunters[0].pos, before_pos, "a null position is not applied");
        g.apply_hunter_frame(&[json!({"index": 999, "id": id, "position": [1.0, 1.0]})]);
    }

    #[test]
    fn a_guest_shows_each_state_name_the_host_frame_carries() {
        let (mut g, _, _) = run();
        let id = g.hunters[0].def.id.clone();
        for name in ["IDLE", "WANDER", "INVESTIGATE", "CHASE", "SEARCH"] {
            g.apply_hunter_frame(&[json!({"index": 0, "id": id, "state": name})]);
            assert_eq!(hunter_events::state_name(g.hunters[0].state), name);
        }
    }

    #[test]
    fn the_host_frame_reaches_the_guest_unchanged() {
        let (host, _, _) = run();
        let frame = host.hunter_frame();
        let (mut guest, _, _) = run();
        for h in guest.hunters.iter_mut() {
            h.pos = (0.5, 0.5);
            h.state = State::Idle;
        }
        guest.apply_hunter_frame(&frame);
        for (i, h) in host.hunters.iter().enumerate() {
            assert_eq!(guest.hunters[i].pos, h.pos);
            assert_eq!(hunter_events::state_name(guest.hunters[i].state), hunter_events::state_name(h.state));
        }
    }

    #[test]
    fn a_guest_runs_no_hunter_simulation_and_the_host_does() {
        let (mut guest, content, catalog) = run();
        guest.remote_hunters = true;
        let sounds = crate::audio::Sounds::default();
        let before: Vec<((f32, f32), &'static str)> = guest
            .hunters
            .iter()
            .map(|h| (h.pos, hunter_events::state_name(h.state)))
            .collect();
        for _ in 0..300 {
            guest.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &sounds);
        }
        for (i, (pos, state)) in before.iter().enumerate() {
            assert_eq!(guest.hunters[i].pos, *pos, "guest hunter {i} moved");
            assert_eq!(hunter_events::state_name(guest.hunters[i].state), *state, "guest hunter {i} changed state");
        }
        assert!(guest.escape.is_none() && guest.report.is_none(), "a guest captures nothing");
        // the same steps on the host move or change at least one hunter, so the flag is what holds them still
        let (mut host, content, catalog) = run();
        for _ in 0..300 {
            host.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &sounds);
        }
        let moved = before.iter().enumerate().any(|(i, (pos, state))| {
            host.hunters.get(i).is_some_and(|h| h.pos != *pos || hunter_events::state_name(h.state) != *state)
        });
        assert!(moved, "the host simulation still runs");
    }

    #[test]
    fn the_frame_rate_is_the_one_in_the_data_file() {
        let text = include_str!("../data/net/hunter_events.json");
        let standard = hunter_events::parse(text).unwrap();
        let slow = hunter_events::parse(&text.replace("\"rate_hz\": 20", "\"rate_hz\": 10")).unwrap();
        let t0 = Instant::now();
        let mut a = hunter_events::Tracker::default();
        let mut b = hunter_events::Tracker::default();
        assert!(a.frame_due(&standard, t0));
        assert!(b.frame_due(&slow, t0));
        let t1 = t0 + Duration::from_millis(60);
        assert!(a.frame_due(&standard, t1), "20 Hz: a frame every 50 ms");
        assert!(!b.frame_due(&slow, t1), "10 Hz: no frame before 100 ms");
    }

    #[test]
    fn a_guest_queues_no_host_world_events() {
        let (mut guest, content, catalog) = run();
        guest.remote_hunters = true;
        let sounds = crate::audio::Sounds::default();
        for _ in 0..300 {
            guest.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &sounds);
        }
        assert!(guest.hunter_out.take().is_empty(), "a guest never raises the host's events");
    }

    #[test]
    fn a_hunter_at_a_locker_queues_the_locker_check_and_the_door_pull() {
        let (mut g, _, _) = run();
        let li = 0usize;
        let centre = g.fac.hiding[li].cell.center();
        g.p.hidden = Some(li);
        g.hide_check = None;
        g.locker_cooldown[li] = 0.0;
        g.hunters[0].active = true;
        g.hunters[0].stun = 0.0;
        g.hunters[0].state = State::Chase;
        g.hunters[0].pos = centre;
        g.hunters[0].last_known = Some(centre);
        let prof = g.prof();
        let sounds = crate::audio::Sounds::default();
        // a hunter that tracked the runner to this locker starts the check (the locker event)
        g.update_hidden(0.0, &Controls::default(), &prof, &sounds);
        let queued = g.hunter_out.take();
        assert!(
            queued.iter().any(|e| e["what"] == json!("hunter_locker") && e["index"] == json!(0) && e["spot"] == json!(li)),
            "the check queues hunter_locker: {queued:?}"
        );
        // the check's tell runs out and the hunter pulls the door (the door event, action open)
        g.hide_check = Some((li, 0, 0.0));
        g.update_hidden(0.0, &Controls::default(), &prof, &sounds);
        let queued = g.hunter_out.take();
        assert!(
            queued.iter().any(|e| e["what"] == json!("hunter_door") && e["action"] == json!("open")),
            "the pull queues hunter_door: {queued:?}"
        );
    }

    #[test]
    fn a_hidden_look_is_held_to_the_window_cone_and_frozen_while_the_locker_shakes() {
        use crate::render::hiding_cam;
        let (mut g, _, _) = run();
        let li = 0usize;
        let facing = (g.fac.hiding[li].facing.0 as f32, g.fac.hiding[li].facing.1 as f32);
        let base = hiding_cam::facing_yaw(facing);
        let (ym, pm) = (hiding_cam::cfg().yaw_deg.to_radians(), hiding_cam::cfg().pitch_deg.to_radians());
        g.p.hidden = Some(li);
        g.remote_hunters = true; // no hunter acts, so the only thing under test is the look
        let (prof, sounds) = (g.prof(), crate::audio::Sounds::default());
        // looking far off to the side and straight up is pulled back to the data's cone
        g.p.yaw = base + 2.5;
        g.p.pitch = 1.4;
        g.update_hidden(0.0, &Controls::default(), &prof, &sounds);
        let wrap = |a: f32| (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        assert!((wrap(g.p.yaw - base) - ym).abs() < 1e-4, "yaw held at the cone edge");
        assert!((g.p.pitch - pm).abs() < 1e-4, "pitch held at the cone edge");
        // a look inside the cone is left alone
        g.p.yaw = base + 0.2;
        g.p.pitch = -0.1;
        g.update_hidden(0.0, &Controls::default(), &prof, &sounds);
        assert!((g.p.yaw - (base + 0.2)).abs() < 1e-5 && (g.p.pitch + 0.1).abs() < 1e-6);
        // the look never moves the eye: the camera position is the spot's fixed centre (the shake is a model transform)
        let l = g.fac.hiding[li].clone();
        let centre = g.fac.locker_center(&l);
        let eye = hiding_cam::eye_position(centre, facing, "locker");
        let (a, ..) = hiding_cam::view_from(g.eye(), 1.0, (g.p.yaw, g.p.pitch), centre, facing, "locker");
        let (b, ..) = hiding_cam::view_from(g.eye(), 1.0, (g.p.yaw + 0.3, g.p.pitch), centre, facing, "locker");
        assert!((a - eye).length() < 1e-4, "the eye sits at the spot's fixed point");
        assert!((a - b).length() < 1e-6, "turning the head does not move the eye");
        // while the locker rattles the look is frozen (the data flag), so the shake cannot drag the view either
        if hiding_cam::cfg().lock_look_while_shaking {
            g.hide_check = Some((li, 0, 5.0));
            g.p.yaw = base + 2.5;
            g.p.pitch = 1.4;
            g.update_hidden(0.0, &Controls::default(), &prof, &sounds);
            assert_eq!((g.p.yaw, g.p.pitch), (base + 2.5, 1.4), "frozen, not clamped, while it shakes");
        }
    }

    #[test]
    fn a_hunters_locker_check_is_the_elapsed_tell_on_that_locker_only() {
        let (mut g, _, _) = run();
        let li = 0usize;
        let tell = g.prof().hide_tell;
        assert_eq!(g.hide_check_elapsed(li), None, "no check: the locker is still");
        g.hide_check = Some((li, 0, tell));
        assert_eq!(g.hide_check_elapsed(li), Some(0.0), "the tell has just begun");
        g.hide_check = Some((li, 0, tell * 0.5));
        assert!((g.hide_check_elapsed(li).unwrap() - tell * 0.5).abs() < 1e-6);
        assert_eq!(g.hide_check_elapsed(li + 1), None, "another locker is not checked");
    }

    #[test]
    fn a_locker_breacher_pulls_the_hidden_runner_out_before_the_struggle() {
        let (mut g, content, _) = run();
        let li = 0usize;
        let centre = g.fac.hiding[li].cell.center();
        g.p.hidden = Some(li);
        g.p.pos = centre;
        g.hunters.clear();
        assert!(crate::app::stress::summon(&mut g, "elise", &content, 0, 0));
        g.hunters[0].pos = centre;
        g.hide_check = Some((li, 0, 5.0));
        let admin = Admin::default();
        g.resolve_capture(0, &admin);
        assert!(g.escape.is_some(), "the struggle starts");
        assert!(g.p.hidden.is_none(), "the runner left the locker");
        assert!(g.hide_check.is_none());
        assert_eq!(g.locker_cooldown[li], g.prof().hide_cooldown, "the locker cools down as after a door pull");
        let sounds = crate::audio::Sounds::default();
        g.update_escape(1.0 / 60.0, &Controls::default(), &sounds);
        assert!(g.p.hidden.is_none(), "the runner stays out while the struggle runs");
    }

    #[test]
    fn a_pulled_out_runner_does_not_stand_inside_the_hunter() {
        let (mut g, _, _) = run();
        let li = 0usize;
        let usual = crate::hunters::gimmicks::hiding_exit(&g.fac, li);
        let apart = HUNTER_RADIUS + PLAYER_RADIUS;
        // the hunter standing on the usual exit
        let exit = capture_exit(&g.fac, li, usual);
        assert!(pdist(exit, usual) > 0.0, "the runner moved off the hunter's spot");
        assert!(g.fac.body_fits(exit.0, exit.1, PLAYER_RADIUS), "and onto open floor");
        assert!(pdist(exit, usual) >= apart || pdist(exit, usual) > 0.5, "clear of the hunter: {}", pdist(exit, usual));
        // a hunter elsewhere leaves the usual exit alone
        let far = (usual.0 + 5.0, usual.1 + 5.0);
        assert_eq!(capture_exit(&g.fac, li, far), usual);
        g.p.hidden = Some(li);
        g.hunters.clear();
        let content = crate::content::Content::load();
        assert!(crate::app::stress::summon(&mut g, "elise", &content, 0, 0));
        g.hunters[0].pos = usual;
        g.resolve_capture(0, &Admin::default());
        assert!(pdist(g.p.pos, usual) > 0.0, "pulled out beside the hunter, not inside it");
    }

    #[test]
    fn a_guest_in_a_locker_starts_no_check_and_leaves_every_hunter_alone() {
        let (mut g, _, _) = run();
        let li = 0usize;
        let centre = g.fac.hiding[li].cell.center();
        g.remote_hunters = true;
        g.p.hidden = Some(li);
        g.hide_check = None;
        g.locker_cooldown[li] = 0.0;
        g.hunters[0].active = true;
        g.hunters[0].stun = 0.0;
        g.hunters[0].state = State::Chase;
        g.hunters[0].pos = centre;
        g.hunters[0].last_known = Some(centre);
        let (prof, sounds) = (g.prof(), crate::audio::Sounds::default());
        g.update_hidden(0.0, &Controls::default(), &prof, &sounds);
        assert!(g.hide_check.is_none(), "only the host decides that a hunter checks a locker");
        assert!(g.hunter_out.take().is_empty(), "no hunter event from a guest");
        assert_eq!(g.hunters[0].state, State::Chase);
        assert_eq!(g.hunters[0].stun, 0.0, "the guest did not stun the hunter");
    }

    #[test]
    fn a_hunter_after_a_bot_is_no_danger_to_a_runner_in_a_locker() {
        let (mut g, _, _) = run();
        g.p.hidden = Some(0);
        let centre = g.fac.hiding[0].cell.center();
        g.hunters[0].active = true;
        g.hunters[0].state = State::Chase;
        g.hunters[0].pos = centre;
        g.hunters[0].target_is_bot = true;
        for h in g.hunters.iter_mut().skip(1) {
            h.active = false;
        }
        assert_eq!(g.danger(), 0.0, "a hunter chasing an NPC teammate feeds no danger");
        g.hunters[0].target_is_bot = false;
        assert!(g.danger() > 0.0, "the same hunter after the runner does");
    }

    #[test]
    fn only_a_lunge_or_a_pounce_counts_as_an_ability() {
        use crate::hunters::gimmicks::loxelle::Loxelle;
        let (g, _, _) = run();
        // an idle hunter holds no ability, and a Loxelle that has not wound up holds none either
        for h in g.hunters.iter() {
            assert_eq!(ability_held(h), None, "{} is idle", h.def.id);
        }
        let mut g = g;
        g.hunters[0].gimmick = Gimmick::Loxelle(Loxelle::default());
        assert_eq!(ability_held(&g.hunters[0]), None, "a Loxelle that has not wound up is not lunging");
    }
}

/// Where a runner pulled out of locker `spot` stands: the usual exit, or, when the hunter that pulled her out is on it,
/// the open spot round the locker that is farthest from the hunter (a body inside the hunter would push out unpredictably).
fn capture_exit(fac: &crate::world::facility::Facility, spot: usize, hunter: (f32, f32)) -> (f32, f32) {
    let usual = crate::hunters::gimmicks::hiding_exit(fac, spot);
    let apart = HUNTER_RADIUS + PLAYER_RADIUS;
    if pdist(usual, hunter) >= apart {
        return usual;
    }
    let (cx, cy) = fac.hiding[spot].cell.center();
    let mut best = usual;
    for reach in [0.75_f32, 1.2] {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let (x, y) = (cx + a.cos() * reach, cy + a.sin() * reach);
            if fac.body_fits(x, y, PLAYER_RADIUS) && pdist((x, y), hunter) > pdist(best, hunter) {
                best = (x, y);
            }
        }
    }
    best
}

#[cfg(test)]
mod resident_floor_tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::player::stair_height_at;
    use crate::hunters::brain::Brain;
    use crate::hunters::flight_edges;
    use crate::world::catalog::Catalog;
    use crate::world::facility::LinkKind;
    use crate::world::Cell;

    const FRAME: f32 = 1.0 / 60.0;

    /// A level-0 window with the body on a flight going down and level -1 resident beside it, its lineup as built.
    fn stacked_raw() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        for n in 0..60 {
            let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-WALK-{n}"), 0, &content, &catalog);
            let Some(s) = g.fac.stairs.iter().copied().find(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs)) else {
                continue;
            };
            let path = s.walk_path(g.fac.level, 0.25);
            let p = path[path.len() / 4];
            g.p.pos = p;
            let Some(h) = stair_height_at(&g.fac, p) else { continue };
            g.p.height = h;
            g.keep_stack(&content, &catalog);
            g.keep_stack(&content, &catalog);
            if !g.inf.as_ref().unwrap().stack.has(-1) {
                continue;
            }
            return (g, content, catalog);
        }
        panic!("no test window has a flight going down with its neighbour resident");
    }

    /// `stacked_raw` with no hunters anywhere, so a test adds exactly the ones it checks.
    fn stacked() -> (Game, Content, Catalog) {
        let (mut g, content, catalog) = stacked_raw();
        g.hunters.clear();
        if let Some(inf) = g.inf.as_mut() {
            inf.loaded_mut().clear();
        }
        for level in g.resident_hunters.levels() {
            g.resident_hunters.list_mut(level).clear();
        }
        (g, content, catalog)
    }

    fn hunter(content: &Content, def: usize, at: (f32, f32), level: i32, stamina: f32) -> Brain {
        let mut h = Brain::new(content.hunters[def].clone(), at, "T", Difficulty::Standard);
        h.active = true;
        h.state = State::Wander;
        h.level = level;
        h.y = floor_y_of(level);
        h.last_known_level = level;
        h.stamina = stamina;
        h
    }

    /// A hunter that picks its own wander target on its first tick (a bare `Wander` with no target stands still).
    fn ready_to_wander(mut b: Brain) -> Brain {
        b.state = State::Idle;
        b.idle_timer = 0.0;
        b
    }

    /// `n` spread-out cells of `level` a hunter stands in, at least `from_min` m from `from`.
    fn open_spots(g: &Game, level: i32, n: usize, from: (f32, f32), from_min: f32) -> Vec<(f32, f32)> {
        let (fac, _) = g.level_parts(level).expect("the level is held");
        let spots: Vec<(f32, f32)> = fac
            .floor_cells()
            .into_iter()
            .filter(|c| {
                let (x, z) = c.center();
                fac.hunter_open(*c) && fac.body_fits(x, z, HUNTER_RADIUS) && pdist((x, z), from) >= from_min
            })
            .map(|c| c.center())
            .step_by(7)
            .take(n)
            .collect();
        assert_eq!(spots.len(), n, "the level has {n} spread-out open cells");
        spots
    }

    fn slot(brain: Brain) -> Slot {
        Slot { id: String::new(), brain }
    }

    fn frames(g: &mut Game, n: usize) {
        let prof = g.prof();
        let sounds = Sounds::default();
        for _ in 0..n {
            g.update_hunters(FRAME, &prof, &sounds);
        }
    }

    #[test]
    fn a_level_never_visited_gets_a_lineup_on_its_own_floor() {
        let (g, _, _) = stacked_raw();
        let list = g.resident_hunters.list(-1);
        assert!(!list.is_empty(), "the specials at least are built");
        for s in list {
            assert_eq!(s.brain.level, -1, "{} stands on its own level", s.brain.def.id);
            assert_eq!(s.brain.y, floor_y_of(-1));
            assert!(s.id.is_empty(), "off level 0 there is no residency id");
        }
    }

    #[test]
    fn a_hunter_on_the_other_resident_floor_keeps_simulating() {
        let (mut g, content, _) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 8.0)[0];
        let mut dormant = hunter(&content, 0, (3.5, 3.5), 0, 0.5);
        dormant.active = false;
        dormant.state = State::Idle;
        let before = (dormant.pos, dormant.state);
        g.hunters.push(dormant);
        g.resident_hunters.list_mut(-1).push(slot(ready_to_wander(hunter(&content, 0, spot, -1, 1.0))));
        frames(&mut g, 300);
        let now = g.resident_hunters.list(-1);
        assert_eq!(now.len(), 1, "it stayed on its floor");
        assert!(pdist(now[0].brain.pos, spot) > 0.3, "five seconds on, the resident hunter has moved");
        assert_eq!((g.hunters[0].pos, g.hunters[0].state), before, "the active hunters are untouched");
    }

    #[test]
    fn relabel_keeps_every_hunter_and_swaps_lists() {
        let (mut g, content, catalog) = stacked();
        let up = open_spots(&g, 0, 1, g.p.pos, 8.0)[0];
        let down = open_spots(&g, -1, 1, g.p.pos, 8.0)[0];
        let mut a = hunter(&content, 0, up, 0, 0.123);
        a.state = State::Search;
        let mut b = hunter(&content, 1, down, -1, 0.456);
        b.state = State::Investigate;
        let (a_at, b_at) = ((a.pos, a.state), (b.pos, b.state));
        g.hunters.push(a);
        g.resident_hunters.list_mut(-1).push(slot(b));
        // where the hunter with this stamina is: the level of its list, its position and its state
        let find = |g: &Game, stamina: f32| -> Option<(i32, (f32, f32), State)> {
            g.hunters
                .iter()
                .filter(|h| h.stamina == stamina)
                .map(|h| (g.runner_level(), h.pos, h.state))
                .chain(
                    g.resident_hunters
                        .iter_leveled()
                        .filter(|(_, h)| h.stamina == stamina)
                        .map(|(l, h)| (l, h.pos, h.state)),
                )
                .next()
        };
        assert!(g.relabel(-1, &content, &catalog), "down to the resident neighbour");
        assert_eq!(find(&g, 0.456), Some((-1, b_at.0, b_at.1)), "the neighbour's hunter is now in the active list, untouched");
        assert_eq!(find(&g, 0.123), Some((0, a_at.0, a_at.1)), "the old active hunter is the resident list's, untouched");
        assert!(g.hunters.iter().any(|h| h.stamina == 0.456));
        assert!(g.resident_hunters.list(0).iter().any(|s| s.brain.stamina == 0.123));
        assert!(g.relabel(0, &content, &catalog), "and back up");
        assert_eq!(find(&g, 0.456), Some((-1, b_at.0, b_at.1)));
        assert_eq!(find(&g, 0.123), Some((0, a_at.0, a_at.1)));
        assert!(g.hunters.iter().any(|h| h.stamina == 0.123), "back in the active list");
        assert!(g.resident_hunters.list(-1).iter().any(|s| s.brain.stamina == 0.456));
    }

    #[test]
    fn a_chase_follows_the_runner_across_a_flight_into_the_active_list() {
        let (mut g, content, _) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 8.0)[0];
        let mut chaser = hunter(&content, 0, spot, -1, 0.9);
        chaser.state = State::Chase;
        g.resident_hunters.list_mut(-1).push(slot(chaser));
        g.move_crossers();
        assert_eq!(g.hunters.len(), 0, "still below: it stays in its list");
        // the flight is walked: the brain's derived level is now the runner's
        {
            let b = &mut g.resident_hunters.list_mut(-1)[0].brain;
            b.level = 0;
            b.y = floor_y_of(0);
        }
        g.move_crossers();
        assert_eq!(g.hunters.len(), 1, "it joined the active list");
        assert!(g.resident_hunters.list(-1).is_empty());
        assert_eq!((g.hunters[0].level, g.hunters[0].state), (0, State::Chase), "its state came with it");
        g.move_crossers();
        assert_eq!(g.hunters.len(), 1, "and one call moved it once");
        // and down again
        g.hunters[0].level = -1;
        g.move_crossers();
        assert!(g.hunters.is_empty());
        assert_eq!(g.resident_hunters.list(-1).len(), 1);
    }

    #[test]
    fn a_crosser_to_a_level_that_is_not_resident_is_stored_at_once() {
        let (mut g, content, _) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 8.0)[0];
        let mut far = hunter(&content, 0, spot, 7, 0.31);
        far.y = floor_y_of(7);
        g.resident_hunters.list_mut(-1).push(slot(far));
        g.move_crossers();
        assert!(g.resident_hunters.list(-1).is_empty());
        let kept = g.inf.as_mut().unwrap().floor_hunters_mut();
        assert_eq!(kept.get(&7).map(Vec::len), Some(1), "level 7 keeps it under its level");
    }

    #[test]
    fn a_hunter_on_a_flight_sees_the_runner_through_the_well() {
        let (g, content, _) = stacked();
        let upper = &g.fac;
        let lower = &g.inf.as_ref().unwrap().stack.get(-1).expect("level -1 is resident").fac;
        let cells = |f: &crate::world::Facility| -> Vec<(i32, i32)> {
            (0..f.rows).flat_map(|y| (0..f.cols).map(move |x| (x, y))).collect()
        };
        let well = cells(upper)
            .into_iter()
            .find(|&(x, y)| upper.is_hole(x, y) && lower.hunter_open(Cell::new(x, y)))
            .expect("the flight's well is a hole in the upper floor over open floor below");
        let solid = cells(upper)
            .into_iter()
            .find(|&(x, y)| {
                let c = Cell::new(x, y);
                upper.hunter_open(c)
                    && !upper.is_hole(x, y)
                    && !upper.is_drop(c)
                    && !upper.open_below(c)
                    && lower.hunter_open(c)
                    && !lower.open_above(c)
            })
            .expect("most of the floor is slab");
        let runner_at = |c: (i32, i32)| {
            let mut t = g.target();
            t.pos = Cell::new(c.0, c.1).center();
            t.level = 0;
            t.y = floor_y_of(0);
            t
        };
        let under = |c: (i32, i32)| hunter(&content, 0, Cell::new(c.0, c.1).center(), -1, 1.0);
        let cross = g.cross_for(&under(well), &runner_at(well)).expect("two levels apart from each other");
        assert!(cross.sees, "straight up through the open well");
        assert_eq!(cross.hear_scale, flight_edges::rules().hear_adjacent_scale, "a storey away is heard at the data scale");
        let blocked = g.cross_for(&under(solid), &runner_at(solid)).expect("two levels");
        assert!(!blocked.sees, "never through a slab");
        let same = hunter(&content, 0, runner_at(solid).pos, 0, 1.0);
        assert_eq!(g.cross_for(&same, &runner_at(solid)), None, "one level: the facility answers");
    }

    #[test]
    fn a_noise_one_floor_down_sends_a_resident_hunter_to_the_flight() {
        let (mut g, content, _) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 6.0)[0];
        g.resident_hunters.list_mut(-1).push(slot(hunter(&content, 0, spot, -1, 1.0)));
        // the splash is on level 0, a storey above the hunter and a metre from it in plan
        let ev = NoiseEvent::new(NoiseKind::PlungeSplash, (spot.0 + 1.0, spot.1), 0);
        assert!(g.hunters_hear_noise(&[ev.clone()], 1.0) >= 1, "heard one storey away at the adjacent scale");
        let b = &g.resident_hunters.list(-1)[0].brain;
        assert_eq!(b.state, State::Investigate, "it walks there first");
        assert_eq!(b.last_known, Some(ev.pos));
        assert_eq!(b.last_known_level, 0, "the point is on the floor above: the way is the flight");
        // two storeys away is not heard
        let far = NoiseEvent::new(NoiseKind::PlungeSplash, spot, 4);
        g.resident_hunters.list_mut(-1)[0].brain.state = State::Wander;
        assert_eq!(g.hunters_hear_noise(&[far], 1.0), 0);
        assert_eq!(g.resident_hunters.list(-1)[0].brain.state, State::Wander);
    }

    /// The secret wolf spiders never leave their home level: a noise a storey above sends an ordinary hunter to the flight, and
    /// a spider on the same floor, next to it, plans no way there and does not stir.
    #[test]
    fn a_wolf_spider_plans_no_way_to_another_level() {
        let (mut g, content, _) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 6.0)[0];
        let spider = content.hunter_index("white_wolf_spider").expect("the spider has a definition");
        let ev = NoiseEvent::new(NoiseKind::PlungeSplash, (spot.0 + 1.0, spot.1), 0);
        g.resident_hunters.list_mut(-1).push(slot(hunter(&content, spider, spot, -1, 1.0)));
        assert_eq!(g.hunters_hear_noise(&[ev.clone()], 1.0), 0, "a spider does not hear a floor above through the flight");
        assert_eq!(g.resident_hunters.list(-1)[0].brain.state, State::Wander);
        g.resident_hunters.list_mut(-1).clear();
        g.resident_hunters.list_mut(-1).push(slot(hunter(&content, 0, spot, -1, 1.0)));
        assert!(g.hunters_hear_noise(&[ev], 1.0) >= 1, "the control: an ordinary hunter does");
    }

    #[test]
    fn resident_cap_limits_ticked_hunters() {
        let (mut g, content, _) = stacked();
        let cap = flight_edges::rules().resident_hunter_cap as usize;
        // spots more than two separations apart: the level's separation moves every hunter it crowds, ticked or not, so
        // a crowded set would move the un-ticked ones too (the railings close some cells that a plain pick would use)
        let spots = {
            let (fac, _) = g.level_parts(-1).expect("the level is held");
            let mut picked: Vec<(f32, f32)> = vec![];
            for c in fac.floor_cells() {
                let p = c.center();
                if fac.hunter_open(c)
                    && fac.body_fits(p.0, p.1, HUNTER_RADIUS)
                    && pdist(p, g.p.pos) >= 8.0
                    && picked.iter().all(|q| pdist(*q, p) >= 2.0 * crate::hunters::director::SEPARATION)
                {
                    picked.push(p);
                    if picked.len() == cap + 3 {
                        break;
                    }
                }
            }
            assert_eq!(picked.len(), cap + 3, "the level has room for {} hunters apart", cap + 3);
            picked
        };
        for (i, s) in spots.iter().enumerate() {
            g.resident_hunters.list_mut(-1).push(slot(ready_to_wander(hunter(&content, 0, *s, -1, 1.0 - i as f32 * 0.01))));
        }
        let print = |g: &Game| -> Vec<((f32, f32), Option<(f32, f32)>)> {
            g.resident_hunters.list(-1).iter().map(|s| (s.brain.pos, s.brain.wander_target)).collect()
        };
        let before = print(&g);
        frames(&mut g, 3);
        let after = print(&g);
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert!(changed >= 1, "the nearest hunters tick");
        assert!(changed <= cap, "{changed} hunters changed with a cap of {cap}");
        assert_eq!(g.resident_hunters.list(-1).len(), cap + 3, "the others are kept, only not ticked");
    }

    #[test]
    fn an_evicted_level_stores_its_hunters_by_absolute_cell_and_restores_them() {
        let (mut g, content, catalog) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 8.0)[0];
        g.resident_hunters.list_mut(-1).push(slot(hunter(&content, 0, spot, -1, 0.777)));
        let o = g.inf.as_ref().unwrap().origin();
        g.inf.as_mut().unwrap().stack.residents.clear();
        g.sync_resident_hunters(&content);
        assert!(!g.resident_hunters.has(-1), "the list left with its level");
        {
            let kept = g.inf.as_mut().unwrap().floor_hunters_mut();
            let list = kept.get(&-1).expect("level -1 keeps its hunters");
            assert_eq!(list.len(), 1);
            let at = (spot.0 + o.0 as f32, spot.1 + o.1 as f32);
            assert!(pdist(list[0].pos, at) < 1e-3, "kept at the absolute cell: {:?} vs {:?}", list[0].pos, at);
        }
        g.keep_stack(&content, &catalog);
        g.keep_stack(&content, &catalog);
        let back = g.resident_hunters.list(-1);
        let me = back.iter().find(|s| s.brain.stamina == 0.777).expect("the hunter came back with its level");
        assert!(pdist(me.brain.pos, spot) < 1e-3, "in the window frame again, where it stood");
        assert!(g.inf.as_mut().unwrap().floor_hunters_mut().get(&-1).is_none(), "and out of the store");
    }

    #[test]
    fn hunter_frame_round_trips_level_and_presentation_y() {
        let (mut host, content, _) = stacked();
        let up = open_spots(&host, 0, 1, host.p.pos, 8.0)[0];
        let down = open_spots(&host, -1, 1, host.p.pos, 8.0)[0];
        let mut a = hunter(&content, 0, up, 0, 1.0);
        a.y = 2.5;
        let mut b = hunter(&content, 1, down, -1, 1.0);
        b.y = -3.25;
        host.hunters.push(a);
        host.resident_hunters.list_mut(-1).push(slot(b));
        let frame = host.hunter_frame();
        assert_eq!(frame.len(), 2, "one entry per hunter of every list");
        assert_eq!(frame[0]["index"].as_u64(), Some(0));
        assert_eq!(frame[1]["index"].as_u64(), Some(1), "residents follow the active list");
        assert_eq!(frame[0]["level"].as_i64(), Some(0));
        assert_eq!(frame[1]["level"].as_i64(), Some(-1));
        assert_eq!(frame[0]["presentation_y"].as_f64(), Some(2.5));
        assert_eq!(frame[1]["presentation_y"].as_f64(), Some(-3.25));

        let (mut guest, _, _) = stacked();
        guest.remote_hunters = true;
        let (up, down) = (open_spots(&guest, 0, 1, guest.p.pos, 8.0)[0], open_spots(&guest, -1, 1, guest.p.pos, 8.0)[0]);
        guest.hunters.push(hunter(&content, 0, up, 0, 1.0));
        guest.resident_hunters.list_mut(-1).push(slot(hunter(&content, 1, down, -1, 1.0)));
        guest.apply_hunter_frame(&frame);
        assert_eq!(guest.hunters[0].y, 2.5);
        let r = &guest.resident_hunters.list(-1)[0].brain;
        assert_eq!((r.level, r.y), (-1, -3.25));

        // the resident hunter walked the flight: the next frame says level 0, and the guest moves it to the active list
        let mut next = frame.clone();
        next[1]["level"] = json!(0);
        next[1]["presentation_y"] = json!(0.0);
        guest.apply_hunter_frame(&next);
        assert_eq!(guest.hunters.len(), 2, "it joined the active list");
        assert!(guest.resident_hunters.list(-1).is_empty());
        assert_eq!(guest.hunters[1].level, 0);
    }

    #[test]
    fn an_old_frame_without_level_applies_on_the_viewers_level() {
        let (mut g, content, _) = stacked();
        let up = open_spots(&g, 0, 1, g.p.pos, 8.0)[0];
        let mut h = hunter(&content, 0, up, -1, 1.0);
        h.y = -3.0;
        let id = h.def.id.clone();
        g.hunters.push(h);
        g.apply_hunter_frame(&[json!({"index": 0, "id": id, "position": [3.5, 4.5], "state": "CHASE"})]);
        assert_eq!(g.hunters.len(), 1, "it stays in the active list");
        assert_eq!(g.hunters[0].level, g.runner_level(), "no level key: the viewer's level");
        assert_eq!(g.hunters[0].y, floor_y_of(g.runner_level()), "at that floor's height");
        assert_eq!(g.hunters[0].pos, (3.5, 4.5));
    }

    #[test]
    fn a_guest_never_simulates_resident_hunters() {
        let (mut g, content, _) = stacked();
        let spot = open_spots(&g, -1, 1, g.p.pos, 8.0)[0];
        g.resident_hunters.list_mut(-1).push(slot(hunter(&content, 0, spot, -1, 1.0)));
        g.remote_hunters = true;
        frames(&mut g, 300);
        let b = &g.resident_hunters.list(-1)[0].brain;
        assert_eq!((b.pos, b.state), (spot, State::Wander), "the host's frames move it, not the guest");
        assert!(b.display_only, "and it never starts an emote of its own");
        // a level that left the stack is not stored by a guest either
        g.inf.as_mut().unwrap().stack.residents.clear();
        g.sync_resident_hunters(&content);
        assert!(g.inf.as_mut().unwrap().floor_hunters_mut().get(&-1).is_none(), "a guest stores nothing");
    }

    #[test]
    fn a_guest_shadow_is_shown_but_never_simulated_stored_or_counted() {
        let (mut guest, content, _) = stacked();
        guest.remote_hunters = true;
        let id = content.hunters[0].id.clone();
        let near = (guest.p.pos.0 + 1.0, guest.p.pos.1);
        let frame = |level: i32, at: (f32, f32)| {
            json!({"index": 0, "id": id, "position": [at.0, at.1], "state": hunter_events::state_name(State::Chase), "active": true,
                   "level": level, "presentation_y": floor_y_of(level)})
        };

        // a hunter the guest's own lineup never built, on the floor below: shown, but marked as the host's shadow
        let below = open_spots(&guest, -1, 1, guest.p.pos, 8.0)[0];
        guest.apply_hunter_frame(&[frame(-1, below)]);
        let s = &guest.resident_hunters.list(-1)[0].brain;
        assert!(s.shadow && s.display_only, "made from a frame, display only");
        assert_eq!(s.pos, below);

        // no brain runs for it, whatever the frames or the noise are
        guest.noise_events.push(NoiseEvent::new(NoiseKind::PlungeSplash, below, -1));
        frames(&mut guest, 120);
        let s = &guest.resident_hunters.list(-1)[0].brain;
        assert_eq!((s.pos, s.state), (below, State::Chase), "only the host's frames move it");

        // a shadow on the viewer's own level feeds neither the danger readout nor the chase cue ...
        guest.apply_hunter_frame(&[frame(0, near)]);
        assert_eq!(guest.hunters.len(), 1, "it came up to the viewer's list");
        assert!(guest.hunters[0].shadow);
        assert_eq!(guest.danger(), 0.0, "a shadow chasing at one metre is no danger readout");
        let audio = |g: &Game| -> (Option<f64>, bool) {
            let list: Vec<crate::audio::world::HunterAudio> = g
                .hunters
                .iter()
                .map(|h| crate::audio::world::HunterAudio {
                    id: &h.def.id,
                    pos: h.pos,
                    active: h.active,
                    chasing: h.state == State::Chase,
                    idle: false,
                    direct_visible: true,
                    winded: false,
                    stunned: false,
                    in_vent: false,
                    nyxie_empty_wake: false,
                    glyra_humming: false,
                    targets_runner: !h.target_is_bot && !h.shadow,
                })
                .collect();
            crate::audio::world::runner_danger_inputs(g.p.pos, &list)
        };
        assert_eq!(audio(&guest), (None, false), "and no chase cue");
        // ... while the same hunter of the guest's own lineup does (the control)
        guest.hunters[0].shadow = false;
        assert!(guest.danger() > 0.75);
        assert!(audio(&guest).1, "a lineup hunter chasing from a metre is a chase cue");
        guest.hunters[0].shadow = true;

        // and when its level leaves the stack nothing is stored or restored
        guest.apply_hunter_frame(&[frame(-1, below)]);
        guest.inf.as_mut().unwrap().stack.residents.clear();
        guest.sync_resident_hunters(&content);
        assert!(!guest.resident_hunters.has(-1), "the guest drops the list instead of storing it");
        assert!(guest.inf.as_mut().unwrap().floor_hunters_mut().is_empty(), "a guest stores nothing");
        assert!(guest.inf.as_mut().unwrap().stored_mut().is_empty());
    }

    /// A level-0 window with a three-cell deck made on its floor (the deck cells at 2.5 m, no ladder), the runner on the
    /// floor beside the deck, and one hunter standing on the deck's far cell, chasing her.
    fn deck_chase() -> (Game, crate::world::Cell) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-DECK-CHASE", 0, &content, &catalog);
        let h = crate::world::route::catwalk::params().height;
        let start = g.fac.start.center();
        let row = g
            .fac
            .floor_cells()
            .into_iter()
            .find(|c| {
                pdist(c.center(), start) > 25.0
                    && (-1..=5).all(|k| g.fac.hunter_open(Cell::new(c.x + k, c.y)) && g.fac.hunter_open(Cell::new(c.x + k, c.y + 1)))
            })
            .expect("a roomy stretch of floor away from the start");
        let deck = [Cell::new(row.x + 1, row.y), Cell::new(row.x + 2, row.y), Cell::new(row.x + 3, row.y)];
        for c in deck {
            g.fac.set_walk_surface(c, crate::world::facility::DeckKind::Catwalk, h);
        }
        g.fac.rebuild_grids();
        g.resident_hunters.nav_key = None; // the graph was built before the deck: build it again
        g.hunters.clear();
        if let Some(inf) = g.inf.as_mut() {
            inf.loaded_mut().clear();
        }
        for level in g.resident_hunters.levels() {
            g.resident_hunters.list_mut(level).clear();
        }
        let mut hunter = Brain::new(content.hunters[0].clone(), deck[2].center(), "DECK-CHASE", Difficulty::Standard);
        hunter.active = true;
        hunter.y = h;
        hunter.state = State::Chase;
        g.hunters.push(hunter);
        // the runner two cells below the deck's middle cell, in the row of floor beside it
        let runner = Cell::new(row.x + 2, row.y + 1);
        g.p.pos = runner.center();
        (g, row)
    }

    #[test]
    fn a_deck_hunter_chases_through_update_hunters_and_stays_in_the_level_zero_list() {
        let (mut g, _) = deck_chase();
        let h = crate::world::route::catwalk::params().height;
        let (mut on_deck_dy, mut jumped, mut landings, mut was_jumping) = (None, false, 0, false);
        let prof = g.prof();
        let sounds = Sounds::default();
        for _ in 0..900 {
            g.update_hunters(FRAME, &prof, &sounds);
            assert!(g.resident_hunters.levels().iter().all(|l| g.resident_hunters.list(*l).is_empty()), "never moved to another list");
            assert_eq!(g.hunters.len(), 1, "it stays in the active list");
            let b = &g.hunters[0];
            assert_eq!(b.level, 0, "a deck is on level 0, so no crosser list swap");
            if b.on_deck() && on_deck_dy.is_none() {
                on_deck_dy = Some(crate::render::stack_actors::actor_dy(b.y, g.runner_level()));
            }
            let jumping = b.walk.as_ref().is_some_and(|w| w.kind == flight_edges::EdgeKind::CatwalkJump);
            jumped |= jumping;
            // the host takes `landed` inside the frame (one noise event), so a landing is the jump ending
            landings += (was_jumping && !jumping) as i32;
            was_jumping = jumping;
            if jumped && !b.on_deck() && b.walk.is_none() {
                break;
            }
        }
        assert_eq!(on_deck_dy, Some(h), "drawn at the deck height while on it");
        assert!(jumped, "it jumped down after the runner (no ladder in this window)");
        assert_eq!(landings, 1);
        assert_eq!(g.hunters[0].y, 0.0);
    }
}
