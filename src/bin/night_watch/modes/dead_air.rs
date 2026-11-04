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
use std::sync::OnceLock;

use serde_json::Value;

use crate::data::{self, CallsData};
use crate::framework::{self, CallScript, NightWatchStream};

/// Steps value for a room with no path to the target.
const NO_PATH: i32 = 999;

/// A room id, as in `lure-data.js` (the office openings are `officeL` and `officeR`).
pub type Room = &'static str;

/// Everything tunable about Dead Air, read once from `data/{rules,cast,buildings}/dead_air.json`.
#[derive(Debug)]
struct Cfg {
    night_ms: f64,
    hour_ms: f64,
    last_hour: usize,
    #[cfg(test)]
    points_per_level: u32,
    max_slider: i32,
    default_focus: f64,
    lure_follow: f64,
    // variety
    xivvy_tempo: f64,
    phantom_tempo: f64,
    phantom_rest_spread: f64,
    phantom_first_spread: f64,
    same_side_again: f64,
    wake_step_ms: f64,
    start_temp_spread: f64,
    drift_spread: f64,
    // timing
    phantom_first_ms: f64,
    phantom_cool_ms: f64,
    phantom_cool_per_night_ms: f64,
    phantom_cool_min_ms: f64,
    phantom_show_ms: f64,
    phantom_pace: f64,
    lure_pace: f64,
    lure_ms: f64,
    lure_cool_ms: f64,
    lure_strain_every: u32,
    office_wait_ms: f64,
    retreat_ms: f64,
    reboot_ms: f64,
    // pacing
    move_base_ms: f64,
    move_per_level_ms: f64,
    move_min_ms: f64,
    move_spread_ms: f64,
    level_bump_by_hour: Vec<i32>,
    // air
    stress_max: f64,
    god_stress_reset: f64,
    story_air: Air,
    custom_air: Air,
    custom_broken_per_level: f64,
    climate: Weather,
    // cast
    #[cfg(test)]
    cast_order: Vec<Hunter>,
    levels: [[i32; 6]; 6],
    phantom_systems: [System; 3],
    phantom_extras: [Vec<Vec<(System, f64)>>; 3],
    // building
    start_room: Room,
    retreat_room: Room,
    office_l: Room,
    office_r: Room,
    cam_rooms: Vec<Room>,
    crawl_rooms: Vec<Room>,
    graph: Vec<(Room, Vec<Room>)>,
}

/// The temperature model's numbers (`rules.climate`).
#[derive(Debug)]
struct Weather {
    start: f64,
    cold: f64,
    hot: f64,
    hot_margin: f64,
    min: f64,
    max: f64,
    meter_max: f64,
    drift_per_sec: f64,
    drift_per_hour: f64,
    heater_per_sec: f64,
    fan_per_sec: f64,
    cold_per_deg: f64,
    heat_per_deg: f64,
    recover_per_sec: f64,
    scale_divisor: f64,
    scale_min: f64,
}

fn system_id(id: &str) -> System {
    match id {
        "audio" => System::Audio,
        "cams" => System::Cams,
        "vent" => System::Vent,
        "heater" => System::Heater,
        "fan" => System::Fan,
        other => panic!("night_watch data for mode 'dead_air': unknown system '{other}'"),
    }
}

fn room_list(building: &'static Value, path: &str) -> Vec<Room> {
    data::list(building, path)
        .iter()
        .map(|v| v.as_str().unwrap_or_else(|| panic!("night_watch data for mode 'dead_air': '{path}' holds a non-string")))
        .collect()
}

