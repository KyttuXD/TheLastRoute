//! The shared hunter brain (game/hunters/ai/hunter_brain.gd): beam attention, perception
//! (sight + hearing), stamina, the camp guard and the idle / wander / investigate / chase /
//! search state machine, with each hunter's gimmick hooked in at the original's points.
//! Positions are grid cells (1 cell = 1 m).

use super::flight_edges::{self, EdgeWalk, Hop, Node, StackNav};
use super::gimmicks::{Ctx, Event, Gimmick};
use super::path_scheduler::{self, PlanRequest, Route};
use super::peek::{self, Corner, Style};
use crate::config::*;
use crate::content::hunters::HunterDef;
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use super::water::{HunterWater, WaterField, WaterPlan, WaterRules};
use std::sync::{Arc, OnceLock};

#[cfg(test)]
#[path = "bench.rs"]
mod bench;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Wander,
    Investigate,
    Chase,
    Search,
    /// Stopped behind a route corner, leaning out to look down the hall (`hunters::peek`).
    Peek,
}

impl State {
    /// The wire state name (`net::wiring::transition_rule` and the wire use these).
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "IDLE",
            State::Wander => "WANDER",
            State::Investigate => "INVESTIGATE",
            State::Chase => "CHASE",
            State::Search => "SEARCH",
            State::Peek => "PEEK",
        }
    }
}

/// The sluice-gate channel cells a hunter may cross: all of them while the gate is down, none while it is raised.
pub fn gate_floor_cells(cells: &[Cell], gate_down: bool) -> Vec<Cell> {
    if gate_down { cells.to_vec() } else { Vec::new() }
}

/// The beats of a corner peek (HunterBrain.Peek): slide into the lean spot, look round the edge, then either carry on
/// (nothing there), burst into a chase (`Seen`, a "burst" hunter) or wait behind the corner (`Ambush`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeekPhase {
    Settle,
    Look,
    Seen,
    Ambush,
}

/// What the brain knows about the runner this frame.
#[derive(Clone, Default)]
pub struct Target {
    pub pos: (f32, f32),
    pub facing: (f32, f32),
    pub hidden: bool,
    pub hiding_spot: Option<usize>,
    pub in_safe_zone: bool,
    pub noise: f32,
    pub flashlight_on: bool,
    pub lamp_charge: f32,
    pub sprinting: bool,
    pub in_vent: bool,
    pub vent_time_in: f32,
    pub temperature: f32,
    pub pesticide: bool,
    /// The full-screen map is open (Godot target["map_open"], main_3d.gd:444). Read by Void Yellow.
    pub map_open: bool,
    /// The level the target stands on (continuous levels). Only read when `cross` is Some.
    pub level: i32,
    /// The target's absolute feet height (m); the capture test needs the same height.
    pub y: f32,
    /// How a target on another level is sensed; None means the same level (the facility answers as before).
    pub cross: Option<CrossSense>,
}

/// What a hunter senses of a target on another level, filled by the host: `sees` from
/// `flight_edges::sight_clear_3d` (an open well passes sight, a slab never does) and `hear_scale` from
/// `flight_edges::rules().hear_adjacent_scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossSense {
    pub sees: bool,
    pub hear_scale: f32,
}

/// LightField.unlit_scale: a runner in pitch dark is seen close, one in a lamp pool far.
fn light_unlit_scale(exposure: f32) -> f32 {
    super::light_field::unlit_scale(exposure)
}

pub fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn cell_of(p: (f32, f32)) -> Cell {
    Cell::new(p.0.floor() as i32, p.1.floor() as i32)
}

/// The hunter target rules (data/hunters/targets.json): the weights, the bot sight rule and which difficulties let a
/// hunter chase an NPC bot. The runner is always a candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetRules {
    /// Divides the runner's distance when a hunter ranks its candidates (a larger weight is preferred).
    pub runner_weight: f32,
    /// Divides a bot's distance when a hunter ranks its candidates.
    pub bot_weight: f32,
    /// Metres a hunter can pick a bot out at.
    pub bot_range: f32,
    /// Whether a bot must also be in line of sight to be a candidate.
    pub bot_line_of_sight: bool,
    /// Per difficulty id ("relaxed", "standard", ...): whether bots are candidates there.
    pub bot_targets: HashMap<String, bool>,
}

impl TargetRules {
    /// Reads the rules from a parsed data/hunters/targets.json. None when a key is missing or has the wrong type.
    pub fn parse(v: &serde_json::Value) -> Option<TargetRules> {
        let num = |key: &str| v[key].as_f64().map(|x| x as f32);
        let bots = v["bot_targets"].as_object()?;
        Some(TargetRules {
            runner_weight: num("runner_weight").filter(|w| *w > 0.0)?,
            bot_weight: num("bot_weight").filter(|w| *w > 0.0)?,
            bot_range: num("bot_range")?,
            bot_line_of_sight: v["bot_line_of_sight"].as_bool()?,
            bot_targets: bots.iter().filter_map(|(k, x)| Some((k.clone(), x.as_bool()?))).collect(),
        })
    }

    /// The rules of the embedded data/hunters/targets.json, read once.
    pub fn get() -> &'static TargetRules {
        static RULES: OnceLock<TargetRules> = OnceLock::new();
        RULES.get_or_init(|| {
            let (_, text) = crate::content::embedded::data_under("hunters/targets.json")
                .next()
                .expect("data/hunters/targets.json is embedded");
            let v: serde_json::Value = serde_json::from_str(text).expect("data/hunters/targets.json is JSON");
            TargetRules::parse(&v).expect("data/hunters/targets.json has the target keys")
        })
    }

    /// Whether hunters on `diff` may chase a bot. A difficulty with no entry allows none.
    pub fn bots_allowed(&self, diff: Difficulty) -> bool {
        self.bot_targets.get(diff.id()).copied().unwrap_or(false)
    }
}

/// Which target a hunter at `from` chases: None is the runner, Some(i) is `bots[i]`. The runner is always a candidate.
/// A bot is one only when bots are allowed on `diff`, it is within `bot_range`, and `los` clears it (when the rule asks
/// for line of sight). The candidate with the smallest distance over its weight wins; a tie keeps the runner.
pub fn choose_target(
    rules: &TargetRules,
    diff: Difficulty,
    from: (f32, f32),
    runner: (f32, f32),
    bots: &[(f32, f32)],
    los: impl Fn((f32, f32), (f32, f32)) -> bool,
) -> Option<usize> {
    if !rules.bots_allowed(diff) || bots.is_empty() {
        return None;
    }
    let mut best: (f32, Option<usize>) = (dist(from, runner) / rules.runner_weight, None);
    for (i, bot) in bots.iter().enumerate() {
        let d = dist(from, *bot);
        if d > rules.bot_range || (rules.bot_line_of_sight && !los(from, *bot)) {
            continue;
        }
        let score = d / rules.bot_weight;
        if score < best.0 {
            best = (score, Some(i));
        }
    }
    best.1
}

fn body_fits_gated(fac: &Facility, gates: &[Cell], x: f32, y: f32) -> bool {
    body_fits_gated_at(fac, gates, x, y, None)
}

/// `body_fits_gated` with an optional absolute height: on an edge walk the footprint is the flight surface at `h`
/// (`navigation::on_flight`); without one it is the plain floor test.
fn body_fits_gated_at(fac: &Facility, gates: &[Cell], x: f32, z: f32, h: Option<f32>) -> bool {
    let fits = match h {
        Some(h) => super::navigation::on_flight(fac, x, z, h),
        None => fac.hunter_fits(x, z),
    };
    fits || (!gates.is_empty() && gates.contains(&cell_of((x, z))))
}

/// Whether a body of HUNTER_RADIUS can sweep the straight line from `from` to `to` (sampled every 0.15 m) on the floor.
fn sweep_clear(fac: &Facility, gates: &[Cell], from: (f32, f32), to: (f32, f32)) -> bool {
    let d = dist(from, to);
    let steps = ((d / 0.15).ceil() as i32).max(1);
    (1..=steps).all(|i| {
        let t = i as f32 / steps as f32;
        body_fits_gated(fac, gates, from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
    })
}

#[derive(PartialEq)]
struct OpenCell(f32, Cell);

impl Eq for OpenCell {}

impl PartialOrd for OpenCell {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for OpenCell {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
    }
}

/// Whether a hunter's body (`HUNTER_RADIUS`) can stand at the centre of each cell for one
/// planning call. The hunter grid is built for the agent radius (`DEFAULT_AGENT_RADIUS`, thinner than the hunter), so a
/// cell it calls open can still be too tight for the body: a route through it strands the hunter at the wall.
/// Memoised per call, since one search asks about the same cell from several neighbours.
pub(super) struct Standable<'a> {
    fac: &'a Facility,
    /// 0 not asked yet, 1 the body fits, 2 it does not (one per grid cell).
    known: RefCell<Vec<u8>>,
}

impl<'a> Standable<'a> {
    pub(super) fn new(fac: &'a Facility) -> Self {
        let cells = (fac.cols.max(0) * fac.rows.max(0)) as usize;
        Standable { fac, known: RefCell::new(vec![0; cells]) }
    }

    pub(super) fn can_stand(&self, c: Cell) -> bool {
        let fac = self.fac;
        if c.x < 0 || c.y < 0 || c.x >= fac.cols || c.y >= fac.rows {
            return false;
        }
        let i = (c.y * fac.cols + c.x) as usize;
        match self.known.borrow()[i] {
            1 => return true,
            2 => return false,
            _ => {}
        }
        let fits = fac.hunter_fits(c.x as f32 + 0.5, c.y as f32 + 0.5);
        self.known.borrow_mut()[i] = if fits { 1 } else { 2 };
        fits
    }
}

/// Whether the grid planner may stand a hunter on `c`: a hunter cell (or a gate channel) that the hunter's body fits,
/// and not a cell the water rules close. Shared by the A* (`plan_route_wet`) and the
/// goal fields of path_scheduler.rs, so both plan over the same cells.
pub(super) fn walk_open(stand: &Standable, c: Cell, gates: &[Cell], water: Option<&WaterPlan>) -> bool {
    let fac = stand.fac;
    (gates.contains(&c) || (fac.hunter_open(c) && stand.can_stand(c)))
        && water.map_or(Some(1.0), |w| w.step_factor((c.x, c.y))).is_some()
}

/// A route over the hunter grid from the cell under `from` to `to`, as `Facility::find_path` plans it, except that the
/// cells in `avoid` (ones a step has just been blocked by) are left out. A closed start cell (the body stands
/// off-centre in it) starts from the nearest open cell beside it.
fn plan_route(fac: &Facility, from: (f32, f32), to: Cell, avoid: &[(Cell, f32)], gates: &[Cell]) -> Vec<Cell> {
    plan_route_wet(fac, from, to, avoid, gates, None)
}

/// `plan_route` with water: each step costs its cell's path cost multiplier (`WaterPlan`), and a cell the hunter will
/// not enter (deep water for a hunter that cannot or will not swim) is closed like a wall.
pub(super) fn plan_route_wet(
    fac: &Facility,
    from: (f32, f32),
    to: Cell,
    avoid: &[(Cell, f32)],
    gates: &[Cell],
    water: Option<&WaterPlan>,
) -> Vec<Cell> {
    let avoided = |c: Cell| avoid.iter().any(|(a, _)| *a == c);
    let wet = |c: Cell| water.map_or(Some(1.0), |w| w.step_factor((c.x, c.y)));
    let stand = Standable::new(fac);
    let open = |c: Cell| walk_open(&stand, c, gates, water) && !avoided(c);
    let here = cell_of(from);
    let start = if open(here) {
        here
    } else {
        let mut near: Vec<Cell> = (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| Cell::new(here.x + dx, here.y + dy)))
            .filter(|c| open(*c))
            .collect();
        near.sort_by(|a, b| dist(a.center(), from).total_cmp(&dist(b.center(), from)));
        match near.first() {
            Some(c) => *c,
            None => return vec![],
        }
    };
    if to == start {
        return vec![start];
    }
    if !open(to) {
        return vec![];
    }
    let h = |c: Cell| {
        let dx = (c.x - to.x).abs() as f32;
        let dy = (c.y - to.y).abs() as f32;
        dx.max(dy) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dy)
    };
    let mut g: HashMap<Cell, f32> = HashMap::from([(start, 0.0)]);
    let mut came: HashMap<Cell, Cell> = HashMap::new();
    let mut closed: HashSet<Cell> = HashSet::new();
    let mut frontier = BinaryHeap::from([OpenCell(h(start), start)]);
    while let Some(OpenCell(_, c)) = frontier.pop() {
        if c == to {
            let mut path = vec![c];
            let mut at = c;
            while let Some(prev) = came.get(&at) {
                at = *prev;
                path.push(at);
            }
            path.reverse();
            return path;
        }
        if !closed.insert(c) {
            continue;
        }
        let gc = g[&c];
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let n = Cell::new(c.x + dx, c.y + dy);
                if closed.contains(&n) || (n != to && !open(n)) {
                    continue;
                }
                if dx != 0
                    && dy != 0
                    && (!open(Cell::new(c.x + dx, c.y)) || !open(Cell::new(c.x, c.y + dy)))
                {
                    continue;
                }
                let step = if dx != 0 && dy != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
                let ng = gc + step * wet(n).unwrap_or(1.0);
                if ng < *g.get(&n).unwrap_or(&f32::INFINITY) {
                    g.insert(n, ng);
                    came.insert(n, c);
                    frontier.push(OpenCell(ng + h(n), n));
                }
            }
        }
    }
    vec![]
}

