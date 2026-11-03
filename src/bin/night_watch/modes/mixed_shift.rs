//! Mixed Shift (Godot `game/night_watch/mixed/mixed_logic.gd`, numbers from `mixed_data.gd`): each night draws 3 to 7
//! threats from a pool of eight. Every threat gives a tell before it can kill and has an answer, and a director paces
//! the beats. This file holds the rules only. The shell draws the building and plays the `Event`s that `step` returns.
//! Draws come from the shared `NightWatchStream` in the same order as the GDScript, so a seed plays the same night.

use crate::framework::{jitter, NightWatchStream};

const HOUR_MS: f64 = 45_000.0;
const NIGHT_MS: f64 = 270_000.0;
/// The shared per-level points (`POINTS_PER_LEVEL` in Godot's `night_watch_modes_data.gd`).
pub const POINTS_PER_LEVEL: u32 = 10;

const TELL_FLOOR_MS: f64 = 1_500.0;
const TELL_JITTER: f64 = 0.2;
const JITTER: f64 = 0.3;
const COOLDOWN_MS: f64 = 1_200.0;
const GIVE_UP_MS: f64 = 1_200.0;
const CHOKE_MS: f64 = 2_000.0;
const END_CLEAR_MS: f64 = 10_000.0;
const MIN_DWELL_MS: f64 = 2_500.0;
const MAX_WALKERS: usize = 4;
const DECOY_QUIET_MS: f64 = 3_000.0;
const CHAOS_HOURS: (u32, u32) = (1, 5);
const FEINT_QUIET_MS: f64 = 2_500.0;

const SEAL_MAX: f64 = 20_000.0;
const SEAL_REGEN: f64 = 0.3;
const SEAL_MIN_TO_SHUT: f64 = 1_500.0;
const SEAL_RESERVE: f64 = 2_600.0;
const DOOR_HOLD_MS: f64 = 8_000.0;
const POP_COOL_MS: f64 = 900.0;

const VENT_SEAL_MAX: f64 = 9_000.0;
const VENT_SEAL_REGEN: f64 = 0.35;
const VENT_MIN_TO_SEAL: f64 = 1_200.0;
const VENT_RESERVE: f64 = 2_600.0;
const VENT_HOLD_MS: f64 = 6_000.0;
const VENT_LINGER_MS: f64 = 2_200.0;
const VENT_POP_COOL_MS: f64 = 1_100.0;
const VENT_LIGHT_MS: f64 = 700.0;
const VENT_LIGHT_COOL_MS: f64 = 2_400.0;

const MOOD_MS: (f64, f64) = (30_000.0, 60_000.0);
const MOOD_CUE_MS: (f64, f64) = (14_000.0, 24_000.0);
const SPIKE_LEN_MS: (f64, f64) = (10_000.0, 13_000.0);
const SPIKE_WARN_MS: f64 = 2_500.0;
const SPIKE_START_PAD_MS: f64 = 8_000.0;
const BREATHER_MS: (f64, f64) = (8_000.0, 11_000.0);
const BEAT_LULL_MS: (f64, f64) = (12_000.0, 20_000.0);
const BEAT_BUILD_MS: (f64, f64) = (10_000.0, 16_000.0);
const FEINT_LULL_MS: (f64, f64) = (9_000.0, 16_000.0);
const FEINT_BUILD_MS: (f64, f64) = (16_000.0, 26_000.0);
const TENSION_FLOOR_FROM: f64 = 5.0 * HOUR_MS;
const TENSION_FLOOR: (f64, f64) = (0.35, 0.85);

const COUNTER_GRACE_MS: f64 = 7_000.0;
const COUNTER_SPAN_MS: f64 = 22_000.0;
const COUNTER_ARM_AT: f64 = 0.65;
const COUNTER_CALM_DECAY: f64 = 3.0;
const MONITOR_ARRIVE_MS: f64 = 30_000.0;
const MONITOR_IDLE_LEAVE_MS: f64 = 16_000.0;
const MONITOR_STEPS_TO_ARM: u32 = 3;
const MONITOR_DECAY_MS: f64 = 8_000.0;
const MONITOR_STEP_LATE_MS: f64 = 1_500.0;
const WALKER_STEP_LATE_MS: f64 = 2_500.0;

const TUNE_SPAN_MS: f64 = 50_000.0;
const TUNE_WIND_PER_SEC: f64 = 0.12;
const TUNE_RESTART_AT: f64 = 0.12;
const TUNE_GRACE_FLOOR: f64 = 0.25;
const CRANKY_MS: f64 = 7_000.0;
const BOX_ROOMS: [Room; 8] = [
    Room::Stage,
    Room::Dining,
    Room::Backstage,
    Room::WestHall,
    Room::Closet,
    Room::Kitchen,
    Room::EastHall,
    Room::Corner,
];