impl Cfg {
    fn load() -> Cfg {
        let (r, c, b) = (data::rules("dead_air"), data::cast("dead_air"), data::building("dead_air"));
        let n = |path: &str| data::num(r, path);
        let air = |path: &str| Air {
            seal: n(&format!("{path}.seal")),
            broken: n(&format!("{path}.broken")),
            appliance: n(&format!("{path}.appliance")),
            decay: n(&format!("{path}.decay")),
        };
        let weather = Weather {
            start: n("climate.start"),
            cold: n("climate.cold"),
            hot: n("climate.hot"),
            hot_margin: n("climate.hot_margin"),
            min: n("climate.min"),
            max: n("climate.max"),
            meter_max: n("climate.meter_max"),
            drift_per_sec: n("climate.drift_per_sec"),
            drift_per_hour: n("climate.drift_per_hour"),
            heater_per_sec: n("climate.heater_per_sec"),
            fan_per_sec: n("climate.fan_per_sec"),
            cold_per_deg: n("climate.cold_per_deg"),
            heat_per_deg: n("climate.heat_per_deg"),
            recover_per_sec: n("climate.recover_per_sec"),
            scale_divisor: n("climate.scale_divisor"),
            scale_min: n("climate.scale_min"),
        };
        let mut levels = [[0; 6]; 6];
        for hunter in Hunter::ALL {
            for (night, slot) in levels[hunter as usize].iter_mut().enumerate() {
                *slot = data::int(c, &format!("hunters.{}.levels.{night}", hunter.id())) as i32;
            }
        }
        let phantom_systems = Phantom::ALL.map(|who| system_id(data::text(c, &format!("phantoms.{}.system", who.hunter().id()))));
        let phantom_extras = Phantom::ALL.map(|who| {
            let id = who.hunter().id();
            (0..6)
                .map(|night| {
                    data::list(c, &format!("phantoms.{id}.extras.{night}"))
                        .iter()
                        .map(|e| (system_id(data::text(e, "system")), data::num(e, "chance")))
                        .collect()
                })
                .collect()
        });
        let graph = data::list(b, "graph")
            .iter()
            .map(|row| {
                let next = data::list(row, "next")
                    .iter()
                    .map(|v| v.as_str().expect("night_watch data for mode 'dead_air': graph.next holds a non-string"))
                    .collect();
                (data::text(row, "room"), next)
            })
            .collect();
        let night_ms = n("night_ms");
        let hours = n("hours");
        Cfg {
            night_ms,
            hour_ms: night_ms / hours,
            last_hour: hours as usize - 1,
            #[cfg(test)]
            points_per_level: n("points_per_level") as u32,
            max_slider: n("max_slider") as i32,
            default_focus: n("default_focus"),
            lure_follow: n("lure_follow"),
            xivvy_tempo: n("variety.xivvy_tempo"),
            phantom_tempo: n("variety.phantom_tempo"),
            phantom_rest_spread: n("variety.phantom_rest_spread"),
            phantom_first_spread: n("variety.phantom_first_spread"),
            same_side_again: n("variety.same_side_again"),
            wake_step_ms: n("variety.wake_step_ms"),
            start_temp_spread: n("variety.start_temp_spread"),
            drift_spread: n("variety.drift_spread"),
            phantom_first_ms: n("timing.phantom_first_ms"),
            phantom_cool_ms: n("timing.phantom_cool_ms"),
            phantom_cool_per_night_ms: n("timing.phantom_cool_per_night_ms"),
            phantom_cool_min_ms: n("timing.phantom_cool_min_ms"),
            phantom_show_ms: n("timing.phantom_show_ms"),
            phantom_pace: n("timing.phantom_pace"),
            lure_pace: n("timing.lure_pace"),
            lure_ms: n("timing.lure_ms"),
            lure_cool_ms: n("timing.lure_cool_ms"),
            lure_strain_every: n("timing.lure_strain_every") as u32,
            office_wait_ms: n("timing.office_wait_ms"),
            retreat_ms: n("timing.retreat_ms"),
            reboot_ms: n("timing.reboot_ms"),
            move_base_ms: n("pacing.move_base_ms"),
            move_per_level_ms: n("pacing.move_per_level_ms"),
            move_min_ms: n("pacing.move_min_ms"),
            move_spread_ms: n("pacing.move_spread_ms"),
            level_bump_by_hour: (0..hours as usize).map(|hour| data::int(r, &format!("pacing.level_bump_by_hour.{hour}")) as i32).collect(),
            stress_max: n("air.stress_max"),
            god_stress_reset: n("air.god_stress_reset"),
            story_air: air("air.story"),
            custom_air: air("air.custom"),
            custom_broken_per_level: n("air.custom.broken_per_level"),
            climate: weather,
            #[cfg(test)]
            cast_order: data::list(c, "order").iter().map(|v| Hunter::from_id(v.as_str().unwrap_or_default())).collect(),
            levels,
            phantom_systems,
            phantom_extras,
            start_room: data::text(b, "start_room"),
            retreat_room: data::text(b, "retreat_room"),
            office_l: data::text(b, "office_left"),
            office_r: data::text(b, "office_right"),
            cam_rooms: room_list(b, "cam_rooms"),
            crawl_rooms: room_list(b, "crawl_rooms"),
            graph,
        }
    }

