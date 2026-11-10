#![allow(dead_code)]
//! Masked Hours' rules (port of `mask_logic.gd`, a port of createMaskGame in the 2D game, number for number). No scene,
//! no input, no drawing: the shift feeds `MaskSim::step` the state of the mask, the monitor, the lights and the wind and
//! shows what happens. Draw ORDER of the random streams is kept as in GDScript, so a seed is deterministic.
//!
//! Streams: `rng` is the shared mulberry32 (rests, waits, who comes, which side); `vr` is the variety stream, seeded from
//! the first state of `rng` XOR a salt (vent kill waits, the favoured vent, the pressure phases).
//!
//! `step(dt_ms, input)` returns the events in order (`Ev`); `over` becomes `Win` or `Caught` (by whom: `by`).

use super::data::{
    active_hunters, box_decay_for_level, dark_grace_ms, duct_of, flash_seconds, other_side, rules, rush_door_ms, rush_flashes, rush_run_ms,
    side_index, DUCTS, HALL_HUNTERS, RUSHER, VENT_HUNTERS,
};
use crate::core::{effective_level, hour_index, spread_ms, Fnv, Mulberry32, Pacer, NIGHT_MS};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    None,
    Win,
    Caught,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HallState {
    Away,
    Room,
    Hall,
    Office,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RushState {
    Away,
    Poised,
    Run,
    Door,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VentState {
    Away,
    Duct,
    Vent,
    Fetch,
}

/// What the player's controls give the rules each frame (MaskControls::control_input).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimInput {
    pub mask: bool,
    pub monitor: bool,
    pub cam: String,
    pub light_l: bool,
    pub light_r: bool,
    pub light_h: bool,
    pub winding: bool,
    pub lamp: bool,
}

/// The things that happen in a step, in order. Strings are hunter ids and room ids.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    Room { id: String, room: String, poised: bool },
    Hall { id: String },
    Step { id: String, step: i32 },
    Office { id: String },
    Leave { id: String },
    Rush { id: String, flashes: i32 },
    FlashHit { id: String, n: i32, of: i32 },
    RushRetreat { id: String },
    Door { id: String },
    Duct { id: String, side: char, room: String, leg: Option<i32> },
    Vent { id: String, side: char },
    Fetch { id: String, side: char, to: char },
    Creep { id: String, side: char, step: i32 },
    VentRetreat { id: String, side: char },
    Dark { id: String },
    BoxStart,
    BoxEmpty,
    BoxSaved,
    Caught { by: String },
    Win,
}

impl Ev {
    /// A short stable name for event logs and digests.
    pub fn name(&self) -> String {
        match self {
            Ev::Room { id, room, poised } => format!("room:{id}:{room}{}", if *poised { ":poised" } else { "" }),
            Ev::Hall { id } => format!("hall:{id}"),
            Ev::Step { id, step } => format!("step:{id}:{step}"),
            Ev::Office { id } => format!("office:{id}"),
            Ev::Leave { id } => format!("leave:{id}"),
            Ev::Rush { id, flashes } => format!("rush:{id}:{flashes}"),
            Ev::FlashHit { id, n, of } => format!("flashHit:{id}:{n}/{of}"),
            Ev::RushRetreat { id } => format!("rushRetreat:{id}"),
            Ev::Door { id } => format!("door:{id}"),
            Ev::Duct { id, side, room, leg } => format!("duct:{id}:{side}:{room}:{}", leg.unwrap_or(-1)),
            Ev::Vent { id, side } => format!("vent:{id}:{side}"),
            Ev::Fetch { id, side, to } => format!("fetch:{id}:{side}>{to}"),
            Ev::Creep { id, side, step } => format!("creep:{id}:{side}:{step}"),
            Ev::VentRetreat { id, side } => format!("ventRetreat:{id}:{side}"),
            Ev::Dark { id } => format!("dark:{id}"),
            Ev::BoxStart => "boxStart".to_string(),
            Ev::BoxEmpty => "boxEmpty".to_string(),
            Ev::BoxSaved => "boxSaved".to_string(),
            Ev::Caught { by } => format!("caught:{by}"),
            Ev::Win => "win".to_string(),
        }
    }
}

/// Thessine: walks the hall one visitor at a time.
#[derive(Clone, Debug)]
pub struct HallHunter {
    pub state: HallState,
    pub room: String,
    pub t: f64,
    pub step: i32,
    pub need: f64,
    pub pacer: Pacer,
    pub cool: f64,
    pub fooled: f64,
    pub off: f64,
    pub window: f64,
}

/// Umbriel: poised in the game area, then a run down the hall that the hall light turns back.
#[derive(Clone, Debug)]
pub struct Rusher {
    pub state: RushState,
    pub room: String,
    pub t: f64,
    pub need: f64,
    pub flashes: i32,
    pub pacer: Pacer,
    pub cool: f64,
    pub runs: i32,
    pub flash_total: i32,
    pub goal: i32,
    pub door_need: f64,
}

/// Verrine, Haelith and Ilyra: one record each; the vent clock is shared.
#[derive(Clone, Debug)]
pub struct VentHunter {
    pub state: VentState,
    pub room: String,
    pub side: char,
    pub t: f64,
    pub need: f64,
    pub lit: f64,
    pub step: i32,
    pub wait: f64,
}

/// The one clock that decides when the next vent visitor comes.
#[derive(Clone, Debug)]
pub struct VentClock {
    pub cool: f64,
    pub pacer: Pacer,
}

