//! Classic night rules (Godot `night_watch_classic.gd`, with the hunter from `night_watch_stalker.gd` and the
//! building and cast from `data/night_watch/buildings/classic.json` and `casts/classic.json`). No window or input:
//! the shell feeds `Input` each frame and draws the returned `Event`s. The same seed gives the same night.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use serde_json::Value;

use crate::building_map;
use crate::data::{building, cast, int, list, num, rules, text, CallsData};
use crate::framework::{chance, pick_index, CallEvent, CallScript, NightWatchStream};

const DOORS: [&str; 2] = ["doorL", "doorR"];

#[derive(Debug)]
struct Def {
    id: &'static str,
    doors: Vec<&'static str>,
    start: &'static str,
    retreat_to: &'static str,
    push_to: &'static str,
    fade_to: Vec<&'static str>,
    alternate: bool,
    only_when_cams_down: bool,
    freezes_when_watched: bool,
    levels: Vec<i32>,
}

#[derive(Debug)]
struct Combo {
    a: &'static str,
    b: &'static str,
    min_level: i32,
    patience_ms: f64,
}

/// Night 7's Kathy knobs (`rules/classic.json` "kathy", `buildings/classic.json` "paths.kathy").
#[derive(Debug)]
struct KathyTune {
    level: i32,
    pace_scale: f64,
    grace_scale: f64,
    hold_ms: (f64, f64),
    hold_tail_ms: f64,
    hum_ms: f64,
    dip_cycle_ms: f64,
    dips: Vec<(f64, f64)>,
    miss_lock_ms: f64,
    meet_rooms: Vec<&'static str>,
}

/// Every Classic tunable, read once from the rules, cast and building JSON.
#[derive(Debug)]
struct Tuning {
    nights: u32,
    tuned_nights: u32,
    kathy_night: u32,
    points_per_level: u32,
    night_ms: f64,
    hour_ms: f64,
    power_base: f64,
    power_per_device: f64,
    door_capacity: usize,
    t_closed_ms: f64,
    focus: f64,
    door_grace_ms: Vec<f64>,
    night_pace: Vec<f64>,
    light_push_ms: Vec<f64>,
    ai_bump: Vec<i32>,
    move_window: (f64, f64, f64, f64),
    blackout: (f64, f64, f64),
    nyxie_levels: Vec<i32>,
    /// The floor's hunters in the order the floor wakes them (cast floor slots 0 to 3).
    defs: Vec<Def>,
    kathy_def: Def,
    kathy: KathyTune,
    combos: Vec<Combo>,
    graph: HashMap<&'static str, Vec<&'static str>>,
}

fn nums(v: &'static Value, path: &str) -> Vec<f64> {
    list(v, path).iter().map(|x| x.as_f64().unwrap_or_else(|| panic!("night_watch classic: '{path}' holds a non-number"))).collect()
}

fn ints(v: &'static Value, path: &str) -> Vec<i32> {
    list(v, path).iter().map(|x| x.as_i64().unwrap_or_else(|| panic!("night_watch classic: '{path}' holds a non-integer")) as i32).collect()
}

