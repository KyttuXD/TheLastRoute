//! Real-entry regression tests for the stair links (TODO: "stairs do not link to the other floors").
//!
//! `stack_walk_tests` places the body ON the flight (height, floor and feet hand-set), which hides any bug at the
//! entry cell. These tests start in the hall behind the landing (or beyond the foot), set only the floor the body
//! stands on, and walk in with nothing but `Controls`: the label, the absolute y and the residency must all follow by
//! themselves, at the frame rates the app can run (it clamps dt to 0.05) and in every gait.
//!
//! Registered by game/mod.rs as `#[cfg(test)] mod stack_entry_tests;`.

use super::level_stack::floor_y_of;
use super::player::stair_height_at;
use super::{Controls, Game};
use crate::audio::Sounds;
use crate::config::Difficulty;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::{Cell, LinkKind, Stair};
use crate::world::flight::well_footprint;

/// (level the walk starts on, direction): every storey pair the game has a flight for.
const LEGS: [(i32, i32); 6] = [(0, -1), (-1, 1), (-1, -1), (-2, -1), (-2, 1), (-3, 1)];
/// Flights walked per seed and leg in the base sweeps.
const FLIGHTS_PER_LEG: usize = 2;
/// The most the absolute feet y may move in one frame (m).
const MAX_STEP_DY: f32 = 0.25;
/// The body ends on the floor it walks to within this (m).
const FLOOR_TOL: f32 = 0.05;
/// A waypoint counts as reached within this many metres.
const REACHED: f32 = 0.3;
const MAX_FRAMES: usize = 2400;
/// The app clamps dt to this (app/mod.rs).
const APP_MAX_DT: f32 = 0.05;
/// Frames between jump presses in the jumping gait.
const JUMP_EVERY: usize = 20;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Gait {
    Walk,
    Sprint,
    Jump,
    Crouch,
}

fn origin(g: &Game) -> (i32, i32) {
    g.inf.as_ref().expect("an infinite run").origin()
}

fn stairs_of(g: &Game, dir: i32) -> Vec<Stair> {
    g.fac.stairs.iter().copied().filter(|s| s.dir == dir && matches!(s.kind, LinkKind::Stairs)).collect()
}

/// The clear hall cell of `s` farthest from the flight end the body walks in at (the landing going down, the foot
/// going up), and that end.
fn far_hall_cell(s: &Stair, level: i32) -> Option<(Cell, Cell)> {
    let fp = well_footprint(&s.flight(level)?);
    let (clear, end) = if s.dir < 0 { (fp.upper_clear, s.landing()) } else { (fp.lower_clear, s.foot()) };
    let start = clear.iter().copied().max_by_key(|c| (c.x - end.x).abs() + (c.y - end.y).abs())?;
    Some((start, end))
}

fn controls(gait: Gait, frame: usize) -> Controls {
    Controls {
        fwd: 1.0,
        sprint: gait == Gait::Sprint,
        crouch: gait == Gait::Crouch,
        jump: gait == Gait::Jump && frame % JUMP_EVERY == 0,
        ..Default::default()
    }
}

/// What one walk from the hall found.
struct Walk {
    end_y: f32,
    end_level: i32,
    reached: bool,
    max_dy: f32,
    /// A frame on which the body was over a flight but in a fall.
    fell_on_flight: bool,
    /// A fall landing the walk left (fall damage).
    landed: bool,
    /// Frames over a flight with the feet off the ground (no footsteps, no head bob) outside the jumping gait.
    ungrounded: usize,
    toast: String,
    toast_t: f32,
    /// Whether the level beyond was resident the first frame the body stood over the flight.
    far_level_ready: Option<bool>,
}

/// Walks flight number `idx` (of the `dir` flights on `level`) from the hall into its far floor.
fn walk_from_hall(seed: &str, level: i32, dir: i32, idx: usize, dt: f32, gait: Gait, c: &Content, cat: &Catalog) -> Option<Walk> {
    walk_from_hall_with(seed, level, dir, idx, dt, gait, false, c, cat)
}

/// The cold start: every resident the preload built is dropped once the body is in the hall.
fn walk_cold(seed: &str, level: i32, dir: i32, c: &Content, cat: &Catalog) -> Option<Walk> {
    walk_from_hall_with(seed, level, dir, 0, APP_MAX_DT, Gait::Walk, true, c, cat)
}