/// A pressure phase: a speed factor for a while.
#[derive(Clone, Copy, Debug)]
pub struct Phase {
    pub v: f64,
    pub left: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lit {
    pub l: bool,
    pub r: bool,
    pub h: bool,
}

impl Lit {
    pub fn side(&self, side: char) -> bool {
        if side == 'L' {
            self.l
        } else {
            self.r
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    pub light: f64,
    pub flash: f64,
    pub lamp: f64,
    pub cam: f64,
}

/// Who is out in the ducts or vents right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Occupant {
    pub id: &'static str,
    pub side: char,
    pub state: VentState,
    pub room: String,
}

pub struct MaskSim {
    pub night_idx: usize,
    pub rng: Mulberry32,
    pub vr: Mulberry32,
    /// Custom-night sliders (hunter id -> level); empty on a normal night.
    pub levels: BTreeMap<String, i32>,
    pub infinite_battery: bool,
    pub custom: bool,
    /// Admin: a multiplier on how fast the hunters' rests and waits run (1.0 = the 2D game's numbers).
    pub hunter_speed: f64,
    box_starts: f64,
    pub decay: f64,
    flash_per_sec: f64,
    // the state (the original's `s`)
    pub elapsed: f64,
    pub battery: f64,
    pub box_pct: f64,
    pub box_zero: f64,
    pub dark: f64,
    pub hall: HallHunter,
    pub vent_clock: VentClock,
    pub verrine: VentHunter,
    pub haelith: VentHunter,
    pub ilyra: VentHunter,
    pub rush: Rusher,
    pub over: Over,
    pub by: String,
    pub flash_count: i32,
    pub flash_spent: f64,
    pub prev_h: bool,
    pub prev_v: [bool; 2],
    pub vent_flash_count: i32,
    pub lamp: bool,
    pub frozen: Vec<String>,
    pub spent: Spent,
    pub side_bias: f64,
    /// hall, rush, vent
    pub mood: [Phase; 3],
    pub lit: Lit,
    pub box_started: bool,
}

impl MaskSim {
    /// `night` is 1..=6, `levels` the custom sliders (empty for a normal night).
    pub fn new(night: i32, seed: u32, levels: BTreeMap<String, i32>, custom: bool) -> MaskSim {
        let r = rules();
        let night_idx = (night.clamp(1, 6) - 1) as usize;
        let rng = Mulberry32::new(seed);
        let vr = Mulberry32::new(rng.state() ^ r.variety.salt);
        let decay = match levels.get("glyra") {
            Some(l) if *l >= 0 => box_decay_for_level(*l),
            _ => r.box_decay.get(night_idx).copied().unwrap_or(2.0),
        };
        let box_starts = if !custom && night_idx == 0 { 2.0 * crate::core::HOUR_MS } else { 0.0 };
        let flash_per_sec = 100.0 / flash_seconds(night_idx);
        let calm = Phase { v: 1.0, left: 1e12 };
        let home = |id: &str| r.home(id).to_string();
        let mut s = MaskSim {
            night_idx,
            rng,
            vr,
            levels,
            infinite_battery: false,
            custom,
            hunter_speed: 1.0,
            box_starts,
            decay,
            flash_per_sec,
            elapsed: 0.0,
            battery: 100.0,
            box_pct: 100.0,
            box_zero: 0.0,
            dark: 0.0,
            hall: HallHunter {
                state: HallState::Away,
                room: home("thessine"),
                t: 0.0,
                step: 0,
                need: 0.0,
                pacer: Pacer::new(),
                cool: r.t("hallCool0"),
                fooled: 0.0,
                off: 0.0,
                window: r.t("windowMs"),
            },
            vent_clock: VentClock { cool: 0.0, pacer: Pacer::new() },
            verrine: VentHunter { state: VentState::Away, room: home("verrine"), side: 'L', t: 0.0, need: 0.0, lit: 0.0, step: 0, wait: r.t("ventWaitMs") },
            haelith: VentHunter { state: VentState::Away, room: home("haelith"), side: 'L', t: 0.0, need: 0.0, lit: 0.0, step: 0, wait: r.t("haelithWaitMs") },
            ilyra: VentHunter { state: VentState::Away, room: home("ilyra"), side: 'L', t: 0.0, need: 0.0, lit: 0.0, step: 0, wait: r.t("ilyraVentMs") },
            rush: Rusher {
                state: RushState::Away,
                room: home(RUSHER),
                t: 0.0,
                need: 0.0,
                flashes: 0,
                pacer: Pacer::new(),
                cool: 0.0,
                runs: 0,
                flash_total: 0,
                goal: 0,
                door_need: 0.0,
            },
            over: Over::None,
            by: String::new(),
            flash_count: 0,
            flash_spent: 0.0,
            prev_h: false,
            prev_v: [false, false],
            vent_flash_count: 0,
            lamp: false,
            frozen: Vec::new(),
            spent: Spent::default(),
            side_bias: 0.5,
            mood: [calm, calm, calm],
            lit: Lit::default(),
            box_started: false,
        };
        // draw order of _build(): the vent clock's rest, then Umbriel's, then the variety stream
        s.vent_clock.cool = s.rest0(r.t("ventCool0"));
        s.rush.cool = s.rest0(r.t("rushCool0"));
        let sb = r.variety.side_bias;
        s.side_bias = sb.0 + s.vr.next() * (sb.1 - sb.0);
        let h = s.draw_phase();
        let u = s.draw_phase();
        let v = s.draw_phase();
        s.mood = [h, u, v];
        s
    }

    // --- draws ---------------------------------------------------------------------------------------------------

    fn rest0(&mut self, ms: f64) -> f64 {
        spread_ms(ms, &mut self.rng, rules().variety.rest_spread)
    }

    fn kill_wait(&mut self, ms: f64) -> f64 {
        let v = &rules().variety;
        let k = v.kill_spread;
        let d = self.vr.next();
        (ms * v.kill_floor).max(ms * (1.0 - k + 2.0 * k * d))
    }

    fn draw_phase(&mut self) -> Phase {
        let v = &rules().variety;
        let n = v.factors.len();
        let pick = ((self.vr.next() * n as f64).floor() as usize).min(n - 1);
        let factor = v.factors[pick];
        let left = v.ms.0 + self.vr.next() * (v.ms.1 - v.ms.0);
        Phase { v: factor, left }
    }

    fn jittered(&mut self, base: &str, jitter: &str) -> f64 {
        let r = rules();
        let j = r.t(jitter);
        r.t(base) * (1.0 - j / 2.0 + self.rng.next() * j)
    }

    fn step_need(&mut self) -> f64 {
        self.jittered("stepMs", "stepJitter")
    }
    fn room_need(&mut self) -> f64 {
        self.jittered("roomMs", "roomJitter")
    }
    fn duct_need(&mut self) -> f64 {
        self.jittered("ductMs", "ductJitter")
    }
    fn ilyra_need(&mut self) -> f64 {
        self.jittered("ilyraStepMs", "ilyraStepJitter")
    }
    fn haelith_duct(&mut self) -> f64 {
        self.jittered("haelithDuctMs", "haelithDuctJitter")
    }

    // --- queries -------------------------------------------------------------------------------------------------

    pub fn base_level(&self, id: &str) -> i32 {
        if let Some(v) = self.levels.get(id) {
            if *v >= 0 {
                return *v;
            }
        }
        rules().level_of(id, self.night_idx)
    }

    /// The level now: the base plus the night's step-up from 3 AM.
    pub fn level(&self, id: &str) -> i32 {
        effective_level(self.base_level(id), hour_index(self.elapsed))
    }

    pub fn vent_state(&self, id: &str) -> &VentHunter {
        match id {
            "haelith" => &self.haelith,
            "ilyra" => &self.ilyra,
            _ => &self.verrine,
        }
    }

    pub fn vent_state_mut(&mut self, id: &str) -> &mut VentHunter {
        match id {
            "haelith" => &mut self.haelith,
            "ilyra" => &mut self.ilyra,
            _ => &mut self.verrine,
        }
    }

    /// Who is out in the ducts or vents right now (at most one).
    pub fn vent_occupant(&self) -> Option<Occupant> {
        for id in VENT_HUNTERS {
            let h = self.vent_state(id);
            if h.state != VentState::Away {
                return Some(Occupant { id, side: h.side, state: h.state, room: h.room.clone() });
            }
        }
        None
    }

    fn vent_out(&self) -> bool {
        self.vent_occupant().is_some()
    }

    /// What the player can SEE of a vent hunter in the office's vent opening: only while the light is on that side.
    pub fn vent_seen(&self, side: char) -> Option<&'static str> {
        match self.vent_occupant() {
            Some(o) if o.state == VentState::Vent && o.side == side && self.lit.side(side) => Some(o.id),
            _ => None,
        }
    }

