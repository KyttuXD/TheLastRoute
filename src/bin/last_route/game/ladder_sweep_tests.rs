//! A seed sweep of the manhole descent (the web build's "black screen on reaching the sewers", which came and went
//! with the room): ladders going down in the level 0 windows of many seeds are climbed by the scripted controls of the
//! headless runner, and the sewer each one arrives in is checked frame by frame. With `render` on, the real
//! `Game::update` runs (so `sync_world` builds the real `WorldMeshes` and `NeighbourWorlds` from `Assets::headless`, no
//! GL behind them) and the drawn set is checked every frame: it must hold the floor round the runner, its boxes must
//! hold its vertices, and it must be the live level's.
//!
//! The quick tests run in the normal suite. `LR_LADDER_SWEEP_SEEDS=200 cargo test --bin last_route ladder_sweep_long --
//! --ignored --nocapture` runs the long one (`LR_LADDER_SWEEP_TRACE=1` prints every climb).

use super::*;
use crate::audio::Sounds;
use crate::config::Difficulty;
use crate::content::Content;
use crate::game::ladder::{facing, run_of, step_center};
use crate::game::level_stack::{floor_y_of, ground_y};
use crate::shared::assets::Assets;
use crate::world::catalog::Catalog;
use crate::world::vertical::STOREY;

/// The window centres tried for each seed and the frame lengths a climb is run at (a slow machine steps with a long frame).
const CENTRES: [(i32, i32); 3] = [(0, 0), (1, 0), (-1, 1)];
const FRAME_LENGTHS: [f32; 3] = [1.0 / 60.0, 1.0 / 20.0, 0.1];
const NAMED: [&str; 4] = ["ROOM-TEST-7", "SEWER", "KYTTU", "ABCDE"];

fn texts(count: u32) -> Vec<String> {
    let mut v: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
    v.extend(NAMED.iter().map(|s| s.to_string()));
    v
}

/// What a climb is run with.
struct Rig {
    content: Content,
    catalog: Catalog,
    assets: Assets,
}

impl Rig {
    fn new() -> Rig {
        Rig { content: Content::load(), catalog: Catalog::load(), assets: Assets::headless() }
    }
}

fn fresh(text: &str, centre: (i32, i32), rig: &Rig) -> Game {
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 1, text, 0, &rig.content, &rig.catalog);
    g.admin.god = true;
    if centre != (0, 0) {
        g.inf.as_mut().unwrap().center = centre;
        g.regen_window(&rig.content, &rig.catalog);
    }
    g
}

/// One frame: the simulation alone, or the whole `update` (which also builds the render meshes) when `render` is on.
fn tick(g: &mut Game, c: &Controls, dt: f32, rig: &Rig, render: bool) {
    if render {
        g.update(dt, c, &rig.content, &rig.catalog, &rig.assets, &Sounds::default());
    } else {
        g.step(dt, c, &rig.content, &rig.catalog, &Sounds::default());
    }
}

fn finite(g: &Game) -> bool {
    g.p.pos.0.is_finite() && g.p.pos.1.is_finite() && g.p.y().is_finite() && g.p.eye_y().is_finite()
}

fn ladders_down(g: &Game) -> usize {
    g.fac.stairs.iter().filter(|s| s.is_ladder() && s.dir < 0).count()
}

/// What the climb measured: the slowest frame and the one the label flipped on (ms of native time), how many frames the
/// drawn set held no floor near the runner after the flip, and how many frames the world took to be whole again.
#[derive(Default, Debug, Clone, Copy)]
struct Stats {
    worst_ms: f64,
    flip_ms: f64,
    floorless_frames: usize,
    frames_to_whole: usize,
}

