//! Phase 3 continuity tests: end-to-end walks of every flight shape through `Game::step` with the neighbour level
//! resident (`inf.stack`), asserting ABSOLUTE y. One continuous space: the level label is relabelled by walking, with
//! no teleport, no window load, no fade, no announce before arrival (docs/STAIRS_CONTINUOUS_DESIGN.md, D1-D8).
//!
//! Registered by game/mod.rs as `#[cfg(test)] mod stack_walk_tests;`. Needs from the owner files (see the T8 report):
//! `InfState.stack` and `InfState.builds` visible to `game`, `Game::keep_stack` and `Game::recentre_window`
//! callable from `game`, `Game::relabel`.

use super::level_stack::floor_y_of;
use super::{pdist, Controls, Game};
use crate::audio::Sounds;
use crate::config::{Difficulty, PLAYER_RADIUS};
use crate::content::Content;
use crate::headless::steer::absolute_path;
use crate::world::catalog::Catalog;
use crate::world::facility::{LinkKind, Stair};
use crate::world::flight::FlightShape;

const FRAME: f32 = 1.0 / 60.0;
const MAX_FRAMES: usize = 1800;
/// The most the absolute feet y may move in one frame (m).
const MAX_STEP_DY: f32 = 0.25;
/// The body lands on the floor it walks to within this (m).
const FLOOR_TOL: f32 = 0.05;
/// Levels whose downward flights include each shape (see headless/flight_tests.rs for why).
const CASES: [(FlightShape, i32); 4] =
    [(FlightShape::Switchback, 0), (FlightShape::Switchback, -1), (FlightShape::Straight, -1), (FlightShape::Spiral, -2)];

/// One frame of a recorded walk.
#[derive(Clone, Copy, Debug)]
struct Rec {
    y: f32,
    /// x/z in absolute cells (the window origin added), so a recentre is not a move.
    abs: (f32, f32),
    level: i32,
    fac_level: i32,
    /// The yaw the walker set before the step and the yaw after it.
    yaw_set: f32,
    yaw: f32,
    move_vel: (f32, f32),
    fade: f32,
    toast_t: f32,
    builds: u32,
    center: (i32, i32),
    residents_share_center: bool,
}

fn origin(g: &Game) -> (f32, f32) {
    g.inf.as_ref().map_or((0.0, 0.0), |i| {
        let o = i.origin();
        (o.0 as f32, o.1 as f32)
    })
}

fn snap(g: &Game, yaw_set: f32) -> Rec {
    let inf = g.inf.as_ref().expect("an infinite run");
    let o = origin(g);
    Rec {
        y: g.p.y(),
        abs: (g.p.pos.0 + o.0, g.p.pos.1 + o.1),
        level: inf.level,
        fac_level: g.fac.level,
        yaw_set,
        yaw: g.p.yaw,
        move_vel: g.p.move_vel,
        fade: g.fade,
        toast_t: g.toast_t,
        builds: inf.builds,
        center: inf.center,
        residents_share_center: inf.stack.residents.iter().all(|r| r.center == inf.center),
    }
}

fn nearest(g: &Game, dir: i32, shape: FlightShape) -> Option<Stair> {
    let me = g.p.pos;
    g.fac
        .stairs
        .iter()
        .copied()
        .filter(|s| s.dir == dir && matches!(s.kind, LinkKind::Stairs) && s.shape == shape)
        .min_by(|a, b| pdist(a.cell.center(), me).partial_cmp(&pdist(b.cell.center(), me)).unwrap())
}

/// A window on `level` with a flight going down of `shape`, the body on its landing and the level beyond resident in
/// `inf.stack`. `None` (with a printed skip message) when 200 seeds have no such flight.
fn window_with(shape: FlightShape, level: i32, content: &Content, catalog: &Catalog) -> Option<(Game, Stair)> {
    for n in 0..200 {
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-STACK-{n}"), level, content, catalog);
        let Some(first) = nearest(&g, -1, shape) else { continue };
        g.p.pos = first.cell.center();
        g.step(FRAME, &Controls::default(), content, catalog, &Sounds::default());
        let Some(s) = nearest(&g, -1, shape) else { continue };
        g.p.pos = s.cell.center();
        // the whole keep set resident before the walk, as a runner standing at a landing has it: the walk then loads nothing
        for _ in 0..8 {
            g.keep_stack(content, catalog);
        }
        return Some((g, s));
    }
    eprintln!("SKIPPED: no window on level {level} has a {shape:?} flight going down in 200 seeds");
    None
}

