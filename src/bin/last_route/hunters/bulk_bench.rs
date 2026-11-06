//! Bulk path-search probe: how the hunters' and the bots' route searches behave when many of them chase one runner.
//!
//! All of it is `#[ignore]`d and asserts no time. Run it alone, one test at a time (the counting allocator below is
//! process-wide), in the normal dev profile or `--release`:
//!
//! ```text
//! CARGO_TARGET_DIR=target/bulk-path cargo test --bin last_route bulk_world_step_cost -- --ignored --nocapture
//! CARGO_TARGET_DIR=target/bulk-path cargo test --bin last_route bulk_brain_search_bursts -- --ignored --nocapture
//! CARGO_TARGET_DIR=target/bulk-path cargo test --bin last_route bulk_shared_field_vs_astar -- --ignored --nocapture
//! CARGO_TARGET_DIR=target/bulk-path cargo test --bin last_route bulk_search_attribution -- --ignored --nocapture
//! ```
//!
//! 1. `bulk_world_step_cost`: a real Infinite window (`Game::new_infinite_sim`), N hunters and M bots, the runner walking
//!    a long route, 600 `Game::step` frames per cell of the matrix; per-frame wall time (avg, p50, p95, p99, worst),
//!    frames over 16.7 and 33.3 ms, the split between frames where the runner changed cell and the rest, and the frames
//!    where the window shifted. The base cost of the same window with no hunters and no bots is shown for subtraction.
//! 2. `bulk_brain_search_bursts`: the hunters alone (the `Brain::update` loop of `Game::update_hunters`, one `Target`),
//!    timing each `update` and counting its heap allocations. A route search grows hash maps and a heap, so an update that
//!    allocates a lot is an update that searched: the probe prints how many updates searched per frame (the burst the
//!    frame has to pay), their share of the time, and the worst frame, with the repath timers either left as spawned
//!    (every brain starts in step, as an admin "summon N" makes them) or staggered.
//! 3. `bulk_shared_field_vs_astar`: the proposed fix measured on the same window: one reverse Dijkstra distance field
//!    from the runner cell against one A* per hunter, with node counts and a check that both give the same route cost.
//! 4. `bulk_search_attribution`: a counting global allocator plus sampled backtraces over `Game::step`: which search
//!    function ran (hunter A*, StackNav A*, Facility::find_path, bot BFS) and who called it.
//!
//! Environment knobs: BULK_HUNTERS="8,32,96,192", BULK_BOTS="0,32,96", BULK_FRAMES=600, BULK_SPREAD=1 (hunters spread over
//! the window instead of the admin's ring round the runner), BULK_DUMP=1 (print raw sampled backtraces), BULK_BUDGET=N
//! (the route scheduler's searches per frame, path_scheduler.rs; unset keeps data/hunters/pathing.json; a huge N is
//! effectively unbudgeted, which still caches and shares goal fields). Both probes print the scheduler's counters.

use super::brain::{Brain, State, Target};
use super::path_scheduler::{self, Stats};
use crate::audio::Sounds;
use crate::config::{Difficulty, HUNTER_RADIUS, NOISE_SPRINT};
use crate::content::Content;
use crate::game::{Controls, Game};
use crate::hunters::gimmicks::Ctx;
use crate::world::catalog::Catalog;
use crate::world::{Cell, Facility};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell as StdCell;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use crate::shared::clock::Instant;

const DT: f32 = 1.0 / 60.0;
const SEED: u32 = 20261101;
/// The runner's speed along the route (m/s): a sprint, a little under `PLAYER_SPRINT_SPEED`.
const RUNNER_SPEED: f32 = 5.5;
/// The admin summon's golden-angle ring (app/stress.rs `summon`).
const RING_START: f32 = 2.5;
const RING_STEP: f32 = 1.2;
const RING_ANGLE: f32 = 2.39996;
/// An update that allocated at least this many times ran a route search (see the module doc; the histogram printed by
/// `bulk_brain_search_bursts` shows the gap this sits in).
const SEARCH_ALLOCS: u64 = 12;

// ---------------------------------------------------------------------------------------------------------------------
// The counting allocator (only the test binary has it)
// ---------------------------------------------------------------------------------------------------------------------

struct Counting;

static ARMED: AtomicBool = AtomicBool::new(false);
static SAMPLING: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
/// `cols * rows * 4`: the two per-search arrays of `Facility::find_path`, so one call allocates this size twice.
static GRID_BYTES: AtomicUsize = AtomicUsize::new(0);
static GRID_ALLOCS: AtomicU64 = AtomicU64::new(0);
static SAMPLE_MIN: AtomicUsize = AtomicUsize::new(256);
static SAMPLE_EVERY: AtomicU64 = AtomicU64::new(16);
static BIG: AtomicU64 = AtomicU64::new(0);
const NS: usize = 9;
const NC: usize = 10;
const Z: AtomicU64 = AtomicU64::new(0);
static TALLY_N: [AtomicU64; NS * NC] = [Z; NS * NC];
static TALLY_B: [AtomicU64; NS * NC] = [Z; NS * NC];
static DUMP: AtomicBool = AtomicBool::new(false);
static SIZE_HIST: [AtomicU64; 16] = [Z; 16];

thread_local! {
    static BUSY: StdCell<bool> = const { StdCell::new(false) };
}

