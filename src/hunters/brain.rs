//! The shared hunter brain (game/hunters/ai/hunter_brain.gd): beam attention, perception
//! (sight + hearing), stamina, the camp guard and the idle / wander / investigate / chase /
//! search state machine, with each hunter's gimmick hooked in at the original's points.
//! Positions are grid cells (1 cell = 1 m).

use super::gimmicks::{Ctx, Event, Gimmick};
use crate::config::*;
use crate::content::hunters::HunterDef;
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Wander,
    Investigate,
    Chase,
    Search,
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
}

pub fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn cell_of(p: (f32, f32)) -> Cell {
    Cell::new(p.0.floor() as i32, p.1.floor() as i32)
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
}

const ARRIVAL_DISTANCE: f32 = 0.65;
const WAYPOINT_DISTANCE: f32 = 0.18;
const BEAM_ATTENTION_TIME: f32 = 0.35;
const BEAM_ATTENTION_DECAY: f32 = 1.6;
const BEAM_ATTENTION_RANGE: f32 = 12.0;
const BEAM_ATTENTION_HALF_ANGLE: f32 = 0.28;

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

    pub fn apply_slow(&mut self, seconds: f32, factor: f32) {
        self.slow_timer = self.slow_timer.max(seconds);
        self.slow_factor = factor;
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
        if self.slow_timer > 0.0 {
            self.slow_timer = (self.slow_timer - dt).max(0.0);
        }
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
        let limit = if flashlight_on {
            FLASHLIGHT_SIGHT_MIN.max(s * FLASHLIGHT_SIGHT_SCALE) * prof.hunter_sight
        } else {
            s * UNLIT_SIGHT_SCALE * prof.hunter_sight
        };
        limit * g.sight_scale()
    }

    fn perceive(&mut self, dt: f32, fac: &Facility, target: &Target, prof: &Profile, g: &Gimmick) {
        let direct = dist(self.pos, target.pos);
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
                line_clear = fac.has_line_of_sight(self.pos, target.pos);
            }
            sees = direct < sight_limit && line_clear;
        }
        let mut hears = false;
        if !sees && can_sense {
            if line_clear && direct <= hearing_limit * HEARING_OPEN_AIR_SCALE {
                hears = true;
            } else if direct < hearing_limit * HEARING_OUTER_SCALE {
                let p = fac.find_path(cell_of(self.pos), cell_of(target.pos));
                hears = !p.is_empty() && p.len() <= hearing_limit.ceil() as usize;
                if hears && self.state == State::Chase {
                    self.path = p;
                    self.path_timer = HUNTER_REPATH_CHASE;
                    self.path_goal = Some(cell_of(target.pos));
                }
            }
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
            State::Idle => None,
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
            State::Idle => {}
        }
    }

    fn clear_sweep(&self, fac: &Facility, from: (f32, f32), to: (f32, f32)) -> bool {
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
            self.path = fac.find_path(cell_of(self.pos), target_cell);
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
        let mut waypoint = goal;
        if !self.path.is_empty() {
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
        let moved = self.step_toward(waypoint, speed * dt, fac, dt);
        if moved <= 0.001 && !self.path.is_empty() {
            self.path_timer = 0.0;
            if self.path.len() > 1 {
                self.path.remove(0);
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
                    && !fac.find_path(cell_of(self.pos), cell_of(c)).is_empty()
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
            serde_json::from_str(include_str!("../../data/hunters/aki.json")).unwrap();
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
}
