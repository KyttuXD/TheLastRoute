//! Dead Air (Godot `game/night_watch/lure`): a radio station after dark. Xivvy, the one real hunter, crawls the
//! building toward the office vents. Three phantoms knock out the audio, cameras and vents now and then, and Solyth
//! and Wynelle push the building's temperature out of range. Port of `lure_logic.gd`: the same numbers and the same
//! draws in the same order, so one seed plays the same shift as the 2D game. Pure: no window, no drawing.
//!
//! Entry point: `DeadAir::new(DeadAirSetup::night(n, seed))` (or `DeadAirSetup::custom`), then `step(dt_ms, watch)`
//! each frame, with the player actions `lure`, `reboot`, `toggle` and `seal`. `call_script` gives Zellia's call.
//!
//! Not ported (art and sound Rust lacks): the station meshes, walkable crawlspaces and office walk (`lure_world.gd`,
//! `lure_office.gd`, `lure_paths.gd`, `lure_map.gd`), the monitor and wall panels (`lure_panel.gd`), the heater and fan
//! cues (`lure_sounds.gd`), and the admin overlay (`lure_admin.gd`). The shared pacer and hour helpers from
//! `night_watch_pacer.gd` and `night_watch_data.gd` live here for now, because framework.rs does not have them yet.

use std::collections::{HashMap, VecDeque};

use crate::data::CallsData;
use crate::framework::{self, CallScript, NightWatchStream};

/// A night is 12 AM to 6 AM (rules/classic.json `night_ms`).
const NIGHT_MS: f64 = 270_000.0;
const HOUR_MS: f64 = NIGHT_MS / 6.0;
/// Points per Custom Night slider step (`night_watch_modes_data.gd` POINTS_PER_LEVEL). Sliders run 0 to 20.
const POINTS_PER_LEVEL: u32 = 10;
const MAX_SLIDER: i32 = 20;
const DEFAULT_FOCUS: f64 = 0.8;
const LURE_FOLLOW: f64 = 0.95;
const NO_PATH: i32 = 999;

const XIVVY_TEMPO: f64 = 0.25;
const PHANTOM_TEMPO: f64 = 0.2;
const PHANTOM_REST_SPREAD: f64 = 0.3;
const PHANTOM_FIRST_SPREAD: f64 = 0.2;
const SAME_SIDE_AGAIN: f64 = 0.25;
const WAKE_STEP_MS: f64 = 8000.0;
const START_TEMP_SPREAD: f64 = 3.0;
const DRIFT_SPREAD: f64 = 0.1;

const PHANTOM_FIRST_MS: f64 = 4000.0;
const PHANTOM_COOL_MS: f64 = 10000.0;
const PHANTOM_SHOW_MS: f64 = 4000.0;
const PHANTOM_PACE: f64 = 0.3;
const LURE_PACE: f64 = 1.8;
const LURE_MS: f64 = 12000.0;
const LURE_COOL_MS: f64 = 7000.0;
const LURE_STRAIN_EVERY: u32 = 2;
const OFFICE_WAIT_MS: f64 = 5200.0;
const RETREAT_MS: f64 = 2000.0;
const REBOOT_MS: f64 = 6000.0;

const SEAL_STRESS: f64 = 1.2;
const BROKEN_STRESS: f64 = 3.0;
const APPLIANCE_STRESS: f64 = 0.9;
const DECAY_STRESS: f64 = 2.0;
const CUSTOM_SEAL_STRESS: f64 = 1.2;
const CUSTOM_BROKEN_STRESS: f64 = 2.8;
const CUSTOM_BROKEN_PER_LEVEL: f64 = 0.02;
const CUSTOM_APPLIANCE_STRESS: f64 = 0.8;
const CUSTOM_DECAY_STRESS: f64 = 2.1;

const CLIMATE_START: f64 = 70.0;
const COLD_TEMP: f64 = 60.0;
const HOT_TEMP: f64 = 100.0;
const DRIFT_PER_SEC: f64 = 0.2;
const DRIFT_PER_HOUR: f64 = 0.12;
const HEATER_PER_SEC: f64 = 0.9;
const FAN_PER_SEC: f64 = 0.9;
const COLD_PER_DEG: f64 = 0.1;
const HEAT_PER_DEG: f64 = 0.2;
const RECOVER_PER_SEC: f64 = 2.0;

const XIVVY_LEVELS: [i32; 6] = [0, 3, 5, 7, 9, 11];
const SOLYTH_LEVELS: [i32; 6] = [0, 0, 0, 0, 0, 12];
const WYNELLE_LEVELS: [i32; 6] = [0, 0, 0, 5, 8, 13];
const FERRYN_LEVELS: [i32; 6] = [2, 3, 5, 7, 9, 0];
const BRISELLE_LEVELS: [i32; 6] = [0, 0, 3, 5, 8, 10];
const MAVRINE_LEVELS: [i32; 6] = [0, 0, 0, 0, 3, 9];