/// Search functions, innermost first in a backtrace: (substring of the symbol, label).
const SEARCHES: [(&str, &str); NS - 1] = [
    ("plan_route_wet", "hunter A* (brain::plan_route_wet, HashMap)"),
    ("find_path_with", "StackNav A* (flight_edges::find_path_with, BTreeMap)"),
    ("Facility::find_path", "Facility::find_path (flat-array A*)"),
    ("dry_route", "bot dry_route (BFS, HashMap)"),
    ("flee_route", "bot flee_route (BFS, HashMap)"),
    ("distance_map", "Facility::distance_map (BFS)"),
    ("nearest_on_layer", "hunter nearest_on_layer (deck BFS)"),
    ("separate", "director::separate (pairwise)"),
];
/// Who asked, outermost-relevant: (substring, label).
const CALLERS: [(&str, &str); NC - 1] = [
    ("perceive", "Brain::perceive (hearing check, every sense tick)"),
    ("pursue", "Brain::pursue (repath timer)"),
    ("hop_to", "Brain::hop_to (cross-layer next_hop)"),
    ("stalk_goal", "Brain::stalk_goal"),
    ("pick_goal", "bot pick_goal (route_cost per candidate)"),
    ("route_cost", "bot route_cost"),
    ("walk_toward", "bot walk_toward -> plan (REPATH 0.5 s)"),
    ("flee", "bot flee"),
    ("update_npcs", "bot other"),
];

fn classify(text: &str) -> (usize, usize) {
    let first = |tags: &[(&str, &str)]| {
        tags.iter()
            .enumerate()
            .filter_map(|(i, (t, _))| text.find(t).map(|p| (p, i)))
            .min()
            .map_or(tags.len(), |(_, i)| i)
    };
    (first(&SEARCHES), first(&CALLERS))
}

fn note(size: usize) {
    if !ARMED.load(Relaxed) {
        return;
    }
    if BUSY.try_with(|b| b.get()).unwrap_or(true) {
        return;
    }
    ALLOCS.fetch_add(1, Relaxed);
    BYTES.fetch_add(size as u64, Relaxed);
    if size == GRID_BYTES.load(Relaxed) {
        GRID_ALLOCS.fetch_add(1, Relaxed);
    }
    if SAMPLING.load(Relaxed) && size >= SAMPLE_MIN.load(Relaxed) {
        let k = BIG.fetch_add(1, Relaxed);
        if k % SAMPLE_EVERY.load(Relaxed).max(1) == 0 {
            sample(size);
        }
    }
}

