//! NPC teammates, phase one: the admin spawns NPCs in an offline room and they follow the runner.
//! This is the offline core of Godot's `game/player/npc_teammates.gd` (spawn, the follow rule, the
//! walk and the turn spring) and of the host's spawn spot rule (`infinite_route_test.gd` 6769-6779).
//! No server is involved: the NPCs live on the `Game` for the run and nothing is sent anywhere.
//!
//! Deferred (see docs/NPC_TEAMMATES in the Godot tree): per-NPC commands, catching and fleeing,
//! chat, storey crossing, co-op packets, name tags, saves, and the mannequin body.

use super::pdist;
use super::Game;
use crate::config::PLAYER_WALK_SPEED;
use crate::render::textures::rng::GodotRng;
use crate::world::route::run_seed_of_text;
use crate::world::{Cell, Facility};
use std::f32::consts::PI;

/// Godot `npc_teammates.gd` 17: the ring radius of the first eight slots.
pub const SLOT_RADIUS: f32 = 2.4;
/// Godot `npc_teammates.gd` 20: slots past this many widen the ring.
pub const SLOT_RING_SLOTS: f32 = 8.0;
/// Godot `npc_teammates.gd` 23: each slot's first repath phase.
pub const PATH_STAGGER: f32 = 0.137;
/// Godot `npc_teammates.gd` 24: an NPC walks to its slot again once the runner is this far.
pub const FOLLOW_RADIUS: f32 = 5.0;
/// Godot `npc_teammates.gd` 27: seconds between path plans.
pub const REPATH: f32 = 0.5;
/// Godot `npc_teammates.gd` 39: time constant of the heading ease.
pub const HEADING_SMOOTH: f32 = 0.3;
/// Godot `npc_teammates.gd` 40: the body-turn spring.
pub const TURN_SPRING: f32 = 60.0;
/// Godot `npc_teammates.gd` 41: rad/s cap on the body turn.
pub const TURN_RATE: f32 = 5.0;
/// Godot literal `.23` (npc_teammates.gd 190, 348): the body radius the nav cells must fit.
pub const NAV_RADIUS: f32 = 0.23;
/// Godot literal `2.39996` (npc_teammates.gd 76, 187, 329): the golden angle between slots.
pub const SLOT_ANGLE: f32 = 2.39996;
/// Godot `admin_npc_panel.gd` 155: the crosshair ray that picks the spawn point, in metres.
pub const SPAWN_RAY_RANGE: f32 = 30.0;
/// Godot `infinite_route_test.gd` 6772: a crosshair point farther than this from the admin is not used.
pub const SPAWN_NEAR_ADMIN: f32 = 40.0;
/// Godot `admin_npc_panel.gd` 149-160: the ray's step while it looks for the floor (the window's quarter cell).
const RAY_STEP: f32 = 0.25;

/// One NPC teammate. Fields for the deferred features (catch, revive, chat, admin commands) are
/// added when those land.
#[derive(Clone, Debug)]
pub struct Npc {
    /// Never reused within a run (Godot `_next`).
    pub id: u32,
    /// The guest name, `R-04821` (Godot `GuestNames.from_seed(run_seed + id)`).
    pub name: String,
    /// `id + 1` (Godot `slot = _next + 1`): sets the NPC's ring slot and its phase.
    pub slot: u32,
    /// Position on the window floor, in the same frame as `Game::p.pos`.
    pub at: (f32, f32),
    /// The infinite level the NPC stands on (0 on a finite floor).
    pub level: i32,
    /// Body yaw in radians, Godot's `facing`.
    pub facing: f32,
    /// The turn spring's velocity (Godot `turn_v`).
    pub turn_v: f32,
    /// The smoothed walking direction (Godot `heading`), unit length once set.
    pub heading: (f32, f32),
    /// Grid cells still to walk. The front is the cell the NPC stands in until it is popped.
    pub path: Vec<(i32, i32)>,
    /// Seconds since the last plan.
    pub path_age: f32,
    /// Walk clip (true) or idle (false).
    pub walking: bool,
    /// Metres walked: drives the stride swing of the drawn body.
    pub stride: f32,
    /// Smartness, 0..100, as set at spawn (stored for the deferred flee).
    pub smart: i32,
}