    /// The only trace of a hunter in a dark duct: eyeshine at a fixed spot (a camera).
    pub fn glint(&self, room: &str) -> bool {
        let side = match room {
            "ductL" => 'L',
            "ductR" => 'R',
            _ => return false,
        };
        match self.vent_occupant() {
            Some(o) => o.state == VentState::Duct && o.room == room && !self.lit.side(side),
            None => false,
        }
    }

    /// Eyeshine in a dark office vent.
    pub fn vent_glint(&self, side: char) -> bool {
        match self.vent_occupant() {
            Some(o) => o.state == VentState::Vent && o.side == side && !self.lit.side(side),
            None => false,
        }
    }

    pub fn in_office(&self) -> Option<&'static str> {
        for id in HALL_HUNTERS {
            if self.hall.state == HallState::Office {
                return Some(id);
            }
        }
        None
    }

    fn walker_in_hall(&self) -> bool {
        matches!(self.hall.state, HallState::Hall | HallState::Office)
    }

    fn rusher_in_hall(&self) -> bool {
        matches!(self.rush.state, RushState::Run | RushState::Door)
    }

    pub fn is_over(&self) -> bool {
        self.over != Over::None
    }

    // --- the clock -----------------------------------------------------------------------------------------------

    fn finish(&mut self, mut ev: Vec<Ev>, by: &str) -> Vec<Ev> {
        self.over = Over::Caught;
        self.by = by.to_string();
        ev.push(Ev::Caught { by: by.to_string() });
        ev
    }

    /// A vent hunter goes home; the shared clock starts a fresh rest.
    fn rest(&mut self, id: &'static str, ms: f64, ev: &mut Vec<Ev>) {
        let home = rules().home(id).to_string();
        let side;
        {
            let h = self.vent_state_mut(id);
            h.state = VentState::Away;
            h.room = home;
            side = h.side;
        }
        self.vent_clock.cool = self.rest0(ms);
        self.vent_clock.pacer.reset();
        ev.push(Ev::VentRetreat { id: id.to_string(), side });
    }

