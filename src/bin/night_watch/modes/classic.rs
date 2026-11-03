//! Classic night rules (Godot `night_watch_classic.gd`, with the hunter from `night_watch_stalker.gd` and the
//! building and cast from `data/night_watch/buildings/classic.json` and `casts/classic.json`). No window or input:
//! the shell feeds `Input` each frame and draws the returned `Event`s. The same seed gives the same night.

use std::collections::{HashMap, VecDeque};

use crate::data::CallsData;
use crate::framework::{chance, pick_index, CallEvent, CallScript, NightWatchStream};

pub const NIGHTS: u32 = 7;
pub const POINTS_PER_LEVEL: u32 = 10;
pub const MAX_SCORE: u32 = POINTS_PER_LEVEL * NIGHTS;

const NIGHT_MS: f64 = 270_000.0;
const HOUR_MS: f64 = NIGHT_MS / 6.0;
const TUNED_NIGHT_MS: f64 = 480_000.0;
const TIME_SCALE: f64 = NIGHT_MS / TUNED_NIGHT_MS;
const POWER_BASE: f64 = 0.1 / TIME_SCALE;
const POWER_PER_DEVICE: f64 = 0.07 / TIME_SCALE;

const TUNED_NIGHTS: u32 = 6;
const KATHY_NIGHT: u32 = 7;
const DOOR_CAPACITY: usize = 2;
const T_CLOSED_MS: f64 = 450.0;
const FOCUS: f64 = 0.75;
const DOOR_GRACE_MS: [f64; 6] = [9000.0, 8200.0, 7500.0, 6900.0, 6400.0, 5800.0];
const NIGHT_PACE: [f64; 6] = [1.3, 1.24, 1.18, 1.12, 1.06, 1.03];
const LIGHT_PUSH_MS: [f64; 6] = [2000.0, 2500.0, 3000.0, 3500.0, 4000.0, 4500.0];
const NYXIE_LEVELS: [i32; 6] = [3, 5, 7, 9, 11, 13];

const KATHY_LEVEL: i32 = 20;
const KATHY_PACE_SCALE: f64 = 0.65;
const KATHY_GRACE_SCALE: f64 = 0.45;
const KATHY_HOLD_MS: (f64, f64) = (2500.0, 20000.0);
const KATHY_HOLD_TAIL_MS: f64 = 4500.0;
const KATHY_HUM_MS: f64 = 1100.0;
const KATHY_DIP_CYCLE_MS: f64 = 2600.0;
const KATHY_DIPS: [(f64, f64); 3] = [(0.0, 180.0), (780.0, 900.0), (1500.0, 1760.0)];
const KATHY_MISS_LOCK_MS: f64 = 2000.0;
const KATHY_MEET_ROOMS: [&str; 2] = ["westhall", "easthall"];

const DOORS: [&str; 2] = ["doorL", "doorR"];

static GRAPH: [(&str, &[&str]); 10] = [
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

#[derive(Debug)]
struct Def {
    id: &'static str,
    doors: &'static [&'static str],
    start: &'static str,
    retreat_to: &'static str,
    push_to: &'static str,
    fade_to: &'static [&'static str],
    alternate: bool,
    only_when_cams_down: bool,
    freezes_when_watched: bool,
    levels: [i32; 6],
}

/// The floor's hunters in the order the floor wakes them (cast floor slots 0 to 3).
static DEFS: [Def; 4] = [
    Def {
        id: "dovelle",
        doors: &["doorL"],
        start: "stage",
        retreat_to: "dining",
        push_to: "stage",
        fade_to: &[],
        alternate: false,
        only_when_cams_down: false,
        freezes_when_watched: false,
        levels: [2, 4, 6, 9, 12, 16],
    },
    Def {
        id: "ravelle",
        doors: &["doorR", "doorL"],
        start: "kitchen",
        retreat_to: "dining",
        push_to: "kitchen",
        fade_to: &[],
        alternate: true,
        only_when_cams_down: false,
        freezes_when_watched: false,
        levels: [0, 3, 5, 7, 10, 14],
    },
    Def {
        id: "aki",
        doors: &["doorR"],
        start: "closet",
        retreat_to: "corner",
        push_to: "closet",
        fade_to: &[],
        alternate: false,
        only_when_cams_down: true,
        freezes_when_watched: false,
        levels: [0, 0, 4, 6, 9, 13],
    },
    Def {
        id: "ilyra",
        doors: &["doorL"],
        start: "backstage",
        retreat_to: "closet",
        push_to: "stage",
        fade_to: &[],
        alternate: false,
        only_when_cams_down: false,
        freezes_when_watched: true,
        levels: [0, 0, 0, 5, 8, 12],
    },
];

