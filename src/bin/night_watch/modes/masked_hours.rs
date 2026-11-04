//! Masked Hours (Godot `game/night_watch/mask/mask_logic.gd`, a port of the 2D `mask-logic.js`): the wing's hall walker,
//! the rusher, the three vent hunters, the music box, the flashlight battery and the vent lights. `MaskedHours::step`
//! takes one tick of the player's inputs and returns what happened. The shift's window, cameras, call script and
//! art and sound are the shell's; the cue and art parts of the Godot scene are not ported here.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use serde_json::Value;

use crate::data;
use crate::framework::Mulberry;
use crate::mask::{VentLightActive, VentSide};

pub const MODE_ID: &str = "mask";
/// The data folder's name for this mode (`data/rules/masked_hours.json`).
const MODE_DATA: &str = "masked_hours";

/// Every tunable of the mode, read once from data/{rules,cast,buildings}/masked_hours.json; the rest of the file reads these fields.
struct Cfg {
    night_ms: f64,
    hour_ms: f64,
    hours: i32,
    tuned_night_ms: f64,
    box_decay: [f64; 6],
    flash_seconds: [f64; 6],
    box_starts_hour_night1: f64,
    levels: Vec<(&'static str, [i32; 6])>,
    hour_bumps: [i32; 6],
    vent_from_hours: [f64; 3],
    hall_steps: u32,
    ilyra_steps: u32,
    step_ms: f64,
    step_jitter: f64,
    hall_window_ms: f64,
    leave_ms: f64,
    off_grace_ms: f64,
    cool_ms: f64,
    cool_spread: f64,
    off_drain: f64,
    vent_wait_ms: f64,
    vent_need_ms: f64,
    vent_cool_ms: f64,
    box_grace_ms: f64,
    vent_mask_rate: f64,
    wind_per_sec: f64,
    room_ms: f64,
    room_jitter: f64,
    duct_ms: f64,
    duct_jitter: f64,
    vent_cool0_ms: f64,
    rush_cool_ms: f64,
    rush_cool0_ms: f64,
    haelith_duct_ms: f64,
    haelith_duct_jitter: f64,
    haelith_wait_ms: f64,
    haelith_need_ms: f64,
    haelith_fetch_ms: f64,
    haelith_cool_ms: f64,
    ilyra_step_ms: f64,
    ilyra_step_jitter: f64,
    ilyra_vent_ms: f64,
    ilyra_need_ms: f64,
    ilyra_cool_ms: f64,
    lit_decay: f64,
    ilyra_lit_decay: f64,
    flash_cost: f64,
    lamp_drain: f64,
    cam_drain: f64,
    pacer_base_ms: f64,
    pacer_per_level_ms: f64,
    pacer_min_ms: f64,
    pacer_spread_ms: f64,
    variety_salt: u32,
    rest_spread_varied: f64,
    rest_spread_plain: f64,
    kill_spread: f64,
    kill_floor: f64,
    side_bias: [f64; 2],
    plain_side_bias: f64,
    phase_factors: Vec<f64>,
    phase_ms: [f64; 2],
    glyra_decay_base: f64,
    glyra_decay_per_level: f64,
    dark_grace: [f64; 3],
    rush_flashes_base: u32,
    rush_extra_flash_levels: [i32; 2],
    rush_run: [f64; 3],
    rush_door: [f64; 3],
    homes: HashMap<&'static str, &'static str>,
    graph: Vec<(&'static str, Vec<&'static str>)>,
}

/// The data's strings live for the whole run (rooms and ids travel in events as &'static str).
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn nums<const N: usize>(v: &Value, path: &str) -> [f64; N] {
    let items = data::list(v, path);
    assert_eq!(items.len(), N, "night_watch data for mode 'masked_hours': '{path}' needs {N} entries");
    std::array::from_fn(|i| items[i].as_f64().unwrap_or_else(|| panic!("night_watch data for mode 'masked_hours': '{path}.{i}' is not a number")))
}

fn ints<const N: usize>(v: &Value, path: &str) -> [i32; N] {
    nums::<N>(v, path).map(|n| n as i32)
}

/// The keys of the object at `path`.
fn keys<'a>(v: &'a Value, path: &str) -> Vec<&'a str> {
    let mut at = v;
    for key in path.split('.') {
        at = &at[key];
    }
    at.as_object().unwrap_or_else(|| panic!("night_watch data for mode 'masked_hours': '{path}' is not an object")).keys().map(String::as_str).collect()
}