impl Npc {
    /// Moves the NPC by `d` after the window recentres. The path is in the old frame, so it is dropped and
    /// replanned at once.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.at = (self.at.0 + d.0, self.at.1 + d.1);
        self.path.clear();
        self.path_age = REPATH;
    }
}

/// Godot `SLOT_RADIUS * maxf(1.0, sqrt(slot / SLOT_RING_SLOTS))`.
pub fn slot_radius(slot: u32) -> f32 {
    SLOT_RADIUS * (slot as f32 / SLOT_RING_SLOTS).sqrt().max(1.0)
}

/// The point a slot holds round its runner: `leader + (cos, sin)(slot * SLOT_ANGLE) * ring`.
fn slot_point(leader: (f32, f32), slot: u32, ring: f32) -> (f32, f32) {
    let angle = slot as f32 * SLOT_ANGLE;
    (
        leader.0 + angle.cos() * ring,
        leader.1 + angle.sin() * ring,
    )
}

/// The NPC name for a seed: one letter, a dash, then five digits, like `K-04817`. The same seed
/// always gives the same name. The generator is Godot's PCG32 port (`render::textures::rng::GodotRng`).
pub fn guest_name_from_seed(seed: u64) -> String {
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"0123456789";
    let mut rng = GodotRng::new(seed);
    let mut name = String::with_capacity(7);
    name.push(LETTERS[(rng.randi() % LETTERS.len() as u32) as usize] as char);
    name.push('-');
    for _ in 0..5 {
        name.push(DIGITS[(rng.randi() % DIGITS.len() as u32) as usize] as char);
    }
    name
}

/// Godot `wrapf`: `value` folded into `[min, max)`.
fn wrapf(value: f32, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range.abs() < f32::EPSILON {
        return min;
    }
    value - range * ((value - min) / range).floor()
}

/// Godot `_turn`: the body yaw through a critically damped spring, its rate capped at `TURN_RATE`.
fn turn(n: &mut Npc, want: f32, dt: f32) {
    let error = wrapf(want - n.facing, -PI, PI);
    let v = n.turn_v
        + (TURN_SPRING * error - 2.0 * TURN_SPRING.sqrt() * n.turn_v) * dt;
    n.turn_v = v.clamp(-TURN_RATE, TURN_RATE);
    n.facing = wrapf(n.facing + n.turn_v * dt, -PI, PI);
}

/// Godot `_walk_toward`: one grid step toward `goal`, replanning every `REPATH` seconds.
fn walk_toward(n: &mut Npc, fac: &Facility, goal: (f32, f32), dt: f32) {
    let here = n.at;
    n.path_age += dt;
    if n.path_age >= REPATH {
        n.path_age = 0.0;
        let to = fac.nearest_open(goal.0, goal.1, NAV_RADIUS).unwrap_or(goal);
        let from = Cell::new(here.0.floor() as i32, here.1.floor() as i32);
        let to = Cell::new(to.0.floor() as i32, to.1.floor() as i32);
        n.path = fac
            .find_path(from, to)
            .into_iter()
            .map(|c| (c.x, c.y))
            .collect();
        if !n.path.is_empty() {
            // the cell it stands in (find_path starts at it)
            n.path.remove(0);
        }
    }
    if n.path.is_empty() {
        n.walking = false;
        return;
    }
    let next = n.path[0];
    let target = (next.0 as f32 + 0.5, next.1 as f32 + 0.5);
    let step = (target.0 - here.0, target.1 - here.1);
    let len = (step.0 * step.0 + step.1 * step.1).sqrt();
    let dir = if len > 1e-6 {
        (step.0 / len, step.1 / len)
    } else {
        (0.0, 0.0)
    };
    if len > 0.01 {
        // the heading starts on the first step and then eases toward it (Godot: lerp, then normalise)
        let k = 1.0 - (-dt / HEADING_SMOOTH).exp();
        let h = if n.heading == (0.0, 0.0) {
            dir
        } else {
            (
                n.heading.0 + (dir.0 - n.heading.0) * k,
                n.heading.1 + (dir.1 - n.heading.1) * k,
            )
        };
        let hl = (h.0 * h.0 + h.1 * h.1).sqrt();
        if hl > 1e-6 {
            n.heading = (h.0 / hl, h.1 / hl);
        }
    }
    let speed = PLAYER_WALK_SPEED;
    let moved;
    if len <= speed * dt {
        n.path.remove(0);
        n.at = target;
        moved = len;
    } else {
        moved = speed * dt;
        n.at = (here.0 + dir.0 * moved, here.1 + dir.1 * moved);
    }
    let heading = n.heading;
    turn(n, (-heading.0).atan2(-heading.1), dt);
    n.stride += moved;
    n.walking = true;
}