    /// Advance by `dt_ms` with the player's current inputs. `Caught` ends the game.
    pub fn step(&mut self, dt_ms: f64, input: &SimInput) -> Vec<Ev> {
        let mut ev: Vec<Ev> = Vec::new();
        if self.over != Over::None {
            return ev;
        }
        let r = rules();
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        for k in 0..3 {
            self.mood[k].left -= dt_ms;
            if self.mood[k].left <= 0.0 {
                let nxt = self.draw_phase();
                self.mood[k] = nxt;
            }
        }
        let mood = self.mood;

        // Battery: the lights, the desk lamp and the monitor all draw on it; the monitor still works at 0.
        let mut lit = Lit::default();
        if self.battery > 0.0 {
            lit = Lit { l: input.light_l, r: input.light_r && !input.light_l, h: input.light_h };
        }
        let using = lit.l || lit.r || lit.h;
        self.lamp = self.battery > 0.0 && input.lamp;
        let monitor = input.monitor;
        if !self.infinite_battery {
            let want = [
                if using { self.flash_per_sec * dt } else { 0.0 },
                if self.lamp { self.flash_per_sec * r.t("lampDrain") * dt } else { 0.0 },
                if monitor && self.battery > 0.0 { self.flash_per_sec * r.t("camDrain") * dt } else { 0.0 },
            ];
            for (i, w) in want.iter().enumerate() {
                let c = self.battery.min(*w);
                self.battery -= c;
                match i {
                    0 => self.spent.light += c,
                    1 => self.spent.lamp += c,
                    _ => self.spent.cam += c,
                }
            }
        }
        // A flash: the hall light coming on. Each one costs a little extra charge.
        let flash = lit.h && !self.prev_h;
        self.prev_h = lit.h;
        if flash {
            self.flash_count += 1;
            if !self.infinite_battery {
                let c = self.battery.min(r.t("flashCost"));
                self.battery -= c;
                self.flash_spent += c;
                self.spent.flash += c;
            }
        }
        // A vent light coming on is a flash as well; swapping from one vent to the other is a new light coming on.
        for side in ['L', 'R'] {
            let on = lit.side(side);
            let i = side_index(side);
            if on && !self.prev_v[i] {
                self.vent_flash_count += 1;
                if !self.infinite_battery {
                    let c = self.battery.min(r.t("flashCost"));
                    self.battery -= c;
                    self.flash_spent += c;
                    self.spent.flash += c;
                }
            }
            self.prev_v[i] = on;
        }
        self.lit = lit;
        let mask = input.mask;
        let watched: String = if monitor { input.cam.clone() } else { String::new() };
        self.frozen.clear();

        // The dark: with the battery gone she is coming, and nothing you do brings the light back.
        let nyx = self.base_level("nyxie");
        if self.battery <= 0.0 && nyx > 0 {
            if self.dark == 0.0 {
                ev.push(Ev::Dark { id: "nyxie".to_string() });
            }
            self.dark += dt_ms;
            if self.dark >= dark_grace_ms(nyx) {
                return self.finish(ev, "nyxie");
            }
        }

        // Music box. On night 1 it is wound and silent until 2 AM.
        if self.box_starts > 0.0 && !self.box_started && self.elapsed >= self.box_starts {
            self.box_started = true;
            ev.push(Ev::BoxStart);
        }
        let drain = if self.elapsed >= self.box_starts { self.decay } else { 0.0 };
        let wind = if input.winding { r.t("windPerSec") * dt } else { 0.0 };
        self.box_pct = (self.box_pct - drain * dt + wind).min(100.0).max(0.0);
        if self.box_pct <= 0.0 {
            if self.box_zero == 0.0 {
                ev.push(Ev::BoxEmpty);
            }
            self.box_zero += dt_ms;
            if self.box_zero >= r.t("boxGraceMs") {
                return self.finish(ev, "glyra");
            }
        } else if self.box_zero > 0.0 {
            self.box_zero = 0.0;
            ev.push(Ev::BoxSaved);
        }

        // Hall hunters (Thessine).
        let id_h = HALL_HUNTERS[0];
        let lvl_h = self.level(id_h);
        match self.hall.state {
            HallState::Away => {
                self.hall.cool = (self.hall.cool - dt_ms * mood[0].v * self.hunter_speed).max(0.0);
                if self.hall.cool <= 0.0 {
                    let rng = &mut self.rng;
                    let due = self.hall.pacer.step(dt_ms * mood[0].v * self.hunter_speed, lvl_h, &mut || rng.next());
                    if due {
                        self.hall.state = HallState::Room;
                        self.hall.room = r.next_toward(&self.hall.room);
                        self.hall.t = 0.0;
                        self.hall.need = self.room_need();
                        ev.push(Ev::Room { id: id_h.to_string(), room: self.hall.room.clone(), poised: false });
                    }
                }
            }
            HallState::Room => {
                if watched == self.hall.room {
                    self.frozen.push(id_h.to_string());
                } else {
                    self.hall.t += dt_ms;
                }
                if self.hall.t >= self.hall.need {
                    let nxt = r.next_toward(&self.hall.room);
                    if nxt == "hall" && self.rusher_in_hall() {
                        // one at a time: she waits at the hall doors while Umbriel is in the hall
                    } else if nxt == "hall" {
                        self.hall.state = HallState::Hall;
                        self.hall.room = "hall".to_string();
                        self.hall.t = 0.0;
                        self.hall.step = 0;
                        self.hall.need = self.step_need();
                        ev.push(Ev::Hall { id: id_h.to_string() });
                    } else {
                        self.hall.room = nxt;
                        self.hall.t = 0.0;
                        self.hall.need = self.room_need();
                        ev.push(Ev::Room { id: id_h.to_string(), room: self.hall.room.clone(), poised: false });
                    }
                }
            }
            HallState::Hall => {
                let steps = r.t("hallSteps") as i32;
                let on_cam = watched == "hall" && self.hall.step >= steps - 1;
                if on_cam {
                    self.frozen.push(id_h.to_string());
                }
                if !lit.h && !on_cam {
                    self.hall.t += dt_ms;
                }
                if self.hall.t >= self.hall.need && self.hall.step < steps {
                    self.hall.t = 0.0;
                    self.hall.step += 1;
                    self.hall.need = self.step_need();
                    ev.push(Ev::Step { id: id_h.to_string(), step: self.hall.step });
                }
                // (one hall hunter, so nobody else can already be in the office)
                if self.hall.step >= steps {
                    self.hall.state = HallState::Office;
                    self.hall.room = "office".to_string();
                    self.hall.t = 0.0;
                    self.hall.step = 0;
                    self.hall.fooled = 0.0;
                    self.hall.off = 0.0;
                    self.hall.window = r.t("windowMs");
                    self.hall.pacer.reset();
                    ev.push(Ev::Office { id: id_h.to_string() });
                }
            }
            HallState::Office => {
                if mask {
                    self.hall.fooled += dt_ms;
                    self.hall.off = (self.hall.off - dt_ms * r.t("offDrain")).max(0.0);
                    if self.hall.fooled >= r.t("leaveMs") {
                        self.hall.state = HallState::Away;
                        self.hall.room = r.home(id_h).to_string();
                        self.hall.cool = spread_ms(r.t("coolMs"), &mut self.rng, r.t("coolSpread"));
                        self.hall.pacer.reset();
                        ev.push(Ev::Leave { id: id_h.to_string() });
                    }
                } else if self.hall.fooled == 0.0 {
                    self.hall.window -= dt_ms;
                    if self.hall.window <= 0.0 {
                        return self.finish(ev, id_h);
                    }
                } else {
                    self.hall.off += dt_ms;
                    if self.hall.off >= r.t("offGraceMs") {
                        return self.finish(ev, id_h);
                    }
                }
            }
        }

        // The rusher. The mask is not checked anywhere here: it does nothing to her.
        let rl = self.level(RUSHER);
        match self.rush.state {
            RushState::Away => {
                self.rush.cool = (self.rush.cool - dt_ms * mood[1].v * self.hunter_speed).max(0.0);
                if self.rush.cool <= 0.0 {
                    let rng = &mut self.rng;
                    let due = self.rush.pacer.step(dt_ms * mood[1].v * self.hunter_speed, rl, &mut || rng.next());
                    if due {
                        self.rush.state = RushState::Poised;
                        self.rush.room = "ballroom".to_string();
                        self.rush.t = 0.0;
                        self.rush.need = self.room_need();
                        ev.push(Ev::Room { id: RUSHER.to_string(), room: "ballroom".to_string(), poised: true });
                    }
                }
            }
            RushState::Poised => {
                if watched == "ballroom" {
                    self.frozen.push(RUSHER.to_string());
                } else {
                    self.rush.t += dt_ms;
                }
                if self.rush.t >= self.rush.need && !self.walker_in_hall() {
                    self.rush.state = RushState::Run;
                    self.rush.room = "hall".to_string();
                    self.rush.t = 0.0;
                    self.rush.need = rush_run_ms(rl);
                    self.rush.flashes = 0;
                    self.rush.runs += 1;
                    self.rush.goal = rush_flashes(rl);
                    self.rush.door_need = rush_door_ms(rl);
                    ev.push(Ev::Rush { id: RUSHER.to_string(), flashes: self.rush.goal });
                }
            }
            RushState::Run | RushState::Door => {
                if flash {
                    self.rush.flashes += 1;
                    self.rush.flash_total += 1;
                    ev.push(Ev::FlashHit { id: RUSHER.to_string(), n: self.rush.flashes, of: self.rush.goal });
                }
                self.rush.t += dt_ms;
                if self.rush.flashes >= self.rush.goal {
                    self.rush.state = RushState::Away;
                    self.rush.room = r.home(RUSHER).to_string();
                    self.rush.cool = self.rest0(r.t("rushCoolMs"));
                    self.rush.pacer.reset();
                    ev.push(Ev::RushRetreat { id: RUSHER.to_string() });
                } else if self.rush.state == RushState::Run && self.rush.t >= self.rush.need {
                    self.rush.state = RushState::Door;
                    self.rush.t = 0.0;
                    self.rush.need = self.rush.door_need;
                    ev.push(Ev::Door { id: RUSHER.to_string() });
                } else if self.rush.state == RushState::Door && self.rush.t >= self.rush.need {
                    return self.finish(ev, RUSHER);
                }
            }
        }

        // The vents: one clock, one visitor at a time. When the clock fires, who comes is drawn from the vent hunters on the floor.
        if !self.vent_out() {
            self.vent_clock.cool = (self.vent_clock.cool - dt_ms * mood[2].v * self.hunter_speed).max(0.0);
            let mut pool: Vec<&'static str> = Vec::new();
            let mut top = 0;
            for id in VENT_HUNTERS {
                if self.level(id) > 0 && self.elapsed >= r.vent_from.get(id).copied().unwrap_or(0.0) {
                    pool.push(id);
                    top = top.max(self.level(id));
                }
            }
            let mut due = false;
            if self.vent_clock.cool <= 0.0 {
                let rng = &mut self.rng;
                due = self.vent_clock.pacer.step(dt_ms * mood[2].v * self.hunter_speed, top, &mut || rng.next());
            }
            if due && !pool.is_empty() {
                let mut who: &'static str = pool[0];
                if pool.len() > 1 {
                    let mut total = 0;
                    for id in pool.iter().copied() {
                        total += self.level(id);
                    }
                    let mut roll = self.rng.next() * total as f64;
                    who = pool[pool.len() - 1];
                    for id in pool.iter().copied() {
                        roll -= self.level(id) as f64;
                        if roll < 0.0 {
                            who = id;
                            break;
                        }
                    }
                }
                let side = if self.rng.next() < self.side_bias { 'L' } else { 'R' };
                let duct = duct_of(side);
                if who == "verrine" {
                    let need = self.duct_need();
                    let h = &mut self.verrine;
                    h.side = side;
                    h.state = VentState::Duct;
                    h.room = duct.to_string();
                    h.t = 0.0;
                    h.need = need;
                    ev.push(Ev::Duct { id: "verrine".to_string(), side, room: duct.to_string(), leg: None });
                } else if who == "haelith" {
                    let need = self.haelith_duct();
                    let h = &mut self.haelith;
                    h.side = side;
                    h.state = VentState::Duct;
                    h.room = duct.to_string();
                    h.t = 0.0;
                    h.need = need;
                    h.lit = 0.0;
                    h.step = 0;
                    ev.push(Ev::Duct { id: "haelith".to_string(), side, room: duct.to_string(), leg: Some(0) });
                } else {
                    let need = self.ilyra_need();
                    let h = &mut self.ilyra;
                    h.side = side;
                    h.state = VentState::Duct;
                    h.room = duct.to_string();
                    h.t = 0.0;
                    h.need = need;
                    h.lit = 0.0;
                    h.step = 1;
                    ev.push(Ev::Duct { id: "ilyra".to_string(), side, room: duct.to_string(), leg: None });
                }
            }
        }

        // Verrine.
        match self.verrine.state {
            VentState::Duct => {
                if watched == self.verrine.room {
                    self.frozen.push("verrine".to_string());
                } else {
                    self.verrine.t += dt_ms;
                }
                if self.verrine.t >= self.verrine.need {
                    let wait = self.kill_wait(r.t("ventWaitMs"));
                    let h = &mut self.verrine;
                    h.state = VentState::Vent;
                    h.room = r.next_toward(&h.room);
                    h.t = 0.0;
                    h.lit = 0.0;
                    h.wait = wait;
                    ev.push(Ev::Vent { id: "verrine".to_string(), side: h.side });
                }
            }
            VentState::Vent => {
                let side = self.verrine.side;
                if mask {
                    self.verrine.t += dt_ms * r.t("ventMaskRate");
                } else if lit.side(side) {
                    self.verrine.lit += dt_ms;
                } else {
                    self.verrine.lit = (self.verrine.lit - dt_ms * 0.5).max(0.0);
                    self.verrine.t += dt_ms;
                }
                if self.verrine.lit >= r.t("ventNeedMs") {
                    self.rest("verrine", r.t("ventCoolMs"), &mut ev);
                } else if self.verrine.t >= self.verrine.wait {
                    return self.finish(ev, "verrine");
                }
            }
            _ => {}
        }

        // Haelith, the fetcher: bounds down a duct, waits for a quick throw of the light, fetches it across to the other duct,
        // wants it there too.
        match self.haelith.state {
            VentState::Duct => {
                if watched == self.haelith.room {
                    self.frozen.push("haelith".to_string());
                } else {
                    self.haelith.t += dt_ms;
                }
                if self.haelith.t >= self.haelith.need {
                    let wait = self.kill_wait(r.t("haelithWaitMs"));
                    let h = &mut self.haelith;
                    h.state = VentState::Vent;
                    h.room = r.next_toward(&h.room);
                    h.t = 0.0;
                    h.lit = 0.0;
                    h.wait = wait;
                    ev.push(Ev::Vent { id: "haelith".to_string(), side: h.side });
                }
            }
            VentState::Fetch => {
                self.haelith.t += dt_ms;
                if self.haelith.t >= r.t("haelithFetchMs") {
                    let need = self.haelith_duct();
                    let h = &mut self.haelith;
                    h.side = other_side(h.side);
                    h.state = VentState::Duct;
                    h.room = DUCTS[side_index(h.side)].to_string();
                    h.t = 0.0;
                    h.need = need;
                    h.lit = 0.0;
                    ev.push(Ev::Duct { id: "haelith".to_string(), side: h.side, room: h.room.clone(), leg: Some(1) });
                }
            }
            VentState::Vent => {
                let side = self.haelith.side;
                let beam = !mask && lit.side(side);
                if beam {
                    self.haelith.lit += dt_ms;
                } else {
                    self.haelith.lit = (self.haelith.lit - dt_ms * 0.5).max(0.0);
                    self.haelith.t += dt_ms;
                }
                if self.haelith.lit >= r.t("haelithNeedMs") {
                    if self.haelith.step == 0 {
                        let to = other_side(side);
                        let h = &mut self.haelith;
                        h.step = 1;
                        h.state = VentState::Fetch;
                        h.room = r.home("haelith").to_string();
                        h.t = 0.0;
                        h.lit = 0.0;
                        ev.push(Ev::Fetch { id: "haelith".to_string(), side, to });
                    } else {
                        self.haelith.step = 0;
                        self.haelith.t = 0.0;
                        self.haelith.lit = 0.0;
                        self.rest("haelith", r.t("haelithCoolMs"), &mut ev);
                    }
                } else if self.haelith.t >= self.haelith.wait {
                    return self.finish(ev, "haelith");
                }
            }
            VentState::Away => {}
        }

        // Ilyra, the creeper: a step only in the dark, frozen in the light, sent home by enough light.
        if self.ilyra.state != VentState::Away {
            let side = self.ilyra.side;
            let beam = !mask && lit.side(side);
            if beam {
                self.ilyra.lit += dt_ms;
            } else {
                self.ilyra.lit = (self.ilyra.lit - dt_ms).max(0.0);
                self.ilyra.t += dt_ms;
            }
            if self.ilyra.lit >= r.t("ilyraNeedMs") {
                self.ilyra.step = 0;
                self.ilyra.t = 0.0;
                self.ilyra.lit = 0.0;
                self.rest("ilyra", r.t("ilyraCoolMs"), &mut ev);
            } else if self.ilyra.state == VentState::Duct && self.ilyra.t >= self.ilyra.need {
                self.ilyra.step += 1;
                self.ilyra.t = 0.0;
                if self.ilyra.step <= r.t("ilyraSteps") as i32 {
                    self.ilyra.need = self.ilyra_need();
                    ev.push(Ev::Creep { id: "ilyra".to_string(), side, step: self.ilyra.step });
                } else {
                    let wait = self.kill_wait(r.t("ilyraVentMs"));
                    let h = &mut self.ilyra;
                    h.state = VentState::Vent;
                    h.room = r.next_toward(&h.room);
                    h.wait = wait;
                    ev.push(Ev::Vent { id: "ilyra".to_string(), side });
                }
            } else if self.ilyra.state == VentState::Vent && self.ilyra.t >= self.ilyra.wait {
                return self.finish(ev, "ilyra");
            }
        }

        if self.elapsed >= NIGHT_MS {
            self.over = Over::Win;
            ev.push(Ev::Win);
        }
        ev
    }