fn strs(v: &'static Value, path: &str) -> Vec<&'static str> {
    list(v, path).iter().map(|x| x.as_str().unwrap_or_else(|| panic!("night_watch classic: '{path}' holds a non-string"))).collect()
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn def_from(h: &'static Value, levels: Vec<i32>) -> Def {
    Def {
        id: h.get("id").and_then(Value::as_str).unwrap_or("kathy"),
        doors: strs(h, "doors"),
        start: text(h, "start"),
        retreat_to: text(h, "retreat_to"),
        push_to: text(h, "push_to"),
        fade_to: Vec::new(),
        alternate: flag(h, "alternate"),
        only_when_cams_down: flag(h, "only_when_cams_down"),
        freezes_when_watched: flag(h, "freezes_when_watched"),
        levels,
    }
}

impl Tuning {
    fn load() -> Tuning {
        let (rules, cast, building) = (rules("classic"), cast("classic"), building("classic"));
        let night_ms = num(rules, "night_ms");
        let time_scale = night_ms / num(rules, "tuned_night_ms");
        let hunters = list(cast, "hunters");
        let mut floor: Vec<(i64, &'static Value)> =
            hunters.iter().filter(|h| h.get("floor").is_some()).map(|h| (int(h, "floor"), h)).collect();
        floor.sort_by_key(|(slot, _)| *slot);
        let defs: Vec<Def> = floor.iter().map(|(_, h)| def_from(h, ints(h, "levels"))).collect();
        let combos = floor
            .iter()
            .filter_map(|(_, h)| h.get("combo").map(|c| (*h, c)))
            .map(|(h, c)| Combo {
                a: text(h, "id"),
                b: text(c, "with"),
                min_level: int(c, "min_level") as i32,
                patience_ms: num(c, "patience_ms"),
            })
            .collect();
        let nyxie = hunters.iter().find(|h| text(h, "id") == "nyxie").expect("night_watch classic: cast has no nyxie");
        let path = building.get("paths").and_then(|p| p.get("kathy")).expect("night_watch classic: building has no paths.kathy");
        let mut kathy_def = def_from(path, vec![int(rules, "kathy.level") as i32; 6]);
        kathy_def.fade_to = strs(path, "fade_to");
        let hold = nums(rules, "kathy.hold_ms");
        let kathy = KathyTune {
            level: int(rules, "kathy.level") as i32,
            pace_scale: num(rules, "kathy.pace_scale"),
            grace_scale: num(rules, "kathy.grace_scale"),
            hold_ms: (hold[0], hold[1]),
            hold_tail_ms: num(rules, "kathy.hold_tail_ms"),
            hum_ms: num(rules, "kathy.hum_ms"),
            dip_cycle_ms: num(rules, "kathy.dip_cycle_ms"),
            dips: list(rules, "kathy.dips").iter().map(|d| (num(d, "0"), num(d, "1"))).collect(),
            miss_lock_ms: num(rules, "kathy.miss_lock_ms"),
            meet_rooms: strs(path, "meet_rooms"),
        };
        // The hunters' graph is the building map's walkable links; the office is where they end, not a room they walk to.
        let map = building_map::load("classic").expect("night_watch classic: no data/maps/classic.json");
        let graph = map
            .rooms
            .iter()
            .filter(|r| !map.is_office(&r.id))
            .map(|r| (r.id.as_str(), map.neighbours(&r.id).into_iter().filter(|n| !map.is_office(n)).collect()))
            .collect();
        Tuning {
            nights: int(rules, "nights") as u32,
            tuned_nights: int(rules, "tuned_nights") as u32,
            kathy_night: int(rules, "kathy_night") as u32,
            points_per_level: int(rules, "points_per_level") as u32,
            night_ms,
            hour_ms: night_ms / 6.0,
            power_base: num(rules, "power_base_tuned") / time_scale,
            power_per_device: num(rules, "power_per_device_tuned") / time_scale,
            door_capacity: int(rules, "door_capacity") as usize,
            t_closed_ms: num(rules, "t_closed_ms"),
            focus: num(rules, "focus"),
            door_grace_ms: nums(rules, "door_grace_ms"),
            night_pace: nums(rules, "night_pace"),
            light_push_ms: nums(rules, "light_push_ms"),
            ai_bump: ints(rules, "ai_bump_by_hour"),
            move_window: (
                num(rules, "move_window.base_ms"),
                num(rules, "move_window.per_level_ms"),
                num(rules, "move_window.min_ms"),
                num(rules, "move_window.spread_ms"),
            ),
            blackout: (num(rules, "blackout.base_ms"), num(rules, "blackout.per_level_ms"), num(rules, "blackout.min_ms")),
            nyxie_levels: ints(nyxie, "levels"),
            defs,
            kathy_def,
            kathy,
            combos,
            graph,
        }
    }

    fn neighbours(&self, room: &str) -> &[&'static str] {
        self.graph.get(room).map_or(&[], Vec::as_slice)
    }
}

fn tuning() -> &'static Tuning {
    static TUNING: OnceLock<Tuning> = OnceLock::new();
    TUNING.get_or_init(Tuning::load)
}

/// What the player can do this frame. `cam_room` is the room on the monitor, when the monitor is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
    pub cam: bool,
    pub cam_room: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Moved { id: &'static str, from: &'static str, room: &'static str },
    Arrived { id: &'static str, from: &'static str, room: &'static str },
    Repelled { id: &'static str, from: &'static str, room: &'static str },
    Pushed { id: &'static str, from: &'static str, room: &'static str },
    Flinch { id: &'static str, room: &'static str },
    Attack { id: &'static str, room: &'static str },
    Hum { id: &'static str, door: &'static str, muffled: bool },
    Out,
    Caught { id: &'static str, darkness: bool },
    Call(CallEvent),
    Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Caught(&'static str),
    Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meet {
    None,
    Locked,
    Miss,
    Met(Event),
}

#[derive(Clone, Copy, Debug)]
struct Ctx {
    cam: bool,
    cam_room: Option<&'static str>,
    lit: bool,
    behind: bool,
}

#[derive(Clone, Copy, Debug)]
struct Snap {
    id: &'static str,
    room: &'static str,
    level: i32,
}

#[derive(Debug)]
struct Stalker {
    t: &'static Tuning,
    id: &'static str,
    def: &'static Def,
    room: &'static str,
    door: &'static str,
    door_index: usize,
    base: i32,
    level: i32,
    attack_ms: f64,
    push_ms: f64,
    pace: f64,
    at_door_ms: f64,
    closed_ms: f64,
    lit_ms: f64,
    behind: bool,
    need_ms: f64,
    need_range: Option<(f64, f64)>,
    need_tail: f64,
    unseen: bool,
    dist: HashMap<&'static str, u32>,
    pacer_t: f64,
    pacer_next: Option<f64>,
}

impl Stalker {
    fn new(t: &'static Tuning, def: &'static Def, base: i32, attack_ms: f64, pace: f64, push_ms: f64) -> Stalker {
        let door = def.doors[0];
        Stalker {
            t,
            id: def.id,
            def,
            room: def.start,
            door,
            door_index: 0,
            base,
            level: base,
            attack_ms,
            push_ms,
            pace,
            at_door_ms: 0.0,
            closed_ms: 0.0,
            lit_ms: 0.0,
            behind: false,
            need_ms: t.t_closed_ms,
            need_range: None,
            need_tail: 0.0,
            unseen: false,
            dist: distances_to(t, door),
            pacer_t: 0.0,
            pacer_next: None,
        }
    }

    fn kathy(t: &'static Tuning, rng: &mut NightWatchStream, base: i32, attack_ms: f64, pace: f64) -> Stalker {
        let mut s = Stalker::new(t, &t.kathy_def, base, attack_ms, pace, f64::INFINITY);
        s.unseen = true;
        s.need_range = Some(t.kathy.hold_ms);
        s.need_ms = t.kathy.hold_ms.0;
        s.need_tail = t.kathy.hold_tail_ms;
        s.aim(pick_index(rng, t.kathy_def.doors.len()));
        s
    }

    fn dist_of(&self, room: &str) -> u32 {
        self.dist.get(room).copied().unwrap_or(99)
    }

    fn aim(&mut self, index: usize) {
        self.door_index = index;
        self.door = self.def.doors[index];
        self.dist = distances_to(self.t, self.door);
    }

    fn reset_waiting(&mut self) {
        self.at_door_ms = 0.0;
        self.closed_ms = 0.0;
        self.lit_ms = 0.0;
        self.behind = false;
        self.pacer_t = 0.0;
        self.pacer_next = None;
    }

    fn fade(&mut self, rng: &mut NightWatchStream) {
        self.room = self.def.fade_to[pick_index(rng, self.def.fade_to.len())];
        self.reset_waiting();
        let index = pick_index(rng, self.def.doors.len());
        self.aim(index);
    }

    fn pacer_step(&mut self, dt: f64, rng: &mut NightWatchStream) -> bool {
        if self.level <= 0 {
            self.pacer_t = 0.0;
            self.pacer_next = None;
            return false;
        }
        let wait = match self.pacer_next {
            Some(wait) => wait,
            None => draw_wait(self.t, self.level, rng),
        };
        self.pacer_t += dt;
        if self.pacer_t < wait {
            self.pacer_next = Some(wait);
            return false;
        }
        self.pacer_t = 0.0;
        self.pacer_next = Some(draw_wait(self.t, self.level, rng));
        true
    }

    fn can_move(&self, ctx: Ctx) -> bool {
        if self.def.only_when_cams_down && ctx.cam {
            return false;
        }
        !(self.def.freezes_when_watched && ctx.cam && ctx.cam_room == Some(self.room))
    }

    fn next_room(&self, rng: &mut NightWatchStream, blocked: &dyn Fn(&str) -> bool) -> &'static str {
        let here = self.dist_of(self.room);
        let around: Vec<&'static str> = self.t.neighbours(self.room).iter().copied().filter(|r| !blocked(*r)).collect();
        let closer: Vec<&'static str> = around.iter().copied().filter(|r| self.dist_of(*r) < here).collect();
        let sideways: Vec<&'static str> = around
            .iter()
            .copied()
            .filter(|r| !closer.contains(r) && *r != self.door && !r.starts_with("door"))
            .collect();
        let keep_closer = !closer.is_empty() && (chance(rng, self.t.focus) || sideways.is_empty());
        let pool = if !keep_closer && !sideways.is_empty() { sideways } else { closer };
        if pool.is_empty() {
            return self.room;
        }
        pool[pick_index(rng, pool.len())]
    }

    fn step(
        &mut self,
        dt: f64,
        door_closed: bool,
        ctx: Ctx,
        rng: &mut NightWatchStream,
        blocked: &dyn Fn(&str) -> bool,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        let (id, from) = (self.id, self.room);
        self.behind = self.room == self.door && ctx.behind;
        if self.room != self.door && self.can_move(ctx) && self.pacer_step(dt / self.pace, rng) {
            self.room = self.next_room(rng, blocked);
            if self.room != from {
                let room = self.room;
                if room == self.door {
                    events.push(Event::Arrived { id, from, room });
                    self.at_door_ms = 0.0;
                    self.closed_ms = 0.0;
                    self.lit_ms = 0.0;
                    if let Some((lo, hi)) = self.need_range {
                        self.need_ms = roll_hold(rng.next(), lo, hi, self.need_tail);
                    }
                    return events;
                }
                events.push(Event::Moved { id, from, room });
            }
        }
        if self.room != self.door {
            return events;
        }
        if door_closed {
            self.closed_ms += dt;
            self.lit_ms = 0.0;
            if self.closed_ms >= self.need_ms {
                let from = self.room;
                self.room = self.def.retreat_to;
                self.reset_waiting();
                if !self.def.fade_to.is_empty() {
                    self.fade(rng);
                } else if self.def.alternate {
                    let next = (self.door_index + 1) % self.def.doors.len();
                    self.aim(next);
                }
                events.push(Event::Repelled { id, from, room: self.room });
            }
            return events;
        }
        self.closed_ms = 0.0;
        if self.unseen || !ctx.lit {
            self.at_door_ms += dt;
            self.lit_ms = 0.0;
        } else if self.behind {
            self.lit_ms = 0.0;
        } else {
            let before = self.lit_ms;
            self.lit_ms += dt;
            let half = self.push_ms / 2.0;
            if before < half && self.lit_ms >= half && self.lit_ms < self.push_ms {
                events.push(Event::Flinch { id, room: self.room });
            }
            if self.lit_ms >= self.push_ms {
                let from = self.room;
                self.room = self.def.push_to;
                self.reset_waiting();
                events.push(Event::Pushed { id, from, room: self.room });
                return events;
            }
        }
        if self.at_door_ms >= self.attack_ms {
            events.push(Event::Attack { id, room: self.room });
        }
        events
    }

    fn is_at_door(&self) -> bool {
        self.room == self.door
    }
}

/// One Classic night in progress. Feed `step` the frame's input; read `outcome`, `points` and the call.
#[derive(Debug)]
pub struct ClassicNight {
    t: &'static Tuning,
    pub night: u32,
    pub call: CallScript,
    rng: NightWatchStream,
    hunters: Vec<Stalker>,
    kathy_night: bool,
    grace_ms: f64,
    nyxie_level: i32,
    elapsed: f64,
    power: f64,
    out: bool,
    out_ms: f64,
    over: Option<Outcome>,
    shut: [bool; 2],
    lit: [bool; 2],
    lines: [Vec<&'static str>; 2],
    hum_ms: f64,
    meet_lock_ms: f64,
}

/// Starts a Classic night. `levels` overrides hunters' AI levels by id (Custom Night); `custom` turns off Night 7's Kathy.
pub fn start(calls: &CallsData, night: u32, seed: i64, levels: &[(&str, i32)], custom: bool) -> ClassicNight {
    let t = tuning();
    let night = night.clamp(1, t.nights);
    let mut rng = NightWatchStream::seeded(seed);
    let index = night_index(t, night);
    let level_of = |id: &str, default: i32| levels.iter().find(|(k, _)| *k == id).map_or(default, |(_, v)| *v);
    let kathy_night = night == t.kathy_night && !custom;
    let grace = t.door_grace_ms[index];
    let pace = t.night_pace[index];
    let push = t.light_push_ms[index];

    let mut hunters = Vec::new();
    let mut nyxie_level = 0;
    if kathy_night {
        let grace_k = (grace * t.kathy.grace_scale).round();
        let pace_k = (pace * t.kathy.pace_scale * 1000.0).round() / 1000.0;
        hunters.push(Stalker::kathy(t, &mut rng, level_of("kathy", t.kathy.level), grace_k, pace_k));
    } else {
        for def in t.defs.iter() {
            let base = level_of(def.id, level_for(t, def, night));
            hunters.push(Stalker::new(t, def, base, grace, pace, push));
        }
        nyxie_level = level_of("nyxie", nyxie_level_for(t, night));
    }

    ClassicNight {
        t,
        night,
        call: CallScript::for_level(calls, "classic", night as i32),
        rng,
        hunters,
        kathy_night,
        grace_ms: grace,
        nyxie_level,
        elapsed: 0.0,
        power: 100.0,
        out: false,
        out_ms: 0.0,
        over: None,
        shut: [false; 2],
        lit: [false; 2],
        lines: [Vec::new(), Vec::new()],
        hum_ms: 0.0,
        meet_lock_ms: 0.0,
    }
}

impl ClassicNight {
    pub fn outcome(&self) -> Option<Outcome> {
        self.over
    }

    /// Ten points for a won night, none otherwise.
    pub fn points(&self) -> u32 {
        if self.over == Some(Outcome::Win) {
            self.t.points_per_level
        } else {
            0
        }
    }

    pub fn hour(&self) -> u32 {
        ((self.elapsed / self.t.hour_ms).floor() as u32).min(5)
    }

    pub fn power(&self) -> f64 {
        self.power
    }

    pub fn is_out(&self) -> bool {
        self.out
    }

    pub fn mute_call(&mut self) -> bool {
        self.call.mute()
    }

    pub fn hunter_room(&self, id: &str) -> Option<&'static str> {
        self.hunters.iter().find(|h| h.id == id).map(|h| h.room)
    }

    /// Who waits at a door, front first.
    pub fn waiting_at(&self, door: &str) -> Vec<&'static str> {
        door_slot(door).map_or_else(Vec::new, |d| self.lines[d].clone())
    }

    /// Night 7's door light dips at its tell times while Kathy stands at that door and the light is on.
    pub fn light_dip(&self, door: &str) -> bool {
        let Some(slot) = door_slot(door) else { return false };
        self.kathy_night
            && self.lit[slot]
            && self.hunter_room("kathy") == Some(DOORS[slot])
            && kathy_dip_on(self.t, self.elapsed)
    }

    /// Meet Kathy's eyes on the monitor (Night 7 only). A hit on her hall feed sends her away; anything else locks the key briefly.
    pub fn meet(&mut self, cam: bool, cam_room: Option<&str>, lost: bool) -> Meet {
        if !self.kathy_night || self.over.is_some() {
            return Meet::None;
        }
        if self.meet_lock_ms > 0.0 {
            return Meet::Locked;
        }
        let out = self.out;
        let Some(index) = self.hunters.iter().position(|h| h.id == "kathy") else {
            return Meet::None;
        };
        let k = &mut self.hunters[index];
        if cam && !lost && !out && cam_room == Some(k.room) && self.t.kathy.meet_rooms.contains(&k.room) {
            let from = k.room;
            k.fade(&mut self.rng);
            return Meet::Met(Event::Repelled { id: "kathy", from, room: k.room });
        }
        self.meet_lock_ms = self.t.kathy.miss_lock_ms;
        Meet::Miss
    }

    pub fn step(&mut self, dt: f64, input: Input) -> Vec<Event> {
        let mut events = Vec::new();
        if self.over.is_some() {
            return events;
        }
        self.elapsed += dt;
        events.extend(self.call.update(self.elapsed, dt).into_iter().map(Event::Call));

        let hour = self.hour();
        if self.out {
            self.out_ms += dt;
        } else {
            let drain = self.t.power_base + self.t.power_per_device * usage(input) as f64;
            self.power = (self.power - drain * dt / 1000.0).max(0.0);
            if self.power <= 0.0 {
                self.out = true;
                self.out_ms = 0.0;
                events.push(Event::Out);
            }
        }
        self.shut = [input.door_l && !self.out, input.door_r && !self.out];
        self.lit = [input.light_l && !self.out, input.light_r && !self.out];
        let cam = input.cam && !self.out;

        for i in 0..self.hunters.len() {
            let snap: Vec<Snap> = self
                .hunters
                .iter()
                .map(|h| Snap { id: h.id, room: h.room, level: h.level })
                .collect();
            let id = self.hunters[i].id;
            let level = effective_level(self.t, self.hunters[i].base, hour);
            self.hunters[i].level = level;
            if !self.kathy_night {
                self.hunters[i].attack_ms = self.grace_ms - patience(self.t, id, level, &snap);
            }
            self.refresh_lines();

            let room = self.hunters[i].room;
            let behind = self.lines_at(room).iter().position(|&w| w == id).is_some_and(|p| p > 0);
            let door = self.hunters[i].door;
            let slot = door_slot(door).unwrap_or(0);
            let ctx = Ctx { cam, cam_room: input.cam_room, lit: self.lit[slot], behind };
            let door_capacity = self.t.door_capacity;
            let blocked = |r: &str| {
                r.starts_with("door")
                    && snap.iter().filter(|s| s.id != id && s.room == r).count() >= door_capacity
            };
            let stepped = self.hunters[i].step(dt, self.shut[slot], ctx, &mut self.rng, &blocked);
            for e in stepped {
                events.push(e);
                if let Event::Attack { id, .. } = e {
                    self.over = Some(Outcome::Caught(id));
                    events.push(Event::Caught { id, darkness: false });
                    return events;
                }
            }
        }

        if self.out && self.nyxie_level > 0 && self.out_ms >= blackout_ms(self.t, self.nyxie_level) {
            self.over = Some(Outcome::Caught("nyxie"));
            events.push(Event::Caught { id: "nyxie", darkness: true });
            return events;
        }
        if self.kathy_night {
            self.meet_lock_ms = (self.meet_lock_ms - dt).max(0.0);
            self.tick_hum(dt, &mut events);
        }
        self.refresh_lines();
        if self.elapsed >= self.t.night_ms {
            self.over = Some(Outcome::Win);
            events.push(Event::Win);
        }
        events
    }

    fn tick_hum(&mut self, dt: f64, events: &mut Vec<Event>) {
        let Some(k) = self.hunters.iter().find(|h| h.id == "kathy") else { return };
        if !k.is_at_door() {
            self.hum_ms = 0.0;
            return;
        }
        self.hum_ms += dt;
        if self.hum_ms >= self.t.kathy.hum_ms {
            self.hum_ms = 0.0;
            let slot = door_slot(k.door).unwrap_or(0);
            events.push(Event::Hum { id: "kathy", door: k.door, muffled: self.shut[slot] });
        }
    }

    fn lines_at(&self, room: &str) -> &[&'static str] {
        match door_slot(room) {
            Some(d) => self.lines[d].as_slice(),
            None => &[],
        }
    }

    fn refresh_lines(&mut self) {
        for d in 0..DOORS.len() {
            let hunters = &self.hunters;
            self.lines[d].retain(|w| hunters.iter().any(|h| h.id == *w && h.room == DOORS[d]));
        }
        for h in self.hunters.iter() {
            if let Some(d) = door_slot(h.room) {
                if !self.lines[d].contains(&h.id) {
                    self.lines[d].push(h.id);
                }
            }
        }
    }
}

fn usage(input: Input) -> usize {
    [input.door_l, input.door_r, input.light_l, input.light_r, input.cam].iter().filter(|b| **b).count()
}

fn door_slot(room: &str) -> Option<usize> {
    DOORS.iter().position(|d| *d == room)
}

fn night_index(t: &Tuning, night: u32) -> usize {
    (night.clamp(1, t.tuned_nights) - 1) as usize
}

fn level_for(t: &Tuning, def: &Def, night: u32) -> i32 {
    def.levels[night_index(t, night)]
}

fn nyxie_level_for(t: &Tuning, night: u32) -> i32 {
    if night == t.kathy_night {
        0
    } else {
        t.nyxie_levels[night_index(t, night)]
    }
}

fn ai_bump(t: &Tuning, hour: u32) -> i32 {
    t.ai_bump[(hour as usize).min(t.ai_bump.len() - 1)]
}

fn effective_level(t: &Tuning, base: i32, hour: u32) -> i32 {
    if base > 0 {
        base + ai_bump(t, hour)
    } else {
        0
    }
}

fn blackout_ms(t: &Tuning, level: i32) -> f64 {
    let (base, per_level, min) = t.blackout;
    (base - f64::from(level) * per_level).max(min)
}

fn move_window(t: &Tuning, level: i32) -> (f64, f64) {
    let (base, per_level, min, spread) = t.move_window;
    let lo = (base - f64::from(level) * per_level).max(min);
    (lo, lo + spread)
}

fn draw_wait(t: &Tuning, level: i32, rng: &mut NightWatchStream) -> f64 {
    let (lo, hi) = move_window(t, level);
    lo + rng.next() * (hi - lo)
}

fn roll_hold(u: f64, lo: f64, hi: f64, tail: f64) -> f64 {
    if tail <= 0.0 {
        return (lo + u * (hi - lo)).round();
    }
    (lo - tail * (1.0 - u * (1.0 - (-(hi - lo) / tail).exp())).ln()).round()
}

fn kathy_dip_on(t: &Tuning, elapsed: f64) -> bool {
    let at = elapsed % t.kathy.dip_cycle_ms;
    t.kathy.dips.iter().any(|&(a, b)| at >= a && at < b)
}

/// The hunter who works with her partner loses patience sooner while the partner waits at a door.
fn patience(t: &Tuning, id: &str, level: i32, snap: &[Snap]) -> f64 {
    let mut best = 0.0_f64;
    for c in t.combos.iter() {
        let partner = if c.a == id {
            c.b
        } else if c.b == id {
            c.a
        } else {
            continue;
        };
        let Some(other) = snap.iter().find(|s| s.id == partner) else { continue };
        if other.room.starts_with("door") && other.level >= c.min_level && level >= c.min_level {
            best = best.max(c.patience_ms);
        }
    }
    best
}

fn distances_to(t: &Tuning, target: &'static str) -> HashMap<&'static str, u32> {
    let mut dist = HashMap::from([(target, 0_u32)]);
    let mut queue = VecDeque::from([target]);
    while let Some(room) = queue.pop_front() {
        let d = dist.get(room).copied().unwrap_or(0);
        for &next in t.neighbours(room) {
            if !dist.contains_key(next) {
                dist.insert(next, d + 1);
                queue.push_back(next);
            }
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shut_door_repels_a_hunter_who_has_waited_there() {
        let calls = CallsData::load();
        let levels = [("dovelle", 20), ("ravelle", 0), ("aki", 0), ("ilyra", 0), ("nyxie", 0)];
        let mut night = start(&calls, 1, 1, &levels, false);
        let shut = Input { door_l: true, ..Input::default() };
        let mut events = Vec::new();
        for _ in 0..2400 {
            events.extend(night.step(50.0, shut));
        }
        assert!(events.contains(&Event::Repelled { id: "dovelle", from: "doorL", room: "dining" }));
        assert!(!events.iter().any(|e| matches!(e, Event::Caught { .. })));
    }

    #[test]
    fn the_json_carries_every_key_the_mode_reads() {
        // Loading panics naming the mode and key when one is missing.
        let t = tuning();
        assert_eq!(t.defs.iter().map(|d| d.id).collect::<Vec<_>>(), ["dovelle", "ravelle", "aki", "ilyra"]);
        assert_eq!(t.kathy_def.id, "kathy");
        assert_eq!(t.kathy_def.fade_to, ["stage", "kitchen"]);
        assert_eq!(t.graph.len(), 10);
    }

    #[test]
    fn the_graph_from_the_building_map_equals_the_old_table() {
        // The table the mode used to carry (and data/buildings/classic.json still lists): same rooms, same neighbours.
        let old: [(&str, &[&str]); 10] = [
            ("stage", &["dining", "backstage"]),
            ("backstage", &["stage", "westhall"]),
            ("dining", &["stage", "westhall", "kitchen", "easthall"]),
            ("kitchen", &["dining"]),
            ("westhall", &["dining", "backstage", "closet", "doorL"]),
            ("closet", &["westhall"]),
            ("easthall", &["dining", "corner", "doorR"]),
            ("corner", &["easthall"]),
            ("doorL", &["westhall"]),
            ("doorR", &["easthall"]),
        ];
        let t = tuning();
        assert_eq!(t.graph.len(), old.len());
        for (room, next) in old {
            let (mut got, mut want) = (t.neighbours(room).to_vec(), next.to_vec());
            got.sort_unstable();
            want.sort_unstable();
            assert_eq!(got, want, "neighbours of {room}");
        }
    }

    #[test]
    fn the_loaded_values_equal_the_old_constants() {
        let t = tuning();
        assert_eq!((t.nights, t.points_per_level, t.tuned_nights, t.kathy_night), (7, 10, 6, 7));
        assert_eq!(t.night_ms, 270_000.0);
        assert_eq!(t.hour_ms, 270_000.0 / 6.0);
        assert_eq!(t.power_base, 0.1 / (270_000.0 / 480_000.0));
        assert_eq!(t.power_per_device, 0.07 / (270_000.0 / 480_000.0));
        assert_eq!((t.door_capacity, t.t_closed_ms, t.focus), (2, 450.0, 0.75));
        assert_eq!(t.door_grace_ms, [9000.0, 8200.0, 7500.0, 6900.0, 6400.0, 5800.0]);
        assert_eq!(t.night_pace, [1.3, 1.24, 1.18, 1.12, 1.06, 1.03]);
        assert_eq!(t.light_push_ms, [2000.0, 2500.0, 3000.0, 3500.0, 4000.0, 4500.0]);
        assert_eq!(t.nyxie_levels, [3, 5, 7, 9, 11, 13]);
        assert_eq!(t.defs[0].levels, [2, 4, 6, 9, 12, 16]);
        assert_eq!(t.defs[2].levels, [0, 0, 4, 6, 9, 13]);
        assert!(t.defs[1].alternate && t.defs[2].only_when_cams_down && t.defs[3].freezes_when_watched);
        assert_eq!((t.combos[0].a, t.combos[0].b, t.combos[0].min_level, t.combos[0].patience_ms), ("dovelle", "ravelle", 8, 1400.0));
        assert_eq!((t.kathy.level, t.kathy.hold_ms, t.kathy.hold_tail_ms, t.kathy.hum_ms), (20, (2500.0, 20000.0), 4500.0, 1100.0));
        assert_eq!(t.kathy.dips, [(0.0, 180.0), (780.0, 900.0), (1500.0, 1760.0)]);
        assert_eq!((t.kathy.pace_scale, t.kathy.grace_scale, t.kathy.miss_lock_ms, t.kathy.dip_cycle_ms), (0.65, 0.45, 2000.0, 2600.0));
        assert_eq!(t.kathy.meet_rooms, ["westhall", "easthall"]);
        assert_eq!(blackout_ms(t, 3), 10_300.0);
        assert_eq!(blackout_ms(t, 40), 4000.0);
        assert_eq!(move_window(t, 0), (13_000.0, 16_000.0));
        assert_eq!(move_window(t, 40), (2500.0, 5500.0));
        assert_eq!([0, 3, 4, 5].map(|h| ai_bump(t, h)), [0, 1, 2, 2]);
        let mut west = t.neighbours("westhall").to_vec();
        west.sort_unstable();
        assert_eq!(west, ["backstage", "closet", "dining", "doorL"]);
    }
}