pub struct Brain {
    pub def: HunterDef,
    pub pos: (f32, f32),
    pub facing: (f32, f32),
    pub state: State,
    pub active: bool,
    pub stamina: f32,
    pub winded: bool,
    pub stun: f32,
    pub stun_heavy: bool,
    pub slow_timer: f32,
    pub slow_factor: f32,
    pub hearing_scale: f32,
    pub idle_timer: f32,
    pub search_timer: f32,
    pub search_visits: i32,
    pub lost_sight: f32,
    sense_timer: f32,
    pub wander_target: Option<(f32, f32)>,
    pub search_target: Option<(f32, f32)>,
    pub last_known: Option<(f32, f32)>,
    pub direct_visible: bool,
    pub ability_direct_sight: bool,
    path: Vec<Cell>,
    path_timer: f32,
    path_goal: Option<Cell>,
    /// Cells a step was just blocked by, with the seconds left to keep them out of the route.
    avoid: Vec<(Cell, f32)>,
    unreachable: f32,
    camp_time: f32,
    camp_cooldown: f32,
    pub rng: Rng,
    // beam attention (the player's torch held on her)
    pub beam_attention: f32,
    pub beam_alerted: bool,
    pub flash_triggered: bool,
    flash_armed: bool,
    pub gimmick: Gimmick,
    /// Walk distance for the stride animation, and the last frame's speed.
    pub stride: f32,
    pub moving_speed: f32,
    /// Model shown instead of her own (Umbriel's disguise).
    pub disguise: Option<String>,
    /// How lit this hunter stands, set by the host a few times a second (None: the light rule is off).
    pub self_exposure: Option<f32>,
    /// The runner's own lamp exposure for this target (None: the light rule is off).
    pub target_exposure: Option<f32>,
    /// Seconds to the host's next refresh of the two exposures above.
    pub light_timer: f32,
    /// The difficulty her peek odds and reaction times are read at.
    pub diff: Difficulty,
    // corner peek (hunter_brain.gd 170-182); the renderer reads the pose fields
    pub peek_phase: PeekPhase,
    pub peek_timer: f32,
    pub peek_body: (f32, f32),
    pub peek_head: (f32, f32),
    /// Down the hall (d2): the way she faces while she looks.
    pub peek_dir: (f32, f32),
    /// Toward the corner edge (d1): the way she leans.
    pub peek_lean: (f32, f32),
    /// +1 when the edge is on her left as she faces down the hall, -1 on her right.
    pub peek_side: i32,
    pub peek_cooldown: f32,
    pub peeks_started: u32,
    /// The state she picks up again when the look finds nothing.
    peek_resume: State,
    /// The corner cell she last stopped at (or rolled for), so one corner is never tried twice in a row.
    peek_corner_cell: Option<Cell>,
    /// A validator's fixed peek chance (peek_chance_override); None: the tuning's.
    pub peek_chance_override: Option<f32>,
    /// Search points picked during this chase (newest last), for `unchecked_first`.
    searched: Vec<(f32, f32)>,
    /// The host chose a player bot (not the runner) as this hunter's target this frame; such a hunter feeds no
    /// danger readout, heartbeat or chase cue.
    pub target_is_bot: bool,
    /// Sluice-gate channel cells that are floor right now (the gate is down); set by the host from the pipes state.
    pub gate_floor: Vec<Cell>,
    /// The emote playing (index into the emote catalogue) and its seconds left; the wire carries the index.
    pub emote: Option<usize>,
    emote_left: f32,
    pub emote_timers: super::emotes::Timers,
    emote_rng: Rng,
    state_time: f32,
    emote_pending: Option<&'static str>,
    blocked_time: f32,
    /// Seconds without a route before the goal is given up (data/hunters/search.json, unreachable_give_up).
    give_up_after: f32,
    /// The water the planner and the speed read (set each frame by game/hunting.rs); dry until set.
    water: Arc<WaterField>,
    water_rules: HunterWater,
    /// The field version the current route was planned against.
    water_seen: u64,
    /// The one-shot mechanic animation raised at a gameplay event and its seconds left (hunters/mechanics.rs).
    cue: Option<(&'static str, f32)>,
    /// A guest's copy of the mechanic name the host sent, and the seconds it is kept without a newer frame.
    net_mech: Option<(&'static str, f32)>,
    /// A guest shows the host's emote index and never starts one.
    pub display_only: bool,
    /// A guest's copy of a host hunter that the guest's own lineup does not have (made by `create_guest_hunter` from a
    /// host frame). It is shown and nothing more: never simulated, stored or restored, and it feeds neither the danger
    /// readout nor the chase cue.
    pub shadow: bool,
    /// The level this hunter stands on, derived from `y` with hysteresis while an edge is walked.
    pub level: i32,
    /// Absolute feet height (m): `floor_y_of(level)` off an edge, the flight or ladder surface on one.
    pub y: f32,
    /// The flight, ladder or drop being walked; None off an edge.
    pub walk: Option<EdgeWalk>,
    /// The level `last_known` is on (read only while `last_known` is the point it was set with).
    pub last_known_level: i32,
    last_known_stamp: Option<(f32, f32)>,
    /// The edge graph across the resident levels; empty until the host sets it.
    nav: Arc<StackNav>,
    /// The hop (edge and entrance cell) the current route heads for, when the goal is on another level.
    hop: Option<Hop>,
    /// Seconds spent on the current edge.
    edge_clock: f32,
}

const ARRIVAL_DISTANCE: f32 = 0.65;
const WAYPOINT_DISTANCE: f32 = 0.18;
/// How long a cell a step was blocked by stays out of a hunter's route.
const BLOCKED_AVOID_SECONDS: f32 = 2.0;
/// How far off the middle of its cell (m) a body may stand before it walks to the middle to get past a corner.
const CENTRE_EPS: f32 = 0.05;
const BEAM_ATTENTION_TIME: f32 = 0.35;
const BEAM_ATTENTION_DECAY: f32 = 1.6;
const BEAM_ATTENTION_RANGE: f32 = 12.0;
const BEAM_ATTENTION_HALF_ANGLE: f32 = 0.28;
/// LightField (game/look/light_field.gd): the exposure a hunter needs to make out a lit runner, and the sight
/// scale of a runner in pitch dark and in a lamp pool.
const LIGHT_VISIBLE_EXPOSURE: f32 = super::light_field::VISIBLE_EXPOSURE;
const LIGHT_DARK_SIGHT_SCALE: f32 = super::light_field::DARK_SIGHT_SCALE;
const LIGHT_LIT_SIGHT_SCALE: f32 = super::light_field::LIT_SIGHT_SCALE;
/// Seconds between the host's refreshes of a hunter's light exposures (game/light_sight.rs; Godot: about 5 a second).
pub const LIGHT_REFRESH: f32 = 0.2;

impl Brain {
    pub fn new(def: HunterDef, spawn: (f32, f32), seed: &str, diff: Difficulty) -> Brain {
        let gimmick = Gimmick::for_id(&def.id, diff);
        let def_id = def.id.clone();
        Brain {
            hearing_scale: def.hearing,
            def,
            pos: spawn,
            facing: (0.0, 1.0),
            state: State::Idle,
            active: false,
            stamina: HUNTER_STAMINA_MAX,
            winded: false,
            stun: 0.0,
            stun_heavy: false,
            slow_timer: 0.0,
            slow_factor: 1.0,
            idle_timer: 0.0,
            search_timer: 0.0,
            search_visits: 0,
            lost_sight: 0.0,
            sense_timer: 0.0,
            wander_target: None,
            search_target: None,
            last_known: None,
            direct_visible: false,
            ability_direct_sight: false,
            path: vec![],
            path_timer: 0.0,
            path_goal: None,
            avoid: vec![],
            unreachable: 0.0,
            camp_time: 0.0,
            camp_cooldown: 0.0,
            rng: Rng::new(seed),
            beam_attention: 0.0,
            beam_alerted: false,
            flash_triggered: false,
            flash_armed: true,
            gimmick,
            stride: 0.0,
            moving_speed: 0.0,
            disguise: None,
            self_exposure: None,
            target_exposure: None,
            light_timer: 0.0,
            diff,
            peek_phase: PeekPhase::Settle,
            peek_timer: 0.0,
            peek_body: spawn,
            peek_head: spawn,
            peek_dir: (0.0, 1.0),
            peek_lean: (0.0, 0.0),
            peek_side: 1,
            peek_cooldown: 0.0,
            peeks_started: 0,
            peek_resume: State::Search,
            peek_corner_cell: None,
            peek_chance_override: None,
            searched: Vec::new(),
            target_is_bot: false,
            gate_floor: Vec::new(),
            emote: None,
            emote_left: 0.0,
            emote_timers: super::emotes::Timers::default(),
            emote_rng: Rng::new(&format!("{seed}:emote")),
            state_time: 0.0,
            emote_pending: None,
            blocked_time: 0.0,
            give_up_after: search_rules().unreachable_give_up,
            water: Arc::new(WaterField::default()),
            water_rules: WaterRules::get().for_hunter(&def_id),
            water_seen: 0,
            cue: None,
            net_mech: None,
            display_only: false,
            shadow: false,
            level: 0,
            y: 0.0,
            walk: None,
            last_known_level: 0,
            last_known_stamp: None,
            nav: Arc::new(StackNav::empty()),
            hop: None,
            edge_clock: 0.0,
        }
    }

    /// Stand on `level`'s floor (a spawn, a respawn or a list swap): level and y together, and no edge in progress.
    pub fn set_level(&mut self, level: i32) {
        self.level = level;
        self.y = crate::game::level_stack::floor_y_of(level);
        self.walk = None;
        self.hop = None;
        self.edge_clock = 0.0;
        self.last_known_level = level;
        self.last_known_stamp = None;
    }

    /// The edge graph this brain plans through. A changed graph version drops the route timer at once; an edge walk
    /// in progress keeps going (it owns its own edge).
    pub fn set_nav(&mut self, nav: Arc<StackNav>) {
        if nav.version() != self.nav.version() {
            self.path_timer = 0.0;
            self.hop = None;
        }
        self.nav = nav;
    }

    /// Investigate `p` on `level`: the hunter walks to the nearest flight when that is not its own level.
    pub fn investigate_on(&mut self, p: (f32, f32), level: i32) {
        self.investigate(p);
        self.set_known_level(p, level);
    }

    /// True while a flight, ladder or drop is being walked.
    pub fn on_edge(&self) -> bool {
        self.walk.is_some()
    }

    fn set_known_level(&mut self, p: (f32, f32), level: i32) {
        self.last_known = Some(p);
        self.last_known_level = level;
        self.last_known_stamp = Some(p);
    }

    /// The level the current goal is on. `last_known_level` counts only while `last_known` is still the point it was
    /// set with; any other write to `last_known` (a gimmick's, a shift of the window) means the hunter's own level.
    fn goal_level(&self) -> i32 {
        match self.state {
            State::Chase | State::Investigate if self.last_known.is_some() && self.last_known == self.last_known_stamp => {
                self.last_known_level
            }
            _ => self.level,
        }
    }

    /// applyRelayActivation(): wake investigating where the runner stood.
    pub fn activate_at(&mut self, p: (f32, f32)) {
        self.active = true;
        self.state = State::Investigate;
        self.last_known = Some(p);
        self.direct_visible = false;
    }

    pub fn capture_reach(&self, prof: &Profile) -> f32 {
        (PLAYER_RADIUS + HUNTER_RADIUS + HUNTER_GRAB_REACH) * prof.capture_distance
    }

    /// The water this brain plans and moves through. A changed field version drops the route timer at once, so a valve
    /// that floods the way, or a sluice that drains it, is planned again on the next step.
    pub fn set_water(&mut self, w: Arc<WaterField>) {
        if w.version() != self.water.version() {
            self.path_timer = 0.0;
        }
        self.water = w;
    }

    /// The splash noise of this hunter's step where it stands (0 on dry ground), for the caller to emit as a sound.
    pub fn water_splash_noise(&self) -> f32 {
        let c = cell_of(self.pos);
        self.water_rules.splash_noise(&self.water, (c.x, c.y))
    }

    /// True when the straight line to `goal` crosses no cell this hunter will not enter (so a chase may walk it
    /// directly instead of following the planned route round the water).
    fn water_line_ok(&self, goal: (f32, f32)) -> bool {
        if self.water.is_dry() {
            return true;
        }
        let plan = WaterPlan { field: &self.water, rules: &self.water_rules, chasing: self.state == State::Chase };
        let d = dist(self.pos, goal);
        let n = (d / 0.4).ceil().max(1.0) as i32;
        (0..=n).all(|i| {
            let t = i as f32 / n as f32;
            let c = cell_of((self.pos.0 + (goal.0 - self.pos.0) * t, self.pos.1 + (goal.1 - self.pos.1) * t));
            plan.step_factor((c.x, c.y)).is_some()
        })
    }

    pub fn clear_path(&mut self) {
        self.path.clear();
        self.path_goal = None;
    }

    pub fn apply_stun(&mut self, s: f32, heavy: bool) {
        // a stun cuts the mechanic animation short: it does not come back when she gets up
        if s > 0.0 {
            self.cue = None;
        }
        if s > self.stun {
            self.stun_heavy = heavy;
        }
        self.stun = self.stun.max(s);
        self.clear_path();
        self.direct_visible = false;
    }

    /// Taser slowdown (game/hunters/ai/hunter_brain.gd apply_slow, lines 476-482): movement speed times `factor`
    /// (clamped to 0..1) for `seconds`. A slow that is already running keeps the stronger (lower) factor, and the
    /// longer time wins. A weaker hit never weakens an active slow.
    pub fn apply_slow(&mut self, seconds: f32, factor: f32) {
        let factor = factor.clamp(0.0, 1.0);
        if self.slow_timer <= 0.0 {
            self.slow_factor = factor;
        } else {
            self.slow_factor = self.slow_factor.min(factor);
        }
        self.slow_timer = self.slow_timer.max(seconds);
    }

    /// Runs the slow clock down (hunter_brain.gd _tick_slow, lines 484-490). When it runs out the factor resets to 1.0.
    pub fn tick_slow(&mut self, dt: f32) {
        if self.slow_timer > 0.0 {
            self.slow_timer = (self.slow_timer - dt).max(0.0);
            if self.slow_timer <= 0.0 {
                self.slow_factor = 1.0;
            }
        }
    }

    /// Raises the one-shot mechanic animation `name` (one of `mechanics::CUES`) for its `seconds` in
    /// data/hunters/mechanic_cues.json. A cue already running is replaced (the newest event wins); a name that is not a
    /// cue is ignored. It never moves or freezes the hunter.
    pub fn raise_cue(&mut self, name: &'static str) {
        if super::mechanics::CUES.contains(&name) {
            self.cue = Some((name, super::mechanics::cues().seconds(name)));
        }
    }

    /// The cue running now.
    pub fn cue(&self) -> Option<&'static str> {
        self.cue.map(|c| c.0)
    }

    /// Runs the cue clock down. Called at the top of every `update`, so a stunned or dormant hunter's cue still runs out.
    pub fn tick_cue(&mut self, dt: f32) {
        if let Some((_, left)) = self.cue.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.cue = None;
            }
        }
        // a brain that simulates (the host's, or a guest's that became one) never keeps a name a host once sent
        self.tick_net_mechanic(dt);
    }

    /// Guest: the mechanic name the host's frame carried (None when it carried none), kept `net_ttl` seconds.
    pub fn set_net_mechanic(&mut self, name: Option<&str>) {
        self.net_mech = name.and_then(super::mechanics::intern).map(|n| (n, super::mechanics::cues().net_ttl));
    }

    /// Guest: runs the kept name down (a guest runs no `update`), so a frame that never comes cannot freeze a pose.
    pub fn tick_net_mechanic(&mut self, dt: f32) {
        if let Some((_, left)) = self.net_mech.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.net_mech = None;
            }
        }
    }

    /// The mechanic animation she is in right now, for the view and the host's hunter frame: the ladder climb, else the
    /// one-shot cue, else what her gimmick is doing. A guest's copy (`display_only`, or a `shadow`) runs no gimmick, so
    /// its idle gimmick must not speak (Glyra's sits at "box playing"): only the name the host last sent counts there.
    pub fn mechanic(&self) -> Option<&'static str> {
        use super::flight_edges::EdgeKind;
        if self.display_only || self.shadow {
            return self.net_mech.map(|m| m.0);
        }
        if matches!(self.walk.as_ref().map(|w| w.kind), Some(EdgeKind::Ladder)) {
            return Some("climb");
        }
        self.cue().or_else(|| self.gimmick.mechanic(self)).or_else(|| self.net_mech.map(|m| m.0))
    }

    pub fn investigate(&mut self, p: (f32, f32)) {
        self.state = State::Investigate;
        self.last_known = Some(p);
        self.clear_path();
        self.direct_visible = false;
    }

    /// path_look_ahead (hunter_brain.gd 565): the point `d` cells along the current route
    /// through cell centres, or None without a route (the head look then follows travel).
    pub fn path_look_ahead(&self, d: f32) -> Option<(f32, f32)> {
        if self.path.is_empty() {
            return None;
        }
        let mut from = self.pos;
        let mut left = d;
        for c in &self.path {
            let p = c.center();
            let seg = dist(from, p);
            if seg >= left {
                let t = left / seg.max(0.001);
                return Some((from.0 + (p.0 - from.0) * t, from.1 + (p.1 - from.1) * t));
            }
            left -= seg;
            from = p;
        }
        Some(from)
    }

    /// direct_to (the director's order): investigate `point`, dropping the current path.
    pub fn direct_to(&mut self, point: (f32, f32)) {
        self.state = State::Investigate;
        self.last_known = Some(point);
        self.clear_path();
        self.direct_visible = false;
    }

    pub fn chase(&mut self, p: (f32, f32)) {
        self.state = State::Chase;
        self.last_known = Some(p);
        self.lost_sight = 0.0;
    }

    pub fn enter_search_from(&mut self, p: (f32, f32), duration: f32, fac: &Facility) {
        self.state = State::Search;
        self.last_known = Some(p);
        self.search_timer = duration;
        self.search_visits = 0;
        self.searched.clear();
        self.search_target = Some(self.pick_search_target(fac));
        self.clear_path();
    }

    pub fn next_float(&mut self) -> f32 {
        self.rng.next_f32()
    }

    pub fn next_index(&mut self, n: usize) -> usize {
        self.rng.next_index(n)
    }

    /// One frame. Returns whether the runner was caught and the gimmick's events.
    pub fn update(
        &mut self,
        dt: f32,
        ctx: &Ctx,
        target: &Target,
        depth: i32,
        relays: i32,
        prof: &Profile,
    ) -> (bool, Vec<Event>) {
        let mut events = vec![];
        self.tick_slow(dt);
        self.tick_cue(dt);
        let state_before = self.state;
        self.moving_speed = 0.0;
        let mut g = std::mem::take(&mut self.gimmick);
        let self_summoning = g.runs_while_inactive();
        if self_summoning {
            g.before_perception(self, dt, ctx, target, &mut events);
        }
        if !self.active {
            self.gimmick = g;
            return (false, events);
        }
        if self.stun > 0.0 {
            self.stun = (self.stun - dt).max(0.0);
            self.gimmick = g;
            return (false, events);
        }
        let ignoring = self.camp_guard(dt, target, prof);
        self.ability_direct_sight = false;
        if !self_summoning {
            g.before_perception(self, dt, ctx, target, &mut events);
        }
        self.update_beam_attention(dt, ctx.fac, target);
        if !ignoring {
            self.perceive(dt, ctx.fac, target, prof, &g);
        }
        g.after_perception(self, dt, ctx, target, &mut events);
        self.update_stamina(dt, prof);
        let holds = g.holds_position(self);
        self.peek_cooldown = (self.peek_cooldown - dt).max(0.0);
        if self.walk.is_none() {
            self.consider_peek(ctx.fac, &g, holds);
        }
        let speed_scale = g.speed_scale(self);
        self.update_movement(dt, ctx.fac, depth, relays, prof, holds, speed_scale);
        self.follow_terrain(ctx.fac);
        self.tick_emotes(dt, state_before, dist(self.pos, target.pos));
        let breaches = g.breaches_hiding();
        let immune = g.immune_to_capture(self);
        self.gimmick = g;
        let may_grab_hidden = target.hidden
            && breaches
            && self
                .last_known
                .map(|k| dist(k, target.pos) < 0.9)
                .unwrap_or(false);
        let caught = (!target.hidden || may_grab_hidden)
            && !target.in_safe_zone
            && !immune
            && dist(self.pos, target.pos) < self.capture_reach(prof)
            && (self.y - target.y).abs() <= flight_edges::rules().capture_height;
        (caught, events)
    }

    /// On the Inferno's noisy floor (`world/inferno_terrain.rs`) a hunter on foot stands on the ground under it, so its
    /// feet follow the surface the player walks on. An edge walk (a flight, a ladder, a drop) owns its own height.
    fn follow_terrain(&mut self, fac: &Facility) {
        if self.walk.is_none() && fac.level == self.level && fac.terrain().is_some() {
            self.y = crate::game::level_stack::floor_y_of(self.level) + fac.ground_height(self.pos.0, self.pos.1);
        }
    }

    /// Set the open sluice-gate channel cells: `cells` become floor for this hunter while `gate_down` (the valve
    /// turned an odd number of times, `Pipes::is_open`), and walls again otherwise.
    pub fn set_gate_floor(&mut self, cells: &[Cell], gate_down: bool) {
        self.gate_floor = gate_floor_cells(cells, gate_down);
    }

    /// Emote wiring (hunter_emoter.gd): a state change fires its transition rule (`net::wiring::transition_rule`),
    /// the chase and idle rules roll per second, and the playing emote counts down. Host only: a guest hunter never
    /// starts one. The rolls come from the hunter's own emote stream, so no other roll shifts.
    fn tick_emotes(&mut self, dt: f32, prev: State, runner_dist: f32) {
        self.emote_timers.tick(dt);
        if self.emote_left > 0.0 {
            self.emote_left -= dt;
            if self.emote_left <= 0.0 {
                self.emote = None;
            }
        }
        if self.display_only {
            return;
        }
        if self.state != prev {
            self.state_time = 0.0;
            self.emote_pending = crate::net::wiring::transition_rule(prev.name(), self.state.name());
        } else {
            self.state_time += dt;
        }
        let cat = super::emotes::Catalogue::get();
        let busy = self.emote.is_some() || self.stun > 0.0;
        let mk = |me: &Self| super::emotes::Ctx { dt, time_in_state: me.state_time, distance: runner_dist, busy };
        let mut fired = None;
        if let Some(rule) = self.emote_pending {
            let min = cat.rule(rule).map_or(0.0, |r| r.min_seconds);
            if self.state_time >= min {
                self.emote_pending = None;
                let (gate, pick) = (self.emote_rng.next_float() as f32, self.emote_rng.next_float() as f32);
                let c = mk(self);
                fired = cat.try_trigger(&mut self.emote_timers, &self.def.id, rule, self.diff.id(), c, gate, pick);
            }
        }
        if fired.is_none() {
            let per_tick = match self.state {
                State::Chase => Some("chase"),
                State::Idle => Some("idle"),
                _ => None,
            };
            if let Some(rule) = per_tick {
                let (gate, pick) = (self.emote_rng.next_float() as f32, self.emote_rng.next_float() as f32);
                let c = mk(self);
                fired = cat.try_trigger(&mut self.emote_timers, &self.def.id, rule, self.diff.id(), c, gate, pick);
            }
        }
        if let Some(i) = fired {
            self.emote = Some(i);
            self.emote_left = cat.emote(i).map_or(1.0, |e| e.seconds);
        }
    }

    fn camp_guard(&mut self, dt: f32, target: &Target, prof: &Profile) -> bool {
        self.camp_cooldown = (self.camp_cooldown - dt).max(0.0);
        if !target.hidden || dist(self.pos, target.pos) > CAMP_RADIUS {
            self.camp_time = (self.camp_time - dt * 2.0).max(0.0);
            return false;
        }
        if self.camp_cooldown > 0.0 {
            return true;
        }
        self.camp_time += dt;
        if self.camp_time <= prof.camp_limit {
            return false;
        }
        self.camp_time = 0.0;
        self.camp_cooldown = CAMP_COOLDOWN;
        self.state = State::Wander;
        self.last_known = None;
        self.search_target = None;
        self.direct_visible = false;
        self.lost_sight = 0.0;
        self.clear_path();
        let d = dist(self.pos, target.pos);
        let away = if d > 0.05 {
            (
                (self.pos.0 - target.pos.0) / d,
                (self.pos.1 - target.pos.1) / d,
            )
        } else {
            (1.0, 0.0)
        };
        self.wander_target = Some((
            self.pos.0 + away.0 * CAMP_LEAVE_DISTANCE,
            self.pos.1 + away.1 * CAMP_LEAVE_DISTANCE,
        ));
        true
    }

    /// beam_reaches: within range, inside the half angle of `facing`, with line of sight.
    pub fn beam_reaches(
        fac: &Facility,
        from: (f32, f32),
        facing: (f32, f32),
        to: (f32, f32),
        range: f32,
        half_angle: f32,
    ) -> bool {
        let d = dist(from, to);
        let fl = (facing.0 * facing.0 + facing.1 * facing.1).sqrt();
        if d <= 0.001 || d > range || fl <= 0.001 {
            return false;
        }
        let dot = ((to.0 - from.0) / d) * (facing.0 / fl) + ((to.1 - from.1) / d) * (facing.1 / fl);
        dot >= half_angle.cos() && fac.has_line_of_sight(from, to)
    }

    fn update_beam_attention(&mut self, dt: f32, fac: &Facility, target: &Target) {
        let lit = target.flashlight_on
            && !target.hidden
            && Self::beam_reaches(
                fac,
                target.pos,
                target.facing,
                self.pos,
                BEAM_ATTENTION_RANGE,
                BEAM_ATTENTION_HALF_ANGLE,
            );
        let prev = self.beam_attention;
        if lit {
            self.beam_attention = (self.beam_attention + dt).min(BEAM_ATTENTION_TIME);
        } else {
            self.beam_attention = (self.beam_attention - dt * BEAM_ATTENTION_DECAY).max(0.0);
        }
        if self.beam_attention <= 0.0 {
            self.flash_armed = true;
        }
        self.beam_alerted = lit && self.beam_attention >= BEAM_ATTENTION_TIME;
        self.flash_triggered = self.beam_alerted && prev < BEAM_ATTENTION_TIME && self.flash_armed;
        if self.flash_triggered {
            self.flash_armed = false;
        }
    }

    /// True when the host should refresh `self_exposure` and `target_exposure` now (about 5 times a second); `stagger`
    /// spreads the hunters of a crowd over the cadence.
    pub fn light_due(&mut self, dt: f32, stagger: f32) -> bool {
        self.light_timer -= dt;
        if self.light_timer > 0.0 {
            return false;
        }
        self.light_timer = LIGHT_REFRESH + stagger;
        true
    }

    pub fn sight_limit(&self, flashlight_on: bool, prof: &Profile, g: &Gimmick) -> f32 {
        let s = self.def.sight;
        let mut limit = if flashlight_on {
            FLASHLIGHT_SIGHT_MIN.max(s * FLASHLIGHT_SIGHT_SCALE) * prof.hunter_sight
        } else {
            let scale = match self.target_exposure {
                Some(e) => light_unlit_scale(e),
                None => UNLIT_SIGHT_SCALE,
            };
            s * scale * prof.hunter_sight
        };
        // mutual light: a hunter standing in light the runner can see her by also sees the runner at full range
        let lit_here = matches!(self.self_exposure, Some(e) if e >= LIGHT_VISIBLE_EXPOSURE);
        if lit_here {
            limit = limit.max(s * prof.hunter_sight);
        }
        limit * g.sight_scale()
    }

    fn perceive(&mut self, dt: f32, fac: &Facility, target: &Target, prof: &Profile, g: &Gimmick) {
        let eye = self.sight_eye();
        let direct = dist(eye, target.pos);
        let can_sense = !target.in_safe_zone && (!target.hidden || g.breaches_hiding());
        if !can_sense {
            self.direct_visible = false;
        }
        self.sense_timer -= dt;
        // A target on another level is sensed through `cross` (the host's 3D sight and the adjacent hearing scale).
        let cross = target.cross;
        let target_level = if cross.is_some() { target.level } else { self.level };
        if self.sense_timer > 0.0 {
            if self.state == State::Chase && can_sense {
                self.set_known_level(target.pos, target_level);
            }
            return;
        }
        self.sense_timer = HUNTER_SENSE_INTERVAL;
        let sight_limit = self.sight_limit(target.flashlight_on, prof, g);
        let hearing_limit = target.noise * self.hearing_scale * prof.hunter_hearing * cross.map_or(1.0, |c| c.hear_scale);
        let mut sees = false;
        let mut line_clear = false;
        if can_sense {
            let ray = sight_limit.max(hearing_limit * HEARING_OPEN_AIR_SCALE);
            if direct < ray {
                line_clear = match cross {
                    Some(c) => c.sees,
                    None => fac.has_line_of_sight(eye, target.pos),
                };
            }
            sees = direct < sight_limit && line_clear;
        }
        let mut hears = false;
        if !sees && can_sense {
            if line_clear && direct <= hearing_limit * HEARING_OPEN_AIR_SCALE {
                hears = true;
            } else if direct < hearing_limit * HEARING_OUTER_SCALE {
                if cross.is_some() {
                    // Another floor: heard when a flight leads there.
                    hears = self.hop_to(fac, target.pos, target.level).is_some();
                } else {
                    let priority = self.plan_priority(target.pos);
                    let Some(p) = self.grid_route(fac, cell_of(target.pos), priority, false) else {
                        // the route search is queued for the next frame: hear the runner then (the sense clock is due again)
                        self.sense_timer = 0.0;
                        return;
                    };
                    hears = !p.is_empty() && p.len() <= hearing_limit.ceil() as usize;
                    if hears && self.state == State::Chase {
                        self.path = p;
                        self.path_timer = HUNTER_REPATH_CHASE;
                        self.path_goal = Some(cell_of(target.pos));
                    }
                }
            }
        }
        if self.state == State::Peek {
            self.peek_perceive(sees, hears, target.pos);
            return;
        }
        if sees {
            self.state = State::Chase;
            self.set_known_level(target.pos, target_level);
            self.lost_sight = 0.0;
            // The straight dash needs the same floor; across levels the hunter walks the flight.
            self.direct_visible = cross.is_none();
        } else if hears {
            self.set_known_level(target.pos, target_level);
            self.direct_visible = false;
            if self.state == State::Chase {
                self.lost_sight = 0.0;
            } else {
                self.state = State::Investigate;
            }
        } else if self.state == State::Chase {
            self.direct_visible = self.ability_direct_sight;
            self.lost_sight += HUNTER_SENSE_INTERVAL;
            if self.lost_sight > prof.lost_sight_grace {
                self.state = State::Investigate;
                self.direct_visible = false;
            }
        } else {
            self.direct_visible = false;
        }
    }

    fn update_stamina(&mut self, dt: f32, prof: &Profile) {
        if self.state == State::Chase {
            self.stamina = (self.stamina - prof.stamina_drain * dt).max(0.0);
            if self.stamina <= 0.0 {
                self.winded = true;
            }
        } else {
            self.stamina = (self.stamina + HUNTER_STAMINA_REGEN * dt).min(HUNTER_STAMINA_MAX);
            if self.winded && self.stamina >= HUNTER_STAMINA_RECOVER {
                self.winded = false;
            }
        }
    }

    pub fn current_speed(
        &self,
        depth: i32,
        relays: i32,
        prof: &Profile,
        gimmick_scale: f32,
    ) -> f32 {
        let depth_boost = HUNTER_DEPTH_SPEED_BONUS_MAX
            .min((depth.max(1) - 1) as f32 * HUNTER_DEPTH_SPEED_BONUS_PER_LEVEL);
        let rules = crate::game::progression::Progression::load();
        let prog = rules.chase_speed_bonus(relays) + depth_boost;
        let mut speed = if self.state == State::Chase {
            // data/progression.json chase_speed_scale: one global knob over every hunter's own `chase` value. The stride
            // animation and the footfalls follow the measured ground speed, so they speed up with it.
            (self.def.chase + prog * self.def.prog) * rules.chase_scale_for(&self.def.category)
        } else {
            self.def.speed + prog
        };
        if self.winded {
            speed *= HUNTER_WINDED_SPEED_SCALE;
        }
        speed *= gimmick_scale;
        if self.slow_timer > 0.0 {
            speed *= self.slow_factor;
        }
        let c = cell_of(self.pos);
        speed * prof.hunter_speed * self.water_rules.speed_scale(&self.water, (c.x, c.y))
    }

    #[allow(clippy::too_many_arguments)]
    fn update_movement(
        &mut self,
        dt: f32,
        fac: &Facility,
        depth: i32,
        relays: i32,
        prof: &Profile,
        holds: bool,
        gscale: f32,
    ) {
        if self.walk.is_some() {
            let speed = self.current_speed(depth, relays, prof, gscale);
            self.walk_edge(dt, fac, speed, prof);
            return;
        }
        match self.state {
            State::Peek => {
                let speed = self.current_speed(depth, relays, prof, gscale);
                self.update_peek(dt, fac, prof, speed);
                return;
            }
            State::Idle => {
                self.idle_timer -= dt;
                if self.idle_timer <= 0.0 {
                    self.state = State::Wander;
                    self.wander_target = Some(self.pick_wander_target(fac));
                }
                return;
            }
            State::Search => {
                self.search_timer -= dt;
                if self.search_timer <= 0.0 {
                    self.state = State::Idle;
                    self.last_known = None;
                    self.idle_timer = 0.6 + self.next_float() * 1.4;
                    self.clear_path();
                    return;
                }
            }
            _ => {}
        }
        if holds {
            return;
        }
        let goal = match self.state {
            State::Wander => self.wander_target,
            State::Chase | State::Investigate => self.last_known,
            State::Search => self.search_target,
            State::Idle | State::Peek => None,
        };
        let Some(goal) = goal else { return };
        let speed = self.current_speed(depth, relays, prof, gscale);
        self.pursue(goal, speed, dt, fac, prof);
        if self.walk.is_none() && self.level == self.goal_level() && dist(self.pos, goal) <= ARRIVAL_DISTANCE {
            self.handle_arrival(fac, prof);
        }
    }

    /// The first hop of the cheapest way to `goal` on `goal_level`, costing own-level cells like `plan_route_wet`
    /// (water and gates); cells on other levels cost 1.
    fn hop_to(&self, fac: &Facility, goal: (f32, f32), goal_level: i32) -> Option<Hop> {
        // the secret wolf spiders never leave their home level: no flight, ladder or drop is planned for them
        if goal_level != self.level && super::secret_creatures::stays_home(&self.def.id) {
            return None;
        }
        let plan = WaterPlan { field: &self.water, rules: &self.water_rules, chasing: self.state == State::Chase };
        let own = self.level;
        let gates = &self.gate_floor;
        let step_cost = |level: i32, _from: Cell, c: Cell| -> Option<f32> {
            if level != own {
                return Some(1.0);
            }
            if !(fac.hunter_open(c) || gates.contains(&c)) {
                return None;
            }
            plan.step_factor((c.x, c.y))
        };
        // a body standing off-centre in a closed cell (the corner of a well beside a flight's landing, a cell the grid
        // closes but the body fits) plans from the open cell beside it, as `plan_route_wet` does
        let here = cell_of(self.pos);
        let start = if fac.hunter_open(here) || gates.contains(&here) {
            here
        } else {
            (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| Cell::new(here.x + dx, here.y + dy)))
                .filter(|c| fac.hunter_open(*c))
                .min_by(|a, b| dist(a.center(), self.pos).total_cmp(&dist(b.center(), self.pos)))
                .unwrap_or(here)
        };
        self.nav.next_hop(
            Node { level: self.level, cell: start },
            Node { level: goal_level, cell: cell_of(goal) },
            &step_cost,
        )
    }

    /// One frame on an edge: the walk alone moves the body (at the hunter's own speed, no climb animation), the level
    /// follows the height, and a hunter held on the edge past its time is put on the nearer end and gives the goal up.
    fn walk_edge(&mut self, dt: f32, fac: &Facility, speed: f32, prof: &Profile) {
        let Some(walk) = self.walk.as_ref() else { return };
        let len = walk.len;
        let (from, to) = (walk.from, walk.to);
        let (start_y, end_y) = (walk.start_point()[2], walk.end_point()[2]);
        let limit_speed = walk.speed(speed, self.nav.rules());
        // The footprint test is for a flight's treads. A ladder shaft and a drop's fall are vertical paths built by
        // the graph (there is no walk surface between the floors to test), so only a flight is checked.
        let flight = walk.kind == flight_edges::EdgeKind::Flight;
        self.edge_clock += dt;
        let mut next = walk.clone();
        let pose = next.advance(speed, dt, self.nav.rules());
        let (done, x, z, y) = (pose.done, pose.x, pose.z, pose.y);
        let prev = self.pos;
        if done {
            self.pos = to.cell.center();
            // the walk's own end height: the floor of the level it ends on
            self.y = next.end_point()[2];
            self.level = to.level;
            self.walk = None;
            self.hop = None;
            self.edge_clock = 0.0;
            self.clear_path();
            self.path_timer = 0.0;
            self.moving_speed = speed;
            self.stride += dist(prev, self.pos).max(speed * dt);
            return;
        }
        if !flight || super::navigation::on_flight(fac, x, z, y) {
            self.pos = (x, z);
            self.y = y;
            self.level = crate::game::level_stack::level_at(self.y, self.level);
            self.walk = Some(next);
            let d = dist(prev, self.pos);
            if d > 1e-4 {
                self.facing = ((self.pos.0 - prev.0) / d, (self.pos.1 - prev.1) / d);
            }
            // A ladder moves only in y, so the stride runs at the walk speed either way.
            self.stride += speed * dt;
            self.moving_speed = speed;
        } else {
            self.moving_speed = 0.0;
        }
        let limit = len / limit_speed.max(0.2) + self.nav.rules().edge_stuck_seconds;
        if self.edge_clock > limit {
            let (from_y, to_y) = (start_y, end_y);
            let end = if (self.y - from_y).abs() <= (to_y - self.y).abs() { from } else { to };
            self.pos = end.cell.center();
            self.y = if end == from { from_y } else { to_y };
            self.level = end.level;
            self.walk = None;
            self.hop = None;
            self.edge_clock = 0.0;
            self.moving_speed = 0.0;
            self.give_up_goal(fac, prof);
        }
    }

    fn handle_arrival(&mut self, fac: &Facility, prof: &Profile) {
        match self.state {
            State::Wander => {
                self.state = State::Idle;
                self.idle_timer = 0.7 + self.next_float() * 1.6;
                self.wander_target = None;
            }
            State::Chase | State::Investigate => {
                self.state = State::Search;
                self.search_timer = prof.search_time;
                self.search_visits = 0;
                self.searched.clear();
                self.search_target = Some(self.pick_search_target(fac));
            }
            State::Search => {
                self.search_visits += 1;
                self.search_target = Some(self.pick_search_target(fac));
            }
            State::Idle | State::Peek => {}
        }
    }

    fn clear_sweep(&self, fac: &Facility, from: (f32, f32), to: (f32, f32)) -> bool {
        sweep_clear(fac, &self.gate_floor, from, to)
    }

    pub fn move_with_collision(&mut self, dx: f32, dy: f32, fac: &Facility) -> f32 {
        let total = (dx * dx + dy * dy).sqrt();
        if total <= 0.0 {
            return 0.0;
        }
        let start = self.pos;
        let steps = ((total / 0.11).ceil() as i32).max(1);
        let (sx, sy) = (dx / steps as f32, dy / steps as f32);
        for _ in 0..steps {
            if body_fits_gated(fac, &self.gate_floor, self.pos.0 + sx, self.pos.1) {
                self.pos.0 += sx;
            }
            if body_fits_gated(fac, &self.gate_floor, self.pos.0, self.pos.1 + sy) {
                self.pos.1 += sy;
            }
        }
        dist(self.pos, start)
    }

    pub fn step_toward(&mut self, point: (f32, f32), travel: f32, fac: &Facility, dt: f32) -> f32 {
        let d = dist(point, self.pos);
        if d <= 0.001 {
            return 0.0;
        }
        let travel = travel.min(d);
        let dir = ((point.0 - self.pos.0) / d, (point.1 - self.pos.1) / d);
        self.facing = dir;
        let moved = self.move_with_collision(dir.0 * travel, dir.1 * travel, fac);
        self.stride += moved;
        if dt > 0.0 {
            self.moving_speed = moved / dt;
        }
        moved
    }

    /// The goal cannot be reached (no route for UNREACHABLE_GIVE_UP seconds, or every step toward it blocked for that
    /// long, the dead-end case): a chaser or investigator starts to search, a searcher picks a new point, a wanderer
    /// falls back to idle. A hunter never keeps pushing at a dead end past that time.
    fn give_up_goal(&mut self, fac: &Facility, prof: &Profile) {
        self.unreachable = 0.0;
        self.blocked_time = 0.0;
        self.avoid.clear();
        match self.state {
            State::Chase | State::Investigate => {
                self.last_known = None;
                self.state = State::Search;
                self.search_timer = prof.search_time;
                self.search_visits = 0;
                self.search_target = Some(self.pick_search_target(fac));
            }
            State::Search => self.search_target = Some(self.pick_search_target(fac)),
            State::Wander => {
                self.state = State::Idle;
                self.wander_target = None;
                self.idle_timer = 0.25 + self.next_float() * 0.5;
            }
            _ => {}
        }
        self.clear_path();
        self.path_goal = None;
    }

    /// The grid route from the body to `to`, through the route scheduler (path_scheduler.rs). `water` plans it with this
    /// hunter's water rules (the walk) or dry (the hearing and search checks). None: the frame's search budget is spent
    /// and the answer is queued for the next frame.
    fn grid_route(&self, fac: &Facility, to: Cell, priority: u64, water: bool) -> Option<Vec<Cell>> {
        let water = water.then_some((&self.water, &self.water_rules, self.state == State::Chase));
        let request = PlanRequest { from: self.pos, to, avoid: &self.avoid, gates: &self.gate_floor, water, priority };
        match path_scheduler::plan(fac, request) {
            Route::Ready(path) => Some(path),
            Route::Deferred => None,
        }
    }

    /// The order a queued route request is answered in when the budget is short: chasing hunters first, then by how far
    /// the goal is (in centimetres), so the nearest work is done first.
    fn plan_priority(&self, goal: (f32, f32)) -> u64 {
        let class: u64 = match self.state {
            State::Chase => 0,
            State::Investigate => 1,
            State::Search => 2,
            State::Wander => 3,
            _ => 4,
        };
        (class << 40) | ((dist(self.pos, goal) * 100.0).clamp(0.0, 1e12) as u64)
    }

    fn pursue(&mut self, goal: (f32, f32), speed: f32, dt: f32, fac: &Facility, prof: &Profile) {
        let mut target_cell = cell_of(goal);
        let goal_level = self.goal_level();
        let cross = goal_level != self.level;
        if !cross {
            self.hop = None;
        }
        self.path_timer -= dt;
        for a in self.avoid.iter_mut() {
            a.1 -= dt;
        }
        self.avoid.retain(|a| a.1 > 0.0);
        if !cross
            && self.state == State::Chase
            && self.direct_visible
            && self.clear_sweep(fac, self.pos, goal)
            && self.water_line_ok(goal)
        {
            self.clear_path();
            self.step_toward(goal, speed * HUNTER_VISIBLE_CHASE_SPEED_SCALE * dt, fac, dt);
            return;
        }
        let interval = if self.state == State::Chase {
            HUNTER_REPATH_CHASE
        } else {
            HUNTER_REPATH_WANDER
        };
        if self.water.version() != self.water_seen {
            self.water_seen = self.water.version();
            self.path_timer = 0.0;
        }
        let mut no_hop = false;
        let mut deferred = false;
        if self.path_timer <= 0.0 || self.path.is_empty() || self.path_goal.is_none() {
            if cross {
                self.hop = self.hop_to(fac, goal, goal_level);
                no_hop = self.hop.is_none();
            }
            if let Some(h) = self.hop.as_ref().filter(|_| cross) {
                target_cell = h.entrance;
            }
            let planned = if no_hop { Some(vec![]) } else { self.grid_route(fac, target_cell, self.plan_priority(goal), true) };
            match planned {
                Some(path) => {
                    self.path = path;
                    self.path_timer = interval;
                    self.path_goal = Some(target_cell);
                }
                None => {
                    // the frame's search budget is spent: walk the route there is and ask again next frame
                    deferred = true;
                    self.path_timer = 0.0;
                }
            }
        } else if cross {
            match self.hop.as_ref() {
                Some(h) => target_cell = h.entrance,
                None => no_hop = true,
            }
        }
        let enter_radius = self.nav.rules().edge_enter_radius;
        let enter = self
            .hop
            .as_ref()
            .filter(|h| cross && dist(self.pos, h.entrance.center()) <= enter_radius)
            .map(|h| self.nav.start_walk(self.level, h));
        if let Some(started) = enter {
            let Some(walk) = started else {
                // the edge is gone (a nav rebuild dropped it): plan the hop again
                self.hop = None;
                self.path_timer = 0.0;
                return;
            };
            self.walk = Some(walk);
            self.edge_clock = 0.0;
            self.hop = None;
            self.unreachable = 0.0;
            self.blocked_time = 0.0;
            self.clear_path();
            return;
        }
        if deferred && self.path.is_empty() {
            // nothing to walk yet and the route is only late, not missing: this is not unreachable
            return;
        }
        if self.path.is_empty() && (no_hop || cell_of(self.pos) != target_cell) {
            self.unreachable += dt;
            if self.unreachable >= self.give_up_after {
                self.give_up_goal(fac, prof);
                return;
            }
            if self.state == State::Wander {
                self.state = State::Idle;
                self.wander_target = None;
                self.idle_timer = 0.25 + self.next_float() * 0.5;
                self.path_goal = None;
            }
            return;
        }
        self.unreachable = 0.0;
        while let Some(first) = self.path.first() {
            if dist(self.pos, first.center()) <= WAYPOINT_DISTANCE {
                self.path.remove(0);
            } else {
                break;
            }
        }
        let on_path = !self.path.is_empty();
        let mut waypoint = if cross { target_cell.center() } else { goal };
        if on_path {
            let mut furthest = 0;
            waypoint = self.path[0].center();
            for i in 0..self.path.len().min(4) {
                let c = self.path[i].center();
                if !body_fits_gated(fac, &self.gate_floor, c.0, c.1) || !self.clear_sweep(fac, self.pos, c) {
                    break;
                }
                furthest = i;
                waypoint = c;
            }
            for _ in 0..furthest {
                if !self.path.is_empty() {
                    self.path.remove(0);
                }
            }
        }
        let travel = speed * dt;
        let d = dist(self.pos, waypoint);
        let t = travel.min(d);
        let next = if d <= 0.001 {
            self.pos
        } else {
            (self.pos.0 + (waypoint.0 - self.pos.0) / d * t, self.pos.1 + (waypoint.1 - self.pos.1) / d * t)
        };
        let blocked_cell = cell_of(waypoint);
        if on_path
            && blocked_cell != cell_of(self.pos)
            && blocked_cell != target_cell
            && !sweep_clear(fac, &self.gate_floor, self.pos, next)
        {
            // The step into the next cell is blocked: do not slide along the wall. Keep that cell out of the route for
            // a moment and plan again from where the body stands. (Off-centre in its own cell the body walks to the
            // centre, as before; the goal's own cell is never kept out, the body closes on it as far as it can.)
            // A body that came round a corner can stand off-centre, its edge over the corner of the wall it has to turn
            // past: then the step is blocked though the cell is open. It walks to the middle of its own cell first (the
            // way a body turns a corner), and only a block it cannot get round by that keeps a cell out of the route.
            let own = cell_of(self.pos).center();
            if dist(self.pos, own) > CENTRE_EPS && sweep_clear(fac, &self.gate_floor, self.pos, own) {
                self.blocked_time = 0.0;
                self.step_toward(own, travel, fac, dt);
                return;
            }
            self.blocked_time += dt;
            if self.blocked_time >= self.give_up_after {
                self.give_up_goal(fac, prof);
                return;
            }
            self.avoid.push((blocked_cell, BLOCKED_AVOID_SECONDS));
            self.clear_path();
            self.path_timer = 0.0;
            self.moving_speed = 0.0;
            return;
        }
        self.blocked_time = 0.0;
        self.step_toward(waypoint, travel, fac, dt);
    }

    // -- corner peek (hunter_brain.gd 1245-1363) ---------------------------------------------------------------------

    /// Where her sight is taken from: the leaned-out head while she looks round a corner, her body otherwise.
    pub fn sight_eye(&self) -> (f32, f32) {
        if self.peeking_out() {
            self.peek_head
        } else {
            self.pos
        }
    }

    /// True while the lean pose shows (the head is past the edge).
    pub fn peeking_out(&self) -> bool {
        self.state == State::Peek && matches!(self.peek_phase, PeekPhase::Look | PeekPhase::Seen)
    }

    /// Searching or investigating and about to turn a real corner: maybe stop behind it and look first. Never
    /// mid-chase, never for a hunter whose own rules hold her in place, and at most `max_per_hunter` times.
    fn consider_peek(&mut self, fac: &Facility, g: &Gimmick, holds: bool) {
        if self.state == State::Peek
            || self.peek_cooldown > 0.0
            || self.stun > 0.0
            || self.direct_visible
            || holds
            || !matches!(self.state, State::Investigate | State::Search)
            || self.path.is_empty()
            || self.peeks_started >= peek::rules().max_per_hunter
        {
            return;
        }
        let mut odds = g.peek_chance(peek::rules().chance(&self.def.id, self.diff));
        if let Some(o) = self.peek_chance_override {
            odds = o;
        }
        if odds <= 0.0 {
            return;
        }
        let mut route = vec![cell_of(self.pos)];
        for c in &self.path {
            if route.last() != Some(c) {
                route.push(*c);
            }
        }
        let Some(corner) = peek::find_corner(&route, fac) else { return };
        if self.peek_corner_cell == Some(corner.c) {
            return;
        }
        let spot = corner.body();
        if dist(self.pos, spot) > 1.0 || !fac.hunter_fits(spot.0, spot.1) {
            return; // not at the corner yet, or no room to stand behind it
        }
        self.peek_corner_cell = Some(corner.c);
        if self.next_float() >= odds {
            return;
        }
        self.start_peek(&corner);
    }

    /// Enters the peek at `corner` (`peek::find_corner`); the tests drive this directly too.
    pub fn start_peek(&mut self, corner: &Corner) {
        self.peek_resume = if matches!(self.state, State::Investigate | State::Search) { self.state } else { State::Search };
        self.state = State::Peek;
        self.peek_phase = PeekPhase::Settle;
        self.peek_timer = peek::rules().settle;
        self.peek_body = corner.body();
        self.peek_head = corner.head();
        self.peek_dir = (corner.d2.0 as f32, corner.d2.1 as f32);
        self.peek_lean = (corner.d1.0 as f32, corner.d1.1 as f32);
        self.peek_side = corner.side();
        self.peek_corner_cell = Some(corner.c);
        self.peeks_started += 1;
        self.direct_visible = false;
        self.moving_speed = 0.0;
    }

    fn update_peek(&mut self, dt: f32, fac: &Facility, prof: &Profile, speed: f32) {
        let off = (self.peek_body.0 - self.pos.0, self.peek_body.1 - self.pos.1);
        let len = (off.0 * off.0 + off.1 * off.1).sqrt();
        if len > 0.01 {
            let travel = len.min(speed * dt);
            let moved = self.move_with_collision(off.0 / len * travel, off.1 / len * travel, fac);
            self.stride += moved;
            if dt > 0.0 {
                self.moving_speed = moved / dt;
            }
        }
        self.facing = self.peek_dir;
        self.peek_timer -= dt;
        let at_spot = dist(self.pos, self.peek_body) <= 0.05;
        if self.peek_phase == PeekPhase::Settle && !at_spot && self.peek_timer > -1.5 {
            return; // still sliding into the spot (gives up waiting after 1.5 s if something blocks it)
        }
        if self.peek_timer > 0.0 {
            return;
        }
        match self.peek_phase {
            PeekPhase::Settle => {
                if dist(self.pos, self.peek_body) > 0.5 {
                    // never reached the spot: the head would be somewhere she is not, so skip this corner
                    self.end_peek(self.peek_resume, fac, prof);
                    return;
                }
                self.peek_phase = PeekPhase::Look;
                self.peek_timer = peek::rules().tuning(&self.def.id).look;
                self.sense_timer = 0.0; // look the moment the head clears the edge
            }
            PeekPhase::Look => self.end_peek(self.peek_resume, fac, prof), // nothing there: pull back and carry on
            PeekPhase::Seen => self.end_peek(State::Chase, fac, prof),     // burst
            PeekPhase::Ambush => self.end_peek(State::Investigate, fac, prof), // they never came round: go to where she saw them
        }
    }

    /// What the senses found while she peeks (`_peek_perceive`): remember where, then burst or duck back.
    fn peek_perceive(&mut self, sees: bool, hears: bool, target: (f32, f32)) {
        if sees || hears {
            self.last_known = Some(target);
        }
        if !sees {
            self.direct_visible = false;
            return;
        }
        match self.peek_phase {
            PeekPhase::Settle | PeekPhase::Look => {
                let scale = peek::rules().scale(self.diff);
                if peek::rules().tuning(&self.def.id).style == Style::Ambush {
                    self.peek_phase = PeekPhase::Ambush; // duck back behind the corner and wait for them to come round
                    self.peek_timer = scale.ambush;
                } else {
                    self.peek_phase = PeekPhase::Seen;
                    self.peek_timer = scale.react;
                }
            }
            PeekPhase::Ambush => {
                self.state = State::Chase; // seen from cover: they came to her
                self.finish_peek();
                self.direct_visible = true;
            }
            PeekPhase::Seen => {}
        }
    }

    /// The common tail of a peek: cooldown, a clean route and a fresh look next frame.
    fn finish_peek(&mut self) {
        self.peek_phase = PeekPhase::Settle;
        self.peek_cooldown = peek::rules().cooldown;
        self.lost_sight = 0.0;
        self.sense_timer = 0.0;
        self.clear_path();
    }

    fn end_peek(&mut self, next: State, fac: &Facility, prof: &Profile) {
        self.state = next;
        self.finish_peek();
        if next == State::Search {
            if self.search_target.is_none() {
                self.search_target = Some(self.pick_search_target(fac));
            }
            if self.search_timer <= 0.0 {
                self.search_timer = prof.search_time;
            }
        }
    }

    fn pick_wander_target(&mut self, fac: &Facility) -> (f32, f32) {
        let mut cands = vec![];
        for room in fac.rooms.iter().filter(|r| !r.lobby()) {
            let (cx, cy) = room.center.center();
            if let Some(p) = fac.nearest_open(cx, cy, HUNTER_RADIUS) {
                if dist(self.pos, p) > 5.0 {
                    cands.push(p);
                }
            }
        }
        if cands.is_empty() {
            return fac.exit.center();
        }
        cands[self.next_index(cands.len())]
    }

    /// _pick_search_target: the next point to search. Hiding spots near the last sighting and the offsets around it
    /// come from data/hunters/search.json (per difficulty); every point is one a route actually leads to.
    pub fn pick_search_target(&mut self, fac: &Facility) -> (f32, f32) {
        let rules = search_rules();
        let scale = rules.scale(self.diff);
        match self.pick_search_point(fac, rules, scale) {
            Some(p) => {
                self.searched.push(p);
                let excess = self.searched.len().saturating_sub(rules.memory);
                self.searched.drain(..excess);
                p
            }
            None => self.pos,
        }
    }

    fn pick_search_point(&mut self, fac: &Facility, rules: &SearchRules, scale: SearchScale) -> Option<(f32, f32)> {
        let want_spot = if scale.last_seen_spot_first {
            true
        } else if scale.hiding_spot_chance > 0.0 {
            self.next_float() < scale.hiding_spot_chance
        } else {
            false
        };
        if want_spot {
            if let Some(spot) = self.hiding_spot_near_last_seen(fac, rules) {
                return Some(spot);
            }
        }
        let lk = self.last_known.unwrap_or(self.pos);
        let mut offs = rules.offsets.clone();
        self.rng.shuffle(&mut offs);
        let mut reachable = Vec::new();
        for (ox, oy) in offs {
            if let Some(c) = fac.nearest_open(lk.0 + ox, lk.1 + oy, HUNTER_RADIUS) {
                if dist(c, self.pos) > 0.5 && self.can_reach(fac, c) {
                    if !scale.unchecked_first {
                        return Some(c);
                    }
                    reachable.push(c);
                }
            }
        }
        least_recently_checked(&reachable, &self.searched)
    }

    /// Whether a route leads from here to `p`.
    fn can_reach(&self, fac: &Facility, p: (f32, f32)) -> bool {
        // a search the budget deferred counts as reachable until it is answered (a walk that cannot get there gives up)
        let priority = self.plan_priority(p);
        self.grid_route(fac, cell_of(p), priority, false).is_none_or(|route| !route.is_empty())
    }

    /// The hiding spot whose centre is nearest the last sighting, when it is within `reach` and a route leads to it.
    fn hiding_spot_near_last_seen(&self, fac: &Facility, rules: &SearchRules) -> Option<(f32, f32)> {
        let lk = self.last_known.unwrap_or(self.pos);
        let mut best: Option<((f32, f32), f32)> = None;
        for h in &fac.hiding {
            let c = h.cell.center();
            let gap = dist(c, lk);
            if gap > rules.reach || dist(c, self.pos) <= 0.5 {
                continue;
            }
            if best.map_or(true, |(_, g)| gap < g) {
                best = Some((c, gap));
            }
        }
        let (c, _) = best?;
        if !self.can_reach(fac, c) {
            return None;
        }
        Some(c)
    }
}

