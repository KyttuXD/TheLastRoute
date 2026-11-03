//! A bot that plays a run (after dev/playtest/bots/last_route_bot.gd): it reads the game the
//! way the bridge's observation does and answers with the same controls a player has. Goal
//! order: escape a capture, stay hidden while a threat is near, run from a threat (safe zone,
//! then a locker, then 25 m away), recover relays, fetch a battery when the lamp is low, take
//! tools, ride the lift (finite) or the stairs (infinite) once the floor is done, else explore
//! the least-visited part of the map. Navigation is A* on the hunters' grid with a look-ahead
//! steer; stuck and soft-lock recovery escalate as in the original.

use super::sim::{r2, Sim};
use crate::config::*;
use crate::game::{pdist, Controls, Focus, Game};
use crate::hunters::brain::State;
use crate::world::rng::Rng;
use crate::world::Cell;
use macroquad::math::vec2;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Safe,
    Hide(usize),
    Flee,
    Relay(usize),
    Battery(usize),
    Pickup(usize),
    Lift,
    Stairs(usize),
    Explore,
    Forced,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Safe => "safe",
            Kind::Hide(_) => "hide",
            Kind::Flee => "flee",
            Kind::Relay(_) => "relay",
            Kind::Battery(_) => "battery",
            Kind::Pickup(_) => "use",
            Kind::Lift => "lift",
            Kind::Stairs(_) => "level",
            Kind::Explore => "explore",
            Kind::Forced => "forced",
        }
    }

    fn threat_goal(self) -> bool {
        matches!(self, Kind::Safe | Kind::Hide(_) | Kind::Flee)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Goal {
    pub kind: Kind,
    /// Target point (window cells).
    pub at: (f32, f32),
    /// The focus that completes it with [E] (None: arrive only).
    pub focus: Option<Focus>,
    pub since: f32,
}

#[derive(Default, Clone)]
pub struct BotStats {
    pub frames: u64,
    pub distance_m: f32,
    pub stuck_events: i32,
    pub soft_locks: i32,
    pub interact_failures: i32,
    pub unreachable_goals: i32,
    pub chase_frames: u64,
    pub relays: i32,
    pub deaths: i32,
    pub hidden_frames: u64,
    pub chunks_visited: usize,
    pub max_depth: i32,
    pub hunters_seen: Map<String, Value>,
    pub uses: Map<String, Value>,
    pub flashlight_toggles: i32,
    pub levels: HashMap<i32, f32>,
    pub level_trips: i32,
    pub idle_max_s: f32,
    pub idle_recoveries: i32,
    pub escapes: i32,
    pub hides: i32,
    pub relays_camped: i32,
    pub jumps: i32,
}

pub struct Bot {
    rng: Rng,
    pub stats: BotStats,
    pub events: Vec<Value>,
    pub goal: Option<Goal>,
    /// A goal set from outside (goto / goto_check); kept until reached or cleared.
    pub forced: Option<(f32, f32)>,
    /// How close counts as arriving at orced.
    pub forced_reach: f32,
    pub debug: bool,
    path: Vec<Cell>,
    path_age: f32,
    path_goal: Option<Cell>,
    blacklist: HashMap<(u8, i32, i32), f32>,
    interact_cd: f32,
    interact_tries: i32,
    time: f32,
    stuck_ref: ((f32, f32), f32),
    stuck_level: i32,
    stuck_since: Option<f32>,
    stuck_next: f32,
    strafe_until: f32,
    strafe_dir: f32,
    hidden_clear: f32,
    flash_cd: f32,
    level_since: f32,
    level_budget: f32,
    last_level: Option<i32>,
    window_center: Option<(i32, i32)>,
    visits: HashMap<(i32, i32), i32>,
    visit_t: f32,
    last_pos: (f32, f32),
    idle_t: f32,
    last_relays: i32,
    pick_t: f32,
    safe_wait: f32,
    dead: bool,
    hidden_for: f32,
}

fn wrap(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    if a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// The yaw that faces (dx, dy) on the grid: forward is (-sin yaw, -cos yaw).
pub fn yaw_to(dx: f32, dy: f32) -> f32 {
    (-dx).atan2(-dy)
}

fn cell_of(p: (f32, f32)) -> Cell {
    Cell::new(p.0.floor() as i32, p.1.floor() as i32)
}

/// The nearest cell the hunters' grid (the A* grid) treats as open, within `r` rings.
pub fn open_cell_near(g: &Game, c: Cell, r: i32) -> Option<Cell> {
    if g.fac.hunter_open(c) {
        return Some(c);
    }
    for ring in 1..=r {
        let mut best: Option<(i32, Cell)> = None;
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let n = Cell::new(c.x + dx, c.y + dy);
                if g.fac.hunter_open(n) {
                    let d = dx * dx + dy * dy;
                    if best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, n));
                    }
                }
            }
        }
        if let Some((_, n)) = best {
            return Some(n);
        }
    }
    None
}