/// A room id, as in `lure-data.js` (the office openings are `officeL` and `officeR`).
pub type Room = &'static str;

const OFFICE_L: Room = "officeL";
const OFFICE_R: Room = "officeR";
const STORAGE: Room = "storage";

/// The eight camera rooms in camera order (`LureData.ROOMS`). The crawlspaces are the two `vent` rooms.
pub const CAM_ROOMS: [Room; 8] = ["lobby", "arcade", "party", "ballpit", "prize", "storage", "ventl", "ventr"];

/// Walkable connections (`lure-data.js` GRAPH, undirected). Each crawlspace ends at an office opening.
const GRAPH: &[(Room, &[Room])] = &[
    ("lobby", &["arcade", "party"]),
    ("arcade", &["lobby", "ballpit"]),
    ("party", &["lobby", "prize", "ballpit"]),
    ("ballpit", &["arcade", "party", "storage"]),
    ("prize", &["party", "storage"]),
    ("storage", &["prize", "ballpit", "ventl", "ventr"]),
    ("ventl", &["storage", "officeL"]),
    ("ventr", &["storage", "officeR"]),
    ("officeL", &["ventl"]),
    ("officeR", &["ventr"]),
];

/// Everyone who can be on the floor. Xivvy hunts; the rest are the climate pair and the phantoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hunter {
    Xivvy,
    Ferryn,
    Briselle,
    Mavrine,
    Solyth,
    Wynelle,
}

impl Hunter {
    /// The cast in `LureData.CAST` order.
    pub const CAST: [Hunter; 6] = [Hunter::Wynelle, Hunter::Xivvy, Hunter::Mavrine, Hunter::Briselle, Hunter::Solyth, Hunter::Ferryn];

    fn table(self) -> &'static [i32; 6] {
        match self {
            Hunter::Xivvy => &XIVVY_LEVELS,
            Hunter::Ferryn => &FERRYN_LEVELS,
            Hunter::Briselle => &BRISELLE_LEVELS,
            Hunter::Mavrine => &MAVRINE_LEVELS,
            Hunter::Solyth => &SOLYTH_LEVELS,
            Hunter::Wynelle => &WYNELLE_LEVELS,
        }
    }
}

/// The three phantoms. Each one breaks one system when she visits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phantom {
    Ferryn,
    Briselle,
    Mavrine,
}

impl Phantom {
    const ALL: [Phantom; 3] = [Phantom::Ferryn, Phantom::Briselle, Phantom::Mavrine];

    fn hunter(self) -> Hunter {
        match self {
            Phantom::Ferryn => Hunter::Ferryn,
            Phantom::Briselle => Hunter::Briselle,
            Phantom::Mavrine => Hunter::Mavrine,
        }
    }

    /// The system her visit always takes (`SYSTEM_OF`).
    fn system(self) -> System {
        match self {
            Phantom::Ferryn => System::Cams,
            Phantom::Briselle => System::Audio,
            Phantom::Mavrine => System::Vent,
        }
    }

    /// What else a visit may take on a night, as (system, chance) (`PHANTOM_EXTRA`).
    fn extras(self, night: usize) -> &'static [(System, f64)] {
        const FERRYN: [&[(System, f64)]; 6] = [
            &[],
            &[],
            &[(System::Audio, 0.3)],
            &[(System::Audio, 0.3), (System::Vent, 0.1)],
            &[(System::Audio, 0.4), (System::Vent, 0.1)],
            &[(System::Audio, 0.45), (System::Vent, 0.35)],
        ];
        const BRISELLE: [&[(System, f64)]; 6] = [
            &[],
            &[],
            &[],
            &[(System::Cams, 0.35)],
            &[(System::Cams, 0.35)],
            &[(System::Cams, 0.45), (System::Vent, 0.3)],
        ];
        const MAVRINE: [&[(System, f64)]; 6] = [
            &[],
            &[],
            &[],
            &[],
            &[(System::Audio, 0.4)],
            &[(System::Audio, 0.45), (System::Cams, 0.4)],
        ];
        match self {
            Phantom::Ferryn => FERRYN[night],
            Phantom::Briselle => BRISELLE[night],
            Phantom::Mavrine => MAVRINE[night],
        }
    }
}

/// The two climate hunters. Solyth heats the office, Wynelle chills it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Climate {
    Solyth = 0,
    Wynelle = 1,
}

impl Climate {
    fn hunter(self) -> Hunter {
        match self {
            Climate::Solyth => Hunter::Solyth,
            Climate::Wynelle => Hunter::Wynelle,
        }
    }
}

/// The five things a phantom or the air can take down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Audio = 0,
    Cams = 1,
    Vent = 2,
    Heater = 3,
    Fan = 4,
}

impl System {
    const ALL: [System; 5] = [System::Audio, System::Cams, System::Vent, System::Heater, System::Fan];
}

/// Which office opening, left or right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left = 0,
    Right = 1,
}