const STRAIN_MS: f64 = 60_000.0;
const STRAIN_HOLD_MS: f64 = 5_000.0;
const STRAIN_REGEN: f64 = 0.5;
const FLICKER_MS: f64 = 20_000.0;
const BOXFAST_MS: f64 = 30_000.0;
const BOXFAST_FACTOR: f64 = 2.0;

const ENDLESS_SEAL_MAX: f64 = 8_000.0;
const ENDLESS_SEAL_REGEN: f64 = 0.25;
const ENDLESS_BUDGET_MS: f64 = 2_400.0;
const ROTATE_MS: f64 = 3.0 * HOUR_MS;
const RAMP_ROSTER: [(usize, usize); 4] = [(3, 4), (4, 5), (4, 5), (5, 6)];
const RAMP_EVENT_FROM: u32 = 4;
const RAMP_EVENT_P: f64 = 0.5;
const RAMP_FLOOR_MAX: f64 = 0.85;
const RAMP_FLOOR_SPAN: f64 = 5.0;

const CAM_ROOMS: [Room; 9] = [
    Room::Stage,
    Room::Dining,
    Room::Backstage,
    Room::WestHall,
    Room::Closet,
    Room::Kitchen,
    Room::EastHall,
    Room::Corner,
    Room::Vent,
];

/// Pool index of the two threats that CHAOS always draws.
const VERRINE: usize = 6;
const GLYRA: usize = 7;

/// The eight threats, in the Godot pool's order (the roster keeps this order too).
pub const POOL: [PoolEntry; 8] = [
    PoolEntry { id: "haelith", name: "Haelith", kind: Kind::Counter },
    PoolEntry { id: "ilyra", name: "Ilyra", kind: Kind::Watched },
    PoolEntry { id: "aki", name: "Aki", kind: Kind::Watched },
    PoolEntry { id: "opalie", name: "Opalie", kind: Kind::Monitor },
    PoolEntry { id: "dovelle", name: "Dovelle", kind: Kind::Path },
    PoolEntry { id: "ravelle", name: "Ravelle", kind: Kind::Path },
    PoolEntry { id: "verrine", name: "Verrine", kind: Kind::Vent },
    PoolEntry { id: "glyra", name: "Glyra", kind: Kind::Winder },
];

/// The five shifts in board order: three paces, CHAOS (random events) and ENDLESS (no dawn).
pub static SHIFTS: [Difficulty; 5] = [
    Difficulty {
        name: "QUIET",
        speed: 0.8,
        tell_ms: 3600.0,
        min_tell_ms: 3000.0,
        start_ms: 50_000.0,
        roster: (3, 3),
        spikes: (2, 2),
        grace_ms: 90_000.0,
        new_max: 1,
        min_types: 3,
        must: &[],
        breather: None,
        chaos: false,
        endless: false,
    },
    Difficulty {
        name: "STEADY",
        speed: 1.0,
        tell_ms: 1900.0,
        min_tell_ms: 0.0,
        start_ms: 32_000.0,
        roster: (3, 5),
        spikes: (2, 3),
        grace_ms: 75_000.0,
        new_max: 2,
        min_types: 3,
        must: &[],
        breather: None,
        chaos: false,
        endless: false,
    },
    Difficulty {
        name: "RESTLESS",
        speed: 1.15,
        tell_ms: 1750.0,
        min_tell_ms: 0.0,
        start_ms: 24_000.0,
        roster: (4, 5),
        spikes: (3, 3),
        grace_ms: 75_000.0,
        new_max: 2,
        min_types: 3,
        must: &[],
        breather: None,
        chaos: false,
        endless: false,
    },
    Difficulty {
        name: "CHAOS",
        speed: 1.3,
        tell_ms: 1650.0,
        min_tell_ms: 0.0,
        start_ms: 15_000.0,
        roster: (6, 7),
        spikes: (4, 6),
        grace_ms: 75_000.0,
        new_max: 2,
        min_types: 4,
        must: &[VERRINE, GLYRA],
        breather: Some((4500.0, 6500.0)),
        chaos: true,
        endless: false,
    },
    Difficulty {
        name: "ENDLESS",
        speed: 1.0,
        tell_ms: 1900.0,
        min_tell_ms: 0.0,
        start_ms: 32_000.0,
        roster: (3, 4),
        spikes: (1, 1),
        grace_ms: 75_000.0,
        new_max: 2,
        min_types: 3,
        must: &[],
        breather: None,
        chaos: false,
        endless: true,
    },
];