/// Checks the drawn world against the live game (call after `update`). Returns whether the drawn set holds a floor
/// surface within 3 m of the runner.
fn check_world(g: &Game) -> Result<bool, String> {
    let w = &g.world;
    if w.level != g.fac.level {
        return Err(format!("the drawn meshes are of level {} but the live floor is {}", w.level, g.fac.level));
    }
    if w.bounds.len() != w.meshes.len() {
        return Err(format!("{} boxes for {} meshes", w.bounds.len(), w.meshes.len()));
    }
    // every mesh of the active level and of the neighbours must fit one macroquad draw call (65536 vertices, 98304
    // indices: `main.rs` window_conf) and index only its own vertices, whole triangles (a 16-bit wrap draws garbage)
    for (what, m) in w.meshes.iter().map(|m| ("active", m)).chain(g.world_nb.items.iter().flat_map(|i| i.meshes.iter()).map(|m| ("neighbour", m))) {
        if m.vertices.len() >= 65536 || m.indices.len() >= 65536 * 3 / 2 {
            return Err(format!("a {what} mesh of {} vertices and {} indices overruns one draw call", m.vertices.len(), m.indices.len()));
        }
        if m.indices.len() % 3 != 0 {
            return Err(format!("a {what} mesh has {} indices (not whole triangles)", m.indices.len()));
        }
        if let Some(bad) = m.indices.iter().find(|i| **i as usize >= m.vertices.len()) {
            return Err(format!("a {what} mesh indexes vertex {bad} of {}", m.vertices.len()));
        }
    }
    let (px, pz) = g.p.pos;
    let mut floor = false;
    for (m, (lo, hi)) in w.meshes.iter().zip(&w.bounds) {
        for v in &m.vertices {
            let p = v.position;
            if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
                return Err(format!("a vertex at {p:?}"));
            }
            let e = 1e-3;
            if lo.x <= hi.x && (p.x < lo.x - e || p.x > hi.x + e || p.y < lo.y - e || p.y > hi.y + e || p.z < lo.z - e || p.z > hi.z + e) {
                return Err(format!("vertex {p:?} outside its box {lo:?} .. {hi:?}"));
            }
            if v.normal.y > 0.5 && (p.x - px).hypot(p.z - pz) < 3.0 && p.y > -1.5 && p.y < 0.3 {
                floor = true;
            }
        }
    }
    if !floor && std::env::var("LR_LADDER_SWEEP_TRACE").is_ok() {
        let verts: Vec<_> = w.meshes.iter().flat_map(|m| m.vertices.iter()).collect();
        let ys: Vec<f32> = verts.iter().map(|v| v.position.y).collect();
        let up = verts.iter().filter(|v| v.normal.y > 0.5).count();
        let near: Vec<_> = verts.iter().filter(|v| (v.position.x - px).hypot(v.position.z - pz) < 3.0).map(|v| (v.position.y, v.normal.y)).take(8).collect();
        eprintln!(
            "   no floor: level {} meshes {} verts {} up-facing {} y {:?}..{:?} pos {:?} dirty {} pending {} near {:?}",
            w.level, w.meshes.len(), verts.len(), up,
            ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max),
            g.p.pos, g.world_dirty, w.has_pending(), near
        );
    }
    Ok(floor)
}