impl Side {
    pub fn office(self) -> Room {
        match self {
            Side::Left => OFFICE_L,
            Side::Right => OFFICE_R,
        }
    }

    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    fn of_office(room: Room) -> Option<Side> {
        match room {
            OFFICE_L => Some(Side::Left),
            OFFICE_R => Some(Side::Right),
            _ => None,
        }
    }
}

/// Why a night ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Caught(Hunter),
}

/// What broke a system: a phantom's visit, or the audio strain from too many lures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokenBy {
    Phantom(Phantom),
    Strain,
}

/// What `step` reports. Matches the 2D events: moved, atOffice, retreat, broken, fixed, phantom, caught and win.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeadAirEvent {
    Moved { room: Room, from: Room },
    AtOffice { room: Room, from: Room },
    Retreat { side: Side },
    Broken { system: System, by: BrokenBy },
    Fixed { system: System },
    PhantomSeen { who: Phantom, room: Room },
    Caught { by: Hunter },
    Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Working,
    Broken,
    Rebooting,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SystemState {
    status: Status,
    t: f64,
}

impl SystemState {
    const WORKING: SystemState = SystemState { status: Status::Working, t: 0.0 };
    const BROKEN: SystemState = SystemState { status: Status::Broken, t: 0.0 };
}

/// The air's stress rates for a game (`air_rates`): the story nights' or the Custom Night's.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Air {
    seal: f64,
    broken: f64,
    appliance: f64,
    decay: f64,
}

fn air_rates(custom: bool, mavrine_level: i32) -> Air {
    if custom {
        Air {
            seal: CUSTOM_SEAL_STRESS,
            broken: CUSTOM_BROKEN_STRESS + CUSTOM_BROKEN_PER_LEVEL * f64::from(mavrine_level.clamp(0, MAX_SLIDER)),
            appliance: CUSTOM_APPLIANCE_STRESS,
            decay: CUSTOM_DECAY_STRESS,
        }
    } else {
        Air { seal: SEAL_STRESS, broken: BROKEN_STRESS, appliance: APPLIANCE_STRESS, decay: DECAY_STRESS }
    }
}

/// Mirrors `NightWatchPacer`: after each move she waits a random time from a window that slides down with her level.
/// Level 0 never moves. Draws only from the stream it is given.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pacer {
    t: f64,
    next: Option<f64>,
}

impl Pacer {
    fn step(&mut self, dt_ms: f64, level: i32, rng: &mut NightWatchStream) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        let next = match self.next {
            Some(wait) => wait,
            None => {
                let wait = draw_move(level, rng);
                self.next = Some(wait);
                wait
            }
        };
        self.t += dt_ms;
        if self.t < next {
            return false;
        }
        self.t = 0.0;
        self.next = Some(draw_move(level, rng));
        true
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.next = None;
    }

    fn skip(&mut self, ms: f64) {
        self.t += ms;
    }
}

#[derive(Clone, Debug, Default)]
struct PhantomAi {
    pacer: Pacer,
    cool: f64,
    tempo: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sighting {
    who: Phantom,
    room: Room,
    t: f64,
}

#[derive(Clone, Debug)]
struct Killer {
    room: Room,
    pacer: Pacer,
    side: Side,
    at_ms: f64,
    target: Option<Room>,
    lure_left: f64,
    tempo: f64,
    /// Steps to the lure target, filled on the first lured move (empty until then).
    dist: HashMap<Room, i32>,
}

/// How a shift is set up: the night, the seeded streams and the Custom Night overrides. `choice_rng` and `var_rng`
/// default to the shift's own stream and a stream derived from it, as in the 2D game.
pub struct DeadAirSetup {
    pub night: i32,
    pub rng: NightWatchStream,
    pub choice_rng: Option<NightWatchStream>,
    pub var_rng: Option<NightWatchStream>,
    pub xivvy_level: Option<i32>,
    pub phantom_levels: Option<HashMap<Hunter, i32>>,
    pub climate_levels: Option<HashMap<Hunter, i32>>,
    pub phantoms: bool,
    pub focus: f64,
    pub infinite_stress: bool,
    pub custom: bool,
}

impl DeadAirSetup {
    /// A story night, 1 to 6, from the shift's seed.
    pub fn night(night: i32, seed: i64) -> Self {
        Self {
            night,
            rng: NightWatchStream::seeded(seed),
            choice_rng: None,
            var_rng: None,
            xivvy_level: None,
            phantom_levels: None,
            climate_levels: None,
            phantoms: true,
            focus: DEFAULT_FOCUS,
            infinite_stress: false,
            custom: false,
        }
    }