/// The routes through Classic's building; the last room before a door is the choke point.
static ROUTES: [Route; 6] = [
    Route { side: Side::L, rooms: &[Room::Stage, Room::Backstage, Room::WestHall, Room::DoorL], spots: &[] },
    Route { side: Side::L, rooms: &[Room::Kitchen, Room::Dining, Room::WestHall, Room::DoorL], spots: &[] },
    Route { side: Side::L, rooms: &[Room::Closet, Room::WestHall, Room::DoorL], spots: &[] },
    Route { side: Side::R, rooms: &[Room::Stage, Room::Dining, Room::EastHall, Room::DoorR], spots: &[] },
    Route { side: Side::R, rooms: &[Room::Kitchen, Room::Dining, Room::EastHall, Room::DoorR], spots: &[] },
    Route { side: Side::R, rooms: &[Room::Corner, Room::EastHall, Room::DoorR], spots: &[] },
];

/// The vent run (CAM 9) to the three office vents; `spots` are the branch points a vent light can catch.
static VENT_ROUTES: [Route; 3] = [
    Route {
        side: Side::L,
        rooms: &[Room::Vent, Room::Vent, Room::Vent, Room::Vent, Room::VentL],
        spots: &[0, 1, 2, 3],
    },
    Route {
        side: Side::R,
        rooms: &[Room::Vent, Room::Vent, Room::Vent, Room::Vent, Room::VentR],
        spots: &[0, 1, 2, 4],
    },
    Route { side: Side::T, rooms: &[Room::Vent, Room::Vent, Room::Vent, Room::VentT], spots: &[0, 1, 2] },
];

const DECOY_KINDS: [Kind; 5] = [Kind::Path, Kind::Watched, Kind::Monitor, Kind::Vent, Kind::Winder];
const CHAOS_ORDER: [ChaosKind; 6] = [
    ChaosKind::Mood,
    ChaosKind::Wake,
    ChaosKind::Strain,
    ChaosKind::Flicker,
    ChaosKind::Decoy,
    ChaosKind::Boxfast,
];

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
        match self {
            Side::L => 3,
            Side::R => 4,
            Side::T => 2,
        }
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
        CAM_ROOMS.contains(&self)
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
        match self {
            Mood::Patient => 0.8,
            Mood::Hungry => 1.25,
            Mood::Shy => 1.0,
            Mood::Reckless => 1.4,
        }
    }

    fn tell(self) -> f64 {
        match self {
            Mood::Patient => 1.0,
            Mood::Hungry => 0.85,
            Mood::Shy => 1.0,
            Mood::Reckless => 1.3,
        }
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
        match self {
            Beat::Lull => 0.65,
            Beat::Build => 1.0,
            Beat::Spike => 1.8,
            Beat::Breather => 0.0,
        }
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
    match k {
        ChaosKind::Mood | ChaosKind::Flicker | ChaosKind::Decoy => 3.0,
        ChaosKind::Wake | ChaosKind::Strain | ChaosKind::Boxfast => 2.0,
    }
}

fn chaos_needs(k: ChaosKind) -> Option<Kind> {
    match k {
        ChaosKind::Wake => Some(Kind::Vent),
        ChaosKind::Boxfast => Some(Kind::Winder),
        _ => None,
    }
}

/// One shift: the numbers the board sets. Endless reads its pace, tell, budget and gaps from the hour instead.
pub struct Difficulty {
    pub name: &'static str,
    speed: f64,
    tell_ms: f64,
    min_tell_ms: f64,
    start_ms: f64,
    roster: (usize, usize),
    spikes: (usize, usize),
    grace_ms: f64,
    new_max: usize,
    min_types: usize,
    must: &'static [usize],
    breather: Option<(f64, f64)>,
    chaos: bool,
    endless: bool,
}

pub struct PoolEntry {
    pub id: &'static str,
    pub name: &'static str,
    kind: Kind,
}

struct Route {
    side: Side,
    rooms: &'static [Room],
    spots: &'static [u8],
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
        POOL[self.pool].name
    }

    pub fn kind(&self) -> Kind {
        POOL[self.pool].kind
    }

    pub fn mood(&self) -> Mood {
        self.mood
    }

    pub fn armed(&self) -> bool {
        self.armed
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
    jitter(base, JITTER, rng)
}

fn xor_seed(seed: i64, salt: u32) -> i64 {
    i64::from((seed as u32) ^ salt)
}

fn side_weights(mem: [u32; 3]) -> [f64; 3] {
    mem.map(|m| (1.0 / (1.0 + 0.8 * f64::from(m))).max(0.2))
}

/// The tell a threat gives before it can kill: the shift's, scaled by mood, never under the floor or the shift's minimum.
fn tell_for(tell_ms: f64, min_tell: f64, mood: Mood, u: f64) -> f64 {
    let raw = (tell_ms * mood.tell() * (1.0 + TELL_JITTER * (2.0 * u - 1.0))).round();
    TELL_FLOOR_MS.max(min_tell.max(raw))
}

fn step_ms(kind: Kind) -> f64 {
    match kind {
        Kind::Path => 11_000.0,
        Kind::Watched => 9_000.0,
        Kind::Vent => 9_500.0,
        Kind::Winder => 12_000.0,
        Kind::Counter | Kind::Monitor => 0.0,
    }
}