/// Places the body on the first waypoint of `s` exactly as headless/steer.rs does.
fn place_on(g: &mut Game, s: Stair, first: (f32, f32)) {
    let level = g.inf.as_ref().map_or(0, |i| i.level);
    let o = origin(g);
    g.p.pos = (first.0 - o.0, first.1 - o.1);
    g.p.height = s.surface_rel(level, g.p.pos.0, g.p.pos.1).unwrap_or(0.0);
    g.p.floor_y = floor_y_of(level);
    g.p.feet_y = 0.0;
    g.p.pitch = 0.0;
    g.fade = 0.0;
    g.toast_t = 0.0;
}

/// Steers along the walk path of `s` from `from_idx`, holding forward, until the last waypoint or `max` frames. The
/// first record is the state before any step.
fn walk_recorded(g: &mut Game, s: Stair, content: &Content, catalog: &Catalog, max: usize) -> Vec<Rec> {
    walk_recorded_with(g, s, content, catalog, max, |_| {})
}

/// As `walk_recorded`, with `before_step` called on the game right before every step.
fn walk_recorded_with(g: &mut Game, s: Stair, content: &Content, catalog: &Catalog, max: usize, mut before_step: impl FnMut(&mut Game)) -> Vec<Rec> {
    let path = absolute_path(g, s);
    place_on(g, s, path[0]);
    let mut recs = vec![snap(g, g.p.yaw)];
    let c = Controls { fwd: 1.0, ..Default::default() };
    let mut idx = 1.min(path.len() - 1);
    for _ in 0..max {
        let o = origin(g);
        let (mut dx, mut dz);
        let mut done = false;
        loop {
            let t = path[idx];
            dx = t.0 - o.0 - g.p.pos.0;
            dz = t.1 - o.1 - g.p.pos.1;
            if (dx * dx + dz * dz).sqrt() > 0.3 {
                break;
            }
            if idx + 1 >= path.len() {
                done = true;
                break;
            }
            idx += 1;
        }
        if done {
            break;
        }
        g.p.yaw = (-dx).atan2(-dz);
        let yaw_set = g.p.yaw;
        before_step(g);
        g.step(FRAME, &c, content, catalog, &Sounds::default());
        recs.push(snap(g, yaw_set));
    }
    recs
}

fn hstep(a: &Rec, b: &Rec) -> f32 {
    pdist(a.abs, b.abs)
}

fn mag(v: (f32, f32)) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

/// The contract of one crossing, over a recorded walk of a flight that starts on `start_level` and goes `sign` levels.
fn check_crossing(recs: &[Rec], start_level: i32, sign: i32, what: &str) {
    assert!(recs.len() > 20, "{what}: the walk stepped frames");
    let (first, last) = (recs[0], recs[recs.len() - 1]);
    let end_level = start_level + sign;
    // absolute y: start on the floor it leaves, end on the floor it reaches
    assert!((first.y - floor_y_of(start_level)).abs() <= FLOOR_TOL, "{what}: starts on its floor: {} vs {}", first.y, floor_y_of(start_level));
    assert!((last.y - floor_y_of(end_level)).abs() <= FLOOR_TOL, "{what}: ends on the floor it reaches: {} vs {}", last.y, floor_y_of(end_level));
    // per-frame continuity
    let mut steps: Vec<f32> = recs.windows(2).map(|w| hstep(&w[0], &w[1])).filter(|d| *d > 1e-4).collect();
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = steps.get(steps.len() / 2).copied().unwrap_or(0.0);
    let mut changes = Vec::new();
    for (i, w) in recs.windows(2).enumerate() {
        let (a, b) = (&w[0], &w[1]);
        assert!((b.y - a.y).abs() <= MAX_STEP_DY, "{what}: frame {i}: y pops {} -> {}", a.y, b.y);
        assert!(hstep(a, b) <= median * 1.5 + 1e-3, "{what}: frame {i}: x/z moved {} (median {median}): a relabel never teleports", hstep(a, b));
        assert_eq!(b.fade, 0.0, "{what}: frame {i}: no fade");
        assert_eq!(b.fac_level, b.level, "{what}: frame {i}: fac.level equals inf.level");
        assert!(b.residents_share_center, "{what}: frame {i}: every resident shares inf.center");
        assert_eq!(b.yaw, b.yaw_set, "{what}: frame {i}: the step does not turn the body");
        if b.level != a.level {
            changes.push(i + 1);
        }
        if i + 1 < recs.len() && b.level == start_level {
            assert_eq!(b.toast_t, 0.0, "{what}: frame {i}: no announce before the body has crossed");
        }
    }
    assert_eq!(changes.len(), 1, "{what}: the level label changes exactly once: {changes:?}");
    let k = changes[0];
    assert_eq!(recs[k].level, end_level, "{what}: the label moves one level the right way");
    assert_eq!(first.builds, recs[k].builds, "{what}: no window is built before or on the crossing frame");
    // after it the keep set of the new level is topped up at the preload budget (one window a frame), nothing more
    let cfg = super::preload::preload();
    assert!(
        last.builds - recs[k].builds <= (cfg.levels_up.max(cfg.levels_down) as u32) * 2,
        "{what}: only the new level's keep set is topped up after the crossing"
    );
    // the relabel frame keeps the motion: speed within 5 %, yaw as set
    let (before, at) = (recs[k - 1], recs[k]);
    let (m0, m1) = (mag(before.move_vel), mag(at.move_vel));
    assert!((m1 - m0).abs() <= 0.05 * m0.max(0.5), "{what}: move_vel across the relabel: {m0} -> {m1}");
    // the body is beyond the start floor when it relabels (past the hysteresis)
    assert!((at.y - floor_y_of(start_level)).abs() > 0.5, "{what}: relabelled with the body already off its floor");
}