fn cfg() -> &'static Cfg {
    static CFG: OnceLock<Cfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let r = data::rules("masked_hours");
        let cast = data::cast("masked_hours");
        let building = data::building("masked_hours");
        let night_ms = data::num(r, "night_ms");
        let hours = data::int(r, "hours") as i32;
        let levels = keys(r, "levels").into_iter().map(|id| (leak(id), ints::<6>(r, &format!("levels.{id}")))).collect();
        let vent_from = |id: &str| data::num(r, &format!("vent_from_hours.{id}"));
        let graph = keys(building, "graph")
            .into_iter()
            .map(|id| (leak(id), data::list(building, &format!("graph.{id}")).iter().map(|n| leak(n.as_str().unwrap_or_default())).collect()))
            .collect();
        let homes = keys(cast, "homes").into_iter().map(|id| (leak(id), leak(data::text(cast, &format!("homes.{id}"))))).collect();
        let t = |key: &str| data::num(r, &format!("timings.{key}"));
        Cfg {
            night_ms,
            hour_ms: night_ms / f64::from(hours),
            hours,
            tuned_night_ms: data::num(r, "tuned_night_ms"),
            box_decay: nums(r, "box_decay"),
            flash_seconds: nums(r, "flash_seconds"),
            box_starts_hour_night1: data::num(r, "box_starts_hour_night1"),
            levels,
            hour_bumps: ints(r, "hour_bumps"),
            vent_from_hours: [vent_from("verrine"), vent_from("haelith"), vent_from("ilyra")],
            hall_steps: data::int(r, "timings.hall_steps") as u32,
            ilyra_steps: data::int(r, "timings.ilyra_steps") as u32,
            step_ms: t("step_ms"),
            step_jitter: t("step_jitter"),
            hall_window_ms: t("hall_window_ms"),
            leave_ms: t("leave_ms"),
            off_grace_ms: t("off_grace_ms"),
            cool_ms: t("cool_ms"),
            cool_spread: t("cool_spread"),
            off_drain: t("off_drain"),
            vent_wait_ms: t("vent_wait_ms"),
            vent_need_ms: t("vent_need_ms"),
            vent_cool_ms: t("vent_cool_ms"),
            box_grace_ms: t("box_grace_ms"),
            vent_mask_rate: t("vent_mask_rate"),
            wind_per_sec: t("wind_per_sec"),
            room_ms: t("room_ms"),
            room_jitter: t("room_jitter"),
            duct_ms: t("duct_ms"),
            duct_jitter: t("duct_jitter"),
            vent_cool0_ms: t("vent_cool0_ms"),
            rush_cool_ms: t("rush_cool_ms"),
            rush_cool0_ms: t("rush_cool0_ms"),
            haelith_duct_ms: t("haelith_duct_ms"),
            haelith_duct_jitter: t("haelith_duct_jitter"),
            haelith_wait_ms: t("haelith_wait_ms"),
            haelith_need_ms: t("haelith_need_ms"),
            haelith_fetch_ms: t("haelith_fetch_ms"),
            haelith_cool_ms: t("haelith_cool_ms"),
            ilyra_step_ms: t("ilyra_step_ms"),
            ilyra_step_jitter: t("ilyra_step_jitter"),
            ilyra_vent_ms: t("ilyra_vent_ms"),
            ilyra_need_ms: t("ilyra_need_ms"),
            ilyra_cool_ms: t("ilyra_cool_ms"),
            lit_decay: t("lit_decay"),
            ilyra_lit_decay: t("ilyra_lit_decay"),
            flash_cost: data::num(r, "battery.flash_cost"),
            lamp_drain: data::num(r, "battery.lamp_drain"),
            cam_drain: data::num(r, "battery.cam_drain"),
            pacer_base_ms: data::num(r, "pacer.base_ms"),
            pacer_per_level_ms: data::num(r, "pacer.per_level_ms"),
            pacer_min_ms: data::num(r, "pacer.min_ms"),
            pacer_spread_ms: data::num(r, "pacer.spread_ms"),
            variety_salt: data::int(r, "variety.salt") as u32,
            rest_spread_varied: data::num(r, "variety.rest_spread_varied"),
            rest_spread_plain: data::num(r, "variety.rest_spread_plain"),
            kill_spread: data::num(r, "variety.kill_spread"),
            kill_floor: data::num(r, "variety.kill_floor"),
            side_bias: nums(r, "variety.side_bias"),
            plain_side_bias: data::num(r, "variety.plain_side_bias"),
            phase_factors: nums::<3>(r, "variety.phase_factors").to_vec(),
            phase_ms: nums(r, "variety.phase_ms"),
            glyra_decay_base: data::num(r, "glyra_decay.base"),
            glyra_decay_per_level: data::num(r, "glyra_decay.per_level"),
            dark_grace: [data::num(r, "dark_grace.base_ms"), data::num(r, "dark_grace.per_level_ms"), data::num(r, "dark_grace.min_ms")],
            rush_flashes_base: data::int(r, "rush.flashes_base") as u32,
            rush_extra_flash_levels: ints(r, "rush.extra_flash_levels"),
            rush_run: [data::num(r, "rush.run_ms.base"), data::num(r, "rush.run_ms.per_level"), data::num(r, "rush.run_ms.min")],
            rush_door: [data::num(r, "rush.door_ms.base"), data::num(r, "rush.door_ms.per_level"), data::num(r, "rush.door_ms.min")],
            homes,
            graph,
        }
    })
}
/// Who the player is watching by camera, and what the lights and mask do this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaskInput {
    pub mask: bool,
    pub monitor: bool,
    pub cam: Option<&'static str>,
    pub vent_lights: VentLightActive,
    pub hall_light: bool,
    pub winding: bool,
    pub lamp: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskEvent {
    Room { id: &'static str, room: &'static str, poised: bool },
    Hall { id: &'static str },
    Step { id: &'static str, step: u32 },
    Office { id: &'static str },
    Leave { id: &'static str },
    Rush { id: &'static str, flashes: u32 },
    FlashHit { id: &'static str, hits: u32, of: u32 },
    RushRetreat { id: &'static str },
    Door { id: &'static str },
    Duct { id: &'static str, side: VentSide, room: &'static str },
    Vent { id: &'static str, side: VentSide },
    VentRetreat { id: &'static str, side: VentSide },
    Fetch { id: &'static str, side: VentSide, to: VentSide },
    Creep { id: &'static str, side: VentSide, step: u32 },
    Dark { id: &'static str },
    BoxStart,
    BoxEmpty,
    BoxSaved,
    Caught { by: &'static str },
    Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Caught(&'static str),
}

/// Length of one shift in ms (`night_ms` in the rules).
pub fn night_ms() -> f64 {
    data::num(data::rules(MODE_DATA), "night_ms")
}

/// In-game hours in one shift (`hours`); the clock shows one step per hour.
pub fn hours() -> u32 {
    data::int(data::rules(MODE_DATA), "hours") as u32
}

/// How many nights the mode plays (`nights` in the rules).
pub fn nights() -> u32 {
    data::int(data::rules(MODE_DATA), "nights") as u32
}

/// The score for a finished night: a win clears the level, a caught night scores nothing.
pub fn points_for(outcome: Outcome) -> u32 {
    match outcome {
        Outcome::Win => data::int(data::rules("masked_hours"), "points_per_level") as u32,
        Outcome::Caught(_) => 0,
    }
}

/// Per-night settings. `levels` overrides a hunter's AI level by id (a negative value keeps the night's default);
/// `levels["glyra"]` also sets the music box's wind-down rate.
#[derive(Clone, Debug)]
pub struct MaskOptions {
    pub levels: HashMap<&'static str, i32>,
    pub infinite_battery: bool,
    pub custom: bool,
    pub variety: bool,
    pub hunter_speed: f64,
}

impl Default for MaskOptions {
    fn default() -> Self {
        Self { levels: HashMap::new(), infinite_battery: false, custom: false, variety: true, hunter_speed: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Away,
    Room,
    Hall,
    Office,
    Poised,
    Run,
    Door,
    Duct,
    Vent,
    Fetch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vent {
    Verrine,
    Haelith,
    Ilyra,
}

impl Vent {
    const ALL: [Vent; 3] = [Vent::Verrine, Vent::Haelith, Vent::Ilyra];

    fn id(self) -> &'static str {
        match self {
            Vent::Verrine => "verrine",
            Vent::Haelith => "haelith",
            Vent::Ilyra => "ilyra",
        }
    }

    fn vent_from_ms(self) -> f64 {
        let hours = &cfg().vent_from_hours;
        match self {
            Vent::Verrine => hours[0] * cfg().hour_ms,
            Vent::Haelith => hours[1] * cfg().hour_ms,
            Vent::Ilyra => hours[2] * cfg().hour_ms,
        }
    }
}

/// A hunter's own wait between moves, drawn from the shift's stream (Godot `NightWatchMulberryPacer`).
#[derive(Clone, Debug, Default)]
struct Pacer {
    t: f64,
    next: Option<f64>,
}

impl Pacer {
    fn step(&mut self, dt_ms: f64, level: i32, rng: &mut Mulberry) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        let next = *self.next.get_or_insert_with(|| move_draw(level, rng));
        self.t += dt_ms;
        if self.t < next {
            return false;
        }
        self.t = 0.0;
        self.next = Some(move_draw(level, rng));
        true
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.next = None;
    }
}

fn move_draw(level: i32, rng: &mut Mulberry) -> f64 {
    let c = cfg();
    let lo = (c.pacer_base_ms - f64::from(level) * c.pacer_per_level_ms).max(c.pacer_min_ms);
    lo + rng.next() * c.pacer_spread_ms
}

#[derive(Clone, Copy, Debug)]
struct Mood {
    v: f64,
    left: f64,
}

const CALM: Mood = Mood { v: 1.0, left: 1e12 };

#[derive(Clone, Debug)]
struct Walker {
    phase: Phase,
    room: &'static str,
    t: f64,
    need: f64,
    step: u32,
    fooled: f64,
    off: f64,
    window: f64,
    cool: f64,
    pacer: Pacer,
}

#[derive(Clone, Debug)]
struct Rusher {
    phase: Phase,
    room: &'static str,
    t: f64,
    need: f64,
    flashes: u32,
    goal: u32,
    door_need: f64,
    cool: f64,
    pacer: Pacer,
}

#[derive(Clone, Debug)]
struct VentHunter {
    phase: Phase,
    room: &'static str,
    side: VentSide,
    t: f64,
    need: f64,
    lit: f64,
    step: u32,
    wait: f64,
}

#[derive(Clone, Copy)]
struct Lights {
    vent: VentLightActive,
    hall: bool,
}

impl Lights {
    fn vent_lit(&self, side: VentSide) -> bool {
        match side {
            VentSide::L => self.vent.l,
            VentSide::R => self.vent.r,
        }
    }
}

#[derive(Clone, Copy)]
struct Tick {
    mask: bool,
    watched: Option<&'static str>,
    lights: Lights,
    flash: bool,
}

#[derive(Clone, Copy)]
struct Moods {
    hall: Mood,
    rush: Mood,
    vent: Mood,
}

pub struct MaskedHours {
    night_idx: usize,
    rng: Mulberry,
    variety: Option<Mulberry>,
    levels: HashMap<&'static str, i32>,
    infinite_battery: bool,
    hunter_speed: f64,
    box_decay: f64,
    box_starts_ms: f64,
    flash_per_sec: f64,
    dist_to_office: HashMap<&'static str, u32>,
    elapsed_ms: f64,
    battery: f64,
    music_box: f64,
    box_zero_ms: f64,
    box_started: bool,
    dark_ms: f64,
    prev_hall_light: bool,
    prev_vent_lights: VentLightActive,
    moods: Moods,
    side_bias: f64,
    walker: Walker,
    rusher: Rusher,
    vent_cool: f64,
    vent_pacer: Pacer,
    verrine: VentHunter,
    haelith: VentHunter,
    ilyra: VentHunter,
    frozen: Vec<&'static str>,
    outcome: Option<Outcome>,
}

impl MaskedHours {
    /// A shift for `night` (clamped to 1 to the mode's nights). `rng` is the shift's stream: the variety stream is derived from its state.
    pub fn new(night: i32, mut rng: Mulberry, options: MaskOptions) -> Self {
        let night_idx = (night.clamp(1, nights() as i32) - 1) as usize;
        let box_decay = match options.levels.get("glyra") {
            Some(&level) if level >= 0 => box_decay_for_level(level),
            _ => cfg().box_decay[night_idx],
        };
        let box_starts_ms = if !options.custom && night_idx == 0 { cfg().box_starts_hour_night1 * cfg().hour_ms } else { 0.0 };
        let flash_per_sec = 100.0 / flash_seconds(night_idx);
        let mut variety = options.variety.then(|| Mulberry::new(i64::from(rng.state() ^ cfg().variety_salt)));

        let rest_spread = if variety.is_some() { cfg().rest_spread_varied } else { cfg().rest_spread_plain };
        let vent_cool = spread(&mut rng, cfg().vent_cool0_ms, rest_spread);
        let rush_cool = spread(&mut rng, cfg().rush_cool0_ms, rest_spread);
        let (side_bias, moods) = match variety.as_mut() {
            Some(vr) => {
                let bias = cfg().side_bias[0] + vr.next() * (cfg().side_bias[1] - cfg().side_bias[0]);
                let moods = Moods { hall: draw_phase(vr), rush: draw_phase(vr), vent: draw_phase(vr) };
                (bias, moods)
            }
            None => (cfg().plain_side_bias, Moods { hall: CALM, rush: CALM, vent: CALM }),
        };

        let home = |id: &str| home_room(id);
        Self {
            night_idx,
            rng,
            variety,
            levels: options.levels,
            infinite_battery: options.infinite_battery,
            hunter_speed: options.hunter_speed,
            box_decay,
            box_starts_ms,
            flash_per_sec,
            dist_to_office: distances_to_office(),
            elapsed_ms: 0.0,
            battery: 100.0,
            music_box: 100.0,
            box_zero_ms: 0.0,
            box_started: false,
            dark_ms: 0.0,
            prev_hall_light: false,
            prev_vent_lights: VentLightActive::default(),
            moods,
            side_bias,
            walker: Walker {
                phase: Phase::Away,
                room: home("thessine"),
                t: 0.0,
                need: 0.0,
                step: 0,
                fooled: 0.0,
                off: 0.0,
                window: cfg().hall_window_ms,
                cool: 0.0,
                pacer: Pacer::default(),
            },
            rusher: Rusher {
                phase: Phase::Away,
                room: home("umbriel"),
                t: 0.0,
                need: 0.0,
                flashes: 0,
                goal: 0,
                door_need: 0.0,
                cool: rush_cool,
                pacer: Pacer::default(),
            },
            vent_cool,
            vent_pacer: Pacer::default(),
            verrine: VentHunter::at_rest(home("verrine"), VentSide::L, cfg().vent_wait_ms),
            haelith: VentHunter::at_rest(home("haelith"), VentSide::L, cfg().haelith_wait_ms),
            ilyra: VentHunter::at_rest(home("ilyra"), VentSide::L, cfg().ilyra_vent_ms),
            frozen: Vec::new(),
            outcome: None,
        }
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    /// Where each room-bound hunter is on the wing's map: `(hunter id, room id)`. Glyra (the music box) and Nyxie
    /// (the dark) have no room, so they are not listed.
    pub fn hunter_rooms(&self) -> [(&'static str, &'static str); 5] {
        [
            ("thessine", self.walker.room),
            ("umbriel", self.rusher.room),
            ("verrine", self.verrine.room),
            ("haelith", self.haelith.room),
            ("ilyra", self.ilyra.room),
        ]
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ms
    }

    pub fn battery(&self) -> f64 {
        self.battery
    }

    pub fn music_box(&self) -> f64 {
        self.music_box
    }

    /// The hunters a watched camera is holding still this tick.
    pub fn frozen(&self) -> &[&'static str] {
        &self.frozen
    }

    /// Advances the shift by `dt_ms` with the player's inputs. Once the night is over, `step` returns nothing.
    pub fn step(&mut self, dt_ms: f64, input: MaskInput) -> Vec<MaskEvent> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        let mut ev = Vec::new();
        match self.tick(dt_ms, input, &mut ev) {
            Some(by) => {
                self.outcome = Some(Outcome::Caught(by));
                ev.push(MaskEvent::Caught { by });
            }
            None if self.elapsed_ms >= cfg().night_ms => {
                self.outcome = Some(Outcome::Win);
                ev.push(MaskEvent::Win);
            }
            None => {}
        }
        ev
    }

    fn tick(&mut self, dt_ms: f64, input: MaskInput, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        let dt = dt_ms / 1000.0;
        self.elapsed_ms += dt_ms;
        self.frozen.clear();
        self.step_moods(dt_ms);
        let tick = self.power(dt, &input);

        if self.battery <= 0.0 && self.base_level("nyxie") > 0 {
            if self.dark_ms == 0.0 {
                ev.push(MaskEvent::Dark { id: "nyxie" });
            }
            self.dark_ms += dt_ms;
            if self.dark_ms >= dark_grace_ms(self.base_level("nyxie")) {
                return Some("nyxie");
            }
        }

        if let Some(by) = self.step_music_box(dt, dt_ms, input.winding, ev) {
            return Some(by);
        }
        if let Some(by) = self.step_walker(dt_ms, &tick, ev) {
            return Some(by);
        }
        if let Some(by) = self.step_rusher(dt_ms, &tick, ev) {
            return Some(by);
        }
        self.step_vent_clock(dt_ms, ev);
        for who in Vent::ALL {
            let caught = match who {
                Vent::Verrine => self.step_verrine(dt_ms, &tick, ev),
                Vent::Haelith => self.step_haelith(dt_ms, &tick, ev),
                Vent::Ilyra => self.step_ilyra(dt_ms, &tick, ev),
            };
            if let Some(by) = caught {
                return Some(by);
            }
        }
        None
    }

    /// The battery feeds the lights, the desk lamp and the monitor; the monitor keeps working at zero.
    fn power(&mut self, dt: f64, input: &MaskInput) -> Tick {
        let mut lights = Lights { vent: VentLightActive::default(), hall: false };
        if self.battery > 0.0 {
            lights.vent = VentLightActive { l: input.vent_lights.l, r: input.vent_lights.r && !input.vent_lights.l };
            lights.hall = input.hall_light;
        }
        let using = lights.vent.l || lights.vent.r || lights.hall;
        let lamp = self.battery > 0.0 && input.lamp;
        if !self.infinite_battery {
            let light_want = if using { self.flash_per_sec * dt } else { 0.0 };
            let lamp_want = if lamp { self.flash_per_sec * cfg().lamp_drain * dt } else { 0.0 };
            let cam_want = if input.monitor && self.battery > 0.0 { self.flash_per_sec * cfg().cam_drain * dt } else { 0.0 };
            for want in [light_want, lamp_want, cam_want] {
                self.battery -= want.min(self.battery);
            }
        }

        let flash = lights.hall && !self.prev_hall_light;
        self.prev_hall_light = lights.hall;
        if flash {
            self.charge_flash();
        }
        for side in VentSide::ALL {
            let now = lights.vent_lit(side);
            if now && !lit_on(self.prev_vent_lights, side) {
                self.charge_flash();
            }
            set_vent_lit(&mut self.prev_vent_lights, side, now);
        }

        Tick { mask: input.mask, watched: if input.monitor { input.cam } else { None }, lights, flash }
    }

    fn charge_flash(&mut self) {
        if !self.infinite_battery {
            self.battery -= cfg().flash_cost.min(self.battery);
        }
    }

    fn step_moods(&mut self, dt_ms: f64) {
        let Some(vr) = self.variety.as_mut() else { return };
        for mood in [&mut self.moods.hall, &mut self.moods.rush, &mut self.moods.vent] {
            mood.left -= dt_ms;
            if mood.left <= 0.0 {
                *mood = draw_phase(vr);
            }
        }
    }

    fn step_music_box(&mut self, dt: f64, dt_ms: f64, winding: bool, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        if self.box_starts_ms > 0.0 && !self.box_started && self.elapsed_ms >= self.box_starts_ms {
            self.box_started = true;
            ev.push(MaskEvent::BoxStart);
        }
        let drain = if self.elapsed_ms >= self.box_starts_ms { self.box_decay } else { 0.0 };
        let wind = if winding { cfg().wind_per_sec * dt } else { 0.0 };
        self.music_box = (self.music_box - drain * dt + wind).clamp(0.0, 100.0);
        if self.music_box <= 0.0 {
            if self.box_zero_ms == 0.0 {
                ev.push(MaskEvent::BoxEmpty);
            }
            self.box_zero_ms += dt_ms;
            if self.box_zero_ms >= cfg().box_grace_ms {
                return Some("glyra");
            }
        } else if self.box_zero_ms > 0.0 {
            self.box_zero_ms = 0.0;
            ev.push(MaskEvent::BoxSaved);
        }
        None
    }

    fn step_walker(&mut self, dt_ms: f64, tick: &Tick, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        const ID: &str = "thessine";
        let v = self.moods.hall.v * self.hunter_speed;
        match self.walker.phase {
            Phase::Away => {
                self.walker.cool = (self.walker.cool - dt_ms * v).max(0.0);
                let lvl = self.level(ID);
                if self.walker.cool <= 0.0 && self.walker.pacer.step(dt_ms * v, lvl, &mut self.rng) {
                    let room = self.next_toward(self.walker.room);
                    self.walker.phase = Phase::Room;
                    self.walker.room = room;
                    self.walker.t = 0.0;
                    self.walker.need = self.room_need();
                    ev.push(MaskEvent::Room { id: ID, room, poised: false });
                }
            }
            Phase::Room => {
                if tick.watched == Some(self.walker.room) {
                    self.frozen.push(ID);
                } else {
                    self.walker.t += dt_ms;
                }
                if self.walker.t >= self.walker.need {
                    let next = self.next_toward(self.walker.room);
                    if next == "hall" && self.rusher_in_hall() {
                        return None;
                    }
                    if next == "hall" {
                        self.walker.phase = Phase::Hall;
                        self.walker.room = "hall";
                        self.walker.t = 0.0;
                        self.walker.step = 0;
                        self.walker.need = self.step_need();
                        ev.push(MaskEvent::Hall { id: ID });
                    } else {
                        self.walker.room = next;
                        self.walker.t = 0.0;
                        self.walker.need = self.room_need();
                        ev.push(MaskEvent::Room { id: ID, room: next, poised: false });
                    }
                }
            }
            Phase::Hall => {
                let on_cam = tick.watched == Some("hall") && self.walker.step >= cfg().hall_steps - 1;
                if on_cam {
                    self.frozen.push(ID);
                }
                if !tick.lights.hall && !on_cam {
                    self.walker.t += dt_ms;
                }
                if self.walker.t >= self.walker.need && self.walker.step < cfg().hall_steps {
                    self.walker.t = 0.0;
                    self.walker.step += 1;
                    self.walker.need = self.step_need();
                    ev.push(MaskEvent::Step { id: ID, step: self.walker.step });
                }
                if self.walker.step >= cfg().hall_steps {
                    self.walker.phase = Phase::Office;
                    self.walker.room = "office";
                    self.walker.t = 0.0;
                    self.walker.step = 0;
                    self.walker.fooled = 0.0;
                    self.walker.off = 0.0;
                    self.walker.window = cfg().hall_window_ms;
                    self.walker.pacer.reset();
                    ev.push(MaskEvent::Office { id: ID });
                }
            }
            Phase::Office => {
                if tick.mask {
                    self.walker.fooled += dt_ms;
                    self.walker.off = (self.walker.off - dt_ms * cfg().off_drain).max(0.0);
                    if self.walker.fooled >= cfg().leave_ms {
                        self.walker.cool = spread(&mut self.rng, cfg().cool_ms, cfg().cool_spread);
                        self.walker.phase = Phase::Away;
                        self.walker.room = home_room(ID);
                        self.walker.pacer.reset();
                        ev.push(MaskEvent::Leave { id: ID });
                    }
                } else if self.walker.fooled == 0.0 {
                    self.walker.window -= dt_ms;
                    if self.walker.window <= 0.0 {
                        return Some(ID);
                    }
                } else {
                    self.walker.off += dt_ms;
                    if self.walker.off >= cfg().off_grace_ms {
                        return Some(ID);
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn step_rusher(&mut self, dt_ms: f64, tick: &Tick, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        const ID: &str = "umbriel";
        let v = self.moods.rush.v * self.hunter_speed;
        let rl = self.level(ID);
        match self.rusher.phase {
            Phase::Away => {
                self.rusher.cool = (self.rusher.cool - dt_ms * v).max(0.0);
                if self.rusher.cool <= 0.0 && self.rusher.pacer.step(dt_ms * v, rl, &mut self.rng) {
                    self.rusher.phase = Phase::Poised;
                    self.rusher.room = "ballroom";
                    self.rusher.t = 0.0;
                    self.rusher.need = self.room_need();
                    ev.push(MaskEvent::Room { id: ID, room: "ballroom", poised: true });
                }
            }
            Phase::Poised => {
                if tick.watched == Some("ballroom") {
                    self.frozen.push(ID);
                } else {
                    self.rusher.t += dt_ms;
                }
                if self.rusher.t >= self.rusher.need && !self.walker_in_hall() {
                    self.rusher.phase = Phase::Run;
                    self.rusher.room = "hall";
                    self.rusher.t = 0.0;
                    self.rusher.need = rush_run_ms(rl);
                    self.rusher.flashes = 0;
                    self.rusher.goal = rush_flashes(rl);
                    self.rusher.door_need = rush_door_ms(rl);
                    ev.push(MaskEvent::Rush { id: ID, flashes: self.rusher.goal });
                }
            }
            _ => {
                if tick.flash {
                    self.rusher.flashes += 1;
                    ev.push(MaskEvent::FlashHit { id: ID, hits: self.rusher.flashes, of: self.rusher.goal });
                }
                self.rusher.t += dt_ms;
                if self.rusher.flashes >= self.rusher.goal {
                    self.rusher.cool = self.rest_spread_ms(cfg().rush_cool_ms);
                    self.rusher.phase = Phase::Away;
                    self.rusher.room = home_room(ID);
                    self.rusher.pacer.reset();
                    ev.push(MaskEvent::RushRetreat { id: ID });
                } else if self.rusher.phase == Phase::Run && self.rusher.t >= self.rusher.need {
                    self.rusher.phase = Phase::Door;
                    self.rusher.t = 0.0;
                    self.rusher.need = self.rusher.door_need;
                    ev.push(MaskEvent::Door { id: ID });
                } else if self.rusher.phase == Phase::Door && self.rusher.t >= self.rusher.need {
                    return Some(ID);
                }
            }
        }
        None
    }

    /// One clock for the vents: when it fires, one vent hunter on the floor comes out, drawn by level.
    fn step_vent_clock(&mut self, dt_ms: f64, ev: &mut Vec<MaskEvent>) {
        if self.vent_out() {
            return;
        }
        let v = self.moods.vent.v * self.hunter_speed;
        self.vent_cool = (self.vent_cool - dt_ms * v).max(0.0);
        let mut pool: Vec<Vent> = Vec::new();
        let mut top = 0;
        for who in Vent::ALL {
            let lvl = self.level(who.id());
            if lvl > 0 && self.elapsed_ms >= who.vent_from_ms() {
                pool.push(who);
                top = top.max(lvl);
            }
        }
        if self.vent_cool > 0.0 || !self.vent_pacer.step(dt_ms * v, top, &mut self.rng) {
            return;
        }
        let Some(&first) = pool.first() else { return };
        let who = if pool.len() > 1 { self.pick_by_level(&pool) } else { first };
        let side = if self.rng.next() < self.side_bias { VentSide::L } else { VentSide::R };
        match who {
            Vent::Verrine => {
                let need = jitter(&mut self.rng, cfg().duct_ms, cfg().duct_jitter);
                let room = side.duct_cam();
                self.verrine = VentHunter { phase: Phase::Duct, room, side, t: 0.0, need, lit: self.verrine.lit, step: 0, wait: self.verrine.wait };
                ev.push(MaskEvent::Duct { id: who.id(), side, room });
            }
            Vent::Haelith => {
                let need = jitter(&mut self.rng, cfg().haelith_duct_ms, cfg().haelith_duct_jitter);
                let room = side.duct_cam();
                self.haelith = VentHunter { phase: Phase::Duct, room, side, t: 0.0, need, lit: 0.0, step: 0, wait: self.haelith.wait };
                ev.push(MaskEvent::Duct { id: who.id(), side, room });
            }
            Vent::Ilyra => {
                let need = jitter(&mut self.rng, cfg().ilyra_step_ms, cfg().ilyra_step_jitter);
                let room = side.duct_cam();
                self.ilyra = VentHunter { phase: Phase::Duct, room, side, t: 0.0, need, lit: 0.0, step: 1, wait: self.ilyra.wait };
                ev.push(MaskEvent::Duct { id: who.id(), side, room });
            }
        }
    }

    /// The vent hunter who comes out: a draw weighted by each one's level.
    fn pick_by_level(&mut self, pool: &[Vent]) -> Vent {
        let total: f64 = pool.iter().map(|who| f64::from(self.level(who.id()))).sum();
        let mut roll = self.rng.next() * total;
        let mut who = pool[pool.len() - 1];
        for &candidate in pool {
            roll -= f64::from(self.level(candidate.id()));
            if roll < 0.0 {
                who = candidate;
                break;
            }
        }
        who
    }

    fn step_verrine(&mut self, dt_ms: f64, tick: &Tick, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        const ID: &str = "verrine";
        match self.verrine.phase {
            Phase::Duct => {
                if tick.watched == Some(self.verrine.room) {
                    self.frozen.push(ID);
                } else {
                    self.verrine.t += dt_ms;
                }
                if self.verrine.t >= self.verrine.need {
                    let room = self.next_toward(self.verrine.room);
                    let wait = self.kill_wait(cfg().vent_wait_ms);
                    let side = self.verrine.side;
                    self.verrine.phase = Phase::Vent;
                    self.verrine.room = room;
                    self.verrine.t = 0.0;
                    self.verrine.lit = 0.0;
                    self.verrine.wait = wait;
                    ev.push(MaskEvent::Vent { id: ID, side });
                }
            }
            Phase::Vent => {
                let side = self.verrine.side;
                if tick.mask {
                    self.verrine.t += dt_ms * cfg().vent_mask_rate;
                } else if tick.lights.vent_lit(side) {
                    self.verrine.lit += dt_ms;
                } else {
                    self.verrine.lit = (self.verrine.lit - dt_ms * cfg().lit_decay).max(0.0);
                    self.verrine.t += dt_ms;
                }
                if self.verrine.lit >= cfg().vent_need_ms {
                    self.retreat(Vent::Verrine, cfg().vent_cool_ms, ev);
                } else if self.verrine.t >= self.verrine.wait {
                    return Some(ID);
                }
            }
            _ => {}
        }
        None
    }

    /// Haelith bounds down a duct, waits for a quick throw of the light, fetches it across to the other duct, and wants it there too.
    fn step_haelith(&mut self, dt_ms: f64, tick: &Tick, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        const ID: &str = "haelith";
        match self.haelith.phase {
            Phase::Duct => {
                if tick.watched == Some(self.haelith.room) {
                    self.frozen.push(ID);
                } else {
                    self.haelith.t += dt_ms;
                }
                if self.haelith.t >= self.haelith.need {
                    let room = self.next_toward(self.haelith.room);
                    let wait = self.kill_wait(cfg().haelith_wait_ms);
                    let side = self.haelith.side;
                    self.haelith.phase = Phase::Vent;
                    self.haelith.room = room;
                    self.haelith.t = 0.0;
                    self.haelith.lit = 0.0;
                    self.haelith.wait = wait;
                    ev.push(MaskEvent::Vent { id: ID, side });
                }
            }
            Phase::Fetch => {
                self.haelith.t += dt_ms;
                if self.haelith.t >= cfg().haelith_fetch_ms {
                    let side = self.haelith.side.opposite();
                    let need = jitter(&mut self.rng, cfg().haelith_duct_ms, cfg().haelith_duct_jitter);
                    let room = side.duct_cam();
                    self.haelith.side = side;
                    self.haelith.phase = Phase::Duct;
                    self.haelith.room = room;
                    self.haelith.t = 0.0;
                    self.haelith.need = need;
                    self.haelith.lit = 0.0;
                    ev.push(MaskEvent::Duct { id: ID, side, room });
                }
            }
            Phase::Vent => {
                let side = self.haelith.side;
                if !tick.mask && tick.lights.vent_lit(side) {
                    self.haelith.lit += dt_ms;
                } else {
                    self.haelith.lit = (self.haelith.lit - dt_ms * cfg().lit_decay).max(0.0);
                    self.haelith.t += dt_ms;
                }
                if self.haelith.lit >= cfg().haelith_need_ms {
                    if self.haelith.step == 0 {
                        let to = side.opposite();
                        self.haelith.step = 1;
                        self.haelith.phase = Phase::Fetch;
                        self.haelith.room = home_room(ID);
                        self.haelith.t = 0.0;
                        self.haelith.lit = 0.0;
                        ev.push(MaskEvent::Fetch { id: ID, side, to });
                    } else {
                        self.haelith.step = 0;
                        self.haelith.t = 0.0;
                        self.haelith.lit = 0.0;
                        self.retreat(Vent::Haelith, cfg().haelith_cool_ms, ev);
                    }
                } else if self.haelith.t >= self.haelith.wait {
                    return Some(ID);
                }
            }
            _ => {}
        }
        None
    }

    /// Ilyra creeps a step only in the dark, freezes in the light, and is sent home by enough light.
    fn step_ilyra(&mut self, dt_ms: f64, tick: &Tick, ev: &mut Vec<MaskEvent>) -> Option<&'static str> {
        const ID: &str = "ilyra";
        if self.ilyra.phase == Phase::Away {
            return None;
        }
        let side = self.ilyra.side;
        if !tick.mask && tick.lights.vent_lit(side) {
            self.ilyra.lit += dt_ms;
        } else {
            self.ilyra.lit = (self.ilyra.lit - dt_ms * cfg().ilyra_lit_decay).max(0.0);
            self.ilyra.t += dt_ms;
        }
        if self.ilyra.lit >= cfg().ilyra_need_ms {
            self.ilyra.step = 0;
            self.ilyra.t = 0.0;
            self.ilyra.lit = 0.0;
            self.retreat(Vent::Ilyra, cfg().ilyra_cool_ms, ev);
        } else if self.ilyra.phase == Phase::Duct && self.ilyra.t >= self.ilyra.need {
            self.ilyra.step += 1;
            self.ilyra.t = 0.0;
            if self.ilyra.step <= cfg().ilyra_steps {
                self.ilyra.need = jitter(&mut self.rng, cfg().ilyra_step_ms, cfg().ilyra_step_jitter);
                ev.push(MaskEvent::Creep { id: ID, side, step: self.ilyra.step });
            } else {
                let room = self.next_toward(self.ilyra.room);
                let wait = self.kill_wait(cfg().ilyra_vent_ms);
                self.ilyra.phase = Phase::Vent;
                self.ilyra.room = room;
                self.ilyra.wait = wait;
                ev.push(MaskEvent::Vent { id: ID, side });
            }
        } else if self.ilyra.phase == Phase::Vent && self.ilyra.t >= self.ilyra.wait {
            return Some(ID);
        }
        None
    }

    /// Sends a vent hunter home. The vent clock's cooldown and pacer are reset whichever hunter goes, as in the 2D logic.
    fn retreat(&mut self, who: Vent, ms: f64, ev: &mut Vec<MaskEvent>) {
        self.vent_cool = self.rest_spread_ms(ms);
        self.vent_pacer.reset();
        let hunter = self.vent_mut(who);
        hunter.phase = Phase::Away;
        hunter.room = home_room(who.id());
        let side = hunter.side;
        ev.push(MaskEvent::VentRetreat { id: who.id(), side });
    }

    fn vent_mut(&mut self, who: Vent) -> &mut VentHunter {
        match who {
            Vent::Verrine => &mut self.verrine,
            Vent::Haelith => &mut self.haelith,
            Vent::Ilyra => &mut self.ilyra,
        }
    }

    fn vent_out(&self) -> bool {
        [&self.verrine, &self.haelith, &self.ilyra].iter().any(|hunter| hunter.phase != Phase::Away)
    }

    fn walker_in_hall(&self) -> bool {
        matches!(self.walker.phase, Phase::Hall | Phase::Office)
    }

    fn rusher_in_hall(&self) -> bool {
        matches!(self.rusher.phase, Phase::Run | Phase::Door)
    }

    fn base_level(&self, id: &str) -> i32 {
        match self.levels.get(id) {
            Some(&level) if level >= 0 => level,
            _ => cfg().levels.iter().find(|entry| entry.0 == id).map_or(0, |entry| entry.1[self.night_idx]),
        }
    }

    fn level(&self, id: &str) -> i32 {
        effective_level(self.base_level(id), hour_index(self.elapsed_ms))
    }

    fn next_toward(&self, room: &'static str) -> &'static str {
        let mut best = room;
        for &next in neighbors(room) {
            if self.dist_of(next) < self.dist_of(best) {
                best = next;
            }
        }
        best
    }

    fn dist_of(&self, room: &str) -> u32 {
        self.dist_to_office.get(room).copied().unwrap_or(u32::MAX)
    }

    fn kill_wait(&mut self, ms: f64) -> f64 {
        match self.variety.as_mut() {
            None => ms,
            Some(vr) => (ms * cfg().kill_floor).max(ms * (1.0 - cfg().kill_spread + 2.0 * cfg().kill_spread * vr.next())),
        }
    }

    fn rest_spread_ms(&mut self, ms: f64) -> f64 {
        let rest_spread = if self.variety.is_some() { cfg().rest_spread_varied } else { cfg().rest_spread_plain };
        spread(&mut self.rng, ms, rest_spread)
    }

    fn step_need(&mut self) -> f64 {
        jitter(&mut self.rng, cfg().step_ms, cfg().step_jitter)
    }

    fn room_need(&mut self) -> f64 {
        jitter(&mut self.rng, cfg().room_ms, cfg().room_jitter)
    }
}

impl VentHunter {
    fn at_rest(room: &'static str, side: VentSide, wait: f64) -> Self {
        Self { phase: Phase::Away, room, side, t: 0.0, need: 0.0, lit: 0.0, step: 0, wait }
    }
}

impl VentSide {
    fn opposite(self) -> VentSide {
        match self {
            VentSide::L => VentSide::R,
            VentSide::R => VentSide::L,
        }
    }
}

fn home_room(id: &str) -> &'static str {
    cfg().homes.get(id).copied().unwrap_or_else(|| panic!("night_watch data for mode 'masked_hours': no home for '{id}'"))
}

fn neighbors(room: &str) -> &'static [&'static str] {
    match cfg().graph.iter().find(|entry| entry.0 == room) {
        Some(entry) => entry.1.as_slice(),
        None => &[],
    }
}

fn distances_to_office() -> HashMap<&'static str, u32> {
    let mut dist = HashMap::from([("office", 0u32)]);
    let mut queue = VecDeque::from(["office"]);
    while let Some(room) = queue.pop_front() {
        let d = dist[room];
        for &next in neighbors(room) {
            if !dist.contains_key(next) {
                dist.insert(next, d + 1);
                queue.push_back(next);
            }
        }
    }
    dist
}

fn lit_on(lit: VentLightActive, side: VentSide) -> bool {
    match side {
        VentSide::L => lit.l,
        VentSide::R => lit.r,
    }
}

fn set_vent_lit(lit: &mut VentLightActive, side: VentSide, on: bool) {
    match side {
        VentSide::L => lit.l = on,
        VentSide::R => lit.r = on,
    }
}

fn draw_phase(vr: &mut Mulberry) -> Mood {
    let idx = ((vr.next() * cfg().phase_factors.len() as f64).floor() as usize).min(cfg().phase_factors.len() - 1);
    let v = cfg().phase_factors[idx];
    let left = cfg().phase_ms[0] + vr.next() * (cfg().phase_ms[1] - cfg().phase_ms[0]);
    Mood { v, left }
}

fn spread(rng: &mut Mulberry, ms: f64, fraction: f64) -> f64 {
    ms * (1.0 - fraction + 2.0 * fraction * rng.next())
}

fn jitter(rng: &mut Mulberry, ms: f64, fraction: f64) -> f64 {
    ms * (1.0 - fraction / 2.0 + rng.next() * fraction)
}

fn flash_seconds(night_idx: usize) -> f64 {
    (cfg().flash_seconds[night_idx] * (cfg().night_ms / cfg().tuned_night_ms)).round()
}

fn hour_index(elapsed_ms: f64) -> i32 {
    ((elapsed_ms / cfg().hour_ms).floor() as i32).min(cfg().hours - 1)
}

fn effective_level(base: i32, hour: i32) -> i32 {
    if base <= 0 {
        return 0;
    }
    base + cfg().hour_bumps[hour.clamp(0, cfg().hours - 1) as usize]
}

fn box_decay_for_level(level: i32) -> f64 {
    if level > 0 {
        cfg().glyra_decay_base + f64::from(level) * cfg().glyra_decay_per_level
    } else {
        0.0
    }
}

fn dark_grace_ms(level: i32) -> f64 {
    let [base, per_level, min] = cfg().dark_grace;
    (base - f64::from(level) * per_level).max(min)
}

fn rush_flashes(level: i32) -> u32 {
    let [first, second] = cfg().rush_extra_flash_levels;
    cfg().rush_flashes_base + u32::from(level >= first) + u32::from(level >= second)
}

fn rush_run_ms(level: i32) -> f64 {
    let [base, per_level, min] = cfg().rush_run;
    (base - f64::from(level) * per_level).max(min)
}

fn rush_door_ms(level: i32) -> f64 {
    let [base, per_level, min] = cfg().rush_door;
    (base - f64::from(level) * per_level).max(min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unattended_music_box_catches_the_player_after_the_grace() {
        let levels = HashMap::from([
            ("glyra", 5),
            ("thessine", 0),
            ("umbriel", 0),
            ("verrine", 0),
            ("haelith", 0),
            ("ilyra", 0),
            ("nyxie", 0),
        ]);
        let options = MaskOptions { levels, variety: false, ..MaskOptions::default() };
        let mut shift = MaskedHours::new(2, Mulberry::new(7), options);
        let mut caught_at = None;
        let mut elapsed = 0.0;
        while shift.outcome().is_none() {
            let events = shift.step(100.0, MaskInput::default());
            elapsed += 100.0;
            if events.contains(&MaskEvent::Caught { by: "glyra" }) {
                caught_at = Some(elapsed);
            }
        }
        let at = caught_at.expect("the music box catches the player");
        assert!((70_000.0..=71_000.0).contains(&at), "caught at {at} ms");
        assert_eq!(shift.outcome(), Some(Outcome::Caught("glyra")));
    }

    #[test]
    fn the_json_carries_every_key_the_mode_reads() {
        // cfg() panics naming the mode and key when one is missing.
        let c = cfg();
        assert_eq!(c.levels.len(), 6);
        assert_eq!(c.graph.len(), 11);
        for id in ["thessine", "umbriel", "verrine", "haelith", "ilyra", "glyra"] {
            assert!(!home_room(id).is_empty());
        }
        for (room, next) in &c.graph {
            for n in next {
                assert!(neighbors(n).contains(room), "{room} -> {n} is not undirected");
            }
        }
    }

    #[test]
    fn loaded_values_equal_the_old_constants() {
        let c = cfg();
        assert_eq!(c.night_ms, 270_000.0);
        assert_eq!(c.hour_ms, 45_000.0);
        assert_eq!(c.box_decay, [2.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(c.variety_salt, 0x3d1f_8a57);
        assert_eq!(c.step_ms, 2300.0);
        assert_eq!(c.hall_steps, 4);
        assert_eq!(c.ilyra_steps, 3);
        assert_eq!(c.phase_factors, vec![0.88, 1.0, 1.12]);
        assert_eq!(MaskedHours::new(1, Mulberry::new(1), MaskOptions::default()).base_level("thessine"), 3);
        assert_eq!(points_for(Outcome::Win), 10);
        assert_eq!(home_room("umbriel"), "gallery");
        assert_eq!(neighbors("office"), &["hall", "ventL", "ventR"]);
        assert_eq!(Vent::Ilyra.vent_from_ms(), 90_000.0);
        assert_eq!(effective_level(5, 3), 6);
        assert_eq!(effective_level(5, 5), 7);
        assert_eq!(effective_level(0, 5), 0);
        assert_eq!(dark_grace_ms(10), 8000.0);
        assert_eq!(rush_flashes(16), 5);
        assert_eq!(rush_run_ms(100), 1200.0);
        assert_eq!(rush_door_ms(10), 1700.0);
        assert_eq!(box_decay_for_level(4), 1.3);
        assert_eq!(flash_seconds(0), 66.0);
    }
}