fn rest_ms(kind: Kind) -> f64 {
    match kind {
        Kind::Path | Kind::Winder => 16_000.0,
        Kind::Watched | Kind::Vent => 18_000.0,
        Kind::Monitor => 20_000.0,
        Kind::Counter => 0.0,
    }
}

fn tension_floor_at(t: f64) -> f64 {
    if t < TENSION_FLOOR_FROM {
        return 0.0;
    }
    let k = ((t - TENSION_FLOOR_FROM) / (NIGHT_MS - TENSION_FLOOR_FROM)).clamp(0.0, 1.0);
    TENSION_FLOOR.0 + (TENSION_FLOOR.1 - TENSION_FLOOR.0) * k
}

fn endless_pace(hr: u32) -> f64 {
    (1.0 + 0.04 * f64::from(hr.saturating_sub(2))).min(2.0)
}

fn endless_tell(hr: u32) -> f64 {
    (1900.0 - 45.0 * f64::from(hr.saturating_sub(3))).max(1600.0)
}

fn endless_budget(hr: u32) -> f64 {
    (ENDLESS_BUDGET_MS - 100.0 * f64::from(hr.saturating_sub(10))).max(1400.0)
}

fn endless_gap(hr: u32) -> f64 {
    (46_000.0 - 3000.0 * f64::from(hr)).max(10_000.0)
}

fn endless_floor_at(t: f64) -> f64 {
    if t < HOUR_MS {
        return 0.0;
    }
    RAMP_FLOOR_MAX * (1.0 - (-(t - HOUR_MS) / HOUR_MS / RAMP_FLOOR_SPAN).exp())
}

fn kinds_of(ids: &[usize]) -> Vec<Kind> {
    let mut out: Vec<Kind> = Vec::new();
    for &p in ids {
        let k = POOL[p].kind;
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

fn allowed(have: &[usize], p: usize, d: &Difficulty) -> bool {
    let kind = POOL[p].kind;
    let new_count = have.iter().filter(|&&x| POOL[x].kind.is_new()).count();
    let walker_count = have.iter().filter(|&&x| POOL[x].kind.is_walker()).count();
    !(kind.is_new() && new_count >= d.new_max) && !(kind.is_walker() && walker_count >= MAX_WALKERS)
}

/// The night's roster, drawn from its own generator: 3 to 7 threats of the pool, at least `min_types` kinds.
fn draw_roster(seed: i64, d: &Difficulty, gen: u32) -> Vec<usize> {
    let base = (u64::from((seed as u32) ^ 0x524f_5354) + u64::from(gen) * 0x9e37_79b1) as u32;
    let mut r = NightWatchStream::seeded(i64::from(base));
    let range = if d.endless { RAMP_ROSTER[(gen as usize).min(RAMP_ROSTER.len() - 1)] } else { d.roster };
    let n = d.must.len().max(range.0 + (r.next() * (range.1 - range.0 + 1) as f64).floor() as usize);

    let mut bag: Vec<usize> = (0..POOL.len()).collect();
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
            let kind = POOL[pick[k]].kind;
            if pick.iter().filter(|&&x| POOL[x].kind == kind).count() > 1 {
                at = k;
                break;
            }
        }
        let rest: Vec<usize> = pick.iter().enumerate().filter(|(j, _)| *j != at).map(|(_, &x)| x).collect();
        match bag.iter().copied().find(|&p| !have.contains(&POOL[p].kind) && allowed(&rest, p, d)) {
            Some(add) => pick[at] = add,
            None => break,
        }
    }
    (0..POOL.len()).filter(|p| pick.contains(p)).collect()
}

/// A threat's whole state. `k` is its place in the roster and staggers the first move.
fn new_threat(p: usize, d: &Difficulty, rng: &mut NightWatchStream, toggles: u32, k: usize) -> Threat {
    let mut t = Threat { pool: p, seen_toggles: toggles, ..Threat::default() };
    t.mood = MOODS[(rng.next() * MOODS.len() as f64).floor() as usize];
    t.mood_left = between(MOOD_MS, rng.next());
    t.mood_cue = between(MOOD_CUE_MS, rng.next());
    match POOL[p].kind {
        Kind::Counter => {
            t.grace = jit(COUNTER_GRACE_MS, rng);
            t.span = jit(COUNTER_SPAN_MS, rng);
            t.active = true;
        }
        Kind::Monitor => t.wait = jit(MONITOR_ARRIVE_MS, rng) + k as f64 * MONITOR_STEP_LATE_MS,
        Kind::Winder => {
            t.box_room = Some(BOX_ROOMS[(rng.next() * BOX_ROOMS.len() as f64).floor() as usize]);
            t.span = jit(TUNE_SPAN_MS, rng);
        }
        Kind::Path | Kind::Watched | Kind::Vent => t.wait = jit(d.start_ms, rng) + k as f64 * WALKER_STEP_LATE_MS,
    }
    t
}