fn skip_or<T>(o: Option<T>) -> Option<T> {
    o
}

/// Down a flight of each shape, then back up the flight beneath it.
#[test]
fn every_flight_shape_walks_down_and_up_in_one_space() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut ran = 0;
    for (shape, level) in CASES {
        let Some((mut g, s)) = skip_or(window_with(shape, level, &content, &catalog)) else { continue };
        let target = level - 1;
        assert!(g.inf.as_ref().unwrap().stack.has(target), "{shape:?}: the level beyond the landing is resident");
        let down = walk_recorded(&mut g, s, &content, &catalog, MAX_FRAMES);
        check_crossing(&down, level, -1, &format!("{shape:?} down from {level}"));
        // the way back: the flight beneath, with the old level kept resident
        g.keep_stack(&content, &catalog);
        assert!(g.inf.as_ref().unwrap().stack.has(level), "{shape:?}: the level above stays resident after the crossing");
        let up_stair = nearest(&g, 1, shape).expect("the flight beneath leads back up with the same shape");
        let up = walk_recorded(&mut g, up_stair, &content, &catalog, MAX_FRAMES);
        check_crossing(&up, target, 1, &format!("{shape:?} up from {target}"));
        assert_eq!(g.inf.as_ref().unwrap().level, level, "{shape:?}: back on the level it started on");
        ran += 1;
    }
    assert!(ran > 0, "at least one flight shape was found and walked");
}

/// `Game::relabel` directly, on a body mid-flight: x, z, y, velocity and facing are exactly what they were.
#[test]
fn relabel_keeps_x_z_y_velocity_and_facing_exactly() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let Some((mut g, s)) = window_with(FlightShape::Switchback, 0, &content, &catalog) else { return };
    let path = absolute_path(&g, s);
    let mid = path[path.len() / 2];
    place_on(&mut g, s, mid);
    g.p.yaw = 1.234;
    g.p.move_vel = (0.7, -1.9);
    g.fade = 0.0;
    let (pos, y, yaw, mv, fade, floor_before) = (g.p.pos, g.p.y(), g.p.yaw, g.p.move_vel, g.fade, g.p.floor_y);
    let (builds, center, level) = {
        let i = g.inf.as_ref().unwrap();
        (i.builds, i.center, i.level)
    };
    // a level that is not resident is refused with no change
    assert!(!g.relabel(level - 3, &content, &catalog), "a non-resident level is not relabelled");
    assert_eq!((g.p.pos, g.p.yaw, g.inf.as_ref().unwrap().level), (pos, yaw, level), "a refused relabel changes nothing");
    assert!(g.relabel(level - 1, &content, &catalog), "the resident neighbour is relabelled");
    let i = g.inf.as_ref().unwrap();
    assert_eq!(i.level, level - 1);
    assert_eq!(g.fac.level, level - 1, "fac.level follows");
    assert_eq!(g.p.pos, pos, "x/z exactly");
    assert!((g.p.y() - y).abs() < 1e-4, "absolute y: {} vs {y}", g.p.y());
    assert!((g.p.floor_y - floor_before).abs() > 4.9, "the floor datum moved one storey");
    assert_eq!(g.p.yaw, yaw, "facing exactly");
    assert_eq!(g.p.move_vel, mv, "velocity exactly");
    assert_eq!(g.fade, fade, "no fade");
    assert_eq!(i.builds, builds, "no window is built");
    assert_eq!(i.center, center, "the window does not move");
    assert!(i.stack.residents.iter().all(|r| r.center == i.center));
    assert!(i.stack.has(level), "the old active level is now the resident neighbour");
}

