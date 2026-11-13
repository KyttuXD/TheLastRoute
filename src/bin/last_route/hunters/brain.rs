//! The shared hunter brain (game/hunters/ai/hunter_brain.gd): beam attention, perception
//! (sight + hearing), stamina, the camp guard and the idle / wander / investigate / chase /
//! search state machine, with each hunter's gimmick hooked in at the original's points.
//! Positions are grid cells (1 cell = 1 m).

use super::activities::{self, Activity, ActivityRun, Poi};
use super::naps;
use super::idle_life::{self, IdleRules, IdleState};
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
    let steps = ((d / walk_rules().sweep_step).ceil() as i32).max(1);
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

/// How far (cells) a hunter standing in closed water looks for the shore.
const ESCAPE_RINGS: i32 = 8;

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
            None => {
                // standing in closed water wider than one cell (a pool): head for the nearest cell it may enter, by rings,
                // so the hunter wades out instead of being stuck with no route
                let mut ring: Vec<Cell> = (-ESCAPE_RINGS..=ESCAPE_RINGS)
                    .flat_map(|dy| (-ESCAPE_RINGS..=ESCAPE_RINGS).map(move |dx| Cell::new(here.x + dx, here.y + dy)))
                    .filter(|c| open(*c))
                    .collect();
                ring.sort_by(|a, b| dist(a.center(), from).total_cmp(&dist(b.center(), from)));
                match ring.first() {
                    Some(c) => *c,
                    None => return vec![],
                }
            }
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
    /// PUBLIC route patrol (app/public_route.rs): the runner this hunter guards while it wanders, and the rules. None elsewhere.
    pub patrol_anchor: Option<(f32, f32)>,
    pub patrol: Option<&'static crate::content::public_roster::PublicPatrol>,
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
    /// A cross-floor goal that had no hop: (nav version, goal cell, goal level, own level, seconds left). While it holds the
    /// hop search is skipped, so a goal with no flight is not searched again every frame.
    no_hop_memo: Option<(u64, Cell, i32, i32, f32)>,
    /// What the cached `hop` was planned for: (nav version, goal cell, goal level, own level). The hop is kept while this
    /// holds and replanned only when it changes (or every `cross_repath_seconds` for a goal cell that moved).
    hop_key: Option<(u64, Cell, i32, i32)>,
    /// Seconds since the cached hop was planned.
    hop_age: f32,
    /// The level a cross-level wander or search goal is on (a stair's far end); None for an ordinary goal. It counts
    /// only while the goal still equals `cross_goal`, the point it was picked with.
    wander_level: Option<i32>,
    cross_goal: Option<(f32, f32)>,
    /// A runner that left this chaser's level: (seconds left, the point it arrived at, its level). Keeps the memory alive
    /// for `follow_cross_seconds` so the chaser walks the flight after it.
    follow_cross: Option<(f32, (f32, f32), i32)>,
    /// Debug counters (admin hunter stats): edge walks ended by setting the body on an end of the edge (only when a
    /// refused walker could not walk back either, so it stays 0 on an edge that exists), and goals given up.
    pub snap_count: u32,
    pub giveup_count: u32,
    /// Slowdown (0..1) the director asks for this frame on a shared edge (follow the leader); `walk_edge` reads and resets it.
    pub(crate) edge_hold: f32,
    /// This hunter's fixed offset from the centre of its landing cell (<= `land_spread`), so followers do not stack.
    land_offset: (f32, f32),
    /// Set when a walk set the hunter down off the centre of its landing cell: the next update checks that spot against the
    /// landing level's own facility (`hunter_fits`; the walk only has the level it left) and falls back to the cell centre.
    land_check: bool,
    /// Seconds in a row the next step on the current edge has been refused (the footprint test); 0 while it advances.
    /// A walker the director holds back is not refused, so waiting behind another walker never counts.
    edge_clock: f32,
    /// The walk on the current edge was turned round after `edge_stuck_seconds` of refusal: the hunter is walking back
    /// to the end it came from, and gives its goal up when it lands there.
    edge_retreat: bool,
    /// The point of interest the current wander goal was picked from (hunters/activities.rs); None for a goal set
    /// by anything else (camp guard, gimmick, fallback).
    poi: Option<Poi>,
    /// The goals picked lately (cell ids, newest last), so a wander never repeats one.
    recent_pois: Vec<(i32, i32)>,
    /// The activity being done at the goal; exposed through `activity_view` for the renderer and the wire.
    activity: Option<ActivityRun>,
    /// Head turn toward a heard noise and the glances of an Idle pause (hunters/idle_life.rs).
    idle_life: IdleState,
    /// A guest's copy of the host's activity (kind, progress); shown, never run.
    net_activity: Option<(Activity, f32)>,
    /// A guest's copy of the host's nap spot (see `nap_spot`).
    net_nap: Option<activities::NapSpot>,
    /// A chaser's shortcut through the ducts (hunters/vent_nav.rs). Read the crawl height with `vent_trip.crawl.height()`.
    pub vent_trip: super::vent_nav::Trip,
    /// The entry grate a planned trip walks to instead of the ordinary goal, this frame.
    vent_walk: Option<(f32, f32)>,
    /// Seconds until the next trip plan is tried.
    vent_timer: f32,
    /// Sheet-metal thumps made crawling since the last `take_vent_thumps`.
    vent_thumps: u32,
    /// The hatch that opened since the last `take_vent_hatch`.
    vent_hatch: Option<super::vent_nav::Hatch>,
    /// Own random stream for goals and activities, so the main stream (and what its tests pin) is untouched.
    act_rng: Rng,
    /// The locker (index into `Facility::hiding`) the hunter leaned into this frame. The simulating side's
    /// game/hunting.rs reads it with `take_locker_check` to resolve whoever hides there.
    locker_check: Option<usize>,
    /// The locker that held this hunter's own target when it leaned in (the target is found and chased).
    locker_found: Option<usize>,
    /// A shut door the hunter knocked on or pushed this frame, for `take_door_action`.
    door_action: Option<(Cell, Activity)>,
    /// A sniff began this tick, for `take_sniff_cue` (the simulating side plays the cue).
    sniff_cue: bool,
    /// Route progress for the stuck escalation (centre-steer, then repath with a snap-back; hunters/navigation.rs).
    stuck: super::navigation::StuckTracker,
    /// Hard abilities (pounce leap, sprint burst, intercept, door slam; hunters/hard_abilities.rs). None when this hunter
    /// has none on its difficulty. Host only: a guest never ticks it.
    pub hard: Option<super::hard_abilities::HardAbilities>,
    /// The party budget and per-runner pounce guard. Each brain starts with its own; the host gives every hunter of a
    /// floor the same one (`Arc::clone`) so the budget is shared the way Godot shares it.
    pub hard_budget: Arc<std::sync::Mutex<super::hard_abilities::Budget>>,
    /// Seconds this brain has run, the clock the budget compares.
    pub hard_clock: f32,
    /// Own random stream for the ability rolls, so the main stream (and what its tests pin) is untouched.
    hard_rng: Rng,
}

const ARRIVAL_DISTANCE: f32 = 0.65;
/// The centre-to-centre distance two bodies keep: a hunter is never closer to its target than this.
const BODY_GAP: f32 = HUNTER_RADIUS + PLAYER_RADIUS;
/// data/hunter_walk.json: how close counts as at a waypoint, how many route cells it may cut ahead, the sweep sample step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WalkRules {
    pub waypoint_distance: f32,
    pub path_lookahead_nodes: usize,
    pub sweep_step: f32,
}

impl WalkRules {
    /// The Godot values (hunter_brain.gd lines 31-34).
    pub const GODOT: WalkRules = WalkRules { waypoint_distance: 0.18, path_lookahead_nodes: 4, sweep_step: 0.15 };

    /// Reads the three keys; a missing or out-of-range key keeps the Godot value.
    pub fn parse(text: &str) -> WalkRules {
        let mut r = WalkRules::GODOT;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(x) = v.get("waypoint_distance").and_then(|x| x.as_f64()).filter(|x| *x > 0.0 && *x < 5.0) {
                r.waypoint_distance = x as f32;
            }
            if let Some(x) = v.get("path_lookahead_nodes").and_then(|x| x.as_u64()).filter(|x| *x >= 1 && *x <= 64) {
                r.path_lookahead_nodes = x as usize;
            }
            if let Some(x) = v.get("sweep_step").and_then(|x| x.as_f64()).filter(|x| *x >= 0.01 && *x < 5.0) {
                r.sweep_step = x as f32;
            }
        }
        r
    }
}