/// The candidate farthest from every point already searched this chase (an unchecked spot). With nothing searched
/// yet, or no candidates, the first candidate wins; ties keep the earlier one.
pub fn least_recently_checked(cands: &[(f32, f32)], searched: &[(f32, f32)]) -> Option<(f32, f32)> {
    let mut best: Option<((f32, f32), f32)> = None;
    for &c in cands {
        let score = searched.iter().map(|&s| dist(c, s)).fold(f32::INFINITY, f32::min);
        if best.map_or(true, |(_, b)| score > b) {
            best = Some((c, score));
        }
    }
    best.map(|(c, _)| c)
}

/// One difficulty's search behaviour (data/hunters/search.json).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchScale {
    /// Chance a search point is the hiding spot nearest the last sighting (Godot hide_random).
    pub hiding_spot_chance: f32,
    /// Always check that spot first when one is in reach, instead of rolling for it.
    pub last_seen_spot_first: bool,
    /// Prefer the reachable offset farthest from points already searched this chase.
    pub unchecked_first: bool,
}

/// The search rules: reach of the hiding-spot check, how many searched points are remembered, the offsets around
/// the last sighting, and the per-difficulty scale. Missing values keep the defaults.
#[derive(Clone, Debug)]
pub struct SearchRules {
    /// Largest gap between a hiding spot and the last sighting for it to be checked.
    pub reach: f32,
    /// Searched points remembered for `unchecked_first`.
    pub memory: usize,
    /// Seconds without a route (or with every step blocked) before a hunter gives its goal up.
    pub unreachable_give_up: f32,
    pub offsets: Vec<(f32, f32)>,
    default: SearchScale,
    difficulty: HashMap<String, SearchScale>,
}