/// Oscillating around the hysteresis band does not flip the label back and forth.
#[test]
fn walking_back_down_mid_flight_does_not_relabel_twice() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let Some((mut g, s)) = window_with(FlightShape::Switchback, 0, &content, &catalog) else { return };
    // walk until the label flips, then wiggle forward and back on the spot
    let path = absolute_path(&g, s);
    place_on(&mut g, s, path[0]);
    let mut idx = 1;
    let fwd = Controls { fwd: 1.0, ..Default::default() };
    let back = Controls { fwd: -1.0, ..Default::default() };
    let start = g.inf.as_ref().unwrap().level;
    let mut yaw = g.p.yaw;
    for _ in 0..MAX_FRAMES {
        let o = origin(&g);
        while idx + 1 < path.len() && pdist((path[idx].0 - o.0, path[idx].1 - o.1), g.p.pos) <= 0.3 {
            idx += 1;
        }
        let (dx, dz) = (path[idx].0 - o.0 - g.p.pos.0, path[idx].1 - o.1 - g.p.pos.1);
        yaw = (-dx).atan2(-dz);
        g.p.yaw = yaw;
        g.step(FRAME, &fwd, &content, &catalog, &Sounds::default());
        if g.inf.as_ref().unwrap().level != start {
            break;
        }
    }
    assert_eq!(g.inf.as_ref().unwrap().level, start - 1, "the walk relabelled once");
    let mut flips = 0;
    let mut prev = g.inf.as_ref().unwrap().level;
    for f in 0..60 {
        g.p.yaw = yaw;
        let c = if (f / 3) % 2 == 0 { &back } else { &fwd };
        g.step(FRAME, c, &content, &catalog, &Sounds::default());
        let now = g.inf.as_ref().unwrap().level;
        if now != prev {
            flips += 1;
            prev = now;
        }
        assert_eq!(g.fac.level, now, "fac.level follows the label");
    }
    assert_eq!(flips, 0, "a wiggle of a few frames inside the hysteresis band relabels nothing");
}

/// A window slide (recentre) while the body is on a flight keeps the absolute y, the level and the residents' frame.
#[test]
fn recentre_while_on_a_flight_keeps_y_continuous() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let Some((mut g, s)) = window_with(FlightShape::Switchback, 0, &content, &catalog) else { return };
    let path = absolute_path(&g, s);
    place_on(&mut g, s, path[path.len() / 2]);
    let (y, abs, level) = (g.p.y(), (g.p.pos.0 + origin(&g).0, g.p.pos.1 + origin(&g).1), g.inf.as_ref().unwrap().level);
    // slide the window one chunk left and move the body one chunk right: the same world cell, so the next recentre
    // brings the window back and shifts every resident with it
    let chunk = crate::world::infinite::CHUNK as f32;
    {
        let inf = g.inf.as_mut().unwrap();
        inf.center.0 -= 1;
    }
    g.p.pos.0 += chunk;
    g.recentre_window(&content, &catalog);
    g.keep_stack(&content, &catalog);
    let o = origin(&g);
    assert!((g.p.y() - y).abs() < 1e-3, "absolute y across the recentre: {} vs {y}", g.p.y());
    assert_eq!(g.inf.as_ref().unwrap().level, level, "the level does not change");
    assert_eq!(g.fac.level, level);
    assert!(pdist((g.p.pos.0 + o.0, g.p.pos.1 + o.1), abs) < 1e-3, "the body is on the same world cell");
    let inf = g.inf.as_ref().unwrap();
    assert!(inf.stack.residents.iter().all(|r| r.center == inf.center), "every resident was re-framed with the window");
    // and the walk goes on from there with no pop
    let rest = walk_recorded(&mut g, s, &content, &catalog, 120);
    for w in rest.windows(2) {
        assert!((w[1].y - w[0].y).abs() <= MAX_STEP_DY);
    }
}