    // --- admin and tests: put things where they belong -------------------------------------------------------------

    /// God mode: a hunter that would catch you is sent home, the box is wound, the flashlight keeps a little charge.
    pub fn god_rescue(&mut self) {
        let r = rules();
        self.over = Over::None;
        self.by.clear();
        self.hall.state = HallState::Away;
        self.hall.room = r.home("thessine").to_string();
        self.hall.cool = 4000.0;
        self.rush.state = RushState::Away;
        self.rush.room = r.home(RUSHER).to_string();
        self.rush.cool = 8000.0;
        for id in VENT_HUNTERS {
            let home = r.home(id).to_string();
            let h = self.vent_state_mut(id);
            h.state = VentState::Away;
            h.room = home;
            h.step = 0;
        }
        self.vent_clock.cool = 6000.0;
        self.box_zero = 0.0;
        self.box_pct = self.box_pct.max(50.0);
        self.dark = 0.0;
        self.battery = self.battery.max(20.0);
    }

    /// Put a hunter where she belongs in that place (Thessine on the floor, in the hall or in the office; Umbriel poised in the
    /// game area or running the hall; the vent hunters in a duct, at a vent or home; only one vent hunter is ever out, so the
    /// others go home first).
    pub fn place_hunter(&mut self, id: &str, place: &str) {
        let r = rules();
        match id {
            "thessine" => {
                let windows = r.t("windowMs");
                let h = &mut self.hall;
                if place == "office" {
                    h.state = HallState::Office;
                    h.room = "office".to_string();
                    h.t = 0.0;
                    h.step = 0;
                    h.fooled = 0.0;
                    h.off = 0.0;
                    h.window = windows;
                } else if place == "hall" {
                    h.state = HallState::Hall;
                    h.room = "hall".to_string();
                    h.t = 0.0;
                    h.step = 2;
                    h.need = 2300.0;
                } else if place == "ballroom" || place == "gallery" {
                    h.state = HallState::Room;
                    h.room = "ballroom".to_string();
                    h.t = 0.0;
                    h.need = 4000.0;
                } else {
                    h.state = HallState::Away;
                    h.room = r.home("thessine").to_string();
                    h.cool = 4000.0;
                }
            }
            "umbriel" => {
                let lv = self.level(RUSHER);
                let x = &mut self.rush;
                if place == "hall" {
                    x.state = RushState::Run;
                    x.room = "hall".to_string();
                    x.t = 0.0;
                    x.need = rush_run_ms(lv);
                    x.flashes = 0;
                    x.goal = rush_flashes(lv);
                    x.door_need = rush_door_ms(lv);
                } else if place == "ballroom" {
                    x.state = RushState::Poised;
                    x.room = "ballroom".to_string();
                    x.t = 0.0;
                    x.need = 4000.0;
                } else {
                    x.state = RushState::Away;
                    x.room = r.home(RUSHER).to_string();
                    x.cool = 8000.0;
                }
            }
            "verrine" | "haelith" | "ilyra" => {
                for other in VENT_HUNTERS {
                    if other != id {
                        let home = r.home(other).to_string();
                        let o = self.vent_state_mut(other);
                        o.state = VentState::Away;
                        o.room = home;
                    }
                }
                let side = if place.ends_with('L') { 'L' } else { 'R' };
                let home = r.home(id).to_string();
                let v = self.vent_state_mut(id);
                if place == "ductL" || place == "ductR" {
                    v.state = VentState::Duct;
                    v.room = place.to_string();
                    v.side = side;
                    v.t = 0.0;
                    v.need = 4000.0;
                    v.lit = 0.0;
                    v.step = 1;
                } else if place == "ventL" || place == "ventR" {
                    v.state = VentState::Vent;
                    v.room = place.to_string();
                    v.side = side;
                    v.t = 0.0;
                    v.lit = 0.0;
                    v.wait = 6000.0;
                    v.step = 0;
                } else {
                    v.state = VentState::Away;
                    v.room = home;
                    v.step = 0;
                }
            }
            _ => {}
        }
    }

