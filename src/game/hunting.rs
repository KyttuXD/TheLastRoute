//! Running the floor's hunters each frame and applying what they do (main_3d.gd): gimmick
//! events, captures and the escape struggle (mash [E] 6 times in 3.2 s), the hide check at a
//! locker, and the run report.

use super::{pdist, Escape, Game, Heater, Report};
use crate::audio::Sounds;
use crate::config::*;
use crate::hunters::brain::{State, Target};
use crate::hunters::gimmicks::{nyxie::Wake, Ctx, Event, Gimmick};

impl Game {
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
        }
    }

    pub(super) fn update_hunters(&mut self, dt: f32, prof: &Profile, _sounds: &Sounds) {
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
        if self.inf.is_some() {
            crate::hunters::director::separate(dt, &mut self.hunters, &self.fac);
        }
        let mut all_events: Vec<(usize, Event)> = vec![];
        let mut caught: Option<usize> = None;
        for i in 0..self.hunters.len() {
            if let Gimmick::Solyth(s) = &mut self.hunters[i].gimmick {
                s.heat_locked = self.meters.heat_locked;
            }
            let ctx = Ctx {
                fac: &self.fac,
                relays_taken: &self.relays_taken,
                batteries_taken: &self.batteries_taken,
                hiding_disabled: &self.hiding_disabled,
                vents: &self.fix.vents,
                heater_on: self.heater == Heater::Powered,
                heater_thawed: self.heater != Heater::Frozen,
                diff: self.difficulty,
                music_box_left: self.music_box_left,
                exposure: self.meters.exposure,
            };
            let (c, ev) =
                self.hunters[i].update(dt, &ctx, &target, self.depth, self.relays_collected, prof);
            if c && caught.is_none() {
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
                heater_on: self.heater == Heater::Powered,
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
                } else {
                    self.hide_check = Some((spot, h, t));
                }
            }
            _ => {
                let centre = self.fac.hiding[li].cell.center();
                if self.locker_cooldown[li] > 0.0 {
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
