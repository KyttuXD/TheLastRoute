//! Bedtime night type (Godot `game/night_watch/bedroom`, rules from `bedroom_logic.gd`). Pure rules and scoring: the
//! pillow view, the closet, hall and bed set pieces, the look controls and the breathing cues belong to the shell.
//!
//! You are in bed with one free hand. Three things come for you, each from its own side:
//!   Loxelle  breathes behind the closet door. Hold the closet door shut until she leaves.
//!   Javelle  waits in the hall doorway. A light on the hall makes her leave.
//!   Haelith  creeps toward the bed, only while the room is dark. Light on the bed pushes her back.
//! Holding the door uses both hands, so no light while holding, and only one light at a time. The battery never
//! recharges. Sitting still for a while makes the house restless.

use crate::framework::{chance, spread_ms, tempo, NightWatchStream};

/// The classic rules' night length and tuned night (`night_ms`, `tuned_night_ms`). Shared with the other shifts.
const NIGHT_MS: f64 = 270_000.0;
const TUNED_NIGHT_MS: f64 = 480_000.0;
const HOUR_MS: f64 = NIGHT_MS / 6.0;
const TIME_SCALE: f64 = NIGHT_MS / TUNED_NIGHT_MS;
const FLASH_SECONDS: [f64; 6] = [120.0, 105.0, 90.0, 75.0, 60.0, 50.0];

const MAX_AI: i32 = 20;
const POINTS_PER_LEVEL: i32 = 10;
/// The most a hunter can be pushed to on a Custom Night: the three sliders at full.
pub const CUSTOM_MAX_SCORE: i32 = 3 * MAX_AI * POINTS_PER_LEVEL;
/// [minimum share of the top score, rating], best first.
const RATINGS: [(f64, &str); 6] = [
    (1.0, "Unbeatable!"),
    (0.8, "Perfect!"),
    (0.6, "Stupendous!"),
    (0.4, "Amazing!"),
    (0.2, "Fantastic!"),
    (0.0, "Great Job!"),
];

const VAR_WAIT_SPREAD: f64 = 0.25;
const VAR_DOOR_TEMPO: f64 = 0.1;
const VAR_HAELITH_TEMPO: f64 = 0.2;
const VAR_SWAP_FIRST: f64 = 0.3;

const STAGE_JITTER: f64 = 0.4;
const HOLD_NEED_MS: f64 = 700.0;
const LIGHT_NEED_MS: f64 = 600.0;
const AFK_MS: f64 = 14_000.0;
const AFK_BONUS: i32 = 4;
const AFK_SPEED: f64 = 1.5;
const KEEN_CAP: i32 = 20;
const BED_GAIN: f64 = 8.0;
const BED_GAIN_PER_LEVEL: f64 = 0.9;
const BED_LIT_DRAIN_PER_SEC: f64 = 35.0;

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

/// One night's pace. Doors use `first` and `rest` (Haelith has no door timing, so her slots are 0).
#[derive(Clone, Copy, Debug)]
struct NightRow {
    ai: [i32; 3],
    breath_ms: f64,
    near_ms: f64,
    first: [f64; 3],
    rest: [f64; 3],
}

const fn row(ai: [i32; 3], breath_ms: f64, near_ms: f64, javelle_first: f64, loxelle_rest: f64, javelle_rest: f64) -> NightRow {
    NightRow {
        ai,
        breath_ms,
        near_ms,
        first: [3000.0, javelle_first, 0.0],
        rest: [loxelle_rest, javelle_rest, 0.0],
    }
}

const NIGHTS: [NightRow; 6] = [
    row([3, 0, 0], 3200.0, 4200.0, 3000.0, 7000.0, 7000.0),
    row([7, 7, 0], 2560.0, 3360.0, 4500.0, 5000.0, 5000.0),
    row([9, 9, 5], 2560.0, 3360.0, 4000.0, 5000.0, 5000.0),
    row([16, 11, 7], 2560.0, 3360.0, 3000.0, 2500.0, 5000.0),
    row([18, 12, 9], 2560.0, 3360.0, 3000.0, 2000.0, 5000.0),
    row([20, 12, 9], 2560.0, 3360.0, 3000.0, 1500.0, 5000.0),
];

