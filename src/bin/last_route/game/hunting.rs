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
            noise: self.p.noise,
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
            Event::Mimic { rare: _ } => self.say("...did you hear that?"),
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