/// The lane divider of a switchback is a wall: walking sideways at several heights never pops y and never puts the
/// body inside the divider.
#[test]
fn switchback_divider_stops_a_sideways_cross() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let Some((mut g, s)) = window_with(FlightShape::Switchback, 0, &content, &catalog) else { return };
    let level = g.inf.as_ref().unwrap().level;
    assert!(!s.dividers(level).is_empty(), "a switchback has a lane divider");
    let local = s.walk_path(level, 0.25);
    let o = origin(&g);
    for k in 1..6 {
        let at = local[local.len() * k / 6];
        for side in [-1.0f32, 1.0] {
            g.p.pos = at;
            g.p.height = s.surface_rel(level, at.0, at.1).unwrap_or(0.0);
            g.p.floor_y = floor_y_of(level);
            g.p.feet_y = 0.0;
            g.p.move_vel = (0.0, 0.0);
            let next = local[(local.len() * k / 6 + 1).min(local.len() - 1)];
            g.p.yaw = (-(next.0 - at.0)).atan2(-(next.1 - at.1));
            let c = Controls { strafe: side, ..Default::default() };
            let mut prev_y = g.p.y();
            for f in 0..45 {
                g.step(FRAME, &c, &content, &catalog, &Sounds::default());
                let y = g.p.y();
                assert!((y - prev_y).abs() <= MAX_STEP_DY, "k={k} side={side} frame {f}: y pops {prev_y} -> {y}");
                prev_y = y;
                assert_eq!(g.inf.as_ref().unwrap().origin(), (o.0 as i32, o.1 as i32), "the window does not move");
                assert!(
                    !g.fac.divider_blocks(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS * 0.5),
                    "k={k} side={side} frame {f}: the body never ends inside the divider"
                );
            }
        }
    }
}

/// The golden seeds of world/stack_contract_tests.rs (the room texts the contract is pinned on).
const GOLDEN: [&str; 3] = ["1426541270", "k7qxm", "demo"];
/// The most flights walked per seed and direction (the walk of one flight costs a window build and a stack).
const FLIGHTS_PER_SEED_DIR: usize = 4;
/// A walked step is no faster than the walking speed (m/s) times the frame, plus slack.
const SPEED_SLACK_Y: f32 = 0.02;

/// Absolute cell of a stair in the window of `g` (window origin added), so it is found again after a recentre.
fn abs_cell(g: &Game, s: &Stair) -> (i32, i32) {
    let o = g.inf.as_ref().map_or((0, 0), |i| i.origin());
    (s.cell.x + o.0, s.cell.y + o.1)
}

/// The non-ladder flights of a fresh window of `seed` on `level` going `dir`, as absolute cells.
fn flights_of(seed: &str, level: i32, dir: i32, content: &Content, catalog: &Catalog) -> Vec<(i32, i32)> {
    let g = Game::new_infinite_sim(Difficulty::Standard, 7, seed, level, content, catalog);
    g.fac.stairs.iter().filter(|s| s.dir == dir && !s.is_ladder() && matches!(s.kind, LinkKind::Stairs)).map(|s| abs_cell(&g, s)).collect()
}

/// A fresh window of `seed` on `level` with the body primed on the flight at absolute cell `at`, the recentre done and
/// the whole keep set resident (as a runner standing at a landing has it). `None` when the flight is gone after the recentre.
fn primed(seed: &str, level: i32, dir: i32, at: (i32, i32), content: &Content, catalog: &Catalog) -> Option<(Game, Stair)> {
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, seed, level, content, catalog);
    let o = origin(&g);
    g.p.pos = (at.0 as f32 - o.0 + 0.5, at.1 as f32 - o.1 + 0.5);
    g.step(FRAME, &Controls::default(), content, catalog, &Sounds::default());
    for _ in 0..8 {
        g.keep_stack(content, catalog);
    }
    let pick = g
        .fac
        .stairs
        .iter()
        .copied()
        .filter(|s| s.dir == dir && !s.is_ladder() && matches!(s.kind, LinkKind::Stairs) && abs_cell(&g, s) == at)
        .next()?;
    Some((g, pick))
}