/// Climbs the `n`th ladder down of the window from its step cell. Err(why) is the first failing check.
fn climb(text: &str, centre: (i32, i32), n: usize, dt: f32, rig: &Rig, render: bool) -> Result<Stats, String> {
    let mut g = fresh(text, centre, rig);
    let hatch = g
        .fac
        .stairs
        .iter()
        .copied()
        .filter(|s| s.is_ladder() && s.dir < 0)
        .nth(n)
        .ok_or("no such ladder")?;
    let run = run_of(&hatch);
    // the runner stands on the step cell facing the hatch, as the headless `teleport {to: ladder_down}` puts it
    g.p.pos = step_center(hatch.cell, run);
    g.p.yaw = facing((-run.0, -run.1));
    g.p.set_abs_y(0.0);
    let idle = Controls::default();
    let mut stats = Stats::default();
    // the frames of the approach: the window may recentre, the neighbours are kept (and with `render`, built)
    for _ in 0..8 {
        tick(&mut g, &idle, dt, rig, render);
    }
    // a teleport lands in a window the render has not built yet: let it stream in, as it does on the way to a hatch
    let mut waited = 0;
    while render && (g.world_dirty || g.world.has_pending()) && waited < 600 {
        tick(&mut g, &idle, dt, rig, render);
        waited += 1;
    }
    for _ in 0..30 {
        if render {
            tick(&mut g, &idle, dt, rig, render);
        }
    }
    if g.inf.as_ref().unwrap().level != 0 {
        return Err("the approach changed the level".into());
    }
    if g.report.is_some() {
        return Err("the runner was caught during the approach".into());
    }
    // the hatch may have moved with a recentre: read it back
    let hatch = g
        .fac
        .stairs
        .iter()
        .copied()
        .filter(|s| s.is_ladder() && s.dir < 0)
        .min_by(|a, b| pdist(a.cell.center(), g.p.pos).total_cmp(&pdist(b.cell.center(), g.p.pos)))
        .ok_or("the hatch left the window")?;
    let center = g.inf.as_ref().unwrap().center;
    let resident = g.inf.as_ref().unwrap().stack.get(-1).ok_or("level -1 is not resident beside the hatch")?;
    if resident.center != center {
        return Err(format!("the resident sewer is for centre {:?}, the window is {:?}", resident.center, center));
    }
    // the sewer under the manhole: the foot of the same ladder, a floor under it and a place to stand
    let foot = resident
        .fac
        .stairs
        .iter()
        .find(|s| s.is_ladder() && s.dir > 0 && s.cell == hatch.cell)
        .ok_or("the sewer window has no foot for the ladder")?;
    if resident.fac.is_hole(hatch.cell.x, hatch.cell.y) {
        return Err("the sewer under the manhole is a hole (no floor under the ladder foot)".into());
    }
    if resident.fac.is_wall_cell(hatch.cell.x, hatch.cell.y) {
        return Err("the ladder foot is a wall cell of the sewer (the climber would be inside rock)".into());
    }
    let exit = step_center(foot.cell, run_of(foot));
    if !resident.fac.body_fits(exit.0, exit.1, PLAYER_RADIUS) {
        return Err(format!("the sewer exit {exit:?} does not fit a body"));
    }
    let open_around = (-3..=3)
        .flat_map(|dx| (-3..=3).map(move |dz| (dx, dz)))
        .filter(|(dx, dz)| !resident.fac.is_hole(hatch.cell.x + dx, hatch.cell.y + dz))
        .count();
    if open_around < 3 {
        return Err(format!("only {open_around} floor cells within 3 of the ladder foot"));
    }
    if render && !check_world(&g)? {
        return Err("before the climb the drawn set holds no floor round the runner".into());
    }
    // mount by walking into the hatch (the walk-in rule), else by interact, and climb down holding back
    let walk = Controls { fwd: 1.0, ..Controls::default() };
    let mut frames = 0;
    while g.ladder.is_none() {
        tick(&mut g, &walk, dt, rig, render);
        frames += 1;
        if frames > 120 {
            let i = g
                .fac
                .stairs
                .iter()
                .position(|s| s.is_ladder() && s.dir < 0 && s.cell == hatch.cell)
                .ok_or("the hatch left the window")?;
            if !g.start_ladder(i) {
                return Err("neither walking in nor interact mounted the ladder".into());
            }
            break;
        }
    }
    let back = Controls { fwd: -1.0, ..Controls::default() };
    let (mut flips, mut guard, mut flipped) = (0, 0, false);
    let mut prev_y = g.p.y();
    while g.ladder.is_some() {
        let level = g.inf.as_ref().unwrap().level;
        let started = std::time::Instant::now();
        tick(&mut g, &back, dt, rig, render);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        stats.worst_ms = stats.worst_ms.max(ms);
        guard += 1;
        if guard > 3000 {
            return Err(format!("the climb never ended; y {}", g.p.y()));
        }
        if !finite(&g) {
            return Err(format!("not finite: pos {:?} y {}", g.p.pos, g.p.y()));
        }
        let now = g.inf.as_ref().unwrap().level;
        if now != level {
            flips += 1;
            flipped = true;
            stats.flip_ms = ms;
            if (level, now) != (0, -1) {
                return Err(format!("the label went {level} -> {now}"));
            }
            if g.fac.level != -1 {
                return Err(format!("the label flipped but the live facility is level {}", g.fac.level));
            }
            if g.fac.is_hole(hatch.cell.x, hatch.cell.y) || g.fac.is_wall_cell(hatch.cell.x, hatch.cell.y) {
                return Err("on the flip frame the live sewer has no floor under the ladder".into());
            }
            if !g.inf.as_ref().unwrap().stack.has(0) {
                return Err("after the flip the maze above is not resident".into());
            }
        }
        if flipped && g.fac.is_wall_cell(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32) {
            return Err(format!("the climber is inside a wall cell of the sewer at {:?}", g.p.pos));
        }
        // the surface under the runner is at most a step above the feet
        let ground = ground_y(&g.fac, &g.inf.as_ref().unwrap().stack, g.p.pos.0, g.p.pos.1, g.p.y());
        if g.ladder.is_some() && ground.y > g.p.y() + 0.6 + 1e-3 {
            return Err(format!("ground {} above the feet {} on the shaft", ground.y, g.p.y()));
        }
        if g.p.y() > prev_y + 0.3 {
            return Err(format!("rose on the way down: {prev_y} -> {}", g.p.y()));
        }
        prev_y = g.p.y();
        if render && flipped {
            if !check_world(&g)? {
                stats.floorless_frames += 1;
            }
        } else if render {
            check_world(&g)?;
        }
    }
    if flips != 1 {
        return Err(format!("the label flipped {flips} times"));
    }
    let inf = g.inf.as_ref().unwrap();
    if inf.level != -1 || g.fac.level != -1 {
        return Err(format!("ended on level {} (facility {})", inf.level, g.fac.level));
    }
    if (g.p.y() - floor_y_of(-1)).abs() > 0.02 || !g.p.grounded || g.p.falling() {
        return Err(format!("ended at y {} grounded {}", g.p.y(), g.p.grounded));
    }
    if !g.fac.body_fits(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS) {
        return Err(format!("ended inside a wall of the sewer at {:?}", g.p.pos));
    }
    let ground = ground_y(&g.fac, &inf.stack, g.p.pos.0, g.p.pos.1, g.p.y());
    if ground.level != -1 || (ground.y - floor_y_of(-1)).abs() > 0.5 {
        return Err(format!("the surface under the runner is {ground:?}"));
    }
    // standing there a few seconds: still the sewer floor, still finite, and (rendering) the world becomes whole with a floor under the runner
    for i in 0..90 {
        tick(&mut g, &idle, dt, rig, render);
        if render {
            let floor = check_world(&g)?;
            if g.world_dirty || g.world.has_pending() {
                stats.frames_to_whole = i + 1;
            }
            if !floor && !g.world_dirty {
                return Err(format!("the world is built whole ({} frames after the climb) and holds no floor round the runner", i + 1));
            }
        }
    }
    if !finite(&g) || g.inf.as_ref().unwrap().level != -1 || (g.p.y() + STOREY).abs() > 0.02 {
        return Err(format!("settled at level {} y {}", g.inf.as_ref().unwrap().level, g.p.y()));
    }
    if render && (g.world_dirty || g.world.has_pending()) {
        return Err("90 frames after the climb the world is still being built".into());
    }
    Ok(stats)
}