    /// A Custom Night: Night 5's tables, with each slider as that hunter's level.
    pub fn custom(seed: i64, levels: HashMap<Hunter, i32>) -> Self {
        let xivvy = levels.get(&Hunter::Xivvy).copied().unwrap_or(0);
        Self {
            xivvy_level: Some(xivvy),
            phantom_levels: Some(levels.clone()),
            climate_levels: Some(levels),
            custom: true,
            ..Self::night(5, seed)
        }
    }
}

/// One shift of Dead Air.
#[derive(Debug)]
pub struct DeadAir {
    n: usize,
    rng: NightWatchStream,
    choice_rng: Option<NightWatchStream>,
    vr: NightWatchStream,
    lvl: i32,
    phantoms_on: bool,
    focus: f64,
    infinite_stress: bool,
    phantom_levels: Option<HashMap<Hunter, i32>>,
    climate: [i32; 2],
    has_climate: bool,
    has_fan: bool,
    air: Air,
    elapsed: f64,
    outcome: Option<Outcome>,
    stress: f64,
    killer: Killer,
    office_dist: [HashMap<Room, i32>; 2],
    systems: [SystemState; 5],
    temp: f64,
    drift_x: f64,
    heater_on: bool,
    fan_on: bool,
    cold: f64,
    heat: f64,
    seals: [bool; 2],
    lure_cool: f64,
    lures: u32,
    pending: Vec<DeadAirEvent>,
    phantom: Option<Sighting>,
    phantom_ai: [PhantomAi; 3],
    /// Admin god mode: whoever would end the night is sent back instead.
    pub god: bool,
    /// Admin: scales every hunter's wait clock (1.0 is the 2D numbers).
    pub hunter_speed: f64,
}

impl DeadAir {
    pub fn new(setup: DeadAirSetup) -> DeadAir {
        let n = (setup.night.clamp(1, 6) - 1) as usize;
        // The variety stream is derived before the night's first draw.
        let vr = NightWatchStream::variety_of(&setup.rng, setup.var_rng);
        let climate = [Climate::Solyth, Climate::Wynelle].map(|c| base_level(c.hunter(), n, setup.climate_levels.as_ref()));
        let has_climate = climate.iter().any(|level| *level > 0);
        let has_fan = climate[Climate::Solyth as usize] > 0;
        let air = air_rates(setup.custom, base_level(Hunter::Mavrine, n, setup.phantom_levels.as_ref()));
        let lvl = setup.xivvy_level.filter(|level| *level >= 0).unwrap_or(XIVVY_LEVELS[n]);
        let mut sim = DeadAir {
            n,
            rng: setup.rng,
            choice_rng: setup.choice_rng,
            vr,
            lvl,
            phantoms_on: setup.phantoms,
            focus: setup.focus,
            infinite_stress: setup.infinite_stress,
            phantom_levels: setup.phantom_levels,
            climate,
            has_climate,
            has_fan,
            air,
            elapsed: 0.0,
            outcome: None,
            stress: 0.0,
            killer: Killer {
                room: "lobby",
                pacer: Pacer::default(),
                side: Side::Left,
                at_ms: 0.0,
                target: None,
                lure_left: 0.0,
                tempo: 1.0,
                dist: HashMap::new(),
            },
            office_dist: [distances_to(OFFICE_L), distances_to(OFFICE_R)],
            systems: [SystemState::WORKING; 5],
            temp: CLIMATE_START,
            drift_x: 1.0,
            heater_on: false,
            fan_on: false,
            cold: 0.0,
            heat: 0.0,
            seals: [false; 2],
            lure_cool: 0.0,
            lures: 0,
            pending: Vec::new(),
            phantom: None,
            phantom_ai: Default::default(),
            god: false,
            hunter_speed: 1.0,
        };

        sim.killer.side = if sim.choice_draw() < 0.5 { Side::Left } else { Side::Right };
        sim.killer.tempo = framework::tempo(&mut sim.vr, XIVVY_TEMPO);
        for who in Phantom::ALL {
            let cool = framework::spread_ms(PHANTOM_FIRST_MS, &mut sim.rng, PHANTOM_FIRST_SPREAD);
            let tempo = framework::tempo(&mut sim.vr, PHANTOM_TEMPO);
            sim.phantom_ai[who as usize] = PhantomAi { pacer: Pacer::default(), cool, tempo };
        }

        // With two or more phantoms on the floor, shuffle which one makes her first visit earlier and which later.
        let awake: Vec<Phantom> = Phantom::ALL.into_iter().filter(|who| sim.phantom_base(*who) > 0).collect();
        if sim.phantoms_on && awake.len() >= 2 && !sim.vr.is_neutral() {
            let count = awake.len();
            let mut slots: Vec<f64> = (0..count).map(|i| (i as f64 - (count as f64 - 1.0) / 2.0) * WAKE_STEP_MS).collect();
            for i in (1..count).rev() {
                let j = framework::pick_index(&mut sim.vr, i + 1);
                slots.swap(i, j);
            }
            for (who, slot) in awake.iter().zip(&slots) {
                sim.phantom_ai[*who as usize].pacer.skip(-slot * PHANTOM_PACE);
            }
        }

        if sim.has_climate {
            sim.temp = CLIMATE_START + (2.0 * sim.vr.next() - 1.0) * START_TEMP_SPREAD;
            sim.drift_x = framework::tempo(&mut sim.vr, DRIFT_SPREAD);
        }
        sim
    }

