//! The shared hunter brain (game/hunters/ai/hunter_brain.gd): beam attention, perception
//! (sight + hearing), stamina, the camp guard and the idle / wander / investigate / chase /
//! search state machine, with each hunter's gimmick hooked in at the original's points.
//! Positions are grid cells (1 cell = 1 m).

use super::gimmicks::{Ctx, Event, Gimmick};
use super::peek::{self, Corner, Style};
use crate::config::*;
use crate::content::hunters::HunterDef;
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

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
}

/// LightField.unlit_scale: a runner in pitch dark is seen close, one in a lamp pool far.
fn light_unlit_scale(exposure: f32) -> f32 {
    let t = exposure.clamp(0.0, 1.0);
    LIGHT_DARK_SIGHT_SCALE + (LIGHT_LIT_SIGHT_SCALE - LIGHT_DARK_SIGHT_SCALE) * t
}

pub fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn cell_of(p: (f32, f32)) -> Cell {
    Cell::new(p.0.floor() as i32, p.1.floor() as i32)
}

/// Whether a body of HUNTER_RADIUS can sweep the straight line from `from` to `to` (sampled every 0.15 m).
fn sweep_clear(fac: &Facility, from: (f32, f32), to: (f32, f32)) -> bool {
    let d = dist(from, to);
    let steps = ((d / 0.15).ceil() as i32).max(1);
    (1..=steps).all(|i| {
        let t = i as f32 / steps as f32;
        fac.body_fits(
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t,
            HUNTER_RADIUS,
        )
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

/// A route over the hunter grid from the cell under `from` to `to`, as `Facility::find_path` plans it, except that the
/// cells in `avoid` (ones a step has just been blocked by) are left out. A closed start cell (the body stands
/// off-centre in it) starts from the nearest open cell beside it.
fn plan_route(fac: &Facility, from: (f32, f32), to: Cell, avoid: &[(Cell, f32)]) -> Vec<Cell> {
    let avoided = |c: Cell| avoid.iter().any(|(a, _)| *a == c);
    let open = |c: Cell| fac.hunter_open(c) && !avoided(c);
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
    if !fac.hunter_open(to) || avoided(to) {
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
                let ng = gc + if dx != 0 && dy != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
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
}

const ARRIVAL_DISTANCE: f32 = 0.65;
const WAYPOINT_DISTANCE: f32 = 0.18;
/// How long a cell a step was blocked by stays out of a hunter's route.
const BLOCKED_AVOID_SECONDS: f32 = 2.0;
const BEAM_ATTENTION_TIME: f32 = 0.35;
const BEAM_ATTENTION_DECAY: f32 = 1.6;
const BEAM_ATTENTION_RANGE: f32 = 12.0;
const BEAM_ATTENTION_HALF_ANGLE: f32 = 0.28;
/// LightField (game/look/light_field.gd): the exposure a hunter needs to make out a lit runner, and the sight
/// scale of a runner in pitch dark and in a lamp pool.
const LIGHT_VISIBLE_EXPOSURE: f32 = 0.36;
const LIGHT_DARK_SIGHT_SCALE: f32 = 0.25;
const LIGHT_LIT_SIGHT_SCALE: f32 = 1.6;

impl Brain {
    pub fn new(def: HunterDef, spawn: (f32, f32), seed: &str, diff: Difficulty) -> Brain {
        let gimmick = Gimmick::for_id(&def.id, diff);
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

    pub fn clear_path(&mut self) {
        self.path.clear();
        self.path_goal = None;
    }

    pub fn apply_stun(&mut self, s: f32, heavy: bool) {
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
        self.consider_peek(ctx.fac, &g, holds);
        let speed_scale = g.speed_scale(self);
        self.update_movement(dt, ctx.fac, depth, relays, prof, holds, speed_scale);
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
            && dist(self.pos, target.pos) < self.capture_reach(prof);
        (caught, events)
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
        if self.sense_timer > 0.0 {
            if self.state == State::Chase && can_sense {
                self.last_known = Some(target.pos);
            }
            return;
        }
        self.sense_timer = HUNTER_SENSE_INTERVAL;
        let sight_limit = self.sight_limit(target.flashlight_on, prof, g);
        let hearing_limit = target.noise * self.hearing_scale * prof.hunter_hearing;
        let mut sees = false;
        let mut line_clear = false;
        if can_sense {
            let ray = sight_limit.max(hearing_limit * HEARING_OPEN_AIR_SCALE);
            if direct < ray {
                line_clear = fac.has_line_of_sight(eye, target.pos);
            }
            sees = direct < sight_limit && line_clear;
        }
        let mut hears = false;
        if !sees && can_sense {
            if line_clear && direct <= hearing_limit * HEARING_OPEN_AIR_SCALE {
                hears = true;
            } else if direct < hearing_limit * HEARING_OUTER_SCALE {
                let p = plan_route(fac, self.pos, cell_of(target.pos), &self.avoid);
                hears = !p.is_empty() && p.len() <= hearing_limit.ceil() as usize;
                if hears && self.state == State::Chase {
                    self.path = p;
                    self.path_timer = HUNTER_REPATH_CHASE;
                    self.path_goal = Some(cell_of(target.pos));
                }
            }
        }
        if self.state == State::Peek {
            self.peek_perceive(sees, hears, target.pos);
            return;
        }
        if sees {
            self.state = State::Chase;
            self.last_known = Some(target.pos);
            self.lost_sight = 0.0;
            self.direct_visible = true;
        } else if hears {
            self.last_known = Some(target.pos);
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
        let prog = relays.max(0) as f32 * HUNTER_RELAY_SPEED_BONUS + depth_boost;
        let mut speed = if self.state == State::Chase {
            self.def.chase + prog * self.def.prog
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
        speed * prof.hunter_speed
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
        if dist(self.pos, goal) <= ARRIVAL_DISTANCE {
            self.handle_arrival(fac, prof);
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
        sweep_clear(fac, from, to)
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
            if fac.body_fits(self.pos.0 + sx, self.pos.1, HUNTER_RADIUS) {
                self.pos.0 += sx;
            }
            if fac.body_fits(self.pos.0, self.pos.1 + sy, HUNTER_RADIUS) {
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

    fn pursue(&mut self, goal: (f32, f32), speed: f32, dt: f32, fac: &Facility, prof: &Profile) {
        let target_cell = cell_of(goal);
        self.path_timer -= dt;
        for a in self.avoid.iter_mut() {
            a.1 -= dt;
        }
        self.avoid.retain(|a| a.1 > 0.0);
        if self.state == State::Chase
            && self.direct_visible
            && self.clear_sweep(fac, self.pos, goal)
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
        if self.path_timer <= 0.0 || self.path.is_empty() || self.path_goal.is_none() {
            self.path = plan_route(fac, self.pos, target_cell, &self.avoid);
            self.path_timer = interval;
            self.path_goal = Some(target_cell);
        }
        if self.path.is_empty() && cell_of(self.pos) != target_cell {
            self.unreachable += dt;
            if self.unreachable >= UNREACHABLE_GIVE_UP {
                self.unreachable = 0.0;
                match self.state {
                    State::Chase | State::Investigate => {
                        self.last_known = None;
                        self.state = State::Search;
                        self.search_timer = prof.search_time;
                        self.search_visits = 0;
                        self.search_target = Some(self.pick_search_target(fac));
                    }
                    State::Search => self.search_target = Some(self.pick_search_target(fac)),
                    _ => {}
                }
                self.path_goal = None;
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
        let mut waypoint = goal;
        if on_path {
            let mut furthest = 0;
            waypoint = self.path[0].center();
            for i in 0..self.path.len().min(4) {
                let c = self.path[i].center();
                if !fac.body_fits(c.0, c.1, HUNTER_RADIUS) || !self.clear_sweep(fac, self.pos, c) {
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
            && !sweep_clear(fac, self.pos, next)
        {
            // The step into the next cell is blocked: do not slide along the wall. Keep that cell out of the route for
            // a moment and plan again from where the body stands. (Off-centre in its own cell the body walks to the
            // centre, as before; the goal's own cell is never kept out, the body closes on it as far as it can.)
            self.avoid.push((blocked_cell, BLOCKED_AVOID_SECONDS));
            self.clear_path();
            self.path_timer = 0.0;
            self.moving_speed = 0.0;
            return;
        }
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
        if dist(self.pos, spot) > 1.0 || !fac.body_fits(spot.0, spot.1, HUNTER_RADIUS) {
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

    pub fn pick_search_target(&mut self, fac: &Facility) -> (f32, f32) {
        let lk = self.last_known.unwrap_or(self.pos);
        let mut offs = [
            (3.0, 0.0),
            (-3.0, 0.0),
            (0.0, 3.0),
            (0.0, -3.0),
            (2.0, 2.0),
            (-2.0, 2.0),
            (2.0, -2.0),
            (-2.0, -2.0),
        ];
        self.rng.shuffle(&mut offs);
        for (ox, oy) in offs {
            if let Some(c) = fac.nearest_open(lk.0 + ox, lk.1 + oy, HUNTER_RADIUS) {
                if dist(c, self.pos) > 0.5
                    && !plan_route(fac, self.pos, cell_of(c), &self.avoid).is_empty()
                {
                    return c;
                }
            }
        }
        self.pos
    }
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
        assert!((b.current_speed(1, 0, &prof, 1.0) - 3.55).abs() < 1e-5);
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
            exposure: 0.0,
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
        assert!(walked <= 3.56 && walked >= 3.4, "walked {walked} cells in one second");

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
        assert!((dashed / (18.0 * dt) - 4.26).abs() < 0.02, "dashed {dashed} cells in 0.3 s");
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
            exposure: 0.0,
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
}