    fn neighbours(&self, room: Room) -> &[Room] {
        self.graph.iter().find(|(name, _)| *name == room).map_or(&[], |(_, next)| next.as_slice())
    }

    /// Breadth-first steps from every room to `target` (`distances_to`).
    fn distances_to(&self, target: Room) -> HashMap<Room, i32> {
        let mut dist = HashMap::from([(target, 0)]);
        let mut queue = VecDeque::from([target]);
        while let Some(room) = queue.pop_front() {
            let steps = dist[&room];
            for &next in self.neighbours(room) {
                if !dist.contains_key(&next) {
                    dist.insert(next, steps + 1);
                    queue.push_back(next);
                }
            }
        }
        dist
    }

    fn lureable(&self, room: Room) -> bool {
        self.cam_rooms.contains(&room) && !self.crawl_rooms.contains(&room)
    }

    fn side_of_office(&self, room: Room) -> Option<Side> {
        if room == self.office_l {
            Some(Side::Left)
        } else if room == self.office_r {
            Some(Side::Right)
        } else {
            None
        }
    }

    fn office(&self, side: Side) -> Room {
        match side {
            Side::Left => self.office_l,
            Side::Right => self.office_r,
        }
    }

    /// `NightWatchData.hour_index`: the hour the shift is in.
    fn hour_index(&self, elapsed_ms: f64) -> usize {
        ((elapsed_ms / self.hour_ms).floor() as usize).min(self.last_hour)
    }

    /// `NightWatchData.effective_level`: AI steps up late in the night. A hunter at level 0 never wakes.
    fn effective_level(&self, base: i32, hour: usize) -> i32 {
        if base > 0 {
            base + self.level_bump_by_hour[hour.min(self.last_hour)]
        } else {
            0
        }
    }

    /// `NightWatchData.move_window`: the wait between a hunter's moves, in ms, as (low, high).
    fn move_window(&self, level: i32) -> (f64, f64) {
        let lo = (self.move_base_ms - f64::from(level) * self.move_per_level_ms).max(self.move_min_ms);
        (lo, lo + self.move_spread_ms)
    }

    fn draw_move(&self, level: i32, rng: &mut NightWatchStream) -> f64 {
        let (lo, hi) = self.move_window(level);
        lo + rng.next() * (hi - lo)
    }

    fn climate_scale(&self, level: i32) -> f64 {
        (f64::from(level) / self.climate.scale_divisor).max(self.climate.scale_min)
    }
}

fn cfg() -> &'static Cfg {
    static CFG: OnceLock<Cfg> = OnceLock::new();
    CFG.get_or_init(Cfg::load)
}

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
    /// Every hunter, indexed by `hunter as usize` (the level tables follow this order).
    const ALL: [Hunter; 6] = [Hunter::Xivvy, Hunter::Ferryn, Hunter::Briselle, Hunter::Mavrine, Hunter::Solyth, Hunter::Wynelle];

    /// The id used in the cast file.
    fn id(self) -> &'static str {
        match self {
            Hunter::Xivvy => "xivvy",
            Hunter::Ferryn => "ferryn",
            Hunter::Briselle => "briselle",
            Hunter::Mavrine => "mavrine",
            Hunter::Solyth => "solyth",
            Hunter::Wynelle => "wynelle",
        }
    }

    pub fn from_id(id: &str) -> Hunter {
        Hunter::ALL.into_iter().find(|hunter| hunter.id() == id).unwrap_or_else(|| panic!("night_watch data for mode 'dead_air': unknown hunter '{id}'"))
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
        cfg().office(self)
    }

    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
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