impl Default for SearchRules {
    fn default() -> Self {
        SearchRules {
            reach: 8.0,
            memory: 12,
            unreachable_give_up: UNREACHABLE_GIVE_UP,
            offsets: vec![
                (3.0, 0.0),
                (-3.0, 0.0),
                (0.0, 3.0),
                (0.0, -3.0),
                (2.0, 2.0),
                (-2.0, 2.0),
                (2.0, -2.0),
                (-2.0, -2.0),
            ],
            default: SearchScale { hiding_spot_chance: 0.0, last_seen_spot_first: false, unchecked_first: false },
            difficulty: HashMap::new(),
        }
    }
}

fn search_scale_of(v: &serde_json::Value, base: SearchScale) -> SearchScale {
    let mut s = base;
    if let Some(x) = v["hiding_spot_chance"].as_f64() {
        s.hiding_spot_chance = (x as f32).clamp(0.0, 1.0);
    }
    if let Some(b) = v["last_seen_spot_first"].as_bool() {
        s.last_seen_spot_first = b;
    }
    if let Some(b) = v["unchecked_first"].as_bool() {
        s.unchecked_first = b;
    }
    s
}

impl SearchRules {
    pub fn parse(v: &serde_json::Value) -> SearchRules {
        let mut r = SearchRules::default();
        if let Some(x) = v["reach"].as_f64() {
            r.reach = (x as f32).clamp(0.0, 64.0);
        }
        if let Some(n) = v["memory"].as_i64() {
            r.memory = n.clamp(0, 256) as usize;
        }
        if let Some(x) = v["unreachable_give_up"].as_f64() {
            r.unreachable_give_up = (x as f32).clamp(0.05, 60.0);
        }
        if let Some(a) = v["offsets"].as_array() {
            let offs: Vec<(f32, f32)> = a
                .iter()
                .filter_map(|p| Some((p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32)))
                .collect();
            if !offs.is_empty() {
                r.offsets = offs;
            }
        }
        r.default = search_scale_of(&v["default"], r.default);
        if let Some(m) = v["difficulty"].as_object() {
            for (id, s) in m {
                r.difficulty.insert(id.clone(), search_scale_of(s, r.default));
            }
        }
        r
    }

    /// The difficulty's scale; a difficulty with no row uses the default.
    pub fn scale(&self, diff: Difficulty) -> SearchScale {
        self.difficulty.get(diff.id()).copied().unwrap_or(self.default)
    }
}