    /// Advance by `dt_ms`. `watch` is the room the monitor shows; a hunter there does not move.
    pub fn step(&mut self, dt_ms: f64, watch: Option<Room>) -> Vec<DeadAirEvent> {
        let mut ev = Vec::new();
        if self.outcome.is_some() {
            return ev;
        }
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        let hour = hour_index(self.elapsed);
        ev.append(&mut self.pending);
        self.lure_cool = (self.lure_cool - dt_ms).max(0.0);
        if self.killer.lure_left > 0.0 {
            self.killer.lure_left -= dt_ms;
            if self.killer.lure_left <= 0.0 {
                self.killer.target = None;
                self.killer.dist.clear();
            }
        }
        self.finish_reboots(dt_ms, &mut ev);
        if self.phantoms_on {
            self.phantoms_act(dt_ms, hour, &mut ev);
        }
        if self.has_climate && self.climate_tick(dt, hour, &mut ev) {
            return ev;
        }
        if self.air_tick(dt, &mut ev) {
            return ev;
        }
        if self.hunter_tick(dt_ms, hour, watch, &mut ev) {
            return ev;
        }
        if self.elapsed >= NIGHT_MS {
            self.outcome = Some(Outcome::Win);
            ev.push(DeadAirEvent::Win);
        }
        ev
    }

    /// Zero draws: the lure needs a working audio system, a short cooldown and a non-vent room.
    pub fn lure(&mut self, room: Room) -> bool {
        if self.outcome.is_some() || !self.online(System::Audio) || self.lure_cool > 0.0 || !lureable(room) {
            return false;
        }
        self.killer.target = Some(room);
        self.killer.dist.clear();
        self.killer.lure_left = LURE_MS;
        self.lure_cool = LURE_COOL_MS;
        self.lures += 1;
        if self.lures % LURE_STRAIN_EVERY == 0 {
            self.systems[System::Audio as usize] = SystemState::BROKEN;
            self.pending.push(DeadAirEvent::Broken { system: System::Audio, by: BrokenBy::Strain });
        }
        true
    }

    /// Start rebooting a broken system (six seconds offline). The fan needs the climate pair to have a Solyth.
    pub fn reboot(&mut self, system: System) -> bool {
        if self.outcome.is_some() || self.systems[system as usize].status != Status::Broken {
            return false;
        }
        if system == System::Fan && !self.has_fan {
            return false;
        }
        self.systems[system as usize] = SystemState { status: Status::Rebooting, t: 0.0 };
        true
    }

    /// Switch the heater or the fan on or off. Turning one on turns the other off.
    pub fn toggle(&mut self, appliance: System) -> bool {
        if self.outcome.is_some() || !self.has_climate || !matches!(appliance, System::Heater | System::Fan) {
            return false;
        }
        if (appliance == System::Fan && !self.has_fan) || !self.online(appliance) {
            return false;
        }
        if appliance == System::Heater {
            self.heater_on = !self.heater_on;
            if self.heater_on {
                self.fan_on = false;
            }
        } else {
            self.fan_on = !self.fan_on;
            if self.fan_on {
                self.heater_on = false;
            }
        }
        true
    }

    /// Close (`Some(true)`), open (`Some(false)`) or flip (`None`) a vent opening.
    pub fn seal(&mut self, side: Side, on: Option<bool>) -> bool {
        if self.outcome.is_some() {
            return false;
        }
        let slot = &mut self.seals[side as usize];
        *slot = on.unwrap_or(!*slot);
        true
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed
    }

    pub fn stress(&self) -> f64 {
        self.stress
    }

    pub fn temperature(&self) -> f64 {
        self.temp
    }

    pub fn cold(&self) -> f64 {
        self.cold
    }

    pub fn heat(&self) -> f64 {
        self.heat
    }

    pub fn online(&self, system: System) -> bool {
        self.systems[system as usize].status == Status::Working
    }

    pub fn seal_closed(&self, side: Side) -> bool {
        self.seals[side as usize]
    }

    pub fn heater_on(&self) -> bool {
        self.heater_on
    }

    pub fn fan_on(&self) -> bool {
        self.fan_on
    }

    pub fn killer_room(&self) -> Room {
        self.killer.room
    }

    pub fn lure_target(&self) -> Option<Room> {
        self.killer.target
    }

    /// The phantom on a camera right now, and its room.
    pub fn phantom(&self) -> Option<(Phantom, Room)> {
        self.phantom.map(|sighting| (sighting.who, sighting.room))
    }

