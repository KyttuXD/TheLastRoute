//! Bedtime night type (Godot `game/night_watch/bedroom`, rules from `bedroom_logic.gd`). Pure rules and scoring. The
//! picture, the controls and the call box are the driver in `modes_play/bedtime.rs`, which reads this state.
//!
//! You are in bed with one free hand. Three things come for you, each from its own side:
//!   Loxelle  breathes behind the closet door. Hold the closet door shut until she leaves.
//!   Javelle  waits in the hall doorway. A light on the hall makes her leave.
//!   Haelith  creeps toward the bed, only while the room is dark. Light on the bed pushes her back.
//! Holding the door uses both hands, so no light while holding, and only one light at a time. The battery never
//! recharges. Sitting still for a while makes the house restless.

use crate::data::{int, list, num, rules};
use crate::framework::{chance, spread_ms, tempo, NightWatchStream};
use serde_json::Value;

/// Every tunable of the mode, read once from `data/rules/bedtime.json` (the 2D numbers; that file documents the keys).
#[derive(Clone, Debug)]
struct Rules {
    night_ms: f64,
    hours: i32,
    time_scale: f64,
    flash_seconds: Vec<f64>,
    max_ai: i32,
    points_per_level: i32,
    custom_battery_night: u32,
    battery_full: f64,
    creep_full: f64,
    /// [minimum share of the top score, rating], best first.
    ratings: Vec<(f64, String)>,
    var_wait_spread: f64,
    var_door_tempo: f64,
    var_haelith_tempo: f64,
    var_swap_first: f64,
    wait: WaitRule,
    /// [first hour, bump], latest hour first.
    level_bumps: Vec<(i32, i32)>,
    stage_jitter: f64,
    hold_need_ms: f64,
    light_need_ms: f64,
    need_decay: f64,
    afk_ms: f64,
    afk_bonus: i32,
    afk_speed: f64,
    keen_cap: i32,
    bed_gain: f64,
    bed_gain_per_level: f64,
    bed_lit_drain_per_sec: f64,
    /// Haelith's exhale is heard once she has got this far toward the bed (`bed_stir_at`).
    bed_stir_at: f64,
    cue: CueRule,
    nights: Vec<NightRow>,
    custom_row: NightRow,
}

/// The wait between moves: a window that slides down with the hunter's level.
#[derive(Clone, Copy, Debug)]
struct WaitRule {
    base_ms: f64,
    per_level_ms: f64,
    floor_ms: f64,
    window_ms: f64,
}

/// How often breathing is heard again while the hunter stays put: the door stages (near is the faster one) and Haelith's
/// exhale at the foot of the bed.
#[derive(Clone, Copy, Debug)]
struct CueRule {
    near_ms: f64,
    far_ms: f64,
    haelith_ms: f64,
}

/// One night's pace. Doors use `first` and `rest` (Haelith has no door timing, so her slots are 0).
#[derive(Clone, Copy, Debug)]
struct NightRow {
    ai: [i32; 3],
    breath_ms: f64,
    near_ms: f64,
    first: [f64; 3],
    rest: [f64; 3],
}

fn night_row_of(v: &Value) -> NightRow {
    let at = |key: &str, i: usize| num(v, &format!("{key}.{i}"));
    NightRow {
        ai: [0, 1, 2].map(|i| int(v, &format!("ai.{i}")) as i32),
        breath_ms: num(v, "breath_ms"),
        near_ms: num(v, "near_ms"),
        first: [at("first_ms", 0), at("first_ms", 1), 0.0],
        rest: [at("rest_ms", 0), at("rest_ms", 1), 0.0],
    }
}