/// The embedded search rules (data/hunters/search.json); the defaults when the file is missing or broken.
pub fn search_rules() -> &'static SearchRules {
    static RULES: OnceLock<SearchRules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("hunters/search.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
            .map(|v| SearchRules::parse(&v))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::world::facility::Options;

    fn aki() -> HunterDef {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        crate::content::hunters::parse(&v, None, "").unwrap()
    }

    /// data/hunters/aki.json: speed 0.82 (PATROL), chase 3.55 (CHASE), prog 0.2, the same numbers as
    /// game/hunters/ai/hunter_brain.gd _current_speed at standard difficulty, depth 1, no relays.
    #[test]
    fn aki_chase_and_patrol_speed_match_the_godot_roster() {
        let prof = Difficulty::Standard.profile();
        let mut b = Brain::new(aki(), (0.0, 0.0), "PIN", Difficulty::Standard);
        b.state = State::Chase;
        let scale = crate::game::progression::Progression::load().chase_speed_scale;
        assert!((b.current_speed(1, 0, &prof, 1.0) - 3.55 * scale).abs() < 1e-5);
        b.state = State::Wander;
        assert!((b.current_speed(1, 0, &prof, 1.0) - 0.82).abs() < 1e-5);
    }

    /// Walking a route in chase (no clear line to the runner) moves her at exactly her chase speed, and a
    /// clear straight line with her in sight dashes at HUNTER_VISIBLE_CHASE_SPEED_SCALE (1.20) times it.
    #[test]
    fn aki_moves_at_her_chase_speed_and_dashes_on_a_clear_sight_line() {
        let prof = Difficulty::Standard.profile();
        let cat = crate::world::catalog::Catalog::load();
        let fac = Facility::generate("PROBE-1", &Options::default(), &cat);
        let cells = fac.floor_cells();
        let relays_taken: Vec<bool> = vec![];
        let batteries_taken: Vec<bool> = vec![];
        let hiding_disabled: Vec<bool> = vec![];
        let ctx = Ctx {
            fac: &fac,
            relays_taken: &relays_taken,
            batteries_taken: &batteries_taken,
            hiding_disabled: &hiding_disabled,
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: false,
        };
        let dt = 1.0 / 60.0;

        // a long route, the runner parked at its far end out of sight
        let mut route: Option<(Cell, Cell)> = None;
        'search: for a in cells.iter().step_by(37).take(60) {
            for b in cells.iter().step_by(53).take(60) {
                if fac.find_path(*a, *b).len() >= 22 {
                    route = Some((*a, *b));
                    break 'search;
                }
            }
        }
        let (from, to) = route.expect("a route long enough to measure");
        let mut walker = Brain::new(aki(), from.center(), "PIN", Difficulty::Standard);
        walker.active = true;
        walker.chase(to.center());
        let far = Target { pos: to.center(), facing: (1.0, 0.0), ..Default::default() };
        let mut walked = 0.0f32;
        let mut prev = walker.pos;
        for _ in 0..60 {
            walker.update(dt, &ctx, &far, 1, 0, &prof);
            walked += dist(prev, walker.pos);
            prev = walker.pos;
        }
        // never faster than her chase speed; the turns of the route cost a little (each waypoint stops the
        // frame's travel short, as hunter_brain.gd _pursue does)
        let scale = crate::game::progression::Progression::load().chase_speed_scale;
        assert!(walked <= 3.56 * scale && walked >= 3.4 * scale, "walked {walked} cells in one second");

        // a straight run of clear floor with the runner 3.8 cells ahead, in plain sight
        let mut line: Option<Cell> = None;
        for c in cells.iter() {
            let (sx, sy) = (c.x as f32 + 0.5, c.y as f32 + 0.5);
            let clear = (0..=38).all(|i| fac.body_fits(sx + i as f32 * 0.1, sy, HUNTER_RADIUS));
            if clear && fac.has_line_of_sight((sx, sy), (sx + 3.8, sy)) {
                line = Some(*c);
                break;
            }
        }
        let c = line.expect("a clear straight line");
        let start = (c.x as f32 + 0.5, c.y as f32 + 0.5);
        let runner = (start.0 + 3.8, start.1);
        let mut dasher = Brain::new(aki(), start, "PIN", Difficulty::Standard);
        dasher.active = true;
        dasher.chase(runner);
        let seen = Target { pos: runner, facing: (-1.0, 0.0), ..Default::default() };
        let mut dashed = 0.0f32;
        let mut prev = dasher.pos;
        for _ in 0..18 {
            dasher.update(dt, &ctx, &seen, 1, 0, &prof);
            dashed += dist(prev, dasher.pos);
            prev = dasher.pos;
        }
        assert!(dasher.direct_visible, "she should see the runner on a clear line");
        assert!((dashed / (18.0 * dt) - 4.26 * scale).abs() < 0.02, "dashed {dashed} cells in 0.3 s");
    }

    /// With the beam off, a runner in a lit cell is seen from further off than one in pitch dark: the unlit
    /// limit follows her lamp exposure (LightField.unlit_scale), and the default is the old unlit scale.
    #[test]
    fn a_runner_standing_in_light_is_seen_further_without_the_beam() {
        let prof = Difficulty::Standard.profile();
        let mut b = Brain::new(aki(), (0.0, 0.0), "PIN", Difficulty::Standard);
        let (s, k) = (b.def.sight, b.gimmick.sight_scale());
        let want = |scale: f32| s * scale * prof.hunter_sight * k;
        let rule_off = b.sight_limit(false, &prof, &b.gimmick);
        assert!((rule_off - want(UNLIT_SIGHT_SCALE)).abs() < 1e-4);
        b.target_exposure = Some(1.0);
        let lit = b.sight_limit(false, &prof, &b.gimmick);
        assert!((lit - want(LIGHT_LIT_SIGHT_SCALE)).abs() < 1e-4);
        b.target_exposure = Some(0.0);
        let dark = b.sight_limit(false, &prof, &b.gimmick);
        assert!((dark - want(LIGHT_DARK_SIGHT_SCALE)).abs() < 1e-4);
        assert!(lit > dark);
    }

    /// A hunter standing in light (self_exposure at the visible level) sees a dark runner at her full range.
    #[test]
    fn a_hunter_in_light_sees_a_dark_runner_at_full_range() {
        let prof = Difficulty::Standard.profile();
        let mut b = Brain::new(aki(), (0.0, 0.0), "PIN", Difficulty::Standard);
        let (s, k) = (b.def.sight, b.gimmick.sight_scale());
        let want = |scale: f32| s * scale * prof.hunter_sight * k;
        b.target_exposure = Some(0.0);
        b.self_exposure = Some(LIGHT_VISIBLE_EXPOSURE - 0.01);
        let dim = b.sight_limit(false, &prof, &b.gimmick);
        assert!((dim - want(LIGHT_DARK_SIGHT_SCALE)).abs() < 1e-4);
        b.self_exposure = Some(LIGHT_VISIBLE_EXPOSURE);
        let lit = b.sight_limit(false, &prof, &b.gimmick);
        assert!((lit - want(1.0)).abs() < 1e-4);
    }

    /// The beam keeps its range whatever the runner's lamp exposure or her own light is.
    #[test]
    fn the_beam_still_gives_its_flashlight_range() {
        let prof = Difficulty::Standard.profile();
        let mut b = Brain::new(aki(), (0.0, 0.0), "PIN", Difficulty::Standard);
        let (s, k) = (b.def.sight, b.gimmick.sight_scale());
        let beam = FLASHLIGHT_SIGHT_MIN.max(s * FLASHLIGHT_SIGHT_SCALE) * prof.hunter_sight * k;
        b.target_exposure = Some(0.0);
        b.self_exposure = Some(1.0);
        assert!((b.sight_limit(true, &prof, &b.gimmick) - beam).abs() < 1e-4);
        assert!(beam > b.sight_limit(false, &prof, &b.gimmick));
    }

    // -- corner peek ------------------------------------------------------------------------------------------------

    use crate::hunters::peek::tests as peeks;

    fn ctx(fac: &Facility) -> Ctx<'_> {
        Ctx {
            fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: false,
        }
    }

    /// A corner with room for her to stand behind and a four-cell hall, and a runner standing down that hall whom
    /// the leaned head sees and the body behind the edge does not.
    fn setup() -> (Facility, Corner, (f32, f32)) {
        let fac = peeks::facility();
        for k in peeks::corners(&fac, 4) {
            let (b, h) = (k.body(), k.head());
            let (cx, cy) = k.c.center();
            let runner = (cx + k.d2.0 as f32 * 3.0, cy + k.d2.1 as f32 * 3.0);
            if fac.body_fits(b.0, b.1, HUNTER_RADIUS) && fac.has_line_of_sight(h, runner) && !fac.has_line_of_sight(b, runner) {
                return (fac, k, runner);
            }
        }
        panic!("the test maze has a corner with a hall");
    }

    fn peeker(id: &str, k: &Corner) -> Brain {
        let mut b = Brain::new(aki(), k.body(), "PEEK", Difficulty::Standard);
        b.def.id = id.into();
        b.active = true;
        b.state = State::Search;
        b.search_timer = 60.0;
        b.search_target = Some(k.body());
        b
    }

    fn run(b: &mut Brain, fac: &Facility, t: &Target, seconds: f32) {
        let prof = Difficulty::Standard.profile();
        let ctx = ctx(fac);
        for _ in 0..(seconds * 60.0).round() as i32 {
            b.update(1.0 / 60.0, &ctx, t, 1, 0, &prof);
        }
    }

    fn runner_at(p: (f32, f32), hidden: bool) -> Target {
        Target { pos: p, facing: (1.0, 0.0), hidden, flashlight_on: true, ..Default::default() }
    }

    /// Settle (slide to the stand), look with the head past the edge, then pull back to what she was doing with the
    /// route cleared and the cooldown running.
    #[test]
    fn a_peek_settles_looks_then_carries_on() {
        let (fac, k, _) = setup();
        let nobody = runner_at((-50.0, -50.0), true);
        let mut b = peeker("kathy", &k);
        b.start_peek(&k);
        assert_eq!((b.state, b.peek_phase, b.peeks_started), (State::Peek, PeekPhase::Settle, 1));
        assert_eq!(b.sight_eye(), b.pos, "the body sees until the head is out");
        run(&mut b, &fac, &nobody, 0.1);
        assert_eq!((b.state, b.peek_phase), (State::Peek, PeekPhase::Settle));
        run(&mut b, &fac, &nobody, 0.3);
        assert_eq!(b.peek_phase, PeekPhase::Look);
        assert!(b.peeking_out());
        assert_eq!(b.sight_eye(), k.head());
        assert_eq!(b.facing, (k.d2.0 as f32, k.d2.1 as f32), "she faces down the hall");
        run(&mut b, &fac, &nobody, 1.1);
        assert_eq!(b.state, State::Search, "nothing there: back to the search");
        assert!(!b.peeking_out());
        assert!(b.peek_cooldown > 4.5 && b.peek_cooldown <= 6.0, "cooldown {}", b.peek_cooldown);
        assert_eq!(b.sight_eye(), b.pos);
    }

    /// A runner down the hall is seen by the leaned head and not by the body: a burst hunter charges once the
    /// reaction time has passed, remembering where she saw them.
    #[test]
    fn a_leaned_head_sees_down_the_hall_and_a_burst_hunter_charges() {
        let (fac, k, runner) = setup();
        let t = runner_at(runner, false);
        let mut b = peeker("kathy", &k);
        b.start_peek(&k);
        run(&mut b, &fac, &t, 0.2);
        assert_eq!(b.peek_phase, PeekPhase::Settle, "the body behind the edge cannot see the hall");
        run(&mut b, &fac, &t, 0.2);
        assert_eq!((b.state, b.peek_phase), (State::Peek, PeekPhase::Seen));
        assert_eq!(b.last_known, Some(runner));
        run(&mut b, &fac, &t, 0.3);
        assert_eq!(b.state, State::Peek, "still reacting");
        run(&mut b, &fac, &t, 0.4);
        assert_eq!(b.state, State::Chase);
        assert!(b.peek_cooldown > 5.0);
    }

    /// An ambusher ducks back when she sees them, waits behind the corner, charges the moment they come round, and
    /// goes to where she saw them when they never do.
    #[test]
    fn an_ambusher_waits_in_cover_and_springs_when_they_come_round() {
        let (fac, k, runner) = setup();
        let mut b = peeker("quenelle", &k);
        b.start_peek(&k);
        run(&mut b, &fac, &runner_at(runner, false), 0.6);
        assert_eq!((b.state, b.peek_phase), (State::Peek, PeekPhase::Ambush));
        assert!(!b.peeking_out(), "she ducks back behind the corner");
        run(&mut b, &fac, &runner_at(runner, true), 1.0);
        assert_eq!(b.state, State::Peek, "nobody comes round yet");
        // they step into the cell she watches: seen from cover
        let round = k.c.center();
        let (prof, ctx) = (Difficulty::Standard.profile(), ctx(&fac));
        let t = runner_at(round, false);
        for _ in 0..30 {
            if b.state != State::Peek {
                break;
            }
            b.update(1.0 / 60.0, &ctx, &t, 1, 0, &prof);
        }
        assert_eq!(b.state, State::Chase);
        assert!(b.direct_visible);

        let mut b = peeker("quenelle", &k);
        b.start_peek(&k);
        run(&mut b, &fac, &runner_at(runner, false), 0.6);
        assert_eq!(b.peek_phase, PeekPhase::Ambush);
        run(&mut b, &fac, &runner_at(runner, true), 2.6);
        assert_eq!(b.state, State::Investigate, "they never came: go to where she saw them");
        assert_eq!(b.last_known, Some(runner));
    }

    /// A runner hiding down the hall is not seen from the leaned head (can_sense), and a runner out of its line is not.
    #[test]
    fn a_hidden_or_out_of_line_runner_is_not_seen() {
        let (fac, k, runner) = setup();
        let mut b = peeker("kathy", &k);
        b.start_peek(&k);
        run(&mut b, &fac, &runner_at(runner, true), 2.0);
        assert_eq!(b.state, State::Search);
        // a runner close by but round another bend, out of line of both the body and the leaned head
        let (b0, h0) = (k.body(), k.head());
        let hidden_spot = fac
            .floor_cells()
            .iter()
            .map(|c| c.center())
            .find(|p| dist(*p, b0) < 6.0 && !fac.has_line_of_sight(b0, *p) && !fac.has_line_of_sight(h0, *p))
            .expect("a spot out of line");
        let mut b = peeker("kathy", &k);
        b.start_peek(&k);
        run(&mut b, &fac, &runner_at(hidden_spot, false), 1.8);
        assert_eq!(b.state, State::Search, "never saw them, carried on");
        assert_eq!(b.last_known, None);
    }

    /// The brain rolls the peek on its own at the corner on her route, once per corner, never in a chase, never
    /// while her gimmick holds her, never for the hunters that cannot peek, and never past the cap or the cooldown.
    #[test]
    fn the_brain_peeks_at_a_route_corner_with_a_cooldown_and_a_cap() {
        let (fac, k, runner) = setup();
        let nobody = runner_at((-50.0, -50.0), true);
        let hall = |n: i32| Cell::new(k.c.x + k.d2.0 * n, k.c.y + k.d2.1 * n);
        let route = || vec![k.c, hall(1), hall(2)];
        let mut b = peeker("kathy", &k);
        b.peek_chance_override = Some(1.0);
        b.path = route();
        let g = std::mem::take(&mut b.gimmick);
        b.consider_peek(&fac, &g, true);
        assert_eq!(b.state, State::Search, "a gimmick that holds her place never peeks");
        b.state = State::Chase;
        b.consider_peek(&fac, &g, false);
        assert_eq!(b.state, State::Chase, "never mid-chase");
        b.state = State::Search;
        b.direct_visible = true;
        b.consider_peek(&fac, &g, false);
        assert_eq!(b.state, State::Search, "never while the runner is in plain sight");
        b.direct_visible = false;
        b.pos = (k.body().0 - k.d1.0 as f32 * 2.0, k.body().1 - k.d1.1 as f32 * 2.0);
        b.consider_peek(&fac, &g, false);
        assert_eq!(b.state, State::Search, "not at the corner yet");
        b.pos = k.body();
        b.gimmick = g;
        run(&mut b, &fac, &nobody, 0.02);
        assert_eq!(b.state, State::Peek, "at the corner with certain odds");
        assert_eq!(b.peeks_started, 1);
        run(&mut b, &fac, &nobody, 2.0);
        assert_eq!(b.state, State::Search);
        // the same corner is not tried twice in a row, and the cooldown holds the next one back
        b.path = route();
        b.pos = k.body();
        run(&mut b, &fac, &nobody, 0.1);
        assert_ne!(b.state, State::Peek);
        b.peek_corner_cell = None;
        b.path = route();
        b.pos = k.body();
        run(&mut b, &fac, &nobody, 0.1);
        assert_ne!(b.state, State::Peek, "the cooldown has not run out");
        b.peek_cooldown = 0.0;
        b.path = route();
        b.pos = k.body();
        b.peeks_started = peek::rules().max_per_hunter;
        run(&mut b, &fac, &nobody, 0.1);
        assert_ne!(b.state, State::Peek, "past the cap");
        b.peeks_started = 0;
        b.path = route();
        b.pos = k.body();
        run(&mut b, &fac, &nobody, 0.1);
        assert_eq!(b.state, State::Peek);
        // hunters that cannot peek never roll, whatever the corner
        let mut aki_b = peeker("aki", &k);
        for _ in 0..50 {
            aki_b.peek_corner_cell = None;
            aki_b.path = route();
            aki_b.pos = k.body();
            run(&mut aki_b, &fac, &nobody, 0.02);
        }
        assert_eq!(aki_b.peeks_started, 0);
        let _ = runner;
    }

    /// Hunters sent across a real maze with certain odds do peek on the way, none stays in a peek longer than a
    /// peek can last, and none peeks more than the cap.
    #[test]
    fn hunters_crossing_a_maze_peek_on_the_way_and_never_stick() {
        let fac = peeks::facility();
        let (prof, ctx) = (Difficulty::Standard.profile(), ctx(&fac));
        let nobody = runner_at((-50.0, -50.0), true);
        let cells = fac.floor_cells();
        let (mut peeks_total, mut longest) = (0u32, 0.0f32);
        for n in 0..6usize {
            let from = cells[(n * 211) % cells.len()];
            let mut b = Brain::new(aki(), from.center(), &format!("CROSS-{n}"), Difficulty::Standard);
            b.def.id = ["kathy", "quenelle", "nyxie"][n % 3].into();
            b.active = true;
            b.peek_chance_override = Some(1.0);
            let mut in_peek = 0.0f32;
            for step in 0..(150 * 30) {
                if step % (20 * 30) == 0 {
                    let goal = cells[(n * 97 + step / 30 * 53 + 7) % cells.len()];
                    b.direct_to(goal.center());
                }
                b.update(1.0 / 30.0, &ctx, &nobody, 1, 0, &prof);
                in_peek = if b.state == State::Peek { in_peek + 1.0 / 30.0 } else { 0.0 };
                longest = longest.max(in_peek);
            }
            assert!(b.peeks_started <= peek::rules().max_per_hunter);
            peeks_total += b.peeks_started;
        }
        assert!(peeks_total > 0, "no hunter peeked in 150 s of walking");
        assert!(longest < 4.5, "a peek lasted {longest} s");
    }

    /// However a peek goes, it ends: a hunter whose stand is walled in gives up after the settle timeout, and a
    /// peek never lasts longer than settle + look.
    #[test]
    fn a_peek_always_ends() {
        let (fac, k, _) = setup();
        let nobody = runner_at((-50.0, -50.0), true);
        let mut b = peeker("porcelle", &k);
        b.start_peek(&k);
        b.pos = (k.body().0 - k.d1.0 as f32 * 3.0, k.body().1 - k.d1.1 as f32 * 3.0);
        b.peek_body = (b.pos.0 - k.d1.0 as f32 * 4.0, b.pos.1 - k.d1.1 as f32 * 4.0); // unreachable stand: a wall in the way
        run(&mut b, &fac, &nobody, 2.5);
        assert_ne!(b.state, State::Peek, "gave up on the stand");
        assert_eq!(b.state, State::Search);
        let mut b = peeker("porcelle", &k);
        b.start_peek(&k);
        run(&mut b, &fac, &nobody, 0.3 + 2.0 + 0.2);
        assert_ne!(b.state, State::Peek);
    }

    /// Taser slow stacking (hunter_brain.gd apply_slow): a weaker hit does not weaken an active slow, the stronger
    /// factor is kept, and the longer time wins. Same sequence as tests/validate_hunter_immunity.gd:120-124.
    #[test]
    fn a_weaker_slow_does_not_weaken_an_active_slow() {
        let mut b = Brain::new(aki(), (0.0, 0.0), "SLOW", Difficulty::Standard);
        b.apply_slow(2.0, 0.5);
        b.apply_slow(1.0, 0.8);
        assert_eq!(b.slow_factor, 0.5, "weaker hit must not raise the factor");
        assert_eq!(b.slow_timer, 2.0, "shorter hit must not cut the time");
        b.apply_slow(5.0, 0.3);
        assert_eq!(b.slow_factor, 0.3, "stronger hit takes the lower factor");
        assert_eq!(b.slow_timer, 5.0, "longer hit extends the time");
    }

    /// A hit with a longer time extends an active slow, but never shortens it.
    #[test]
    fn a_longer_slow_extends_the_active_one() {
        let mut b = Brain::new(aki(), (0.0, 0.0), "SLOW", Difficulty::Standard);
        b.apply_slow(3.0, 0.5);
        b.tick_slow(2.0);
        b.apply_slow(4.0, 0.5);
        assert!((b.slow_timer - 4.0).abs() < 1e-6, "longer hit extends to 4 s, got {}", b.slow_timer);
        b.tick_slow(1.0);
        b.apply_slow(0.5, 0.5);
        assert!((b.slow_timer - 3.0).abs() < 1e-6, "shorter hit must not cut the remaining 3 s, got {}", b.slow_timer);
    }

    /// Expiry resets the factor to 1.0 (the clock runs out inside tick_slow), and a fresh slow after expiry takes
    /// its own factor rather than the old stronger one.
    #[test]
    fn expiry_resets_the_slow_factor() {
        let mut b = Brain::new(aki(), (0.0, 0.0), "SLOW", Difficulty::Standard);
        b.apply_slow(1.0, 0.3);
        b.tick_slow(0.6);
        assert_eq!(b.slow_factor, 0.3, "factor holds while the slow runs");
        b.tick_slow(0.6);
        assert_eq!(b.slow_timer, 0.0);
        assert_eq!(b.slow_factor, 1.0, "expiry resets the factor");
        b.apply_slow(2.0, 0.7);
        assert_eq!(b.slow_factor, 0.7, "a new slow after expiry is not stacked on the old factor");
        assert_eq!(b.slow_timer, 2.0);
    }

    /// A chasing hunter with a wall between her and the runner walks round the wall to the runner's side, and her walk
    /// does not slide along the wall: the distance she covers while moving off the way her step was aimed stays under
    /// 1 m. Over the pairs of a maze that a route walks round (the hunter starts and the runner stands in cells the
    /// route connects, with no line of sight), almost every hunter arrives; the few that do not are the cells a
    /// hunter cannot close on at all, and none of those slides.
    #[test]
    fn a_chasing_hunter_walks_round_a_wall_without_sliding_along_it() {
        let prof = Difficulty::Standard.profile();
        let cat = crate::world::catalog::Catalog::load();
        let fac = Facility::generate("NAV-2", &Options::default(), &cat);
        let cells = fac.floor_cells();
        let (mut pairs, mut arrived, mut worst_slide) = (0, 0, 0.0f32);
        for a in cells.iter().step_by(23) {
            for b in cells.iter().step_by(31) {
                let (ca, cb) = (a.center(), b.center());
                let straight = dist(ca, cb);
                if !(3.0..=12.0).contains(&straight) || fac.has_line_of_sight(ca, cb) {
                    continue;
                }
                if fac.find_path(*a, *b).len() < 8 {
                    continue;
                }
                let (reached, slide) = chase_round_wall(&fac, &prof, ca, cb);
                pairs += 1;
                arrived += reached as i32;
                worst_slide = worst_slide.max(slide);
            }
        }
        assert!(pairs >= 20, "the test maze has walls between cells a route walks round: {pairs} pairs");
        assert!(worst_slide <= 1.0, "a hunter slid {worst_slide:.2} m along a wall");
        assert!(arrived * 100 >= pairs * 95, "only {arrived} of {pairs} hunters reached the runner's side");
    }

    /// Walks one hunter from `from` towards a runner at `goal` for up to 40 s. Returns whether she came within 1 m of
    /// the runner, and how far she slid: distance covered in steps that were not along the way she was facing.
    fn chase_round_wall(
        fac: &Facility,
        prof: &crate::config::Profile,
        from: (f32, f32),
        goal: (f32, f32),
    ) -> (bool, f32) {
        let mut h = Brain::new(aki(), from, "NAV-WALL", Difficulty::Standard);
        h.active = true;
        h.state = State::Chase;
        h.last_known = Some(goal);
        let speed = h.current_speed(1, 0, prof, 1.0);
        let dt = 1.0 / 60.0;
        let mut slide = 0.0f32;
        for _ in 0..(60 * 40) {
            let before = h.pos;
            h.pursue(goal, speed, dt, fac, prof);
            let d = (h.pos.0 - before.0, h.pos.1 - before.1);
            let len = (d.0 * d.0 + d.1 * d.1).sqrt();
            if len > 1e-3 && (d.0 * h.facing.0 + d.1 * h.facing.1) / len < 0.5 {
                slide += len;
            }
            if dist(h.pos, goal) <= 1.0 {
                return (true, slide);
            }
        }
        (false, slide)
    }

    /// A planned route crosses only cells the hunter's body fits: the grid is built for the thinner agent radius, and a
    /// route through a cell the body cannot stand in strands the hunter at the wall (the NAV-2 floor's doorway).
    #[test]
    fn a_planned_route_only_crosses_cells_the_hunter_body_fits() {
        let cat = crate::world::catalog::Catalog::load();
        let fac = Facility::generate("NAV-2", &Options::default(), &cat);
        let stand = Standable::new(&fac);
        let cells = fac.floor_cells();
        let mut routes = 0;
        for a in cells.iter().step_by(23) {
            for b in cells.iter().step_by(31) {
                let route = plan_route_wet(&fac, a.center(), *b, &[], &[], None);
                if route.len() < 2 {
                    continue;
                }
                routes += 1;
                for c in &route {
                    assert!(stand.can_stand(*c), "{a:?} -> {b:?}: the route enters {c:?}, which the hunter's body does not fit");
                }
            }
        }
        assert!(routes >= 20, "the test floor has routes to check: {routes}");
    }
}