fn air_rates(cfg: &Cfg, custom: bool, mavrine_level: i32) -> Air {
    if custom {
        let custom = cfg.custom_air;
        Air { broken: custom.broken + cfg.custom_broken_per_level * f64::from(mavrine_level.clamp(0, cfg.max_slider)), ..custom }
    } else {
        cfg.story_air
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
    fn step(&mut self, cfg: &Cfg, dt_ms: f64, level: i32, rng: &mut NightWatchStream) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        let next = match self.next {
            Some(wait) => wait,
            None => {
                let wait = cfg.draw_move(level, rng);
                self.next = Some(wait);
                wait
            }
        };
        self.t += dt_ms;
        if self.t < next {
            return false;
        }
        self.t = 0.0;
        self.next = Some(cfg.draw_move(level, rng));
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
            focus: cfg().default_focus,
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
    cfg: &'static Cfg,
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
        let cfg = cfg();
        let n = (setup.night.clamp(1, 6) - 1) as usize;
        // The variety stream is derived before the night's first draw.
        let vr = NightWatchStream::variety_of(&setup.rng, setup.var_rng);
        let climate = [Climate::Solyth, Climate::Wynelle].map(|c| base_level(cfg, c.hunter(), n, setup.climate_levels.as_ref()));
        let has_climate = climate.iter().any(|level| *level > 0);
        let has_fan = climate[Climate::Solyth as usize] > 0;
        let air = air_rates(cfg, setup.custom, base_level(cfg, Hunter::Mavrine, n, setup.phantom_levels.as_ref()));
        let lvl = setup.xivvy_level.filter(|level| *level >= 0).unwrap_or(cfg.levels[Hunter::Xivvy as usize][n]);
        let mut sim = DeadAir {
            cfg,
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
                room: cfg.start_room,
                pacer: Pacer::default(),
                side: Side::Left,
                at_ms: 0.0,
                target: None,
                lure_left: 0.0,
                tempo: 1.0,
                dist: HashMap::new(),
            },
            office_dist: [cfg.distances_to(cfg.office_l), cfg.distances_to(cfg.office_r)],
            systems: [SystemState::WORKING; 5],
            temp: cfg.climate.start,
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
        sim.killer.tempo = framework::tempo(&mut sim.vr, cfg.xivvy_tempo);
        for who in Phantom::ALL {
            let cool = framework::spread_ms(cfg.phantom_first_ms, &mut sim.rng, cfg.phantom_first_spread);
            let tempo = framework::tempo(&mut sim.vr, cfg.phantom_tempo);
            sim.phantom_ai[who as usize] = PhantomAi { pacer: Pacer::default(), cool, tempo };
        }

        // With two or more phantoms on the floor, shuffle which one makes her first visit earlier and which later.
        let awake: Vec<Phantom> = Phantom::ALL.into_iter().filter(|who| sim.phantom_base(*who) > 0).collect();
        if sim.phantoms_on && awake.len() >= 2 && !sim.vr.is_neutral() {
            let count = awake.len();
            let mut slots: Vec<f64> = (0..count).map(|i| (i as f64 - (count as f64 - 1.0) / 2.0) * cfg.wake_step_ms).collect();
            for i in (1..count).rev() {
                let j = framework::pick_index(&mut sim.vr, i + 1);
                slots.swap(i, j);
            }
            for (who, slot) in awake.iter().zip(&slots) {
                sim.phantom_ai[*who as usize].pacer.skip(-slot * cfg.phantom_pace);
            }
        }

        if sim.has_climate {
            sim.temp = cfg.climate.start + (2.0 * sim.vr.next() - 1.0) * cfg.start_temp_spread;
            sim.drift_x = framework::tempo(&mut sim.vr, cfg.drift_spread);
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
        let hour = self.cfg.hour_index(self.elapsed);
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
        if self.elapsed >= self.cfg.night_ms {
            self.outcome = Some(Outcome::Win);
            ev.push(DeadAirEvent::Win);
        }
        ev
    }

    /// Zero draws: the lure needs a working audio system, a short cooldown and a non-vent room.
    pub fn lure(&mut self, room: Room) -> bool {
        if self.outcome.is_some() || !self.online(System::Audio) || self.lure_cool > 0.0 || !self.cfg.lureable(room) {
            return false;
        }
        self.killer.target = Some(room);
        self.killer.dist.clear();
        self.killer.lure_left = self.cfg.lure_ms;
        self.lure_cool = self.cfg.lure_cool_ms;
        self.lures += 1;
        if self.lures % self.cfg.lure_strain_every == 0 {
            self.systems[System::Audio as usize] = SystemState::BROKEN;
            self.pending.push(DeadAirEvent::Broken { system: System::Audio, by: BrokenBy::Strain });
        }
        true
    }

    /// Start rebooting a broken system (offline for `reboot_ms`). The fan needs the climate pair to have a Solyth.
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

    #[cfg(test)]
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
        base_level(self.cfg, who.hunter(), self.n, self.phantom_levels.as_ref())
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
                    self.killer.room = self.cfg.retreat_room;
                    self.killer.at_ms = 0.0;
                    self.killer.pacer.reset();
                }
                Hunter::Mavrine => self.stress = self.cfg.god_stress_reset,
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
        let reboot_ms = self.cfg.reboot_ms;
        for system in System::ALL {
            let state = &mut self.systems[system as usize];
            if state.status == Status::Rebooting {
                state.t += dt_ms;
                if state.t >= reboot_ms {
                    *state = SystemState::WORKING;
                    ev.push(DeadAirEvent::Fixed { system });
                }
            }
        }
    }

    fn phantoms_act(&mut self, dt_ms: f64, hour: usize, ev: &mut Vec<DeadAirEvent>) {
        let cfg = self.cfg;
        if let Some(sighting) = self.phantom.as_mut() {
            sighting.t += dt_ms;
            if sighting.t >= cfg.phantom_show_ms {
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
            let level = cfg.effective_level(self.phantom_base(who), hour);
            let rate = dt_ms * cfg.phantom_pace * self.phantom_ai[i].tempo * self.hunter_speed;
            if self.phantom_ai[i].pacer.step(cfg, rate, level, &mut self.rng) {
                self.phantom_visit(who, ev);
            }
        }
    }

    fn phantom_visit(&mut self, who: Phantom, ev: &mut Vec<DeadAirEvent>) {
        let cfg = self.cfg;
        let i = who as usize;
        let cool_ms = (cfg.phantom_cool_ms - self.n as f64 * cfg.phantom_cool_per_night_ms).max(cfg.phantom_cool_min_ms);
        self.phantom_ai[i].cool = framework::spread_ms(cool_ms, &mut self.rng, cfg.phantom_rest_spread);
        self.phantom_ai[i].tempo = framework::tempo(&mut self.vr, cfg.phantom_tempo);
        let cam_count = cfg.cam_rooms.len();
        let room = cfg.cam_rooms[((self.rng.next() * cam_count as f64).floor() as usize).min(cam_count - 1)];
        self.phantom = Some(Sighting { who, room, t: 0.0 });

        let mut targets = vec![cfg.phantom_systems[i]];
        if who == Phantom::Mavrine && self.has_climate {
            if self.climate[Climate::Solyth as usize] > 0 && self.climate[Climate::Wynelle as usize] > 0 {
                targets.extend([System::Heater, System::Fan]);
            } else {
                targets.push(if self.fan_on { System::Fan } else { System::Heater });
            }
        }
        for &(system, chance) in &cfg.phantom_extras[i][self.n] {
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
        let (cfg, w) = (self.cfg, &self.cfg.climate);
        let solyth = self.climate[Climate::Solyth as usize];
        let wynelle = self.climate[Climate::Wynelle as usize];
        let dir = if wynelle > 0 { -1.0 } else { 1.0 };
        let drift = w.drift_per_sec * self.drift_x * (1.0 + hour as f64 * w.drift_per_hour) * dir;
        let mut push = 0.0;
        if self.heater_on && self.online(System::Heater) {
            push += w.heater_per_sec;
        }
        if self.fan_on && self.online(System::Fan) {
            push -= w.fan_per_sec;
        }
        self.temp = (self.temp + (drift + push) * dt).clamp(w.min, w.max);

        let hot_from = w.hot - w.hot_margin;
        if wynelle > 0 && self.temp < w.cold {
            self.cold = (self.cold + (w.cold - self.temp) * w.cold_per_deg * cfg.climate_scale(wynelle) * dt).min(w.meter_max);
        } else {
            self.cold = (self.cold - w.recover_per_sec * dt).max(0.0);
        }
        if solyth > 0 && self.temp >= hot_from {
            self.heat = (self.heat + (self.temp - hot_from) * w.heat_per_deg * cfg.climate_scale(solyth) * dt).min(w.meter_max);
        } else {
            self.heat = (self.heat - w.recover_per_sec * dt).max(0.0);
        }

        if self.cold >= w.meter_max {
            return self.finish(ev, Hunter::Wynelle);
        }
        if self.heat >= w.meter_max {
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
        self.stress = (self.stress + change * dt).clamp(0.0, self.cfg.stress_max);
        if self.stress >= self.cfg.stress_max && !self.infinite_stress {
            return self.finish(ev, Hunter::Mavrine);
        }
        false
    }

    /// Xivvy waits at an open office vent (or is turned back by a seal) and otherwise walks toward it, or toward a lure.
    /// Returns true when the tick ends (she got in).
    fn hunter_tick(&mut self, dt_ms: f64, hour: usize, watch: Option<Room>, ev: &mut Vec<DeadAirEvent>) -> bool {
        let cfg = self.cfg;
        let room = self.killer.room;
        if let Some(side) = cfg.side_of_office(room) {
            self.killer.at_ms += dt_ms;
            if self.seals[side as usize] {
                if self.killer.at_ms >= cfg.retreat_ms {
                    self.killer.room = cfg.retreat_room;
                    self.killer.at_ms = 0.0;
                    self.killer.pacer.reset();
                    let same = framework::chance(&mut self.vr, cfg.same_side_again);
                    self.killer.side = if same { side } else { side.other() };
                    self.killer.tempo = framework::tempo(&mut self.vr, cfg.xivvy_tempo);
                    ev.push(DeadAirEvent::Retreat { side });
                }
            } else if self.killer.at_ms >= cfg.office_wait_ms {
                return self.finish(ev, Hunter::Xivvy);
            }
            return false;
        }

        let pace = if self.killer.lure_left > 0.0 { cfg.lure_pace } else { 1.0 };
        let level = cfg.effective_level(self.lvl, hour);
        if watch != Some(room) {
            let rate = dt_ms * pace * self.killer.tempo * self.hunter_speed;
            if self.killer.pacer.step(cfg, rate, level, &mut self.rng) {
                self.killer.tempo = framework::tempo(&mut self.vr, cfg.xivvy_tempo);
                let from = self.killer.room;
                let to = self.next_room();
                self.killer.room = to;
                if to != from {
                    let to_office = cfg.side_of_office(to).is_some();
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
        let cfg = self.cfg;
        let room = self.killer.room;
        let side = self.killer.side;
        let lured_target = if self.killer.lure_left > 0.0 { self.killer.target } else { None };
        let lured = lured_target.is_some();
        let target = lured_target.unwrap_or(side.office());
        if lured && self.killer.dist.is_empty() {
            self.killer.dist = cfg.distances_to(target);
        }

        let around: Vec<Room> = cfg
            .neighbours(room)
            .iter()
            .copied()
            .filter(|next| match cfg.side_of_office(*next) {
                Some(opening) => !self.seals[opening as usize] && target == *next,
                None => true,
            })
            .collect();
        let d = if lured { &self.killer.dist } else { &self.office_dist[side as usize] };
        let here = dist_of(d, room);
        let closer: Vec<Room> = around.iter().copied().filter(|next| dist_of(d, next) < here).collect();
        let follow = if self.killer.lure_left > 0.0 { cfg.lure_follow } else { self.focus };
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

#[cfg(test)]
/// Who is on the floor on a night (0-based), in cast order (`LureLogic.active_hunters`).
pub fn active_hunters(night_index: usize) -> Vec<Hunter> {
    let cfg = cfg();
    cfg.cast_order.iter().copied().filter(|hunter| cfg.levels[*hunter as usize][night_index.min(5)] > 0).collect()
}

#[cfg(test)]
/// A Custom Night's score: the sum of the sliders (each 0 to 20) times 10 (`NightWatchMode.record_win`).
pub fn custom_score(levels: &HashMap<Hunter, i32>) -> u32 {
    let cfg = cfg();
    levels.values().map(|level| (*level).clamp(0, cfg.max_slider) as u32).sum::<u32>() * cfg.points_per_level
}

fn base_level(cfg: &Cfg, hunter: Hunter, night: usize, overrides: Option<&HashMap<Hunter, i32>>) -> i32 {
    overrides.and_then(|levels| levels.get(&hunter).copied()).unwrap_or(cfg.levels[hunter as usize][night])
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
        let events = sim.step(cfg().reboot_ms, None);
        assert!(events.contains(&DeadAirEvent::Fixed { system: System::Audio }));
        assert!(sim.lure("lobby"));
    }

    #[test]
    fn the_json_carries_every_key_the_mode_reads() {
        // Cfg::load panics naming the mode and key when one is missing.
        let c = Cfg::load();
        assert_eq!(c.cam_rooms.len(), 8);
        assert_eq!(c.graph.len(), 10);
        assert_eq!(c.cast_order.len(), 6);
        assert_eq!(c.level_bump_by_hour.len(), 6);
        for extras in &c.phantom_extras {
            assert_eq!(extras.len(), 6);
        }
        assert_eq!(data::list(data::rules("dead_air"), "custom_night.presets").len(), 4);
        assert_eq!(data::list(data::rules("dead_air"), "systems").len(), 5);
    }

    #[test]
    fn loaded_values_equal_the_old_constants() {
        let c = cfg();
        assert_eq!(c.night_ms, 270_000.0);
        assert_eq!(c.hour_ms, 45_000.0);
        assert_eq!((c.points_per_level, c.max_slider), (10, 20));
        assert_eq!((c.reboot_ms, c.office_wait_ms, c.retreat_ms), (6000.0, 5200.0, 2000.0));
        assert_eq!((c.lure_ms, c.lure_cool_ms, c.lure_strain_every), (12000.0, 7000.0, 2));
        assert_eq!((c.climate.start, c.climate.cold, c.climate.hot), (70.0, 60.0, 100.0));
        assert_eq!(c.levels[Hunter::Xivvy as usize], [0, 3, 5, 7, 9, 11]);
        assert_eq!(c.levels[Hunter::Wynelle as usize], [0, 0, 0, 5, 8, 13]);
        assert_eq!(c.phantom_systems, [System::Cams, System::Audio, System::Vent]);
        assert_eq!(c.phantom_extras[Phantom::Mavrine as usize][5], vec![(System::Audio, 0.45), (System::Cams, 0.4)]);
        assert_eq!(air_rates(c, false, 0), Air { seal: 1.2, broken: 3.0, appliance: 0.9, decay: 2.0 });
        assert_eq!(air_rates(c, true, 10), Air { seal: 1.2, broken: 2.8 + 0.02 * 10.0, appliance: 0.8, decay: 2.1 });
        assert_eq!(c.move_window(4), (11000.0, 14000.0));
        assert_eq!(c.move_window(40), (2500.0, 5500.0));
        assert_eq!((c.effective_level(5, 2), c.effective_level(5, 3), c.effective_level(5, 5), c.effective_level(0, 5)), (5, 6, 7, 0));
        assert_eq!(c.hour_index(269_999.0), 5);
        assert_eq!(c.hour_index(1e9), 5);
        assert_eq!(active_hunters(0), vec![Hunter::Ferryn]);
        assert_eq!(active_hunters(5), vec![Hunter::Wynelle, Hunter::Xivvy, Hunter::Mavrine, Hunter::Briselle, Hunter::Solyth]);
        assert_eq!(custom_score(&HashMap::from([(Hunter::Xivvy, 25), (Hunter::Ferryn, 3)])), 230);
        assert_eq!(c.distances_to("officeL")["lobby"], 5);
        assert!(c.lureable("lobby") && !c.lureable("ventl") && !c.lureable("officeL"));
    }
}