#[allow(clippy::too_many_arguments)]
fn walk_from_hall_with(seed: &str, level: i32, dir: i32, idx: usize, dt: f32, gait: Gait, cold: bool, c: &Content, cat: &Catalog) -> Option<Walk> {
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, seed, level, c, cat);
    let s = *stairs_of(&g, dir).get(idx)?;
    let (start, end) = far_hall_cell(&s, level)?;
    // the first step recentres the window by whole chunks: every target is converted by the origin shift
    let o_before = origin(&g);
    let mut targets: Vec<(f32, f32)> = vec![end.center()];
    let mut path = s.walk_path(level, 0.25);
    if s.dir > 0 {
        path.reverse();
    }
    targets.extend(path);
    // only the floor the body stands on is set: no height, no feet, no label
    g.p.pos = start.center();
    g.p.floor_y = floor_y_of(level);
    g.step(dt, &Controls::default(), c, cat, &Sounds::default());
    if cold {
        g.inf.as_mut().unwrap().stack.retain_levels(&[]);
    }
    let mut w = Walk {
        end_y: g.p.y(),
        end_level: level,
        reached: false,
        max_dy: 0.0,
        fell_on_flight: false,
        landed: false,
        ungrounded: 0,
        toast: String::new(),
        toast_t: 0.0,
        far_level_ready: None,
    };
    let (mut i, mut last_y) = (0usize, g.p.y());
    for f in 0..MAX_FRAMES {
        let o = origin(&g);
        let (shift_x, shift_z) = ((o.0 - o_before.0) as f32, (o.1 - o_before.1) as f32);
        let t = targets[i];
        let (dx, dz) = (t.0 - shift_x - g.p.pos.0, t.1 - shift_z - g.p.pos.1);
        if (dx * dx + dz * dz).sqrt() < REACHED {
            if i + 1 >= targets.len() {
                w.reached = true;
                break;
            }
            i += 1;
            continue;
        }
        g.p.yaw = (-dx).atan2(-dz);
        g.step(dt, &controls(gait, f), c, cat, &Sounds::default());
        w.max_dy = w.max_dy.max((g.p.y() - last_y).abs());
        last_y = g.p.y();
        if stair_height_at(&g.fac, g.p.pos).is_some() {
            if w.far_level_ready.is_none() {
                w.far_level_ready = Some(g.inf.as_ref().unwrap().stack.has(level + dir));
            }
            w.fell_on_flight |= g.p.falling();
            w.ungrounded += (!g.p.grounded && gait != Gait::Jump) as usize;
        }
        w.landed |= g.p.take_landing().is_some();
    }
    w.end_y = g.p.y();
    w.end_level = g.inf.as_ref().unwrap().level;
    w.toast = g.toast.clone();
    w.toast_t = g.toast_t;
    Some(w)
}

fn sweep(dt: f32, gait: Gait, flights_per_leg: usize, seeds: &[u32]) {
    let (c, cat) = (Content::load(), Catalog::load());
    let mut walked = 0;
    for n in seeds {
        let seed = format!("ROOM-STACK-{n}");
        for (level, dir) in LEGS {
            for idx in 0..flights_per_leg {
                let Some(w) = walk_from_hall(&seed, level, dir, idx, dt, gait, &c, &cat) else { continue };
                let what = format!("{seed} level {level} dir {dir} flight {idx} dt {dt} {gait:?}");
                assert!(w.reached, "{what}: the body never got to the far end (y {})", w.end_y);
                assert!((w.end_y - floor_y_of(level + dir)).abs() <= FLOOR_TOL, "{what}: ends at y {} not {}", w.end_y, floor_y_of(level + dir));
                assert_eq!(w.end_level, level + dir, "{what}: the label follows the body");
                assert!(w.max_dy <= MAX_STEP_DY, "{what}: y popped {} in one frame", w.max_dy);
                assert!(!w.fell_on_flight, "{what}: in a fall over a flight");
                assert_eq!(w.ungrounded, 0, "{what}: frames over a flight with the feet off the ground (the footsteps stop)");
                assert!(!w.landed, "{what}: a walk over a flight took fall damage");
                assert_eq!(w.far_level_ready, Some(true), "{what}: the far level is resident when the body first stands over the well");
                assert_eq!(w.toast, format!("LEVEL {}", level + dir), "{what}: the relabel toast");
                walked += 1;
            }
        }
    }
    assert!(walked >= 4, "{walked} flights walked: the sweep found too few");
}

#[test]
fn real_entry_walks_every_flight_from_the_hall() {
    sweep(1.0 / 60.0, Gait::Walk, FLIGHTS_PER_LEG, &[1, 2, 3]);
}

#[test]
fn real_entry_works_at_30_fps() {
    sweep(1.0 / 30.0, Gait::Walk, FLIGHTS_PER_LEG, &[1, 2, 3]);
}