/// A* from the runner to `to` on the hunters' grid (both ends moved to the nearest open cell).
pub fn path_to(g: &Game, to: (f32, f32)) -> Vec<Cell> {
    let Some(a) = open_cell_near(g, cell_of(g.p.pos), 3) else {
        return vec![];
    };
    let Some(b) = open_cell_near(g, cell_of(to), 3) else {
        return vec![];
    };
    if a == b {
        return vec![a];
    }
    g.fac.find_path(a, b)
}

impl Bot {
    pub fn new(seed: &str, bot_seed: u32) -> Bot {
        Bot {
            rng: Rng::new(&format!("{seed}|{bot_seed}")),
            stats: BotStats::default(),
            events: vec![],
            goal: None,
            forced: None,
            forced_reach: 1.6,
            debug: true,
            path: vec![],
            path_age: 0.0,
            path_goal: None,
            blacklist: HashMap::new(),
            interact_cd: 0.0,
            interact_tries: 0,
            time: 0.0,
            stuck_ref: ((0.0, 0.0), 0.0),
            stuck_level: 0,
            stuck_since: None,
            stuck_next: 0.0,
            strafe_until: 0.0,
            strafe_dir: 1.0,
            hidden_clear: 0.0,
            flash_cd: 0.0,
            level_since: 0.0,
            level_budget: 200.0,
            last_level: None,
            window_center: None,
            visits: HashMap::new(),
            visit_t: 0.0,
            last_pos: (0.0, 0.0),
            idle_t: 0.0,
            last_relays: 0,
            pick_t: 0.0,
            safe_wait: 0.0,
            dead: false,
            hidden_for: 0.0,
        }
    }

    fn event(&mut self, frame: u64, kind: &str, data: Value) {
        let mut e = json!({ "kind": kind, "frame": frame });
        if let (Some(o), Value::Object(d)) = (e.as_object_mut(), data) {
            for (k, v) in d {
                o.insert(k, v);
            }
        }
        self.events.push(e);
        if self.events.len() > 4000 {
            self.events.drain(..1000);
        }
    }

    fn abs_key(g: &Game, kind: u8, c: Cell) -> (u8, i32, i32) {
        (kind, c.x + g.fac.origin.0, c.y + g.fac.origin.1)
    }

    fn banned(&self, key: (u8, i32, i32)) -> bool {
        self.blacklist
            .get(&key)
            .map(|t| *t > self.time)
            .unwrap_or(false)
    }

    fn ban(&mut self, key: (u8, i32, i32), secs: f32) {
        self.blacklist.insert(key, self.time + secs);
    }

    fn goal_key(g: &Game, goal: &Goal) -> (u8, i32, i32) {
        let k = match goal.kind {
            Kind::Safe => 0,
            Kind::Hide(_) => 1,
            Kind::Flee => 2,
            Kind::Relay(_) => 3,
            Kind::Battery(_) => 4,
            Kind::Pickup(_) => 5,
            Kind::Lift => 6,
            Kind::Stairs(_) => 7,
            Kind::Explore => 8,
            Kind::Forced => 9,
        };
        Bot::abs_key(g, k, cell_of(goal.at))
    }

    pub fn stats_json(&self) -> Value {
        let s = &self.stats;
        let mut levels = Map::new();
        for (k, v) in &s.levels {
            levels.insert(k.to_string(), json!(r2(*v)));
        }
        json!({
            "frames": s.frames,
            "distance_m": r2(s.distance_m),
            "stuck_events": s.stuck_events,
            "soft_locks": s.soft_locks,
            "interact_failures": s.interact_failures,
            "unreachable_goals": s.unreachable_goals,
            "chase_frames": s.chase_frames,
            "relays": s.relays,
            "deaths": s.deaths,
            "hidden_frames": s.hidden_frames,
            "chunks_visited": s.chunks_visited,
            "max_depth": s.max_depth,
            "hunters_seen": s.hunters_seen,
            "uses": s.uses,
            "flashlight_toggles": s.flashlight_toggles,
            "levels": levels,
            "level_trips": s.level_trips,
            "idle_max_s": r2(s.idle_max_s),
            "idle_recoveries": s.idle_recoveries,
            "escapes": s.escapes,
            "hides": s.hides,
            "relays_camped": s.relays_camped,
            "jumps": s.jumps,
            "goal": self.goal.map(|g| json!({ "kind": g.kind.id(), "at": [r2(g.at.0), r2(g.at.1)] })),
        })
    }