/// Runs the climbs of `texts`: every ladder down of each window at the first frame length, the first one at the rest.
fn sweep(texts: &[String], centres: &[(i32, i32)], max_ladders: usize, frame_lengths: &[f32], render: bool) -> (usize, Vec<String>, Stats) {
    let rig = Rig::new();
    let trace = std::env::var("LR_LADDER_SWEEP_TRACE").is_ok();
    let (mut climbs, mut failures, mut worst) = (0, Vec::new(), Stats::default());
    for text in texts {
        for &centre in centres {
            let count = ladders_down(&fresh(text, centre, &rig)).min(max_ladders);
            for n in 0..count {
                for (k, &dt) in frame_lengths.iter().enumerate() {
                    if n > 0 && k > 0 {
                        continue;
                    }
                    climbs += 1;
                    if trace {
                        eprintln!("climb {climbs}: seed {text} centre {centre:?} ladder {n}/{count} dt {dt:.3}");
                    }
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| climb(text, centre, n, dt, &rig, render)));
                    let run = run.unwrap_or_else(|p| {
                        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                        Err(format!("PANIC {}", msg.unwrap_or_default()))
                    });
                    match run {
                        Ok(s) => {
                            if trace {
                                eprintln!("   {s:?}");
                            }
                            worst.worst_ms = worst.worst_ms.max(s.worst_ms);
                            worst.flip_ms = worst.flip_ms.max(s.flip_ms);
                            worst.floorless_frames = worst.floorless_frames.max(s.floorless_frames);
                            worst.frames_to_whole = worst.frames_to_whole.max(s.frames_to_whole);
                        }
                        Err(why) => {
                            eprintln!("FAIL seed {text} centre {centre:?} ladder {n} dt {dt:.3}: {why}");
                            failures.push(format!("seed {text} centre {centre:?} ladder {n} dt {dt:.3}: {why}"));
                        }
                    }
                }
            }
        }
    }
    (climbs, failures, worst)
}