#[test]
fn real_entry_works_at_the_app_clamped_20_fps() {
    sweep(APP_MAX_DT, Gait::Walk, FLIGHTS_PER_LEG, &[1, 2, 3]);
}

#[test]
fn real_entry_works_sprinting() {
    sweep(1.0 / 60.0, Gait::Sprint, 1, &[1, 2, 3]);
}

#[test]
fn real_entry_works_jumping() {
    sweep(1.0 / 60.0, Gait::Jump, 1, &[1, 2, 3]);
}

#[test]
fn real_entry_works_crouched() {
    sweep(1.0 / 60.0, Gait::Crouch, 1, &[4, 5, 6]);
}

/// Far approach: a level-0 flight reached by A* from the window start with no explicit residency call: the level
/// beneath is resident by the time the body stands over the well, and the walk ends a storey down.
#[test]
fn far_approach_reaches_a_level0_flight_with_the_stack_built_on_the_way() {
    let (c, cat) = (Content::load(), Catalog::load());
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-STACK-6", 0, &c, &cat);
    let o0 = origin(&g);
    let s = *stairs_of(&g, -1).first().expect("ROOM-STACK-6 has a flight going down on level 0");
    let (far_cell, end) = far_hall_cell(&s, 0).unwrap();
    let from = Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
    let path = g.fac.find_path(from, far_cell);
    assert!(!path.is_empty(), "the flight is reachable from the window start");
    let mut targets: Vec<(f32, f32)> = path.iter().map(|c| c.center()).collect();
    targets.push(end.center());
    targets.extend(s.walk_path(0, 0.25));
    let (mut i, mut reached, mut ready_over_well) = (0, false, None);
    for _ in 0..8000 {
        let o = origin(&g);
        let (sx, sz) = ((o.0 - o0.0) as f32, (o.1 - o0.1) as f32);
        let t = targets[i];
        let (dx, dz) = (t.0 - sx - g.p.pos.0, t.1 - sz - g.p.pos.1);
        if (dx * dx + dz * dz).sqrt() < REACHED {
            if i + 1 >= targets.len() {
                reached = true;
                break;
            }
            i += 1;
            continue;
        }
        g.p.yaw = (-dx).atan2(-dz);
        g.step(1.0 / 60.0, &controls(Gait::Walk, 1), &c, &cat, &Sounds::default());
        if ready_over_well.is_none() && stair_height_at(&g.fac, g.p.pos).is_some() {
            ready_over_well = Some(g.inf.as_ref().unwrap().stack.has(-1));
        }
    }
    assert!(reached, "the far approach got to the flight end, y {}", g.p.y());
    assert!((g.p.y() - floor_y_of(-1)).abs() <= FLOOR_TOL, "ends a storey down: y {}", g.p.y());
    assert_eq!(g.inf.as_ref().unwrap().level, -1);
    assert_eq!(ready_over_well, Some(true), "level -1 was resident when the body first stood over the well");
}

/// The worst entry: a window just loaded around the body, so nothing but the active level is resident, with the body
/// in the farthest hall cell and a slow frame rate. The far level must be there before the body stands over the well.
#[test]
fn the_neighbour_is_built_before_the_body_reaches_the_well() {
    let (c, cat) = (Content::load(), Catalog::load());
    let mut checked = 0;
    for n in 1..=6 {
        let seed = format!("ROOM-STACK-{n}");
        for (level, dir) in LEGS {
            let Some(w) = walk_cold(&seed, level, dir, &c, &cat) else { continue };
            let what = format!("{seed} level {level} dir {dir}");
            assert!(w.reached, "{what}: never reached the far end");
            assert_eq!(w.far_level_ready, Some(true), "{what}: level {} not resident when the body reached the well", level + dir);
            assert!((w.end_y - floor_y_of(level + dir)).abs() <= FLOOR_TOL, "{what}: ends at y {}", w.end_y);
            checked += 1;
        }
    }
    assert!(checked >= 3, "{checked} flights checked");
}

/// A run resumed from a checkpoint exactly as app/mod.rs `Loading::build` does it: the label from the checkpoint, the
/// window at its start cell, the body put at the saved absolute y. Returns the game and the y of every frame of
/// `secs` seconds of standing still.
fn resume(depth: i32, y: Option<f32>, secs: f32, c: &Content, cat: &Catalog) -> (Game, Vec<f32>) {
    use crate::shared::checkpoint::Checkpoint;
    let cp = Checkpoint {
        run_seed: 7,
        room_text: "ROOM-RESUME".into(),
        depth,
        score: 0,
        coins: 0,
        difficulty: Difficulty::Standard,
        infinite: true,
        y,
    };
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &cp.room_text, cp.level(), c, cat);
    g.p.set_abs_y(cp.migrated_y());
    let mut ys = vec![];
    for _ in 0..(secs * 60.0) as usize {
        g.step(1.0 / 60.0, &Controls::default(), c, cat, &Sounds::default());
        ys.push(g.p.y());
    }
    (g, ys)
}