/// Every non-ladder flight of the golden seeds (start window and levels -1, -2; up to 4 per seed and direction) is
/// walked down from its landing and up from its foot: absolute y never pops, the body moves at walking speed, the two
/// ends sit on the two floors, and the label flips exactly once per crossing.
#[test]
fn every_flight_in_the_golden_seeds_walks_up_and_down_with_y_continuity() {
    let started = std::time::Instant::now();
    let (content, catalog) = (Content::load(), Catalog::load());
    let max_h = crate::config::PLAYER_WALK_SPEED * FRAME * 1.5 + 1e-3;
    let max_dy = crate::config::PLAYER_WALK_SPEED * FRAME + SPEED_SLACK_Y;
    let mut walked = 0usize;
    for seed in GOLDEN {
        for dir in [-1, 1] {
            let mut n = 0;
            'levels: for level in [0, -1, -2] {
                for at in flights_of(seed, level, dir, &content, &catalog) {
                    if n >= FLIGHTS_PER_SEED_DIR {
                        break 'levels;
                    }
                    let Some((mut g, s)) = primed(seed, level, dir, at, &content, &catalog) else {
                        eprintln!("flight {at:?} of {seed} level {level} gone after the recentre; skipped");
                        continue;
                    };
                    let what = format!("seed {seed} level {level} dir {dir} flight at {at:?} ({:?})", s.shape);
                    let center0 = g.inf.as_ref().unwrap().center;
                    let recs = walk_recorded(&mut g, s, &content, &catalog, MAX_FRAMES);
                    assert!(recs.len() > 20, "{what}: the walk stepped frames");
                    let end_level = level + dir;
                    let (first, last) = (recs[0], recs[recs.len() - 1]);
                    assert!((first.y - floor_y_of(level)).abs() <= FLOOR_TOL, "{what}: starts on its floor: {}", first.y);
                    assert!((last.y - floor_y_of(end_level)).abs() <= FLOOR_TOL, "{what}: ends on the floor it reaches: {}", last.y);
                    let mut flips = Vec::new();
                    for (i, w) in recs.windows(2).enumerate() {
                        let (a, b) = (&w[0], &w[1]);
                        assert!((b.y - a.y).abs() <= MAX_STEP_DY, "{what}: frame {i}: y pops {} -> {}", a.y, b.y);
                        assert!((b.y - a.y).abs() <= max_dy, "{what}: frame {i}: dy {} exceeds walking speed * dt", (b.y - a.y).abs());
                        assert!(hstep(a, b) <= max_h, "{what}: frame {i}: x/z step {} exceeds walking speed * 1.5", hstep(a, b));
                        if b.level != a.level {
                            flips.push(i + 1);
                        }
                    }
                    assert_eq!(flips.len(), 1, "{what}: the label flips exactly once: {flips:?}");
                    assert_eq!(recs[flips[0]].level, end_level, "{what}: the label moves one level the right way");
                    assert_eq!(g.inf.as_ref().unwrap().center, center0, "{what}: the window never moves for a walked flight");
                    n += 1;
                    walked += 1;
                }
            }
            eprintln!("seed {seed} dir {dir}: walked {n} flights");
        }
    }
    eprintln!("walked {walked} flights in {:.1}s", started.elapsed().as_secs_f32());
    assert!(walked > 0, "at least one flight was walked");
}

/// Walking a flight with its neighbour resident builds no window: `inf.builds` is the same on the crossing frame as at
/// the start, and the window does not move.
#[test]
fn walking_a_flight_never_calls_load_window() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let Some((mut g, s)) = window_with(FlightShape::Switchback, 0, &content, &catalog) else { return };
    let builds = g.inf.as_ref().unwrap().builds;
    let center = g.inf.as_ref().unwrap().center;
    let recs = walk_recorded(&mut g, s, &content, &catalog, MAX_FRAMES);
    let k = recs.iter().position(|r| r.level != recs[0].level).expect("the walk relabelled");
    for (i, r) in recs[..=k].iter().enumerate() {
        assert_eq!(r.builds, builds, "frame {i}: no window was built up to and on the crossing");
        assert_eq!(r.center, center, "frame {i}: the window did not move");
    }
}

