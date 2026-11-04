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