#[cfg(test)]
mod search_rules_tests {
    use super::*;
    use crate::config::Difficulty;
    use serde_json::json;

    #[test]
    fn the_embedded_search_rules_match_godot_hide_random_per_difficulty() {
        // game_config.gd DIFFICULTY_PROFILES hide_random: 0.0, 0.05, 0.25, 0.45. Standard stays 0.0 here, not
        // Godot's 0.05: a Standard hiding-spot check keeps hunters searching beside the runner, so none are free
        // for the relay rush (headless hunters suite). Change it in data/hunters/search.json with that test in mind.
        let rules = search_rules();
        assert_eq!(rules.scale(Difficulty::Relaxed).hiding_spot_chance, 0.0);
        assert_eq!(rules.scale(Difficulty::Standard).hiding_spot_chance, 0.0);
        assert_eq!(rules.scale(Difficulty::Hard).hiding_spot_chance, 0.25);
        assert_eq!(rules.scale(Difficulty::Nightmare).hiding_spot_chance, 0.45);
        // Standard is the one intentional difference from the profile's hide_random (see above)
        for d in [Difficulty::Relaxed, Difficulty::Hard, Difficulty::Nightmare] {
            assert_eq!(rules.scale(d).hiding_spot_chance, d.profile().hide_random, "{d:?}");
        }
    }

    #[test]
    fn standard_keeps_the_plain_search_and_higher_difficulties_check_last_seen_first() {
        let rules = search_rules();
        let standard = rules.scale(Difficulty::Standard);
        assert!(!standard.last_seen_spot_first && !standard.unchecked_first);
        for d in [Difficulty::Hard, Difficulty::Nightmare] {
            let s = rules.scale(d);
            assert!(s.last_seen_spot_first && s.unchecked_first, "{d:?}");
        }
        assert_eq!(rules.offsets.len(), 8);
        assert_eq!(rules.offsets[0], (3.0, 0.0));
    }

    #[test]
    fn changing_the_search_data_changes_the_scale_the_brain_reads() {
        let v = json!({
            "schema": "lr.search/1",
            "reach": 5.0,
            "memory": 3,
            "offsets": [[1, 0], [0, 1]],
            "default": { "hiding_spot_chance": 0.1 },
            "difficulty": { "standard": { "hiding_spot_chance": 0.9, "last_seen_spot_first": true } }
        });
        let rules = SearchRules::parse(&v);
        assert_eq!(rules.reach, 5.0);
        assert_eq!(rules.memory, 3);
        assert_eq!(rules.offsets, vec![(1.0, 0.0), (0.0, 1.0)]);
        let standard = rules.scale(Difficulty::Standard);
        assert_eq!(standard.hiding_spot_chance, 0.9);
        assert!(standard.last_seen_spot_first);
        assert!(!standard.unchecked_first, "a field left out keeps its default");
        // a difficulty with no row falls back to the default
        assert_eq!(rules.scale(Difficulty::Hard).hiding_spot_chance, 0.1);
        // the chance is clamped to 0..1
        let high = SearchRules::parse(&json!({ "difficulty": { "hard": { "hiding_spot_chance": 7.0 } } }));
        assert_eq!(high.scale(Difficulty::Hard).hiding_spot_chance, 1.0);
    }

    #[test]
    fn unchecked_first_picks_the_spot_farthest_from_what_was_searched() {
        let cands = [(0.0, 0.0), (5.0, 0.0), (9.0, 0.0)];
        assert_eq!(least_recently_checked(&cands, &[(0.0, 0.0)]), Some((9.0, 0.0)));
        assert_eq!(least_recently_checked(&cands, &[]), Some((0.0, 0.0)), "nothing searched: first candidate");
        assert_eq!(least_recently_checked(&[], &[(0.0, 0.0)]), None);
    }
}

#[cfg(test)]
mod target_choice_tests {
    use super::*;
    use serde_json::json;

    fn rules_from(runner_weight: f64, bot_weight: f64, range: f64, los: bool) -> TargetRules {
        let v = json!({
            "schema": "lr.hunter_targets/1",
            "runner_weight": runner_weight,
            "bot_weight": bot_weight,
            "bot_range": range,
            "bot_line_of_sight": los,
            "bot_targets": { "relaxed": false, "standard": false, "hard": true, "nightmare": true }
        });
        TargetRules::parse(&v).expect("test rules parse")
    }

    fn clear(_: (f32, f32), _: (f32, f32)) -> bool {
        true
    }

    #[test]
    fn embedded_standard_keeps_bots_off() {
        let r = TargetRules::get();
        assert!(!r.bots_allowed(Difficulty::Standard), "Standard chases the runner only, as before");
        assert!(!r.bots_allowed(Difficulty::Relaxed));
        assert!(r.bots_allowed(Difficulty::Hard));
        assert!(r.bots_allowed(Difficulty::Nightmare));
        assert_eq!(r.bot_range, 7.0);
        assert!(r.bot_line_of_sight);
    }

    #[test]
    fn standard_ignores_a_bot_right_beside_the_hunter() {
        let r = TargetRules::get();
        let got = choose_target(r, Difficulty::Standard, (0.0, 0.0), (9.0, 0.0), &[(0.5, 0.0)], clear);
        assert_eq!(got, None, "Standard: the runner stays the only target");
    }

    #[test]
    fn hard_chases_the_nearer_bot() {
        let r = TargetRules::get();
        let got = choose_target(r, Difficulty::Hard, (0.0, 0.0), (5.0, 0.0), &[(2.0, 0.0)], clear);
        assert_eq!(got, Some(0), "a bot 2 m off beats the runner 5 m off");
    }

    #[test]
    fn a_bot_beyond_range_is_not_a_candidate() {
        let r = rules_from(1.0, 1.0, 7.0, true);
        let got = choose_target(&r, Difficulty::Hard, (0.0, 0.0), (5.0, 0.0), &[(0.0, 9.0)], clear);
        assert_eq!(got, None, "9 m is past the 7 m bot range");
    }

    #[test]
    fn line_of_sight_gates_a_bot_when_the_rule_asks() {
        let blocked = |_: (f32, f32), _: (f32, f32)| false;
        let on = rules_from(1.0, 1.0, 7.0, true);
        assert_eq!(choose_target(&on, Difficulty::Hard, (0.0, 0.0), (5.0, 0.0), &[(2.0, 0.0)], blocked), None);
        let off = rules_from(1.0, 1.0, 7.0, false);
        assert_eq!(
            choose_target(&off, Difficulty::Hard, (0.0, 0.0), (5.0, 0.0), &[(2.0, 0.0)], blocked),
            Some(0),
            "with the rule off, a wall does not hide the bot"
        );
    }

    #[test]
    fn bot_weight_moves_the_choice() {
        // runner 3 m off, bot 2.5 m off
        let heavy = rules_from(1.0, 0.9, 7.0, true);
        assert_eq!(choose_target(&heavy, Difficulty::Hard, (0.0, 0.0), (3.0, 0.0), &[(2.5, 0.0)], clear), Some(0));
        let light = rules_from(1.0, 0.5, 7.0, true);
        assert_eq!(choose_target(&light, Difficulty::Hard, (0.0, 0.0), (3.0, 0.0), &[(2.5, 0.0)], clear), None);
    }

    #[test]
    fn a_tie_keeps_the_runner() {
        let r = rules_from(1.0, 1.0, 7.0, true);
        let got = choose_target(&r, Difficulty::Hard, (0.0, 0.0), (2.0, 0.0), &[(0.0, 2.0)], clear);
        assert_eq!(got, None);
    }

    #[test]
    fn no_bots_is_the_runner() {
        let r = TargetRules::get();
        assert_eq!(choose_target(r, Difficulty::Nightmare, (0.0, 0.0), (4.0, 0.0), &[], clear), None);
    }
}