    pub fn all_home(&mut self) {
        for id in ["thessine", "umbriel", "verrine", "haelith", "ilyra"] {
            self.place_hunter(id, "home");
        }
    }

    /// Glyra is on the floor: from night 2, or whenever a custom slider gives her a level.
    pub fn glyra_present(&self) -> bool {
        active_hunters(self.night_idx).iter().any(|h| h == "glyra") || self.levels.get("glyra").map(|l| *l > 0).unwrap_or(false)
    }

    /// Set Glyra's custom level (and the box's decay with it).
    pub fn set_hunter_level(&mut self, id: &str, level: i32) {
        self.levels.insert(id.to_string(), level);
        if id == "glyra" {
            self.decay = box_decay_for_level(level);
        }
    }

    /// FNV-1a of the state that matters (stable across runs).
    pub fn digest(&self) -> u64 {
        let mut f = Fnv::new();
        f.f64(self.elapsed).f64(self.battery).f64(self.box_pct).f64(self.dark).f64(self.box_zero);
        f.i32(self.hall.state as i32).str(&self.hall.room).f64(self.hall.t).i32(self.hall.step);
        f.i32(self.rush.state as i32).i32(self.rush.flashes).i32(self.rush.runs).i32(self.rush.flash_total);
        for id in VENT_HUNTERS {
            let h = self.vent_state(id);
            f.i32(h.state as i32).str(&h.room).i32(h.step).f64(h.t).f64(h.lit);
        }
        f.f64(self.vent_clock.cool).f64(self.rng.state() as f64);
        f.i32(self.flash_count).i32(self.vent_flash_count);
        f.i32(self.over as i32).str(&self.by);
        f.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim(night: i32, seed: u32) -> MaskSim {
        MaskSim::new(night, seed, BTreeMap::new(), false)
    }

    /// Run `ms` of the night in 100 ms steps with a fixed input; every event comes back.
    fn run(s: &mut MaskSim, ms: f64, input: &SimInput) -> Vec<Ev> {
        let mut out = Vec::new();
        let mut t = 0.0;
        while t < ms && !s.is_over() {
            out.extend(s.step(100.0, input));
            t += 100.0;
        }
        out
    }

    fn quiet() -> SimInput {
        SimInput { cam: "music".to_string(), ..SimInput::default() }
    }

    #[test]
    fn the_same_seed_gives_the_same_night() {
        let mut a = sim(3, 777);
        let mut b = sim(3, 777);
        let ea: Vec<String> = run(&mut a, 200_000.0, &quiet()).iter().map(|e| e.name()).collect();
        let eb: Vec<String> = run(&mut b, 200_000.0, &quiet()).iter().map(|e| e.name()).collect();
        assert_eq!(ea, eb);
        assert_eq!(a.digest(), b.digest());
        let mut c = sim(3, 778);
        let ec: Vec<String> = run(&mut c, 200_000.0, &quiet()).iter().map(|e| e.name()).collect();
        assert_ne!(ea, ec, "a different seed plays a different night");
    }

    #[test]
    fn a_night_with_nobody_in_the_way_is_won_at_six() {
        let mut s = sim(1, 5);
        s.infinite_battery = true;
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.decay = 0.0;
        let ev = run(&mut s, 280_000.0, &quiet());
        assert_eq!(s.over, Over::Win);
        assert!(matches!(ev.last(), Some(Ev::Win)));
        assert!(s.elapsed >= NIGHT_MS);
    }

    #[test]
    fn the_flashlight_drains_per_second_of_light() {
        let mut s = sim(1, 1);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        let on = SimInput { light_l: true, ..quiet() };
        // one second of the left vent light on night 1: 100 / 66 per second, plus one flash of 0.25
        run(&mut s, 1000.0, &on);
        let want = 100.0 - 100.0 / 66.0 - 0.25;
        assert!((s.battery - want).abs() < 1e-6, "battery {} want {}", s.battery, want);
        assert_eq!(s.vent_flash_count, 1);
        // the monitor and the lamp cost far less
        let mut m = sim(1, 1);
        m.hall.cool = 1e12;
        m.rush.cool = 1e12;
        m.vent_clock.cool = 1e12;
        run(&mut m, 1000.0, &SimInput { monitor: true, lamp: true, ..quiet() });
        let want_m = 100.0 - (100.0 / 66.0) * (0.1 + 0.025);
        assert!((m.battery - want_m).abs() < 1e-6);
    }

    #[test]
    fn a_dead_battery_turns_the_lights_off_and_nyxie_comes() {
        let mut s = sim(1, 1);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.battery = 0.0;
        let ev = run(&mut s, 1000.0, &SimInput { light_h: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::Dark { .. })));
        assert!(!s.lit.h, "no light with no battery");
        let grace = dark_grace_ms(3);
        let ev2 = run(&mut s, grace + 500.0, &quiet());
        assert!(ev2.iter().any(|e| matches!(e, Ev::Caught { by } if by == "nyxie")));
        assert_eq!(s.over, Over::Caught);
    }