pub(crate) fn walk_rules() -> &'static WalkRules {
    // The pursuit block of data/hunters/nav_tuning.json (hunters/navigation.rs); hunter_walk.json stays for WalkRules::parse.
    static RULES: OnceLock<WalkRules> = OnceLock::new();
    RULES.get_or_init(|| {
        let p = super::navigation::tuning().pursuit;
        WalkRules { waypoint_distance: p.waypoint_distance, path_lookahead_nodes: p.path_lookahead_nodes, sweep_step: p.sweep_step }
    })
}
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
    pub fn path_cells(&self) -> &[Cell] {
        &self.path
    }

    pub fn path_goal_cell(&self) -> Option<Cell> {
        self.path_goal
    }

    pub fn new(def: HunterDef, spawn: (f32, f32), seed: &str, diff: Difficulty) -> Brain {
        let gimmick = Gimmick::for_id(&def.id, diff);
        let def_id = def.id.clone();
        let hard = super::hard_abilities::HardAbilities::new(diff, &def_id);
        let hard_rng = Rng::new(&format!("{seed}:hard"));
        // a real Idle pause on spawn (data/hunters/activities.json spawn_pause), not a wander on the first frame
        let mut act_rng = Rng::new(&format!("{seed}:activity"));
        let spawn_pause = activities::Rules::get().pause_for(true, act_rng.next_float());
        let mut land_rng = Rng::new(&format!("{seed}:land"));
        let land_angle = land_rng.next_float() as f32 * std::f32::consts::TAU;
        let land_mag = flight_edges::rules().land_spread.min(0.5 - HUNTER_RADIUS - 0.02) * (0.5 + 0.5 * land_rng.next_float() as f32);
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
            idle_timer: spawn_pause,
            search_timer: 0.0,
            search_visits: 0,
            lost_sight: 0.0,
            sense_timer: 0.0,
            wander_target: None,
            patrol_anchor: None,
            patrol: None,
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
            no_hop_memo: None,
            hop_key: None,
            hop_age: 0.0,
            wander_level: None,
            cross_goal: None,
            follow_cross: None,
            snap_count: 0,
            giveup_count: 0,
            edge_hold: 0.0,
            land_offset: (land_angle.cos() * land_mag, land_angle.sin() * land_mag),
            land_check: false,
            edge_clock: 0.0,
            edge_retreat: false,
            poi: None,
            recent_pois: Vec::new(),
            activity: None,
            idle_life: IdleState::default(),
            net_activity: None,
            net_nap: None,
            vent_trip: Default::default(),
            vent_walk: None,
            vent_timer: 0.0,
            vent_thumps: 0,
            vent_hatch: None,
            act_rng,
            locker_check: None,
            locker_found: None,
            door_action: None,
            sniff_cue: false,
            stuck: Default::default(),
            hard,
            hard_budget: Default::default(),
            hard_clock: 0.0,
            hard_rng,
        }
    }

    /// Stand on `level`'s floor (a spawn, a respawn or a list swap): level and y together, and no edge in progress.
    pub fn set_level(&mut self, level: i32) {
        self.level = level;
        self.y = crate::game::level_stack::floor_y_of(level);
        self.walk = None;
        self.hop = None;
        self.hop_key = None;
        self.wander_level = None;
        self.cross_goal = None;
        self.follow_cross = None;
        self.edge_clock = 0.0;
        self.edge_retreat = false;
        self.last_known_level = level;
        self.last_known_stamp = None;
    }

    /// Ends an edge walk in progress by standing the hunter on a real end of that edge: the end on `level` (the level
    /// of the list it is kept under) when the edge has one, else the end nearer in height. For a hunter that leaves
    /// the simulation in the middle of a flight (its level is shelved or stored): it is never kept in mid-air, and it
    /// comes back standing on a floor. Nothing changes for a hunter that is not on an edge.
    pub fn settle_edge(&mut self, level: i32) {
        let Some(w) = self.walk.take() else { return };
        let (start, end) = (w.start_point(), w.end_point());
        let at_start = if w.from.level == level {
            true
        } else if w.to.level == level {
            false
        } else {
            (self.y - start[2]).abs() <= (end[2] - self.y).abs()
        };
        let (node, y) = if at_start { (w.from, start[2]) } else { (w.to, end[2]) };
        self.pos = node.cell.center();
        self.y = y;
        self.level = node.level;
        self.hop = None;
        self.hop_key = None;
        self.edge_clock = 0.0;
        self.edge_retreat = false;
        self.moving_speed = 0.0;
        self.clear_path();
    }

    /// The level this hunter is on its way to: the far end of the edge it walks, else the level of the crossing it
    /// follows the runner over. None for a hunter that stays on its floor.
    pub fn bound_for(&self) -> Option<i32> {
        if let Some(w) = self.walk.as_ref() {
            return Some(w.to.level);
        }
        self.follow_cross.map(|f| f.2)
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
            // a wander or search goal picked on the far end of a stair (pick_cross_goal)
            State::Wander if self.cross_goal.is_some() && self.wander_target == self.cross_goal => {
                self.wander_level.unwrap_or(self.level)
            }
            State::Search if self.cross_goal.is_some() && self.search_target == self.cross_goal => {
                self.wander_level.unwrap_or(self.level)
            }
            _ => self.level,
        }
    }

    /// Pushes the levels this brain needs built into `out`: both ends of the edge it walks, its goal level when the goal is
    /// on another floor, and the two floors of the edge it heads for. Allocates nothing itself; the caller owns `out`.
    pub fn pin_levels(&self, out: &mut Vec<i32>) {
        if let Some(w) = self.walk.as_ref() {
            out.push(w.from.level);
            out.push(w.to.level);
        }
        let g = self.goal_level();
        if g != self.level {
            out.push(g);
            if let Some(h) = self.hop.as_ref() {
                out.push(h.edge.upper_level);
                out.push(h.edge.upper_level - 1);
            }
        }
    }

    /// The runner left this hunter's level for `to` and stands at `runner`: a chaser or investigator (not a stay-at-home
    /// creature) goes after it by the flight, and keeps that goal for `follow_cross_seconds` so a quick give-up does not
    /// drop it. `entrance` is the near end of the flight or ladder the runner walked (None when it took none): an awake
    /// hunter that was not after the runner comes too when it stands within `follow_cross_cells` of it, unless its
    /// gimmick holds it in place or it is in the middle of a peek. A sleeping hunter never follows. Host only.
    pub fn follow_cross(&mut self, runner: (f32, f32), to: i32, entrance: Option<(f32, f32)>) {
        if !self.active || self.display_only || self.shadow || to == self.level {
            return;
        }
        if super::secret_creatures::stays_home(&self.def.id) {
            return;
        }
        if !matches!(self.state, State::Chase | State::Investigate) {
            let reach = self.nav.rules().follow_cross_cells;
            let near = entrance.is_some_and(|e| dist(self.pos, e) <= reach);
            if !near || self.state == State::Peek || self.gimmick.holds_position(self) {
                return;
            }
            // whatever it was doing at a point of interest ends: it goes to look where the runner went
            self.end_activity();
        }
        let p = self.nav.open_goal(to, cell_of(runner)).map_or(runner, |c| c.center());
        self.investigate_on(p, to);
        self.follow_cross = Some((self.nav.rules().follow_cross_seconds, p, to));
    }

    /// Keeps a followed crossing alive: counts the time down, and re-sets the goal if the chase dropped it.
    fn tick_follow(&mut self, dt: f32) {
        let Some(f) = self.follow_cross.as_mut() else { return };
        f.0 -= dt;
        let (left, p, lvl) = *f;
        if left <= 0.0 || self.level == lvl {
            self.follow_cross = None;
            return;
        }
        if matches!(self.state, State::Chase | State::Investigate) && self.last_known.is_none() && self.walk.is_none() {
            self.investigate_on(p, lvl);
        }
    }

    /// The window moved by `d` cells: the cross-level goal and the followed crossing move with it, and the cached hop (planned
    /// in the old frame) is dropped. The walk on an edge is shifted by the caller (`EdgeWalk::shift`).
    pub fn shift_cross_goal(&mut self, d: (f32, f32)) {
        let s = move |p: (f32, f32)| (p.0 + d.0, p.1 + d.1);
        self.cross_goal = self.cross_goal.map(s);
        if let Some(f) = self.follow_cross.as_mut() {
            f.1 = s(f.1);
        }
        self.hop = None;
        self.hop_key = None;
        self.no_hop_memo = None;
    }

    /// A facility alert from `level`: investigate `at`, which is on that level.
    pub fn alert_on(&mut self, at: (f32, f32), level: i32) {
        self.state = State::Investigate;
        self.set_known_level(at, level);
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
            // a stun ends a hard leap where it is: it never resumes along the old line once the hunter is free
            if let Some(h) = self.hard.as_mut() {
                h.interrupt_leap();
            }
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
        // a hard-ability leap (hunters/hard_abilities.rs) shows the pounce pose; the frame carries it to the guests
        if self.hard.as_ref().is_some_and(|h| h.leaping) {
            return Some("pounce");
        }
        self.cue().or_else(|| self.gimmick.mechanic(self)).or_else(|| self.net_mech.map(|m| m.0))
    }

    /// A wandering or idle hunter heard something: maybe turn its head to it before investigating (data chance and
    /// time, by personality). Host side only; never during a walk edge, a chase, a search or a peek.
    fn begin_head_turn(&mut self, at: (f32, f32)) -> bool {
        if !matches!(self.state, State::Wander | State::Idle) || self.walk.is_some() || self.display_only || self.shadow {
            return false;
        }
        let pers = activities::Rules::get().personality_of(&self.def.id, self.def.tier);
        let (a, b) = (self.act_rng.next_float(), self.act_rng.next_float());
        if !self.idle_life.begin_turn(IdleRules::get(), pers, a, b) {
            return false;
        }
        self.end_activity();
        self.poi = None;
        self.wander_target = None;
        self.clear_path();
        self.path_goal = None;
        self.facing = activities::dir_to(self.pos, at, self.facing);
        self.state = State::Idle;
        true
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
        self.verify_landing(ctx.fac);
        self.locker_check = None;
        self.door_action = None;
        self.sniff_cue = false;
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
        // a hunter asleep in a bed (Activity::Nap, hunters/naps.rs): no perception, no movement, no catch; only the nap
        // clock and the wake rule. A stun wakes it as well. Anything that moved it off its wander (an alert, a chase, a
        // noise it goes to investigate) ends the nap first, so the hunter carries on with the state it was given.
        if self.napping() && self.state != State::Wander {
            self.end_activity();
            self.poi = None;
        }
        if self.napping() {
            if self.stun > 0.0 || naps::wakes_for(activities::Rules::get(), self.pos, self.level, target) {
                self.wake_from_nap(target);
            } else {
                self.tick_activity(dt, ctx.fac, 0.0, prof);
            }
            // the gimmick's after-hook and the stamina still run while it sleeps (Briselle counts the relays it sees)
            g.after_perception(self, dt, ctx, target, &mut events);
            self.update_stamina(dt, prof);
            self.gimmick = g;
            return (false, events);
        }
        if self.stun > 0.0 {
            self.stun = (self.stun - dt).max(0.0);
            self.gimmick = g;
            return (false, events);
        }
        self.tick_follow(dt);
        let ignoring = self.camp_guard(dt, target, prof);
        self.ability_direct_sight = false;
        if !self_summoning {
            g.before_perception(self, dt, ctx, target, &mut events);
        }
        self.update_beam_attention(dt, ctx.fac, target);
        // Beam attention is updated after gimmick perception. Nyxie must be
        // removed in this same frame, before movement/capture can continue.
        if self.flash_triggered {
            if let Gimmick::Nyxie(n) = &mut g {
                n.dispel_by_flash();
                self.active = false;
                self.state = State::Idle;
                self.gimmick = g;
                return (false, events);
            }
        }
        if !ignoring {
            self.perceive(dt, ctx.fac, target, prof, &g);
        }
        g.after_perception(self, dt, ctx, target, &mut events);
        self.update_stamina(dt, prof);
        let holds = g.holds_position(self);
        // a chaser may take the ducts when they beat the floor; inside them it is not walked and cannot be grabbed
        let holds = self.vent_frame(dt, ctx, holds) || holds;
        self.peek_cooldown = (self.peek_cooldown - dt).max(0.0);
        if self.walk.is_none() {
            self.consider_peek(ctx.fac, &g, holds);
        }
        let speed_scale = g.speed_scale(self);
        let leaping = self.hard_frame(dt, ctx.fac, target, &g, holds);
        // state-leave cleanup runs every frame, before the leap guard: a pounce skips update_movement, so without
        // this a Listen hearing boost (or a stale poi/head turn) would linger across the leap and perturb perception
        self.leave_state_cleanup();
        if !leaping {
            self.update_movement(dt, ctx.fac, depth, relays, prof, holds, speed_scale);
        }
        self.resolve_locker_check(target);
        self.follow_terrain(ctx.fac);
        self.tick_emotes(dt, state_before, dist(self.pos, target.pos));
        let breaches = g.breaches_hiding();
        let immune = g.immune_to_capture(self) || self.vent_trip.inside();
        self.gimmick = g;
        let may_grab_hidden = target.hidden
            && breaches
            && self
                .last_known
                .map(|k| dist(k, target.pos) < 0.9)
                .unwrap_or(false);
        let gap = dist(self.pos, target.pos);
        let caught = (!target.hidden || may_grab_hidden)
            && !target.in_safe_zone
            && !immune
            && gap < self.capture_reach(prof)
            && (self.y - target.y).abs() <= flight_edges::rules().capture_height
            && (target.cross.is_some() || gap < BODY_GAP + flight_edges::rules().capture_los_slack || self.grab_line_clear(ctx.fac, target.pos));
        if !caught {
            self.keep_body_gap(ctx.fac, target);
        }
        (caught, events)
    }

    /// Whether nothing solid lies between this hunter and `to`: no wall (`has_line_of_sight`), and no shut door, railing
    /// or prop either (a thin body swept along the line). A hiding spot or prop the target stands in does not count when
    /// the sweep ends within reach of it.
    fn grab_line_clear(&self, fac: &Facility, to: (f32, f32)) -> bool {
        if !fac.has_line_of_sight(self.pos, to) {
            return false;
        }
        let reached = fac.clear_toward(self.pos, to, 0.05);
        dist(reached, to) <= 0.5
    }

    /// Two bodies never share a spot: a hunter that ends the frame nearer its target than their radii (and not caught: a
    /// hidden, safe or immune target) is pushed back along the line, as far as its body fits. Not on an edge, in a duct,
    /// or against a target hiding in a locker (the locker check stands right at it).
    fn keep_body_gap(&mut self, fac: &Facility, target: &Target) {
        if self.walk.is_some() || self.vent_trip.inside() || target.hidden || self.display_only || self.shadow {
            return;
        }
        if (self.y - target.y).abs() > flight_edges::rules().capture_height {
            return;
        }
        let gap = dist(self.pos, target.pos);
        if gap >= BODY_GAP {
            return;
        }
        let dir = if gap > 1e-3 {
            ((self.pos.0 - target.pos.0) / gap, (self.pos.1 - target.pos.1) / gap)
        } else {
            (-self.facing.0, -self.facing.1)
        };
        let want = (target.pos.0 + dir.0 * BODY_GAP, target.pos.1 + dir.1 * BODY_GAP);
        self.pos = fac.clear_toward(self.pos, want, HUNTER_RADIUS);
        self.clear_path();
    }

    /// One frame of a chaser's vent shortcut (hunters/vent_nav.rs). Plans a trip when the ducts are much shorter than the
    /// floor, walks to the entry grate (`vent_walk`), crawls and climbs out. Returns true while the body is in the ducts
    /// (the brain must not walk it). Never for Verrine (she lives there), never across floors.
    fn vent_frame(&mut self, dt: f32, ctx: &Ctx, holds: bool) -> bool {
        self.vent_walk = None;
        let inside = self.vent_trip.inside();
        let chasing = matches!(self.state, State::Chase | State::Investigate)
            && self.def.id != "verrine"
            && ctx.vents.len() >= 2
            && !holds
            && self.walk.is_none()
            && self.level == self.goal_level();
        if !chasing && !inside {
            self.vent_trip.cancel();
            return false;
        }
        // a stunned or held hunter inside the ducts stays where it is: no crawl step and no thumps until it is free
        if holds && inside {
            self.pos = self.vent_trip.crawl.pos;
            return true;
        }
        let goal = self.last_known.unwrap_or(self.pos);
        if !self.vent_trip.planned() && !inside {
            self.vent_timer -= dt;
            if self.vent_timer > 0.0 {
                return false;
            }
            self.vent_timer = 1.0;
            let floor_len = if self.path.is_empty() { dist(self.pos, goal) } else { self.path.len() as f32 };
            self.vent_trip.plan(ctx.vents, self.pos, goal, floor_len, &self.def.id, ctx.diff.id());
            if !self.vent_trip.planned() {
                return false;
            }
        }
        let fac = ctx.fac;
        let s = self.vent_trip.step(dt, self.pos, goal, &|g, _goal| fac.nearest_open_or(g.0, g.1, 2.0));
        self.vent_walk = s.walk_to;
        self.vent_thumps += s.thumps;
        if let Some(h) = s.hatch {
            self.vent_hatch = Some(h);
        }
        if self.vent_trip.inside() || s.done {
            self.pos = self.vent_trip.crawl.pos;
            self.clear_path();
            self.stuck.reset();
            self.path_timer = 0.0;
        }
        self.vent_trip.inside()
    }

    /// The thumps made crawling in the ducts since the last call (the host raises a noise for each).
    pub fn take_vent_thumps(&mut self) -> u32 {
        std::mem::take(&mut self.vent_thumps)
    }

    /// The hatch that opened since the last call (a sound cue).
    pub fn take_vent_hatch(&mut self) -> Option<super::vent_nav::Hatch> {
        self.vent_hatch.take()
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
                    hears = self.hears_cross(fac, target.pos, target.level);
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
            } else if (self.state == State::Idle && self.idle_life.turning()) || self.begin_head_turn(target.pos) {
                // turning to the noise first; the Idle branch of update_movement sends it to investigate
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
            // data/progression.json chase_speed_scale: one knob per difficulty over every hunter's own `chase` value. The
            // stride animation and the footfalls follow the measured ground speed, so they speed up with it.
            (self.def.chase + prog * self.def.prog) * rules.chase_scale_for(self.diff, &self.def.category)
        } else {
            let base = self.def.speed + prog;
            match self.patrol {
                Some(p) if self.state == State::Wander => crate::app::public_route::patrol_speed(base, p, self.patrol_anchor.is_some()),
                _ => base,
            }
        };
        if self.winded {
            speed *= HUNTER_WINDED_SPEED_SCALE;
        }
        speed *= gimmick_scale;
        if self.slow_timer > 0.0 {
            speed *= self.slow_factor;
        }
        // the hard-ability speed model (burst, stumble, per-hunter scale) and the per-level danger (data/danger.json)
        speed *= super::hard_abilities::speed_multiplier(&self.def.id, self.hard.as_ref());
        speed *= self.diff.danger(depth).speed_scale;
        let c = cell_of(self.pos);
        speed * prof.hunter_speed * self.water_rules.speed_scale(&self.water, (c.x, c.y))
    }

    /// Drop whatever a non-Wander/Idle state has no business keeping: a goal's activity (and its hearing boost,
    /// undone by `end_activity`) and its `poi` belong to the wander they were picked in, so chase, investigate,
    /// search or a peek ends them; a head turn lives only in an Idle pause, so any other state cancels it (a stale
    /// turn must never make the hunter deaf to the next noise). Runs every frame in `update`, before the leap guard,
    /// because a leaping hunter skips `update_movement` and a stale activity/hearing boost would otherwise linger
    /// across the pounce and perturb perception until the leap ends.
    fn leave_state_cleanup(&mut self) {
        if self.state != State::Wander && (self.activity.is_some() || self.poi.is_some()) {
            self.end_activity();
            self.poi = None;
        }
        if self.state != State::Idle {
            self.idle_life.cancel();
        }
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
                if self.idle_life.turning() {
                    // the head turn toward a heard noise, then go and look
                    if self.idle_life.tick_turn(dt) {
                        self.state = State::Investigate;
                        self.clear_path();
                    }
                    return;
                }
                let rng = &mut self.act_rng;
                if let Some(f) = self.idle_life.glance(dt, IdleRules::get(), self.facing, || rng.next_float()) {
                    self.facing = f;
                }
                self.idle_timer -= dt;
                if self.idle_timer <= 0.0 {
                    // nothing to chase: walk the floor instead of standing still
                    self.state = State::Wander;
                    self.wander_target = Some(self.pick_wander_target(fac));
                }
                return;
            }
            State::Search => {
                self.search_timer -= dt;
                if self.search_timer <= 0.0 {
                    // search ran out with no target: keep moving (Wander), do not park in Idle
                    self.state = State::Wander;
                    self.last_known = None;
                    self.wander_target = Some(self.pick_wander_target(fac));
                    self.clear_path();
                    return;
                }
            }
            _ => {}
        }
        if self.state == State::Wander && self.activity.is_some() {
            if !holds {
                let speed = self.current_speed(depth, relays, prof, gscale);
                self.tick_activity(dt, fac, speed, prof);
            }
            return;
        }
        if holds {
            return;
        }
        if self.state == State::Wander && self.wander_target.is_none() {
            // back from a peek (or set to Wander with no goal): pick one rather than stand
            let t = self.pick_wander_target(fac);
            if self.state == State::Wander {
                self.wander_target = Some(t);
                self.clear_path();
            }
            return;
        }
        let goal = match self.state {
            State::Wander => self.wander_target,
            State::Chase | State::Investigate => self.vent_walk.or(self.last_known),
            State::Search => self.search_target,
            State::Idle | State::Peek => None,
        };
        let Some(goal) = goal else { return };
        let speed = self.current_speed(depth, relays, prof, gscale);
        self.pursue(goal, speed, dt, fac, prof);
        if self.vent_walk.is_none() && self.walk.is_none() && self.level == self.goal_level() && dist(self.pos, goal) <= ARRIVAL_DISTANCE {
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
        // a goal standing on a closed cell (a stair tread, a well edge) is planned to the open cell beside it
        let goal_cell = self.nav.open_goal(goal_level, cell_of(goal))?;
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
            Node { level: goal_level, cell: goal_cell },
            &step_cost,
        )
    }

    /// Whether a flight leads to a target on `level` (the hearing check of a cross-floor sound), through the cached hop
    /// and the no-hop memo so a sensed target is not searched for again every sense tick.
    fn hears_cross(&mut self, fac: &Facility, goal: (f32, f32), level: i32) -> bool {
        let key = (self.nav.version(), cell_of(goal), level, self.level);
        if self.hop_key == Some(key) && self.hop.is_some() {
            return true;
        }
        if self.no_hop_memo.is_some_and(|m| (m.0, m.1, m.2, m.3) == key && m.4 > 0.0) {
            return false;
        }
        match self.hop_to(fac, goal, level) {
            Some(h) => {
                if self.walk.is_none() {
                    self.hop = Some(h);
                    self.hop_key = Some(key);
                    self.hop_age = 0.0;
                }
                self.no_hop_memo = None;
                true
            }
            None => {
                self.no_hop_memo = Some((key.0, key.1, key.2, key.3, self.give_up_after));
                false
            }
        }
    }

    /// Keeps `self.hop` current for a goal on another level without a search per frame. A replan happens when the nav
    /// graph, the goal level or the own level changed, when there is no hop and no fresh no-hop memo, and every
    /// `cross_repath_seconds` when the goal cell moved or there is no hop. A hop already committed to is kept otherwise.
    fn refresh_hop(&mut self, fac: &Facility, goal: (f32, f32), goal_level: i32) {
        let key = (self.nav.version(), cell_of(goal), goal_level, self.level);
        let repath = self.nav.rules().cross_repath_seconds;
        let memo_ok = self.no_hop_memo.is_some_and(|m| (m.0, m.1, m.2, m.3) == key && m.4 > 0.0);
        let (immediate, same) = match self.hop_key {
            Some(k) => (k.0 != key.0 || k.2 != key.2 || k.3 != key.3, k == key),
            None => (true, false),
        };
        let due = immediate || (self.hop_age >= repath && (self.hop.is_none() || !same)) || (self.hop.is_none() && !memo_ok);
        if !due {
            return;
        }
        let old = self.hop.as_ref().map(|h| (h.edge, h.entrance));
        self.hop = if memo_ok { None } else { self.hop_to(fac, goal, goal_level) };
        if self.hop.is_some() {
            self.no_hop_memo = None;
        } else if !memo_ok {
            self.no_hop_memo = Some((key.0, key.1, key.2, key.3, self.give_up_after));
        }
        self.hop_key = Some(key);
        self.hop_age = 0.0;
        if self.hop.as_ref().map(|h| (h.edge, h.entrance)) != old {
            // a new entrance: the route to it is planned this frame
            self.path_timer = 0.0;
            self.clear_path();
        }
    }

    /// A landing spot off the cell centre must be ground a hunter fits (a prop, a door or a divider can sit in an open
    /// cell); otherwise the hunter stands at the cell centre. Runs once, on the first update on the landing level.
    fn verify_landing(&mut self, fac: &Facility) {
        if !self.land_check {
            return;
        }
        self.land_check = false;
        if !fac.hunter_fits(self.pos.0, self.pos.1) {
            self.pos = cell_of(self.pos).center();
        }
    }

    /// One frame on an edge: the walk alone moves the body (at the hunter's own speed, no climb animation) and the level
    /// follows the height. The body is never set anywhere it did not walk to. A step the footprint test refuses is not
    /// taken; after `edge_stuck_seconds` of refusal in a row the walk is turned round (`EdgeWalk::reversed`) and the
    /// hunter walks back to the end it came from, where it gives its goal up. A walker the director slows or stops
    /// behind another is not refused and is never timed out. Only a walker that is refused on the way back as well (the
    /// facility under it no longer has the flight: its level was unloaded) is put on the nearer end, counted in
    /// `snap_count`.
    fn walk_edge(&mut self, dt: f32, fac: &Facility, speed: f32, prof: &Profile) {
        let speed = speed * (1.0 - self.edge_hold.clamp(0.0, 1.0));
        self.edge_hold = 0.0;
        let Some(walk) = self.walk.as_ref() else { return };
        let (from, to) = (walk.from, walk.to);
        let (start_y, end_y) = (walk.start_point()[2], walk.end_point()[2]);
        // The footprint test is for a flight's treads. A ladder shaft and a drop's fall are vertical paths built by
        // the graph (there is no walk surface between the floors to test), so only a flight is checked.
        let flight = walk.kind == flight_edges::EdgeKind::Flight;
        let mut next = walk.clone();
        let pose = next.advance(speed, dt, self.nav.rules());
        let (done, x, z, y) = (pose.done, pose.x, pose.z, pose.y);
        let prev = self.pos;
        if done {
            // set down by the end of the route, a small fixed offset off it so a queue of followers does not stack on one
            // point; the offset keeps the body inside the landing cell (open by construction), else the cell centre
            let end = next.end_point();
            let spot = (end[0] + self.land_offset.0, end[1] + self.land_offset.1);
            self.pos = if cell_of(spot) == to.cell && self.nav.grid(to.level).is_some_and(|g| g.is_open(to.cell)) {
                self.land_check = true;
                spot
            } else {
                to.cell.center()
            };
            self.stuck.reset();
            // the walk's own end height: the floor of the level it ends on
            self.y = next.end_point()[2];
            self.level = to.level;
            self.walk = None;
            self.hop = None;
            self.hop_key = None;
            self.edge_clock = 0.0;
            self.clear_path();
            self.path_timer = 0.0;
            self.moving_speed = speed;
            self.stride += dist(prev, self.pos).max(speed * dt);
            if self.edge_retreat {
                // back where it started after a refused crossing: the goal behind that flight is given up
                self.edge_retreat = false;
                self.give_up_goal(fac, prof);
            }
            return;
        }
        if !flight || super::navigation::on_flight(fac, x, z, y) {
            self.pos = (x, z);
            self.y = y;
            self.level = crate::game::level_stack::level_at(self.y, self.level);
            self.walk = Some(next);
            self.edge_clock = 0.0;
            let d = dist(prev, self.pos);
            if d > 1e-4 {
                self.facing = ((self.pos.0 - prev.0) / d, (self.pos.1 - prev.1) / d);
            }
            // A ladder moves only in y, so the stride runs at the walk speed either way.
            self.stride += speed * dt;
            self.moving_speed = speed;
            return;
        }
        // the step ahead is refused: stand, and count how long that lasts
        self.moving_speed = 0.0;
        self.edge_clock += dt;
        if self.edge_clock <= self.nav.rules().edge_stuck_seconds {
            return;
        }
        self.edge_clock = 0.0;
        if !self.edge_retreat {
            // truly blocked: turn round where it stands and walk the same edge back
            self.edge_retreat = true;
            self.walk = self.walk.take().map(|w| w.reversed());
            return;
        }
        // refused on the way back as well: the edge is gone under the hunter, so it stands on the end it is nearer to
        let end = if (self.y - start_y).abs() <= (end_y - self.y).abs() { from } else { to };
        self.pos = end.cell.center();
        self.y = if end == from { start_y } else { end_y };
        self.level = end.level;
        self.walk = None;
        self.hop = None;
        self.hop_key = None;
        self.edge_retreat = false;
        self.snap_count += 1;
        self.give_up_goal(fac, prof);
    }

    fn handle_arrival(&mut self, fac: &Facility, prof: &Profile) {
        match self.state {
            State::Wander => {
                // a goal picked from a point of interest ends in an activity, then an Idle pause
                let at_poi = match (&self.poi, self.wander_target) {
                    (Some(p), Some(g)) => dist(p.stand, g) < 0.01,
                    _ => false,
                };
                if at_poi {
                    self.begin_activity(fac);
                    return;
                }
                // any other goal: chain another wander immediately
                self.wander_target = Some(self.pick_wander_target(fac));
                self.clear_path();
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
        self.no_hop_memo = None;
        self.hop = None;
        self.hop_key = None;
        self.giveup_count += 1;
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
                self.end_activity();
                self.wander_target = Some(self.pick_wander_target(fac));
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
        self.hop_age += dt;
        if let Some(m) = self.no_hop_memo.as_mut() {
            m.4 -= dt;
        }
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
            // stop where the bodies touch: the chase never steps onto the target's own spot
            let reach = (dist(self.pos, goal) - BODY_GAP).max(0.0);
            self.step_toward(goal, (speed * HUNTER_VISIBLE_CHASE_SPEED_SCALE * dt).min(reach), fac, dt);
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
        if cross {
            // the hop is cached: a search only when the graph, the goal or the level changed, or on the data timer
            self.refresh_hop(fac, goal, goal_level);
            match self.hop.as_ref() {
                Some(h) => target_cell = h.entrance,
                None => no_hop = true,
            }
        }
        if self.path_timer <= 0.0 || self.path.is_empty() || self.path_goal.is_none() {
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
            self.edge_retreat = false;
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
                // unreachable wander point: pick another instead of standing Idle
                self.wander_target = Some(self.pick_wander_target(fac));
                self.clear_path();
                self.path_goal = None;
            }
            return;
        }
        self.unreachable = 0.0;
        while let Some(first) = self.path.first() {
            if dist(self.pos, first.center()) <= walk_rules().waypoint_distance {
                self.path.remove(0);
            } else {
                break;
            }
        }
        let on_path = !self.path.is_empty();
        let mut waypoint = if cross { target_cell.center() } else { goal };
        if on_path {
            // While centre-steering after getting stuck it looks only one cell ahead (no corner cutting).
            let n = super::navigation::lookahead_for(self.stuck.steering(), walk_rules().path_lookahead_nodes);
            let pts: Vec<(f32, f32)> = self.path.iter().take(n).map(|c| c.center()).collect();
            if let Some((furthest, wp)) = super::navigation::look_ahead(&pts, n, |_, c| {
                body_fits_gated(fac, &self.gate_floor, c.0, c.1) && self.clear_sweep(fac, self.pos, c)
            }) {
                waypoint = wp;
                for _ in 0..furthest {
                    if !self.path.is_empty() {
                        self.path.remove(0);
                    }
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
        let before = self.pos;
        self.step_toward(waypoint, travel, fac, dt);
        // No real progress: centre-steer first, then plan again and snap the body out of a wall (hunter_brain.gd _track_progress).
        let wanted = travel.min(dist(before, waypoint));
        if self.stuck.track(&super::navigation::tuning().stuck, dist(before, self.pos), wanted, dt)
            == super::navigation::StuckAction::Repath
        {
            self.clear_path();
            self.path_timer = 0.0;
            if let Some(p) = super::navigation::snap_back(
                self.pos,
                |q| fac.can_occupy(q.0 as f64, q.1 as f64, HUNTER_RADIUS as f64),
                // a cell the hunters' grid is open at, never a drop (a bare nearest_open can name a floor-less cell)
                |q| fac.nearest_dry_open(q.0, q.1, HUNTER_RADIUS).filter(|p| !fac.is_drop(cell_of(*p))),
            ) {
                self.pos = p;
            }
        }
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

    /// The next wander goal, from the facility's points of interest (hunters/activities.rs): weighted by personality, region
    /// and room type, never one of the last few. With none to go to (B11) it falls back to a reachable open cell nearby,
    /// then the exit when that is far, and otherwise stands in a real Idle pause (the state is set to Idle and the
    /// caller's goal is the hunter's own spot), so it never re-picks a zero-distance goal every frame.
    fn pick_wander_target(&mut self, fac: &Facility) -> (f32, f32) {
        self.pick_wander_target_with(fac, activities::Rules::get())
    }

    fn pick_wander_target_with(&mut self, fac: &Facility, rules: &activities::Rules) -> (f32, f32) {
        self.poi = None;
        // now and then the goal is the far end of a stair: the hunter walks the flight and wanders on the other floor
        let chance = self.nav.rules().wander_cross_chance;
        if let Some(t) = self.pick_cross_goal(chance) {
            return t;
        }
        let mut pois = activities::points_of_interest(fac, rules);
        pois.extend(idle_life::junction_pois(fac, rules, IdleRules::get(), self.pos));
        if let Some(p) = self.patrol {
            let stands: Vec<(f32, f32)> = pois.iter().map(|q| q.stand).collect();
            let kept = crate::app::public_route::filter_candidates(p, self.patrol_anchor, &stands);
            pois.retain(|q| kept.contains(&q.stand));
        } else if self.patrol_anchor.is_some() {
            // An anchor off the public route: the nav_tuning.json patrol radius (hunters/patrol_anchor.rs, Godot PATROL_RADIUS).
            let stands: Vec<(f32, f32)> = pois.iter().map(|q| q.stand).collect();
            let kept = super::patrol_anchor::candidates_near(&super::navigation::tuning().patrol, self.patrol_anchor, &stands);
            let mut i = 0;
            pois.retain(|_| {
                let keep = kept.contains(&i);
                i += 1;
                keep
            });
        }
        let personality = rules.personality_of(&self.def.id, self.def.tier).to_string();
        let roll = self.act_rng.next_float();
        if let Some(i) = rules.choose(&pois, self.pos, &self.recent_pois, &personality, roll) {
            let poi = pois.swap_remove(i);
            self.recent_pois.push(poi.id());
            let excess = self.recent_pois.len().saturating_sub(rules.memory);
            self.recent_pois.drain(..excess);
            let stand = poi.stand;
            self.poi = Some(poi);
            return stand;
        }
        if let Some(p) = self.local_wander_target(fac) {
            return p;
        }
        self.state = State::Idle;
        self.idle_timer = rules.pause_for(false, self.act_rng.next_float());
        self.pos
    }

    /// With probability `chance` (seeded, so a run replays) a goal on another level: the far end of a stair or ladder
    /// within the `cross_wander_radius` rule of the hunter. Sets `wander_level` and `cross_goal` (read by `goal_level`) and returns
    /// the point; None, with both cleared, for an ordinary goal. Never for a stay-at-home creature, a hunter held by an
    /// activity, a patrol, or one on an edge. Draws no random number while the nav graph has no neighbour level, so a
    /// single floor behaves exactly as before.
    fn pick_cross_goal(&mut self, chance: f32) -> Option<(f32, f32)> {
        self.wander_level = None;
        self.cross_goal = None;
        if chance <= 0.0 || self.walk.is_some() || self.activity.is_some() || self.patrol.is_some() || self.patrol_anchor.is_some() {
            return None;
        }
        let level = self.level;
        if !(self.nav.has_level(level - 1) || self.nav.has_level(level + 1)) || super::secret_creatures::stays_home(&self.def.id) {
            return None;
        }
        if self.act_rng.next_float() as f32 >= chance {
            return None;
        }
        let (x, z) = self.pos;
        let radius = self.nav.rules().cross_wander_radius;
        let n = self.nav.edges_near(level, x, z, radius).filter(|e| e.to.level != level).count();
        if n == 0 {
            return None;
        }
        let k = self.act_rng.next_index(n);
        let (to, to_level) = {
            let e = self.nav.edges_near(level, x, z, radius).filter(|e| e.to.level != level).nth(k)?;
            (e.to.cell.center(), e.to.level)
        };
        self.wander_level = Some(to_level);
        self.cross_goal = Some(to);
        Some(to)
    }

    /// A short wander when there is no point of interest: a reachable open cell 3 to 8 m away, else the exit when it
    /// is more than 5 m off. None when neither exists.
    fn local_wander_target(&mut self, fac: &Facility) -> Option<(f32, f32)> {
        for _ in 0..12 {
            let a = self.act_rng.next_float() as f32 * std::f32::consts::TAU;
            let d = 3.0 + self.act_rng.next_float() as f32 * 5.0;
            let (x, y) = (self.pos.0 + a.cos() * d, self.pos.1 + a.sin() * d);
            if let Some(c) = fac.nearest_open(x, y, HUNTER_RADIUS) {
                if dist(c, self.pos) >= 1.5 && self.can_reach(fac, c) {
                    return Some(c);
                }
            }
        }
        let exit = fac.exit.center();
        (dist(self.pos, exit) > 5.0).then_some(exit)
    }

    /// _pick_search_target: the next point to search. Hiding spots near the last sighting and the offsets around it
    /// come from data/hunters/search.json (per difficulty); every point is one a route actually leads to.
    pub fn pick_search_target(&mut self, fac: &Facility) -> (f32, f32) {
        let cross = self.nav.rules().search_cross_chance;
        if let Some(t) = self.pick_cross_goal(cross) {
            return t;
        }
        let rules = search_rules();
        let scale = rules.scale(self.diff);
        match self.pick_search_point(fac, rules, scale) {
            Some(p) => {
                self.searched.push(p);
                let excess = self.searched.len().saturating_sub(rules.memory);
                self.searched.drain(..excess);
                p
            }
            None => match self.nearest_reachable_search_point(fac) {
                Some(p) => {
                    self.searched.push(p);
                    p
                }
                None => {
                    // nothing reachable at all: end the search so the hunter walks on instead of standing in place
                    self.search_timer = 0.0;
                    self.pos
                }
            },
        }
    }

    /// B12: the last-known point (or the hunter's own spot) cannot be reached, so search from the nearest reachable
    /// cell: rings around it, near to far, the first open cell a route leads to.
    fn nearest_reachable_search_point(&mut self, fac: &Facility) -> Option<(f32, f32)> {
        let lk = self.last_known.unwrap_or(self.pos);
        for ring in 1..=6 {
            let r = ring as f32 * 1.5;
            let start = self.next_float() * std::f32::consts::TAU;
            for k in 0..8 {
                let a = start + k as f32 * std::f32::consts::TAU / 8.0;
                if let Some(c) = fac.nearest_open(lk.0 + a.cos() * r, lk.1 + a.sin() * r, HUNTER_RADIUS) {
                    if dist(c, self.pos) > 0.5 && self.can_reach(fac, c) {
                        return Some(c);
                    }
                }
            }
        }
        None
    }

    fn pick_search_point(&mut self, fac: &Facility, rules: &SearchRules, scale: SearchScale) -> Option<(f32, f32)> {
        // Search habits by difficulty (data/hunters/search_difficulty.json) on top of the search.json rates. A rate of
        // 0 takes no roll, so the main stream of a difficulty without habits is unchanged.
        let id = self.diff.id();
        let habits = super::navigation::search_difficulty().habits(id);
        let roll_a = if habits.locker_check_rate > 0.0 { self.next_float() } else { 1.0 };
        let roll_b = if habits.unchecked_room_rate > 0.0 { self.next_float() } else { 1.0 };
        let step = super::navigation::search_difficulty().next_step(id, self.searched.is_empty(), roll_a, roll_b);
        let habit_spot = matches!(step, super::navigation::SearchStep::LastSeen | super::navigation::SearchStep::Locker);
        let want_spot = if scale.last_seen_spot_first || habit_spot {
            true
        } else if scale.hiding_spot_chance > 0.0 {
            self.next_float() < scale.hiding_spot_chance
        } else {
            false
        };
        let unchecked_first = scale.unchecked_first || step == super::navigation::SearchStep::UncheckedRoom;
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
                    if !unchecked_first {
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
        self.grid_route(fac, cell_of(p), priority, false).map_or(true, |route| !route.is_empty())
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

impl Brain {
    /// The activity being done and its progress 0..1 (a guest shows the host's copy), for the renderer and the wire.
    pub fn activity_view(&self) -> Option<(Activity, f32)> {
        self.activity.as_ref().map(|a| (a.kind, a.progress())).or(self.net_activity)
    }

    /// A guest's copy of the host's activity (wire keys `act` and `actp`, both optional); None clears it.
    pub fn set_net_activity(&mut self, a: Option<(u8, f32)>) {
        self.net_activity = a.and_then(|(i, p)| Some((Activity::from_index(i)?, p.clamp(0.0, 1.0))));
    }

    /// The locker the hunter leaned into this frame (index into `Facility::hiding`); read once per frame on the side
    /// that simulates hunters.
    pub fn take_locker_check(&mut self) -> Option<usize> {
        self.locker_check.take()
    }

    /// The locker whose occupant this hunter found (its own target); read once.
    pub fn take_locker_found(&mut self) -> Option<usize> {
        self.locker_found.take()
    }

    /// A shut door the hunter knocked on or pushed this frame, with which of the two; read once.
    pub fn take_door_action(&mut self) -> Option<(Cell, Activity)> {
        self.door_action.take()
    }

    /// True once when a sniff has just begun; read once.
    pub fn take_sniff_cue(&mut self) -> bool {
        std::mem::take(&mut self.sniff_cue)
    }

    /// Whether an activity is running now.
    pub fn doing_activity(&self) -> bool {
        self.activity.is_some()
    }

    fn begin_activity(&mut self, fac: &Facility) {
        let Some(poi) = self.poi.take() else { return };
        let rules = activities::Rules::get();
        let door_shut = poi.door.is_some_and(|c| !fac.door_is_open(c));
        let pers = rules.personality_of(&self.def.id, self.def.tier);
        let kind = idle_life::pick_activity(rules, IdleRules::get(), &poi, door_shut, pers, self.act_rng.next_float());
        self.start_activity(kind, &poi, fac);
    }

    /// Start `kind` at `poi` now (the hunter stands at the goal).
    pub(crate) fn start_activity(&mut self, kind: Activity, poi: &Poi, fac: &Facility) {
        let rules = activities::Rules::get();
        self.end_activity();
        let base = activities::dir_to(self.pos, poi.focus, self.facing);
        let mut run = ActivityRun {
            kind,
            duration: rules.duration(kind, self.act_rng.next_float()),
            elapsed: 0.0,
            base,
            focus: poi.focus,
            locker: poi.locker,
            door: poi.door,
            fired: false,
            leg: 0,
            loop_points: vec![],
            hearing_applied: 1.0,
            axis: poi.axis,
        };
        match kind {
            Activity::CheckRoom => {
                if let Some(r) = poi.room.or_else(|| fac.room_at(self.pos.0 as f64, self.pos.1 as f64)) {
                    let start = (self.act_rng.next_float() * 4.0) as usize;
                    run.loop_points = activities::room_loop(fac, r, rules.check_room_inset, start);
                }
            }
            Activity::Listen => {
                self.hearing_scale *= rules.listen_hearing_scale;
                run.hearing_applied = rules.listen_hearing_scale;
            }
            Activity::Fidget | Activity::Pace | Activity::CornerPeek => {
                idle_life::setup(&mut run, fac, self.pos, IdleRules::get(), self.act_rng.next_float());
            }
            _ => {}
        }
        self.facing = base;
        self.wander_target = None;
        self.clear_path();
        self.path_goal = None;
        self.activity = Some(run);
    }

    /// Whether the hunter is asleep in a bed (Activity::Nap).
    pub fn napping(&self) -> bool {
        self.activity.as_ref().is_some_and(|a| a.kind == Activity::Nap)
    }

    /// Where a napping hunter lies: the bed's mattress centre and its long axis (for the lying pose). None when awake.
    /// A guest reads the spot the host sent with the frame (`set_net_nap`) while the wire says it naps.
    pub fn nap_spot(&self) -> Option<activities::NapSpot> {
        match self.activity.as_ref().filter(|a| a.kind == Activity::Nap) {
            Some(run) => Some(activities::NapSpot { at: run.focus, axis: run.axis }),
            None => self.net_nap.filter(|_| self.net_activity.is_some_and(|(k, _)| k == Activity::Nap)),
        }
    }

    /// A guest's copy of the host's nap spot (wire key `nap`, already moved into this window); None clears it.
    pub fn set_net_nap(&mut self, n: Option<activities::NapSpot>) {
        self.net_nap = n;
    }

    /// A startled wake from a nap: it flinches (a short stun) and investigates where the runner was (Godot
    /// `HunterNaps.wake`, the startled branch). The runner's level is the one it was seen on.
    fn wake_from_nap(&mut self, target: &Target) {
        self.end_activity();
        self.wander_target = None;
        self.poi = None;
        self.path_goal = None;
        self.investigate(target.pos);
        self.set_known_level(target.pos, target.level);
        self.apply_stun(activities::Rules::get().nap_startle_stun, false);
    }

    /// Drop the running activity (a chase, a give-up or a peek takes over) and undo what it changed.
    fn end_activity(&mut self) {
        if let Some(run) = self.activity.take() {
            self.undo_activity(&run);
        }
    }

    fn undo_activity(&mut self, run: &ActivityRun) {
        if run.hearing_applied != 1.0 {
            self.hearing_scale /= run.hearing_applied;
        }
    }

    /// One frame of the activity; at its duration the hunter pauses in Idle for a short random time.
    fn tick_activity(&mut self, dt: f32, fac: &Facility, speed: f32, prof: &Profile) {
        let rules = activities::Rules::get();
        let Some(mut run) = self.activity.take() else { return };
        run.elapsed += dt;
        let p = run.progress();
        match run.kind {
            Activity::InspectLocker => {
                self.facing = activities::dir_to(self.pos, run.focus, run.base);
                if !run.fired && p >= rules.lean_at {
                    run.fired = true;
                    self.locker_check = run.locker;
                }
            }
            Activity::Nap => {
                // asleep: it faces the mattress and does nothing else (the wake rule is in `update`)
                self.facing = run.base;
            }
            Activity::LookAround => {
                self.facing = activities::sweep_facing(run.base, p, rules.sweep_degrees, rules.sweeps);
            }
            Activity::Sniff => {
                if !run.fired {
                    run.fired = true;
                    self.sniff_cue = true;
                }
                let ahead = (self.pos.0 + run.base.0, self.pos.1 + run.base.1);
                self.step_toward(ahead, rules.sniff_creep_speed * dt, fac, dt);
                self.facing = run.base;
            }
            Activity::Listen => {
                self.facing = run.base;
            }
            Activity::CheckRoom => {
                if let Some(&goal) = run.loop_points.get(run.leg) {
                    if dist(self.pos, goal) <= ARRIVAL_DISTANCE {
                        run.leg += 1;
                    } else {
                        self.pursue(goal, speed * rules.check_room_speed_scale, dt, fac, prof);
                        if self.wander_target.is_some() || self.state != State::Wander {
                            // the walk gave up and picked a new goal: this activity is over
                            self.undo_activity(&run);
                            return;
                        }
                    }
                } else {
                    self.facing = activities::sweep_facing(run.base, p, rules.sweep_degrees, rules.sweeps);
                }
            }
            Activity::Fidget | Activity::Pace | Activity::CornerPeek => {
                let step = idle_life::tick(&mut run, p, self.pos, IdleRules::get());
                if let Some((to, speed)) = step.step_to {
                    self.step_toward(to, speed * dt, fac, dt);
                }
                if let Some(f) = step.facing {
                    self.facing = f;
                }
            }
            Activity::Knock | Activity::Push => {
                self.facing = activities::dir_to(self.pos, run.focus, run.base);
                let beat = if run.kind == Activity::Push { rules.push_at } else { rules.lean_at };
                if !run.fired && p >= beat {
                    run.fired = true;
                    if let Some(c) = run.door {
                        self.door_action = Some((c, run.kind));
                    }
                }
            }
        }
        if run.elapsed >= run.duration {
            self.undo_activity(&run);
            self.state = State::Idle;
            self.idle_timer = rules.pause_for(false, self.act_rng.next_float());
            self.wander_target = None;
            self.poi = None;
            self.clear_path();
            self.path_goal = None;
            return;
        }
        self.activity = Some(run);
    }

    /// One frame of the hard abilities (hunters/hard_abilities.rs, Godot hard_abilities.gd), after perception and before
    /// movement: timers and the runner's velocity, the burst and intercept on losing sight, the pounce roll while
    /// chasing, and the leap itself. Returns true while a leap moved the body (the ordinary walk is skipped that frame).
    /// Only the simulating side calls `update`, so a guest never runs this.
    fn hard_frame(&mut self, dt: f32, fac: &Facility, target: &Target, g: &Gimmick, holds: bool) -> bool {
        use super::hard_abilities::{rules, LeapEnd, PounceCtx};
        let Some(mut h) = self.hard.take() else { return false };
        self.hard_clock += dt;
        let now = self.hard_clock;
        let chasing = self.state == State::Chase;
        let shared = Arc::clone(&self.hard_budget);
        let mut budget = shared.lock().unwrap_or_else(|e| e.into_inner());
        h.tick(dt, self.direct_visible.then_some(target.pos));
        let rolls = [self.hard_rng.next_f32(), self.hard_rng.next_f32()];
        let lk = self.last_known.unwrap_or(target.pos);
        if let Some(lost) = h.update_sight(chasing, self.direct_visible, lk, rolls, &mut budget, now) {
            // the intercept: head for the open spot ahead of the runner's line instead of where they were last seen
            if let Some(p) = lost.intercept {
                if let Some(c) = fac.nearest_open(p.0, p.1, HUNTER_RADIUS) {
                    self.last_known = Some(c);
                    self.clear_path();
                }
            }
        }
        if let Gimmick::VoidBlack(v) = g {
            h.note_watched(v.watched, dt);
        }
        // Nyxie and Solyth pounce through their own gimmick (gimmicks/nyxie.rs pounce_frame) where its data applies
        // (Nightmare): the shared roster must not give them a second, independent pounce there. Elsewhere (Solyth on
        // Hard) the hard ability is their only pounce.
        let gimmick_pounces = matches!(g, Gimmick::Nyxie(_) | Gimmick::Solyth(_)) && super::gimmicks::nyxie::pounce().applies(h.diff);
        if !h.leaping && !holds && !gimmick_pounces {
            let near_link = chasing && {
                let clear = rules().leap.link_clearance;
                self.walk.is_some()
                    || fac.stairs.iter().any(|s| {
                        let c = s.cell.center();
                        dist(c, self.pos) <= clear || dist(c, target.pos) <= clear
                    })
                    // a drop (waterfall void, level edge) has no wall, so Facility::fits lets a body
                    // enter it and the straight sweep never blocks: the stair guard alone would launch
                    // a pounce out over open air. Reject a pounce whose ends straddle a drop cell too.
                    || fac.drop_cells().iter().any(|d| {
                        let c = d.center();
                        dist(c, self.pos) <= clear || dist(c, target.pos) <= clear
                    })
            };
            let ctx = PounceCtx {
                hunter: self.pos,
                target: target.pos,
                // the brain does not know which runner it chases; the host's shared budget still spaces the pounces
                runner: 0,
                chasing,
                visible: self.direct_visible,
                stunned: self.stun > 0.0,
                target_protected: target.hidden || target.in_safe_zone || target.in_vent || target.cross.is_some(),
                near_link,
                sweep_clear: chasing && self.clear_sweep(fac, self.pos, target.pos),
                flashlight_on: target.flashlight_on,
                temperature: target.temperature,
                hunter_in_room: fac.room_at(self.pos.0 as f64, self.pos.1 as f64).is_some(),
                roll: self.hard_rng.next_f32(),
                dt,
                now,
            };
            h.try_pounce(&ctx, &mut budget);
        }
        drop(budget);
        let mut leapt = false;
        if h.leaping {
            let wanted = h.leap_wanted(dt);
            let d = h.leap_dir;
            let moved = self.move_with_collision(d.0 * wanted, d.1 * wanted, fac);
            self.facing = d;
            self.stride += moved;
            if dt > 0.0 {
                self.moving_speed = moved / dt;
            }
            if let LeapEnd::Bonk(s) = h.finish_leap_step(dt, wanted, moved) {
                self.hard = Some(h);
                self.apply_stun(s, false);
                return true;
            }
            leapt = true;
        }
        self.hard = Some(h);
        leapt
    }

    /// The hunter leaned into a locker this frame: when its own target hides there it is found and chased (Godot
    /// hunter_brain.gd 1366-1380). Anyone else in that locker is resolved by the host through `take_locker_check`.
    fn resolve_locker_check(&mut self, target: &Target) {
        let Some(i) = self.locker_check else { return };
        if target.hidden && target.hiding_spot == Some(i) && !target.in_safe_zone {
            self.locker_found = Some(i);
            self.end_activity();
            self.poi = None;
            self.chase(target.pos);
            self.clear_path();
        }
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

    #[test]
    fn the_walk_data_equals_the_godot_values_and_a_changed_value_applies() {
        assert_eq!(*walk_rules(), WalkRules::GODOT);
        assert_eq!(WalkRules::parse("{}"), WalkRules::GODOT);
        let r = WalkRules::parse(r#"{"waypoint_distance":0.3,"path_lookahead_nodes":2,"sweep_step":0.1}"#);
        assert_eq!((r.waypoint_distance, r.path_lookahead_nodes, r.sweep_step), (0.3, 2, 0.1));
        assert_eq!(WalkRules::parse(r#"{"path_lookahead_nodes":0}"#).path_lookahead_nodes, 4);
    }
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
        let scale = crate::game::progression::Progression::load().chase_scale_for(Difficulty::Standard, &b.def.category);
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
        let scale = crate::game::progression::Progression::load().chase_scale_for(Difficulty::Standard, "az");
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
    fn embedded_every_difficulty_chases_bots() {
        let r = TargetRules::get();
        assert!(r.bots_allowed(Difficulty::Standard), "bots are chased like runners on every difficulty");
        assert!(r.bots_allowed(Difficulty::Relaxed));
        assert!(r.bots_allowed(Difficulty::Hard));
        assert!(r.bots_allowed(Difficulty::Nightmare));
        assert_eq!(r.bot_range, 12.0);
        assert!(r.bot_line_of_sight);
    }

    #[test]
    fn standard_chases_a_bot_right_beside_the_hunter() {
        let r = TargetRules::get();
        let got = choose_target(r, Difficulty::Standard, (0.0, 0.0), (9.0, 0.0), &[(0.5, 0.0)], clear);
        assert_eq!(got, Some(0), "Standard: a near bot beats a far runner");
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
    fn giving_up_leaves_a_chase_a_search_and_a_wander_picks_a_new_goal() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (_, floor) = wall_beside_floor(&fac);
        for (from, to) in [(State::Chase, State::Search), (State::Investigate, State::Search), (State::Wander, State::Wander)] {
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

    /// A facility with no flight in it: every step of a flight walk is refused on it (a body off the floor line fits
    /// only on a flight), which is how the tests block a walker.
    fn no_flight() -> Facility {
        Facility::test_levels_drop().0
    }

    #[test]
    fn a_walker_refused_past_the_stuck_time_walks_back_without_a_jump() {
        let s = stage(TestShape::Straight);
        let stuck = flight_edges::rules().edge_stuck_seconds;
        let mut b = hunter(&s, s.high, HIGH);
        let t = runner(s.low, LOW, true);
        let part_way = |h: &Brain| h.walk.as_ref().is_some_and(|w| w.fraction() > 0.3);
        run_until(&mut b, &s, &t, 25.0, |_| {}, part_way).expect("the hunter is part-way down the flight");
        assert_eq!(b.walk.as_ref().map(|w| w.to.level), Some(LOW));
        let prof = Difficulty::Standard.profile();
        let blocked = no_flight();
        let (at, at_y) = (b.pos, b.y);
        let (mut prev, mut prev_y) = (b.pos, b.y);
        let (mut worst, mut worst_y) = (0.0f32, 0.0f32);
        let mut step = |h: &Brain, prev: &mut (f32, f32), prev_y: &mut f32| {
            worst = worst.max(dist(*prev, h.pos));
            worst_y = worst_y.max((h.y - *prev_y).abs());
            *prev = h.pos;
            *prev_y = h.y;
        };
        // refused for less than the stuck time: it stands where it is and still means to go down
        let mut held = 0.0;
        while held < stuck - 0.5 {
            b.update(1.0 / 60.0, &ctx(&blocked), &t, 1, 0, &prof);
            step(&b, &mut prev, &mut prev_y);
            held += 1.0 / 60.0;
        }
        assert_eq!((b.pos, b.y), (at, at_y), "a refused walker stands still");
        assert_eq!(b.walk.as_ref().map(|w| w.to.level), Some(LOW), "and is not turned before the data time");
        // past the stuck time it is turned round on the spot
        while held < stuck + 0.5 {
            b.update(1.0 / 60.0, &ctx(&blocked), &t, 1, 0, &prof);
            step(&b, &mut prev, &mut prev_y);
            held += 1.0 / 60.0;
        }
        assert_eq!((b.pos, b.y), (at, at_y), "turning round moves nothing");
        assert_eq!(b.walk.as_ref().map(|w| w.to.level), Some(HIGH), "it now walks the same edge back");
        assert_eq!(b.snap_count, 0);
        // the way back is free: it walks up to where it came from
        let dt = 1.0 / 60.0;
        let mut time = 0.0;
        while b.walk.is_some() && time < 30.0 {
            let c = ctx(s.fac_of(b.level));
            b.update(dt, &c, &t, 1, 0, &prof);
            step(&b, &mut prev, &mut prev_y);
            time += dt;
        }
        assert!(crossed(&b, HIGH), "it ends on the level it started from (level {} y {} walk {})", b.level, b.y, b.walk.is_some());
        assert_eq!(b.snap_count, 0, "it walked back, it was not placed");
        assert!(worst < 0.7 && worst_y < 0.7, "no frame jumps (worst {worst:.2} m, {worst_y:.2} m in y)");
        assert_ne!(b.state, State::Chase, "the chase behind the refused flight is given up");
    }

    #[test]
    fn a_walker_held_back_by_the_director_is_never_timed_out() {
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.high, HIGH);
        let t = runner(s.low, LOW, true);
        run_until(&mut b, &s, &t, 25.0, |_| {}, |h| h.on_edge()).expect("the hunter steps onto the flight");
        let prof = Difficulty::Standard.profile();
        let wait = flight_edges::rules().edge_stuck_seconds * 3.0;
        let mut time = 0.0;
        while time < wait {
            b.edge_hold = 1.0; // the walker ahead is in the way
            let c = ctx(s.fac_of(b.level));
            b.update(1.0 / 60.0, &c, &t, 1, 0, &prof);
            time += 1.0 / 60.0;
        }
        assert_eq!(b.walk.as_ref().map(|w| w.to.level), Some(LOW), "still on the edge, still going down");
        assert_eq!(b.snap_count, 0);
        assert!(run_until(&mut b, &s, &t, 25.0, |_| {}, |h| crossed(h, LOW)).is_some(), "and it goes on once the way is free");
    }

    #[test]
    fn a_walker_refused_both_ways_is_stood_on_the_nearer_end() {
        // the one placement left: the flight is gone under the hunter (its level was unloaded), so it cannot walk back
        let s = stage(TestShape::Straight);
        let stuck = flight_edges::rules().edge_stuck_seconds;
        let mut b = hunter(&s, s.high, HIGH);
        let t = runner(s.low, LOW, true);
        run_until(&mut b, &s, &t, 25.0, |_| {}, |h| h.walk.as_ref().is_some_and(|w| w.fraction() > 0.2)).expect("on the flight");
        let prof = Difficulty::Standard.profile();
        let blocked = no_flight();
        let mut time = 0.0;
        while b.on_edge() && time < 2.0 * stuck + 2.0 {
            b.update(1.0 / 60.0, &ctx(&blocked), &t, 1, 0, &prof);
            time += 1.0 / 60.0;
        }
        assert!(!b.on_edge(), "off the edge after two stuck times");
        assert!(time > 2.0 * stuck - 0.1, "not before it tried to walk back: {time}");
        assert_eq!(b.snap_count, 1);
        assert!(b.y == floor_y_of(LOW) || b.y == floor_y_of(HIGH));
        assert_eq!(b.y, floor_y_of(b.level));
    }

    #[test]
    fn a_hunter_shelved_in_the_middle_of_an_edge_stands_on_an_end_of_it() {
        for shape in [TestShape::Straight, TestShape::Ladder] {
            for keep in [HIGH, LOW] {
                let s = stage(shape);
                let mut b = hunter(&s, s.high, HIGH);
                let t = runner(s.low, LOW, true);
                run_until(&mut b, &s, &t, 60.0, |_| {}, |h| h.walk.as_ref().is_some_and(|w| w.fraction() > 0.4)).expect("part-way");
                let w = b.walk.clone().unwrap();
                b.settle_edge(keep);
                assert!(b.walk.is_none(), "{shape:?}: the walk is over");
                assert_eq!((b.level, b.y), (keep, floor_y_of(keep)), "{shape:?}: it stands on the floor of the level it is kept under");
                let end = if keep == HIGH { w.from.cell } else { w.to.cell };
                assert_eq!(b.pos, end.center(), "{shape:?}: on that end of its edge");
                assert_eq!(b.snap_count, 0);
            }
        }
        // a hunter that is not on an edge is left alone
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.high, HIGH);
        b.settle_edge(LOW);
        assert_eq!((b.pos, b.level, b.y), (s.high, HIGH, floor_y_of(HIGH)));
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

    // -- cross-level walking (nav3d pass) ---------------------------------------------------------------------------

    /// The first edge leaving `HIGH` on a stage (its polyline is the treads).
    fn high_edge(s: &Stage) -> flight_edges::Edge {
        s.nav.edges_near(HIGH, s.high.0, s.high.1, 200.0).next().expect("the stage has an edge off the upper level").clone()
    }

    #[test]
    fn a_hunter_walks_every_stair_type_both_ways_without_a_teleport() {
        let storey = floor_y_of(HIGH) - floor_y_of(LOW);
        for shape in [TestShape::Straight, TestShape::Switchback, TestShape::Spiral, TestShape::Ladder] {
            for down in [true, false] {
                let s = stage(shape);
                let (from, level, goal, to) = if down { (s.high, HIGH, s.low, LOW) } else { (s.low, LOW, s.high, HIGH) };
                let mut b = hunter(&s, from, level);
                let t = runner(goal, to, true);
                let (mut prev, mut prev_y) = (b.pos, b.y);
                let (mut worst, mut worst_y, mut walked) = (0.0f32, 0.0f32, false);
                let (mut top, mut bottom) = (f32::MIN, f32::MAX);
                let took = run_until(
                    &mut b,
                    &s,
                    &t,
                    60.0,
                    |h| {
                        worst = worst.max(dist(prev, h.pos));
                        worst_y = worst_y.max((h.y - prev_y).abs());
                        walked |= h.walk.is_some();
                        top = top.max(h.y);
                        bottom = bottom.min(h.y);
                        prev = h.pos;
                        prev_y = h.y;
                    },
                    |h| crossed(h, to),
                );
                let way = if down { "down" } else { "up" };
                assert!(took.is_some(), "{shape:?} {way}: the hunter reaches the other floor (level {} y {})", b.level, b.y);
                assert!(walked, "{shape:?} {way}: it walked the edge rather than appearing on the other floor");
                assert_eq!(b.snap_count, 0, "{shape:?} {way}: no walk ended by a placement");
                assert!(worst < 0.7, "{shape:?} {way}: no frame moves the body like a teleport (worst {worst:.2} m)");
                assert!(worst_y < 0.7, "{shape:?} {way}: the height is continuous (worst {worst_y:.2} m in one frame)");
                assert!(top - bottom > storey - 0.05, "{shape:?} {way}: the height went through the whole storey ({bottom} to {top})");
                assert_eq!((b.level, b.y), (to, floor_y_of(to)), "{shape:?} {way}");
            }
        }
    }

    #[test]
    fn a_hunter_walks_down_every_flight_without_a_teleport() {
        for shape in [TestShape::Straight, TestShape::Switchback, TestShape::Spiral] {
            let s = stage(shape);
            let mut b = hunter(&s, s.high, HIGH);
            let t = runner(s.low, LOW, true);
            let (mut prev, mut prev_y) = (b.pos, b.y);
            let (mut worst, mut walked, mut rose) = (0.0f32, false, 0.0f32);
            let (mut top, mut bottom) = (f32::MIN, f32::MAX);
            let took = run_until(
                &mut b,
                &s,
                &t,
                45.0,
                |h| {
                    worst = worst.max(dist(prev, h.pos));
                    rose = rose.max(h.y - prev_y);
                    if h.walk.is_some() {
                        walked = true;
                        top = top.max(h.y);
                        bottom = bottom.min(h.y);
                    }
                    prev = h.pos;
                    prev_y = h.y;
                },
                |h| crossed(h, LOW),
            );
            assert!(took.is_some(), "{shape:?}: the hunter reaches the lower floor");
            assert!(walked, "{shape:?}: it walked the flight rather than appearing below");
            assert!(worst < 0.7, "{shape:?}: no frame moves the body like a teleport (worst {worst:.2} m)");
            assert!(rose < 1e-4, "{shape:?}: going down, y never rises ({rose})");
            assert!(top - bottom > 0.9 * (floor_y_of(HIGH) - floor_y_of(LOW)) - 0.4, "{shape:?}: y fell through the flight ({top} to {bottom})");
            assert_eq!(b.level, LOW);
        }
    }

    #[test]
    fn a_runner_standing_on_the_treads_is_still_followed() {
        for shape in [TestShape::Straight, TestShape::Switchback, TestShape::Spiral] {
            let s = stage(shape);
            let e = high_edge(&s);
            let mid = e.path[e.path.len() / 2];
            let mut b = hunter(&s, s.high, HIGH);
            let t = runner((mid[0], mid[1]), LOW, true);
            let started = run_until(&mut b, &s, &t, 45.0, |_| {}, |h| h.on_edge());
            assert!(started.is_some(), "{shape:?}: the goal on a tread is snapped to open ground and the hunter takes the flight");
        }
    }

    #[test]
    fn a_chaser_follows_a_runner_that_changed_level() {
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.high, HIGH);
        b.state = State::Chase;
        b.last_known = Some(s.high);
        b.follow_cross(s.low, LOW, None);
        assert_eq!(b.state, State::Investigate, "it goes to where the runner arrived");
        assert_eq!(b.goal_level(), LOW);
        assert!(b.follow_cross.is_some(), "and keeps that memory for the data time");
        let t = Target { pos: (-500.0, -500.0), hidden: true, ..Default::default() };
        assert!(run_until(&mut b, &s, &t, 45.0, |_| {}, |h| crossed(h, LOW)).is_some(), "it walks the flight after the runner");
        // a wanderer is not called when the runner took no flight
        let mut w = hunter(&s, s.high, HIGH);
        w.state = State::Wander;
        w.follow_cross(s.low, LOW, None);
        assert_eq!(w.state, State::Wander);
        assert!(w.follow_cross.is_none());
        assert_eq!(w.bound_for(), None);
    }

    #[test]
    fn an_awake_hunter_near_the_flight_the_runner_took_follows_and_a_far_one_does_not() {
        let s = stage(TestShape::Straight);
        let reach = flight_edges::rules().follow_cross_cells;
        let entrance = high_edge(&s).from.cell.center();
        let t = Target { pos: (-500.0, -500.0), hidden: true, ..Default::default() };
        for state in [State::Wander, State::Idle, State::Search] {
            let mut near = hunter(&s, s.high, HIGH);
            assert!(dist(near.pos, entrance) <= reach, "the stage's floor point is within the follow distance");
            near.state = state;
            near.follow_cross(s.low, LOW, Some(entrance));
            assert_eq!(near.state, State::Investigate, "{state:?}: it goes to where the runner arrived");
            assert_eq!((near.goal_level(), near.bound_for()), (LOW, Some(LOW)), "{state:?}: on the floor below");
            assert!(run_until(&mut near, &s, &t, 60.0, |_| {}, |h| crossed(h, LOW)).is_some(), "{state:?}: it walks the flight after the runner");
            assert_eq!(near.snap_count, 0);
            // one step beyond the data distance stays as it was
            let mut far = hunter(&s, (entrance.0 + reach + 1.0, entrance.1), HIGH);
            far.state = state;
            far.follow_cross(s.low, LOW, Some(entrance));
            assert_eq!(far.state, state, "{state:?}: a hunter beyond the follow distance is not called");
            assert_eq!(far.bound_for(), None);
        }
        // a chaser follows from anywhere, as before
        let mut chaser = hunter(&s, (entrance.0 + reach + 1.0, entrance.1), HIGH);
        chaser.state = State::Chase;
        chaser.last_known = Some(s.high);
        chaser.follow_cross(s.low, LOW, Some(entrance));
        assert_eq!(chaser.bound_for(), Some(LOW));
        // a sleeping hunter and one in the middle of a peek never follow
        let mut asleep = hunter(&s, s.high, HIGH);
        asleep.active = false;
        asleep.state = State::Wander;
        asleep.follow_cross(s.low, LOW, Some(entrance));
        assert_eq!((asleep.state, asleep.bound_for()), (State::Wander, None));
        let mut peeking = hunter(&s, s.high, HIGH);
        peeking.state = State::Peek;
        peeking.follow_cross(s.low, LOW, Some(entrance));
        assert_eq!((peeking.state, peeking.bound_for()), (State::Peek, None));
    }

    #[test]
    fn a_hunter_on_an_edge_or_bound_for_another_floor_pins_its_levels() {
        let s = stage(TestShape::Straight);
        let idle = hunter(&s, s.high, HIGH);
        let mut out = Vec::new();
        idle.pin_levels(&mut out);
        assert!(out.is_empty(), "a hunter at home pins nothing");
        let mut walker = hunter(&s, s.high, HIGH);
        let t = runner(s.low, LOW, true);
        run_until(&mut walker, &s, &t, 25.0, |_| {}, |h| h.on_edge()).expect("on the edge");
        walker.pin_levels(&mut out);
        assert!(out.contains(&HIGH) && out.contains(&LOW), "both ends of the edge walked: {out:?}");
        let mut bound = hunter(&s, s.high, HIGH);
        bound.investigate_on(s.low, LOW);
        let mut out = Vec::new();
        bound.pin_levels(&mut out);
        assert!(out.contains(&LOW), "the goal's level: {out:?}");
    }

    #[test]
    fn a_wanderer_picks_the_far_end_of_a_stair_and_arrives_on_the_other_floor() {
        let mut rules = flight_edges::rules().clone();
        rules.wander_cross_chance = 1.0;
        let s = stage_with(TestShape::Straight, rules);
        let prof = Difficulty::Standard.profile();
        let mut b = hunter(&s, s.high, HIGH);
        b.state = State::Idle;
        b.idle_timer = 0.0;
        let t = Target { pos: (-500.0, -500.0), hidden: true, ..Default::default() };
        let c = ctx(s.fac_of(HIGH));
        b.update(1.0 / 60.0, &c, &t, 1, 0, &prof);
        assert_eq!(b.state, State::Wander);
        assert_eq!(b.goal_level(), LOW, "the goal is on the other level");
        assert!(run_until(&mut b, &s, &t, 45.0, |_| {}, |h| crossed(h, LOW)).is_some(), "it walks the flight and arrives");
    }

    #[test]
    fn followers_landing_from_one_flight_do_not_stack() {
        let s = stage(TestShape::Straight);
        let t = runner(s.low, LOW, true);
        let mut spots = Vec::new();
        for seed in ["EDGE-A", "EDGE-B", "EDGE-C"] {
            let mut b = Brain::new(aki(), s.high, seed, Difficulty::Standard);
            b.active = true;
            b.set_level(HIGH);
            b.set_nav(s.nav.clone());
            run_until(&mut b, &s, &t, 40.0, |_| {}, |h| h.level == LOW && h.walk.is_none()).expect("lands");
            spots.push(b.pos);
        }
        let apart = (0..3).any(|i| ((i + 1)..3).any(|j| dist(spots[i], spots[j]) > 0.003));
        assert!(apart, "three followers do not land on one point: {spots:?}");
        for p in &spots {
            assert!(dist(*p, spots[0]) <= 2.0 * flight_edges::rules().land_spread + 0.5, "and stay close to the foot: {p:?}");
        }
    }

    #[test]
    fn a_landing_spot_that_does_not_fit_falls_back_to_the_cell_centre() {
        let s = stage(TestShape::Straight);
        let fac = s.fac_of(LOW);
        // an open cell with a spot inside it that a hunter does not fit (a prop, a divider or a door edge in the cell)
        let mut found = None;
        'scan: for x in 0..40 {
            for y in 0..40 {
                let c = Cell::new(x, y);
                if !fac.hunter_open(c) {
                    continue;
                }
                let (cx, cz) = c.center();
                for (dx, dz) in [(0.3, 0.0), (-0.3, 0.0), (0.0, 0.3), (0.0, -0.3)] {
                    if !fac.hunter_fits(cx + dx, cz + dz) {
                        found = Some((c, (cx + dx, cz + dz)));
                        break 'scan;
                    }
                }
            }
        }
        let (cell, bad) = found.expect("a cell with an unfit corner on the landing level");
        let mut b = hunter(&s, bad, LOW);
        b.land_check = true;
        let t = Target { pos: (-500.0, -500.0), hidden: true, ..Default::default() };
        b.update(1.0 / 60.0, &ctx(fac), &t, 1, 0, &Difficulty::Standard.profile());
        assert!(!b.land_check, "the check runs once");
        assert_eq!(cell_of(b.pos), cell);
        assert!(fac.hunter_fits(b.pos.0, b.pos.1), "set on ground a hunter fits: {:?}", b.pos);
        // and a fitting landing spot is kept
        let (cx, cz) = cell.center();
        let mut g = hunter(&s, (cx, cz), LOW);
        g.land_check = true;
        g.update(1.0 / 60.0, &ctx(fac), &t, 1, 0, &Difficulty::Standard.profile());
        assert!(fac.hunter_fits(g.pos.0, g.pos.1));
    }

    #[test]
    fn a_chase_toward_a_drop_never_stands_on_a_drop_cell() {
        let s = stage(TestShape::Drop);
        let hole = s.upper.drop_cells()[0];
        let a = (hole.x as f32 - 3.5, hole.y as f32 + 0.5);
        let c = (hole.x as f32 + 4.5, hole.y as f32 + 0.5);
        assert!(s.upper.hunter_open(cell_of(a)) && s.upper.hunter_open(cell_of(c)), "the stage floor is open either side of the hole");
        let mut b = hunter(&s, a, HIGH);
        b.state = State::Chase;
        b.last_known = Some(c);
        let t = Target { pos: c, hidden: true, ..Default::default() };
        let prof = Difficulty::Standard.profile();
        let cx = ctx(&s.upper);
        for _ in 0..900 {
            b.update(1.0 / 60.0, &cx, &t, 1, 0, &prof);
            assert!(!s.upper.is_drop(cell_of(b.pos)), "the hunter stands on a drop cell at {:?}", b.pos);
        }
    }

    #[test]
    fn a_hop_is_planned_once_and_kept_while_nothing_changes() {
        let s = stage(TestShape::Straight);
        let mut b = hunter(&s, s.high, HIGH);
        b.investigate_on(s.low, LOW);
        let prof = Difficulty::Standard.profile();
        let fac = s.fac_of(HIGH);
        b.pursue(s.low, 1.0, 1.0 / 60.0, fac, &prof);
        let first = b.hop;
        assert!(first.is_some(), "the flight is found");
        let key = b.hop_key;
        for _ in 0..20 {
            b.pursue(s.low, 1.0, 1.0 / 60.0, fac, &prof);
        }
        assert_eq!(b.hop, first, "a committed edge is kept");
        assert_eq!(b.hop_key, key);
        assert!(b.hop_age > 0.3, "the cache ages instead of being replanned each frame");
    }
}

/// The body rules between a hunter and its target: a grab needs a clear line, two bodies never share a spot, and the
/// straight chase stops where the bodies touch.
#[cfg(test)]
mod body_rule_tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn aki() -> HunterDef {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        crate::content::hunters::parse(&v, None, "").unwrap()
    }

    fn facility() -> Facility {
        Facility::generate("BODY-RULES-1", &Options::default(), &Catalog::load())
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

    /// The centre of a cell with a clear run of `len` metres along +x for a hunter's body and a clear line of sight.
    fn clear_run(fac: &Facility, len: f32) -> (f32, f32) {
        let steps = (len * 10.0) as i32;
        for c in fac.floor_cells() {
            let (sx, sy) = (c.x as f32 + 0.5, c.y as f32 + 0.5);
            if (0..=steps).all(|i| fac.body_fits(sx + i as f32 * 0.1, sy, HUNTER_RADIUS)) && fac.has_line_of_sight((sx, sy), (sx + len, sy)) {
                return (sx, sy);
            }
        }
        panic!("a generated floor has a clear run of {len} m");
    }

    /// A hunter spot and a runner spot with a thin wall (one to three cells) between them, close enough that a long arm
    /// would reach. Generated walls are seldom one cell thick, so the run of wall cells is searched for.
    fn across_a_wall(fac: &Facility) -> ((f32, f32), (f32, f32)) {
        for y in 1..fac.rows - 1 {
            for x in 1..fac.cols - 1 {
                if !fac.hunter_open(Cell::new(x - 1, y)) || fac.is_floor(x as f64 + 0.5, y as f64 + 0.5) {
                    continue;
                }
                for k in 1..=3 {
                    if (0..k).any(|i| fac.is_floor((x + i) as f64 + 0.5, y as f64 + 0.5)) {
                        break;
                    }
                    let h = (x as f32 - 0.35, y as f32 + 0.5);
                    let r = ((x + k) as f32 + 0.25, y as f32 + 0.5);
                    if fac.body_fits(h.0, h.1, HUNTER_RADIUS) && fac.body_fits(r.0, r.1, PLAYER_RADIUS) && !fac.has_line_of_sight(h, r) {
                        return (h, r);
                    }
                }
            }
        }
        panic!("a generated floor has a thin wall with floor on both sides");
    }

    fn chaser(at: (f32, f32), known: (f32, f32)) -> Brain {
        let mut b = Brain::new(aki(), at, "BODY", Difficulty::Standard);
        b.active = true;
        b.chase(known);
        b
    }

    #[test]
    fn a_grab_through_a_wall_is_refused_and_a_clear_line_is_taken() {
        let fac = facility();
        let mut prof = Difficulty::Standard.profile();
        prof.capture_distance = 3.0; // a long arm: only the wall decides
        let (h, r) = across_a_wall(&fac);
        assert!(dist(h, r) < 1.47 * 3.0 && dist(h, r) > BODY_GAP + flight_edges::rules().capture_los_slack);
        let mut b = chaser(h, r);
        let t = Target { pos: r, flashlight_on: true, ..Default::default() };
        let (caught, _) = b.update(1.0 / 60.0, &ctx(&fac), &t, 1, 0, &prof);
        assert!(!caught, "the wall is between them");

        let (sx, sy) = clear_run(&fac, 3.0);
        let mut b = chaser((sx, sy), (sx + 1.2, sy));
        let t = Target { pos: (sx + 1.2, sy), flashlight_on: true, ..Default::default() };
        let (caught, _) = b.update(1.0 / 60.0, &ctx(&fac), &t, 1, 0, &prof);
        assert!(caught, "a clear line is a grab");
    }

    #[test]
    fn a_hunter_is_pushed_back_from_a_target_it_cannot_catch() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (sx, sy) = clear_run(&fac, 4.0);
        let mut b = Brain::new(aki(), (sx + 1.5, sy), "BODY", Difficulty::Standard);
        b.active = true;
        b.state = State::Idle;
        b.idle_timer = 99.0;
        // a runner in a safe zone cannot be caught: the hunter must not stand on it
        let t = Target { pos: (sx + 1.2, sy), in_safe_zone: true, ..Default::default() };
        let (caught, _) = b.update(1.0 / 60.0, &ctx(&fac), &t, 1, 0, &prof);
        assert!(!caught);
        assert!(dist(b.pos, t.pos) >= BODY_GAP - 0.02, "pushed back to {:.2} m", dist(b.pos, t.pos));
        assert!(fac.body_fits(b.pos.0, b.pos.1, HUNTER_RADIUS), "and still on the floor");
    }

    #[test]
    fn the_straight_chase_stops_where_the_bodies_touch() {
        let fac = facility();
        let prof = Difficulty::Standard.profile();
        let (sx, sy) = clear_run(&fac, 3.5);
        let goal = (sx + 3.0, sy);
        let mut b = chaser((sx, sy), goal);
        // out of reach by height, so nothing grabs: the chase itself must keep the gap
        let t = Target { pos: goal, y: 9.0, flashlight_on: true, ..Default::default() };
        let mut reached = false;
        for _ in 0..600 {
            b.update(1.0 / 60.0, &ctx(&fac), &t, 1, 0, &prof);
            if dist(b.pos, goal) <= ARRIVAL_DISTANCE {
                reached = true;
                assert!(dist(b.pos, goal) >= BODY_GAP - 0.03, "stopped at {:.2} m", dist(b.pos, goal));
                break;
            }
        }
        assert!(reached, "the hunter closes on the runner");
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
    fn a_cross_floor_goal_with_no_flight_is_remembered_not_searched_every_frame() {
        path_scheduler::reset_thread();
        let prof = Difficulty::Standard.profile();
        let fac = facility();
        let cells = fac.floor_cells();
        let mut h = Brain::new(aki(), cells[0].center(), "NOHOP-1", Difficulty::Standard);
        h.active = true;
        h.state = State::Chase;
        let goal = cells[cells.len() / 2].center();
        h.last_known = Some(goal);
        h.last_known_stamp = Some(goal);
        h.last_known_level = h.level - 2; // no flight links the (empty) nav graph
        h.pursue(goal, 1.0, 1.0 / 60.0, &fac, &prof);
        let first = h.no_hop_memo.expect("the failed hop search is remembered");
        h.pursue(goal, 1.0, 1.0 / 60.0, &fac, &prof);
        let second = h.no_hop_memo.expect("and kept");
        assert_eq!((first.0, first.1, first.2, first.3), (second.0, second.1, second.2, second.3));
        assert!(second.4 < first.4, "the memo ran down instead of being searched afresh");
        h.give_up_goal(&fac, &prof);
        assert!(h.no_hop_memo.is_none(), "giving up forgets it");
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
        for (i, scale) in rules.chase_speed_scale.iter().enumerate() {
            assert!((1.0..1.12).contains(scale), "slightly faster, never slower: {scale} on difficulty {i}");
        }
        let content = Content::base();
        assert!(content.hunters.len() >= 38);
        let prof = Difficulty::Standard.profile();
        for def in &content.hunters {
            let scale = if def.category == "secret" { 1.0 } else { rules.chase_speed_scale[1] };
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
        for id in ["white_wolf_spider"] {
            let def = content.hunter(id).unwrap();
            assert_eq!(def.category, "secret");
            let chase = brain_of(def, Difficulty::Standard, State::Chase).current_speed(1, 0, &prof, 1.0);
            assert!((chase - def.chase).abs() < 1e-5, "{id}: {chase} vs {}", def.chase);
        }
    }

    /// The scale is one data key: a changed value, a skipped category, and a missing key (the shipped default).
    #[test]
    fn the_scale_comes_from_the_data_file() {
        let st = Difficulty::Standard;
        let p = Progression::parse(&serde_json::json!({ "chase_speed_scale": 1.5, "chase_speed_scale_skip": ["void"] }));
        for d in PROFILES {
            assert_eq!(p.chase_scale_for(d, "az"), 1.5, "a single number applies to every difficulty ({d:?})");
        }
        assert_eq!(p.chase_scale_for(st, "void"), 1.0);
        assert_eq!(p.chase_scale_for(st, "secret"), 1.5, "the skip list is replaced, not merged");
        let d = Progression::parse(&serde_json::json!({}));
        assert_eq!(d.chase_speed_scale, Progression::load().chase_speed_scale);
        assert_eq!(d.chase_scale_for(st, "secret"), 1.0);
        let off = Progression::parse(&serde_json::json!({ "chase_speed_scale": 1.0 }));
        assert_eq!(off.chase_scale_for(st, "az"), 1.0, "1.0 gives the original roster back");
    }

    /// The scale is a per-difficulty table: each named difficulty is read, an unnamed one keeps its shipped value.
    #[test]
    fn the_scale_is_a_per_difficulty_table() {
        let p = Progression::parse(&serde_json::json!({
            "chase_speed_scale": { "relaxed": 1.02, "nightmare": 1.3, "hard": "fast" }
        }));
        let shipped = Progression::default();
        assert_eq!(p.chase_scale_for(Difficulty::Relaxed, "az"), 1.02);
        assert_eq!(p.chase_scale_for(Difficulty::Nightmare, "az"), 1.3);
        assert_eq!(p.chase_scale_for(Difficulty::Standard, "az"), shipped.chase_scale_for(Difficulty::Standard, "az"));
        assert_eq!(p.chase_scale_for(Difficulty::Hard, "az"), shipped.chase_scale_for(Difficulty::Hard, "az"));
        assert_eq!(p.chase_scale_for(Difficulty::Nightmare, "secret"), 1.0, "skipped categories ignore the table");
        // out of range entries are clamped like the single number
        let wild = Progression::parse(&serde_json::json!({ "chase_speed_scale": { "standard": 99.0, "hard": -1.0 } }));
        assert_eq!(wild.chase_scale_for(Difficulty::Standard, "az"), 2.0);
        assert_eq!(wild.chase_scale_for(Difficulty::Hard, "az"), 0.5);
        // the shipped file is the table
        let rules = Progression::load();
        assert_eq!(rules.chase_speed_scale, shipped.chase_speed_scale, "data/progression.json matches the defaults");
        let [relaxed, standard, hard, nightmare] = rules.chase_speed_scale;
        assert!(nightmare > standard.max(relaxed).max(hard), "Nightmare is the fastest: {:?}", rules.chase_speed_scale);
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
            let scale = rules.chase_scale_for(diff, &aki.category);
            let base = aki.chase * scale * prof.hunter_speed;
            assert!((b.current_speed(1, 0, &prof, 1.0) - base).abs() < 1e-4, "{diff:?}");
            // relays: the bonus times `prog`, then the scale
            let with_relays = (aki.chase + 3.0 * rules.chase_speed_bonus_per_relay * aki.prog)
                * scale
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

    /// The runner's full-stamina sprint on `diff` and a hunter's clear-line dash (depth 1, no relays), both end to end.
    fn sprint_and_dash(def: &HunterDef, diff: Difficulty, scale: Option<f32>) -> (f32, f32) {
        let prof = diff.profile();
        let sprint = PLAYER_SPRINT_SPEED * prof.sprint_speed;
        let mut chase = brain_of(def, diff, State::Chase).current_speed(1, 0, &prof, 1.0);
        if let Some(s) = scale {
            // the same hunter with another scale (1.0 = before the knob existed)
            chase = chase / Progression::load().chase_scale_for(diff, &def.category) * s;
        }
        (sprint, chase * HUNTER_VISIBLE_CHASE_SPEED_SCALE)
    }

    /// Before/after dash vs sprint for every hunter on every difficulty (run with --nocapture for the table in the
    /// commit message). The knob never lets a dash out-run the sprint except on Nightmare:
    /// - Relaxed and Standard: no non-secret hunter's dash passes the runner's sprint.
    /// - Hard: a hunter that was under the sprint before the scale is still under it, and one that was already over it is
    ///   no faster than before.
    /// - Nightmare: the scale is what lifts some hunters (kathy) past the sprint.
    #[test]
    fn only_nightmare_lets_a_scaled_dash_outrun_the_sprint() {
        let content = Content::base();
        let mut crossed_on_nightmare = Vec::new();
        for diff in PROFILES {
            let sprint = sprint_and_dash(&content.hunters[0], diff, None).0;
            println!("{diff:?}: runner sprint {sprint:.3}");
            for def in content.hunters.iter().filter(|d| d.category != "secret") {
                let (_, old) = sprint_and_dash(def, diff, Some(1.0));
                let (_, new) = sprint_and_dash(def, diff, None);
                println!("  {:<14} old dash {old:.2}  new dash {new:.2}", def.id);
                match diff {
                    Difficulty::Relaxed | Difficulty::Standard => {
                        assert!(new <= sprint, "{diff:?} {}: dash {new} vs sprint {sprint}", def.id);
                    }
                    Difficulty::Hard => {
                        if old <= sprint {
                            assert!(new <= sprint, "Hard {}: pushed over the sprint ({old} -> {new})", def.id);
                        }
                        assert!(new <= old + 1e-4, "Hard {}: faster than before ({old} -> {new})", def.id);
                    }
                    Difficulty::Nightmare => {
                        assert!(new >= old, "{}", def.id);
                        if old <= sprint && new > sprint {
                            crossed_on_nightmare.push(def.id.clone());
                        }
                    }
                }
            }
        }
        assert!(
            crossed_on_nightmare.iter().any(|id| id == "kathy"),
            "kathy's dash passes the Nightmare sprint only through the scale: {crossed_on_nightmare:?}"
        );
    }

    /// The secret creatures keep their own authored speed on every difficulty (the table does not touch them).
    #[test]
    fn the_table_never_touches_the_secret_creatures() {
        let content = Content::base();
        for diff in PROFILES {
            let prof = diff.profile();
            let def = content.hunter("white_wolf_spider").unwrap();
            let chase = brain_of(def, diff, State::Chase).current_speed(1, 0, &prof, 1.0);
            assert!((chase - def.chase * prof.hunter_speed).abs() < 1e-4, "{diff:?}");
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
        assert!(hi.chase_speed_scale.iter().all(|x| *x <= 2.0));
        let lo = Progression::parse(&serde_json::json!({ "chase_speed_scale": -5.0 }));
        assert!(lo.chase_speed_scale.iter().all(|x| *x >= 0.5));
        let junk = Progression::parse(
            &serde_json::json!({ "chase_speed_scale": "fast", "chase_speed_scale_skip": [1, null, "az"] }),
        );
        assert_eq!(junk.chase_speed_scale, Progression::default().chase_speed_scale, "a non-number keeps the default");
        assert_eq!(junk.chase_scale_for(Difficulty::Standard, "az"), 1.0);
        assert_eq!(junk.chase_scale_for(Difficulty::Standard, "void"), junk.chase_speed_scale[1]);
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::hunters::activities::{points_of_interest, PoiKind, Rules};
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn aki() -> HunterDef {
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        crate::content::hunters::parse(&v, None, "").unwrap()
    }

    fn fac() -> Facility {
        Facility::generate("ACT-BRAIN-1", &Options::default(), &Catalog::load())
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

    /// A runner so far away that nothing senses it.
    fn far() -> Target {
        Target { pos: (-900.0, -900.0), facing: (1.0, 0.0), ..Default::default() }
    }

    /// A bed point of interest at the stand of a room centre, its mattress a metre along +x (the long axis).
    fn bed_poi(f: &Facility) -> Poi {
        let stand = points_of_interest(f, Rules::get()).into_iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap().stand;
        Poi {
            kind: PoiKind::Bed,
            stand,
            focus: (stand.0 + 1.0, stand.1),
            room: None,
            rtype: String::new(),
            region: "maze".into(),
            locker: None,
            door: None,
            axis: (1.0, 0.0),
        }
    }

    #[test]
    fn a_hunter_naps_in_its_bed_for_the_duration_and_then_idles() {
        let f = fac();
        let rules = Rules::get();
        let bed = bed_poi(&f);
        let mut b = wanderer(&f, bed.stand);
        b.start_activity(Activity::Nap, &bed, &f);
        assert!(b.napping(), "asleep");
        let spot = b.nap_spot().expect("a napper has a spot");
        assert_eq!((spot.at, spot.axis), (bed.focus, (1.0, 0.0)), "it lies on the mattress along the bed");
        let (lo, hi) = rules.durations[&Activity::Nap];
        run(&mut b, &f, &far(), lo * 0.5);
        assert!(b.napping(), "a far runner does not wake it");
        assert_eq!(b.state, State::Wander);
        // the duration is drawn in [lo, hi]: run until the nap is over (it can end at any point of that range)
        let mut slept = lo * 0.5;
        while b.napping() && slept < hi + 1.0 {
            run(&mut b, &f, &far(), 0.1);
            slept += 0.1;
        }
        assert!(!b.napping() && b.nap_spot().is_none(), "the nap is over after {slept} s");
        assert!(slept >= lo - 0.2 && slept <= hi + 0.2, "a nap lasts its drawn duration: {slept} s");
        assert_eq!(b.state, State::Idle, "it wakes rested and idles");
    }

    #[test]
    fn a_runner_close_by_or_a_loud_one_wakes_a_napper_with_a_startle_and_it_goes_to_look() {
        let f = fac();
        let rules = Rules::get();
        let bed = bed_poi(&f);
        let mut b = wanderer(&f, bed.stand);
        b.start_activity(Activity::Nap, &bed, &f);
        let mut t = far();
        t.pos = (b.pos.0 + rules.nap_wake_distance * 0.5, b.pos.1);
        run(&mut b, &f, &t, 0.1);
        assert!(!b.napping(), "a runner close by wakes it");
        assert_eq!(b.state, State::Investigate, "it goes to where the runner was");
        assert_eq!(b.last_known, Some(t.pos));
        assert!(b.stun > 0.0, "the flinch stuns it for a moment");
        // a loud runner a little further off wakes a napper too
        let mut b = wanderer(&f, bed.stand);
        b.start_activity(Activity::Nap, &bed, &f);
        let mut t = far();
        t.pos = (b.pos.0 + rules.nap_wake_noise_distance * 0.5, b.pos.1);
        t.noise = rules.nap_wake_noise;
        run(&mut b, &f, &t, 0.1);
        assert!(!b.napping(), "a loud runner wakes it");
    }

    #[test]
    fn an_alert_ends_a_nap_and_the_hunter_goes_to_the_alert() {
        let f = fac();
        let bed = bed_poi(&f);
        let mut b = wanderer(&f, bed.stand);
        b.start_activity(Activity::Nap, &bed, &f);
        b.alert_on((bed.stand.0 + 4.0, bed.stand.1), 0);
        run(&mut b, &f, &far(), 0.1);
        assert!(!b.napping(), "an alert wakes a napper");
        assert_eq!(b.state, State::Investigate, "and it goes to the alert, it does not sleep on");
    }

    #[test]
    fn a_hidden_runner_close_by_does_not_wake_a_napper() {
        let f = fac();
        let bed = bed_poi(&f);
        let mut b = wanderer(&f, bed.stand);
        b.start_activity(Activity::Nap, &bed, &f);
        let mut t = far();
        t.pos = (b.pos.0 + 0.5, b.pos.1);
        t.hidden = true;
        run(&mut b, &f, &t, 0.5);
        assert!(b.napping(), "a hidden runner is not heard");
    }


    fn run(b: &mut Brain, fac: &Facility, t: &Target, seconds: f32) {
        let prof = Difficulty::Standard.profile();
        let c = ctx(fac);
        for _ in 0..(seconds * 60.0).round() as i32 {
            b.update(1.0 / 60.0, &c, t, 1, 0, &prof);
        }
    }

    fn wanderer(fac: &Facility, at: (f32, f32)) -> Brain {
        let mut b = Brain::new(aki(), at, "ACT", Difficulty::Standard);
        b.active = true;
        b.state = State::Wander;
        b.idle_timer = 0.0;
        b.set_level(fac.level);
        b
    }

    /// An open cell with its whole 3x3 open, walled in on its four sides: a sealed one-cell pocket. The pocket is
    /// at least 4 cells from `away`.
    fn seal_a_pocket(fac: &mut Facility, away: Cell) -> (f32, f32) {
        for y in 2..fac.rows - 2 {
            for x in 2..fac.cols - 2 {
                let c = Cell::new(x, y);
                let open3 = (-1..=1).all(|dy| (-1..=1).all(|dx| fac.hunter_open(Cell::new(x + dx, y + dy))));
                let far_enough = (c.x - away.x).abs() > 3 || (c.y - away.y).abs() > 3;
                if open3 && far_enough {
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        fac.wall[(y + dy) as usize][(x + dx) as usize] = true;
                    }
                    fac.rebuild_grids();
                    return c.center();
                }
            }
        }
        panic!("the generated floor has a 3x3 open patch");
    }

    #[test]
    fn a_new_hunter_pauses_in_a_real_idle_before_it_wanders() {
        let f = fac();
        let mut b = Brain::new(aki(), f.start.center(), "ACT", Difficulty::Standard);
        let sp = Rules::get().spawn_pause;
        assert!(b.idle_timer >= sp.0 && b.idle_timer <= sp.1);
        b.active = true;
        b.set_level(f.level);
        run(&mut b, &f, &far(), 0.1);
        assert_eq!(b.state, State::Idle, "not skipped on the first frame");
        run(&mut b, &f, &far(), sp.1 + 0.3);
        assert_eq!(b.state, State::Wander);
        assert!(b.wander_target.is_some());
    }

    #[test]
    fn wander_goals_are_points_of_interest_and_never_repeat_in_a_row() {
        let f = fac();
        let stands: Vec<(f32, f32)> = points_of_interest(&f, Rules::get()).iter().map(|p| p.stand).collect();
        let mut b = wanderer(&f, f.start.center());
        let mut last = None;
        let mut kinds = HashSet::new();
        for _ in 0..80 {
            b.state = State::Wander;
            let g = b.pick_wander_target(&f);
            assert!(stands.contains(&g), "goal {g:?} is not a point of interest");
            assert_ne!(Some(g), last);
            last = Some(g);
            kinds.insert(b.poi.as_ref().unwrap().kind.id());
            b.pos = g;
        }
        assert!(kinds.len() >= 2, "only {kinds:?}");
        assert!(b.recent_pois.len() <= Rules::get().memory);
    }

    #[test]
    fn an_activity_runs_for_its_duration_then_the_hunter_idles_for_a_short_pause() {
        let f = fac();
        let rules = Rules::get();
        let poi = points_of_interest(&f, rules).into_iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap();
        let mut b = wanderer(&f, poi.stand);
        b.start_activity(Activity::LookAround, &poi, &f);
        let (lo, hi) = rules.durations[&Activity::LookAround];
        let t = far();
        let first = b.facing;
        run(&mut b, &f, &t, lo * 0.5);
        let (kind, p) = b.activity_view().expect("still looking round");
        assert_eq!(kind, Activity::LookAround);
        assert!(p > 0.0 && p < 1.0);
        assert_eq!(b.state, State::Wander);
        assert!(dist(b.facing, first) > 0.01, "the head and body sweep");
        run(&mut b, &f, &t, hi + 0.2 - lo * 0.5);
        assert!(b.activity_view().is_none());
        assert_eq!(b.state, State::Idle);
        assert!(b.idle_timer > 0.0 && b.idle_timer <= rules.pause.1);
        run(&mut b, &f, &t, rules.pause.1 + 0.3);
        assert_eq!(b.state, State::Wander, "after the pause it picks the next goal");
    }

    #[test]
    fn listening_raises_hearing_and_the_end_restores_it() {
        let f = fac();
        let rules = Rules::get();
        let poi = points_of_interest(&f, rules).into_iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap();
        let mut b = wanderer(&f, poi.stand);
        let base = b.hearing_scale;
        b.start_activity(Activity::Listen, &poi, &f);
        assert!((b.hearing_scale - base * rules.listen_hearing_scale).abs() < 1e-4);
        run(&mut b, &f, &far(), rules.durations[&Activity::Listen].1 + 0.3);
        assert!((b.hearing_scale - base).abs() < 1e-4);
        // a chase that interrupts it restores the hearing too
        b.state = State::Wander;
        b.start_activity(Activity::Listen, &poi, &f);
        b.state = State::Chase;
        b.last_known = Some(poi.stand);
        run(&mut b, &f, &far(), 0.1);
        assert!((b.hearing_scale - base).abs() < 1e-4);
        assert!(b.activity_view().is_none());
    }

    #[test]
    fn inspecting_the_locker_a_runner_hides_in_finds_them() {
        let f = fac();
        assert!(!f.hiding.is_empty(), "the generated floor has lockers");
        let poi = points_of_interest(&f, Rules::get()).into_iter().find(|p| p.kind == PoiKind::Locker).unwrap();
        let i = poi.locker.unwrap();
        let hider = Target {
            pos: f.hiding[i].cell.center(),
            facing: (1.0, 0.0),
            hidden: true,
            hiding_spot: Some(i),
            ..Default::default()
        };
        let mut b = wanderer(&f, poi.stand);
        b.start_activity(Activity::InspectLocker, &poi, &f);
        let mut found = None;
        let prof = Difficulty::Standard.profile();
        let c = ctx(&f);
        for _ in 0..(4.5 * 60.0) as i32 {
            b.update(1.0 / 60.0, &c, &hider, 1, 0, &prof);
            if let Some(l) = b.take_locker_found() {
                found = Some(l);
                break;
            }
        }
        assert_eq!(found, Some(i));
        assert_eq!(b.state, State::Chase);
        assert!(b.activity_view().is_none());
    }

    #[test]
    fn inspecting_an_empty_locker_reports_the_check_and_finds_nobody() {
        let f = fac();
        let poi = points_of_interest(&f, Rules::get()).into_iter().find(|p| p.kind == PoiKind::Locker).unwrap();
        let mut b = wanderer(&f, poi.stand);
        b.start_activity(Activity::InspectLocker, &poi, &f);
        let prof = Difficulty::Standard.profile();
        let c = ctx(&f);
        let mut checked = None;
        for _ in 0..(4.5 * 60.0) as i32 {
            b.update(1.0 / 60.0, &c, &far(), 1, 0, &prof);
            checked = checked.or(b.take_locker_check());
            assert_eq!(b.take_locker_found(), None);
        }
        assert_eq!(checked, poi.locker, "the host gets the locker to resolve other hiders");
        assert_ne!(b.state, State::Chase);
    }

    /// B11: no point of interest: a short local wander to a reachable cell, not a goal on the hunter's own spot.
    #[test]
    fn b11_a_wander_with_no_points_of_interest_goes_somewhere_reachable() {
        let f = fac();
        let mut none = Rules::default();
        none.kinds.clear();
        let mut b = wanderer(&f, f.exit.center());
        let g = b.pick_wander_target_with(&f, &none);
        assert_eq!(b.state, State::Wander);
        assert!(dist(g, b.pos) >= 1.5, "the goal is away from the hunter");
        assert!(b.can_reach(&f, g));
        assert!(b.poi.is_none());
    }

    /// B11: boxed in with nothing to go to, the hunter idles instead of re-picking its own spot every frame.
    #[test]
    fn b11_a_boxed_in_wanderer_idles_instead_of_looping_on_its_own_spot() {
        let mut f = fac();
        let pocket = seal_a_pocket(&mut f, Cell::new(-100, -100));
        let mut none = Rules::default();
        none.kinds.clear();
        let mut b = wanderer(&f, pocket);
        let g = b.pick_wander_target_with(&f, &none);
        if dist(g, b.pos) < 1.5 {
            assert_eq!(b.state, State::Idle);
            assert!(b.idle_timer > 0.0);
        } else {
            // the exit is farther than 5 m and was the answer, as before
            assert_eq!(g, f.exit.center());
        }
    }

    /// B12: the last-known point is walled off, so the search goes to the nearest cell that can be reached.
    #[test]
    fn b12_a_search_on_an_unreachable_point_goes_to_the_nearest_reachable_cell() {
        let mut f = fac();
        let start = f.start;
        let pocket = seal_a_pocket(&mut f, start);
        let mut b = Brain::new(aki(), f.start.center(), "ACT", Difficulty::Standard);
        b.active = true;
        b.set_level(f.level);
        b.last_known = Some(pocket);
        let t = b.nearest_reachable_search_point(&f).expect("a reachable cell lies near the pocket");
        assert!(dist(t, b.pos) > 0.5);
        assert!(b.can_reach(&f, t));
        assert!(dist(t, pocket) <= 10.0);
        b.search_timer = 9.0;
        let _ = b.pick_search_target(&f);
        assert_eq!(b.search_timer, 9.0, "a search with somewhere to go keeps its time");
    }

    /// B12: nothing reachable at all (the hunter itself is sealed in): the search ends rather than standing in place.
    #[test]
    fn b12_a_search_with_nowhere_to_go_ends_instead_of_standing_still() {
        let mut f = fac();
        let pocket = seal_a_pocket(&mut f, Cell::new(-100, -100));
        let mut b = Brain::new(aki(), pocket, "ACT", Difficulty::Standard);
        b.active = true;
        b.set_level(f.level);
        b.last_known = Some(pocket);
        b.search_timer = 9.0;
        let _ = b.pick_search_target(&f);
        assert_eq!(b.search_timer, 0.0);
    }
    #[test]
    fn idle_life_activities_keep_the_hunter_on_ground_a_body_fits() {
        let f = fac();
        let pois = points_of_interest(&f, Rules::get());
        for kind in [Activity::Fidget, Activity::Pace, Activity::CornerPeek] {
            for poi in pois.iter().filter(|p| p.kind == PoiKind::RoomCentre).take(6) {
                let mut b = wanderer(&f, poi.stand);
                b.start_activity(kind, poi, &f);
                let prof = Difficulty::Standard.profile();
                let c = ctx(&f);
                let t = far();
                for _ in 0..(Rules::get().durations[&kind].1 * 60.0) as i32 + 30 {
                    b.update(1.0 / 60.0, &c, &t, 1, 0, &prof);
                    assert!(f.body_fits(b.pos.0, b.pos.1, HUNTER_RADIUS), "{} left open ground", kind.name());
                }
                assert!(b.activity_view().is_none(), "{} ends", kind.name());
            }
        }
    }

    #[test]
    fn a_chase_interrupts_an_idle_life_activity_at_once() {
        let f = fac();
        let poi = points_of_interest(&f, Rules::get()).into_iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap();
        for kind in [Activity::Fidget, Activity::Pace, Activity::CornerPeek] {
            let mut b = wanderer(&f, poi.stand);
            b.start_activity(kind, &poi, &f);
            run(&mut b, &f, &far(), 0.3);
            assert!(b.activity_view().is_some());
            b.state = State::Chase;
            b.last_known = Some(poi.stand);
            run(&mut b, &f, &far(), 1.0 / 60.0);
            assert!(b.activity_view().is_none(), "{} dropped on the first chase frame", kind.name());
        }
    }

    #[test]
    fn a_head_turn_holds_the_hunter_still_then_investigates_and_a_chase_cancels_it() {
        let f = fac();
        let pois = points_of_interest(&f, Rules::get());
        let here = pois.iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap();
        let there = pois.iter().rev().find(|p| dist(p.stand, here.stand) > 6.0).unwrap();
        let idle = IdleRules::get();
        let mut b = wanderer(&f, here.stand);
        b.state = State::Idle;
        b.idle_timer = 5.0;
        b.last_known = Some(there.stand);
        assert!(b.idle_life.begin_turn(idle, "steady", 0.0, 0.0));
        run(&mut b, &f, &far(), idle.head_turn_time.0 * 0.5);
        assert_eq!(b.state, State::Idle, "still turning");
        assert!(dist(b.pos, here.stand) < 0.01, "does not walk while turning");
        run(&mut b, &f, &far(), idle.head_turn_time.1 + 0.2);
        assert!(!b.idle_life.turning());
        assert_ne!(b.state, State::Idle, "it goes to look once it has turned");
        // a chase mid-turn drops the turn
        let mut c = wanderer(&f, here.stand);
        c.state = State::Idle;
        c.idle_timer = 5.0;
        assert!(c.idle_life.begin_turn(idle, "steady", 0.0, 1.0));
        c.state = State::Chase;
        c.last_known = Some(there.stand);
        run(&mut c, &f, &far(), 1.0 / 60.0);
        assert!(!c.idle_life.turning());
    }

    #[test]
    fn a_stale_head_turn_does_not_make_a_wanderer_deaf() {
        let f = fac();
        let pois = points_of_interest(&f, Rules::get());
        let here = pois.iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap();
        let idle = IdleRules::get();
        let mut b = wanderer(&f, here.stand);
        b.state = State::Idle;
        b.idle_timer = 5.0;
        assert!(b.idle_life.begin_turn(idle, "steady", 0.0, 1.0));
        // something other than a chase (the director, a gimmick) sets a wander mid-turn
        b.state = State::Wander;
        run(&mut b, &f, &far(), 1.0 / 60.0);
        assert!(!b.idle_life.turning(), "a turn outside an Idle pause is dropped");
        // a noise it hears and cannot see: in line of sight, beyond its (shrunk) sight, well inside its hearing
        b.def.sight = 0.05;
        let eye = b.sight_eye();
        let spot = (3..=6)
            .flat_map(|d| {
                let d = d as f32;
                [(d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)]
            })
            .map(|(dx, dy)| (b.pos.0 + dx, b.pos.1 + dy))
            .find(|&p| f.body_fits(p.0, p.1, HUNTER_RADIUS) && f.has_line_of_sight(eye, p))
            .expect("open ground in sight of a room centre");
        let noise = Target { pos: spot, facing: (1.0, 0.0), noise: 40.0, flashlight_on: false, ..Default::default() };
        let mut heard = false;
        // it may turn its head to the noise first (a fresh turn), then it goes to look
        for _ in 0..((idle.head_turn_time.1 + 0.6) * 60.0) as i32 {
            run(&mut b, &f, &noise, 1.0 / 60.0);
            if b.state == State::Investigate {
                assert!(!b.idle_life.turning());
                heard = true;
                break;
            }
        }
        assert!(heard, "the noise is investigated, not ignored (state {:?})", b.state);
    }

    // -- hard abilities (hunters/hard_abilities.rs) -----------------------------------------------------------------

    use crate::hunters::gimmicks::nyxie;
    use crate::hunters::hard_abilities::{self, Ability, Budget, HardAbilities, PounceCtx};

    /// A hunter spot and a runner spot three cells east of it on open ground: in sight, the straight sweep clear and
    /// no link near either end, so every pounce gate but the hunter's own and the budget passes.
    fn pounce_lane(f: &Facility) -> ((f32, f32), (f32, f32)) {
        let probe = Brain::new(aki(), (0.0, 0.0), "LANE", Difficulty::Hard);
        let clear = hard_abilities::rules().leap.link_clearance + 0.5;
        for y in 1..f.rows - 1 {
            for x in 1..f.cols - 4 {
                let h = Cell::new(x, y).center();
                let t = (h.0 + 3.0, h.1);
                let near_link = f.stairs.iter().any(|s| {
                    let c = s.cell.center();
                    dist(c, h) <= clear || dist(c, t) <= clear
                }) || f.drop_cells().iter().any(|d| {
                    let c = d.center();
                    dist(c, h) <= clear || dist(c, t) <= clear
                });
                if !near_link
                    && f.body_fits(h.0, h.1, HUNTER_RADIUS)
                    && f.body_fits(t.0, t.1, HUNTER_RADIUS)
                    && f.has_line_of_sight(h, t)
                    && probe.clear_sweep(f, h, t)
                {
                    return (h, t);
                }
            }
        }
        panic!("the generated floor has a three-cell straight");
    }

    /// A host brain of `id` on `diff`, chasing a runner it sees at `runner`.
    fn hard_hunter(f: &Facility, id: &str, diff: Difficulty, at: (f32, f32), runner: (f32, f32)) -> Brain {
        let mut b = Brain::new(aki(), at, "HARD", diff);
        b.def.id = id.into();
        b.hard = HardAbilities::new(diff, id);
        assert!(b.hard.is_some(), "{id} has a hard ability on {diff:?}");
        b.active = true;
        b.set_level(f.level);
        b.state = State::Chase;
        b.last_known = Some(runner);
        b.direct_visible = true;
        b
    }

    /// The runner in the dark (Nyxie's gate) and warm (Solyth's gate).
    fn dark_runner(p: (f32, f32)) -> Target {
        Target { pos: p, facing: (1.0, 0.0), flashlight_on: false, temperature: 70.0, ..Default::default() }
    }

    /// Forces a hard pounce at `t` (roll 0, every gate open) and returns the leap direction.
    fn force_pounce(b: &mut Brain, h: (f32, f32), t: (f32, f32)) -> (f32, f32) {
        let ctx = PounceCtx {
            hunter: h,
            target: t,
            runner: 0,
            chasing: true,
            visible: true,
            stunned: false,
            target_protected: false,
            near_link: false,
            sweep_clear: true,
            flashlight_on: false,
            temperature: 70.0,
            hunter_in_room: false,
            roll: 0.0,
            dt: 1.0,
            now: 0.0,
        };
        let hard = b.hard.as_mut().unwrap();
        assert!(hard.try_pounce(&ctx, &mut Budget::default()));
        hard.leap_dir
    }

    // The frames below use dt 3 so rate * dt is above 1 and the pounce roll always passes.
    const SURE_DT: f32 = 3.0;

    #[test]
    fn nightmare_nyxie_and_solyth_pounce_only_through_their_gimmick() {
        let f = fac();
        let (h, t) = pounce_lane(&f);
        let runner = dark_runner(t);
        assert!(nyxie::pounce().applies(Difficulty::Nightmare));
        for id in ["nyxie", "solyth"] {
            assert!(hard_abilities::rules().has(Ability::Pounce, Difficulty::Nightmare, id), "the shared roster still lists {id}");
            let mut b = hard_hunter(&f, id, Difficulty::Nightmare, h, t);
            let g = Gimmick::for_id(id, Difficulty::Nightmare);
            for _ in 0..20 {
                b.pos = h;
                b.stun = 0.0;
                b.state = State::Chase;
                b.direct_visible = true;
                b.hard_frame(SURE_DT, &f, &runner, &g, false);
                let hard = b.hard.as_ref().unwrap();
                assert!(!hard.leaping && hard.leap_seq == 0, "{id} got a second, hard-ability pounce on Nightmare");
            }
        }
        // the same lane and frame do launch the hard pounce of a hunter with no gimmick pounce
        let mut control = hard_hunter(&f, "haelith", Difficulty::Nightmare, h, t);
        control.hard_frame(SURE_DT, &f, &runner, &Gimmick::for_id("aki", Difficulty::Nightmare), false);
        assert_eq!(control.hard.as_ref().unwrap().leap_seq, 1, "the lane is a pounce lane");
    }

    #[test]
    fn hard_solyth_keeps_her_hard_pounce() {
        assert!(!nyxie::pounce().applies(Difficulty::Hard), "the gimmick pounce is Nightmare only");
        let f = fac();
        let (h, t) = pounce_lane(&f);
        let mut b = hard_hunter(&f, "solyth", Difficulty::Hard, h, t);
        b.hard_frame(SURE_DT, &f, &dark_runner(t), &Gimmick::for_id("solyth", Difficulty::Hard), false);
        assert_eq!(b.hard.as_ref().unwrap().leap_seq, 1, "Hard Solyth starts a hard pounce");
    }

    #[test]
    fn a_drop_between_hunter_and_runner_refuses_the_pounce() {
        // Facility::fits lets a body enter a drop and the straight sweep never blocks over one, so without a
        // drop-aware link guard a pounce would launch the hunter out over a waterfall void. The near_link gate
        // must reject a pounce whose line straddles a drop, exactly as it does a stair flight.
        let mut f = fac();
        let (h, t) = pounce_lane(&f);
        // one cell east of the hunter, on the lane between the two ends (center 1.0 cell from h, inside link_clearance)
        let between = Cell::new((h.0 + 1.0) as i32, h.1 as i32);
        f.set_drop_cells(&[between]);
        let mut b = hard_hunter(&f, "solyth", Difficulty::Hard, h, t);
        for _ in 0..20 {
            b.pos = h;
            b.stun = 0.0;
            b.state = State::Chase;
            b.direct_visible = true;
            b.hard_frame(SURE_DT, &f, &dark_runner(t), &Gimmick::for_id("solyth", Difficulty::Hard), false);
            let hard = b.hard.as_ref().unwrap();
            assert!(!hard.leaping && hard.leap_seq == 0, "a pounce launched across a drop that lies between the hunter and the runner");
        }
    }

    #[test]
    fn a_stun_ends_a_hard_leap_for_good() {
        let f = fac();
        let (h, t) = pounce_lane(&f);
        let mut b = hard_hunter(&f, "haelith", Difficulty::Hard, h, t);
        let dir = force_pounce(&mut b, h, t);
        b.apply_stun(1.0, false);
        let hard = b.hard.as_ref().unwrap();
        assert!(!hard.leaping, "the stun ends the leap");
        assert!(hard.pounce_cooldown() > 0.0, "and starts the pounce cooldown");
        // stand it in a long Idle pause, far from any runner, and let the stun run out
        b.state = State::Idle;
        b.idle_timer = 30.0;
        b.direct_visible = false;
        let start = b.pos;
        run(&mut b, &f, &far(), 1.5);
        assert_eq!(b.stun, 0.0);
        let along = (b.pos.0 - start.0) * dir.0 + (b.pos.1 - start.1) * dir.1;
        assert!(along.abs() < 0.3, "no jump along the old leap line once free: {along}");
        assert!(!b.hard.as_ref().unwrap().leaping);
    }

    #[test]
    fn a_hard_leap_shows_the_pounce_mechanic() {
        let f = fac();
        let (h, t) = pounce_lane(&f);
        let mut b = hard_hunter(&f, "haelith", Difficulty::Hard, h, t);
        assert_ne!(b.mechanic(), Some("pounce"));
        force_pounce(&mut b, h, t);
        assert_eq!(b.mechanic(), Some("pounce"), "the host frame carries the leap to the guests");
        b.hard.as_mut().unwrap().cancel();
        assert_ne!(b.mechanic(), Some("pounce"));
    }

    #[test]
    fn a_pounce_drops_a_running_activity_on_its_first_frame() {
        // a hunter mid-Listen (its hearing boosted) flips to a chase and pounces: the leap skips update_movement,
        // so the state-leave cleanup must still run in update() and drop the stale activity and its hearing boost,
        // or they would linger across the leap and perturb perception until it ends.
        let f = fac();
        let (h, t) = pounce_lane(&f);
        let mut b = hard_hunter(&f, "haelith", Difficulty::Hard, h, t);
        let poi = points_of_interest(&f, Rules::get()).into_iter().find(|p| p.kind == PoiKind::RoomCentre).unwrap();
        let base = b.hearing_scale;
        b.start_activity(Activity::Listen, &poi, &f);
        b.state = State::Chase; // the chase takes over in the same frame the pounce fires
        assert!(b.hearing_scale > base, "Listen raises the hearing");
        // launch the leap, then run one brain frame: hard_frame reports leaping, so update_movement is skipped
        force_pounce(&mut b, h, t);
        assert!(b.activity_view().is_some(), "the activity still lingers the instant the leap starts");
        let prof = Difficulty::Hard.profile();
        let mut c = ctx(&f);
        c.diff = Difficulty::Hard;
        b.update(1.0 / 60.0, &c, &dark_runner(t), 1, 0, &prof);
        assert!(b.hard.as_ref().unwrap().leaping, "still mid-leap, so update_movement was skipped this frame");
        assert!(b.activity_view().is_none(), "the leap's first frame drops the stale activity");
        assert!((b.hearing_scale - base).abs() < 1e-4, "and restores the hearing boost: {}", b.hearing_scale);
    }

    #[test]
    fn host_brains_sharing_one_budget_space_their_pounces() {
        let f = fac();
        let (h, t) = pounce_lane(&f);
        let runner = dark_runner(t);
        let g = Gimmick::for_id("aki", Difficulty::Hard);
        let gap = hard_abilities::rules().budget(Difficulty::Hard);
        assert!(gap.is_finite() && gap > SURE_DT, "the Hard party gap is longer than one test frame: {gap}");
        let shared = Arc::new(std::sync::Mutex::new(Budget::default()));
        let mut a = hard_hunter(&f, "haelith", Difficulty::Hard, h, t);
        let mut b = hard_hunter(&f, "xivvy", Difficulty::Hard, h, t);
        for x in [&mut a, &mut b] {
            x.hard_budget = Arc::clone(&shared);
            x.hard_clock = 100.0;
        }
        a.hard_frame(SURE_DT, &f, &runner, &g, false);
        assert_eq!(a.hard.as_ref().unwrap().leap_seq, 1, "the first hunter pounces");
        let fired = a.hard_clock;
        // every frame of the second hunter that lands inside the gap is refused
        while b.hard_clock + SURE_DT - fired < gap {
            b.pos = h;
            b.hard_frame(SURE_DT, &f, &runner, &g, false);
            assert_eq!(b.hard.as_ref().unwrap().leap_seq, 0, "inside the party gap at {}", b.hard_clock);
        }
        // a fresh budget (a new floor) allows a pounce again
        b.hard_budget = Arc::new(std::sync::Mutex::new(Budget::default()));
        b.pos = h;
        b.hard_frame(SURE_DT, &f, &runner, &g, false);
        assert_eq!(b.hard.as_ref().unwrap().leap_seq, 1, "a fresh budget allows a pounce");
    }
}