/// The spikes of a night: each at a jittered time inside its own slot, never in the first hour, ending before dawn.
fn plan_spikes(rng: &mut NightWatchStream, d: &Difficulty) -> Vec<Spike> {
    let count = d.spikes.0 + (rng.next() * (d.spikes.1 - d.spikes.0 + 1) as f64).floor() as usize;
    let start = HOUR_MS + SPIKE_START_PAD_MS;
    let end_ms = NIGHT_MS - END_CLEAR_MS - SPIKE_LEN_MS.1 - 6000.0;
    let width = (end_ms - start) / count as f64;
    let breather = d.breather.unwrap_or(BREATHER_MS);
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let len = between(SPIKE_LEN_MS, rng.next()).round();
        let room = width - len - breather.1 - 2000.0;
        let at = (start + k as f64 * width + rng.next() * room).round();
        out.push(Spike { at, len });
    }
    out
}

fn first_endless_spike(rng: &mut NightWatchStream) -> Spike {
    let at = (HOUR_MS + SPIKE_START_PAD_MS + rng.next() * 0.8 * HOUR_MS).round();
    let len = between(SPIKE_LEN_MS, rng.next()).round();
    Spike { at, len }
}

impl MixedShift {
    /// Starts a night from `seed` on shift `shift` (0 QUIET, 1 STEADY, 2 RESTLESS, 3 CHAOS, 4 ENDLESS).
    pub fn start(seed: i64, shift: usize) -> Self {
        let diff: &'static Difficulty = &SHIFTS[shift.min(SHIFTS.len() - 1)];
        let mut dir = NightWatchStream::seeded(xor_seed(seed, 0x4449_5243));
        let mut th_rng = NightWatchStream::seeded(xor_seed(seed, 0x5448_5254));
        let mut fx = NightWatchStream::seeded(xor_seed(seed, 0x4658_5f31));
        let roster = draw_roster(seed, diff, 0);
        let plan = if diff.endless { vec![first_endless_spike(&mut dir)] } else { plan_spikes(&mut dir, diff) };
        let beat_left = between(BEAT_LULL_MS, dir.next());
        let feint_left = between(FEINT_LULL_MS, fx.next());
        let mut threats: Vec<Threat> = (0..POOL.len()).map(|p| Threat { pool: p, ..Threat::default() }).collect();
        for (k, &p) in roster.iter().enumerate() {
            threats[p] = new_threat(p, diff, &mut th_rng, 0, k);
        }
        let seal_max = if diff.endless { ENDLESS_SEAL_MAX } else { SEAL_MAX };
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
            tension: 0.3,
            plan,
            spike_idx: 0,
            spike_end: 0.0,
            warned: false,
            feint_left,
            seal: seal_max,
            doors: [Gate::default(); 3],
            vent_seal: VENT_SEAL_MAX,
            vents: [Gate::default(); 3],
            lamp_cool: 0.0,
            wind: false,
            mon: Monitor { up: false, room: Room::Stage, stay: 0.0, toggles: 0 },
            roster,
            threats,
            chaos: ChaosState::new(),
            run: Run { gen: 0, rotate_at: ROTATE_MS, budget: ENDLESS_BUDGET_MS },
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
        if self.elapsed >= NIGHT_MS && !self.diff.endless {
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
            match POOL[p].kind {
                Kind::Counter => self.step_counter(p, dt, &mut ev),
                Kind::Monitor => self.step_monitor(p, dt, &mut ev),
                Kind::Winder => self.step_winder(p, dt, &mut ev),
                Kind::Path | Kind::Watched | Kind::Vent => self.step_walker(p, dt, &mut ev),
            }
        }
        if self.lock.is_none() && self.since_act > 6000.0 {
            self.scare(dt / 60000.0);
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
        } else if !self.doors[ix].shut && self.doors[ix].cool <= 0.0 && self.seal >= SEAL_MIN_TO_SHUT {
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
            } else if self.vents[ix].cool <= 0.0 && self.vent_seal >= VENT_MIN_TO_SEAL {
                self.vents[ix].shut = true;
                self.vents[ix].held = 0.0;
                self.vents[ix].linger = 0.0;
            }
        } else if self.vents[ix].shut {
            self.vents[ix].linger = VENT_LINGER_MS;
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
        self.lamp_cool = VENT_LIGHT_COOL_MS;
        self.vents[ix].flash = VENT_LIGHT_MS;
        let mut seen = Vec::new();
        for &p in &self.roster {
            if POOL[p].kind != Kind::Vent {
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

    pub fn shift_name(&self) -> &'static str {
        self.diff.name
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
                let kind = POOL[p].kind;
                (kind.is_walker() || kind == Kind::Vent) && t.active && !t.armed && t.rooms.get(t.i) == Some(&room)
            })
            .collect()
    }