    /// The AI level this shift runs a hunter at, with the night's step-ups.
    pub fn level_of(&self, hunter: Hunter) -> i32 {
        match hunter {
            Hunter::Xivvy => self.lvl,
            Hunter::Solyth => self.climate[Climate::Solyth as usize],
            Hunter::Wynelle => self.climate[Climate::Wynelle as usize],
            Hunter::Ferryn => self.phantom_base(Phantom::Ferryn),
            Hunter::Briselle => self.phantom_base(Phantom::Briselle),
            Hunter::Mavrine => self.phantom_base(Phantom::Mavrine),
        }
    }

    fn phantom_base(&self, who: Phantom) -> i32 {
        base_level(who.hunter(), self.n, self.phantom_levels.as_ref())
    }

    /// One draw from the choice stream (the shift's own stream when no separate one was given).
    fn choice_draw(&mut self) -> f64 {
        match self.choice_rng.as_mut() {
            Some(stream) => stream.next(),
            None => self.rng.next(),
        }
    }

    fn break_system(&mut self, system: System) {
        self.systems[system as usize] = SystemState::BROKEN;
        match system {
            System::Heater => self.heater_on = false,
            System::Fan => self.fan_on = false,
            System::Audio | System::Cams | System::Vent => {}
        }
    }

    /// Ends the night, or in god mode sends whoever it was back. Always ends the tick.
    fn finish(&mut self, ev: &mut Vec<DeadAirEvent>, by: Hunter) -> bool {
        if self.god {
            match by {
                Hunter::Xivvy => {
                    self.killer.room = STORAGE;
                    self.killer.at_ms = 0.0;
                    self.killer.pacer.reset();
                }
                Hunter::Mavrine => self.stress = 40.0,
                Hunter::Wynelle => self.cold = 0.0,
                Hunter::Solyth => self.heat = 0.0,
                Hunter::Ferryn | Hunter::Briselle => {}
            }
            return true;
        }
        self.outcome = Some(Outcome::Caught(by));
        ev.push(DeadAirEvent::Caught { by });
        true
    }

    fn finish_reboots(&mut self, dt_ms: f64, ev: &mut Vec<DeadAirEvent>) {
        for system in System::ALL {
            let state = &mut self.systems[system as usize];
            if state.status == Status::Rebooting {
                state.t += dt_ms;
                if state.t >= REBOOT_MS {
                    *state = SystemState::WORKING;
                    ev.push(DeadAirEvent::Fixed { system });
                }
            }
        }
    }

    fn phantoms_act(&mut self, dt_ms: f64, hour: usize, ev: &mut Vec<DeadAirEvent>) {
        if let Some(sighting) = self.phantom.as_mut() {
            sighting.t += dt_ms;
            if sighting.t >= PHANTOM_SHOW_MS {
                self.phantom = None;
            }
        }
        for who in Phantom::ALL {
            let i = who as usize;
            let cool = (self.phantom_ai[i].cool - dt_ms).max(0.0);
            self.phantom_ai[i].cool = cool;
            if cool > 0.0 {
                continue;
            }
            let level = effective_level(self.phantom_base(who), hour);
            let rate = dt_ms * PHANTOM_PACE * self.phantom_ai[i].tempo * self.hunter_speed;
            if self.phantom_ai[i].pacer.step(rate, level, &mut self.rng) {
                self.phantom_visit(who, ev);
            }
        }
    }

    fn phantom_visit(&mut self, who: Phantom, ev: &mut Vec<DeadAirEvent>) {
        let i = who as usize;
        let cool_ms = (PHANTOM_COOL_MS - self.n as f64 * 800.0).max(5000.0);
        self.phantom_ai[i].cool = framework::spread_ms(cool_ms, &mut self.rng, PHANTOM_REST_SPREAD);
        self.phantom_ai[i].tempo = framework::tempo(&mut self.vr, PHANTOM_TEMPO);
        let cam_count = CAM_ROOMS.len();
        let room = CAM_ROOMS[((self.rng.next() * cam_count as f64).floor() as usize).min(cam_count - 1)];
        self.phantom = Some(Sighting { who, room, t: 0.0 });

        let mut targets = vec![who.system()];
        if who == Phantom::Mavrine && self.has_climate {
            if self.climate[Climate::Solyth as usize] > 0 && self.climate[Climate::Wynelle as usize] > 0 {
                targets.extend([System::Heater, System::Fan]);
            } else {
                targets.push(if self.fan_on { System::Fan } else { System::Heater });
            }
        }
        for &(system, chance) in who.extras(self.n) {
            if self.choice_draw() < chance && !targets.contains(&system) {
                targets.push(system);
            }
        }
        for system in targets {
            if self.online(system) {
                self.break_system(system);
                ev.push(DeadAirEvent::Broken { system, by: BrokenBy::Phantom(who) });
            }
        }
        ev.push(DeadAirEvent::PhantomSeen { who, room });
    }