    fn threat(g: &Game) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, h) in g.hunters.iter().enumerate() {
            if !h.active || h.stun > 0.5 || h.gimmick.hidden_from_view() {
                continue;
            }
            let d = pdist(h.pos, g.p.pos);
            // a chaser out of sight counts only up close (the port's hunters keep "chase"
            // a while after losing the runner)
            let seen = g.fac.has_line_of_sight(h.pos, g.p.pos);
            let hot = match h.state {
                State::Chase => d < 20.0 && (seen || d < 9.0),
                State::Investigate | State::Search => d < 7.0,
                _ => false,
            };
            if hot && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|b| b.1)
    }

    fn free_slot(g: &Game) -> bool {
        g.slots.iter().any(|s| s.is_none())
    }

    fn pick_goal(&mut self, g: &Game, threat: Option<usize>) -> Option<Goal> {
        let me = g.p.pos;
        let t = self.time;
        if let Some(f) = self.forced {
            return Some(Goal {
                kind: Kind::Forced,
                at: f,
                focus: None,
                since: t,
            });
        }
        if let Some(hi) = threat {
            let hp = g.hunters[hi].pos;
            if let Some(s) = g.fac.safe_zone {
                if pdist(s.center(), me) < 70.0 && !self.banned(Bot::abs_key(g, 0, s)) {
                    return Some(Goal {
                        kind: Kind::Safe,
                        at: s.center(),
                        focus: None,
                        since: t,
                    });
                }
            }
            let mut best: Option<(f32, usize)> = None;
            for (i, h) in g.fac.hiding.iter().enumerate() {
                let d = pdist(h.cell.center(), me);
                // a locker past the hunter is no refuge
                if d < 18.0
                    && !g.hiding_disabled[i]
                    && !self.banned(Bot::abs_key(g, 1, h.cell))
                    && pdist(h.cell.center(), hp) > d * 0.8
                    && best.map(|b| d < b.0).unwrap_or(true)
                {
                    best = Some((d, i));
                }
            }
            if let Some((_, i)) = best {
                let h = &g.fac.hiding[i];
                let front = (
                    h.cell.center().0 + h.facing.0 as f32 * 0.9,
                    h.cell.center().1 + h.facing.1 as f32 * 0.9,
                );
                return Some(Goal {
                    kind: Kind::Hide(i),
                    at: front,
                    focus: Some(Focus::Locker(i)),
                    since: t,
                });
            }
            let away = (me.0 - hp.0, me.1 - hp.1);
            let l = (away.0 * away.0 + away.1 * away.1).sqrt().max(0.01);
            for k in [25.0, 16.0, 10.0] {
                let p = (me.0 + away.0 / l * k, me.1 + away.1 / l * k);
                if let Some(o) = g.fac.nearest_open(p.0, p.1, PLAYER_RADIUS) {
                    if pdist(o, hp) > pdist(me, hp) {
                        return Some(Goal {
                            kind: Kind::Flee,
                            at: o,
                            focus: None,
                            since: t,
                        });
                    }
                }
            }
        }
        // relays: the nearest within 90 m that is not camped
        let mut best: Option<(f32, usize)> = None;
        for (i, c) in g.fac.relays.iter().enumerate() {
            if g.relays_taken[i] || self.banned(Bot::abs_key(g, 3, *c)) {
                continue;
            }
            let d = pdist(c.center(), me);
            if d > 90.0 {
                continue;
            }
            let camped = g
                .hunters
                .iter()
                .any(|h| h.active && h.state != State::Chase && pdist(h.pos, c.center()) < 2.0);
            if camped {
                self.stats.relays_camped += 1;
                let key = Bot::abs_key(g, 3, *c);
                self.ban(key, 30.0);
                continue;
            }
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        if let Some((_, i)) = best {
            return Some(Goal {
                kind: Kind::Relay(i),
                at: g.fac.relays[i].center(),
                focus: Some(Focus::Relay(i)),
                since: t,
            });
        }
        if g.p.lamp < 30.0 {
            let mut best: Option<(f32, usize)> = None;
            for (i, c) in g.fac.batteries.iter().enumerate() {
                let d = pdist(c.center(), me);
                if !g.batteries_taken[i]
                    && d < 60.0
                    && !self.banned(Bot::abs_key(g, 4, *c))
                    && best.map(|b| d < b.0).unwrap_or(true)
                {
                    best = Some((d, i));
                }
            }
            if let Some((_, i)) = best {
                return Some(Goal {
                    kind: Kind::Battery(i),
                    at: g.fac.batteries[i].center(),
                    focus: Some(Focus::Battery(i)),
                    since: t,
                });
            }
        }
        if Bot::free_slot(g) {
            let mut best: Option<(f32, usize)> = None;
            for (i, p) in g.pickups.iter().enumerate() {
                let d = pdist(p.pos, me);
                let held = g.slots.iter().flatten().any(|s| s.item == p.item);
                if !p.taken
                    && d < 25.0
                    && (!held || p.item.stack_max() > 1)
                    && !self.banned(Bot::abs_key(g, 5, cell_of(p.pos)))
                    && best.map(|b| d < b.0).unwrap_or(true)
                {
                    best = Some((d, i));
                }
            }
            if let Some((_, i)) = best {
                return Some(Goal {
                    kind: Kind::Pickup(i),
                    at: g.pickups[i].pos,
                    focus: Some(Focus::Pickup(i)),
                    since: t,
                });
            }
        }
        if g.fac.has_lift
            && g.relays_collected >= g.fac.relays.len() as i32
            && !self.banned(Bot::abs_key(g, 6, g.fac.exit))
        {
            return Some(Goal {
                kind: Kind::Lift,
                at: g.fac.exit.center(),
                focus: Some(Focus::Lift),
                since: t,
            });
        }
        if g.is_infinite() && t - self.level_since > self.level_budget && !g.fac.stairs.is_empty() {
            // the nearest flight toward the least-visited neighbouring level (within +-3)
            let level = g.inf.as_ref().map(|i| i.level).unwrap_or(0);
            let spent = |l: i32| self.stats.levels.get(&l).copied().unwrap_or(0.0);
            let mut best: Option<(f32, usize)> = None;
            for (i, s) in g.fac.stairs.iter().enumerate() {
                let to = level + s.dir;
                if to > 0 || to < -3 || self.banned(Bot::abs_key(g, 7, s.cell)) {
                    continue;
                }
                let score = pdist(s.cell.center(), me) + spent(to) * 0.2;
                if best.map(|b| score < b.0).unwrap_or(true) {
                    best = Some((score, i));
                }
            }
            if let Some((_, i)) = best {
                let s = g.fac.stairs[i];
                let (cx, cy) = s.cell.center();
                let at = (cx - s.run.0 as f32 * 0.4, cy - s.run.1 as f32 * 0.4);
                return Some(Goal {
                    kind: Kind::Stairs(i),
                    at,
                    focus: Some(Focus::Stairs(i)),
                    since: t,
                });
            }
        }
        self.explore_goal(g)
    }

    /// The 9-cell block in the 5x5 around the runner that scores best on
    /// -visits*40 - dist*0.35 + rand*6 (avoided -80), aiming at an open cell inside it.
    fn explore_goal(&mut self, g: &Game) -> Option<Goal> {
        const B: i32 = 9;
        let me = g.p.pos;
        let (ox, oy) = g.fac.origin;
        let (bx, by) = (
            ((me.0 as i32) + ox).div_euclid(B),
            ((me.1 as i32) + oy).div_euclid(B),
        );
        let mut best: Option<(f32, (f32, f32))> = None;
        for dy in -2..=2 {
            for dx in -2..=2 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let k = (bx + dx, by + dy);
                let centre = (
                    (k.0 * B + B / 2 - ox) as f32 + 0.5,
                    (k.1 * B + B / 2 - oy) as f32 + 0.5,
                );
                if centre.0 < 1.0
                    || centre.1 < 1.0
                    || centre.0 > g.fac.cols as f32 - 1.0
                    || centre.1 > g.fac.rows as f32 - 1.0
                {
                    continue;
                }
                let Some(open) = g.fac.nearest_open(centre.0, centre.1, PLAYER_RADIUS) else {
                    continue;
                };
                if pdist(open, centre) > 6.0 {
                    continue;
                }
                let visits = self.visits.get(&k).copied().unwrap_or(0) as f32;
                let mut score = -visits * 40.0 - pdist(open, me) * 0.35 + self.rng.next_f32() * 6.0;
                if self.banned(Bot::abs_key(g, 8, cell_of(open))) {
                    score -= 80.0;
                }
                if let Some(cur) = self.goal {
                    if cur.kind == Kind::Explore && pdist(cur.at, open) < 1.5 {
                        score += 12.0;
                    }
                }
                if best.map(|b| score > b.0).unwrap_or(true) {
                    best = Some((score, open));
                }
            }
        }
        best.map(|(_, at)| Goal {
            kind: Kind::Explore,
            at,
            focus: None,
            since: self.time,
        })
    }

    fn set_goal(&mut self, goal: Option<Goal>) {
        let same = match (self.goal, goal) {
            (Some(a), Some(b)) => a.kind == b.kind && pdist(a.at, b.at) < 0.5,
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.goal = goal;
            self.path.clear();
            self.path_goal = None;
            self.interact_tries = 0;
            self.stuck_level = 0;
            self.stuck_since = None;
            self.safe_wait = 0.0;
        }
    }

    fn drop_goal(&mut self, g: &Game, ban: f32) {
        if let Some(goal) = self.goal {
            if ban > 0.0 {
                let k = Bot::goal_key(g, &goal);
                self.ban(k, ban);
            }
        }
        self.goal = None;
        self.path.clear();
        self.path_goal = None;
    }

    /// Turns toward `yaw`/`pitch` at a capped rate (look deltas in radians this frame).
    fn aim(c: &mut Controls, g: &Game, yaw: f32, pitch: f32, dt: f32) -> f32 {
        let err = wrap(yaw - g.p.yaw);
        let cap = 9.0 * dt;
        c.look.x = -err.clamp(-cap, cap);
        let perr = pitch - g.p.pitch;
        c.look.y = -perr.clamp(-cap, cap);
        err
    }

    /// One frame of play: the controls the bot presses this frame.
    pub fn decide(&mut self, sim: &Sim) -> Controls {
        let dt = sim.dt;
        self.time += dt;
        self.interact_cd = (self.interact_cd - dt).max(0.0);
        self.flash_cd = (self.flash_cd - dt).max(0.0);
        self.pick_t -= dt;
        let mut c = Controls::default();
        let Some(g) = sim.game.as_ref() else { return c };
        let frame = sim.frame;
        self.stats.frames += 1;
        if g.report.is_some() {
            return c;
        }
        // bookkeeping: distance, levels, windows, chases, hunters seen
        let moved = pdist(g.p.pos, self.last_pos);
        let center = g.inf.as_ref().map(|i| i.center);
        let level = g.inf.as_ref().map(|i| i.level).unwrap_or(1 - g.depth);
        if center != self.window_center || Some(level) != self.last_level {
            // the window recentred (or a new floor): window cells moved under us
            if self.last_level.is_some() && Some(level) != self.last_level {
                self.stats.level_trips += 1;
                self.level_since = self.time;
                self.level_budget = 150.0 + self.rng.next_f32() * 150.0;
                self.event(
                    frame,
                    "level_goal",
                    json!({ "level": level, "depth": g.depth }),
                );
            }
            self.window_center = center;
            self.last_level = Some(level);
            self.goal = None;
            self.path.clear();
            self.path_goal = None;
            self.forced = None;
            self.stuck_ref = (g.p.pos, self.time);
        } else if moved < 3.0 {
            self.stats.distance_m += moved;
        }
        self.last_pos = g.p.pos;
        *self.stats.levels.entry(level).or_insert(0.0) += dt;
        self.stats.max_depth = self.stats.max_depth.max(g.depth);
        self.stats.chunks_visited = g.chunks_visited().max(self.stats.chunks_visited);

        if g.hunters
            .iter()
            .any(|h| h.active && h.state == State::Chase)
        {
            self.stats.chase_frames += 1;
        }
        for h in g
            .hunters
            .iter()
            .filter(|h| h.active && !h.gimmick.hidden_from_view())
        {
            let d = pdist(h.pos, g.p.pos);
            if d < 20.0
                && !self.stats.hunters_seen.contains_key(&h.def.id)
                && g.fac.has_line_of_sight(h.pos, g.p.pos)
            {
                self.stats
                    .hunters_seen
                    .insert(h.def.id.clone(), json!(frame));
                self.event(frame, "hunter_encounter", json!({ "hunter": h.def.id, "state": format!("{:?}", h.state).to_uppercase(), "dist": r2(d) }));
            }
        }
        self.visit_t -= dt;
        if self.visit_t <= 0.0 {
            self.visit_t = 1.0;
            let (ox, oy) = g.fac.origin;
            let k = (
                ((g.p.pos.0 as i32) + ox).div_euclid(9),
                ((g.p.pos.1 as i32) + oy).div_euclid(9),
            );
            *self.visits.entry(k).or_insert(0) += 1;
        }

        // 1. captured: mash [E]
        if let Some(e) = &g.escape {
            if frame % 2 == 0 {
                c.interact = true;
            }
            if e.progress >= ESCAPE_PRESSES - 1 && c.interact {
                self.stats.escapes += 1;
            }
            return c;
        }
        if g.shop_open() {
            // leave the store without trading
            c.shop_choice = Some(9);
            return c;
        }
        let threat = Bot::threat(g);
        // 2. hidden: stay while a threat is near, leave after 3 s clear
        if let Some(spot) = g.p.hidden {
            self.stats.hidden_frames += 1;
            self.hidden_for += dt;
            let close = g
                .hunters
                .iter()
                .any(|h| h.active && pdist(h.pos, g.p.pos) < 4.0);
            if threat.is_some() && !(self.hidden_for > 25.0 && !close) {
                self.hidden_clear = 0.0;
            } else {
                self.hidden_clear += dt;
                if self.hidden_clear >= 3.0 && self.interact_cd <= 0.0 && g.p.hide_lock <= 0.0 {
                    c.interact = true;
                    self.interact_cd = 0.7;
                    let k = Bot::abs_key(g, 1, g.fac.hiding[spot].cell);
                    self.ban(k, 10.0);
                    self.goal = None;
                }
            }
            return c;
        }
        self.hidden_clear = 0.0;
        self.hidden_for = 0.0;
        // flashlight: on below the top level of the Infinite route (headless rule), else off
        let want_lamp = g.is_infinite() && level != 0 && g.p.lamp > 3.0 && threat.is_none();
        if want_lamp != g.p.flashlight_on
            && self.flash_cd <= 0.0
            && (g.p.lamp > 3.0 || g.p.flashlight_on)
        {
            c.flashlight = true;
            self.flash_cd = 1.0;
            self.stats.flashlight_toggles += 1;
        }
        if g.p.lamp < 40.0 && g.p.lamp < MAX_LAMP_CHARGE - BATTERY_RECHARGE {
            if let Some(i) = g.slots.iter().position(|s| {
                s.map(|s| s.item == crate::game::Item::Battery)
                    .unwrap_or(false)
            }) {
                if g.selected != i {
                    c.slot = Some(i);
                } else if self.interact_cd <= 0.0 {
                    c.use_item = true;
                    self.interact_cd = 0.5;
                    *self.stats.uses.entry("battery").or_insert(json!(0)) = json!(
                        self.stats
                            .uses
                            .get("battery")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            + 1
                    );
                }
            }
        }
        // 3. goals
        let threat_changed =
            threat.is_some() != self.goal.map(|g| g.kind.threat_goal()).unwrap_or(false);
        if self.goal.is_none() || self.pick_t <= 0.0 || threat_changed {
            self.pick_t = 0.25;
            let keep_safe = self.goal.map(|g| g.kind == Kind::Safe).unwrap_or(false)
                && threat.is_none()
                && self.safe_wait > 0.0
                && self.safe_wait < 25.0;
            if !keep_safe {
                let next = self.pick_goal(g, threat);
                self.set_goal(next);
            }
        }
        let Some(goal) = self.goal else { return c };

        // completion by focus: press [E] as soon as the crosshair is on the goal
        if let Some(want) = goal.focus {
            if g.focus == Some(want) && self.interact_cd <= 0.0 {
                c.interact = true;
                self.interact_cd = 0.7;
                self.interact_tries += 1;
                if matches!(goal.kind, Kind::Hide(_)) {
                    self.stats.hides += 1;
                }
                let key = goal.kind.id().to_string();
                let n = self
                    .stats
                    .uses
                    .get(&key)
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    + 1;
                self.stats.uses.insert(key, json!(n));
                if self.interact_tries > 5 {
                    self.stats.interact_failures += 1;
                    self.event(frame, "interact_failed", json!({ "goal_kind": goal.kind.id(), "at": [r2(goal.at.0), r2(goal.at.1)] }));
                    self.drop_goal(g, 25.0);
                }
                return c;
            }
        }
        let d = pdist(goal.at, g.p.pos);
        let reach = match goal.kind {
            Kind::Safe => SAFE_ZONE_RADIUS - 0.6,
            Kind::Explore | Kind::Flee => 1.2,
            Kind::Forced => self.forced_reach,
            _ => 1.1,
        };
        if d <= reach {
            match goal.kind {
                Kind::Safe => {
                    self.safe_wait += dt;
                    if g.p.lamp < 90.0 || g.p.stamina < 90.0 {
                        if g.focus == Some(Focus::Safe) && self.interact_cd <= 0.0 {
                            c.interact = true;
                            self.interact_cd = 2.0;
                        }
                    }
                    if threat.is_none() && self.safe_wait > 6.0 || self.safe_wait > 25.0 {
                        self.drop_goal(g, 20.0);
                    } else {
                        let (sx, sy) = goal.at;
                        let y = yaw_to(sx - g.p.pos.0, sy - g.p.pos.1);
                        Bot::aim(&mut c, g, y, -0.3, dt);
                    }
                    return c;
                }
                Kind::Explore | Kind::Flee => {
                    self.drop_goal(
                        g,
                        if goal.kind == Kind::Explore {
                            30.0
                        } else {
                            0.0
                        },
                    );
                    return c;
                }
                Kind::Forced => {
                    self.forced = None;
                    self.drop_goal(g, 0.0);
                    self.event(
                        frame,
                        "arrived",
                        json!({ "at": [r2(goal.at.0), r2(goal.at.1)] }),
                    );
                    return c;
                }
                _ => {
                    // aim at the object: yaw toward it, pitch toward its height
                    let target = self.focus_target(g, &goal);
                    let (dx, dy) = (target.0 - g.p.pos.0, target.1 - g.p.pos.1);
                    let horiz = (dx * dx + dy * dy).sqrt().max(0.05);
                    let h = self.focus_height(&goal);
                    let pitch = (h - g.eye().y).atan2(horiz).clamp(-1.05, 0.8);
                    Bot::aim(&mut c, g, yaw_to(dx, dy), pitch, dt);
                    if horiz > 0.75 && wrap(yaw_to(dx, dy) - g.p.yaw).abs() < 0.4 {
                        c.fwd = 0.3;
                    }
                    if self.time - goal.since > 12.0 {
                        self.stats.interact_failures += 1;
                        self.event(frame, "interact_failed", json!({ "goal_kind": goal.kind.id(), "at": [r2(goal.at.0), r2(goal.at.1)], "focus": format!("{:?}", g.focus) }));
                        self.drop_goal(g, 25.0);
                    }
                    return c;
                }
            }
        }

        // 4. navigate
        self.path_age += dt;
        let goal_cell = cell_of(goal.at);
        if self.path.is_empty() || self.path_age > 0.5 || self.path_goal != Some(goal_cell) {
            self.path_age = 0.0;
            self.path = path_to(g, goal.at);
            self.path_goal = Some(goal_cell);
            if self.path.is_empty() {
                self.stats.unreachable_goals += 1;
                self.event(frame, "unreachable_goal", json!({ "goal_kind": goal.kind.id(), "from": [r2(g.p.pos.0), r2(g.p.pos.1)], "to": [r2(goal.at.0), r2(goal.at.1)] }));
                self.drop_goal(g, 25.0);
                return c;
            }
        }
        let here = cell_of(g.p.pos);
        let mut idx = 0;
        let mut bestd = i32::MAX;
        for (i, p) in self.path.iter().enumerate() {
            let dd = (p.x - here.x).abs().max((p.y - here.y).abs());
            if dd <= bestd {
                bestd = dd;
                idx = i;
            }
        }
        let ahead = if idx + 2 < self.path.len() {
            self.path[idx + 2].center()
        } else if idx + 1 < self.path.len() {
            self.path[idx + 1].center()
        } else {
            goal.at
        };
        let ahead = if self.path.len() - idx <= 2 {
            goal.at
        } else {
            ahead
        };
        let (dx, dy) = (ahead.0 - g.p.pos.0, ahead.1 - g.p.pos.1);
        let err = Bot::aim(&mut c, g, yaw_to(dx, dy), -0.05, dt);
        c.fwd = if err.abs() < 0.7 {
            1.0
        } else if err.abs() < 1.4 {
            0.25
        } else {
            0.0
        };
        c.sprint = threat.is_some() && g.p.stamina > 15.0 && !g.p.exhausted;

        // 5. stuck and soft-lock recovery
        if self.time - self.stuck_ref.1 >= 1.5 {
            let gone = pdist(g.p.pos, self.stuck_ref.0);
            self.stuck_ref = (g.p.pos, self.time);
            if gone < 0.3 && d > 3.0 {
                if self.stuck_since.is_none() {
                    self.stuck_since = Some(self.time);
                    self.stuck_next = self.time;
                }
            } else {
                self.stuck_since = None;
                self.stuck_level = 0;
            }
        }
        if let Some(since) = self.stuck_since {
            if self.time >= self.stuck_next {
                self.stuck_next = self.time + 1.2;
                self.stuck_level += 1;
                match self.stuck_level {
                    1 => {
                        c.jump = true;
                        self.stats.jumps += 1;
                    }
                    2 => {
                        self.stats.stuck_events += 1;
                        let at = [r2(g.p.pos.0), r2(g.p.pos.1)];
                        self.event(frame, "stuck", json!({ "count": self.stats.stuck_events, "at": at, "goal": [r2(goal.at.0), r2(goal.at.1)], "goal_kind": goal.kind.id(), "focus": format!("{:?}", g.focus) }));
                        c.interact = g.focus.is_some() && goal.focus.is_none();
                        c.jump = true;
                    }
                    3 => {
                        self.strafe_until = self.time + 0.6;
                        self.strafe_dir = if self.rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
                    }
                    4 => {
                        self.path.clear();
                    }
                    _ => {
                        if self.stuck_level == 5 {
                            let k = Bot::goal_key(g, &goal);
                            self.ban(k, 8.0);
                            self.path.clear();
                        }
                    }
                }
            }
            if self.time - since > 12.0 {
                self.stats.soft_locks += 1;
                self.event(frame, "soft_lock", json!({ "at": [r2(g.p.pos.0), r2(g.p.pos.1)], "seconds": r2(self.time - since) }));
                self.drop_goal(g, 25.0);
                self.stuck_since = None;
                self.stuck_level = 0;
            }
        }
        if self.time < self.strafe_until {
            c.strafe = self.strafe_dir;
            c.fwd = c.fwd.max(0.3);
        }
        // idle watchdog: no walking and no turning
        if moved < 0.002 && c.look.x.abs() < 1e-4 {
            self.idle_t += dt;
            self.stats.idle_max_s = self.stats.idle_max_s.max(self.idle_t);
            if self.idle_t > 45.0 {
                self.idle_t = 0.0;
                self.stats.idle_recoveries += 1;
                self.event(
                    frame,
                    "idle",
                    json!({ "at": [r2(g.p.pos.0), r2(g.p.pos.1)] }),
                );
                self.drop_goal(g, 25.0);
            }
        } else {
            self.idle_t = 0.0;
        }
        let _ = vec2(0.0, 0.0);
        c
    }

    /// Soft-lock teleport (debug runs only): a random open cell within +-6 m.
    pub fn unstick_spot(&mut self, g: &Game) -> Option<(f32, f32)> {
        for _ in 0..24 {
            let p = (
                g.p.pos.0 + (self.rng.next_f32() - 0.5) * 12.0,
                g.p.pos.1 + (self.rng.next_f32() - 0.5) * 12.0,
            );
            if let Some(o) = g.fac.nearest_open(p.0, p.1, PLAYER_RADIUS) {
                if pdist(o, g.p.pos) > 1.0 {
                    return Some(o);
                }
            }
        }
        None
    }

    fn focus_target(&self, g: &Game, goal: &Goal) -> (f32, f32) {
        match goal.focus {
            Some(Focus::Locker(i)) => g.fac.locker_center(&g.fac.hiding[i]),
            Some(Focus::Stairs(i)) => {
                let s = g.fac.stairs[i];
                let (cx, cy) = s.cell.center();
                (cx + s.run.0 as f32 * 0.9, cy + s.run.1 as f32 * 0.9)
            }
            Some(Focus::Relay(i)) => g.fac.relays[i].center(),
            Some(Focus::Battery(i)) => g.fac.batteries[i].center(),
            Some(Focus::Pickup(i)) => g.pickups[i].pos,
            Some(Focus::Lift) => g.fac.exit.center(),
            _ => goal.at,
        }
    }

    fn focus_height(&self, goal: &Goal) -> f32 {
        match goal.focus {
            Some(Focus::Relay(_)) => 0.8,
            Some(Focus::Battery(_)) | Some(Focus::Pickup(_)) => 0.15,
            Some(Focus::Locker(_)) => 1.0,
            Some(Focus::Lift) => 1.2,
            Some(Focus::Stairs(_)) => 0.3,
            _ => 0.8,
        }
    }

    /// Bot step with the soft-lock teleport applied (debug runs).
    pub fn step(&mut self, sim: &mut Sim) {
        let soft = self.stats.soft_locks;
        let c = self.decide(sim);
        sim.step_with(Some(c));
        if self.debug && self.stats.soft_locks > soft {
            if let Some(g) = sim.game.as_mut() {
                let spot = self.unstick_spot(g);
                if let Some(p) = spot {
                    g.p.pos = p;
                }
            }
        }
        if let Some(g) = sim.game.as_ref() {
            // relays recovered this frame (a new floor resets the count to 0)
            if g.relays_collected > self.last_relays {
                self.stats.relays += g.relays_collected - self.last_relays;
            }
            self.last_relays = g.relays_collected;
            match &g.report {
                Some(r) if !self.dead => {
                    self.dead = true;
                    self.stats.deaths += 1;
                    let e = json!({ "t": r2(g.time), "hunter": r.hunter, "cause": r.cause, "depth": r.depth });
                    self.event(sim.frame, "death", e);
                }
                None => self.dead = false,
                _ => {}
            }
        }
    }
}
