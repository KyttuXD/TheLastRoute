//! Running the floor's hunters each frame and applying what they do (main_3d.gd): gimmick
//! events, captures and the escape struggle (mash [E] 6 times in 3.2 s), the hide check at a
//! locker, and the run report.

use super::{pdist, Escape, Game, Heater, Report};
use crate::audio::Sounds;
use crate::config::*;
use crate::hunters::brain::{choose_target, State, Target, TargetRules};
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
    /// The host's hunter frame (Godot `_collect_hunter_report`): one object per hunter, indexed as `hunters` is, with
    /// its id, position, facing, state name, active flag, stun, slow factor and disguise. Guests read it through
    /// [`Game::apply_hunter_frame`].
    pub fn hunter_frame(&self) -> Vec<Value> {
        self.hunters
            .iter()
            .enumerate()
            .map(|(index, h)| {
                json!({
                    "index": index,
                    "id": h.def.id,
                    "position": [h.pos.0, h.pos.1],
                    "facing": [h.facing.0, h.facing.1],
                    "state": hunter_events::state_name(h.state),
                    "active": h.active,
                    "stun": h.stun,
                    "slow_factor": h.slow_factor,
                    "disguise": h.disguise,
                })
            })
            .collect()
    }

    /// Guest: shows the host's hunters. Each entry is applied to the hunter at its `index` when the id matches, and a
    /// field that is missing or not finite is left as it was. Only the display state is set here.
    pub fn apply_hunter_frame(&mut self, data: &[Value]) {
        for entry in data {
            let Some(index) = entry["index"].as_u64().map(|i| i as usize) else { continue };
            let Some(h) = self.hunters.get_mut(index) else { continue };
            if entry["id"].as_str() != Some(h.def.id.as_str()) {
                continue;
            }
            let finite = |v: &Value| v.as_f64().map(|f| f as f32).filter(|f| f.is_finite());
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
        }
    }

    fn target(&self) -> Target {
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
            temperature: self.meters.temperature,
            pesticide: self.p.pesticide > 0.0,
            map_open: self.map_open,
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
        // a guest runs no hunter simulation: the host's frames set every hunter (apply_hunter_frame)
        if self.remote_hunters {
            return;
        }
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
        // the NPC bots on the runner's level are candidate targets too (data/hunters/targets.json); the runner stays the
        // default target, and a bot is chased only where the rules allow it (hunters::brain::choose_target)
        let runner_level = self.runner_level();
        let bot_targets: Vec<Target> = self
            .npcs
            .iter()
            .filter(|n| n.level == runner_level)
            .map(|n| Target { pos: n.at, facing: n.heading, ..Default::default() })
            .collect();
        let bot_pos: Vec<(f32, f32)> = bot_targets.iter().map(|t| t.pos).collect();
        let rules = TargetRules::get();
        let mut all_events: Vec<(usize, Event)> = vec![];
        let mut caught: Option<usize> = None;
        for i in 0..self.hunters.len() {
            let chosen = choose_target(
                rules,
                self.difficulty,
                self.hunters[i].pos,
                target.pos,
                &bot_pos,
                |a, b| self.fac.has_line_of_sight(a, b),
            );
            self.hunters[i].target_is_bot = chosen.is_some();
            let hunter_target = match chosen {
                Some(b) => &bot_targets[b],
                None => &target,
            };
            if let Gimmick::Solyth(s) = &mut self.hunters[i].gimmick {
                s.heat_locked = self.meters.heat_locked;
            }
            let ctx = Ctx {
                fac: &self.fac,
                relays_taken: &self.relays_taken,
                batteries_taken: &self.batteries_taken,
                hiding_disabled: &self.hiding_disabled,
                vents: &self.fix.vents,
                heater_on: self.heater_running(),
                heater_thawed: self.heater != Heater::Frozen,
                diff: self.difficulty,
                music_box_left: self.music_box_left,
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
                    std::time::Instant::now(),
                );
            }
            // a capture ends the run only when the runner was the target: a bot caught is no run end
            if c && chosen.is_none() && caught.is_none() {
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
        // hunter footsteps, heartbeat, chase_start and Nyxie's wake cue: crate::audio::world::update (Game::update)
        if let Some(i) = caught {
            if self.escape_grace > 0.0 || admin.god || admin.no_target {
                return;
            }
            crate::audio::sound::capture();
            if self.hunters[i].gimmick.capture_inescapable() || self.escape_cooldown > 0.0 {
                let cause = if matches!(&self.hunters[i].gimmick, Gimmick::Nyxie(n) if n.wake == Wake::Empty)
                {
                    "lamp"
                } else {
                    "contact"
                };
                self.end_run(i, cause);
            } else {
                self.p.flashlight_on = false;
                self.escape = Some(Escape {
                    remaining: ESCAPE_DURATION,
                    progress: 0,
                    hunter: i,
                });
            }
        }
    }

    fn apply_event(&mut self, from: usize, e: Event) {
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
            self.p.pos = self.fac.nearest_open_or(
                self.p.pos.0 + away.0 * ESCAPE_KNOCKBACK,
                self.p.pos.1 + away.1 * ESCAPE_KNOCKBACK,
                PLAYER_RADIUS,
            );
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

    pub(super) fn update_hidden(
        &mut self,
        dt: f32,
        c: &super::Controls,
        prof: &Profile,
        _sounds: &Sounds,
    ) {
        let li = self.p.hidden.unwrap();
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
                local: self.local_climate(),
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
                        std::time::Instant::now(),
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
                        std::time::Instant::now(),
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
fn ability_held(b: &crate::hunters::brain::Brain) -> Option<(&'static str, &'static str)> {
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

    /// The window cells each running pipe flow reaches. Empty until the pipe geometry is wired (the pipes' step cells
    /// are not stored by `PipeNetwork`; the integrator maps a flow id to its walkway cells).
    pub(super) fn hunter_pipe_cells(&self) -> Vec<PipeCells> {
        vec![]
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
        noise_events::react(&mut self.hunters, events, level, hearing, &self.fac)
    }

    /// Producer accessor for the integrator: a noise event at `pos` on the runner's level, from the host's own
    /// simulation. A guest's runner never causes an event for the host's hunters (the host sees only its own).
    pub fn push_noise(&mut self, kind: NoiseKind, pos: (f32, f32)) {
        if self.remote_hunters {
            return;
        }
        let level = self.runner_level();
        self.noise_events.push(NoiseEvent::new(kind, pos, level));
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
    use std::time::{Duration, Instant};

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