#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::world::facility::Options;

    fn aki() -> HunterDef {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        crate::content::hunters::parse(&v, None, "").unwrap()
    }

    fn facility() -> Facility {
        let cat = crate::world::catalog::Catalog::load();
        Facility::generate("WIRING-1", &Options::default(), &cat)
    }

    /// A wall cell with an open cell beside it, and that open cell.
    fn wall_beside_floor(fac: &Facility) -> (Cell, Cell) {
        for y in 1..fac.rows - 1 {
            for x in 1..fac.cols - 1 {
                let w = Cell::new(x, y);
                if fac.hunter_open(w) {
                    continue;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let o = Cell::new(x + dx, y + dy);
                    if fac.hunter_open(o) {
                        return (w, o);
                    }
                }
            }
        }
        panic!("a generated floor has a wall beside floor");
    }

    #[test]
    fn state_names_feed_the_emote_transition_rules() {
        assert_eq!(State::Chase.name(), "CHASE");
        assert_eq!(crate::net::wiring::transition_rule(State::Wander.name(), State::Chase.name()), Some("spot"));
        assert_eq!(crate::net::wiring::transition_rule(State::Chase.name(), State::Search.name()), Some("lose"));
        assert_eq!(crate::net::wiring::transition_rule(State::Idle.name(), State::Search.name()), Some("search"));
    }

    #[test]
    fn an_open_gate_makes_its_cells_floor_and_a_raised_gate_does_not() {
        let cells = [Cell::new(3, 4), Cell::new(3, 5)];
        assert_eq!(gate_floor_cells(&cells, true).len(), 2);
        assert!(gate_floor_cells(&cells, false).is_empty());
        let mut b = Brain::new(aki(), (0.0, 0.0), "G", Difficulty::Standard);
        b.set_gate_floor(&cells, true);
        assert_eq!(b.gate_floor.len(), 2);
        b.set_gate_floor(&cells, false);
        assert!(b.gate_floor.is_empty());
    }

    #[test]
    fn a_hunter_routes_through_an_open_gate_cell_and_not_a_closed_one() {
        let fac = facility();
        let (wall, floor) = wall_beside_floor(&fac);
        assert!(plan_route(&fac, floor.center(), wall, &[], &[]).is_empty(), "a wall is not a goal");
        let through = plan_route(&fac, floor.center(), wall, &[], &[wall]);
        assert_eq!(through.last(), Some(&wall), "with the gate down the channel cell is a goal");
        assert!(body_fits_gated(&fac, &[wall], wall.center().0, wall.center().1));
        assert!(!body_fits_gated(&fac, &[], wall.center().0, wall.center().1));
    }

    #[test]
    fn a_state_change_fires_the_data_emote_rule_on_the_host_only() {
        let prof_dt = 0.1;
        let mut host_fired = 0;
        for i in 0..400 {
            let mut b = Brain::new(aki(), (0.0, 0.0), &format!("E{i}"), Difficulty::Standard);
            b.state = State::Chase;
            b.tick_emotes(prof_dt, State::Wander, 6.0);
            if let Some(e) = b.emote {
                assert!(super::super::emotes::Catalogue::get().emote(e).is_some(), "a real catalogue index");
                host_fired += 1;
            }
            let mut g = Brain::new(aki(), (0.0, 0.0), &format!("E{i}"), Difficulty::Standard);
            g.display_only = true;
            g.state = State::Chase;
            g.tick_emotes(prof_dt, State::Wander, 6.0);
            assert!(g.emote.is_none(), "a guest never starts an emote");
        }
        assert!(host_fired > 0, "the spot rule fires for some hunters (its chance is data)");
        assert!(host_fired < 400, "and not for all of them");
    }

    #[test]
    fn a_hunter_gives_up_an_unreachable_goal_within_the_data_time() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (_, floor) = wall_beside_floor(&fac);
        let mut b = Brain::new(aki(), floor.center(), "DEAD", Difficulty::Standard);
        b.active = true;
        b.state = State::Investigate;
        let goal = (-40.5, -40.5);
        let dt = 1.0 / 30.0;
        let mut t = 0.0;
        while b.state == State::Investigate && t < UNREACHABLE_GIVE_UP + 1.0 {
            b.pursue(goal, 2.0, dt, &fac, &prof);
            t += dt;
        }
        assert_eq!(b.state, State::Search, "no route: it stops pushing and searches");
        assert!(t <= UNREACHABLE_GIVE_UP + 2.0 * dt, "within the give-up time, took {t}");
    }

    #[test]
    fn the_give_up_time_is_data_and_matches_the_fallback_constant() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/hunters/search.json")).unwrap();
        assert_eq!(SearchRules::parse(&data).unreachable_give_up, UNREACHABLE_GIVE_UP);
        assert_eq!(search_rules().unreachable_give_up, UNREACHABLE_GIVE_UP);
        let mut changed = data.clone();
        changed["unreachable_give_up"] = serde_json::json!(0.5);
        assert_eq!(SearchRules::parse(&changed).unreachable_give_up, 0.5);
    }

    #[test]
    fn changing_the_give_up_data_changes_when_a_hunter_gives_up() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (_, floor) = wall_beside_floor(&fac);
        let dt = 1.0 / 30.0;
        let time_to_give_up = |secs: f32| {
            let mut b = Brain::new(aki(), floor.center(), "DEAD", Difficulty::Standard);
            b.give_up_after = secs;
            b.active = true;
            b.state = State::Investigate;
            let mut t = 0.0;
            while b.state == State::Investigate && t < 10.0 {
                b.pursue((-40.5, -40.5), 2.0, dt, &fac, &prof);
                t += dt;
            }
            t
        };
        let short = time_to_give_up(0.5);
        let long = time_to_give_up(3.0);
        assert!(short <= 0.5 + 2.0 * dt, "short give-up took {short}");
        assert!(long >= 3.0 - 2.0 * dt, "long give-up took {long}");
    }

    #[test]
    fn giving_up_leaves_a_chase_a_search_and_a_wander_idle() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (_, floor) = wall_beside_floor(&fac);
        for (from, to) in [(State::Chase, State::Search), (State::Investigate, State::Search), (State::Wander, State::Idle)] {
            let mut b = Brain::new(aki(), floor.center(), "GU", Difficulty::Standard);
            b.state = from;
            b.blocked_time = UNREACHABLE_GIVE_UP;
            b.give_up_goal(&fac, &prof);
            assert_eq!(b.state, to);
            assert_eq!(b.blocked_time, 0.0);
        }
    }

    // -- water and rails ---------------------------------------------------------------------------------------

    fn long_route(fac: &Facility) -> (Cell, Cell, Vec<Cell>) {
        let cells = fac.floor_cells();
        for a in cells.iter().step_by(7) {
            for b in cells.iter().rev().step_by(11) {
                let p = fac.find_path(*a, *b);
                if p.len() >= 14 {
                    return (*a, *b, p);
                }
            }
        }
        panic!("a generated floor has a long route");
    }

    fn field_on(cells: &[Cell], depth: f32) -> WaterField {
        let d: Vec<((i32, i32), f32)> = cells.iter().map(|c| ((c.x, c.y), depth)).collect();
        WaterField::from_depths(&d, &[])
    }

    fn everywhere_but(fac: &Facility, keep: &[Cell], depth: f32) -> WaterField {
        let cells: Vec<Cell> = fac.floor_cells().into_iter().filter(|c| !keep.contains(c)).collect();
        field_on(&cells, depth)
    }

    fn wet_route(fac: &Facility, a: Cell, b: Cell, rules: &HunterWater, field: &WaterField, chasing: bool) -> Vec<Cell> {
        let plan = WaterPlan { field, rules, chasing };
        plan_route_wet(fac, a.center(), b, &[], &[], Some(&plan))
    }

    #[test]
    fn a_hunter_paths_around_deep_water_when_idle() {
        let fac = facility();
        let (a, b, p) = long_route(&fac);
        let deep: Vec<Cell> = p[3..p.len() - 3].to_vec();
        let field = field_on(&deep, 2.0);
        let rules = HunterWater::default();
        let dry = wet_route(&fac, a, b, &rules, &WaterField::default(), false);
        assert!(dry.iter().any(|c| deep.contains(c)), "the dry route runs through the water cells");
        let wet = wet_route(&fac, a, b, &rules, &field, false);
        assert!(wet.iter().all(|c| !deep.contains(c)), "no step into deep water");
        assert!(wet.is_empty() || wet.last() == Some(&b));
    }

    #[test]
    fn a_hunter_wades_through_shallow_and_wading_depth() {
        let fac = facility();
        let (a, b, p) = long_route(&fac);
        let rules = HunterWater::default();
        for depth in [rules.shallow_below_m * 0.5, (rules.shallow_below_m + rules.deep_from_m) * 0.5] {
            let field = everywhere_but(&fac, &[a, b], depth);
            let route = wet_route(&fac, a, b, &rules, &field, false);
            assert_eq!(route.last(), Some(&b), "wades at depth {depth}");
            assert!(route.len() >= p.len().saturating_sub(2), "and still takes a real route");
        }
    }

    #[test]
    fn deep_water_is_crossed_only_when_chasing_and_able_to_swim() {
        let fac = facility();
        let (a, b, _) = long_route(&fac);
        let field = everywhere_but(&fac, &[a, b], 2.0);
        let walker = HunterWater::default();
        let swimmer = HunterWater { can_swim: true, ..walker };
        assert!(wet_route(&fac, a, b, &walker, &field, false).is_empty());
        assert!(wet_route(&fac, a, b, &walker, &field, true).is_empty(), "a walker never swims, chasing or not");
        assert!(wet_route(&fac, a, b, &swimmer, &field, false).is_empty(), "an idle swimmer still avoids it");
        assert_eq!(wet_route(&fac, a, b, &swimmer, &field, true).last(), Some(&b));
    }

    #[test]
    fn changing_the_water_data_changes_the_route() {
        let fac = facility();
        let (a, b, _) = long_route(&fac);
        let field = everywhere_but(&fac, &[a, b], 2.0);
        let mut v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/water.json")).unwrap();
        let std = WaterRules::parse(&v.to_string(), 0.45).unwrap().default;
        v["default"]["can_swim"] = serde_json::json!(true);
        let swims = WaterRules::parse(&v.to_string(), 0.45).unwrap().default;
        assert!(wet_route(&fac, a, b, &std, &field, true).is_empty());
        assert!(!wet_route(&fac, a, b, &swims, &field, true).is_empty());
    }

    #[test]
    fn a_hunter_replans_when_the_water_version_changes() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (a, b, _) = long_route(&fac);
        let mut br = Brain::new(aki(), a.center(), "WET", Difficulty::Standard);
        br.active = true;
        br.state = State::Investigate;
        let dt = 1.0 / 30.0;
        br.pursue(b.center(), 2.0, dt, &fac, &prof);
        assert!(!br.path.is_empty(), "a dry route");
        let flooded: Vec<(i32, i32)> =
            fac.floor_cells().into_iter().filter(|c| *c != a && *c != b).map(|c| (c.x, c.y)).collect();
        br.set_water(Arc::new(WaterField::from_depths(&[], &flooded)));
        br.pursue(b.center(), 2.0, dt, &fac, &prof);
        assert!(br.path.is_empty(), "a valve floods the way: the route was dropped and planned again on the next step");
        assert!(br.unreachable > 0.0);
        br.set_water(Arc::new(WaterField::default()));
        br.pursue(b.center(), 2.0, dt, &fac, &prof);
        assert!(!br.path.is_empty(), "the water drains: a route again, without waiting for the repath timer");
    }

    #[test]
    fn a_hunter_walled_in_by_water_gives_up_within_the_data_time() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (a, b, _) = long_route(&fac);
        let mut br = Brain::new(aki(), a.center(), "DROWN", Difficulty::Standard);
        br.active = true;
        br.state = State::Investigate;
        br.set_water(Arc::new(everywhere_but(&fac, &[a, b], 2.0)));
        let dt = 1.0 / 30.0;
        let mut t = 0.0;
        while br.state == State::Investigate && t < UNREACHABLE_GIVE_UP + 1.0 {
            br.pursue(b.center(), 2.0, dt, &fac, &prof);
            t += dt;
        }
        assert_eq!(br.state, State::Search, "water makes the goal unreachable: it stops and searches");
        assert!(t <= br.give_up_after + 2.0 * dt, "within the give-up time, took {t}");
    }

    #[test]
    fn a_hunter_standing_in_water_is_slower_and_splashes() {
        let prof = Difficulty::Standard.profile();
        let mut br = Brain::new(aki(), (4.5, 4.5), "SLOW", Difficulty::Standard);
        let dry = br.current_speed(1, 0, &prof, 1.0);
        assert_eq!(br.water_splash_noise(), 0.0);
        br.set_water(Arc::new(WaterField::from_depths(&[((4, 4), 0.5)], &[])));
        assert!(br.current_speed(1, 0, &prof, 1.0) < dry);
        assert!(br.water_splash_noise() > 0.0);
        br.pos = (9.5, 9.5);
        assert_eq!(br.current_speed(1, 0, &prof, 1.0), dry, "dry ground keeps full speed");
    }

    fn ctx(fac: &Facility) -> Ctx<'_> {
        Ctx {
            fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: false,
        }
    }

    fn run(b: &mut Brain, fac: &Facility, t: &Target, seconds: f32) {
        let prof = Difficulty::Standard.profile();
        let c = ctx(fac);
        for _ in 0..(seconds * 60.0).round() as i32 {
            b.update(1.0 / 60.0, &c, t, 1, 0, &prof);
        }
    }

    fn watcher(at: (f32, f32), y: f32) -> Brain {
        let mut b = Brain::new(aki(), at, "WATCH", Difficulty::Standard);
        b.active = true;
        b.y = y;
        b
    }

    fn runner_at(p: (f32, f32), y: f32) -> Target {
        Target { pos: p, y, facing: (1.0, 0.0), flashlight_on: true, ..Default::default() }
    }

    /// An open floor cell 2 to 6 m from `from` that `from` has a clear line to.
    fn floor_in_sight(fac: &Facility, from: Cell) -> Option<Cell> {
        fac.floor_cells().into_iter().find(|c| {
            let d = dist(c.center(), from.center());
            fac.hunter_open(*c) && (2.0..6.0).contains(&d) && fac.has_line_of_sight(from.center(), c.center())
        })
    }

    #[test]
    fn a_runner_part_way_up_a_flight_over_the_hunters_floor_is_still_dashed_at() {
        // the dash needs the surface under her, not her feet height: climbing a flight or a ladder, or falling, over a
        // floor cell is still the hunter's floor, so her height changes nothing about whether she is dashed at
        let fac = facility();
        let dashed = |ground: Cell, seen: Cell, y: f32| {
            let mut b = watcher(ground.center(), 0.0);
            run(&mut b, &fac, &runner_at(seen.center(), y), 0.5);
            b.state == State::Chase && b.direct_visible
        };
        let (ground, seen_floor) = fac
            .floor_cells()
            .into_iter()
            .filter(|c| fac.hunter_open(*c))
            .filter_map(|c| floor_in_sight(&fac, c).map(|s| (c, s)))
            .take(60)
            .find(|(g, s)| dashed(*g, *s, 0.0))
            .expect("a hunter cell with a runner on the floor in sight that it chases");
        for y in [0.8, 2.0, 3.4] {
            assert!(dashed(ground, seen_floor, y), "a runner at feet height {y} over the floor is dashed at like one on it");
        }
    }
}

/// Cross-level behaviour (continuous levels, phase 8). The stage comes from `flight_edges::test_nav`; `stage` and
/// `stage_with` are the only places that know its shape.
#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::game::level_stack::floor_y_of;
    use crate::hunters::flight_edges::{self, FlightEdgeRules as FlightRules, TestShape, TestStage as Stage, TEST_HIGH_LEVEL as HIGH, TEST_LOW_LEVEL as LOW};
    use crate::world::facility::FLIGHT_FIT_TOL;
    use crate::world::vertical::LADDER_CLIMB_SPEED;

    fn aki() -> HunterDef {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        crate::content::hunters::parse(&v, None, "").unwrap()
    }

    fn stage(shape: TestShape) -> Stage {
        stage_with(shape, flight_edges::rules().clone())
    }

    fn stage_with(shape: TestShape, rules: FlightRules) -> Stage {
        flight_edges::test_stage(shape, &rules)
    }

    fn hunter(s: &Stage, at: (f32, f32), level: i32) -> Brain {
        let mut b = Brain::new(aki(), at, "EDGE", Difficulty::Standard);
        b.active = true;
        b.set_level(level);
        b.set_nav(s.nav.clone());
        b
    }

    fn ctx(fac: &Facility) -> Ctx<'_> {
        Ctx {
            fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: false,
        }
    }

    /// A target on `level` at `p` that is seen (an open well) or not.
    fn runner(p: (f32, f32), level: i32, sees: bool) -> Target {
        Target {
            pos: p,
            y: floor_y_of(level),
            level,
            flashlight_on: true,
            cross: Some(CrossSense { sees, hear_scale: 1.0 }),
            ..Default::default()
        }
    }

    /// Updates at 1/60 s until `done`, returning the seconds taken (None when `limit` ran out). `each` sees every frame.
    fn run_until(
        b: &mut Brain,
        s: &Stage,
        t: &Target,
        limit: f32,
        mut each: impl FnMut(&Brain),
        done: impl Fn(&Brain) -> bool,
    ) -> Option<f32> {
        let prof = Difficulty::Standard.profile();
        let dt = 1.0 / 60.0;
        let mut time = 0.0;
        while time < limit {
            // the facility of the level the hunter is on now (the lists swap when its level flips)
            let c = ctx(s.fac_of(b.level));
            b.update(dt, &c, t, 1, 0, &prof);
            time += dt;
            each(b);
            if done(b) {
                return Some(time);
            }
        }
        None
    }

    fn crossed(b: &Brain, level: i32) -> bool {
        b.level == level && (b.y - floor_y_of(level)).abs() < 0.05 && b.walk.is_none()
    }

    #[test]
    fn a_hunter_chases_the_runner_up_a_flight() {
        for shape in [TestShape::Switchback, TestShape::Straight, TestShape::Spiral] {
            let s = stage(shape);
            let mut b = hunter(&s, s.low, LOW);
            let t = runner(s.high, HIGH, true);
            let mut last_y = b.y;
            let mut off_surface = 0.0f32;
            let took = run_until(
                &mut b,
                &s,
                &t,
                45.0, // the Switchback foot can hold a hunter for ten seconds at some chase speeds (1.06 and 1.09 times); it still gets up
                |h| {
                    assert!(h.y >= last_y - 1e-4, "{shape:?}: y is monotone going up {} -> {} level {} walk {:?} pos {:?}", last_y, h.y, h.level, h.walk.as_ref().map(|w| (w.s, w.len)), h.pos);
                    last_y = h.y;
                    if h.walk.is_some() {
                        off_surface = off_surface.max(flight_edges::surface_error(&s, h.pos, h.y));
                    }
                },
                |h| crossed(h, HIGH),
            );
            assert!(took.is_some(), "{shape:?}: the hunter reaches the upper floor");
            assert!(off_surface <= FLIGHT_FIT_TOL, "{shape:?}: stayed on the flight surface ({off_surface})");
        }
    }

    #[test]
    fn a_hunter_chases_down_a_flight() {
        for shape in [TestShape::Switchback, TestShape::Straight, TestShape::Spiral] {
            let s = stage(shape);
            let mut b = hunter(&s, s.high, HIGH);
            let t = runner(s.low, LOW, true);
            let mut last_y = b.y;
            let took = run_until(
                &mut b,
                &s,
                &t,
                25.0,
                |h| {
                    assert!(h.y <= last_y + 1e-4, "{shape:?}: y is monotone going down");
                    last_y = h.y;
                },
                |h| crossed(h, LOW),
            );
            assert!(took.is_some(), "{shape:?}: the hunter reaches the lower floor");
        }
    }

    #[test]
    fn the_brain_flips_level_in_the_middle_of_a_flight() {
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.low, LOW);
        let t = runner(s.high, HIGH, true);
        let mut flipped_mid = false;
        run_until(&mut b, &s, &t, 25.0, |h| flipped_mid |= h.level == HIGH && h.walk.is_some(), |h| crossed(h, HIGH));
        assert!(flipped_mid, "the level changes while still on the flight (hysteresis), not at the end");
    }

    fn ladder_time(rules: FlightRules, down: bool) -> (f32, f32) {
        let s = stage_with(TestShape::Ladder, rules);
        let (from, level, to_level) = if down { (s.high, HIGH, LOW) } else { (s.low, LOW, HIGH) };
        let mut b = hunter(&s, from, level);
        let t = runner(if down { s.low } else { s.high }, to_level, true);
        let took = run_until(&mut b, &s, &t, 60.0, |_| {}, |h| crossed(h, to_level)).expect("the ladder is climbed");
        (took, s.len)
    }

    #[test]
    fn a_hunter_chases_down_a_ladder() {
        let base = flight_edges::rules().clone();
        let (took, len) = ladder_time(base.clone(), true);
        let floor = len / (LADDER_CLIMB_SPEED * base.ladder_speed_scale);
        assert!(took >= floor - 0.05, "a ladder takes at least len / climb speed: {took} vs {floor}");
        let mut slow = base;
        slow.ladder_speed_scale *= 0.5;
        let (slower, _) = ladder_time(slow, true);
        assert!(slower > took, "the ladder speed is data: {slower} vs {took}");
    }

    #[test]
    fn a_hunter_climbs_a_ladder_up() {
        let (took, _) = ladder_time(flight_edges::rules().clone(), false);
        assert!(took > 0.0);
    }

    #[test]
    fn a_hunter_does_not_jump_a_drop_by_default() {
        let base = flight_edges::rules().clone();
        assert!(!base.drop_allowed, "Standard keeps drops off");
        let s = stage_with(TestShape::Drop, base.clone());
        let mut b = hunter(&s, s.high, HIGH);
        let t = runner(s.low, LOW, true);
        let give_up = search_rules().unreachable_give_up;
        let took = run_until(&mut b, &s, &t, give_up + 3.0, |h| assert!(h.walk.is_none(), "no jump"), |h| h.state == State::Search);
        assert!(took.is_some(), "the unreachable goal is given up and the hunter searches");
        assert_eq!(b.level, HIGH);

        let mut on = base;
        on.drop_allowed = true;
        let s = stage_with(TestShape::Drop, on);
        let mut b = hunter(&s, s.high, HIGH);
        let t = runner(s.low, LOW, true);
        assert!(run_until(&mut b, &s, &t, 25.0, |_| {}, |h| crossed(h, LOW)).is_some(), "with the data on, the drop is taken");
        // and it never climbs back up the drop
        let far = runner(s.high, HIGH, true);
        run_until(&mut b, &s, &far, 12.0, |_| {}, |h| h.level == HIGH);
        assert_eq!(b.level, LOW, "a drop is one way");
    }

    #[test]
    fn a_hunter_sees_through_a_well_not_a_slab() {
        let s = stage(TestShape::Straight);
        let prof = Difficulty::Standard.profile();
        let c = ctx(s.fac_of(LOW));
        for (sees, chased) in [(true, true), (false, false)] {
            let mut b = hunter(&s, s.low, LOW);
            b.state = State::Wander;
            let t = Target { noise: 0.0, ..runner((s.low.0 + 1.0, s.low.1), HIGH, sees) };
            for _ in 0..30 {
                b.update(1.0 / 60.0, &c, &t, 1, 0, &prof);
            }
            assert_eq!(b.state == State::Chase, chased, "sees={sees}");
            if chased {
                assert_eq!(b.last_known_level, HIGH);
            }
        }
    }

    #[test]
    fn a_capture_needs_the_same_height() {
        let s = stage(TestShape::Straight);
        let prof = Difficulty::Standard.profile();
        let c = ctx(s.fac_of(LOW));
        let mut near = runner(s.low, LOW, true);
        near.cross = None;
        let mut b = hunter(&s, s.low, LOW);
        b.state = State::Chase;
        b.last_known = Some(s.low);
        assert!(b.update(1.0 / 60.0, &c, &near, 1, 0, &prof).0, "same spot, same height: caught");
        let mut above = near.clone();
        above.y += flight_edges::rules().capture_height + 1.0;
        let mut b = hunter(&s, s.low, LOW);
        b.state = State::Chase;
        b.last_known = Some(s.low);
        assert!(!b.update(1.0 / 60.0, &c, &above, 1, 0, &prof).0, "right above: not caught");
    }

    #[test]
    fn a_hunter_stuck_on_an_edge_gives_up_within_the_data_time() {
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.low, LOW);
        let t = runner(s.high, HIGH, true);
        run_until(&mut b, &s, &t, 25.0, |_| {}, |h| h.on_edge()).expect("the hunter steps onto the flight");
        let prof = Difficulty::Standard.profile();
        let c = ctx(s.fac_of(b.level));
        let held = b.walk.clone();
        // the brain measures the limit with its live speed (stamina and dashes change it), so the slowest speed it had
        // on the edge gives the latest moment it may let go
        let mut slowest = b.current_speed(1, 0, &prof, 1.0);
        let mut on_edge_for = 0.0;
        while b.on_edge() && on_edge_for < 20.0 {
            b.walk = held.clone(); // freeze the advance
            b.update(1.0 / 60.0, &c, &t, 1, 0, &prof);
            slowest = slowest.min(b.current_speed(1, 0, &prof, 1.0));
            on_edge_for += 1.0 / 60.0;
        }
        let limit = s.len / slowest.max(0.2) + flight_edges::rules().edge_stuck_seconds;
        assert!(!b.on_edge(), "put back on an end");
        assert!(on_edge_for >= flight_edges::rules().edge_stuck_seconds, "not before the data time: {on_edge_for}");
        assert!(on_edge_for <= limit + 0.2, "within the data time: {on_edge_for} vs {limit}");
        assert!(b.y == floor_y_of(LOW) || b.y == floor_y_of(HIGH));
        assert_ne!(b.state, State::Chase, "the chase gives up to a search");
    }

    #[test]
    fn the_edge_walk_survives_a_nav_rebuild() {
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.low, LOW);
        let t = runner(s.high, HIGH, true);
        run_until(&mut b, &s, &t, 25.0, |_| {}, |h| h.on_edge()).expect("on the edge");
        let s2 = stage(TestShape::Straight);
        b.set_nav(s2.nav.clone());
        assert!(b.on_edge(), "a new graph does not drop the walk in progress");
        assert!(run_until(&mut b, &s, &t, 25.0, |_| {}, |h| crossed(h, HIGH)).is_some());
    }

    #[test]
    fn a_hunter_with_no_route_to_another_level_gives_up_within_the_data_time() {
        let s = stage(TestShape::Straight);
        let mut b = Brain::new(aki(), s.low, "EDGE", Difficulty::Standard);
        b.active = true; // no graph set: nothing leads off the floor
        b.investigate_on(s.high, HIGH);
        let t = Target { pos: (-500.0, -500.0), hidden: true, ..Default::default() };
        let give_up = search_rules().unreachable_give_up;
        let took = run_until(&mut b, &s, &t, give_up + 3.0, |_| {}, |h| h.state == State::Search).expect("gives up");
        assert!(took <= give_up + 1.0, "{took} vs {give_up}");
    }
}

