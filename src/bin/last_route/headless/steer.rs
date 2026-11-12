//! A steered walker for stair flights. A flight is no longer a straight ramp (switchback, spiral), so walking forward
//! does not follow it: the walker faces the next point of the flight's own walk path (`Stair::walk_path`) and holds
//! forward, through the game's own frame (`Game::step`). The levels are one continuous space: the walk never rebuilds the
//! floor or moves the body, and `stop_on_level_change` stops on the frame the derived label changes. Used by the `walk_flight` protocol command and the flight
//! tests (flight_tests.rs).

use crate::audio::Sounds;
use crate::content::Content;
use crate::game::level_stack::floor_y_of;
use crate::game::{Controls, Game};
use crate::world::catalog::Catalog;
use crate::world::facility::Stair;
use serde_json::{json, Value};

/// One frame of the walk, as the game runs it.
const FRAME: f32 = 1.0 / 60.0;
/// A waypoint is reached when the body is this close (metres).
const REACHED: f32 = 0.3;
/// The walk path is sampled this far apart (metres).
const PATH_STEP: f32 = 0.25;

/// What a walk along one flight did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlightWalk {
    /// Frames stepped.
    pub frames: usize,
    /// The body came within reach of the last waypoint.
    pub reached_end: bool,
    /// How many times `inf.level` changed.
    pub level_changes: usize,
    /// The largest frame-to-frame change of the absolute feet y.
    pub max_step_dy: f32,
    pub start_y: f32,
    pub end_y: f32,
    pub start_level: i32,
    pub end_level: i32,
    /// Frames on which a fade was running (must be 0).
    pub fades: usize,
}

impl FlightWalk {
    pub fn to_json(&self) -> Value {
        json!({
            "frames": self.frames,
            "reached_end": self.reached_end,
            "level_changes": self.level_changes,
            "max_step_dy": self.max_step_dy,
            "start_y": self.start_y,
            "end_y": self.end_y,
            "start_level": self.start_level,
            "end_level": self.end_level,
            "fades": self.fades,
        })
    }
}

fn level_of(g: &Game) -> i32 {
    g.inf.as_ref().map_or(0, |i| i.level)
}

fn origin_of(g: &Game) -> (f32, f32) {
    g.inf.as_ref().map_or((0.0, 0.0), |i| {
        let o = i.origin();
        (o.0 as f32, o.1 as f32)
    })
}

/// The walk path of `s` in ABSOLUTE cells (window origin added), from the stair's own end to its far end: a stair going
/// down (`dir < 0`) starts at its landing A, a stair going up (`dir > 0`) at its foot B, so the path is reversed for it.
pub fn absolute_path(g: &Game, s: Stair) -> Vec<(f32, f32)> {
    let level = level_of(g);
    let mut path = s.walk_path(level, PATH_STEP);
    if s.dir > 0 {
        path.reverse();
    }
    let o = origin_of(g);
    path.into_iter().map(|p| (p.0 + o.0, p.1 + o.1)).collect()
}

/// Walks the flight `s` (a stair of the current facility) from its own end to its far end for at most `max_frames`
/// frames. Places the body on the first waypoint, then every frame faces the next waypoint and holds forward.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn walk_flight(g: &mut Game, s: Stair, content: &Content, catalog: &Catalog, max_frames: usize) -> FlightWalk {
    walk_flight_until(g, s, content, catalog, max_frames, false)
}

/// As `walk_flight`; with `stop_on_level_change` the walk ends on the frame `inf.level` changes.
pub fn walk_flight_until(
    g: &mut Game,
    s: Stair,
    content: &Content,
    catalog: &Catalog,
    max_frames: usize,
    stop_on_level_change: bool,
) -> FlightWalk {
    walk_flight_trace(g, s, content, catalog, max_frames, stop_on_level_change).0
}

/// As `walk_flight`, returning the absolute feet y after every stepped frame (the first entry is the start y). The walk
/// is the walked relabel of the continuous levels: no interact press, the level label flips by itself.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn walk_flight_abs_y(g: &mut Game, s: Stair, content: &Content, catalog: &Catalog, max_frames: usize) -> Vec<f32> {
    walk_flight_trace(g, s, content, catalog, max_frames, false).1
}

fn walk_flight_trace(
    g: &mut Game,
    s: Stair,
    content: &Content,
    catalog: &Catalog,
    max_frames: usize,
    stop_on_level_change: bool,
) -> (FlightWalk, Vec<f32>) {
    let path = absolute_path(g, s);
    let mut w = FlightWalk::default();
    let mut trace: Vec<f32> = Vec::new();
    let Some(first) = path.first().copied() else {
        return (w, trace);
    };
    let level = level_of(g);
    let o = origin_of(g);
    g.p.pos = (first.0 - o.0, first.1 - o.1);
    g.p.height = s.surface_rel(level, g.p.pos.0, g.p.pos.1).unwrap_or(0.0);
    g.p.floor_y = floor_y_of(level);
    g.p.feet_y = 0.0;
    g.p.pitch = 0.0;
    g.fade = 0.0;
    w.start_y = g.p.y();
    w.start_level = level;
    w.end_level = level;
    trace.push(w.start_y);
    let mut prev_y = w.start_y;
    let mut prev_level = level;
    let mut idx = 1.min(path.len() - 1);
    let c = Controls { fwd: 1.0, ..Default::default() };
    let sounds = Sounds::default();
    for frame in 0..=max_frames {
        // the window recentres on the first frames and the level swap keeps the cell: subtract the origin each frame
        let o = origin_of(g);
        let (mut dx, mut dz);
        loop {
            let t = path[idx];
            dx = t.0 - o.0 - g.p.pos.0;
            dz = t.1 - o.1 - g.p.pos.1;
            if (dx * dx + dz * dz).sqrt() > REACHED {
                break;
            }
            if idx + 1 >= path.len() {
                w.reached_end = true;
                break;
            }
            idx += 1;
        }
        if w.reached_end || frame == max_frames {
            break;
        }
        g.p.yaw = (-dx).atan2(-dz);
        g.step(FRAME, &c, content, catalog, &sounds);
        w.frames += 1;
        let y = g.p.y();
        w.max_step_dy = w.max_step_dy.max((y - prev_y).abs());
        prev_y = y;
        trace.push(y);
        let now = level_of(g);
        if now != prev_level {
            w.level_changes += 1;
            prev_level = now;
            if stop_on_level_change {
                break;
            }
        }
        if g.fade > 0.0 {
            w.fades += 1;
        }
    }
    w.end_y = g.p.y();
    w.end_level = level_of(g);
    (w, trace)
}