fn sample(size: usize) {
    let _ = BUSY.try_with(|b| {
        b.set(true);
        let text = std::backtrace::Backtrace::force_capture().to_string();
        let (s, c) = classify(&text);
        TALLY_N[s * NC + c].fetch_add(1, Relaxed);
        TALLY_B[s * NC + c].fetch_add(size as u64, Relaxed);
        let bucket = (usize::BITS - size.max(1).leading_zeros()) as usize;
        SIZE_HIST[bucket.min(15)].fetch_add(1, Relaxed);
        if DUMP.swap(false, Relaxed) {
            println!("---- sampled allocation of {size} bytes ----\n{text}");
        }
        b.set(false);
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        note(l.size());
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        note(new);
        System.realloc(p, l, new)
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn reset_counters() {
    for a in [&ALLOCS, &BYTES, &GRID_ALLOCS, &BIG] {
        a.store(0, Relaxed);
    }
    for t in TALLY_N.iter().chain(TALLY_B.iter()).chain(SIZE_HIST.iter()) {
        t.store(0, Relaxed);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The scene: a real Infinite window, a long walkable route for the runner, hunters and bots
// ---------------------------------------------------------------------------------------------------------------------

struct Scene {
    g: Game,
    /// The runner's route in absolute cells (window origin added), as a polyline of cell centres.
    route: Vec<(f32, f32)>,
    cum: Vec<f32>,
    travelled: f32,
}

fn env_list(key: &str, default: &[usize]) -> Vec<usize> {
    std::env::var(key)
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn cell_of(p: (f32, f32)) -> Cell {
    Cell::new(p.0.floor() as i32, p.1.floor() as i32)
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn route_from(fac: &Facility, start: (f32, f32), origin: (i32, i32)) -> Vec<(f32, f32)> {
    let from = cell_of(start);
    let reach = fac.distance_map(from);
    // the farthest cell that is at most ~95 steps away: a route long enough for 600 frames at the runner's speed
    let far = reach
        .iter()
        .filter(|(c, d)| **d <= 95 && fac.hunter_open(**c))
        .max_by_key(|(c, d)| (**d, c.x, c.y))
        .map(|(c, _)| *c)
        .unwrap_or(from);
    let cells = fac.find_path(from, far);
    cells
        .iter()
        .map(|c| {
            let p = c.center();
            (p.0 + origin.0 as f32, p.1 + origin.1 as f32)
        })
        .collect()
}

fn along(route: &[(f32, f32)], cum: &[f32], s: f32) -> (f32, f32) {
    let total = *cum.last().unwrap_or(&0.0);
    if route.len() < 2 || total <= 0.0 {
        return route.first().copied().unwrap_or((0.0, 0.0));
    }
    // ping-pong
    let m = s.rem_euclid(2.0 * total);
    let s = if m > total { 2.0 * total - m } else { m };
    let i = cum.partition_point(|c| *c < s).clamp(1, route.len() - 1);
    let (a, b) = (route[i - 1], route[i]);
    let seg = (cum[i] - cum[i - 1]).max(1e-4);
    let t = ((s - cum[i - 1]) / seg).clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// `n` brains awake and heading for the runner: the admin's golden-angle ring round it, or spread over the open cells.
fn make_hunters(g: &Game, content: &Content, n: usize, spread: bool, tag: &str) -> Vec<Brain> {
    let pool = content.draw_pool();
    let runner = g.p.pos;
    let mut out = Vec::with_capacity(n);
    let spots: Vec<(f32, f32)> = if spread {
        let cells: Vec<Cell> = g
            .fac
            .floor_cells()
            .into_iter()
            .filter(|c| g.fac.hunter_open(*c) && dist(c.center(), runner) >= 8.0)
            .collect();
        let stride = (cells.len() / n.max(1)).max(1);
        cells.iter().step_by(stride).take(n).map(|c| c.center()).collect()
    } else {
        (0..n)
            .filter_map(|slot| {
                let angle = slot as f32 * RING_ANGLE;
                let radius = RING_START + RING_STEP * (slot as f32).sqrt();
                let want = (runner.0 + angle.cos() * radius, runner.1 + angle.sin() * radius);
                g.fac
                    .nearest_open(want.0, want.1, HUNTER_RADIUS)
                    .or_else(|| g.fac.nearest_open(runner.0, runner.1, HUNTER_RADIUS))
            })
            .collect()
    };
    for (i, at) in spots.into_iter().enumerate() {
        let mut b = Brain::new(content.hunters[pool[i % pool.len()]].clone(), at, &format!("BULK:{tag}:{i}"), Difficulty::Standard);
        b.activate_at(runner);
        out.push(b);
    }
    out
}

fn scene(content: &Content, catalog: &Catalog, hunters: usize, bots: usize, spread: bool) -> Scene {
    let mut g = Game::new_infinite_sim(
        Difficulty::Standard,
        SEED,
        &crate::world::route::room_text(SEED),
        0,
        content,
        catalog,
    );
    g.hunters.clear();
    g.admin.god = true; // a catch must not end the run or start a struggle
    g.p.flashlight_on = true;
    let origin = g.window_origin();
    let route = route_from(&g.fac, g.p.pos, origin);
    let mut cum = vec![0.0f32];
    for w in route.windows(2) {
        let last = *cum.last().unwrap();
        cum.push(last + dist(w[0], w[1]));
    }
    let list = make_hunters(&g, content, hunters, spread, "WORLD");
    g.hunters.extend(list);
    if bots > 0 {
        let p = g.p.pos;
        g.npc_spawn(bots as i32, 70, (p.0, 1.6, p.1), (0.0, 1.0, 0.0));
    }
    GRID_BYTES.store((g.fac.cols * g.fac.rows * 4) as usize, Relaxed);
    Scene { g, route, cum, travelled: 0.0 }
}

struct Frames {
    ms: Vec<f32>,
    cell_changed: Vec<bool>,
    shifted: Vec<bool>,
}

/// `n` frames of `Game::step`, the runner teleported along the route before each (so it never leaves open floor) and
/// sprinting in place (the noise the hunters hear), timing each step.
fn run_world(sc: &mut Scene, content: &Content, catalog: &Catalog, n: usize) -> Frames {
    let sounds = Sounds::default();
    let controls = Controls { fwd: 1.0, sprint: true, ..Default::default() };
    let mut f = Frames { ms: Vec::with_capacity(n), cell_changed: vec![], shifted: vec![] };
    let mut last_cell = cell_of(sc.g.p.pos);
    for _ in 0..n {
        sc.travelled += RUNNER_SPEED * DT;
        let abs = along(&sc.route, &sc.cum, sc.travelled);
        let o = sc.g.window_origin();
        sc.g.p.pos = (abs.0 - o.0 as f32, abs.1 - o.1 as f32);
        sc.g.p.flashlight_on = true;
        sc.g.p.lamp = sc.g.p.lamp.max(0.5);
        let cell = cell_of(sc.g.p.pos);
        let t0 = Instant::now();
        sc.g.step(DT, &controls, content, catalog, &sounds);
        f.ms.push(t0.elapsed().as_secs_f32() * 1000.0);
        f.cell_changed.push(cell != last_cell);
        f.shifted.push(sc.g.window_origin() != o);
        last_cell = cell;
    }
    f
}

fn pct(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(((sorted.len() - 1) as f32) * p).round() as usize]
}

fn mean(v: impl Iterator<Item = f32>) -> f32 {
    let (s, n) = v.fold((0.0f32, 0usize), |(s, n), x| (s + x, n + 1));
    if n == 0 {
        0.0
    } else {
        s / n as f32
    }
}

struct Summary {
    avg: f32,
    p50: f32,
    p95: f32,
    p99: f32,
    worst: f32,
    over16: usize,
    over33: usize,
}

fn summarize(ms: &[f32]) -> Summary {
    let mut s = ms.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    Summary {
        avg: mean(ms.iter().copied()),
        p50: pct(&s, 0.5),
        p95: pct(&s, 0.95),
        p99: pct(&s, 0.99),
        worst: *s.last().unwrap_or(&0.0),
        over16: ms.iter().filter(|m| **m > 16.7).count(),
        over33: ms.iter().filter(|m| **m > 33.3).count(),
    }
}

fn state_histogram(hunters: &[Brain]) -> String {
    let mut c = [0usize; 6];
    let mut visible = 0;
    for h in hunters {
        let i = match h.state {
            State::Idle => 0,
            State::Wander => 1,
            State::Investigate => 2,
            State::Chase => 3,
            State::Search => 4,
            State::Peek => 5,
        };
        c[i] += 1;
        visible += h.direct_visible as usize;
    }
    format!(
        "idle {} wander {} investigate {} chase {} search {} peek {} | direct_visible {}",
        c[0], c[1], c[2], c[3], c[4], c[5], visible
    )
}

// ---------------------------------------------------------------------------------------------------------------------
// 1. The world step matrix
// ---------------------------------------------------------------------------------------------------------------------

#[test]
#[ignore = "timing benchmark: run with --ignored --nocapture"]
fn bulk_world_step_cost() {
    let content = Content::load();
    let catalog = Catalog::load();
    let hunters = env_list("BULK_HUNTERS", &[8, 32, 96, 192]);
    let bots = env_list("BULK_BOTS", &[0, 32, 96]);
    let frames = env_usize("BULK_FRAMES", 600);
    let spread = env_usize("BULK_SPREAD", 0) == 1;
    path_scheduler::reset_thread();
    path_scheduler::set_searches_per_frame(budget_env());
    println!(
        "Game::step wall time per frame, {frames} frames, runner sprinting a route at {RUNNER_SPEED} m/s, hunters placed {}",
        if spread { "spread over the window" } else { "on the admin summon ring" }
    );
    println!("(dev profile unless --release; hunters = N extra brains, bots = M NPC teammates; all ms)");

    // the base: the same window with nothing in it
    let mut base = scene(&content, &catalog, 0, 0, spread);
    let f0 = run_world(&mut base, &content, &catalog, frames);
    let b = summarize(&f0.ms);
    println!(
        "window {}x{} cells, route {:.0} m, hunters left by the lineup after clear: {}",
        base.g.fac.cols,
        base.g.fac.rows,
        base.cum.last().copied().unwrap_or(0.0),
        base.g.hunters.len()
    );
    println!(
        "{:>4} {:>4} | avg {:>7.3} p50 {:>7.3} p95 {:>7.3} p99 {:>7.3} worst {:>8.3} | >16.7ms {:>3} >33.3ms {:>3}   (base: nothing but the runner)",
        0, 0, b.avg, b.p50, b.p95, b.p99, b.worst, b.over16, b.over33
    );
    for &m in &bots {
        for &n in &hunters {
            let mut sc = scene(&content, &catalog, n, m, spread);
            let routes0 = path_scheduler::stats();
            let f = run_world(&mut sc, &content, &catalog, frames);
            let routes = route_delta(routes0, path_scheduler::stats());
            let s = summarize(&f.ms);
            let moved = mean(f.ms.iter().zip(&f.cell_changed).filter(|(_, c)| **c).map(|(m, _)| *m));
            let still = mean(f.ms.iter().zip(&f.cell_changed).filter(|(_, c)| !**c).map(|(m, _)| *m));
            let shifted: Vec<f32> = f.ms.iter().zip(&f.shifted).filter(|(_, c)| **c).map(|(m, _)| *m).collect();
            println!(
                "{:>4} {:>4} | avg {:>7.3} p50 {:>7.3} p95 {:>7.3} p99 {:>7.3} worst {:>8.3} | >16.7ms {:>3} >33.3ms {:>3} | net of base avg {:>7.3} = {:>6.1} us per hunter | runner cell changed {:>6.3} vs not {:>6.3} | window shifts {} (avg {:>6.2})",
                n,
                m,
                s.avg,
                s.p50,
                s.p95,
                s.p99,
                s.worst,
                s.over16,
                s.over33,
                s.avg - b.avg,
                (s.avg - b.avg) * 1000.0 / n.max(1) as f32,
                moved,
                still,
                shifted.len(),
                mean(shifted.iter().copied()),
            );
            // lockstep: where the worst frames fall
            let mut order: Vec<usize> = (0..f.ms.len()).collect();
            order.sort_by(|a, b| f.ms[*b].total_cmp(&f.ms[*a]));
            let worst: Vec<usize> = {
                let mut w: Vec<usize> = order.iter().take(10).copied().collect();
                w.sort_unstable();
                w
            };
            println!(
                "       worst-10 frame indices {:?} (ms of the worst {:.1}, median {:.2}); first 3 frames {:.1} {:.1} {:.1}",
                worst,
                f.ms[order[0]],
                s.p50,
                f.ms.first().copied().unwrap_or(0.0),
                f.ms.get(1).copied().unwrap_or(0.0),
                f.ms.get(2).copied().unwrap_or(0.0)
            );
            println!(
                "       after {frames} frames: {} | hunters {} bots {}",
                state_histogram(&sc.g.hunters),
                sc.g.hunters.len(),
                sc.g.npcs.len()
            );
            println!("       this scene {}", route_summary(routes));
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// 2. The brain loop: how many updates search in one frame
// ---------------------------------------------------------------------------------------------------------------------

struct BrainRun {
    frame_ms: Vec<f32>,
    searching_per_frame: Vec<u32>,
    search_ms_per_frame: Vec<f32>,
    update_allocs: Vec<u32>,
    update_us: Vec<f32>,
}

/// BULK_BUDGET overrides the route scheduler's frame budget (searches per frame); unset or 0 keeps data/hunters/pathing.json.
fn budget_env() -> Option<usize> {
    match env_usize("BULK_BUDGET", 0) {
        0 => None,
        n => Some(n),
    }
}

/// The route scheduler's counters, as one line.
fn route_summary(s: Stats) -> String {
    format!(
        "routes: {} A* searches, {} read off {} shared goal fields, {} cache hits, {} deferred to the next frame",
        s.searches, s.field_routes, s.field_builds, s.cache_hits, s.deferred
    )
}

/// The counters a stretch of frames added, given the counters before it.
fn route_delta(before: Stats, after: Stats) -> Stats {
    Stats {
        searches: after.searches - before.searches,
        field_builds: after.field_builds - before.field_builds,
        field_routes: after.field_routes - before.field_routes,
        cache_hits: after.cache_hits - before.cache_hits,
        deferred: after.deferred - before.deferred,
    }
}

fn brain_run(g: &Game, content: &Content, hunters: &mut [Brain], route: &[(f32, f32)], cum: &[f32], frames: usize) -> BrainRun {
    path_scheduler::reset_thread();
    path_scheduler::set_searches_per_frame(budget_env());
    let prof = Difficulty::Standard.profile();
    let relays_taken = vec![false; g.fac.relays.len()];
    let batteries_taken = vec![false; g.fac.batteries.len()];
    let hiding_disabled = vec![false; g.fac.hiding.len()];
    let ctx = Ctx {
        fac: &g.fac,
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
    let _ = content;
    let o = g.window_origin();
    let mut r = BrainRun {
        frame_ms: vec![],
        searching_per_frame: vec![],
        search_ms_per_frame: vec![],
        update_allocs: vec![],
        update_us: vec![],
    };
    let mut travelled = 0.0f32;
    ARMED.store(true, Relaxed);
    SAMPLING.store(false, Relaxed);
    for _ in 0..frames {
        // the frame's route budget opens before the brains ask (as Game::update_hunters does)
        path_scheduler::begin_frame(&g.fac);
        travelled += RUNNER_SPEED * DT;
        let abs = along(route, cum, travelled);
        let pos = (abs.0 - o.0 as f32, abs.1 - o.1 as f32);
        let target = Target {
            pos,
            facing: (0.0, 1.0),
            noise: NOISE_SPRINT,
            flashlight_on: true,
            lamp_charge: 1.0,
            sprinting: true,
            ..Default::default()
        };
        let frame_t = Instant::now();
        let (mut searching, mut search_ms) = (0u32, 0.0f32);
        for b in hunters.iter_mut() {
            let a0 = ALLOCS.load(Relaxed);
            let t0 = Instant::now();
            let _ = b.update(DT, &ctx, &target, 1, 0, &prof);
            let us = t0.elapsed().as_secs_f32() * 1e6;
            let allocs = ALLOCS.load(Relaxed) - a0;
            r.update_allocs.push(allocs as u32);
            r.update_us.push(us);
            if allocs >= SEARCH_ALLOCS {
                searching += 1;
                search_ms += us / 1000.0;
            }
        }
        r.frame_ms.push(frame_t.elapsed().as_secs_f32() * 1000.0);
        r.searching_per_frame.push(searching);
        r.search_ms_per_frame.push(search_ms);
    }
    ARMED.store(false, Relaxed);
    r
}

fn print_brain_run(label: &str, n: usize, r: &BrainRun) {
    let s = summarize(&r.frame_ms);
    let total_updates = r.update_allocs.len().max(1);
    let searches: u64 = r.searching_per_frame.iter().map(|x| *x as u64).sum();
    let search_ms: f32 = r.search_ms_per_frame.iter().sum();
    let all_ms: f32 = r.frame_ms.iter().sum();
    let max_burst = r.searching_per_frame.iter().copied().max().unwrap_or(0);
    let mut sp = r.searching_per_frame.clone();
    sp.sort_unstable();
    let p95_burst = sp.get(((sp.len().max(1) - 1) as f32 * 0.95) as usize).copied().unwrap_or(0);
    let frames_with: usize = r.searching_per_frame.iter().filter(|x| **x > 0).count();
    println!("           {}", route_summary(path_scheduler::stats()));
    println!(
        "{label:<10} N={n:>3} | frame avg {:>7.3} p95 {:>7.3} worst {:>8.3} ms | searching updates/frame avg {:>5.2} p95 {} max {} (of {n}) | frames with any search {}/{} | search time {:.0}% of hunter time | {:.1} searches/hunter/s",
        s.avg,
        s.p95,
        s.worst,
        searches as f32 / r.frame_ms.len().max(1) as f32,
        p95_burst,
        max_burst,
        frames_with,
        r.frame_ms.len(),
        100.0 * search_ms / all_ms.max(1e-6),
        searches as f32 / (n.max(1) as f32 * r.frame_ms.len() as f32 * DT),
    );
    let _ = total_updates;
}

#[test]
#[ignore = "timing benchmark: run with --ignored --nocapture"]
fn bulk_brain_search_bursts() {
    let content = Content::load();
    let catalog = Catalog::load();
    let hunters = env_list("BULK_HUNTERS", &[8, 32, 96, 192]);
    let frames = env_usize("BULK_FRAMES", 600);
    let base = scene(&content, &catalog, 0, 0, false);

    // the allocation histogram of one update, to see the gap the SEARCH_ALLOCS threshold sits in
    {
        let mut list = make_hunters(&base.g, &content, 96, false, "HIST");
        let r = brain_run(&base.g, &content, &mut list, &base.route, &base.cum, frames);
        let edges = [0u32, 1, 2, 4, 8, 12, 16, 32, 64, 128, 256, 1024];
        println!("allocations per Brain::update over {} updates (96 hunters, ring):", r.update_allocs.len());
        for w in edges.windows(2) {
            let (lo, hi) = (w[0], w[1]);
            let sel: Vec<usize> = (0..r.update_allocs.len()).filter(|i| r.update_allocs[*i] >= lo && r.update_allocs[*i] < hi).collect();
            if sel.is_empty() {
                continue;
            }
            println!(
                "  {lo:>4}..{hi:<4} allocs: {:>7} updates, avg {:>8.1} us, max {:>8.1} us",
                sel.len(),
                mean(sel.iter().map(|i| r.update_us[*i])),
                sel.iter().map(|i| r.update_us[*i]).fold(0.0, f32::max)
            );
        }
        let big = r.update_allocs.iter().filter(|a| **a >= 1024).count();
        println!("  >=1024 allocs: {big} updates");
    }

    println!("-- repath timers as spawned (an admin summon of N: every brain starts in step) --");
    for spread in [false, true] {
        for &n in &hunters {
            let mut list = make_hunters(&base.g, &content, n, spread, "BURST");
            let r = brain_run(&base.g, &content, &mut list, &base.route, &base.cum, frames);
            print_brain_run(if spread { "spread" } else { "ring" }, n, &r);
            if n == *hunters.last().unwrap() {
                println!("           states at the end: {}", state_histogram(&list));
                // the first frames: the burst on frame 0
                println!(
                    "           searching updates in the first 20 frames: {:?}",
                    &r.searching_per_frame[..20.min(r.searching_per_frame.len())]
                );
                let late = r.searching_per_frame.len().saturating_sub(40);
                println!("           ... and frames {late}..{}: {:?}", r.searching_per_frame.len(), &r.searching_per_frame[late..]);
            }
        }
    }
    println!("-- the same hunters with their first sense/repath phase staggered by 0..0.24 s (one warm-up frame each) --");
    for &n in &hunters {
        let mut list = make_hunters(&base.g, &content, n, false, "STAG");
        // warm up unevenly: brain i waits i * 0.24/n s before its first frame, with the runner standing
        let prof = Difficulty::Standard.profile();
        let relays_taken = vec![false; base.g.fac.relays.len()];
        let batteries_taken = vec![false; base.g.fac.batteries.len()];
        let hiding_disabled = vec![false; base.g.fac.hiding.len()];
        let ctx = Ctx {
            fac: &base.g.fac,
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
        let o = base.g.window_origin();
        let abs = along(&base.route, &base.cum, 0.0);
        let t = Target { pos: (abs.0 - o.0 as f32, abs.1 - o.1 as f32), noise: NOISE_SPRINT, flashlight_on: true, lamp_charge: 1.0, sprinting: true, ..Default::default() };
        for (i, b) in list.iter_mut().enumerate() {
            let steps = (i as f32 * 0.24 / n as f32 / DT) as usize;
            for _ in 0..steps {
                let _ = b.update(DT, &ctx, &t, 1, 0, &prof);
            }
        }
        let r = brain_run(&base.g, &content, &mut list, &base.route, &base.cum, frames);
        print_brain_run("staggered", n, &r);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// 3. One distance field against one A* per hunter
// ---------------------------------------------------------------------------------------------------------------------

#[derive(PartialEq)]
struct Item(f32, u32);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

const SQRT2: f32 = std::f32::consts::SQRT_2;

/// Whether the 8-way step `c -> c + (dx, dy)` is legal on the hunter grid (no corner cutting), as `plan_route_wet` and
/// `Facility::find_path` take it.
fn step_ok(fac: &Facility, c: Cell, dx: i32, dy: i32) -> bool {
    let n = Cell::new(c.x + dx, c.y + dy);
    if !fac.hunter_open(n) {
        return false;
    }
    !(dx != 0 && dy != 0 && (!fac.hunter_open(Cell::new(c.x + dx, c.y)) || !fac.hunter_open(Cell::new(c.x, c.y + dy))))
}

/// A* with a closed set (what `plan_route_wet` does, without water, avoid cells or gates). Returns (found, cost, nodes
/// expanded (closed), heap pops).
fn astar(fac: &Facility, from: Cell, to: Cell) -> (bool, f32, usize, usize) {
    if !fac.hunter_open(from) || !fac.hunter_open(to) {
        return (false, 0.0, 0, 0);
    }
    let w = fac.cols;
    let idx = |c: Cell| (c.y * w + c.x) as usize;
    let h = |c: Cell| {
        let dx = (c.x - to.x).abs() as f32;
        let dy = (c.y - to.y).abs() as f32;
        dx.max(dy) + (SQRT2 - 1.0) * dx.min(dy)
    };
    let n = (fac.cols * fac.rows) as usize;
    let mut g = vec![f32::INFINITY; n];
    let mut closed = vec![false; n];
    let mut heap = BinaryHeap::new();
    g[idx(from)] = 0.0;
    heap.push(Item(h(from), idx(from) as u32));
    let (mut expanded, mut pops) = (0, 0);
    while let Some(Item(_, ci)) = heap.pop() {
        pops += 1;
        let c = Cell::new(ci as i32 % w, ci as i32 / w);
        if c == to {
            return (true, g[ci as usize], expanded, pops);
        }
        if closed[ci as usize] {
            continue;
        }
        closed[ci as usize] = true;
        expanded += 1;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) == (0, 0) || !step_ok(fac, c, dx, dy) {
                    continue;
                }
                let nb = Cell::new(c.x + dx, c.y + dy);
                let ng = g[ci as usize] + if dx != 0 && dy != 0 { SQRT2 } else { 1.0 };
                if ng < g[idx(nb)] {
                    g[idx(nb)] = ng;
                    heap.push(Item(ng + h(nb), idx(nb) as u32));
                }
            }
        }
    }
    (false, 0.0, expanded, pops)
}

/// The reverse Dijkstra field from `goal`: the cost to reach `goal` from every reachable cell (the graph is symmetric on
/// one walk surface, so reverse = forward). Returns (field, cells settled).
fn field(fac: &Facility, goal: Cell) -> (Vec<f32>, usize) {
    let n = (fac.cols * fac.rows) as usize;
    let w = fac.cols;
    let mut d = vec![f32::INFINITY; n];
    if !fac.hunter_open(goal) {
        return (d, 0);
    }
    let idx = |c: Cell| (c.y * w + c.x) as usize;
    let mut heap = BinaryHeap::new();
    d[idx(goal)] = 0.0;
    heap.push(Item(0.0, idx(goal) as u32));
    let mut settled = 0;
    while let Some(Item(dc, ci)) = heap.pop() {
        if dc > d[ci as usize] {
            continue;
        }
        settled += 1;
        let c = Cell::new(ci as i32 % w, ci as i32 / w);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) == (0, 0) || !step_ok(fac, c, dx, dy) {
                    continue;
                }
                let nb = Cell::new(c.x + dx, c.y + dy);
                let nd = dc + if dx != 0 && dy != 0 { SQRT2 } else { 1.0 };
                if nd < d[idx(nb)] {
                    d[idx(nb)] = nd;
                    heap.push(Item(nd, idx(nb) as u32));
                }
            }
        }
    }
    (d, settled)
}

/// The route from `from` down the field to the goal (steepest descent), its cell count and cost.
fn descend(fac: &Facility, d: &[f32], from: Cell) -> Option<(usize, f32)> {
    let w = fac.cols;
    let idx = |c: Cell| (c.y * w + c.x) as usize;
    if !d[idx(from)].is_finite() {
        return None;
    }
    let (mut at, mut cost, mut cells) = (from, 0.0f32, 1usize);
    while d[idx(at)] > 0.0 {
        let mut best: Option<(f32, Cell, f32)> = None;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) == (0, 0) || !step_ok(fac, at, dx, dy) {
                    continue;
                }
                let nb = Cell::new(at.x + dx, at.y + dy);
                let step = if dx != 0 && dy != 0 { SQRT2 } else { 1.0 };
                let v = d[idx(nb)] + step;
                if best.map_or(true, |(bv, _, _)| v < bv) {
                    best = Some((v, nb, step));
                }
            }
        }
        let (_, nb, step) = best?;
        at = nb;
        cost += step;
        cells += 1;
        if cells > 100_000 {
            return None;
        }
    }
    Some((cells, cost))
}

fn best_of<T>(reps: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::MAX;
    let mut last = None;
    for _ in 0..reps {
        let t0 = Instant::now();
        let v = f();
        best = best.min(t0.elapsed().as_secs_f64() * 1e6);
        last = Some(v);
    }
    (best, last.unwrap())
}

#[test]
#[ignore = "timing benchmark: run with --ignored --nocapture"]
fn bulk_shared_field_vs_astar() {
    let content = Content::load();
    let catalog = Catalog::load();
    let mut sc = scene(&content, &catalog, 0, 0, false);
    let fac = &sc.g.fac;
    let open = fac.floor_cells().into_iter().filter(|c| fac.hunter_open(*c)).count();
    println!("window {}x{} cells, {open} open hunter cells", fac.cols, fac.rows);
    let runner = cell_of(sc.g.p.pos);
    let (field_us, (d, settled)) = best_of(20, || field(fac, runner));
    println!("one reverse Dijkstra field from the runner cell: {settled} cells settled, {field_us:.0} us");
    let spread_n = [8usize, 32, 96, 192];
    for spread in [false, true] {
        for &n in &spread_n {
            let list = make_hunters(&sc.g, &content, n, spread, "FIELD");
            let cells: Vec<Cell> = list.iter().map(|b| cell_of(b.pos)).collect();
            let mut nodes = 0usize;
            let mut pops = 0usize;
            let mut found = 0usize;
            let mut mismatch = 0usize;
            let (astar_us, _) = best_of(5, || {
                nodes = 0;
                pops = 0;
                found = 0;
                for &c in &cells {
                    let (ok, _, e, p) = astar(fac, c, runner);
                    nodes += e;
                    pops += p;
                    found += ok as usize;
                }
            });
            for &c in &cells {
                let (ok, cost, _, _) = astar(fac, c, runner);
                let via = descend(fac, &d, c);
                match (ok, via) {
                    (true, Some((_, fc))) if (fc - cost).abs() < 1e-2 => {}
                    (false, None) => {}
                    _ => mismatch += 1,
                }
            }
            let (lookup_us, steps) = best_of(5, || cells.iter().filter_map(|c| descend(fac, &d, *c)).map(|(n, _)| n).sum::<usize>());
            println!(
                "{} N={n:>3}: {n} x A* = {astar_us:>8.0} us ({:.0} nodes/search avg, {:.0} heap pops/search, {found}/{n} found) | 1 field + {n} descents = {:>7.0} us ({steps} route cells in all) | A* per frame if all replan = {:.2} ms vs field once = {:.2} ms | route-cost mismatches {mismatch}",
                if spread { "spread" } else { "ring  " },
                nodes as f32 / n as f32,
                pops as f32 / n as f32,
                field_us + lookup_us,
                astar_us / 1000.0,
                (field_us + lookup_us) / 1000.0,
            );
        }
    }
    // the runner crosses a cell every 1/RUNNER_SPEED s: how often the field is stale
    println!(
        "the runner changes cell {:.1} times a second at {RUNNER_SPEED} m/s ({:.2} per 60 Hz frame): a field rebuilt per cell change costs {:.2} ms per 60 Hz frame on average, one per HUNTER_REPATH_CHASE (0.24 s) tick {:.2} ms",
        RUNNER_SPEED,
        RUNNER_SPEED / 60.0,
        field_us as f32 / 1000.0 * RUNNER_SPEED / 60.0,
        field_us as f32 / 1000.0 / (0.24 * 60.0),
    );
    sc.travelled = 0.0;
}

// ---------------------------------------------------------------------------------------------------------------------
// 4. Which search ran, and who asked
// ---------------------------------------------------------------------------------------------------------------------

#[test]
#[ignore = "timing benchmark: run with --ignored --nocapture"]
fn bulk_search_attribution() {
    let content = Content::load();
    let catalog = Catalog::load();
    let frames = env_usize("BULK_FRAMES", 240);
    let every = env_usize("BULK_EVERY", 8) as u64;
    let min = env_usize("BULK_MIN", 256);
    let configs = [(96usize, 0usize), (0, 96), (96, 96), (32, 32)];
    for (n, m) in configs {
        let mut sc = scene(&content, &catalog, n, m, false);
        reset_counters();
        SAMPLE_EVERY.store(every, Relaxed);
        SAMPLE_MIN.store(min, Relaxed);
        DUMP.store(std::env::var("BULK_DUMP").is_ok(), Relaxed);
        ARMED.store(true, Relaxed);
        SAMPLING.store(true, Relaxed);
        let f = run_world(&mut sc, &content, &catalog, frames);
        SAMPLING.store(false, Relaxed);
        ARMED.store(false, Relaxed);
        let allocs = ALLOCS.load(Relaxed);
        println!(
            "== hunters {n}, bots {m}, {frames} frames: {:.0} allocations/frame, {:.1} MB/frame, Facility::find_path calls/frame ~ {:.2} (arrays of {} bytes seen {}x), sampled 1 in {every} of the {} allocations >= {min} bytes",
            allocs as f32 / frames as f32,
            BYTES.load(Relaxed) as f32 / 1e6 / frames as f32,
            GRID_ALLOCS.load(Relaxed) as f32 / 2.0 / frames as f32,
            GRID_BYTES.load(Relaxed),
            GRID_ALLOCS.load(Relaxed),
            BIG.load(Relaxed),
        );
        let _ = f;
        let total: u64 = TALLY_N.iter().map(|t| t.load(Relaxed)).sum::<u64>().max(1);
        let mut rows: Vec<(u64, u64, usize, usize)> = (0..NS * NC)
            .map(|i| (TALLY_N[i].load(Relaxed), TALLY_B[i].load(Relaxed), i / NC, i % NC))
            .filter(|r| r.0 > 0)
            .collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        for (cnt, bytes, s, c) in rows.into_iter().take(10) {
            let sn = SEARCHES.get(s).map_or("other (no search function on the stack)", |x| x.1);
            let cn = CALLERS.get(c).map_or("other caller", |x| x.1);
            println!("   {:>5.1}% of sampled allocations ({} samples, {:.1} KB avg) | {sn} <- {cn}", 100.0 * cnt as f32 / total as f32, cnt, bytes as f32 / cnt as f32 / 1024.0);
        }
        let sizes: Vec<String> = SIZE_HIST
            .iter()
            .enumerate()
            .filter(|(_, v)| v.load(Relaxed) > 0)
            .map(|(i, v)| format!("<2^{i}:{}", v.load(Relaxed)))
            .collect();
        println!("   sampled size classes: {}", sizes.join(" "));
    }
}