    #[test]
    fn the_music_box_runs_down_after_two_am_on_night_one_and_glyra_comes() {
        let mut s = sim(1, 1);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        // wound and silent until 2 AM
        run(&mut s, 89_000.0, &quiet());
        assert!((s.box_pct - 100.0).abs() < 1e-9);
        let ev = run(&mut s, 3000.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::BoxStart)));
        assert!(s.box_pct < 100.0);
        // empty, then six seconds of grace
        s.box_pct = 0.01;
        let ev = run(&mut s, 500.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::BoxEmpty)));
        let ev = run(&mut s, 7000.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Caught { by } if by == "glyra")));
    }

    #[test]
    fn winding_saves_the_box() {
        let mut s = sim(1, 1);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        s.elapsed = 100_000.0;
        s.box_pct = 0.0;
        let ev = run(&mut s, 300.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::BoxEmpty)));
        let ev = run(&mut s, 400.0, &SimInput { monitor: true, winding: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::BoxSaved)));
        assert!(s.box_pct > 0.0 && s.box_zero == 0.0);
    }

    #[test]
    fn thessine_leaves_after_the_mask_has_fooled_her_for_two_and_a_half_seconds() {
        let mut s = sim(1, 1);
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.place_hunter("thessine", "office");
        let masked = SimInput { mask: true, ..quiet() };
        let ev = run(&mut s, 2300.0, &masked);
        assert!(ev.is_empty() || !ev.iter().any(|e| matches!(e, Ev::Leave { .. })));
        let ev = run(&mut s, 400.0, &masked);
        assert!(ev.iter().any(|e| matches!(e, Ev::Leave { .. })));
        assert_eq!(s.hall.state, HallState::Away);
        assert_eq!(s.hall.room, "costume");
    }

    #[test]
    fn without_the_mask_thessine_catches_after_3500_ms_and_a_late_mask_gets_1200_ms_of_grace() {
        let mut s = sim(1, 1);
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.place_hunter("thessine", "office");
        let ev = run(&mut s, 3400.0, &quiet());
        assert!(ev.is_empty());
        let ev = run(&mut s, 200.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Caught { by } if by == "thessine")));
        // fooled once, then the mask drops: 1200 ms of grace
        let mut g = sim(1, 1);
        g.rush.cool = 1e12;
        g.vent_clock.cool = 1e12;
        g.place_hunter("thessine", "office");
        run(&mut g, 500.0, &SimInput { mask: true, ..quiet() });
        let ev = run(&mut g, 1100.0, &quiet());
        assert!(ev.is_empty());
        let ev = run(&mut g, 200.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Caught { .. })));
    }

    #[test]
    fn a_watched_room_holds_her_still() {
        let mut s = sim(1, 1);
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.place_hunter("thessine", "ballroom");
        let cam = SimInput { monitor: true, cam: "ballroom".to_string(), ..quiet() };
        run(&mut s, 6000.0, &cam);
        assert_eq!(s.hall.state, HallState::Room);
        assert_eq!(s.hall.t, 0.0);
        assert_eq!(s.frozen, vec!["thessine".to_string()]);
        // look away and she goes to the hall once her room timer runs out
        let ev = run(&mut s, 4800.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Hall { .. })));
        assert_eq!(s.hall.state, HallState::Hall);
    }

    #[test]
    fn the_hall_light_holds_her_steps_and_the_fourth_step_puts_her_in_the_office() {
        let mut s = sim(1, 1);
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        s.place_hunter("thessine", "hall");
        s.hall.step = 0;
        s.hall.need = 2000.0;
        // the light on: the timer never moves
        run(&mut s, 5000.0, &SimInput { light_h: true, ..quiet() });
        assert_eq!(s.hall.t, 0.0);
        assert_eq!(s.hall.step, 0);
        // light off: she walks
        let ev = run(&mut s, 40_000.0, &quiet());
        let steps = ev.iter().filter(|e| matches!(e, Ev::Step { .. })).count();
        assert_eq!(steps, 4);
        assert!(ev.iter().any(|e| matches!(e, Ev::Office { .. })));
    }

    #[test]
    fn umbriel_is_turned_back_by_three_flashes_and_only_the_flashes_count() {
        let mut s = sim(3, 9);
        s.hall.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        s.place_hunter("umbriel", "hall");
        let goal = rush_flashes(10);
        assert_eq!(s.rush.goal, goal);
        assert_eq!(goal, 4, "level 10 wants four flashes");
        let mut hits = 0;
        for _ in 0..goal {
            let on = run(&mut s, 100.0, &SimInput { light_h: true, ..quiet() });
            hits += on.iter().filter(|e| matches!(e, Ev::FlashHit { .. })).count();
            run(&mut s, 100.0, &quiet());
        }
        assert_eq!(hits as i32, goal);
        assert_eq!(s.rush.state, RushState::Away);
        assert_eq!(s.rush.room, "gallery");
        // held light is one flash, not many
        let mut h = sim(3, 9);
        h.hall.cool = 1e12;
        h.vent_clock.cool = 1e12;
        h.infinite_battery = true;
        h.place_hunter("umbriel", "hall");
        run(&mut h, 600.0, &SimInput { light_h: true, ..quiet() });
        assert_eq!(h.rush.flashes, 1);
    }

    #[test]
    fn umbriel_unchecked_runs_the_hall_and_catches_at_the_door() {
        let mut s = sim(3, 9);
        s.hall.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.place_hunter("umbriel", "hall");
        let lv = s.level(RUSHER);
        let total = rush_run_ms(lv) + rush_door_ms(lv);
        let ev = run(&mut s, total - 200.0, &SimInput { mask: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::Door { .. })));
        assert_eq!(s.over, Over::None, "the mask does nothing to her");
        let ev = run(&mut s, 400.0, &SimInput { mask: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::Caught { by } if by == "umbriel")));
    }

    #[test]
    fn she_waits_at_the_hall_doors_while_umbriel_runs() {
        let mut s = sim(3, 9);
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.place_hunter("thessine", "ballroom");
        s.hall.need = 100.0;
        s.place_hunter("umbriel", "hall");
        let ev = run(&mut s, 1000.0, &SimInput { light_h: false, ..quiet() });
        assert!(!ev.iter().any(|e| matches!(e, Ev::Hall { .. })));
        assert_eq!(s.hall.state, HallState::Room);
    }

    #[test]
    fn the_vent_clock_sends_one_visitor_at_a_time() {
        let mut s = sim(5, 31);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.infinite_battery = true;
        s.decay = 0.0;
        s.elapsed = 100_000.0;
        s.vent_clock.cool = 0.0;
        let mut out = 0;
        let mut seen = Vec::new();
        for _ in 0..1500 {
            let ev = s.step(100.0, &quiet());
            for e in ev {
                if let Ev::Duct { id, .. } = &e {
                    seen.push(id.clone());
                }
            }
            let n = VENT_HUNTERS.iter().filter(|id| s.vent_state(id).state != VentState::Away).count();
            assert!(n <= 1, "two vent hunters out at once");
            if n == 1 {
                out += 1;
            }
            if s.is_over() {
                break;
            }
        }
        assert!(out > 0 && !seen.is_empty());
    }

    #[test]
    fn verrine_crawls_the_duct_waits_in_the_vent_and_the_light_sends_her_home() {
        let mut s = sim(2, 4);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        s.place_hunter("verrine", "ductL");
        s.verrine.need = 1000.0;
        let ev = run(&mut s, 1100.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Vent { id, side: 'L' } if id == "verrine")));
        assert_eq!(s.verrine.room, "ventL");
        assert_eq!(s.vent_seen('L'), None, "dark: nothing to see");
        assert!(s.vent_glint('L'));
        let ev = run(&mut s, 1500.0, &SimInput { light_l: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::VentRetreat { id, .. } if id == "verrine")));
        assert_eq!(s.verrine.state, VentState::Away);
        assert_eq!(s.verrine.room, "crawl");
    }

    #[test]
    fn verrine_catches_a_dark_vent_after_her_wait_and_the_mask_only_slows_her() {
        let mut s = sim(2, 4);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.place_hunter("verrine", "ventR");
        s.verrine.wait = 6000.0;
        let ev = run(&mut s, 5900.0, &quiet());
        assert!(ev.is_empty());
        let ev = run(&mut s, 300.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Caught { by } if by == "verrine")));
        let mut m = sim(2, 4);
        m.hall.cool = 1e12;
        m.rush.cool = 1e12;
        m.vent_clock.cool = 1e12;
        m.place_hunter("verrine", "ventR");
        m.verrine.wait = 6000.0;
        run(&mut m, 6200.0, &SimInput { mask: true, ..quiet() });
        assert_eq!(m.over, Over::None, "0.9 of the clock under the mask");
    }

    #[test]
    fn haelith_wants_the_light_twice_and_fetches_it_across() {
        let mut s = sim(4, 6);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        s.place_hunter("haelith", "ventL");
        let ev = run(&mut s, 600.0, &SimInput { light_l: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::Fetch { side: 'L', to: 'R', .. })));
        assert_eq!(s.haelith.state, VentState::Fetch);
        assert_eq!(s.haelith.room, "crawl");
        let ev = run(&mut s, 1600.0, &quiet());
        assert!(ev.iter().any(|e| matches!(e, Ev::Duct { side: 'R', leg: Some(1), .. })));
        assert_eq!(s.haelith.room, "ductR");
        s.place_hunter("haelith", "ventR");
        s.haelith.step = 1;
        let ev = run(&mut s, 700.0, &SimInput { light_r: true, ..quiet() });
        assert!(ev.iter().any(|e| matches!(e, Ev::VentRetreat { id, .. } if id == "haelith")));
    }

    #[test]
    fn ilyra_only_creeps_in_the_dark_and_three_steps_bring_her_to_the_vent() {
        let mut s = sim(5, 8);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s.infinite_battery = true;
        s.place_hunter("ilyra", "ductL");
        s.ilyra.need = 500.0;
        // with the light on her duct she never creeps
        run(&mut s, 400.0, &SimInput { light_l: true, ..quiet() });
        assert_eq!(s.ilyra.step, 1);
        s.ilyra.lit = 0.0;
        s.ilyra.t = 0.0;
        let mut creeps = 0;
        let mut vented = false;
        for _ in 0..200 {
            for e in s.step(100.0, &quiet()) {
                match e {
                    Ev::Creep { .. } => creeps += 1,
                    Ev::Vent { .. } => vented = true,
                    _ => {}
                }
            }
            if vented {
                break;
            }
        }
        assert_eq!(creeps, 2, "steps 2 and 3 are announced, the fourth is the vent");
        assert!(vented);
        assert_eq!(s.ilyra.room, "ventL");
    }

    #[test]
    fn custom_levels_replace_the_night_table() {
        let mut lv = BTreeMap::new();
        lv.insert("thessine".to_string(), 0);
        lv.insert("umbriel".to_string(), 20);
        lv.insert("glyra".to_string(), 8);
        let s = MaskSim::new(5, 1, lv, true);
        assert_eq!(s.base_level("thessine"), 0);
        assert_eq!(s.base_level("umbriel"), 20);
        assert_eq!(s.base_level("verrine"), 9, "unlisted hunters keep the night table (night 5)");
        assert!((s.decay - box_decay_for_level(8)).abs() < 1e-9);
        assert!(s.glyra_present());
        assert_eq!(s.box_starts, 0.0, "custom nights run the box from the start");
    }

    #[test]
    fn the_level_steps_up_from_three_and_four_am() {
        let mut s = sim(3, 1);
        assert_eq!(s.level("thessine"), 9);
        s.elapsed = 3.0 * crate::core::HOUR_MS;
        assert_eq!(s.level("thessine"), 10);
        s.elapsed = 4.0 * crate::core::HOUR_MS;
        assert_eq!(s.level("thessine"), 11);
        assert_eq!(s.level("verrine"), 5 + 2);
        assert_eq!(s.level("ilyra"), 0, "level 0 never wakes up");
    }

    #[test]
    fn god_rescue_sends_everyone_home() {
        let mut s = sim(5, 3);
        s.place_hunter("thessine", "office");
        s.place_hunter("umbriel", "hall");
        s.place_hunter("ilyra", "ventL");
        s.over = Over::Caught;
        s.by = "ilyra".into();
        s.box_pct = 3.0;
        s.god_rescue();
        assert_eq!(s.over, Over::None);
        assert_eq!(s.hall.state, HallState::Away);
        assert_eq!(s.rush.state, RushState::Away);
        assert!(VENT_HUNTERS.iter().all(|id| s.vent_state(id).state == VentState::Away));
        assert!(s.box_pct >= 50.0 && s.battery >= 20.0);
    }

    #[test]
    fn a_whole_night_reaches_an_outcome_for_every_seed_tried() {
        for seed in [1u32, 2, 3, 12345] {
            for night in 1..=6 {
                let mut s = sim(night, seed);
                let ev = run(&mut s, 300_000.0, &quiet());
                assert!(s.is_over(), "night {night} seed {seed}");
                assert!(ev.iter().any(|e| matches!(e, Ev::Win | Ev::Caught { .. })));
            }
        }
    }
}
