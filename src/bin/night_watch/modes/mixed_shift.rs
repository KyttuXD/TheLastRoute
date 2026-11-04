//! Mixed Shift (Godot `game/night_watch/mixed/mixed_logic.gd`, numbers from `mixed_data.gd`): each night draws 3 to 7
//! threats from a pool of eight. Every threat gives a tell before it can kill and has an answer, and a director paces
//! the beats. This file holds the rules only. The shell draws the building and plays the `Event`s that `step` returns.
//! Draws come from the shared `NightWatchStream` in the same order as the GDScript, so a seed plays the same night.

use crate::data::{self, int, list, num, text};
use crate::framework::{jitter, NightWatchStream};
use serde_json::Value;
use std::sync::OnceLock;

/// Every number the mode tunes, read once from `data/rules/mixed_shift.json`, `cast/mixed_shift.json` and
/// `buildings/mixed_shift.json` (the Godot `mixed_data.gd` constants). The logic below only reads this struct.
struct Tuning {
    hour_ms: f64,
    night_ms: f64,
    points_per_level: u32,
    dawn_hour: u32,

    tell_floor_ms: f64,
    tell_jitter: f64,
    jitter: f64,
    cooldown_ms: f64,
    give_up_ms: f64,
    choke_ms: f64,
    end_clear_ms: f64,
    min_dwell_ms: f64,
    max_walkers: usize,
    feint_quiet_ms: f64,
    feint_retry_ms: f64,
    idle_scare_ms: f64,

    seal_max: f64,
    seal_regen: f64,
    seal_min_to_shut: f64,
    seal_reserve: f64,
    door_hold_ms: f64,
    pop_cool_ms: f64,

    vent_seal_max: f64,
    vent_seal_regen: f64,
    vent_min_to_seal: f64,
    vent_reserve: f64,
    vent_hold_ms: f64,
    vent_linger_ms: f64,
    vent_pop_cool_ms: f64,
    vent_light_ms: f64,
    vent_light_cool_ms: f64,

    mood_ms: (f64, f64),
    mood_cue_ms: (f64, f64),
    mood_speed: [f64; 4],
    mood_tell: [f64; 4],

    spike_len_ms: (f64, f64),
    spike_warn_ms: f64,
    spike_start_pad_ms: f64,
    spike_end_pad_ms: f64,
    spike_slot_pad_ms: f64,
    breather_ms: (f64, f64),
    beat_rate: [f64; 4],
    beat_lull_ms: (f64, f64),
    beat_build_ms: (f64, f64),
    feint_lull_ms: (f64, f64),
    feint_build_ms: (f64, f64),
    tension_start: f64,
    tension_speed_base: f64,
    tension_speed_scale: f64,
    tension_floor_from: f64,
    tension_floor: (f64, f64),
    build_again_above: f64,
    build_again_p: f64,
    build_again_p_last: f64,
    scare_arm: f64,
    scare_away: f64,
    scare_turn: f64,
    scare_idle_per_min: f64,

    side_decay: f64,
    side_floor: f64,
    mem_cap: u32,

    walker_step_late_ms: f64,
    step_ms: [f64; 6],
    rest_ms: [f64; 6],
    counter_grace_ms: f64,
    counter_span_ms: f64,
    counter_arm_at: f64,
    counter_calm_decay: f64,
    monitor_arrive_ms: f64,
    monitor_idle_leave_ms: f64,
    monitor_steps_to_arm: u32,
    monitor_decay_ms: f64,
    monitor_step_late_ms: f64,
    cranky_ms: f64,
    tune_span_ms: f64,
    tune_wind_per_sec: f64,
    tune_restart_at: f64,
    tune_grace_floor: f64,

    chaos_hours: (u32, u32),
    chaos_order: Vec<ChaosKind>,
    chaos_weight: [f64; 6],
    chaos_needs: [Option<Kind>; 6],
    decoy_quiet_ms: f64,
    decoy_kinds: Vec<Kind>,
    wake_late_ms: f64,
    chaos_pad_start_ms: f64,
    chaos_pad_end_ms: f64,
    strain_ms: f64,
    strain_hold_ms: f64,
    strain_regen: f64,
    flicker_ms: f64,
    boxfast_ms: f64,
    boxfast_factor: f64,

    endless_seal_max: f64,
    endless_seal_regen: f64,
    endless_budget_ms: f64,
    rotate_ms: f64,
    ramp_roster: Vec<(usize, usize)>,
    ramp_event_from: u32,
    ramp_event_p: f64,
    ramp_floor_from: f64,
    ramp_floor_max: f64,
    ramp_floor_span: f64,
    pace_from: u32,
    pace_step: f64,
    pace_cap: f64,
    tell_from: u32,
    tell_step: f64,
    tell_min: f64,
    tell_base: f64,
    gap_start: f64,
    gap_step: f64,
    gap_min: f64,
    gap_jitter: f64,
    budget_from: u32,
    budget_step: f64,
    budget_min: f64,
    first_spike_span_ms: f64,

    pool: Vec<PoolEntry>,
    shifts: Vec<Difficulty>,
    routes: Vec<Route>,
    vent_routes: Vec<Route>,
    cam_rooms: Vec<Room>,
    box_rooms: Vec<Room>,
    vent_near: [u8; 3],
}

const KIND_KEYS: [&str; 6] = ["counter", "watched", "monitor", "path", "vent", "winder"];
const KINDS: [Kind; 6] = [Kind::Counter, Kind::Watched, Kind::Monitor, Kind::Path, Kind::Vent, Kind::Winder];
const MOOD_KEYS: [&str; 4] = ["patient", "hungry", "shy", "reckless"];
const BEAT_KEYS: [&str; 4] = ["lull", "build", "spike", "breather"];
const CHAOS_KEYS: [&str; 6] = ["mood", "wake", "strain", "flicker", "decoy", "boxfast"];
const CHAOS_KINDS: [ChaosKind; 6] =
    [ChaosKind::Mood, ChaosKind::Wake, ChaosKind::Strain, ChaosKind::Flicker, ChaosKind::Decoy, ChaosKind::Boxfast];
const ROOM_KEYS: [(&str, Room); 14] = [
    ("stage", Room::Stage),
    ("dining", Room::Dining),
    ("backstage", Room::Backstage),
    ("westhall", Room::WestHall),
    ("closet", Room::Closet),
    ("kitchen", Room::Kitchen),
    ("easthall", Room::EastHall),
    ("corner", Room::Corner),
    ("vent", Room::Vent),
    ("doorL", Room::DoorL),
    ("doorR", Room::DoorR),
    ("ventL", Room::VentL),
    ("ventR", Room::VentR),
    ("ventT", Room::VentT),
];

fn tn() -> &'static Tuning {
    static TUNING: OnceLock<Tuning> = OnceLock::new();
    TUNING.get_or_init(Tuning::load)
}

fn pair(v: &Value, path: &str) -> (f64, f64) {
    (num(v, &format!("{path}.0")), num(v, &format!("{path}.1")))
}

fn key_ix(keys: &[&str], key: &str, what: &str) -> usize {
    keys.iter().position(|k| *k == key).unwrap_or_else(|| panic!("night_watch mixed_shift data: unknown {what} '{key}'"))
}

fn kind_named(key: &str) -> Kind {
    KINDS[key_ix(&KIND_KEYS, key, "threat type")]
}

fn room_named(key: &str) -> Room {
    ROOM_KEYS[key_ix(&ROOM_KEYS.map(|(k, _)| k), key, "room")].1
}

fn side_named(key: &str) -> Side {
    match key {
        "L" => Side::L,
        "R" => Side::R,
        "T" => Side::T,
        _ => panic!("night_watch mixed_shift data: unknown side '{key}'"),
    }
}

fn rooms_of(ids: &[Value]) -> Vec<Room> {
    ids.iter().map(|r| room_named(r.as_str().unwrap_or("?"))).collect()
}

fn routes_of(b: &Value, path: &str) -> Vec<Route> {
    list(b, path)
        .iter()
        .map(|e| Route {
            side: side_named(text(e, "side")),
            rooms: rooms_of(list(e, "rooms")),
            spots: e.get("spots").and_then(Value::as_array).map_or_else(Vec::new, |s| s.iter().filter_map(|x| x.as_u64().map(|x| x as u8)).collect()),
        })
        .collect()
}

fn opt_num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