    /// The hour the player held out to: six at most in the timed shifts, and the full count in ENDLESS.
    pub fn hour_reached(&self) -> u32 {
        if self.diff.endless {
            self.hour_of()
        } else {
            self.hour_of().min(6)
        }
    }

    /// Points for a shift won (the timed shifts only; ENDLESS has no dawn to win).
    pub fn points(&self) -> u32 {
        match self.over {
            Outcome::Win if !self.diff.endless => POINTS_PER_LEVEL,
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
        (self.elapsed / HOUR_MS).floor() as u32
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
        self.diff.breather.unwrap_or(BREATHER_MS)
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
        self.beat.rate() * (0.8 + 0.5 * self.tension) * self.threats[p].mood.speed() * self.live_speed()
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
        self.cool = COOLDOWN_MS;
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
        *m = (*m + 1).min(6);
    }

    fn gate_shut(&self, p: usize) -> bool {
        let Some(side) = self.threats[p].side else { return false };
        if POOL[p].kind == Kind::Vent {
            self.vents[side.ix()].shut
        } else {
            self.doors[side.ix()].shut
        }
    }

    fn apply_floor(&mut self) {
        let floor = if self.diff.endless {
            endless_floor_at(self.elapsed)
        } else if self.elapsed >= TENSION_FLOOR_FROM {
            tension_floor_at(self.elapsed)
        } else {
            0.0
        };
        self.tension = self.tension.max(floor);
    }

    fn wind_jump(&mut self) {
        let Some(p) = self.roster.iter().copied().find(|&p| POOL[p].kind == Kind::Winder) else { return };
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
        if POOL[p].kind == Kind::Vent {
            if self.vent_seal < VENT_RESERVE {
                self.vent_seal = VENT_RESERVE;
            }
        } else if side.is_some() && self.seal < SEAL_RESERVE {
            if self.diff.endless {
                let g = (SEAL_RESERVE - self.seal).min(self.run.budget);
                self.seal += g;
                self.run.budget -= g;
            } else {
                self.seal = SEAL_RESERVE;
            }
        }
        self.scare(-0.15);
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
        if lead < TELL_FLOOR_MS {
            return;
        }
        self.act(ev, vec![Event::Kill { pool: p, lead }]);
        self.over = Outcome::Caught(p);
    }

    /// The threat leaves the gate: answered by the player, or it gave up.
    fn send_away(&mut self, p: usize, how: &'static str, ev: &mut Vec<Event>) {
        let kind = POOL[p].kind;
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
        self.scare(-0.25);
        self.act(ev, vec![Event::Answered { pool: p, how }, Event::Gone { pool: p }]);
    }

    fn step_doors(&mut self, dt: f64, ev: &mut Vec<Event>) {
        let strained = self.chaos.strain > 0.0;
        let hold_ms = if strained { STRAIN_HOLD_MS } else { DOOR_HOLD_MS };
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
                    d.cool = POP_COOL_MS;
                    ev.push(Event::Strain { side });
                }
            } else if self.doors[ix].want && self.doors[ix].cool <= 0.0 && self.seal >= SEAL_MIN_TO_SHUT {
                self.doors[ix].shut = true;
                self.doors[ix].held = 0.0;
                ev.push(Event::DoorShut { side });
            }
            if !self.doors[ix].shut {
                open += 1;
            }
        }
        if open > 0 {
            let base = if self.diff.endless { ENDLESS_SEAL_REGEN } else { SEAL_REGEN };
            let rate = base * if strained { STRAIN_REGEN } else { 1.0 };
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
                if self.vents[ix].held >= VENT_HOLD_MS || self.vent_seal <= 0.0 {
                    let v = &mut self.vents[ix];
                    v.shut = false;
                    v.want = false;
                    v.held = 0.0;
                    v.linger = 0.0;
                    v.cool = VENT_POP_COOL_MS;
                    ev.push(Event::VentStrain { side });
                } else if !self.vents[ix].want && self.vents[ix].linger <= 0.0 {
                    self.vents[ix].shut = false;
                    self.vents[ix].held = 0.0;
                    ev.push(Event::VentOpen { side });
                }
            } else if self.vents[ix].want && self.vents[ix].cool <= 0.0 && self.vent_seal >= VENT_MIN_TO_SEAL {
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
            let gain = VENT_SEAL_REGEN * dt * (open as f64 / 3.0);
            self.vent_seal = (self.vent_seal + gain).min(VENT_SEAL_MAX);
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
                between(BEAT_LULL_MS, u)
            }
            Beat::Build => {
                let u = self.dir.next();
                between(BEAT_BUILD_MS, u)
            }
        };
        ev.push(Event::Beat(beat));
    }

    fn end_spike(&mut self) {
        if self.diff.endless {
            if !self.plan.is_empty() {
                self.plan.remove(0);
            }
            let gap = jitter(endless_gap(self.hour_of()), 0.25, &mut self.dir);
            let at = (self.elapsed + BREATHER_MS.1 + gap).round();
            let len = between(SPIKE_LEN_MS, self.dir.next()).round();
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
                    if !self.warned && self.elapsed >= n.at - SPIKE_WARN_MS {
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
                        if t.active && !t.armed && POOL[p].kind != Kind::Counter {
                            let late = jitter(WALKER_STEP_LATE_MS, JITTER, &mut self.dir);
                            self.threats[p].wait = self.threats[p].wait.min(late);
                        }
                    }
                } else if self.beat_left <= 0.0 {
                    let last = self.elapsed >= TENSION_FLOOR_FROM;
                    let mut again = false;
                    if self.beat == Beat::Build && self.tension > 0.5 {
                        let u = self.dir.next();
                        again = u < if last { 0.8 } else { 0.35 };
                    }
                    let next = if self.beat == Beat::Lull || again { Beat::Build } else { Beat::Lull };
                    self.start_beat(next, ev);
                }
            }
        }
    }

    fn step_feints(&mut self, dt: f64, ev: &mut Vec<Event>) {
        let range = match self.beat {
            Beat::Lull => FEINT_LULL_MS,
            Beat::Build => FEINT_BUILD_MS,
            _ => return,
        };
        self.feint_left -= dt;
        if self.feint_left > 0.0 {
            return;
        }
        if self.lock.is_some() || self.since_act < FEINT_QUIET_MS || self.cool > 0.0 {
            self.feint_left = 1000.0;
            return;
        }
        if self.fx.next() < 0.5 {
            let u = self.fx.next();
            let room = CAM_ROOMS[(u * CAM_ROOMS.len() as f64).floor() as usize];
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
            let left = between(MOOD_MS, self.th_rng.next());
            let t = &mut self.threats[p];
            t.mood = m;
            t.mood_left = left;
            ev.push(Event::Mood { pool: p, mood: m });
        }
        self.threats[p].mood_cue -= dt;
        if self.threats[p].mood_cue <= 0.0 {
            self.threats[p].mood_cue = between(MOOD_CUE_MS, self.th_rng.next());
        }
    }

    fn choose_route(&mut self, p: usize) {
        let kind = POOL[p].kind;
        let bx = self.threats[p].box_room;
        let table: Vec<&'static Route> = match kind {
            Kind::Vent => VENT_ROUTES.iter().collect(),
            Kind::Winder => ROUTES.iter().filter(|r| bx.map_or(false, |b| r.rooms.contains(&b))).collect(),
            _ => ROUTES.iter().collect(),
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
        let all: &'static [Room] = r.rooms;
        let start = match (kind, bx.and_then(|b| all.iter().position(|&x| x == b))) {
            (Kind::Winder, Some(at)) if at + 3 <= all.len() => at,
            _ => 0,
        };
        let t = &mut self.threats[p];
        t.route_side = r.side;
        t.spots = r.spots;
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
        let kind = POOL[p].kind;
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
            && self.threats[p].dwell >= MIN_DWELL_MS
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
            self.scare(-0.1);
            self.act(ev, vec![Event::Turned { pool: p, room }]);
            return;
        }

        if kind == Kind::Path && room == hall && self.time_runs(p) {
            if seen_now {
                self.threats[p].seen += dt;
                if self.threats[p].seen >= CHOKE_MS && self.can_act() {
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
                    self.scare(-0.1);
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
        if t.wait > 0.0 || t.dwell < MIN_DWELL_MS || !self.can_act() {
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
        let kind = POOL[p].kind;
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
            let give_up = t.give_ms >= GIVE_UP_MS;
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
            self.threats[p].c = (c - COUNTER_CALM_DECAY * dt / span).max(0.0);
        }
        if self.threats[p].c > 0.0 {
            self.threats[p].hot = true;
        } else if self.threats[p].hot {
            self.threats[p].hot = false;
            self.threats[p].grace = jit(COUNTER_GRACE_MS, &mut self.th_rng);
            self.threats[p].span = jit(COUNTER_SPAN_MS, &mut self.th_rng);
        }

        if !self.threats[p].armed {
            if self.threats[p].c >= COUNTER_ARM_AT && holding {
                if self.lock.is_none() && self.can_act() && self.grace_ok() {
                    self.arm(p, None);
                    let room = Some(self.mon.room);
                    self.act(ev, vec![Event::Arrive { pool: p, room, side: None }]);
                    self.start_tell(p, false, ev);
                } else {
                    let c = self.threats[p].c;
                    self.threats[p].c = c.min(COUNTER_ARM_AT);
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
            self.threats[p].grace = jit(COUNTER_GRACE_MS, &mut self.th_rng);
            self.threats[p].span = jit(COUNTER_SPAN_MS, &mut self.th_rng);
            self.scare(-0.25);
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
            let idle_max = jit(MONITOR_IDLE_LEAVE_MS, &mut self.th_rng);
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
            if self.threats[p].give_ms >= GIVE_UP_MS && self.can_act() {
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
            if self.threats[p].steps < MONITOR_STEPS_TO_ARM {
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
        if self.threats[p].decay >= MONITOR_DECAY_MS && self.threats[p].steps > 0 {
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
            let boxfast = if self.chaos.boxfast > 0.0 { BOXFAST_FACTOR } else { 1.0 };
            let speed = self.speed_of(p);
            let t = &self.threats[p];
            let next = (t.tune - dt * speed * boxfast / t.span).max(0.0);
            self.threats[p].tune = next;
        }

        if winding {
            let next = (self.threats[p].tune + TUNE_WIND_PER_SEC * dt / 1000.0).min(1.0);
            self.threats[p].tune = next;
            if was && next >= TUNE_RESTART_AT {
                let span = jit(TUNE_SPAN_MS, &mut self.th_rng);
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
                self.threats[p].span = jit(TUNE_SPAN_MS, &mut self.th_rng);
            }
        }

        if !self.threats[p].stopped {
            if self.elapsed < self.diff.grace_ms {
                let t = self.threats[p].tune;
                self.threats[p].tune = t.max(TUNE_GRACE_FLOOR);
            }
            if self.threats[p].tune <= 0.0005 && self.threats[p].wind_ms == 0.0 {
                if self.can_act() && self.grace_ok() && self.time_runs(p) {
                    let wait = jit(CRANKY_MS, &mut self.th_rng);
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
                (CHAOS_HOURS.0..=CHAOS_HOURS.1).contains(&hr)
            } else {
                self.diff.endless && hr >= RAMP_EVENT_FROM && self.dir.next() < RAMP_EVENT_P
            };
            if roll && self.chaos.at < 0.0 {
                self.schedule_event(f64::from(hr) * HOUR_MS);
            }
        }
    }

    fn schedule_event(&mut self, hour_start: f64) {
        let have: Vec<Kind> = self.roster.iter().map(|&p| POOL[p].kind).collect();
        let kinds: Vec<ChaosKind> = CHAOS_ORDER
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
        self.chaos.until = hour_start + HOUR_MS;
        self.chaos.at = (hour_start + 5000.0 + u * (HOUR_MS - 15000.0)).max(self.diff.grace_ms);
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
        let decoy_wait = self.since_act < DECOY_QUIET_MS
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
                t.mood_left = between(MOOD_MS, u3);
                ev.push(Event::Chaos { kind, start: true, room: None });
                ev.push(Event::Mood { pool: p, mood: m });
            }
            ChaosKind::Wake => {
                let found = self
                    .roster
                    .iter()
                    .copied()
                    .find(|&p| POOL[p].kind == Kind::Vent && !self.threats[p].active && !self.threats[p].armed);
                let Some(p) = found else { return };
                let late = jit(2500.0, &mut self.dir);
                self.threats[p].wait = self.threats[p].wait.min(late);
                ev.push(Event::Chaos { kind, start: true, room: None });
            }
            ChaosKind::Decoy => {
                let types: Vec<Kind> = DECOY_KINDS
                    .iter()
                    .copied()
                    .filter(|&k| self.roster.iter().any(|&p| POOL[p].kind == k))
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
                self.cool = self.cool.max(DECOY_QUIET_MS);
                ev.push(Event::Chaos { kind, start: true, room: None });
                ev.push(Event::Decoy { kind: tkind, side });
            }
            ChaosKind::Flicker => {
                let u = self.dir.next();
                let room = CAM_ROOMS[(u * CAM_ROOMS.len() as f64).floor() as usize];
                self.chaos.flicker = FLICKER_MS;
                ev.push(Event::Chaos { kind, start: true, room: Some(room) });
            }
            ChaosKind::Strain => {
                self.chaos.strain = STRAIN_MS;
                ev.push(Event::Chaos { kind, start: true, room: None });
            }
            ChaosKind::Boxfast => {
                self.chaos.boxfast = BOXFAST_MS;
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
        self.run.rotate_at += ROTATE_MS;
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
        assert_eq!(tell_for(1000.0, 0.0, Mood::Hungry, 0.5), TELL_FLOOR_MS);
        assert_eq!(tell_for(1000.0, 3000.0, Mood::Patient, 0.5), 3000.0);
        assert_eq!(tell_for(SHIFTS[1].tell_ms, 0.0, Mood::Patient, 0.5), 1900.0);
    }
}