/// Godot `_think` for phase one, same-level only: follow the runner to the NPC's slot when it has
/// drifted past `FOLLOW_RADIUS + ring - SLOT_RADIUS` (or is mid-walk), else stand still.
fn follow(n: &mut Npc, fac: &Facility, leader: (f32, f32), dt: f32) {
    let ring = slot_radius(n.slot);
    if pdist(n.at, leader) > FOLLOW_RADIUS + ring - SLOT_RADIUS || !n.path.is_empty() {
        let goal = slot_point(leader, n.slot, ring);
        walk_toward(n, fac, goal, dt);
    } else {
        n.walking = false;
    }
}

impl Game {
    /// The infinite level the runner stands on (0 on a finite route).
    pub fn runner_level(&self) -> i32 {
        self.inf.as_ref().map_or(0, |i| i.level)
    }

    /// Godot `RoomState.run_seed() + id` for the NPC's name.
    fn npc_name(&self, id: u32) -> String {
        guest_name_from_seed(run_seed_of_text(&self.room_text) as u64 + id as u64)
    }

    /// The crosshair's floor point (Godot `crosshair_floor`): a ray from the eye along `dir`, up to
    /// `SPAWN_RAY_RANGE` metres, that must reach the floor over open cells. A wall in the way gives no point.
    fn npc_crosshair_floor(&self, eye: (f32, f32, f32), dir: (f32, f32, f32), floor_y: f32) -> Option<(f32, f32)> {
        let mut d = 0.0f32;
        while d <= SPAWN_RAY_RANGE {
            let p = (eye.0 + dir.0 * d, eye.1 + dir.1 * d, eye.2 + dir.2 * d);
            if !self
                .fac
                .hunter_open(Cell::new(p.0.floor() as i32, p.2.floor() as i32))
            {
                return None;
            }
            if p.1 <= floor_y + 0.05 {
                return Some((p.0, p.2));
            }
            d += RAY_STEP;
        }
        None
    }