    /// Returns true when the tick ends (someone caught her).
    fn climate_tick(&mut self, dt: f64, hour: usize, ev: &mut Vec<DeadAirEvent>) -> bool {
        let solyth = self.climate[Climate::Solyth as usize];
        let wynelle = self.climate[Climate::Wynelle as usize];
        let dir = if wynelle > 0 { -1.0 } else { 1.0 };
        let drift = DRIFT_PER_SEC * self.drift_x * (1.0 + hour as f64 * DRIFT_PER_HOUR) * dir;
        let mut push = 0.0;
        if self.heater_on && self.online(System::Heater) {
            push += HEATER_PER_SEC;
        }
        if self.fan_on && self.online(System::Fan) {
            push -= FAN_PER_SEC;
        }
        self.temp = (self.temp + (drift + push) * dt).clamp(20.0, 130.0);

        if wynelle > 0 && self.temp < COLD_TEMP {
            self.cold = (self.cold + (COLD_TEMP - self.temp) * COLD_PER_DEG * climate_scale(wynelle) * dt).min(100.0);
        } else {
            self.cold = (self.cold - RECOVER_PER_SEC * dt).max(0.0);
        }
        if solyth > 0 && self.temp >= HOT_TEMP - 5.0 {
            self.heat = (self.heat + (self.temp - (HOT_TEMP - 5.0)) * HEAT_PER_DEG * climate_scale(solyth) * dt).min(100.0);
        } else {
            self.heat = (self.heat - RECOVER_PER_SEC * dt).max(0.0);
        }

        if self.cold >= 100.0 {
            return self.finish(ev, Hunter::Wynelle);
        }
        if self.heat >= 100.0 {
            return self.finish(ev, Hunter::Solyth);
        }
        false
    }

    /// Stress builds from closed vents, a broken vent and running appliances, and decays otherwise.
    /// Returns true when the tick ends (the air gave out).
    fn air_tick(&mut self, dt: f64, ev: &mut Vec<DeadAirEvent>) -> bool {
        let sealed = self.seals.iter().filter(|closed| **closed).count() as f64;
        let running = usize::from(self.heater_on && self.online(System::Heater)) + usize::from(self.fan_on && self.online(System::Fan));
        let vent_fault = if self.online(System::Vent) { 0.0 } else { self.air.broken };
        let bad = sealed * self.air.seal + vent_fault + running as f64 * self.air.appliance;
        let change = if bad > 0.0 { bad } else { -self.air.decay };
        self.stress = (self.stress + change * dt).clamp(0.0, 100.0);
        if self.stress >= 100.0 && !self.infinite_stress {
            return self.finish(ev, Hunter::Mavrine);
        }
        false
    }

    /// Xivvy waits at an open office vent (or is turned back by a seal) and otherwise walks toward it, or toward a lure.
    /// Returns true when the tick ends (she got in).
    fn hunter_tick(&mut self, dt_ms: f64, hour: usize, watch: Option<Room>, ev: &mut Vec<DeadAirEvent>) -> bool {
        let room = self.killer.room;
        if let Some(side) = Side::of_office(room) {
            self.killer.at_ms += dt_ms;
            if self.seals[side as usize] {
                if self.killer.at_ms >= RETREAT_MS {
                    self.killer.room = STORAGE;
                    self.killer.at_ms = 0.0;
                    self.killer.pacer.reset();
                    let same = framework::chance(&mut self.vr, SAME_SIDE_AGAIN);
                    self.killer.side = if same { side } else { side.other() };
                    self.killer.tempo = framework::tempo(&mut self.vr, XIVVY_TEMPO);
                    ev.push(DeadAirEvent::Retreat { side });
                }
            } else if self.killer.at_ms >= OFFICE_WAIT_MS {
                return self.finish(ev, Hunter::Xivvy);
            }
            return false;
        }

        let pace = if self.killer.lure_left > 0.0 { LURE_PACE } else { 1.0 };
        let level = effective_level(self.lvl, hour);
        if watch != Some(room) {
            let rate = dt_ms * pace * self.killer.tempo * self.hunter_speed;
            if self.killer.pacer.step(rate, level, &mut self.rng) {
                self.killer.tempo = framework::tempo(&mut self.vr, XIVVY_TEMPO);
                let from = self.killer.room;
                let to = self.next_room();
                self.killer.room = to;
                if to != from {
                    let to_office = Side::of_office(to).is_some();
                    if to_office {
                        ev.push(DeadAirEvent::AtOffice { room: to, from });
                        self.killer.at_ms = 0.0;
                    } else {
                        ev.push(DeadAirEvent::Moved { room: to, from });
                    }
                    if self.killer.lure_left > 0.0 && self.killer.target == Some(to) {
                        self.killer.lure_left = 0.0;
                        self.killer.target = None;
                        self.killer.dist.clear();
                    }
                }
            }
        }
        false
    }