impl Tuning {
    fn load() -> Self {
        let r = data::rules("mixed_shift");
        let cast = data::cast("mixed_shift");
        let b = data::building("mixed_shift");
        let n = |p: &str| num(r, p);
        let hour_ms = n("hour_ms");
        let by_key = |keys: &[&str], prefix: &str, suffix: &str| -> Vec<f64> {
            keys.iter().map(|k| n(&format!("{prefix}.{k}{suffix}"))).collect()
        };
        let step_of = |k: &str| n(&format!("timing.{k}.step_ms"));
        let rest_of = |k: &str| n(&format!("timing.{k}.rest_ms"));

        let pool: Vec<PoolEntry> = list(cast, "pool")
            .iter()
            .map(|e| PoolEntry { id: text(e, "id").to_string(), name: text(e, "name").to_string(), kind: kind_named(text(e, "type")) })
            .collect();
        let shifts: Vec<Difficulty> = list(r, "shifts")
            .iter()
            .map(|s| {
                let (roster_lo, roster_hi) = pair(s, "roster");
                let (spikes_lo, spikes_hi) = pair(s, "spikes");
                Difficulty {
                    name: text(s, "name").to_string(),
                    speed: num(s, "speed"),
                    tell_ms: num(s, "tell_ms"),
                    min_tell_ms: opt_num(s, "min_tell_ms").unwrap_or(0.0),
                    start_ms: num(s, "start_ms"),
                    roster: (roster_lo as usize, roster_hi as usize),
                    spikes: (spikes_lo as usize, spikes_hi as usize),
                    grace_ms: num(s, "grace_ms"),
                    new_max: num(s, "new_max") as usize,
                    min_types: num(s, "min_types") as usize,
                    must: s.get("must").and_then(Value::as_array).map_or_else(Vec::new, |ids| {
                        ids.iter()
                            .map(|id| {
                                let id = id.as_str().unwrap_or("?");
                                pool.iter().position(|p| p.id == id).unwrap_or_else(|| panic!("night_watch mixed_shift data: unknown pool id '{id}'"))
                            })
                            .collect()
                    }),
                    breather: s.get("breather_ms").map(|_| pair(s, "breather_ms")),
                    chaos: s.get("chaos").and_then(Value::as_bool).unwrap_or(false),
                    endless: s.get("endless").and_then(Value::as_bool).unwrap_or(false),
                }
            })
            .collect();
        let tell_base = shifts.iter().find(|d| d.endless).map_or(0.0, |d| d.tell_ms);
        let weights = by_key(&CHAOS_KEYS, "chaos.events", ".weight");
        let (chaos_lo, chaos_hi) = pair(r, "chaos.hours");
        let needs_of = |k: &str| match text(r, &format!("chaos.events.{k}.needs")) {
            "" => None,
            kind => Some(kind_named(kind)),
        };
        let mood_speed = by_key(&MOOD_KEYS, "moods.speed", "");
        let mood_tell = by_key(&MOOD_KEYS, "moods.tell", "");
        let beat_rate = by_key(&BEAT_KEYS, "director.beat_rate", "");
        let mut step_ms = [0.0; 6];
        let mut rest_ms = [0.0; 6];
        for k in [Kind::Watched, Kind::Path, Kind::Vent, Kind::Winder] {
            step_ms[k.ix()] = step_of(KIND_KEYS[k.ix()]);
        }
        for k in [Kind::Watched, Kind::Monitor, Kind::Path, Kind::Vent, Kind::Winder] {
            rest_ms[k.ix()] = rest_of(KIND_KEYS[k.ix()]);
        }
        Self {
            hour_ms,
            night_ms: n("night_ms"),
            points_per_level: int(r, "points_per_level") as u32,
            dawn_hour: int(r, "dawn_hour") as u32,

            tell_floor_ms: n("fairness.tell_floor_ms"),
            tell_jitter: n("fairness.tell_jitter"),
            jitter: n("fairness.jitter"),
            cooldown_ms: n("fairness.cooldown_ms"),
            give_up_ms: n("fairness.give_up_ms"),
            choke_ms: n("fairness.choke_ms"),
            end_clear_ms: n("fairness.end_clear_ms"),
            min_dwell_ms: n("fairness.min_dwell_ms"),
            max_walkers: int(r, "fairness.max_walkers") as usize,
            feint_quiet_ms: n("fairness.feint_quiet_ms"),
            feint_retry_ms: n("fairness.feint_retry_ms"),
            idle_scare_ms: n("fairness.idle_scare_ms"),

            seal_max: n("doors.seal_max"),
            seal_regen: n("doors.seal_regen"),
            seal_min_to_shut: n("doors.seal_min_to_shut"),
            seal_reserve: n("doors.seal_reserve"),
            door_hold_ms: n("doors.hold_ms"),
            pop_cool_ms: n("doors.pop_cool_ms"),

            vent_seal_max: n("vents.seal_max"),
            vent_seal_regen: n("vents.seal_regen"),
            vent_min_to_seal: n("vents.min_to_seal"),
            vent_reserve: n("vents.reserve"),
            vent_hold_ms: n("vents.hold_ms"),
            vent_linger_ms: n("vents.linger_ms"),
            vent_pop_cool_ms: n("vents.pop_cool_ms"),
            vent_light_ms: n("vents.light_ms"),
            vent_light_cool_ms: n("vents.light_cool_ms"),

            mood_ms: pair(r, "moods.duration_ms"),
            mood_cue_ms: pair(r, "moods.cue_ms"),
            mood_speed: mood_speed.try_into().expect("four moods"),
            mood_tell: mood_tell.try_into().expect("four moods"),

            spike_len_ms: pair(r, "director.spike_len_ms"),
            spike_warn_ms: n("director.spike_warn_ms"),
            spike_start_pad_ms: n("director.spike_start_pad_ms"),
            spike_end_pad_ms: n("director.spike_end_pad_ms"),
            spike_slot_pad_ms: n("director.spike_slot_pad_ms"),
            breather_ms: pair(r, "director.breather_ms"),
            beat_rate: beat_rate.try_into().expect("four beats"),
            beat_lull_ms: pair(r, "director.beat_len_ms.lull"),
            beat_build_ms: pair(r, "director.beat_len_ms.build"),
            feint_lull_ms: pair(r, "director.feint_ms.lull"),
            feint_build_ms: pair(r, "director.feint_ms.build"),
            tension_start: n("director.tension_start"),
            tension_speed_base: n("director.tension_speed_base"),
            tension_speed_scale: n("director.tension_speed_scale"),
            tension_floor_from: n("director.tension_floor_from_hour") * hour_ms,
            tension_floor: pair(r, "director.tension_floor"),
            build_again_above: n("director.build_again_above"),
            build_again_p: n("director.build_again_p"),
            build_again_p_last: n("director.build_again_p_last"),
            scare_arm: n("director.scare.arm"),
            scare_away: n("director.scare.away"),
            scare_turn: n("director.scare.turn"),
            scare_idle_per_min: n("director.scare.idle_per_min"),

            side_decay: n("routing.side_decay"),
            side_floor: n("routing.side_floor"),
            mem_cap: int(r, "routing.mem_cap") as u32,

            walker_step_late_ms: n("timing.walker_step_late_ms"),
            step_ms,
            rest_ms,
            counter_grace_ms: n("timing.counter.grace_ms"),
            counter_span_ms: n("timing.counter.span_ms"),
            counter_arm_at: n("timing.counter.arm_at"),
            counter_calm_decay: n("timing.counter.calm_decay"),
            monitor_arrive_ms: n("timing.monitor.arrive_ms"),
            monitor_idle_leave_ms: n("timing.monitor.idle_leave_ms"),
            monitor_steps_to_arm: int(r, "timing.monitor.steps_to_arm") as u32,
            monitor_decay_ms: n("timing.monitor.decay_ms"),
            monitor_step_late_ms: n("timing.monitor.step_late_ms"),
            cranky_ms: n("timing.winder.cranky_ms"),
            tune_span_ms: n("tune.span_ms"),
            tune_wind_per_sec: n("tune.wind_per_sec"),
            tune_restart_at: n("tune.restart_at"),
            tune_grace_floor: n("tune.grace_floor"),

            chaos_hours: (chaos_lo as u32, chaos_hi as u32),
            chaos_order: list(r, "chaos.order").iter().map(|k| CHAOS_KINDS[key_ix(&CHAOS_KEYS, k.as_str().unwrap_or("?"), "chaos event")]).collect(),
            chaos_weight: weights.try_into().expect("six chaos events"),
            chaos_needs: CHAOS_KEYS.map(needs_of),
            decoy_quiet_ms: n("chaos.decoy_quiet_ms"),
            decoy_kinds: list(r, "chaos.decoy_kinds").iter().map(|k| kind_named(k.as_str().unwrap_or("?"))).collect(),
            wake_late_ms: n("chaos.wake_late_ms"),
            chaos_pad_start_ms: n("chaos.at_pad_start_ms"),
            chaos_pad_end_ms: n("chaos.at_pad_end_ms"),
            strain_ms: n("chaos.events.strain.ms"),
            strain_hold_ms: n("chaos.events.strain.hold_ms"),
            strain_regen: n("chaos.events.strain.regen"),
            flicker_ms: n("chaos.events.flicker.ms"),
            boxfast_ms: n("chaos.events.boxfast.ms"),
            boxfast_factor: n("chaos.events.boxfast.factor"),

            endless_seal_max: n("endless.seal.max"),
            endless_seal_regen: n("endless.seal.regen"),
            endless_budget_ms: n("endless.seal.budget"),
            rotate_ms: n("endless.rotate_hours") * hour_ms,
            ramp_roster: list(r, "endless.roster_by_gen").iter().map(|e| (num(e, "0") as usize, num(e, "1") as usize)).collect(),
            ramp_event_from: int(r, "endless.event_from") as u32,
            ramp_event_p: n("endless.event_p"),
            ramp_floor_from: n("endless.floor_from_hours") * hour_ms,
            ramp_floor_max: n("endless.floor_max"),
            ramp_floor_span: n("endless.floor_span"),
            pace_from: int(r, "endless.pace_from") as u32,
            pace_step: n("endless.pace_step"),
            pace_cap: n("endless.pace_cap"),
            tell_from: int(r, "endless.tell_from") as u32,
            tell_step: n("endless.tell_step"),
            tell_min: n("endless.tell_min"),
            tell_base,
            gap_start: n("endless.gap_start"),
            gap_step: n("endless.gap_step"),
            gap_min: n("endless.gap_min"),
            gap_jitter: n("endless.gap_jitter"),
            budget_from: int(r, "endless.budget_from") as u32,
            budget_step: n("endless.budget_step"),
            budget_min: n("endless.budget_min"),
            first_spike_span_ms: n("endless.first_spike_hours") * hour_ms,

            pool,
            shifts,
            routes: routes_of(b, "routes"),
            vent_routes: routes_of(b, "vent_routes"),
            cam_rooms: rooms_of(list(b, "cam_rooms")),
            box_rooms: rooms_of(list(b, "box_rooms")),
            vent_near: [num(b, "vent_near.L") as u8, num(b, "vent_near.R") as u8, num(b, "vent_near.T") as u8],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Watched,
    Monitor,
    Path,
    Vent,
    Winder,
}

impl Kind {
    fn ix(self) -> usize {
        self as usize
    }

    fn is_walker(self) -> bool {
        matches!(self, Kind::Path | Kind::Watched | Kind::Winder)
    }

    fn is_new(self) -> bool {
        matches!(self, Kind::Vent | Kind::Winder)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    L,
    R,
    T,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::L, Side::R, Side::T];

    fn ix(self) -> usize {
        self as usize
    }

    /// The one branch spot a vent light must catch for a crawler in this vent.
    fn vent_near(self) -> u8 {
        tn().vent_near[self.ix()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    Stage,
    Dining,
    Backstage,
    WestHall,
    Closet,
    Kitchen,
    EastHall,
    Corner,
    Vent,
    DoorL,
    DoorR,
    VentL,
    VentR,
    VentT,
}

impl Room {
    pub fn is_camera(self) -> bool {
        tn().cam_rooms.contains(&self)
    }

    /// The side of the door or vent opening this room is, if it is one.
    fn gate(self) -> Option<Side> {
        match self {
            Room::DoorL | Room::VentL => Some(Side::L),
            Room::DoorR | Room::VentR => Some(Side::R),
            Room::VentT => Some(Side::T),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Patient,
    Hungry,
    Shy,
    Reckless,
}

const MOODS: [Mood; 4] = [Mood::Patient, Mood::Hungry, Mood::Shy, Mood::Reckless];

impl Mood {
    fn speed(self) -> f64 {
        tn().mood_speed[self as usize]
    }

    fn tell(self) -> f64 {
        tn().mood_tell[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beat {
    Lull,
    Build,
    Spike,
    Breather,
}

impl Beat {
    fn rate(self) -> f64 {
        tn().beat_rate[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChaosKind {
    Mood,
    Wake,
    Strain,
    Flicker,
    Decoy,
    Boxfast,
}

fn chaos_weight(k: ChaosKind) -> f64 {
    tn().chaos_weight[k as usize]
}

fn chaos_needs(k: ChaosKind) -> Option<Kind> {
    tn().chaos_needs[k as usize]
}

/// One shift: the numbers the board sets. Endless reads its pace, tell, budget and gaps from the hour instead.
pub struct Difficulty {
    pub name: String,
    speed: f64,
    tell_ms: f64,
    min_tell_ms: f64,
    start_ms: f64,
    roster: (usize, usize),
    spikes: (usize, usize),
    grace_ms: f64,
    new_max: usize,
    min_types: usize,
    must: Vec<usize>,
    breather: Option<(f64, f64)>,
    chaos: bool,
    endless: bool,
}

pub struct PoolEntry {
    pub id: String,
    pub name: String,
    kind: Kind,
}

struct Route {
    side: Side,
    rooms: Vec<Room>,
    spots: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Gate {
    shut: bool,
    want: bool,
    held: f64,
    cool: f64,
    linger: f64,
    flash: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Spike {
    pub at: f64,
    pub len: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpikePhase {
    Warn,
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ongoing,
    Win,
    /// The threat (by pool index) that reached the security office.
    Caught(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VentLightError {
    Over,
    Shut,
    Cooling,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Beat(Beat),
    Spike(SpikePhase),
    Step { pool: usize, room: Room },
    Arrive { pool: usize, room: Option<Room>, side: Option<Side> },
    Tell { pool: usize, side: Option<Side>, lead: f64, rearm: bool },
    Turned { pool: usize, room: Room },
    Answered { pool: usize, how: &'static str },
    Gone { pool: usize },
    Echo { pool: usize, level: u32 },
    Stop { pool: usize },
    Wound { pool: usize },
    Kill { pool: usize, lead: f64 },
    Mood { pool: usize, mood: Mood },
    Feint { room: Option<Room>, side: Option<Side> },
    Strain { side: Side },
    DoorShut { side: Side },
    VentShut { side: Side },
    VentOpen { side: Side },
    VentStrain { side: Side },
    Hour(u32),
    Chaos { kind: ChaosKind, start: bool, room: Option<Room> },
    Decoy { kind: Kind, side: Side },
    Rotate { gen: u32, added: Vec<usize>, removed: Vec<usize> },
    Win,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Threat {
    pool: usize,
    mood: Mood,
    mood_left: f64,
    mood_cue: f64,
    active: bool,
    armed: bool,
    armed_ms: f64,
    armed_at: f64,
    tell_on: bool,
    tell_need: f64,
    side: Option<Side>,
    give_ms: f64,
    wait: f64,
    mem: [u32; 3],
    route_side: Side,
    spots: &'static [u8],
    rooms: &'static [Room],
    i: usize,
    seen: f64,
    dwell: f64,
    flinch: bool,
    seen_toggles: u32,
    present: bool,
    steps: u32,
    pend: u32,
    idle: f64,
    idle_max: f64,
    decay: f64,
    c: f64,
    hot: bool,
    grace: f64,
    span: f64,
    rewound: bool,
    tune: f64,
    wind_ms: f64,
    stopped: bool,
    box_room: Option<Room>,
}

impl Threat {
    pub fn name(&self) -> &'static str {
        tn().pool[self.pool].name.as_str()
    }

    pub fn kind(&self) -> Kind {
        tn().pool[self.pool].kind
    }

    pub fn mood(&self) -> Mood {
        self.mood
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    /// The side of the gate this threat stands at while it is armed (None for a counter, or when it is not armed).
    pub fn gate(&self) -> Option<Side> {
        if self.armed {
            self.side
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
struct ChaosState {
    hr: u32,
    at: f64,
    kind: Option<ChaosKind>,
    until: f64,
    strain: f64,
    flicker: f64,
    boxfast: f64,
}

impl ChaosState {
    fn new() -> Self {
        Self { hr: 0, at: -1.0, kind: None, until: 0.0, strain: 0.0, flicker: 0.0, boxfast: 0.0 }
    }
}

#[derive(Clone, Copy, Debug)]
struct Monitor {
    up: bool,
    room: Room,
    stay: f64,
    toggles: u32,
}

#[derive(Clone, Copy, Debug)]
struct Run {
    gen: u32,
    rotate_at: f64,
    budget: f64,
}

/// One night of Mixed Shift. The shell calls `step` each frame with the frame time in milliseconds, forwards the
/// player's door, vent, wind, monitor and camera inputs to the setters, and plays the returned events.
pub struct MixedShift {
    seed: i64,
    diff: &'static Difficulty,
    dir: NightWatchStream,
    th_rng: NightWatchStream,
    fx: NightWatchStream,
    seal_max: f64,
    elapsed: f64,
    over: Outcome,
    cool: f64,
    since_act: f64,
    lock: Option<usize>,
    beat: Beat,
    beat_left: f64,
    tension: f64,
    plan: Vec<Spike>,
    spike_idx: usize,
    spike_end: f64,
    warned: bool,
    feint_left: f64,
    seal: f64,
    doors: [Gate; 3],
    vent_seal: f64,
    vents: [Gate; 3],
    lamp_cool: f64,
    wind: bool,
    mon: Monitor,
    roster: Vec<usize>,
    threats: Vec<Threat>,
    chaos: ChaosState,
    run: Run,
}

fn between(range: (f64, f64), u: f64) -> f64 {
    range.0 + (range.1 - range.0) * u
}

fn jit(base: f64, rng: &mut NightWatchStream) -> f64 {
    jitter(base, tn().jitter, rng)
}

fn xor_seed(seed: i64, salt: u32) -> i64 {
    i64::from((seed as u32) ^ salt)
}

fn side_weights(mem: [u32; 3]) -> [f64; 3] {
    mem.map(|m| (1.0 / (1.0 + tn().side_decay * f64::from(m))).max(tn().side_floor))
}

/// The tell a threat gives before it can kill: the shift's, scaled by mood, never under the floor or the shift's minimum.
fn tell_for(tell_ms: f64, min_tell: f64, mood: Mood, u: f64) -> f64 {
    let raw = (tell_ms * mood.tell() * (1.0 + tn().tell_jitter * (2.0 * u - 1.0))).round();
    tn().tell_floor_ms.max(min_tell.max(raw))
}

fn step_ms(kind: Kind) -> f64 {
    tn().step_ms[kind.ix()]
}

fn rest_ms(kind: Kind) -> f64 {
    tn().rest_ms[kind.ix()]
}

fn tension_floor_at(t: f64) -> f64 {
    if t < tn().tension_floor_from {
        return 0.0;
    }
    let k = ((t - tn().tension_floor_from) / (tn().night_ms - tn().tension_floor_from)).clamp(0.0, 1.0);
    tn().tension_floor.0 + (tn().tension_floor.1 - tn().tension_floor.0) * k
}

fn endless_pace(hr: u32) -> f64 {
    (1.0 + tn().pace_step * f64::from(hr.saturating_sub(tn().pace_from))).min(tn().pace_cap)
}

fn endless_tell(hr: u32) -> f64 {
    (tn().tell_base - tn().tell_step * f64::from(hr.saturating_sub(tn().tell_from))).max(tn().tell_min)
}

fn endless_budget(hr: u32) -> f64 {
    (tn().endless_budget_ms - tn().budget_step * f64::from(hr.saturating_sub(tn().budget_from))).max(tn().budget_min)
}

fn endless_gap(hr: u32) -> f64 {
    (tn().gap_start - tn().gap_step * f64::from(hr)).max(tn().gap_min)
}

fn endless_floor_at(t: f64) -> f64 {
    if t < tn().ramp_floor_from {
        return 0.0;
    }
    tn().ramp_floor_max * (1.0 - (-(t - tn().ramp_floor_from) / tn().hour_ms / tn().ramp_floor_span).exp())
}

fn kinds_of(ids: &[usize]) -> Vec<Kind> {
    let mut out: Vec<Kind> = Vec::new();
    for &p in ids {
        let k = tn().pool[p].kind;
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

fn allowed(have: &[usize], p: usize, d: &Difficulty) -> bool {
    let kind = tn().pool[p].kind;
    let new_count = have.iter().filter(|&&x| tn().pool[x].kind.is_new()).count();
    let walker_count = have.iter().filter(|&&x| tn().pool[x].kind.is_walker()).count();
    !(kind.is_new() && new_count >= d.new_max) && !(kind.is_walker() && walker_count >= tn().max_walkers)
}

/// The night's roster, drawn from its own generator: 3 to 7 threats of the pool, at least `min_types` kinds.
fn draw_roster(seed: i64, d: &Difficulty, gen: u32) -> Vec<usize> {
    let base = (u64::from((seed as u32) ^ 0x524f_5354) + u64::from(gen) * 0x9e37_79b1) as u32;
    let mut r = NightWatchStream::seeded(i64::from(base));
    let range = if d.endless { tn().ramp_roster[(gen as usize).min(tn().ramp_roster.len() - 1)] } else { d.roster };
    let n = d.must.len().max(range.0 + (r.next() * (range.1 - range.0 + 1) as f64).floor() as usize);

    let mut bag: Vec<usize> = (0..tn().pool.len()).collect();
    let mut i = bag.len() - 1;
    while i > 0 {
        let j = (r.next() * (i + 1) as f64).floor() as usize;
        bag.swap(i, j);
        i -= 1;
    }

    let mut pick: Vec<usize> = d.must.to_vec();
    for &p in &bag {
        if pick.len() >= n {
            break;
        }
        if !pick.contains(&p) && allowed(&pick, p, d) {
            pick.push(p);
        }
    }

    let mut guard = 0;
    while kinds_of(&pick).len() < d.min_types && guard < 8 {
        guard += 1;
        let have = kinds_of(&pick);
        let mut at = pick.len().saturating_sub(1);
        for k in (0..pick.len()).rev() {
            let kind = tn().pool[pick[k]].kind;
            if pick.iter().filter(|&&x| tn().pool[x].kind == kind).count() > 1 {
                at = k;
                break;
            }
        }
        let rest: Vec<usize> = pick.iter().enumerate().filter(|(j, _)| *j != at).map(|(_, &x)| x).collect();
        match bag.iter().copied().find(|&p| !have.contains(&tn().pool[p].kind) && allowed(&rest, p, d)) {
            Some(add) => pick[at] = add,
            None => break,
        }
    }
    (0..tn().pool.len()).filter(|p| pick.contains(p)).collect()
}

/// A threat's whole state. `k` is its place in the roster and staggers the first move.
fn new_threat(p: usize, d: &Difficulty, rng: &mut NightWatchStream, toggles: u32, k: usize) -> Threat {
    let mut t = Threat { pool: p, seen_toggles: toggles, ..Threat::default() };
    t.mood = MOODS[(rng.next() * MOODS.len() as f64).floor() as usize];
    t.mood_left = between(tn().mood_ms, rng.next());
    t.mood_cue = between(tn().mood_cue_ms, rng.next());
    match tn().pool[p].kind {
        Kind::Counter => {
            t.grace = jit(tn().counter_grace_ms, rng);
            t.span = jit(tn().counter_span_ms, rng);
            t.active = true;
        }
        Kind::Monitor => t.wait = jit(tn().monitor_arrive_ms, rng) + k as f64 * tn().monitor_step_late_ms,
        Kind::Winder => {
            t.box_room = Some(tn().box_rooms[(rng.next() * tn().box_rooms.len() as f64).floor() as usize]);
            t.span = jit(tn().tune_span_ms, rng);
        }
        Kind::Path | Kind::Watched | Kind::Vent => t.wait = jit(d.start_ms, rng) + k as f64 * tn().walker_step_late_ms,
    }
    t
}

/// The spikes of a night: each at a jittered time inside its own slot, never in the first hour, ending before dawn.
fn plan_spikes(rng: &mut NightWatchStream, d: &Difficulty) -> Vec<Spike> {
    let count = d.spikes.0 + (rng.next() * (d.spikes.1 - d.spikes.0 + 1) as f64).floor() as usize;
    let start = tn().hour_ms + tn().spike_start_pad_ms;
    let end_ms = tn().night_ms - tn().end_clear_ms - tn().spike_len_ms.1 - tn().spike_end_pad_ms;
    let width = (end_ms - start) / count as f64;
    let breather = d.breather.unwrap_or(tn().breather_ms);
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let len = between(tn().spike_len_ms, rng.next()).round();
        let room = width - len - breather.1 - tn().spike_slot_pad_ms;
        let at = (start + k as f64 * width + rng.next() * room).round();
        out.push(Spike { at, len });
    }
    out
}

fn first_endless_spike(rng: &mut NightWatchStream) -> Spike {
    let at = (tn().hour_ms + tn().spike_start_pad_ms + rng.next() * tn().first_spike_span_ms).round();
    let len = between(tn().spike_len_ms, rng.next()).round();
    Spike { at, len }
}

impl MixedShift {
    /// Starts a night from `seed` on shift `shift` (0 QUIET, 1 STEADY, 2 RESTLESS, 3 CHAOS, 4 ENDLESS).
    pub fn start(seed: i64, shift: usize) -> Self {
        let diff: &'static Difficulty = &tn().shifts[shift.min(tn().shifts.len() - 1)];
        let mut dir = NightWatchStream::seeded(xor_seed(seed, 0x4449_5243));
        let mut th_rng = NightWatchStream::seeded(xor_seed(seed, 0x5448_5254));
        let mut fx = NightWatchStream::seeded(xor_seed(seed, 0x4658_5f31));
        let roster = draw_roster(seed, diff, 0);
        let plan = if diff.endless { vec![first_endless_spike(&mut dir)] } else { plan_spikes(&mut dir, diff) };
        let beat_left = between(tn().beat_lull_ms, dir.next());
        let feint_left = between(tn().feint_lull_ms, fx.next());
        let mut threats: Vec<Threat> = (0..tn().pool.len()).map(|p| Threat { pool: p, ..Threat::default() }).collect();
        for (k, &p) in roster.iter().enumerate() {
            threats[p] = new_threat(p, diff, &mut th_rng, 0, k);
        }
        let seal_max = if diff.endless { tn().endless_seal_max } else { tn().seal_max };
        Self {
            seed,
            diff,
            dir,
            th_rng,
            fx,
            seal_max,
            elapsed: 0.0,
            over: Outcome::Ongoing,
            cool: 0.0,
            since_act: 0.0,
            lock: None,
            beat: Beat::Lull,
            beat_left,
            tension: tn().tension_start,
            plan,
            spike_idx: 0,
            spike_end: 0.0,
            warned: false,
            feint_left,
            seal: seal_max,
            doors: [Gate::default(); 3],
            vent_seal: tn().vent_seal_max,
            vents: [Gate::default(); 3],
            lamp_cool: 0.0,
            wind: false,
            mon: Monitor { up: false, room: Room::Stage, stay: 0.0, toggles: 0 },
            roster,
            threats,
            chaos: ChaosState::new(),
            run: Run { gen: 0, rotate_at: tn().rotate_ms, budget: tn().endless_budget_ms },
        }
    }

    /// Advances the night by `dt_ms` and returns what happened in it.
    pub fn step(&mut self, dt: f64) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.over != Outcome::Ongoing {
            return ev;
        }
        self.elapsed += dt;
        self.since_act += dt;
        self.cool = (self.cool - dt).max(0.0);
        if self.elapsed >= tn().night_ms && !self.diff.endless {
            self.over = Outcome::Win;
            ev.push(Event::Win);
            return ev;
        }
        if self.chaos_on() {
            self.step_hours(&mut ev);
        }
        self.apply_floor();
        if self.mon.up {
            self.mon.stay += dt;
        }
        self.step_doors(dt, &mut ev);
        self.step_vents(dt, &mut ev);
        self.step_director(dt, &mut ev);
        self.step_feints(dt, &mut ev);
        if self.chaos_on() {
            self.step_chaos(dt, &mut ev);
        }
        if self.diff.endless {
            self.step_rotation(&mut ev);
        }
        for p in self.roster.clone() {
            if self.over != Outcome::Ongoing {
                break;
            }
            self.step_mood(p, dt, &mut ev);
            match tn().pool[p].kind {
                Kind::Counter => self.step_counter(p, dt, &mut ev),
                Kind::Monitor => self.step_monitor(p, dt, &mut ev),
                Kind::Winder => self.step_winder(p, dt, &mut ev),
                Kind::Path | Kind::Watched | Kind::Vent => self.step_walker(p, dt, &mut ev),
            }
        }
        if self.lock.is_none() && self.since_act > tn().idle_scare_ms {
            self.scare(dt * tn().scare_idle_per_min / 60_000.0);
        }
        self.apply_floor();
        ev
    }

    pub fn set_door(&mut self, side: Side, want: bool) -> bool {
        if self.over != Outcome::Ongoing || side == Side::T {
            return false;
        }
        let ix = side.ix();
        self.doors[ix].want = want;
        if !want {
            self.doors[ix].shut = false;
            self.doors[ix].held = 0.0;
        } else if !self.doors[ix].shut && self.doors[ix].cool <= 0.0 && self.seal >= tn().seal_min_to_shut {
            self.doors[ix].shut = true;
            self.doors[ix].held = 0.0;
        }
        self.doors[ix].shut
    }

    /// Holds (`want`) or lets go of a vent seal. A released seal stays shut for a moment, as in the 2D game.
    pub fn set_vent(&mut self, side: Side, want: bool) -> bool {
        if self.over != Outcome::Ongoing {
            return false;
        }
        let ix = side.ix();
        if want == self.vents[ix].want {
            return self.vents[ix].shut;
        }
        self.vents[ix].want = want;
        if want {
            if self.vents[ix].shut {
                self.vents[ix].linger = 0.0;
            } else if self.vents[ix].cool <= 0.0 && self.vent_seal >= tn().vent_min_to_seal {
                self.vents[ix].shut = true;
                self.vents[ix].held = 0.0;
                self.vents[ix].linger = 0.0;
            }
        } else if self.vents[ix].shut {
            self.vents[ix].linger = tn().vent_linger_ms;
        }
        self.vents[ix].shut
    }

    /// The vent light: a short flash down one vent. Returns the crawlers it catches, which turn back.
    pub fn light_vent(&mut self, side: Side) -> Result<Vec<usize>, VentLightError> {
        if self.over != Outcome::Ongoing {
            return Err(VentLightError::Over);
        }
        let ix = side.ix();
        if self.vents[ix].shut {
            return Err(VentLightError::Shut);
        }
        if self.lamp_cool > 0.0 {
            return Err(VentLightError::Cooling);
        }
        self.lamp_cool = tn().vent_light_cool_ms;
        self.vents[ix].flash = tn().vent_light_ms;
        let mut seen = Vec::new();
        for &p in &self.roster {
            if tn().pool[p].kind != Kind::Vent {
                continue;
            }
            let t = &self.threats[p];
            let hit = (t.armed && t.side == Some(side))
                || (t.active && !t.armed && self.vent_spot(p) == side.vent_near());
            if hit {
                self.threats[p].flinch = true;
                seen.push(p);
            }
        }
        Ok(seen)
    }

    pub fn set_wind(&mut self, on: bool) {
        if self.over == Outcome::Ongoing {
            self.wind = on;
            self.wind_jump();
        }
    }

    pub fn set_monitor(&mut self, up: bool) {
        if self.over != Outcome::Ongoing || up == self.mon.up {
            return;
        }
        self.mon.up = up;
        self.mon.stay = 0.0;
        self.mon.toggles += 1;
        self.wind_jump();
    }

    pub fn set_room(&mut self, room: Room) {
        if self.over != Outcome::Ongoing || !room.is_camera() || room == self.mon.room {
            return;
        }
        self.mon.room = room;
        self.mon.stay = 0.0;
    }

    pub fn outcome(&self) -> Outcome {
        self.over
    }

    /// The door and vent seal budgets as 0..=1, for the HUD bars.
    pub fn seal_levels(&self) -> (f64, f64) {
        (self.seal / self.seal_max.max(1.0), self.vent_seal / tn().vent_seal_max.max(1.0))
    }

    /// True while the music box is being wound.
    pub fn winding(&self) -> bool {
        self.wind
    }

    pub fn shift_name(&self) -> &'static str {
        self.diff.name.as_str()
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed
    }

    pub fn tension(&self) -> f64 {
        self.tension
    }

    pub fn beat(&self) -> Beat {
        self.beat
    }

    pub fn roster(&self) -> &[usize] {
        &self.roster
    }

    pub fn threat(&self, pool: usize) -> &Threat {
        &self.threats[pool]
    }

    pub fn next_spike(&self) -> Option<Spike> {
        self.plan.get(self.next_idx()).copied()
    }

    /// The threats standing in a camera room (walkers on their route, and crawlers in the vent run).
    pub fn occupants(&self, room: Room) -> Vec<usize> {
        self.roster
            .iter()
            .copied()
            .filter(|&p| {
                let t = &self.threats[p];
                let kind = tn().pool[p].kind;
                (kind.is_walker() || kind == Kind::Vent) && t.active && !t.armed && t.rooms.get(t.i) == Some(&room)
            })
            .collect()
    }

    /// The hour the player held out to: six at most in the timed shifts, and the full count in ENDLESS.
    pub fn hour_reached(&self) -> u32 {
        if self.diff.endless {
            self.hour_of()
        } else {
            self.hour_of().min(tn().dawn_hour)
        }
    }

    /// Points for a shift won (the timed shifts only; ENDLESS has no dawn to win).
    pub fn points(&self) -> u32 {
        match self.over {
            Outcome::Win if !self.diff.endless => tn().points_per_level,
            _ => 0,
        }
    }

    /// The hours to record as the ENDLESS best (`save.record_best`), once the night has ended.
    pub fn best_hours(&self) -> Option<u32> {
        match self.over {
            Outcome::Caught(_) if self.diff.endless => Some(self.hour_reached()),
            _ => None,
        }
    }

    fn hour_of(&self) -> u32 {
        (self.elapsed / tn().hour_ms).floor() as u32
    }

    fn live_speed(&self) -> f64 {
        if self.diff.endless {
            endless_pace(self.hour_of())
        } else {
            self.diff.speed
        }
    }

    fn live_tell(&self) -> f64 {
        if self.diff.endless {
            endless_tell(self.hour_of())
        } else {
            self.diff.tell_ms
        }
    }

    fn breather(&self) -> (f64, f64) {
        self.diff.breather.unwrap_or(tn().breather_ms)
    }

    fn chaos_on(&self) -> bool {
        self.diff.chaos || self.diff.endless
    }

    fn quiet(&self) -> bool {
        self.beat == Beat::Breather
    }

    fn can_act(&self) -> bool {
        self.cool <= 0.0 && !self.quiet()
    }

    fn time_runs(&self, p: usize) -> bool {
        !self.quiet() && self.lock.map_or(true, |l| l == p)
    }

    fn speed_of(&self, p: usize) -> f64 {
        self.beat.rate() * (tn().tension_speed_base + tn().tension_speed_scale * self.tension) * self.threats[p].mood.speed() * self.live_speed()
    }

    fn grace_ok(&self) -> bool {
        self.elapsed >= self.diff.grace_ms
    }

    fn observed(&self, room: Room) -> bool {
        self.mon.up && self.mon.room == room
    }

    fn fx_running(&self) -> bool {
        self.chaos.strain > 0.0 || self.chaos.flicker > 0.0 || self.chaos.boxfast > 0.0
    }

    fn scare(&mut self, d: f64) {
        self.tension = (self.tension + d).clamp(0.0, 1.0);
    }

    fn act(&mut self, ev: &mut Vec<Event>, events: Vec<Event>) {
        self.cool = tn().cooldown_ms;
        self.since_act = 0.0;
        ev.extend(events);
    }

    fn next_idx(&self) -> usize {
        if self.diff.endless {
            0
        } else {
            self.spike_idx
        }
    }

    fn vent_spot(&self, p: usize) -> u8 {
        let t = &self.threats[p];
        t.spots.get(t.i).copied().unwrap_or(0)
    }

    fn remember(&mut self, p: usize, side: Side) {
        let m = &mut self.threats[p].mem[side.ix()];
        *m = (*m + 1).min(tn().mem_cap);
    }

    fn gate_shut(&self, p: usize) -> bool {
        let Some(side) = self.threats[p].side else { return false };
        if tn().pool[p].kind == Kind::Vent {
            self.vents[side.ix()].shut
        } else {
            self.doors[side.ix()].shut
        }
    }

    fn apply_floor(&mut self) {
        let floor = if self.diff.endless {
            endless_floor_at(self.elapsed)
        } else if self.elapsed >= tn().tension_floor_from {
            tension_floor_at(self.elapsed)
        } else {
            0.0
        };
        self.tension = self.tension.max(floor);
    }

    fn wind_jump(&mut self) {
        let Some(p) = self.roster.iter().copied().find(|&p| tn().pool[p].kind == Kind::Winder) else { return };
        if let Some(box_room) = self.threats[p].box_room {
            if self.wind && self.mon.up && self.mon.room != box_room {
                self.set_room(box_room);
            }
        }
    }

    /// Takes the threat's place at a gate and starts its tell when the answer is not already shut.
    fn arm(&mut self, p: usize, side: Option<Side>) {
        self.lock = Some(p);
        let mood = self.threats[p].mood;
        let need = tell_for(self.live_tell(), self.diff.min_tell_ms, mood, self.th_rng.next());
        let t = &mut self.threats[p];
        t.armed = true;
        t.armed_ms = 0.0;
        t.armed_at = -1.0;
        t.tell_on = false;
        t.give_ms = 0.0;
        t.side = side;
        t.tell_need = need;
        if tn().pool[p].kind == Kind::Vent {
            if self.vent_seal < tn().vent_reserve {
                self.vent_seal = tn().vent_reserve;
            }
        } else if side.is_some() && self.seal < tn().seal_reserve {
            if self.diff.endless {
                let g = (tn().seal_reserve - self.seal).min(self.run.budget);
                self.seal += g;
                self.run.budget -= g;
            } else {
                self.seal = tn().seal_reserve;
            }
        }
        self.scare(tn().scare_arm);
    }

    fn start_tell(&mut self, p: usize, rearm: bool, ev: &mut Vec<Event>) {
        let elapsed = self.elapsed;
        let t = &mut self.threats[p];
        t.tell_on = true;
        t.armed_ms = 0.0;
        t.armed_at = elapsed;
        ev.push(Event::Tell { pool: p, side: t.side, lead: t.tell_need, rearm });
    }

    fn kill(&mut self, p: usize, ev: &mut Vec<Event>) {
        let lead = self.elapsed - self.threats[p].armed_at;
        if lead < tn().tell_floor_ms {
            return;
        }
        self.act(ev, vec![Event::Kill { pool: p, lead }]);
        self.over = Outcome::Caught(p);
    }

    /// The threat leaves the gate: answered by the player, or it gave up.
    fn send_away(&mut self, p: usize, how: &'static str, ev: &mut Vec<Event>) {
        let kind = tn().pool[p].kind;
        let side = self.threats[p].side;
        {
            let t = &mut self.threats[p];
            t.armed = false;
            t.tell_on = false;
            t.armed_ms = 0.0;
            t.give_ms = 0.0;
        }
        if self.lock == Some(p) {
            self.lock = None;
        }
        if let Some(s) = side {
            self.remember(p, s);
        }
        let wait = jit(rest_ms(kind), &mut self.th_rng);
        let t = &mut self.threats[p];
        t.active = false;
        t.present = false;
        t.steps = 0;
        t.pend = 0;
        t.i = 0;
        t.seen = 0.0;
        t.dwell = 0.0;
        t.side = None;
        t.flinch = false;
        t.rewound = false;
        if kind == Kind::Winder && how == "wind" {
            t.stopped = false;
            t.wait = 0.0;
        } else {
            t.wait = wait;
        }
        self.scare(tn().scare_away);
        self.act(ev, vec![Event::Answered { pool: p, how }, Event::Gone { pool: p }]);
    }

    fn step_doors(&mut self, dt: f64, ev: &mut Vec<Event>) {
        let strained = self.chaos.strain > 0.0;
        let hold_ms = if strained { tn().strain_hold_ms } else { tn().door_hold_ms };
        let mut open = 0;
        for ix in 0..2 {
            let side = Side::ALL[ix];
            self.doors[ix].cool = (self.doors[ix].cool - dt).max(0.0);
            if self.doors[ix].shut {
                self.doors[ix].held += dt;
                self.seal = (self.seal - dt).max(0.0);
                if self.doors[ix].held >= hold_ms || self.seal <= 0.0 {
                    let d = &mut self.doors[ix];
                    d.shut = false;
                    d.want = false;
                    d.held = 0.0;
                    d.cool = tn().pop_cool_ms;
                    ev.push(Event::Strain { side });
                }
            } else if self.doors[ix].want && self.doors[ix].cool <= 0.0 && self.seal >= tn().seal_min_to_shut {
                self.doors[ix].shut = true;
                self.doors[ix].held = 0.0;
                ev.push(Event::DoorShut { side });
            }
            if !self.doors[ix].shut {
                open += 1;
            }
        }
        if open > 0 {
            let base = if self.diff.endless { tn().endless_seal_regen } else { tn().seal_regen };
            let rate = base * if strained { tn().strain_regen } else { 1.0 };
            let mut gain = rate * dt * (open as f64 / 2.0);
            if self.diff.endless {
                gain = gain.min(self.run.budget.min((self.seal_max - self.seal).max(0.0)));
                self.run.budget -= gain;
            }
            self.seal = (self.seal + gain).min(self.seal_max);
        }
    }

    fn step_vents(&mut self, dt: f64, ev: &mut Vec<Event>) {
        self.lamp_cool = (self.lamp_cool - dt).max(0.0);
        let mut open = 0;
        for ix in 0..3 {
            let side = Side::ALL[ix];
            self.vents[ix].cool = (self.vents[ix].cool - dt).max(0.0);
            self.vents[ix].flash = (self.vents[ix].flash - dt).max(0.0);
            if self.vents[ix].shut {
                self.vent_seal = (self.vent_seal - dt).max(0.0);
                if self.vents[ix].want {
                    self.vents[ix].held += dt;
                } else {
                    self.vents[ix].linger = (self.vents[ix].linger - dt).max(0.0);
                }
                if self.vents[ix].held >= tn().vent_hold_ms || self.vent_seal <= 0.0 {
                    let v = &mut self.vents[ix];
                    v.shut = false;
                    v.want = false;
                    v.held = 0.0;
                    v.linger = 0.0;
                    v.cool = tn().vent_pop_cool_ms;
                    ev.push(Event::VentStrain { side });
                } else if !self.vents[ix].want && self.vents[ix].linger <= 0.0 {
                    self.vents[ix].shut = false;
                    self.vents[ix].held = 0.0;
                    ev.push(Event::VentOpen { side });
                }
            } else if self.vents[ix].want && self.vents[ix].cool <= 0.0 && self.vent_seal >= tn().vent_min_to_seal {
                let v = &mut self.vents[ix];
                v.shut = true;
                v.held = 0.0;
                v.linger = 0.0;
                ev.push(Event::VentShut { side });
            }
            if !self.vents[ix].shut {
                open += 1;
            }
        }
        if open > 0 {
            let gain = tn().vent_seal_regen * dt * (open as f64 / 3.0);
            self.vent_seal = (self.vent_seal + gain).min(tn().vent_seal_max);
        }
    }

    fn start_beat(&mut self, beat: Beat, ev: &mut Vec<Event>) {
        self.beat = beat;
        self.beat_left = match beat {
            Beat::Breather => {
                let u = self.dir.next();
                between(self.breather(), u)
            }
            Beat::Spike => self.next_spike().map_or(0.0, |s| s.len),
            Beat::Lull => {
                let u = self.dir.next();
                between(tn().beat_lull_ms, u)
            }
            Beat::Build => {
                let u = self.dir.next();
                between(tn().beat_build_ms, u)
            }
        };
        ev.push(Event::Beat(beat));
    }

    fn end_spike(&mut self) {
        if self.diff.endless {
            if !self.plan.is_empty() {
                self.plan.remove(0);
            }
            let gap = jitter(endless_gap(self.hour_of()), tn().gap_jitter, &mut self.dir);
            let at = (self.elapsed + tn().breather_ms.1 + gap).round();
            let len = between(tn().spike_len_ms, self.dir.next()).round();
            self.plan.push(Spike { at, len });
        }
        self.spike_idx += 1;
    }

    fn step_director(&mut self, dt: f64, ev: &mut Vec<Event>) {
        self.beat_left -= dt;
        let nxt = self.next_spike();
        match self.beat {
            Beat::Spike => {
                if self.elapsed >= self.spike_end && self.lock.is_none() {
                    ev.push(Event::Spike(SpikePhase::End));
                    self.end_spike();
                    self.start_beat(Beat::Breather, ev);
                }
            }
            Beat::Breather => {
                if self.beat_left <= 0.0 {
                    self.start_beat(Beat::Lull, ev);
                }
            }
            Beat::Lull | Beat::Build => {
                if let Some(n) = nxt {
                    if !self.warned && self.elapsed >= n.at - tn().spike_warn_ms {
                        self.warned = true;
                        ev.push(Event::Spike(SpikePhase::Warn));
                    }
                }
                let due = nxt.filter(|n| self.elapsed >= n.at);
                if let (Some(n), true) = (due, self.lock.is_none()) {
                    self.warned = false;
                    self.spike_end = self.elapsed + n.len;
                    self.start_beat(Beat::Spike, ev);
                    ev.push(Event::Spike(SpikePhase::Start));
                    for p in self.roster.clone() {
                        let t = &self.threats[p];
                        if t.active && !t.armed && tn().pool[p].kind != Kind::Counter {
                            let late = jitter(tn().walker_step_late_ms, tn().jitter, &mut self.dir);
                            self.threats[p].wait = self.threats[p].wait.min(late);
                        }
                    }
                } else if self.beat_left <= 0.0 {
                    let last = self.elapsed >= tn().tension_floor_from;
                    let mut again = false;
                    if self.beat == Beat::Build && self.tension > tn().build_again_above {
                        let u = self.dir.next();
                        again = u < if last { tn().build_again_p_last } else { tn().build_again_p };
                    }
                    let next = if self.beat == Beat::Lull || again { Beat::Build } else { Beat::Lull };
                    self.start_beat(next, ev);
                }
            }
        }
    }

    fn step_feints(&mut self, dt: f64, ev: &mut Vec<Event>) {
        let range = match self.beat {
            Beat::Lull => tn().feint_lull_ms,
            Beat::Build => tn().feint_build_ms,
            _ => return,
        };
        self.feint_left -= dt;
        if self.feint_left > 0.0 {
            return;
        }
        if self.lock.is_some() || self.since_act < tn().feint_quiet_ms || self.cool > 0.0 {
            self.feint_left = tn().feint_retry_ms;
            return;
        }
        if self.fx.next() < 0.5 {
            let u = self.fx.next();
            let room = tn().cam_rooms[(u * tn().cam_rooms.len() as f64).floor() as usize];
            ev.push(Event::Feint { room: Some(room), side: None });
        } else {
            let side = if self.fx.next() < 0.5 { Side::L } else { Side::R };
            ev.push(Event::Feint { room: None, side: Some(side) });
        }
        self.feint_left = between(range, self.fx.next());
    }

    fn step_mood(&mut self, p: usize, dt: f64, ev: &mut Vec<Event>) {
        self.threats[p].mood_left -= dt;
        if self.threats[p].mood_left <= 0.0 {
            let cur = self.threats[p].mood;
            let others: Vec<Mood> = MOODS.iter().copied().filter(|&m| m != cur).collect();
            let m = others[(self.th_rng.next() * others.len() as f64).floor() as usize];
            let left = between(tn().mood_ms, self.th_rng.next());
            let t = &mut self.threats[p];
            t.mood = m;
            t.mood_left = left;
            ev.push(Event::Mood { pool: p, mood: m });
        }
        self.threats[p].mood_cue -= dt;
        if self.threats[p].mood_cue <= 0.0 {
            self.threats[p].mood_cue = between(tn().mood_cue_ms, self.th_rng.next());
        }
    }

    fn choose_route(&mut self, p: usize) {
        let kind = tn().pool[p].kind;
        let bx = self.threats[p].box_room;
        let table: Vec<&'static Route> = match kind {
            Kind::Vent => tn().vent_routes.iter().collect(),
            Kind::Winder => tn().routes.iter().filter(|r| bx.map_or(false, |b| r.rooms.contains(&b))).collect(),
            _ => tn().routes.iter().collect(),
        };
        if table.is_empty() {
            return;
        }
        let w = side_weights(self.threats[p].mem);
        let weights: Vec<f64> = table.iter().map(|r| w[r.side.ix()]).collect();
        let total: f64 = weights.iter().sum();
        let mut x = self.th_rng.next() * total;
        let mut k = 0;
        while k + 1 < table.len() && x >= weights[k] {
            x -= weights[k];
            k += 1;
        }
        let r = table[k];
        let all: &'static [Room] = &r.rooms;
        let start = match (kind, bx.and_then(|b| all.iter().position(|&x| x == b))) {
            (Kind::Winder, Some(at)) if at + 3 <= all.len() => at,
            _ => 0,
        };
        let t = &mut self.threats[p];
        t.route_side = r.side;
        t.spots = &r.spots;
        t.rooms = &all[start..];
        t.i = 0;
        t.seen = 0.0;
        t.flinch = false;
    }

    fn step_walker(&mut self, p: usize, dt: f64, ev: &mut Vec<Event>) {
        if self.threats[p].armed {
            self.step_armed(p, dt, ev);
            return;
        }
        let kind = tn().pool[p].kind;
        if !self.threats[p].active {
            if !self.time_runs(p) {
                return;
            }
            let speed = self.speed_of(p);
            self.threats[p].wait -= dt * speed;
            if self.threats[p].wait > 0.0 || !self.can_act() {
                return;
            }
            self.choose_route(p);
            let Some(&room) = self.threats[p].rooms.first() else { return };
            let wait = jit(step_ms(kind), &mut self.th_rng);
            let t = &mut self.threats[p];
            t.active = true;
            t.dwell = 0.0;
            t.wait = wait;
            self.act(ev, vec![Event::Step { pool: p, room }]);
            return;
        }
        if kind == Kind::Winder && self.threats[p].rewound {
            if self.can_act() {
                self.send_away(p, "wind", ev);
            }
            return;
        }
        let rooms = self.threats[p].rooms;
        let i = self.threats[p].i;
        if rooms.len() < 2 || i >= rooms.len() {
            return;
        }
        let room = rooms[i];
        let hall = rooms[rooms.len() - 2];
        let seen_now = self.observed(room);
        self.threats[p].dwell += dt;

        if kind == Kind::Vent
            && self.threats[p].flinch
            && self.threats[p].dwell >= tn().min_dwell_ms
            && self.can_act()
        {
            let wait = jit(step_ms(kind), &mut self.th_rng);
            let t = &mut self.threats[p];
            t.flinch = false;
            t.i = 0;
            t.dwell = 0.0;
            t.wait = wait;
            let side = t.route_side;
            self.remember(p, side);
            self.scare(tn().scare_turn);
            self.act(ev, vec![Event::Turned { pool: p, room }]);
            return;
        }

        if kind == Kind::Path && room == hall && self.time_runs(p) {
            if seen_now {
                self.threats[p].seen += dt;
                if self.threats[p].seen >= tn().choke_ms && self.can_act() {
                    let to = i.saturating_sub(2);
                    let wait = jit(step_ms(kind), &mut self.th_rng);
                    let t = &mut self.threats[p];
                    t.i = to;
                    t.seen = 0.0;
                    t.dwell = 0.0;
                    t.wait = wait;
                    if let Some(side) = rooms[rooms.len() - 1].gate() {
                        self.remember(p, side);
                    }
                    self.scare(tn().scare_turn);
                    self.act(ev, vec![Event::Turned { pool: p, room: rooms[to] }]);
                    return;
                }
            } else {
                self.threats[p].seen = 0.0;
            }
        }

        let mood = self.threats[p].mood;
        if seen_now && (kind == Kind::Watched || mood == Mood::Shy) {
            return;
        }
        if !self.time_runs(p) {
            return;
        }
        let speed = self.speed_of(p);
        self.threats[p].wait -= dt * speed;
        let t = &self.threats[p];
        if t.wait > 0.0 || t.dwell < tn().min_dwell_ms || !self.can_act() {
            return;
        }
        if i + 1 >= rooms.len() {
            return;
        }
        let nxt = rooms[i + 1];
        if let Some(side) = nxt.gate() {
            if self.lock.is_some() || !self.grace_ok() {
                return;
            }
            self.threats[p].i = i + 1;
            self.arm(p, Some(side));
            self.act(ev, vec![Event::Arrive { pool: p, room: Some(nxt), side: Some(side) }]);
            if !self.gate_shut(p) {
                self.start_tell(p, false, ev);
            }
            return;
        }
        let wait = jit(step_ms(kind), &mut self.th_rng);
        let t = &mut self.threats[p];
        t.i = i + 1;
        t.dwell = 0.0;
        t.wait = wait;
        self.act(ev, vec![Event::Step { pool: p, room: nxt }]);
    }

    fn step_armed(&mut self, p: usize, dt: f64, ev: &mut Vec<Event>) {
        let kind = tn().pool[p].kind;
        if kind == Kind::Vent && self.threats[p].flinch && self.can_act() {
            self.send_away(p, "light", ev);
            return;
        }
        if kind == Kind::Winder && self.threats[p].rewound && self.can_act() {
            self.send_away(p, "wind", ev);
            return;
        }
        if self.gate_shut(p) {
            let t = &mut self.threats[p];
            t.tell_on = false;
            t.armed_ms = 0.0;
            t.give_ms += dt;
            let give_up = t.give_ms >= tn().give_up_ms;
            if give_up && self.can_act() {
                let how = if kind == Kind::Vent { "seal" } else { "door" };
                self.send_away(p, how, ev);
            }
            return;
        }
        self.threats[p].give_ms = 0.0;
        if !self.threats[p].tell_on {
            self.start_tell(p, true, ev);
            return;
        }
        self.threats[p].armed_ms += dt;
        let t = &self.threats[p];
        if t.armed_ms >= t.tell_need && self.can_act() {
            self.kill(p, ev);
        }
    }

    fn step_counter(&mut self, p: usize, dt: f64, ev: &mut Vec<Event>) {
        let holding = self.mon.up && self.mon.stay >= self.threats[p].grace;
        let span = self.threats[p].span;
        let span_eff = span / self.speed_of(p).max(0.3);
        if holding && !self.quiet() {
            let c = self.threats[p].c;
            self.threats[p].c = (c + dt / span_eff).min(1.0);
        } else if !holding {
            let c = self.threats[p].c;
            self.threats[p].c = (c - tn().counter_calm_decay * dt / span).max(0.0);
        }
        if self.threats[p].c > 0.0 {
            self.threats[p].hot = true;
        } else if self.threats[p].hot {
            self.threats[p].hot = false;
            self.threats[p].grace = jit(tn().counter_grace_ms, &mut self.th_rng);
            self.threats[p].span = jit(tn().counter_span_ms, &mut self.th_rng);
        }

        if !self.threats[p].armed {
            if self.threats[p].c >= tn().counter_arm_at && holding {
                if self.lock.is_none() && self.can_act() && self.grace_ok() {
                    self.arm(p, None);
                    let room = Some(self.mon.room);
                    self.act(ev, vec![Event::Arrive { pool: p, room, side: None }]);
                    self.start_tell(p, false, ev);
                } else {
                    let c = self.threats[p].c;
                    self.threats[p].c = c.min(tn().counter_arm_at);
                }
            }
            return;
        }
        if !holding {
            if !self.can_act() {
                return;
            }
            let t = &mut self.threats[p];
            t.armed = false;
            t.tell_on = false;
            t.armed_ms = 0.0;
            if self.lock == Some(p) {
                self.lock = None;
            }
            self.threats[p].grace = jit(tn().counter_grace_ms, &mut self.th_rng);
            self.threats[p].span = jit(tn().counter_span_ms, &mut self.th_rng);
            self.scare(tn().scare_away);
            let how = if self.mon.up { "switch" } else { "monitor" };
            self.act(ev, vec![Event::Answered { pool: p, how }]);
            return;
        }
        self.threats[p].armed_ms += dt;
        let t = &self.threats[p];
        if t.c >= 1.0 && t.armed_ms >= t.tell_need && self.can_act() {
            self.kill(p, ev);
        }
    }

    fn step_monitor(&mut self, p: usize, dt: f64, ev: &mut Vec<Event>) {
        let fresh = self.mon.toggles.saturating_sub(self.threats[p].seen_toggles);
        self.threats[p].seen_toggles = self.mon.toggles;

        if !self.threats[p].present {
            if !self.time_runs(p) {
                return;
            }
            let speed = self.speed_of(p);
            self.threats[p].wait -= dt * speed;
            if self.threats[p].wait > 0.0 || !self.can_act() || self.lock.is_some() || !self.grace_ok() {
                return;
            }
            let w = side_weights(self.threats[p].mem);
            let side = if self.th_rng.next() * (w[0] + w[1]) < w[0] { Side::L } else { Side::R };
            let idle_max = jit(tn().monitor_idle_leave_ms, &mut self.th_rng);
            let t = &mut self.threats[p];
            t.present = true;
            t.active = true;
            t.side = Some(side);
            t.steps = 0;
            t.pend = 0;
            t.idle = 0.0;
            t.decay = 0.0;
            t.give_ms = 0.0;
            t.idle_max = idle_max;
            self.act(ev, vec![Event::Arrive { pool: p, room: None, side: Some(side) }]);
            return;
        }
        if self.threats[p].armed {
            self.step_armed(p, dt, ev);
            return;
        }
        if self.quiet() {
            self.threats[p].pend = 0;
            return;
        }
        self.threats[p].pend = (self.threats[p].pend + fresh).min(1);
        let Some(side) = self.threats[p].side else { return };
        if self.doors[side.ix()].shut {
            self.threats[p].give_ms += dt;
            if self.threats[p].give_ms >= tn().give_up_ms && self.can_act() {
                self.send_away(p, "door", ev);
                return;
            }
        } else {
            self.threats[p].give_ms = 0.0;
        }
        if !self.time_runs(p) {
            return;
        }
        self.threats[p].idle += dt;
        self.threats[p].decay += dt;
        if self.threats[p].pend > 0 && self.can_act() {
            self.threats[p].pend = 0;
            self.threats[p].idle = 0.0;
            self.threats[p].decay = 0.0;
            if self.threats[p].steps < tn().monitor_steps_to_arm {
                self.threats[p].steps += 1;
                let level = self.threats[p].steps;
                self.act(ev, vec![Event::Echo { pool: p, level }]);
            } else if self.lock.is_none() && self.grace_ok() {
                self.arm(p, Some(side));
                self.act(ev, vec![Event::Arrive { pool: p, room: None, side: Some(side) }]);
                if !self.doors[side.ix()].shut {
                    self.start_tell(p, false, ev);
                }
            } else {
                self.threats[p].pend = 1;
            }
            return;
        }
        if self.threats[p].decay >= tn().monitor_decay_ms && self.threats[p].steps > 0 {
            self.threats[p].steps -= 1;
            self.threats[p].decay = 0.0;
        }
        let t = &self.threats[p];
        if t.idle >= t.idle_max && self.can_act() {
            self.send_away(p, "steady", ev);
        }
    }

    fn step_winder(&mut self, p: usize, dt: f64, ev: &mut Vec<Event>) {
        let box_room = self.threats[p].box_room;
        let winding = self.wind && self.mon.up && Some(self.mon.room) == box_room && self.over == Outcome::Ongoing;
        let wind_ms = self.threats[p].wind_ms;
        self.threats[p].wind_ms = if winding { wind_ms + dt } else { 0.0 };
        let was = self.threats[p].stopped;
        let before = self.threats[p].tune;

        if !was && self.time_runs(p) {
            let boxfast = if self.chaos.boxfast > 0.0 { tn().boxfast_factor } else { 1.0 };
            let speed = self.speed_of(p);
            let t = &self.threats[p];
            let next = (t.tune - dt * speed * boxfast / t.span).max(0.0);
            self.threats[p].tune = next;
        }

        if winding {
            let next = (self.threats[p].tune + tn().tune_wind_per_sec * dt / 1000.0).min(1.0);
            self.threats[p].tune = next;
            if was && next >= tn().tune_restart_at {
                let span = jit(tn().tune_span_ms, &mut self.th_rng);
                let t = &mut self.threats[p];
                t.stopped = false;
                t.span = span;
                let rewind = t.active || t.armed;
                if rewind {
                    t.rewound = true;
                } else {
                    t.wait = 0.0;
                }
                ev.push(Event::Wound { pool: p });
            } else if !was && before < 1.0 && next >= 1.0 {
                self.threats[p].span = jit(tn().tune_span_ms, &mut self.th_rng);
            }
        }

        if !self.threats[p].stopped {
            if self.elapsed < self.diff.grace_ms {
                let t = self.threats[p].tune;
                self.threats[p].tune = t.max(tn().tune_grace_floor);
            }
            if self.threats[p].tune <= 0.0005 && self.threats[p].wind_ms == 0.0 {
                if self.can_act() && self.grace_ok() && self.time_runs(p) {
                    let wait = jit(tn().cranky_ms, &mut self.th_rng);
                    let t = &mut self.threats[p];
                    t.tune = 0.0;
                    t.stopped = true;
                    t.wait = wait;
                    self.act(ev, vec![Event::Stop { pool: p }]);
                } else {
                    self.threats[p].tune = 0.0006;
                }
            }
        }

        let t = &self.threats[p];
        if t.stopped || t.active || t.armed {
            self.step_walker(p, dt, ev);
        }
    }

    fn step_hours(&mut self, ev: &mut Vec<Event>) {
        while self.chaos.hr < self.hour_of() {
            self.chaos.hr += 1;
            let hr = self.chaos.hr;
            if self.diff.endless {
                self.run.budget = endless_budget(hr);
                ev.push(Event::Hour(hr));
            }
            let roll = if self.diff.chaos {
                (tn().chaos_hours.0..=tn().chaos_hours.1).contains(&hr)
            } else {
                self.diff.endless && hr >= tn().ramp_event_from && self.dir.next() < tn().ramp_event_p
            };
            if roll && self.chaos.at < 0.0 {
                self.schedule_event(f64::from(hr) * tn().hour_ms);
            }
        }
    }

    fn schedule_event(&mut self, hour_start: f64) {
        let have: Vec<Kind> = self.roster.iter().map(|&p| tn().pool[p].kind).collect();
        let kinds: Vec<ChaosKind> = tn()
            .chaos_order
            .iter()
            .copied()
            .filter(|&k| chaos_needs(k).map_or(true, |n| have.contains(&n)))
            .collect();
        let total: f64 = kinds.iter().map(|&k| chaos_weight(k)).sum();
        let mut x = self.dir.next() * total;
        let mut kind = kinds[0];
        for &k in &kinds {
            if x < chaos_weight(k) {
                kind = k;
                break;
            }
            x -= chaos_weight(k);
        }
        let u = self.dir.next();
        self.chaos.kind = Some(kind);
        self.chaos.until = hour_start + tn().hour_ms;
        self.chaos.at = (hour_start + tn().chaos_pad_start_ms + u * (tn().hour_ms - tn().chaos_pad_end_ms)).max(self.diff.grace_ms);
        if self.chaos.at >= self.chaos.until {
            self.chaos.kind = None;
            self.chaos.at = -1.0;
        }
    }

    fn step_chaos(&mut self, dt: f64, ev: &mut Vec<Event>) {
        for kind in [ChaosKind::Strain, ChaosKind::Flicker, ChaosKind::Boxfast] {
            let field = match kind {
                ChaosKind::Strain => &mut self.chaos.strain,
                ChaosKind::Flicker => &mut self.chaos.flicker,
                _ => &mut self.chaos.boxfast,
            };
            if *field > 0.0 {
                *field = (*field - dt).max(0.0);
                if *field == 0.0 {
                    ev.push(Event::Chaos { kind, start: false, room: None });
                }
            }
        }
        if self.chaos.at < 0.0 || self.elapsed < self.chaos.at {
            return;
        }
        if self.elapsed >= self.chaos.until {
            self.chaos.at = -1.0;
            self.chaos.kind = None;
            return;
        }
        let Some(kind) = self.chaos.kind else { return };
        if self.lock.is_some() || self.fx_running() || self.over != Outcome::Ongoing {
            return;
        }
        let decoy_wait = self.since_act < tn().decoy_quiet_ms
            || self.cool > 0.0
            || !matches!(self.beat, Beat::Lull | Beat::Build);
        if kind == ChaosKind::Decoy && decoy_wait {
            return;
        }
        self.fire_event(kind, ev);
        self.chaos.at = -1.0;
        self.chaos.kind = None;
    }

    fn fire_event(&mut self, kind: ChaosKind, ev: &mut Vec<Event>) {
        match kind {
            ChaosKind::Mood => {
                if self.roster.is_empty() {
                    return;
                }
                let u = self.dir.next();
                let p = self.roster[(u * self.roster.len() as f64).floor() as usize];
                let cur = self.threats[p].mood;
                let others: Vec<Mood> = MOODS.iter().copied().filter(|&m| m != cur).collect();
                let u2 = self.dir.next();
                let m = others[(u2 * others.len() as f64).floor() as usize];
                let u3 = self.dir.next();
                let t = &mut self.threats[p];
                t.mood = m;
                t.mood_left = between(tn().mood_ms, u3);
                ev.push(Event::Chaos { kind, start: true, room: None });
                ev.push(Event::Mood { pool: p, mood: m });
            }
            ChaosKind::Wake => {
                let found = self
                    .roster
                    .iter()
                    .copied()
                    .find(|&p| tn().pool[p].kind == Kind::Vent && !self.threats[p].active && !self.threats[p].armed);
                let Some(p) = found else { return };
                let late = jit(tn().wake_late_ms, &mut self.dir);
                self.threats[p].wait = self.threats[p].wait.min(late);
                ev.push(Event::Chaos { kind, start: true, room: None });
            }
            ChaosKind::Decoy => {
                let types: Vec<Kind> = tn().decoy_kinds
                    .iter()
                    .copied()
                    .filter(|&k| self.roster.iter().any(|&p| tn().pool[p].kind == k))
                    .collect();
                let mut tkind = Kind::Path;
                if !types.is_empty() {
                    let u = self.dir.next();
                    tkind = types[(u * types.len() as f64).floor() as usize];
                }
                let side = if tkind == Kind::Vent {
                    let u = self.dir.next();
                    Side::ALL[(u * 3.0).floor() as usize]
                } else if self.dir.next() < 0.5 {
                    Side::L
                } else {
                    Side::R
                };
                self.cool = self.cool.max(tn().decoy_quiet_ms);
                ev.push(Event::Chaos { kind, start: true, room: None });
                ev.push(Event::Decoy { kind: tkind, side });
            }
            ChaosKind::Flicker => {
                let u = self.dir.next();
                let room = tn().cam_rooms[(u * tn().cam_rooms.len() as f64).floor() as usize];
                self.chaos.flicker = tn().flicker_ms;
                ev.push(Event::Chaos { kind, start: true, room: Some(room) });
            }
            ChaosKind::Strain => {
                self.chaos.strain = tn().strain_ms;
                ev.push(Event::Chaos { kind, start: true, room: None });
            }
            ChaosKind::Boxfast => {
                self.chaos.boxfast = tn().boxfast_ms;
                ev.push(Event::Chaos { kind, start: true, room: None });
            }
        }
    }

    /// ENDLESS: every three hours the roster is drawn again, once no deadly tell is running. Whoever stays keeps state.
    fn step_rotation(&mut self, ev: &mut Vec<Event>) {
        if self.elapsed < self.run.rotate_at || self.lock.is_some() {
            return;
        }
        if self.roster.iter().any(|&p| self.threats[p].armed) {
            return;
        }
        self.run.gen += 1;
        self.run.rotate_at += tn().rotate_ms;
        let gen = self.run.gen;
        let next = draw_roster(self.seed, self.diff, gen);
        let removed: Vec<usize> = self.roster.iter().copied().filter(|p| !next.contains(p)).collect();
        let added: Vec<usize> = next.iter().copied().filter(|p| !self.roster.contains(p)).collect();
        let toggles = self.mon.toggles;
        for (k, &p) in added.iter().enumerate() {
            self.threats[p] = new_threat(p, self.diff, &mut self.th_rng, toggles, k);
        }
        self.roster = next;
        ev.push(Event::Rotate { gen, added, removed });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tell_never_drops_below_its_floor() {
        assert_eq!(tell_for(1000.0, 0.0, Mood::Hungry, 0.5), tn().tell_floor_ms);
        assert_eq!(tell_for(1000.0, 3000.0, Mood::Patient, 0.5), 3000.0);
        assert_eq!(tell_for(tn().shifts[1].tell_ms, 0.0, Mood::Patient, 0.5), 1900.0);
    }

    #[test]
    fn the_rules_file_carries_every_key_the_mode_reads() {
        // Loading reads (and panics on a missing key of) every rules, cast and building key; then spot-check the tables.
        let t = tn();
        assert_eq!(t.pool.len(), 8);
        assert_eq!(t.shifts.len(), 5);
        assert_eq!(t.routes.len(), 6);
        assert_eq!(t.vent_routes.len(), 3);
        assert_eq!(t.cam_rooms.len(), 9);
        assert_eq!(t.box_rooms.len(), 8);
        assert_eq!(t.chaos_order.len(), 6);
        assert_eq!(t.ramp_roster.len(), 4);
        assert!(t.shifts[3].chaos && t.shifts[4].endless);
        assert_eq!(t.shifts[3].must, vec![6, 7]);
        assert_eq!(t.pool[0].kind, Kind::Counter);
        assert_eq!(t.pool[7].kind, Kind::Winder);
        assert_eq!(t.pool[4].name, "Dovelle");
        for route in t.routes.iter().chain(&t.vent_routes) {
            assert!(route.rooms.last().and_then(|r| r.gate()).is_some());
        }
    }

    #[test]
    fn loaded_values_equal_the_old_constants() {
        let t = tn();
        assert_eq!((t.hour_ms, t.night_ms, t.points_per_level, t.dawn_hour), (45_000.0, 270_000.0, 10, 6));
        assert_eq!((t.tell_floor_ms, t.cooldown_ms, t.give_up_ms, t.choke_ms), (1_500.0, 1_200.0, 1_200.0, 2_000.0));
        assert_eq!((t.seal_max, t.seal_regen, t.door_hold_ms), (20_000.0, 0.3, 8_000.0));
        assert_eq!((t.vent_seal_max, t.vent_hold_ms, t.vent_light_cool_ms), (9_000.0, 6_000.0, 2_400.0));
        assert_eq!(t.mood_ms, (30_000.0, 60_000.0));
        assert_eq!(t.tension_floor_from, 225_000.0);
        assert_eq!(t.rotate_ms, 135_000.0);
        assert_eq!(t.vent_near, [3, 4, 2]);
        assert_eq!((Mood::Reckless.speed(), Mood::Hungry.tell(), Beat::Spike.rate(), Beat::Breather.rate()), (1.4, 0.85, 1.8, 0.0));
        assert_eq!((step_ms(Kind::Winder), rest_ms(Kind::Monitor), step_ms(Kind::Counter)), (12_000.0, 20_000.0, 0.0));
        assert_eq!((chaos_weight(ChaosKind::Flicker), chaos_weight(ChaosKind::Wake)), (3.0, 2.0));
        assert_eq!((chaos_needs(ChaosKind::Wake), chaos_needs(ChaosKind::Boxfast), chaos_needs(ChaosKind::Mood)), (Some(Kind::Vent), Some(Kind::Winder), None));
        assert_eq!(t.shifts[0].min_tell_ms, 3_000.0);
        assert_eq!(t.shifts[3].breather, Some((4_500.0, 6_500.0)));
        assert_eq!(t.shifts[1].breather, None);
        assert_eq!(t.shifts[2].roster, (4, 5));
        assert_eq!((endless_pace(10), endless_tell(0), endless_tell(20), endless_gap(0), endless_gap(20)), (1.32, 1_900.0, 1_600.0, 46_000.0, 10_000.0));
        assert_eq!((endless_budget(0), endless_budget(30)), (2_400.0, 1_400.0));
        assert_eq!(endless_floor_at(0.0), 0.0);
        assert_eq!(tension_floor_at(225_000.0), 0.35);
        assert_eq!(Side::T.vent_near(), 2);
    }
}