    /// The admin spawn (Godot `NpcTeammates.spawn` with the host's spot rule, `infinite_route_test.gd`
    /// 6769-6779). `count` NPCs go round the crosshair floor point when it is open and within
    /// `SPAWN_NEAR_ADMIN`; otherwise round the runner. `eye` and `dir` are the crosshair ray (the
    /// runner's eye and forward). Returns how many were made. No server is involved.
    pub fn npc_spawn(
        &mut self,
        count: i32,
        smart: i32,
        eye: (f32, f32, f32),
        dir: (f32, f32, f32),
    ) -> usize {
        if count <= 0 {
            return 0;
        }
        let runner = self.p.pos;
        let floor_y = self.p.feet_y.max(0.0);
        let spot = match self.npc_crosshair_floor(eye, dir, floor_y) {
            Some(p) if pdist(p, runner) < SPAWN_NEAR_ADMIN && self.fac.body_fits(p.0, p.1, NAV_RADIUS) => {
                p
            }
            _ => runner,
        };
        let level = self.runner_level();
        for _ in 0..count {
            let id = self.npc_next;
            let slot = id + 1;
            // Godot: one NPC stands on the spot itself, more fan out round it
            let ring = if count == 1 { 0.0 } else { slot_radius(slot) };
            let want = slot_point(spot, slot, ring);
            let at = self
                .fac
                .nearest_open(want.0, want.1, NAV_RADIUS)
                .or_else(|| self.fac.nearest_open(spot.0, spot.1, NAV_RADIUS))
                .unwrap_or(spot);
            let phase = (slot as f32 * PATH_STAGGER) % REPATH;
            let name = self.npc_name(id);
            self.npcs.push(Npc {
                id,
                name,
                slot,
                at,
                level,
                facing: 0.0,
                turn_v: 0.0,
                heading: (0.0, 0.0),
                path: vec![],
                path_age: REPATH - phase,
                walking: false,
                stride: 0.0,
                smart: smart.clamp(0, 100),
            });
            self.npc_next += 1;
        }
        count as usize
    }