    /// The next room on the way to her target: the lure while it lasts, otherwise her side's vent. She prefers steps
    /// that get her closer, with a focus chance (a lure follows more closely).
    fn next_room(&mut self) -> Room {
        let room = self.killer.room;
        let side = self.killer.side;
        let lured_target = if self.killer.lure_left > 0.0 { self.killer.target } else { None };
        let lured = lured_target.is_some();
        let target = lured_target.unwrap_or(side.office());
        if lured && self.killer.dist.is_empty() {
            self.killer.dist = distances_to(target);
        }

        let around: Vec<Room> = neighbours(room)
            .iter()
            .copied()
            .filter(|next| match *next {
                OFFICE_L => !self.seals[Side::Left as usize] && target == OFFICE_L,
                OFFICE_R => !self.seals[Side::Right as usize] && target == OFFICE_R,
                _ => true,
            })
            .collect();
        let d = if lured { &self.killer.dist } else { &self.office_dist[side as usize] };
        let here = dist_of(d, room);
        let closer: Vec<Room> = around.iter().copied().filter(|next| dist_of(d, next) < here).collect();
        let follow = if self.killer.lure_left > 0.0 { LURE_FOLLOW } else { self.focus };
        let pool = if !closer.is_empty() && self.choice_draw() < follow { closer } else { around };

        if pool.is_empty() {
            return room;
        }
        let u = self.choice_draw();
        pool[((u * pool.len() as f64).floor() as usize).min(pool.len() - 1)]
    }
}

/// The Dead Air call for a level (Zellia's lines, `calls/lure.json`). Custom Nights get no call.
pub fn call_script(calls: &CallsData, level: i32, custom: bool) -> CallScript {
    if custom {
        CallScript::new(Vec::new(), 6.0)
    } else {
        CallScript::for_level(calls, "lure", level)
    }
}

/// Who is on the floor on a night (0-based), in cast order (`LureLogic.active_hunters`).
pub fn active_hunters(night_index: usize) -> Vec<Hunter> {
    Hunter::CAST.into_iter().filter(|hunter| hunter.table()[night_index.min(5)] > 0).collect()
}

/// A Custom Night's score: the sum of the sliders (each 0 to 20) times 10 (`NightWatchMode.record_win`).
pub fn custom_score(levels: &HashMap<Hunter, i32>) -> u32 {
    levels.values().map(|level| (*level).clamp(0, MAX_SLIDER) as u32).sum::<u32>() * POINTS_PER_LEVEL
}

fn lureable(room: Room) -> bool {
    CAM_ROOMS.contains(&room) && !room.starts_with("vent")
}

fn base_level(hunter: Hunter, night: usize, overrides: Option<&HashMap<Hunter, i32>>) -> i32 {
    overrides.and_then(|levels| levels.get(&hunter).copied()).unwrap_or(hunter.table()[night])
}

fn climate_scale(level: i32) -> f64 {
    (f64::from(level) / 10.0).max(0.4)
}

/// `NightWatchData.hour_index`: the hour (0 to 5) the shift is in.
fn hour_index(elapsed_ms: f64) -> usize {
    ((elapsed_ms / HOUR_MS).floor() as usize).min(5)
}

/// `NightWatchData.effective_level`: AI steps up at 3 AM and again at 4 AM. A hunter at level 0 never wakes.
fn effective_level(base: i32, hour: usize) -> i32 {
    let bump = if hour >= 4 {
        2
    } else if hour >= 3 {
        1
    } else {
        0
    };
    if base > 0 {
        base + bump
    } else {
        0
    }
}

fn draw_move(level: i32, rng: &mut NightWatchStream) -> f64 {
    let (lo, hi) = move_window(level);
    lo + rng.next() * (hi - lo)
}

/// `NightWatchData.move_window`: the wait between a hunter's moves, in ms, as (low, high).
fn move_window(level: i32) -> (f64, f64) {
    let lo = (13000.0 - f64::from(level) * 500.0).max(2500.0);
    (lo, lo + 3000.0)
}

fn neighbours(room: Room) -> &'static [Room] {
    for (name, next) in GRAPH {
        if *name == room {
            return *next;
        }
    }
    &[]
}

/// Breadth-first steps from every room to `target` (`distances_to`).
fn distances_to(target: Room) -> HashMap<Room, i32> {
    let mut dist = HashMap::from([(target, 0)]);
    let mut queue = VecDeque::from([target]);
    while let Some(room) = queue.pop_front() {
        let steps = dist[&room];
        for &next in neighbours(room) {
            if !dist.contains_key(&next) {
                dist.insert(next, steps + 1);
                queue.push_back(next);
            }
        }
    }
    dist
}

fn dist_of(dist: &HashMap<Room, i32>, room: Room) -> i32 {
    dist.get(&room).copied().unwrap_or(NO_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_audio_system_refuses_the_lure_until_it_is_rebooted() {
        let mut sim = DeadAir::new(DeadAirSetup::night(2, 7));
        sim.systems[System::Audio as usize] = SystemState::BROKEN;
        assert!(!sim.lure("lobby"));
        assert!(sim.reboot(System::Audio));
        let events = sim.step(REBOOT_MS, None);
        assert!(events.contains(&DeadAirEvent::Fixed { system: System::Audio }));
        assert!(sim.lure("lobby"));
    }
}