/// A hunter on the Inferno's noisy floor (`world/inferno_terrain.rs`).
#[cfg(test)]
mod inferno_floor_tests {
    use super::*;
    use crate::content::loot::LootTables;
    use crate::world::catalog::Catalog;
    use crate::world::infinite::{window, INFERNO_LEVEL};

    #[test]
    fn a_hunter_path_across_the_noisy_floor_keeps_its_feet_on_the_surface() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        let def = crate::content::hunters::parse(&v, None, "").unwrap();
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let mut cache = HashMap::new();
        let (fac, _) = window("1426541270", (0, 0), INFERNO_LEVEL, &cat, &loot, &mut cache);
        assert_eq!(fac.level, INFERNO_LEVEL);
        let terrain = fac.terrain().expect("terrain");
        // a route of 40 or more cells between two open cells
        let cells = fac.floor_cells();
        let mut route = None;
        'search: for a in cells.iter().step_by(61) {
            for b in cells.iter().step_by(53) {
                if (a.x - b.x).abs() + (a.y - b.y).abs() >= 40 && fac.find_path(*a, *b).len() >= 40 {
                    route = Some((*a, *b));
                    break 'search;
                }
            }
        }
        let (from, to) = route.expect("a long route on the Inferno floor");
        let ctx = Ctx {
            fac: &fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: false,
        };
        let prof = Difficulty::Standard.profile();
        let mut b = Brain::new(def, from.center(), "PIN", Difficulty::Standard);
        b.active = true;
        b.set_level(INFERNO_LEVEL);
        b.chase(to.center());
        let far = Target { pos: to.center(), facing: (1.0, 0.0), level: INFERNO_LEVEL, ..Default::default() };
        let floor = crate::game::level_stack::floor_y_of(INFERNO_LEVEL);
        let (mut walked, mut prev, mut lo, mut hi) = (0.0_f32, (b.pos, b.y), f32::MAX, f32::MIN);
        for _ in 0..1200 {
            b.update(1.0 / 60.0, &ctx, &far, 1, 0, &prof);
            // the feet are exactly on the surface under the hunter, and a frame never moves them like a step
            assert!(
                (b.y - (floor + terrain.floor_at(b.pos.0, b.pos.1))).abs() < 1e-4,
                "feet {} over ground {} at {:?}",
                b.y,
                floor + terrain.floor_at(b.pos.0, b.pos.1),
                b.pos
            );
            assert!(fac.body_fits(b.pos.0, b.pos.1, HUNTER_RADIUS), "the hunter walked into a wall at {:?}", b.pos);
            assert!((b.y - prev.1).abs() < 0.05, "a {} m jump in one frame", b.y - prev.1);
            walked += dist(prev.0, b.pos);
            prev = (b.pos, b.y);
            lo = lo.min(b.y);
            hi = hi.max(b.y);
        }
        assert!(walked > 20.0, "the hunter walked {walked} cells");
        assert!(hi - lo > 0.05, "its route crossed no height ({lo}..{hi})");
        assert_eq!(b.level, INFERNO_LEVEL);
    }
}

#[cfg(test)]
mod route_budget_tests {
    //! The route scheduler's budget seen from a brain: a deferred search keeps the route the hunter has, and a crowd
    //! under a tight budget still reaches the runner.
    use super::*;
    use crate::config::Difficulty;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn aki() -> HunterDef {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        crate::content::hunters::parse(&v, None, "").unwrap()
    }

    fn facility() -> Facility {
        Facility::generate("BUDGET-1", &Options::default(), &Catalog::load())
    }

    #[test]
    fn a_hunter_whose_search_is_deferred_keeps_walking_the_route_it_has() {
        path_scheduler::reset_thread();
        path_scheduler::set_searches_per_frame(Some(0));
        let prof = Difficulty::Standard.profile();
        let fac = facility();
        let cells = fac.floor_cells();
        let (a, b, route) = cells
            .iter()
            .step_by(13)
            .flat_map(|a| cells.iter().rev().step_by(29).map(move |b| (*a, *b)))
            .find_map(|(a, b)| {
                let p = fac.find_path(a, b);
                (p.len() >= 30).then_some((a, b, p))
            })
            .expect("the test floor has a long route");
        let mut h = Brain::new(aki(), a.center(), "BUDGET-WALK", Difficulty::Standard);
        h.active = true;
        h.state = State::Chase;
        h.last_known = Some(b.center());
        h.path = route;
        h.path_goal = Some(b);
        h.path_timer = 0.0;
        let speed = h.current_speed(1, 0, &prof, 1.0);
        let start = h.pos;
        for _ in 0..30 {
            path_scheduler::begin_frame(&fac);
            h.pursue(b.center(), speed, 1.0 / 60.0, &fac, &prof);
        }
        assert!(path_scheduler::stats().deferred >= 1, "the search was asked for and deferred");
        assert!(!h.path.is_empty(), "the route it has is kept while the new one waits");
        assert!(dist(start, h.pos) > 1.0, "and it walks it: moved {:.2} m", dist(start, h.pos));
        assert_eq!(h.unreachable, 0.0, "a route that is late is not a route that is missing");
        path_scheduler::reset_thread();
    }

    #[test]
    fn hunters_still_reach_the_runner_under_a_tight_budget() {
        path_scheduler::reset_thread();
        path_scheduler::set_searches_per_frame(Some(2));
        let prof = Difficulty::Standard.profile();
        let fac = facility();
        let goal = fac.floor_cells()[fac.floor_cells().len() / 2];
        let starts: Vec<Cell> = fac
            .floor_cells()
            .into_iter()
            .step_by(17)
            .filter(|c| {
                let d = dist(c.center(), goal.center());
                (4.0..14.0).contains(&d) && fac.find_path(*c, goal).len() >= 6
            })
            .take(8)
            .collect();
        assert!(starts.len() >= 4, "the test floor has hunters' starts round the runner: {}", starts.len());
        let mut hunters: Vec<Brain> = starts
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut h = Brain::new(aki(), s.center(), &format!("BUDGET-{i}"), Difficulty::Standard);
                h.active = true;
                h.state = State::Chase;
                h.last_known = Some(goal.center());
                h
            })
            .collect();
        let speed = hunters[0].current_speed(1, 0, &prof, 1.0);
        let mut reached = vec![false; hunters.len()];
        for _ in 0..(60 * 40) {
            path_scheduler::begin_frame(&fac);
            for (i, h) in hunters.iter_mut().enumerate() {
                if !reached[i] {
                    h.pursue(goal.center(), speed, 1.0 / 60.0, &fac, &prof);
                    reached[i] = dist(h.pos, goal.center()) <= 1.0;
                }
            }
            if reached.iter().all(|r| *r) {
                break;
            }
        }
        let deferred = path_scheduler::stats().deferred;
        path_scheduler::reset_thread();
        assert!(deferred > 0, "the budget was tight: some searches waited for a frame");
        assert!(reached.iter().all(|r| *r), "every hunter reached the runner: {reached:?} of {} starts", starts.len());
    }
}

#[cfg(test)]
mod mechanic_review_tests {
    use super::*;

    /// A simulating brain (not a display copy) lets go of a name a host once sent, instead of holding the pose for good.
    #[test]
    fn a_simulating_brain_drains_a_stale_host_name() {
        let content = crate::content::Content::load();
        let def = content.hunter("aki").expect("aki").clone();
        let mut b = Brain::new(def, (1.5, 1.5), "PIN", Difficulty::Standard);
        b.set_net_mechanic(Some("steal_battery"));
        assert_eq!(b.mechanic(), Some("steal_battery"));
        b.tick_cue(0.3);
        assert_eq!(b.mechanic(), Some("steal_battery"), "still inside net_ttl");
        b.tick_cue(1.0);
        assert_eq!(b.mechanic(), None, "a brain that ticks its own cue clock forgets the name");
    }
}

/// data/progression.json `chase_speed_scale`: every hunter is a little faster while chasing, nothing else changes.
#[cfg(test)]
mod chase_speed_tests {
    use super::*;
    use crate::config::{Difficulty, PLAYER_SPRINT_SPEED};
    use crate::content::Content;
    use crate::game::progression::Progression;

    const PROFILES: [Difficulty; 4] =
        [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare];

    fn brain_of(def: &HunterDef, diff: Difficulty, state: State) -> Brain {
        let mut b = Brain::new(def.clone(), (0.0, 0.0), "PIN", diff);
        b.state = state;
        b
    }

    /// Depth 1, no relays: the chase speed is `chase` times the scale; wander, investigate and search keep the old numbers.
    #[test]
    fn every_hunter_chases_at_the_scaled_speed_and_walks_at_the_old_one() {
        let rules = Progression::load();
        assert!(
            rules.chase_speed_scale > 1.05 && rules.chase_speed_scale < 1.12,
            "slightly faster: {}",
            rules.chase_speed_scale
        );
        let content = Content::base();
        assert!(content.hunters.len() >= 38);
        let prof = Difficulty::Standard.profile();
        for def in &content.hunters {
            let scale = if def.category == "secret" { 1.0 } else { rules.chase_speed_scale };
            let chase = brain_of(def, Difficulty::Standard, State::Chase).current_speed(1, 0, &prof, 1.0);
            assert!((chase - def.chase * scale).abs() < 1e-4, "{}: chase {chase} vs {} x {scale}", def.id, def.chase);
            for state in [State::Wander, State::Investigate, State::Search] {
                let walk = brain_of(def, Difficulty::Standard, state).current_speed(1, 0, &prof, 1.0);
                assert!((walk - def.speed).abs() < 1e-5, "{} {state:?}: {walk} vs {}", def.id, def.speed);
            }
        }
    }

    /// The secret wolf spiders keep the speed they were authored with.
    #[test]
    fn the_secret_creatures_keep_their_authored_chase_speed() {
        let content = Content::base();
        let prof = Difficulty::Standard.profile();
        for id in ["white_wolf_spider", "black_wolf_spider"] {
            let def = content.hunter(id).unwrap();
            assert_eq!(def.category, "secret");
            let chase = brain_of(def, Difficulty::Standard, State::Chase).current_speed(1, 0, &prof, 1.0);
            assert!((chase - def.chase).abs() < 1e-5, "{id}: {chase} vs {}", def.chase);
        }
    }

    /// The scale is one data key: a changed value, a skipped category, and a missing key (the shipped default).
    #[test]
    fn the_scale_comes_from_the_data_file() {
        let p = Progression::parse(&serde_json::json!({ "chase_speed_scale": 1.5, "chase_speed_scale_skip": ["void"] }));
        assert_eq!(p.chase_scale_for("az"), 1.5);
        assert_eq!(p.chase_scale_for("void"), 1.0);
        assert_eq!(p.chase_scale_for("secret"), 1.5, "the skip list is replaced, not merged");
        let d = Progression::parse(&serde_json::json!({}));
        assert_eq!(d.chase_speed_scale, Progression::load().chase_speed_scale);
        assert_eq!(d.chase_scale_for("secret"), 1.0);
        let off = Progression::parse(&serde_json::json!({ "chase_speed_scale": 1.0 }));
        assert_eq!(off.chase_scale_for("az"), 1.0, "1.0 gives the original roster back");
    }

    /// The relay/depth bonus, winded, taser slow and the difficulty profile all stack on top of the scaled speed.
    #[test]
    fn difficulty_slow_winded_and_relays_still_multiply_the_scaled_speed() {
        let rules = Progression::load();
        let content = Content::base();
        let aki = content.hunter("aki").unwrap();
        for diff in PROFILES {
            let prof = diff.profile();
            let mut b = brain_of(aki, diff, State::Chase);
            let base = aki.chase * rules.chase_speed_scale * prof.hunter_speed;
            assert!((b.current_speed(1, 0, &prof, 1.0) - base).abs() < 1e-4, "{diff:?}");
            // relays: the bonus times `prog`, then the scale
            let with_relays = (aki.chase + 3.0 * rules.chase_speed_bonus_per_relay * aki.prog)
                * rules.chase_speed_scale
                * prof.hunter_speed;
            assert!((b.current_speed(1, 3, &prof, 1.0) - with_relays).abs() < 1e-4, "{diff:?} relays");
            b.slow_timer = 3.0;
            b.slow_factor = 0.5;
            assert!((b.current_speed(1, 0, &prof, 1.0) - base * 0.5).abs() < 1e-4, "{diff:?} taser slow");
            b.slow_timer = 0.0;
            b.winded = true;
            assert!(
                (b.current_speed(1, 0, &prof, 1.0) - base * HUNTER_WINDED_SPEED_SCALE).abs() < 1e-4,
                "{diff:?} winded"
            );
        }
    }

    /// A deterred Solyth drops out of Chase into a Wander, so the cold source holds her at her walking pace.
    #[test]
    fn a_deterred_solyth_moves_at_her_wander_speed() {
        let content = Content::base();
        let solyth = content.hunter("solyth").unwrap();
        let prof = Difficulty::Standard.profile();
        let mut b = brain_of(solyth, Difficulty::Standard, State::Chase);
        let chasing = b.current_speed(1, 0, &prof, 1.0);
        // what Solyth::after does to the brain while a deterrent is in reach
        b.state = State::Wander;
        let retreating = b.current_speed(1, 0, &prof, 1.0);
        assert!((retreating - solyth.speed).abs() < 1e-5, "{retreating}");
        assert!(chasing > retreating * 3.0);
    }

    /// Before/after numbers for every hunter (run with --nocapture for the table in the commit message). Asserts that
    /// the plain A-Z roster can still be out-run on a full-stamina sprint at Standard, even on a clear-line dash.
    #[test]
    fn a_full_stamina_runner_still_outruns_the_az_roster_at_standard() {
        let rules = Progression::load();
        let content = Content::base();
        let sprint = PLAYER_SPRINT_SPEED * Difficulty::Standard.profile().sprint_speed;
        println!("runner sprint: standard {sprint:.2}");
        println!("hunter                  old    new | oldDsh newDsh | newNM newNMdsh");
        let nm = Difficulty::Nightmare.profile();
        for def in &content.hunters {
            let scale = rules.chase_scale_for(&def.category);
            let (old, new) = (def.chase, def.chase * scale);
            let dash = HUNTER_VISIBLE_CHASE_SPEED_SCALE;
            println!(
                "{:<20} {:>6.2} {:>6.2} | {:>6.2} {:>6.2} | {:>6.2} {:>6.2}   [{}]",
                def.id,
                old,
                new,
                old * dash,
                new * dash,
                new * nm.hunter_speed,
                new * nm.hunter_speed * dash,
                def.category
            );
            if def.category == "az" {
                assert!(new * dash < sprint, "{}: dash {} vs sprint {sprint}", def.id, new * dash);
            }
        }
    }

    /// Every profile, every hunter: print the chase speed (the dash is 1.2x that on a clear line).
    #[test]
    fn print_the_chase_speed_table_per_difficulty() {
        let content = Content::base();
        for diff in PROFILES {
            let prof = diff.profile();
            println!("{diff:?}: runner sprint {:.2}", PLAYER_SPRINT_SPEED * prof.sprint_speed);
            let mut worst: f32 = 0.0;
            for def in content.hunters.iter().filter(|d| d.category == "az") {
                worst = worst.max(brain_of(def, diff, State::Chase).current_speed(1, 0, &prof, 1.0));
            }
            println!("  fastest A-Z chase {worst:.2}, dash {:.2}", worst * HUNTER_VISIBLE_CHASE_SPEED_SCALE);
        }
    }

    /// A hostile data file cannot zero, freeze or runaway the roster: the scale is clamped, junk skip entries dropped.
    #[test]
    fn a_hostile_scale_is_clamped_and_junk_skip_entries_are_ignored() {
        let hi = Progression::parse(&serde_json::json!({ "chase_speed_scale": 1.0e9 }));
        assert!(hi.chase_speed_scale <= 2.0);
        let lo = Progression::parse(&serde_json::json!({ "chase_speed_scale": -5.0 }));
        assert!(lo.chase_speed_scale >= 0.5);
        let junk = Progression::parse(
            &serde_json::json!({ "chase_speed_scale": "fast", "chase_speed_scale_skip": [1, null, "az"] }),
        );
        assert_eq!(junk.chase_speed_scale, Progression::default().chase_speed_scale, "a non-number keeps the default");
        assert_eq!(junk.chase_scale_for("az"), 1.0);
        assert_eq!(junk.chase_scale_for("void"), junk.chase_speed_scale);
    }
}