#[test]
fn every_manhole_of_the_quick_sweep_arrives_on_a_floor_in_the_sewer() {
    let seeds: Vec<String> = ["1", "2", "SEWER"].iter().map(|s| s.to_string()).collect();
    let (climbs, failures, _) = sweep(&seeds, &CENTRES[..1], 2, &FRAME_LENGTHS, false);
    assert!(climbs >= 8, "the sweep climbed only {climbs} ladders");
    assert!(failures.is_empty(), "{} of {climbs} climbs failed:\n{}", failures.len(), failures.join("\n"));
}

/// The real `update` with the render meshes built from `Assets::headless`: seed "2" at window centre (1, 0) is the room
/// where the sewer part holding a room-less prop panicked in `variant_key` (the browser build's black screen).
#[test]
fn the_drawn_world_holds_a_floor_round_the_runner_through_every_quick_descent() {
    use crate::render::world_mesh::SLOW_FRAMES;
    let seeds = vec!["2".to_string()];
    let (climbs, failures, worst) = sweep(&seeds, &[(1, 0)], 1, &FRAME_LENGTHS[..2], true);
    assert!(climbs >= 2, "the sweep climbed only {climbs} ladders");
    assert!(failures.is_empty(), "{} of {climbs} climbs failed (worst {worst:?}):\n{}", failures.len(), failures.join("\n"));
    // the same on a slow machine: each frame builds only the one part it always may, so the window streams in over frames
    SLOW_FRAMES.with(|s| s.set(true));
    let (climbs, failures, worst) = sweep(&seeds, &[(0, 0)], 1, &FRAME_LENGTHS[..1], true);
    SLOW_FRAMES.with(|s| s.set(false));
    assert!(climbs >= 1);
    assert!(failures.is_empty(), "slow frames: {} of {climbs} climbs failed (worst {worst:?}):\n{}", failures.len(), failures.join("\n"));
}

#[test]
#[ignore]
fn ladder_sweep_long() {
    use crate::render::world_mesh::SLOW_FRAMES;
    let seeds: u32 = std::env::var("LR_LADDER_SWEEP_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    let render = std::env::var("LR_LADDER_SWEEP_RENDER").is_ok();
    SLOW_FRAMES.with(|s| s.set(std::env::var("LR_LADDER_SWEEP_SLOW").is_ok()));
    let (climbs, failures, worst) = sweep(&texts(seeds), &CENTRES, 8, &FRAME_LENGTHS[..2], render);
    println!("{climbs} climbs, {} failed, worst {worst:?}", failures.len());
    for f in &failures {
        println!("{f}");
    }
    assert!(failures.is_empty());
}