impl Rules {
    fn load() -> Self {
        let r = rules("bedtime");
        let night_ms = num(r, "night_ms");
        Self {
            night_ms,
            hours: int(r, "hours") as i32,
            time_scale: night_ms / num(r, "tuned_night_ms"),
            flash_seconds: list(r, "flash_seconds").iter().filter_map(Value::as_f64).collect(),
            max_ai: int(r, "max_ai") as i32,
            points_per_level: int(r, "points_per_level") as i32,
            custom_battery_night: int(r, "custom_battery_night") as u32,
            battery_full: num(r, "battery_full"),
            creep_full: num(r, "creep_full"),
            ratings: (0..list(r, "ratings").len())
                .map(|i| (num(r, &format!("ratings.{i}.min_share")), crate::data::text(r, &format!("ratings.{i}.name")).to_string()))
                .collect(),
            var_wait_spread: num(r, "variety.wait_spread"),
            var_door_tempo: num(r, "variety.door_tempo"),
            var_haelith_tempo: num(r, "variety.haelith_tempo"),
            var_swap_first: num(r, "variety.swap_first"),
            wait: WaitRule {
                base_ms: num(r, "wait.base_ms"),
                per_level_ms: num(r, "wait.per_level_ms"),
                floor_ms: num(r, "wait.floor_ms"),
                window_ms: num(r, "wait.window_ms"),
            },
            level_bumps: (0..list(r, "level_bumps").len())
                .map(|i| (int(r, &format!("level_bumps.{i}.from_hour")) as i32, int(r, &format!("level_bumps.{i}.bump")) as i32))
                .collect(),
            stage_jitter: num(r, "stage_jitter"),
            hold_need_ms: num(r, "hold_need_ms"),
            light_need_ms: num(r, "light_need_ms"),
            need_decay: num(r, "need_decay"),
            afk_ms: num(r, "afk_ms"),
            afk_bonus: int(r, "afk_bonus") as i32,
            afk_speed: num(r, "afk_speed"),
            keen_cap: int(r, "keen_cap") as i32,
            bed_gain: num(r, "bed_gain"),
            bed_gain_per_level: num(r, "bed_gain_per_level"),
            bed_lit_drain_per_sec: num(r, "bed_lit_drain_per_sec"),
            bed_stir_at: num(r, "bed_stir_at"),
            cue: CueRule {
                near_ms: num(r, "breath_cue.near_ms"),
                far_ms: num(r, "breath_cue.far_ms"),
                haelith_ms: num(r, "breath_cue.haelith_ms"),
            },
            nights: list(r, "nights").iter().map(night_row_of).collect(),
            custom_row: night_row_of(&r["custom_night"]),
        }
    }

    fn hour_index(&self, elapsed_ms: f64) -> i32 {
        ((elapsed_ms / (self.night_ms / f64::from(self.hours))).floor() as i32).clamp(0, self.hours - 1)
    }

    /// The hunters step up at 3 AM and 4 AM. A hunter at level 0 never wakes.
    fn effective_level(&self, base: i32, hour: i32) -> i32 {
        if base <= 0 {
            return 0;
        }
        let bump = self.level_bumps.iter().find(|(from, _)| hour >= *from).map_or(0, |(_, bump)| *bump);
        base + bump
    }

    fn flash_seconds(&self, night_index: usize) -> f64 {
        (self.flash_seconds[night_index] * self.time_scale).round()
    }
}

/// How many campaign nights the mode has (its rules file lists one row per night).
pub fn night_count() -> u32 {
    Rules::load().nights.len() as u32
}