/// Custom Night keeps Night 1's timings.
const CUSTOM_ROW: NightRow = row([0, 0, 0], 3200.0, 4200.0, 3000.0, 7000.0, 7000.0);

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
        Self { night: 5, seed, custom: Some(sliders), infinite_battery: false, var_rng: None }
    }
}

/// Points for a Custom Night's sliders. A campaign night scores nothing here; the clear is saved by the shell.
pub fn custom_score(sliders: [i32; 3]) -> i32 {
    sliders.into_iter().map(|s| s.clamp(0, MAX_AI)).sum::<i32>() * POINTS_PER_LEVEL
}

pub fn rating_of(score: i32, max_score: i32) -> &'static str {
    let share = if max_score > 0 { f64::from(score) / f64::from(max_score) } else { 0.0 };
    RATINGS.iter().find(|(min, _)| share >= *min).map_or("Great Job!", |(_, name)| *name)
}

/// Wait between moves: a random time from a three-second window that slides down with the hunter's level.
#[derive(Clone, Debug, Default)]
struct Pacer {
    t: f64,
    next: Option<f64>,
}

fn draw_wait(rng: &mut NightWatchStream, level: i32) -> f64 {
    let lo = (13_000.0 - f64::from(level) * 500.0).max(2500.0);
    lo + rng.next() * 3000.0
}