// ---- phase 9 audit tests: the level-swap machinery is gone ----------------------------------------------------------
//
// Behavioural proof that no path moves the body on a label change is the existing continuity tests, kept as the gate:
// relabel_keeps_x_z_y_velocity_and_facing_exactly, every_flight_in_the_golden_seeds_walks_up_and_down_with_y_continuity,
// walking_a_flight_never_calls_load_window, endless::a_ladder_climb_down_calls_no_load_window and
// endless::a_walk_off_the_lip_falls_through_the_relabel_with_no_pos_rewrite. The tests below add the source audits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every `.rs` file under src/bin/last_route as (path relative to it with `/`, lines with `//` comment lines blanked).
fn sources() -> Vec<(String, Vec<String>)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("readable source dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/last_route");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            let text = std::fs::read_to_string(&p).expect("readable source file");
            let lines = text
                .lines()
                .map(|l| if l.trim_start().starts_with("//") { String::new() } else { l.to_string() })
                .collect();
            (rel, lines)
        })
        .collect()
}

/// The name of the fn enclosing line `i`: the nearest `fn ` at or above it.
fn enclosing_fn(lines: &[String], i: usize) -> String {
    for j in (0..=i).rev() {
        let l = &lines[j];
        if let Some(at) = l.find("fn ") {
            let name: String = l[at + 3..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() && (at == 0 || !l[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_')) {
                return name;
            }
        }
    }
    String::new()
}

/// Callers of the storey change. Allowed: `respawn_on_level` (the admin jump and any future respawn: an explicit
/// teleport, not a walk) and `go_to_level` (the cfg(test) helper of the tests). Checkpoint resume and a new run use
/// `enter_infinite` / `load_window`, which the load_window audit covers.
#[test]
fn change_storey_has_only_the_allowed_callers() {
    let needle = concat!("change_", "storey", "(");
    let mut found: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (file, lines) in sources() {
        for (i, l) in lines.iter().enumerate() {
            if l.contains(needle) {
                found.entry((file.clone(), enclosing_fn(&lines, i))).or_default().push(i + 1);
            }
        }
    }
    let mut by_fn: BTreeMap<String, usize> = BTreeMap::new();
    for ((file, f), at) in &found {
        *by_fn.entry(f.clone()).or_default() += at.len();
        if f == "change_storey" {
            assert_eq!(file, "game/endless.rs", "the definition lives in game/endless.rs");
        }
    }
    let want: BTreeMap<String, usize> = ["change_storey", "go_to_level", "respawn_on_level"].iter().map(|s| (s.to_string(), 1)).collect();
    assert_eq!(by_fn, want, "unexpected callers, (file, fn) -> lines: {found:?}");
}

/// The retired swap machinery has no code left, only (at most) comments.
#[test]
fn no_swap_machinery_names_are_left() {
    let names = [
        concat!("take_", "stairs"),
        concat!("walked_", "level_change"),
        concat!("place_beside_", "ladder"),
        concat!("start_", "stairs"),
        concat!("take_", "flight"),
        concat!("prefetch_", "connections"),
        concat!("pre", "fetched"),
        concat!("store_level0_", "hunters"),
        concat!("store_floor_", "hunters"),
        concat!("restore_level_", "hunters"),
        concat!("LEVEL_CHANGE_", "HEIGHT"),
        concat!("flight_", "steps"),
        concat!("landing_", "offset"),
        concat!("climb_", "lift"),
    ];
    let mut hits = Vec::new();
    for (file, lines) in sources() {
        for (i, l) in lines.iter().enumerate() {
            for n in names {
                if l.contains(n) {
                    hits.push(format!("{file}:{} {n}", i + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "retired names still in code: {hits:?}");
}

/// The body of `fn name(` in `file`: from its line to the next line that is exactly four spaces and `}`.
fn body_of(sources: &[(String, Vec<String>)], file: &str, name: &str) -> Vec<String> {
    let (_, lines) = sources.iter().find(|(f, _)| f == file).unwrap_or_else(|| panic!("{file} exists"));
    let head = format!("fn {name}(");
    let start = lines.iter().position(|l| l.contains(&head)).unwrap_or_else(|| panic!("{file} has {head}"));
    let end = lines[start..].iter().position(|l| l.trim_end() == "    }").map_or(lines.len(), |k| start + k);
    lines[start..end].to_vec()
}

/// `load_window` is called only by enter_infinite, regen_window, recentre_window, change_storey and tests, and the
/// relabel functions never move the body or build anything (`rebase_floor` is allowed: it re-expresses the same y).
#[test]
fn load_window_has_only_the_allowed_callers() {
    let src = sources();
    let allowed = ["enter_infinite", "regen_window", "recentre_window", "change_storey"];
    let mut bad = Vec::new();
    for (file, lines) in &src {
        let in_tests_file = file.ends_with("tests.rs");
        let tests_from = lines.iter().position(|l| l.trim_start().starts_with("mod tests"));
        for (i, l) in lines.iter().enumerate() {
            if !(l.contains("self.load_window(") || l.contains("g.load_window(")) {
                continue;
            }
            let in_tests = in_tests_file || tests_from.is_some_and(|t| i > t);
            if !in_tests && !allowed.contains(&enclosing_fn(lines, i).as_str()) {
                bad.push(format!("{file}:{}", i + 1));
            }
        }
    }
    assert!(bad.is_empty(), "load_window called outside the allowed callers: {bad:?}");
    let banned = ["p.pos =", ".place(", "set_abs_y", "nearest_open", "load_window", concat!("change_", "storey")];
    for (file, name) in [("game/endless.rs", "relabel"), ("game/endless.rs", "walk_relabel"), ("game/resident_hunters.rs", "relabel_hunters")] {
        for l in body_of(&src, file, name) {
            for b in banned {
                assert!(!l.contains(b), "{file} fn {name} must not touch the body or build a window, but has {b:?}: {l}");
            }
        }
    }
}

/// A label change saves the run once, on the relabel frame, and nothing else on the way (the ladder start, a climb, a
/// fall or the walk) does.
#[test]
fn a_label_change_autosaves_once_and_nothing_else_does() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let Some((mut g, s)) = window_with(FlightShape::Switchback, 0, &content, &catalog) else { return };
    g.last_autosave = "";
    let mut saves: Vec<&'static str> = Vec::new();
    let recs = walk_recorded_with(&mut g, s, &content, &catalog, MAX_FRAMES, |g| {
        saves.push(g.last_autosave);
        g.last_autosave = "";
    });
    saves.push(g.last_autosave);
    assert_eq!(saves.len(), recs.len(), "one autosave reading per record");
    let mut changes = 0;
    for i in 0..recs.len() {
        let relabelled = i > 0 && recs[i].level != recs[i - 1].level;
        changes += relabelled as usize;
        let want = if relabelled { "level" } else { "" };
        assert_eq!(saves[i], want, "frame {i}: autosave only on the relabel frame");
    }
    assert_eq!(changes, 1, "the walk relabelled exactly once");
}

/// An admin jump (and any respawn) rebuilds the floor: the body lands on the start cell of that level, standing on
/// its floor, the label changes once, one announce, one autosave.
#[test]
fn respawn_on_level_moves_the_body_to_the_start_and_changes_the_label_once() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-RESPAWN", 0, &content, &catalog);
    g.last_autosave = "";
    g.toast.clear();
    let to = -1;
    assert!(g.respawn_on_level(to, &content, &catalog), "a host respawns on an allowed level");
    assert_eq!(g.inf.as_ref().unwrap().level, to, "the label is the new level");
    let start = g.fac.start.center();
    assert!(pdist(g.p.pos, start) < 0.75, "the body is on the start cell: {:?} vs {:?}", g.p.pos, start);
    assert!((g.p.y() - floor_y_of(to)).abs() < FLOOR_TOL, "the body stands on the floor of {to}: y {}", g.p.y());
    assert_eq!(g.toast, format!("LEVEL {to}"), "one relabel announce");
    assert_eq!(g.last_autosave, "level", "one autosave for the label change");
}

#[test]
fn a_guest_cannot_respawn_on_a_level() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-RESPAWN-GUEST", 0, &content, &catalog);
    g.remote_hunters = true;
    g.last_autosave = "";
    let (pos, level) = (g.p.pos, g.inf.as_ref().unwrap().level);
    assert!(!g.respawn_on_level(-1, &content, &catalog), "a guest is refused");
    assert_eq!(g.inf.as_ref().unwrap().level, level, "the label did not change");
    assert_eq!(g.p.pos, pos, "the body did not move");
    assert_eq!(g.last_autosave, "", "nothing was saved");
}