/// The most a hunter can be pushed to on a Custom Night: the three sliders at full.
pub fn custom_max_score() -> i32 {
    let r = Rules::load();
    3 * r.max_ai * r.points_per_level
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hunter {
    Loxelle,
    Javelle,
    Haelith,
}

impl Hunter {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Light {
    #[default]
    Off,
    Hall,
    Bed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BedtimeInput {
    pub hold: bool,
    pub light: Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Caught(Hunter),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BedtimeEvent {
    Restless,
    Breath(Hunter),
    Near(Hunter),
    /// Breathing heard again while a hunter stays at her door, or Haelith's exhale once she is close to the bed.
    Breathing(Hunter),
    Leave(Hunter),
    Stir { p: f64 },
    Caught(Hunter),
    Win,
}

#[derive(Clone, Debug)]
pub struct BedtimeOptions {
    pub night: u32,
    pub seed: i64,
    /// Custom Night: the three sliders (0 to 20) become the hunters' levels.
    pub custom: Option<[i32; 3]>,
    pub infinite_battery: bool,
    pub var_rng: Option<NightWatchStream>,
}

impl BedtimeOptions {
    pub fn campaign(night: u32, seed: i64) -> Self {
        Self { night, seed, custom: None, infinite_battery: false, var_rng: None }
    }

    /// A Custom Night uses Night 5's battery, as in the 2D menu.
    pub fn custom(sliders: [i32; 3], seed: i64) -> Self {
        Self { night: Rules::load().custom_battery_night, seed, custom: Some(sliders), infinite_battery: false, var_rng: None }
    }
}

/// Points for a Custom Night's sliders. A campaign night scores nothing here; the clear is saved by the shell.
pub fn custom_score(sliders: [i32; 3]) -> i32 {
    let r = Rules::load();
    sliders.into_iter().map(|s| s.clamp(0, r.max_ai)).sum::<i32>() * r.points_per_level
}

pub fn rating_of(score: i32, max_score: i32) -> String {
    let share = if max_score > 0 { f64::from(score) / f64::from(max_score) } else { 0.0 };
    let r = Rules::load();
    let found = r.ratings.iter().find(|(min, _)| share >= *min).or(r.ratings.last());
    found.map_or("", |(_, name)| name.as_str()).to_string()
}

/// Wait between moves: a random time from a three-second window that slides down with the hunter's level.
#[derive(Clone, Debug, Default)]
struct Pacer {
    t: f64,
    next: Option<f64>,
}

fn draw_wait(rng: &mut NightWatchStream, level: i32, wait: &WaitRule) -> f64 {
    let lo = (wait.base_ms - f64::from(level) * wait.per_level_ms).max(wait.floor_ms);
    lo + rng.next() * wait.window_ms
}

impl Pacer {
    /// Advances by `dt_ms`; true when the hunter is due to move.
    fn step(&mut self, rng: &mut NightWatchStream, dt_ms: f64, level: i32, rule: &WaitRule) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        let wait = self.next.unwrap_or_else(|| draw_wait(rng, level, rule));
        self.t += dt_ms;
        if self.t < wait {
            self.next = Some(wait);
            return false;
        }
        self.t = 0.0;
        self.next = Some(draw_wait(rng, level, rule));
        true
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.next = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorState {
    Away,
    Breath,
    Near,
}

#[derive(Clone, Debug)]
struct Door {
    state: DoorState,
    t: f64,
    pacer: Pacer,
    cool: f64,
    need: f64,
    jitter: f64,
    tempo: f64,
}

/// One frame's inputs for a door hunter.
struct DoorTick<'a> {
    rules: &'a Rules,
    dt_ms: f64,
    speed: f64,
    hunter_speed: f64,
    level: i32,
    restless: bool,
    answered: bool,
    freezes_t: bool,
    need_ms: f64,
    rest_ms: f64,
    breath_ms: f64,
    near_ms: f64,
}

fn stage_jitter(rng: &mut NightWatchStream, spread: f64) -> f64 {
    1.0 - spread / 2.0 + rng.next() * spread
}

impl Door {
    fn new(cool: f64, tempo: f64) -> Self {
        Self { state: DoorState::Away, t: 0.0, pacer: Pacer::default(), cool, need: 0.0, jitter: 1.0, tempo }
    }

    /// True when she reaches the door, which loses the shift.
    fn step(
        &mut self,
        who: Hunter,
        tick: &DoorTick,
        rng: &mut NightWatchStream,
        vr: &mut NightWatchStream,
        ev: &mut Vec<BedtimeEvent>,
    ) -> bool {
        if self.state == DoorState::Away {
            self.cool = (self.cool - tick.dt_ms).max(0.0);
            let mut keen = tick.level;
            if keen > 0 && tick.restless {
                keen = (keen + tick.rules.afk_bonus).min(tick.rules.keen_cap);
            }
            if self.cool <= 0.0 && self.pacer.step(rng, tick.dt_ms * self.tempo * tick.hunter_speed, keen, &tick.rules.wait) {
                self.state = DoorState::Breath;
                self.t = 0.0;
                self.need = 0.0;
                self.jitter = stage_jitter(rng, tick.rules.stage_jitter);
                self.tempo = tempo(vr, tick.rules.var_door_tempo);
                ev.push(BedtimeEvent::Breath(who));
            }
            return false;
        }
        if !tick.freezes_t {
            self.t += tick.dt_ms * tick.speed;
        }
        self.need = if tick.answered { self.need + tick.dt_ms } else { (self.need - tick.dt_ms * tick.rules.need_decay).max(0.0) };
        if self.need >= tick.need_ms {
            self.state = DoorState::Away;
            self.cool = spread_ms(tick.rest_ms, rng, tick.rules.var_wait_spread);
            self.pacer.reset();
            self.need = 0.0;
            ev.push(BedtimeEvent::Leave(who));
        } else if self.state == DoorState::Breath && self.t >= tick.breath_ms * self.jitter {
            self.state = DoorState::Near;
            self.t = 0.0;
            self.jitter = stage_jitter(rng, tick.rules.stage_jitter);
            ev.push(BedtimeEvent::Near(who));
        } else if self.state == DoorState::Near && self.t >= tick.near_ms * self.jitter {
            return true;
        }
        false
    }
}

#[derive(Clone, Debug)]
struct Creep {
    p: f64,
    pacer: Pacer,
    tempo: f64,
}

fn base_level(levels: Option<[i32; 3]>, row: &NightRow, who: Hunter) -> i32 {
    if let Some(levels) = levels {
        let v = levels[who.index()];
        if v >= 0 {
            return v;
        }
    }
    row.ai[who.index()]
}

pub struct Bedtime {
    rules: Rules,
    row: NightRow,
    rng: NightWatchStream,
    vr: NightWatchStream,
    levels: Option<[i32; 3]>,
    infinite_battery: bool,
    flash_per_sec: f64,
    loxelle: Door,
    javelle: Door,
    creep: Creep,
    idle: f64,
    restless: bool,
    /// When each hunter's breathing was last heard, in elapsed ms (the cadence of `breathing_cues`).
    cue_ms: [f64; 3],
    pub elapsed: f64,
    pub battery: f64,
    pub light: Light,
    pub hold: bool,
    pub over: Option<Outcome>,
    /// Admin god mode: whoever would have got in is sent back.
    pub god: bool,
    /// Admin multiplier on the hunters' wait clocks (1.0 is the 2D numbers).
    pub hunter_speed: f64,
}

impl Bedtime {
    pub fn new(opts: BedtimeOptions) -> Self {
        let rules = Rules::load();
        let night_index = (opts.night.clamp(1, rules.nights.len() as u32) - 1) as usize;
        let row = if opts.custom.is_some() { rules.custom_row } else { rules.nights[night_index] };
        let levels = opts.custom;
        let mut rng = NightWatchStream::seeded(opts.seed);
        let mut vr = NightWatchStream::variety_of(&rng, opts.var_rng);

        let swap_roll = chance(&mut vr, rules.var_swap_first);
        let swapped = swap_roll
            && base_level(levels, &row, Hunter::Loxelle) > 0
            && base_level(levels, &row, Hunter::Javelle) > 0;

        let first_of = |who: Hunter| {
            let other = if who == Hunter::Loxelle { Hunter::Javelle } else { Hunter::Loxelle };
            let source = if swapped { other } else { who };
            row.first[source.index()]
        };
        let loxelle_cool = spread_ms(first_of(Hunter::Loxelle), &mut rng, rules.var_wait_spread);
        let loxelle_tempo = tempo(&mut vr, rules.var_door_tempo);
        let javelle_cool = spread_ms(first_of(Hunter::Javelle), &mut rng, rules.var_wait_spread);
        let javelle_tempo = tempo(&mut vr, rules.var_door_tempo);
        let creep_tempo = tempo(&mut vr, rules.var_haelith_tempo);
        let flash_per_sec = rules.battery_full / rules.flash_seconds(night_index);
        let battery = rules.battery_full;

        Self {
            rules,
            row,
            rng,
            vr,
            levels,
            infinite_battery: opts.infinite_battery,
            flash_per_sec,
            loxelle: Door::new(loxelle_cool, loxelle_tempo),
            javelle: Door::new(javelle_cool, javelle_tempo),
            creep: Creep { p: 0.0, pacer: Pacer::default(), tempo: creep_tempo },
            idle: 0.0,
            restless: false,
            cue_ms: [0.0; 3],
            elapsed: 0.0,
            battery,
            light: Light::Off,
            hold: false,
            over: None,
            god: false,
            hunter_speed: 1.0,
        }
    }

    pub fn level(&self, who: Hunter) -> i32 {
        self.rules.effective_level(base_level(self.levels, &self.row, who), self.rules.hour_index(self.elapsed))
    }

    /// The clock hour: 0 at the start (12 AM), `hours` at the win (6 AM).
    pub fn hour(&self) -> u32 {
        let per_hour = self.rules.night_ms / f64::from(self.rules.hours);
        ((self.elapsed / per_hour).floor().max(0.0) as u32).min(self.rules.hours as u32)
    }

    /// Where a hunter is in her approach. Haelith has no door, so she is always away from one.
    pub fn door_state(&self, who: Hunter) -> DoorState {
        match who {
            Hunter::Loxelle => self.loxelle.state,
            Hunter::Javelle => self.javelle.state,
            Hunter::Haelith => DoorState::Away,
        }
    }

    /// The share (0 to 1) of the hold or light a door hunter needs answered before she leaves.
    pub fn answer_share(&self, who: Hunter) -> f64 {
        let share = match who {
            Hunter::Loxelle => self.loxelle.need / self.rules.hold_need_ms,
            Hunter::Javelle => self.javelle.need / self.rules.light_need_ms,
            Hunter::Haelith => 0.0,
        };
        share.clamp(0.0, 1.0)
    }

    /// How far Haelith has got toward the bed (0 to 1). She only moves while the room is dark.
    pub fn creep_share(&self) -> f64 {
        (self.creep.p / self.rules.creep_full).clamp(0.0, 1.0)
    }

    /// The battery left (0 to 1).
    pub fn battery_share(&self) -> f64 {
        (self.battery / self.rules.battery_full).clamp(0.0, 1.0)
    }

    /// True while the house has gone still long enough to make them restless.
    pub fn restless(&self) -> bool {
        self.restless
    }

    /// The Custom Night total, or None for a campaign night.
    pub fn score(&self) -> Option<i32> {
        self.levels.map(custom_score)
    }

    /// The score on the result screen: 10 points per level of the three hunters (the Custom Night sliders, or the
    /// campaign night's own levels). A cleared night keeps all of it; a caught night keeps the share of hours survived.
    pub fn result_score(&self, won: bool) -> i32 {
        let full = self.score().unwrap_or_else(|| {
            let sum: i32 = [Hunter::Loxelle, Hunter::Javelle, Hunter::Haelith]
                .iter()
                .map(|&w| base_level(self.levels, &self.row, w).clamp(0, self.rules.max_ai))
                .sum();
            sum * self.rules.points_per_level
        });
        if won {
            return full;
        }
        full * self.rules.hour_index(self.elapsed) / self.rules.hours.max(1)
    }

    pub fn step(&mut self, dt_ms: f64, input: BedtimeInput) -> Vec<BedtimeEvent> {
        let mut ev = Vec::new();
        if self.over.is_some() {
            return ev;
        }
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        self.hold = input.hold;
        let mut lit = if input.hold { Light::Off } else { input.light };
        if self.battery <= 0.0 {
            lit = Light::Off;
        }
        self.light = lit;
        if lit != Light::Off && !self.infinite_battery {
            self.battery = (self.battery - self.flash_per_sec * dt).max(0.0);
        }

        self.idle = if input.hold || lit != Light::Off { 0.0 } else { self.idle + dt_ms };
        let restless = self.idle >= self.rules.afk_ms && (self.level(Hunter::Loxelle) > 0 || self.level(Hunter::Javelle) > 0);
        if restless && !self.restless {
            ev.push(BedtimeEvent::Restless);
        }
        self.restless = restless;
        let speed = if restless { self.rules.afk_speed } else { 1.0 };

        let start = ev.len();
        for who in [Hunter::Loxelle, Hunter::Javelle] {
            let tick = DoorTick {
                rules: &self.rules,
                dt_ms,
                speed,
                hunter_speed: self.hunter_speed,
                level: self.level(who),
                restless,
                answered: if who == Hunter::Loxelle { input.hold } else { lit == Light::Hall },
                freezes_t: who == Hunter::Javelle && lit == Light::Hall,
                need_ms: if who == Hunter::Loxelle { self.rules.hold_need_ms } else { self.rules.light_need_ms },
                rest_ms: self.row.rest[who.index()],
                breath_ms: self.row.breath_ms,
                near_ms: self.row.near_ms,
            };
            let door = if who == Hunter::Loxelle { &mut self.loxelle } else { &mut self.javelle };
            if door.step(who, &tick, &mut self.rng, &mut self.vr, &mut ev) {
                self.finish(who, &mut ev);
                return ev;
            }
        }

        for e in &ev[start..] {
            if let BedtimeEvent::Breath(who) | BedtimeEvent::Near(who) = *e {
                self.cue_ms[who.index()] = self.elapsed;
            }
        }

        let creep_level = self.level(Hunter::Haelith);
        if lit == Light::Off
            && self.creep.pacer.step(&mut self.rng, dt_ms * self.creep.tempo * self.hunter_speed, creep_level, &self.rules.wait)
        {
            self.creep.p = (self.creep.p + self.rules.bed_gain + self.rules.bed_gain_per_level * f64::from(creep_level)).min(self.rules.creep_full);
            self.creep.tempo = tempo(&mut self.vr, self.rules.var_haelith_tempo);
            ev.push(BedtimeEvent::Stir { p: self.creep.p });
        }
        if lit == Light::Bed {
            self.creep.p = (self.creep.p - self.rules.bed_lit_drain_per_sec * dt).max(0.0);
        }
        self.breathing_cues(&mut ev);
        if self.creep.p >= self.rules.creep_full {
            self.finish(Hunter::Haelith, &mut ev);
            return ev;
        }

        if self.elapsed >= self.rules.night_ms {
            self.over = Some(Outcome::Win);
            ev.push(BedtimeEvent::Win);
        }
        ev
    }

    /// The breathing a player listens for: a door hunter breathes again every so often while she waits at her door
    /// (faster when she is near), and Haelith exhales at the foot of the bed once she has crept far enough.
    fn breathing_cues(&mut self, ev: &mut Vec<BedtimeEvent>) {
        let cue = self.rules.cue;
        for who in [Hunter::Loxelle, Hunter::Javelle] {
            let door = if who == Hunter::Loxelle { &self.loxelle } else { &self.javelle };
            if door.state == DoorState::Away {
                continue;
            }
            let gap = if door.state == DoorState::Near { cue.near_ms } else { cue.far_ms };
            if self.elapsed - self.cue_ms[who.index()] > gap {
                self.cue_ms[who.index()] = self.elapsed;
                ev.push(BedtimeEvent::Breathing(who));
            }
        }
        let haelith = Hunter::Haelith.index();
        if self.creep.p >= self.rules.bed_stir_at && self.elapsed - self.cue_ms[haelith] > cue.haelith_ms {
            self.cue_ms[haelith] = self.elapsed;
            ev.push(BedtimeEvent::Breathing(Hunter::Haelith));
        }
    }

    fn finish(&mut self, who: Hunter, ev: &mut Vec<BedtimeEvent>) {
        if self.god {
            match who {
                Hunter::Haelith => self.creep.p = 0.0,
                _ => {
                    let rest = self.row.rest[who.index()];
                    let door = if who == Hunter::Loxelle { &mut self.loxelle } else { &mut self.javelle };
                    door.state = DoorState::Away;
                    door.t = 0.0;
                    door.need = 0.0;
                    door.cool = rest;
                    door.pacer.reset();
                }
            }
            return;
        }
        self.over = Some(Outcome::Caught(who));
        ev.push(BedtimeEvent::Caught(who));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hunter_at_her_door_breathes_again_every_so_often() {
        let opts = BedtimeOptions {
            var_rng: Some(NightWatchStream::neutral()),
            ..BedtimeOptions::campaign(1, 0)
        };
        let mut game = Bedtime::new(opts);
        game.loxelle.state = DoorState::Breath;
        let events = game.step(1800.0, BedtimeInput::default());
        assert!(events.contains(&BedtimeEvent::Breathing(Hunter::Loxelle)), "{events:?}");
        let quiet = game.step(800.0, BedtimeInput::default());
        assert!(!quiet.contains(&BedtimeEvent::Breathing(Hunter::Loxelle)), "{quiet:?}");
    }

    #[test]
    fn haelith_exhales_only_once_she_is_close_enough_to_the_bed() {
        let opts = BedtimeOptions {
            var_rng: Some(NightWatchStream::neutral()),
            ..BedtimeOptions::campaign(1, 0)
        };
        let mut game = Bedtime::new(opts);
        game.creep.p = 10.0;
        game.elapsed = 3000.0;
        let far = game.step(10.0, BedtimeInput::default());
        assert!(!far.contains(&BedtimeEvent::Breathing(Hunter::Haelith)), "{far:?}");
        game.creep.p = 50.0;
        let close = game.step(10.0, BedtimeInput::default());
        assert!(close.contains(&BedtimeEvent::Breathing(Hunter::Haelith)), "{close:?}");
    }

    #[test]
    fn holding_the_closet_door_shut_sends_loxelle_away() {
        let opts = BedtimeOptions {
            var_rng: Some(NightWatchStream::neutral()),
            ..BedtimeOptions::campaign(1, 0)
        };
        let mut game = Bedtime::new(opts);
        game.loxelle.state = DoorState::Near;
        let events = game.step(800.0, BedtimeInput { hold: true, light: Light::Off });
        assert!(events.contains(&BedtimeEvent::Leave(Hunter::Loxelle)));
        assert_eq!(game.loxelle.state, DoorState::Away);
        assert_eq!(game.over, None);
    }

    #[test]
    fn the_rules_file_carries_every_key_the_mode_reads() {
        let r = Rules::load();
        assert_eq!(r.nights.len(), 6);
        assert_eq!(r.flash_seconds.len(), 6);
        assert_eq!(r.ratings.len(), 6);
        assert_eq!(r.level_bumps.len(), 2);
    }

    #[test]
    fn result_score_is_ten_points_per_level_and_a_caught_night_keeps_the_hours_survived() {
        let g = Bedtime::new(BedtimeOptions::campaign(1, 3));
        assert_eq!(g.result_score(true), 30, "night 1 is levels 3, 0, 0");
        assert_eq!(g.result_score(false), 0, "caught in the first hour keeps nothing");
        let mut late = Bedtime::new(BedtimeOptions::campaign(4, 3));
        late.elapsed = Rules::load().night_ms * 0.5;
        assert_eq!(late.result_score(true), 340);
        assert_eq!(late.result_score(false), 340 * 3 / 6);
        let custom = Bedtime::new(BedtimeOptions::custom([20, 20, 20], 1));
        assert_eq!(custom.result_score(true), custom_max_score());
        assert_eq!(rating_of(custom.result_score(true), custom_max_score()), "Unbeatable!");
    }

    #[test]
    fn loaded_values_equal_the_old_constants() {
        let r = Rules::load();
        assert_eq!(r.night_ms, 270_000.0);
        assert!((r.time_scale - 270_000.0 / 480_000.0).abs() < 1e-12);
        assert_eq!(r.flash_seconds(0), (120.0_f64 * 270_000.0 / 480_000.0).round());
        assert_eq!((r.hold_need_ms, r.light_need_ms, r.afk_ms, r.afk_bonus, r.keen_cap), (700.0, 600.0, 14_000.0, 4, 20));
        assert_eq!((r.bed_gain, r.bed_gain_per_level, r.bed_lit_drain_per_sec), (8.0, 0.9, 35.0));
        assert_eq!(r.nights[3].ai, [16, 11, 7]);
        assert_eq!(r.nights[5].rest, [1500.0, 5000.0, 0.0]);
        assert_eq!(r.nights[1].first, [3000.0, 4500.0, 0.0]);
        assert_eq!(r.custom_row.near_ms, 4200.0);
        assert_eq!(r.bed_stir_at, 45.0);
        assert_eq!((r.cue.near_ms, r.cue.far_ms, r.cue.haelith_ms), (1100.0, 1700.0, 2400.0));
        assert_eq!(r.effective_level(10, 2), 10);
        assert_eq!(r.effective_level(10, 3), 11);
        assert_eq!(r.effective_level(10, 5), 12);
        assert_eq!(r.effective_level(0, 5), 0);
        assert_eq!(custom_max_score(), 600);
        assert_eq!(custom_score([20, 25, -3]), 400);
        assert_eq!(BedtimeOptions::custom([1, 2, 3], 0).night, 5);
        assert_eq!(rating_of(600, 600), "Unbeatable!");
        assert_eq!(rating_of(0, 600), "Great Job!");
        assert_eq!(rating_of(250, 600), "Amazing!");
    }
}