impl Pacer {
    /// Advances by `dt_ms`; true when the hunter is due to move.
    fn step(&mut self, rng: &mut NightWatchStream, dt_ms: f64, level: i32) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        let wait = self.next.unwrap_or_else(|| draw_wait(rng, level));
        self.t += dt_ms;
        if self.t < wait {
            self.next = Some(wait);
            return false;
        }
        self.t = 0.0;
        self.next = Some(draw_wait(rng, level));
        true
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.next = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DoorState {
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
struct DoorTick {
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

fn stage_jitter(rng: &mut NightWatchStream) -> f64 {
    1.0 - STAGE_JITTER / 2.0 + rng.next() * STAGE_JITTER
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
                keen = (keen + AFK_BONUS).min(KEEN_CAP);
            }
            if self.cool <= 0.0 && self.pacer.step(rng, tick.dt_ms * self.tempo * tick.hunter_speed, keen) {
                self.state = DoorState::Breath;
                self.t = 0.0;
                self.need = 0.0;
                self.jitter = stage_jitter(rng);
                self.tempo = tempo(vr, VAR_DOOR_TEMPO);
                ev.push(BedtimeEvent::Breath(who));
            }
            return false;
        }
        if !tick.freezes_t {
            self.t += tick.dt_ms * tick.speed;
        }
        self.need = if tick.answered { self.need + tick.dt_ms } else { (self.need - tick.dt_ms * 0.5).max(0.0) };
        if self.need >= tick.need_ms {
            self.state = DoorState::Away;
            self.cool = spread_ms(tick.rest_ms, rng, VAR_WAIT_SPREAD);
            self.pacer.reset();
            self.need = 0.0;
            ev.push(BedtimeEvent::Leave(who));
        } else if self.state == DoorState::Breath && self.t >= tick.breath_ms * self.jitter {
            self.state = DoorState::Near;
            self.t = 0.0;
            self.jitter = stage_jitter(rng);
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

fn hour_index(elapsed_ms: f64) -> i32 {
    ((elapsed_ms / HOUR_MS).floor() as i32).clamp(0, 5)
}

/// The hunters step up at 3 AM and 4 AM. A hunter at level 0 never wakes.
fn effective_level(base: i32, hour: i32) -> i32 {
    if base <= 0 {
        return 0;
    }
    let bump = if hour >= 4 {
        2
    } else if hour >= 3 {
        1
    } else {
        0
    };
    base + bump
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

fn flash_seconds(night_index: usize) -> f64 {
    (FLASH_SECONDS[night_index] * TIME_SCALE).round()
}

pub struct Bedtime {
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
        let night_index = (opts.night.clamp(1, 6) - 1) as usize;
        let row = if opts.custom.is_some() { CUSTOM_ROW } else { NIGHTS[night_index] };
        let levels = opts.custom;
        let mut rng = NightWatchStream::seeded(opts.seed);
        let mut vr = NightWatchStream::variety_of(&rng, opts.var_rng);

        let swap_roll = chance(&mut vr, VAR_SWAP_FIRST);
        let swapped = swap_roll
            && base_level(levels, &row, Hunter::Loxelle) > 0
            && base_level(levels, &row, Hunter::Javelle) > 0;

        let first_of = |who: Hunter| {
            let other = if who == Hunter::Loxelle { Hunter::Javelle } else { Hunter::Loxelle };
            let source = if swapped { other } else { who };
            row.first[source.index()]
        };
        let loxelle_cool = spread_ms(first_of(Hunter::Loxelle), &mut rng, VAR_WAIT_SPREAD);
        let loxelle_tempo = tempo(&mut vr, VAR_DOOR_TEMPO);
        let javelle_cool = spread_ms(first_of(Hunter::Javelle), &mut rng, VAR_WAIT_SPREAD);
        let javelle_tempo = tempo(&mut vr, VAR_DOOR_TEMPO);
        let creep_tempo = tempo(&mut vr, VAR_HAELITH_TEMPO);

        Self {
            row,
            rng,
            vr,
            levels,
            infinite_battery: opts.infinite_battery,
            flash_per_sec: 100.0 / flash_seconds(night_index),
            loxelle: Door::new(loxelle_cool, loxelle_tempo),
            javelle: Door::new(javelle_cool, javelle_tempo),
            creep: Creep { p: 0.0, pacer: Pacer::default(), tempo: creep_tempo },
            idle: 0.0,
            restless: false,
            elapsed: 0.0,
            battery: 100.0,
            light: Light::Off,
            hold: false,
            over: None,
            god: false,
            hunter_speed: 1.0,
        }
    }

    pub fn level(&self, who: Hunter) -> i32 {
        effective_level(base_level(self.levels, &self.row, who), hour_index(self.elapsed))
    }

    /// The Custom Night total, or None for a campaign night.
    pub fn score(&self) -> Option<i32> {
        self.levels.map(custom_score)
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
        let restless = self.idle >= AFK_MS && (self.level(Hunter::Loxelle) > 0 || self.level(Hunter::Javelle) > 0);
        if restless && !self.restless {
            ev.push(BedtimeEvent::Restless);
        }
        self.restless = restless;
        let speed = if restless { AFK_SPEED } else { 1.0 };

        for who in [Hunter::Loxelle, Hunter::Javelle] {
            let tick = DoorTick {
                dt_ms,
                speed,
                hunter_speed: self.hunter_speed,
                level: self.level(who),
                restless,
                answered: if who == Hunter::Loxelle { input.hold } else { lit == Light::Hall },
                freezes_t: who == Hunter::Javelle && lit == Light::Hall,
                need_ms: if who == Hunter::Loxelle { HOLD_NEED_MS } else { LIGHT_NEED_MS },
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

        let creep_level = self.level(Hunter::Haelith);
        if lit == Light::Off
            && self.creep.pacer.step(&mut self.rng, dt_ms * self.creep.tempo * self.hunter_speed, creep_level)
        {
            self.creep.p = (self.creep.p + BED_GAIN + BED_GAIN_PER_LEVEL * f64::from(creep_level)).min(100.0);
            self.creep.tempo = tempo(&mut self.vr, VAR_HAELITH_TEMPO);
            ev.push(BedtimeEvent::Stir { p: self.creep.p });
        }
        if lit == Light::Bed {
            self.creep.p = (self.creep.p - BED_LIT_DRAIN_PER_SEC * dt).max(0.0);
        }
        if self.creep.p >= 100.0 {
            self.finish(Hunter::Haelith, &mut ev);
            return ev;
        }

        if self.elapsed >= NIGHT_MS {
            self.over = Some(Outcome::Win);
            ev.push(BedtimeEvent::Win);
        }
        ev
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
}