    /// Admin REMOVE NEAREST: the NPC nearest the runner, by distance on the floor. Returns its name.
    pub fn npc_remove_nearest(&mut self) -> Option<String> {
        let runner = self.p.pos;
        let mut best: Option<(usize, f32)> = None;
        for (i, n) in self.npcs.iter().enumerate() {
            let d = pdist(n.at, runner);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        let (i, _) = best?;
        Some(self.npcs.remove(i).name)
    }

    /// Admin REMOVE ALL. Returns how many were removed.
    pub fn npc_remove_all(&mut self) -> usize {
        let n = self.npcs.len();
        self.npcs.clear();
        n
    }

    /// Re-places every NPC beside the runner after the floor is rebuilt (a new finite floor replaces the
    /// facility, so old positions mean nothing). Ring rule as at spawn.
    pub(super) fn npc_place_beside_runner(&mut self) {
        let level = self.runner_level();
        let spot = self.p.pos;
        let total = self.npcs.len();
        let fac = &self.fac;
        for n in self.npcs.iter_mut() {
            let ring = if total == 1 { 0.0 } else { slot_radius(n.slot) };
            let want = slot_point(spot, n.slot, ring);
            n.at = fac
                .nearest_open(want.0, want.1, NAV_RADIUS)
                .or_else(|| fac.nearest_open(spot.0, spot.1, NAV_RADIUS))
                .unwrap_or(spot);
            n.level = level;
            n.path.clear();
            n.path_age = REPATH;
            n.walking = false;
        }
    }

    /// The per-frame tick: every NPC on the runner's level follows the runner; an NPC on another level
    /// stands still (its level's storey crossing is deferred).
    pub(super) fn update_npcs(&mut self, dt: f32) {
        if self.npcs.is_empty() {
            return;
        }
        let level = self.runner_level();
        let leader = self.p.pos;
        let fac = &self.fac;
        for n in self.npcs.iter_mut() {
            if n.level != level {
                n.walking = false;
                continue;
            }
            follow(n, fac, leader, dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    /// A finite route (no server, no renderer, no assets): the offline game the admin plays.
    fn offline_game() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        (g, content, catalog)
    }

    /// A ray from above the runner straight down (the crosshair at the runner's own feet).
    fn down_at(p: (f32, f32)) -> ((f32, f32, f32), (f32, f32, f32)) {
        ((p.0, 1.6, p.1), (0.0, -1.0, 0.0))
    }

    /// A ray pointing at the sky: the crosshair finds no floor.
    fn sky(p: (f32, f32)) -> ((f32, f32, f32), (f32, f32, f32)) {
        ((p.0, 1.6, p.1), (0.0, 1.0, 0.0))
    }

    #[test]
    fn spawn_count_and_smartness_are_kept() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        assert_eq!(g.npc_spawn(3, 90, e, d), 3);
        assert_eq!(g.npcs.len(), 3);
        assert!(g.npcs.iter().all(|n| n.smart == 90));
        assert_eq!(g.npc_spawn(2, 30, e, d), 2);
        assert_eq!(g.npcs.len(), 5);
        assert_eq!(g.npcs[4].smart, 30);
        // ids and slots never repeat, as Godot's _next
        let ids: Vec<u32> = g.npcs.iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);
        assert!(g.npcs.iter().all(|n| n.slot == n.id + 1));
        // a smartness out of range is clamped, as Godot clampi(smart, 0, 100)
        g.npc_spawn(1, 250, e, d);
        assert_eq!(g.npcs.last().unwrap().smart, 100);
        // zero or less makes nothing
        assert_eq!(g.npc_spawn(0, 70, e, d), 0);
        assert_eq!(g.npc_spawn(-2, 70, e, d), 0);
        assert_eq!(g.npcs.len(), 6);
    }

    #[test]
    fn names_follow_the_guest_name_shape() {
        for seed in [1u64, 2, 99, 4_000_000_000] {
            let name = guest_name_from_seed(seed);
            let b = name.as_bytes();
            assert_eq!(b.len(), 7, "{name}");
            assert!(b"ABCDEFGHJKLMNPQRSTUVWXYZ".contains(&b[0]), "{name}");
            assert_eq!(b[1], b'-');
            for &c in &b[2..] {
                assert!(c.is_ascii_digit(), "{name}");
            }
            assert_eq!(guest_name_from_seed(seed), name, "same seed, same name");
        }
    }

    #[test]
    fn spawned_names_use_the_room_seed_plus_id() {
        let (mut g, _, _) = offline_game();
        let (e, d) = sky(g.p.pos);
        g.npc_spawn(2, 70, e, d);
        let base = run_seed_of_text(&g.room_text) as u64;
        assert_eq!(g.npcs[0].name, guest_name_from_seed(base));
        assert_eq!(g.npcs[1].name, guest_name_from_seed(base + 1));
    }

    #[test]
    fn slot_radius_is_godot_s() {
        assert!((slot_radius(1) - 2.4).abs() < 1e-6);
        assert!((slot_radius(8) - 2.4).abs() < 1e-6);
        assert!(slot_radius(9) > 2.4);
        assert!((slot_radius(32) - 4.8).abs() < 1e-4, "sqrt(32/8) = 2");
    }

    #[test]
    fn remove_nearest_then_all() {
        let (mut g, _, _) = offline_game();
        assert!(g.npc_remove_nearest().is_none(), "no NPCs, nothing to remove");
        assert_eq!(g.npc_remove_all(), 0);
        let (e, d) = sky(g.p.pos);
        g.npc_spawn(2, 70, e, d);
        // move one NPC far away: the other is the nearest and goes first
        let far = (g.p.pos.0 + 20.0, g.p.pos.1);
        g.npcs[0].at = far;
        let near_name = g.npcs[1].name.clone();
        assert_eq!(g.npc_remove_nearest(), Some(near_name));
        assert_eq!(g.npcs.len(), 1);
        assert_eq!(g.npcs[0].id, 0, "the far NPC is kept");
        assert_eq!(g.npc_remove_all(), 1);
        assert!(g.npcs.is_empty());
        // spawning again after a clear carries on the id counter (ids never reused)
        g.npc_spawn(1, 70, e, d);
        assert_eq!(g.npcs[0].id, 2);
    }

    #[test]
    fn crosshair_floor_is_used_when_open() {
        let (mut g, _, _) = offline_game();
        // an open floor cell a few metres off, looked at from above
        let runner = g.p.pos;
        let spot = (-12..=12)
            .flat_map(|dx| (-12..=12).map(move |dy| (dx, dy)))
            .map(|(dx, dy)| (runner.0 + dx as f32 * 0.5, runner.1 + dy as f32 * 0.5))
            .find(|&p| pdist(p, runner) > 2.0 && g.fac.body_fits(p.0, p.1, NAV_RADIUS));
        let Some(spot) = spot else {
            panic!("no open floor 2 m or more from the runner in the test floor");
        };
        let (e, d) = down_at(spot);
        g.npc_spawn(1, 70, e, d);
        let at = g.npcs[0].at;
        // the NPC stands on its cell's centre (nearest_open), so it is within half a cell diagonal of the point
        assert!(pdist(at, spot) <= 0.71, "spawned at the crosshair point, got {at:?} for {spot:?}");
    }

    #[test]
    fn crosshair_into_the_sky_spawns_beside_the_runner() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        let at = g.npcs[0].at;
        assert!(pdist(at, runner) < 1.0, "beside the runner, got {at:?} for {runner:?}");
    }