static KATHY: Def = Def {
    id: "kathy",
    doors: &["doorL", "doorR"],
    start: "stage",
    retreat_to: "stage",
    push_to: "stage",
    fade_to: &["stage", "kitchen"],
    alternate: false,
    only_when_cams_down: false,
    freezes_when_watched: false,
    levels: [KATHY_LEVEL; 6],
};

struct Combo {
    a: &'static str,
    b: &'static str,
    min_level: i32,
    patience_ms: f64,
}

static COMBOS: [Combo; 1] = [Combo { a: "dovelle", b: "ravelle", min_level: 8, patience_ms: 1400.0 }];

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
    fn new(def: &'static Def, base: i32, attack_ms: f64, pace: f64, push_ms: f64) -> Stalker {
        let door = def.doors[0];
        Stalker {
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
            need_ms: T_CLOSED_MS,
            need_range: None,
            need_tail: 0.0,
            unseen: false,
            dist: distances_to(door),
            pacer_t: 0.0,
            pacer_next: None,
        }
    }

    fn kathy(rng: &mut NightWatchStream, base: i32, attack_ms: f64, pace: f64) -> Stalker {
        let mut s = Stalker::new(&KATHY, base, attack_ms, pace, f64::INFINITY);
        s.unseen = true;
        s.need_range = Some(KATHY_HOLD_MS);
        s.need_ms = KATHY_HOLD_MS.0;
        s.need_tail = KATHY_HOLD_TAIL_MS;
        s.aim(pick_index(rng, KATHY.doors.len()));
        s
    }

    fn dist_of(&self, room: &str) -> u32 {
        self.dist.get(room).copied().unwrap_or(99)
    }

    fn aim(&mut self, index: usize) {
        self.door_index = index;
        self.door = self.def.doors[index];
        self.dist = distances_to(self.door);
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
            None => draw_wait(self.level, rng),
        };
        self.pacer_t += dt;
        if self.pacer_t < wait {
            self.pacer_next = Some(wait);
            return false;
        }
        self.pacer_t = 0.0;
        self.pacer_next = Some(draw_wait(self.level, rng));
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
        let around: Vec<&'static str> = neighbours(self.room).iter().copied().filter(|r| !blocked(*r)).collect();
        let closer: Vec<&'static str> = around.iter().copied().filter(|r| self.dist_of(*r) < here).collect();
        let sideways: Vec<&'static str> = around
            .iter()
            .copied()
            .filter(|r| !closer.contains(r) && *r != self.door && !r.starts_with("door"))
            .collect();
        let keep_closer = !closer.is_empty() && (chance(rng, FOCUS) || sideways.is_empty());
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
    let night = night.clamp(1, NIGHTS);
    let mut rng = NightWatchStream::seeded(seed);
    let index = night_index(night);
    let level_of = |id: &str, default: i32| levels.iter().find(|(k, _)| *k == id).map_or(default, |(_, v)| *v);
    let kathy_night = night == KATHY_NIGHT && !custom;
    let grace = DOOR_GRACE_MS[index];
    let pace = NIGHT_PACE[index];
    let push = LIGHT_PUSH_MS[index];

    let mut hunters = Vec::new();
    let mut nyxie_level = 0;
    if kathy_night {
        let grace_k = (grace * KATHY_GRACE_SCALE).round();
        let pace_k = (pace * KATHY_PACE_SCALE * 1000.0).round() / 1000.0;
        hunters.push(Stalker::kathy(&mut rng, level_of("kathy", KATHY_LEVEL), grace_k, pace_k));
    } else {
        for def in DEFS.iter() {
            let base = level_of(def.id, level_for(def, night));
            hunters.push(Stalker::new(def, base, grace, pace, push));
        }
        nyxie_level = level_of("nyxie", nyxie_level_for(night));
    }

    ClassicNight {
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
            POINTS_PER_LEVEL
        } else {
            0
        }
    }

    pub fn hour(&self) -> u32 {
        ((self.elapsed / HOUR_MS).floor() as u32).min(5)
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
            && kathy_dip_on(self.elapsed)
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
        if cam && !lost && !out && cam_room == Some(k.room) && KATHY_MEET_ROOMS.contains(&k.room) {
            let from = k.room;
            k.fade(&mut self.rng);
            return Meet::Met(Event::Repelled { id: "kathy", from, room: k.room });
        }
        self.meet_lock_ms = KATHY_MISS_LOCK_MS;
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
            let drain = POWER_BASE + POWER_PER_DEVICE * usage(input) as f64;
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
            let level = effective_level(self.hunters[i].base, hour);
            self.hunters[i].level = level;
            if !self.kathy_night {
                self.hunters[i].attack_ms = self.grace_ms - patience(id, level, &snap);
            }
            self.refresh_lines();

            let room = self.hunters[i].room;
            let behind = self.lines_at(room).iter().position(|&w| w == id).is_some_and(|p| p > 0);
            let door = self.hunters[i].door;
            let slot = door_slot(door).unwrap_or(0);
            let ctx = Ctx { cam, cam_room: input.cam_room, lit: self.lit[slot], behind };
            let blocked = |r: &str| {
                r.starts_with("door")
                    && snap.iter().filter(|s| s.id != id && s.room == r).count() >= DOOR_CAPACITY
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

        if self.out && self.nyxie_level > 0 && self.out_ms >= blackout_ms(self.nyxie_level) {
            self.over = Some(Outcome::Caught("nyxie"));
            events.push(Event::Caught { id: "nyxie", darkness: true });
            return events;
        }
        if self.kathy_night {
            self.meet_lock_ms = (self.meet_lock_ms - dt).max(0.0);
            self.tick_hum(dt, &mut events);
        }
        self.refresh_lines();
        if self.elapsed >= NIGHT_MS {
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
        if self.hum_ms >= KATHY_HUM_MS {
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

fn night_index(night: u32) -> usize {
    (night.clamp(1, TUNED_NIGHTS) - 1) as usize
}

fn level_for(def: &Def, night: u32) -> i32 {
    def.levels[night_index(night)]
}

fn nyxie_level_for(night: u32) -> i32 {
    if night == KATHY_NIGHT {
        0
    } else {
        NYXIE_LEVELS[night_index(night)]
    }
}

fn ai_bump(hour: u32) -> i32 {
    match hour {
        h if h >= 4 => 2,
        3 => 1,
        _ => 0,
    }
}

fn effective_level(base: i32, hour: u32) -> i32 {
    if base > 0 {
        base + ai_bump(hour)
    } else {
        0
    }
}

fn blackout_ms(level: i32) -> f64 {
    (11500.0 - f64::from(level) * 400.0).max(4000.0)
}

fn move_window(level: i32) -> (f64, f64) {
    let lo = (13000.0 - f64::from(level) * 500.0).max(2500.0);
    (lo, lo + 3000.0)
}

fn draw_wait(level: i32, rng: &mut NightWatchStream) -> f64 {
    let (lo, hi) = move_window(level);
    lo + rng.next() * (hi - lo)
}

fn roll_hold(u: f64, lo: f64, hi: f64, tail: f64) -> f64 {
    if tail <= 0.0 {
        return (lo + u * (hi - lo)).round();
    }
    (lo - tail * (1.0 - u * (1.0 - (-(hi - lo) / tail).exp())).ln()).round()
}

fn kathy_dip_on(elapsed: f64) -> bool {
    let t = elapsed % KATHY_DIP_CYCLE_MS;
    KATHY_DIPS.iter().any(|&(a, b)| t >= a && t < b)
}

/// The hunter who works with her partner loses patience sooner while the partner waits at a door.
fn patience(id: &str, level: i32, snap: &[Snap]) -> f64 {
    let mut best = 0.0_f64;
    for c in COMBOS.iter() {
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

fn neighbours(room: &str) -> &'static [&'static str] {
    match GRAPH.iter().find(|(r, _)| *r == room) {
        Some((_, n)) => n,
        None => &[],
    }
}

fn distances_to(target: &'static str) -> HashMap<&'static str, u32> {
    let mut dist = HashMap::from([(target, 0_u32)]);
    let mut queue = VecDeque::from([target]);
    while let Some(room) = queue.pop_front() {
        let d = dist.get(room).copied().unwrap_or(0);
        for &next in neighbours(room) {
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
}