/// A save with no `y_abs` (older build) resumes standing on the floor of its level.
#[test]
fn a_resumed_save_without_y_abs_stands_on_its_floor() {
    let (c, cat) = (Content::load(), Catalog::load());
    for depth in [0, -1, -2, -3] {
        let (g, ys) = resume(depth, None, 1.0, &c, &cat);
        assert_eq!(g.inf.as_ref().unwrap().level, depth);
        assert!(ys.iter().all(|y| (y - floor_y_of(depth)).abs() <= FLOOR_TOL), "level {depth}: stays on its floor");
        assert!(!g.p.falling());
    }
}

/// The relabel autosaves mid-flight (the body is already past the hysteresis, a couple of metres off its old floor).
/// The save stores no x/z, so the resume stands at the level's start cell: the body must come up standing on the floor
/// of the resumed level, not hang in the air above the start cell and fall.
#[test]
fn a_save_made_mid_flight_resumes_on_the_floor_not_in_the_air() {
    let (c, cat) = (Content::load(), Catalog::load());
    for (depth, y) in [(-1, floor_y_of(-1) + 2.6), (-1, floor_y_of(-1) - 2.6), (-2, floor_y_of(-2) + 2.6), (0, -2.6)] {
        let (g, ys) = resume(depth, Some(y), 2.0, &c, &cat);
        let level = g.inf.as_ref().unwrap().level;
        let last = *ys.last().unwrap();
        assert!(
            (last - floor_y_of(level)).abs() <= FLOOR_TOL,
            "saved y {y} depth {depth}: resumed on level {level} but at y {last} (floor {})",
            floor_y_of(level)
        );
        assert_eq!(g.p.stun, 0.0, "saved y {y}: the resume hit the ground hard (stun from a fall)");
        assert_eq!(g.p.noise_burst, 0.0, "saved y {y}: the resume made a landing noise");
        assert!(!g.p.falling());
    }
}

/// A flight whose lower level has no matching record would be a stair that leads nowhere: over 3x3 windows at 25
/// centres (so flights at chunk seams and window edges are met), every `Stairs` record has its twin on the other
/// level of the same window, with the same run and shape, and the twin's end cell is the foot or landing of this one.
#[test]
fn every_flight_record_has_its_twin_on_the_other_level_of_the_same_window() {
    let cat = Catalog::load();
    let loot = crate::content::loot::LootTables::default();
    let mut pairs = 0;
    for n in 1..=3 {
        // the chunk cache is keyed without the seed: one per seed
        let mut cache = std::collections::HashMap::new();
        let seed = crate::world::route::world_seed(&format!("ROOM-STACK-{n}"));
        for cx in -2..=2 {
            for cy in -2..=2 {
                for upper in [0, -1, -2] {
                    let (hi, _) = crate::world::infinite::window(&seed, (cx, cy), upper, &cat, &loot, &mut cache);
                    let (lo, _) = crate::world::infinite::window(&seed, (cx, cy), upper - 1, &cat, &loot, &mut cache);
                    for s in hi.stairs.iter().filter(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs)) {
                        let want = s.foot();
                        let twin = lo.stairs.iter().find(|t| t.dir > 0 && matches!(t.kind, LinkKind::Stairs) && t.cell == want);
                        let what = format!("seed {n} centre ({cx},{cy}) level {upper} flight at {:?}", s.cell);
                        let t = twin.unwrap_or_else(|| panic!("{what}: level {} has no record with its foot at {want:?}", upper - 1));
                        assert_eq!((t.run, t.shape, t.clockwise), (s.run, s.shape, s.clockwise), "{what}: the twin is another flight");
                        assert_eq!(t.landing(), s.landing(), "{what}: the two ends disagree on the landing");
                        pairs += 1;
                    }
                    for t in lo.stairs.iter().filter(|t| t.dir > 0 && matches!(t.kind, LinkKind::Stairs)) {
                        if !hi.stairs.iter().any(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs) && s.foot() == t.cell) {
                            panic!("seed {n} centre ({cx},{cy}) level {}: a flight foot at {:?} with no flight on the level above", upper - 1, t.cell);
                        }
                    }
                }
            }
        }
    }
    assert!(pairs >= 20, "{pairs} flight pairs checked");
}