    #[test]
    fn an_npc_follows_the_runner_and_then_stands_still() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        // put the NPC on an open cell reachable from the runner, about 9 m away
        let fac = &g.fac;
        let start = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
        let reach = fac.distance_map(start);
        let far = reach
            .iter()
            .filter(|(c, dist)| **dist >= 8 && **dist <= 12 && fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
            .map(|(c, _)| c.center())
            .next();
        let Some(far) = far else {
            panic!("no reachable open cell 8 to 12 cells away in the test floor");
        };
        g.npcs[0].at = far;
        let before = pdist(far, runner);
        assert!(before > FOLLOW_RADIUS, "starts out of follow range");
        // 40 simulated seconds at 60 Hz: long enough to walk ~9 m at 4 m/s, with room to spare
        for _ in 0..(40 * 60) {
            g.update_npcs(1.0 / 60.0);
        }
        let after = pdist(g.npcs[0].at, g.p.pos);
        assert!(after < before, "the NPC closed in: {before} -> {after}");
        assert!(
            after <= FOLLOW_RADIUS + 0.5,
            "the NPC is within follow range of the runner: {after}"
        );
        // once in range and not walking, it stands still
        let settled = g.npcs[0].at;
        for _ in 0..120 {
            g.update_npcs(1.0 / 60.0);
        }
        assert!(!g.npcs[0].walking, "idle once settled in range");
        assert_eq!(g.npcs[0].at, settled, "no drift while idle");
    }

    #[test]
    fn an_npc_on_another_level_stands_still() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        g.npcs[0].level = 1; // the runner is on level 0
        g.npcs[0].at = (runner.0 + 9.0, runner.1);
        let before = g.npcs[0].at;
        for _ in 0..120 {
            g.update_npcs(1.0 / 60.0);
        }
        assert_eq!(g.npcs[0].at, before, "an NPC on another level does not move");
        assert!(!g.npcs[0].walking);
    }

    #[test]
    fn spawn_needs_no_server() {
        // the whole path runs on a headless game: no network, no assets, no window
        let (mut g, _, _) = offline_game();
        let (e, d) = sky(g.p.pos);
        assert_eq!(g.npc_spawn(4, 50, e, d), 4);
        for _ in 0..60 {
            g.update_npcs(1.0 / 60.0);
        }
        assert_eq!(g.npc_remove_all(), 4);
    }

    #[test]
    fn shift_drops_the_path_and_forces_a_replan() {
        let mut n = Npc {
            id: 0,
            name: guest_name_from_seed(1),
            slot: 1,
            at: (3.0, 4.0),
            level: 0,
            facing: 0.0,
            turn_v: 0.0,
            heading: (0.0, 1.0),
            path: vec![(1, 2), (1, 3)],
            path_age: 0.1,
            walking: true,
            stride: 0.0,
            smart: 70,
        };
        n.shift((-16.0, 32.0));
        assert_eq!(n.at, (-13.0, 36.0));
        assert!(n.path.is_empty());
        assert!(n.path_age >= REPATH);
    }
}
