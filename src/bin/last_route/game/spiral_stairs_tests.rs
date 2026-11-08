//! Spiral stairs, end to end (KT's reports: "can easily get stuck behind props going down the spiral stairs" and "spiral
//! stairs are super broken when going down to inferno level"). Registered by game/mod.rs as
//! `#[cfg(test)] mod spiral_stairs_tests;`.
//!
//! * `a_spiral_descent_walks_from_the_hall_into_the_level_below_for_every_pair`: for many seeds and every level pair
//!   (0 to -1, -1 to -2, -2 to -3 the Inferno) the runner is put in the hall, walks to the landing, down the helix with the
//!   game's own `Game::step`, and out of the foot into the level below by a route over dry cells; then, in the Inferno,
//!   keeps standing there past the secret creature's dwell clock, because that is what a run does next.
//! * `the_secret_creature_may_spawn_in_the_middle_of_the_hunter_frame`: the regression for the crash that gave those
//!   30 seconds their name (the creature is pushed onto the hunter list inside the hunter frame, after the per-hunter
//!   aim list was sized).
//! * the generator-level validators (no prop or locker in a stair's zone or ring, a body can walk it, no lava or cave
//!   decoration in the Inferno bay) are `world/spiral_stairs_tests.rs`.

use super::level_stack::body_fits_at_y;
use super::{Controls, Game};
use crate::audio::Sounds;
use crate::config::{Difficulty, PLAYER_RADIUS};
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::{Cell, LinkKind, Stair};
use crate::world::flight::FlightShape;
use crate::world::infinite::INFERNO_LEVEL;

const FRAME: f32 = 1.0 / 60.0;

/// How the runner is steered: the flight shape it looks for, how far its heading wanders off the next waypoint (rad, a
/// smooth random walk, as a hand on a mouse), whether it sprints, and the seconds it stands in the Inferno afterwards.
#[derive(Clone, Copy)]
struct Gait {
    shape: FlightShape,
    wobble: f32,
    sprint: bool,
    inferno_wait: f32,
}

impl Gait {
    fn new(shape: FlightShape, wobble: f32, sprint: bool) -> Gait {
        Gait { shape, wobble, sprint, inferno_wait: 35.0 }
    }
}

/// What one descent did.
#[derive(Default)]
struct Out {
    reached_landing: bool,
    reached_foot: bool,
    reached_out: bool,
    end_level: i32,
    fell: bool,
    unfit: usize,
    why: String,
}

fn origin(g: &Game) -> (i32, i32) {
    g.inf.as_ref().expect("an infinite run").origin()
}

/// Steers the body through each absolute-cell target in turn, holding forward, as `stack_entry_tests` does. True when
/// the last one was reached within `max` frames; otherwise `o.why` says where it stalled. Counts the frames on which
/// the body did not fit its place or fell.
fn follow(g: &mut Game, targets: &[(f32, f32)], o: &mut Out, max: usize, gait: Gait, c: &Content, cat: &Catalog, tag: &str) -> bool {
    let ctl = Controls { fwd: 1.0, sprint: gait.sprint, ..Default::default() };
    let mut rng = crate::world::rng::Rng::new(&format!("{tag}:WOBBLE"));
    let mut off = 0.0f32;
    let mut i = 0;
    for f in 0..max {
        let oc = origin(g);
        let t = targets[i];
        let (dx, dz) = (t.0 - oc.0 as f32 - g.p.pos.0, t.1 - oc.1 as f32 - g.p.pos.1);
        if (dx * dx + dz * dz).sqrt() < 0.3 {
            if i + 1 >= targets.len() {
                return true;
            }
            i += 1;
            continue;
        }
        // a hand on a mouse: the heading drifts off the waypoint and is pulled back (never more than `wobble` off)
        off = (off * 0.97 + (rng.next_f32() - 0.5) * 0.12 * gait.wobble).clamp(-gait.wobble, gait.wobble);
        g.p.yaw = (-dx).atan2(-dz) + off;
        g.step(FRAME, &ctl, c, cat, &Sounds::default());
        o.fell |= g.p.falling();
        let stack = &g.inf.as_ref().unwrap().stack;
        if !body_fits_at_y(&g.fac, stack, g.p.pos.0, g.p.pos.1, g.p.y(), PLAYER_RADIUS) {
            o.unfit += 1;
            if o.why.is_empty() {
                o.why = format!("{tag}: the body does not fit at frame {f}, target {i}/{}, pos {:?}, y {}", targets.len(), g.p.pos, g.p.y());
            }
        }
    }
    if o.why.is_empty() {
        o.why = format!("{tag}: stalled at target {i}/{} pos {:?} y {}", targets.len(), g.p.pos, g.p.y());
    }
    false
}

/// A breadth-first route over cells hunters stand on that are not a stair well, `depth` steps long, from `from`: the way
/// a runner leaves the foot of a flight (the well cells under the helix are not a way out).
fn dry_path(f: &crate::world::Facility, from: Cell, depth: usize) -> Vec<Cell> {
    let wells = f.well_cells_all();
    let mut parent: std::collections::HashMap<(i32, i32), (i32, i32)> = std::collections::HashMap::new();
    let mut frontier = vec![from];
    parent.insert((from.x, from.y), (from.x, from.y));
    for d in 0..=depth {
        if d == depth {
            let mut at = frontier[0];
            let mut out = vec![at];
            while at != from {
                let p = parent[&(at.x, at.y)];
                at = Cell::new(p.0, p.1);
                out.push(at);
            }
            out.reverse();
            return out;
        }
        let mut next = vec![];
        for c in &frontier {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if f.hunter_open(n) && !wells.contains(&n) && !parent.contains_key(&(n.x, n.y)) {
                    parent.insert((n.x, n.y), (c.x, c.y));
                    next.push(n);
                }
            }
        }
        if next.is_empty() {
            return vec![];
        }
        frontier = next;
    }
    vec![]
}

/// Walks spiral number `idx` going down on `level` in the window of `seed`: hall, landing, helix, foot, ten cells out.
/// In the Inferno it then stands for `inferno_wait` seconds. `None` when the window has no such spiral.
fn descend(seed: &str, level: i32, idx: usize, gait: Gait, c: &Content, cat: &Catalog) -> Option<(Out, Stair)> {
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, seed, level, c, cat);
    // the hall walk starts from the window's start cell itself: on a sewer level enter_infinite stands the runner on the
    // nearest dry cell instead (endless.rs `dry_start`), and this walk is about the flight, not about that spot
    g.p.pos = g.fac.start.center();
    // the first step recentres the window by whole chunks: pick the flight after it
    g.step(FRAME, &Controls::default(), c, cat, &Sounds::default());
    let s = *g
        .fac
        .stairs
        .iter()
        .filter(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs) && s.shape == gait.shape)
        .nth(idx)?;
    let fp = crate::world::flight::well_footprint(&s.flight(level)?);
    let o0 = origin(&g);
    let abs = |c: Cell| (c.center().0 + o0.0 as f32, c.center().1 + o0.1 as f32);
    // the hall: from the level's start by A*; failing that from the far end of the landing's own margin
    let from = Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
    let mut path = g.fac.find_path(from, s.cell);
    if path.is_empty() {
        let hall = fp.upper_clear.iter().copied().max_by_key(|c| (c.x - s.cell.x).abs() + (c.y - s.cell.y).abs())?;
        g.p.pos = hall.center();
        path = g.fac.find_path(hall, s.cell);
    }
    let mut o = Out::default();
    if path.is_empty() {
        o.why = "no hall path to the landing".into();
        return Some((o, s));
    }
    for _ in 0..8 {
        g.keep_stack(c, cat);
    }
    let hall: Vec<(f32, f32)> = path.iter().map(|c| abs(*c)).collect();
    o.reached_landing = follow(&mut g, &hall, &mut o, 6000, gait, c, cat, "hall");
    if !o.reached_landing {
        return Some((o, s));
    }
    let walk: Vec<(f32, f32)> = s.walk_path(level, 0.25).into_iter().map(|p| (p.0 + o0.0 as f32, p.1 + o0.1 as f32)).collect();
    o.reached_foot = follow(&mut g, &walk, &mut o, 3000, gait, c, cat, "flight");
    o.end_level = g.inf.as_ref().unwrap().level;
    if !o.reached_foot {
        return Some((o, s));
    }
    // out of the foot into the level below
    let foot = Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
    let out = dry_path(&g.fac, foot, 10);
    if out.is_empty() {
        o.why = format!("no dry way out of the foot {foot:?}");
        return Some((o, s));
    }
    let o1 = origin(&g);
    let outp: Vec<(f32, f32)> = out.iter().map(|c| (c.center().0 + o1.0 as f32, c.center().1 + o1.1 as f32)).collect();
    o.reached_out = follow(&mut g, &outp, &mut o, 6000, gait, c, cat, "out");
    o.end_level = g.inf.as_ref().unwrap().level;
    // a run goes on: in the Inferno the creature's dwell clock runs out while the runner is still there. Nothing may panic
    // and the hunter list and the per-hunter lists must still be the same length.
    if o.end_level == INFERNO_LEVEL && gait.inferno_wait > 0.0 {
        for _ in 0..(gait.inferno_wait / FRAME) as usize {
            g.step(FRAME, &Controls::default(), c, cat, &Sounds::default());
            if g.hunter_aim.len() != g.hunters.len() {
                o.why = format!("the aim list has {} entries for {} hunters", g.hunter_aim.len(), g.hunters.len());
                o.unfit += 1;
                break;
            }
        }
    }
    Some((o, s))
}

/// Walks every flight of `gait.shape` going down that `seeds` seeds have on each of `levels` (at most two per window),
/// and returns the failures. `at_least` is the number of descents each level must have found, so a level pair cannot
/// pass by looking at nothing.
fn sweep(levels: &[(i32, usize)], seeds: usize, gait: Gait) -> Vec<String> {
    let started = crate::shared::clock::Instant::now();
    let (c, cat) = (Content::load(), Catalog::load());
    let mut bad: Vec<String> = vec![];
    for &(level, at_least) in levels {
        let mut walked = 0;
        for k in 0..seeds {
            let seed = format!("ROOM-SPIRAL-{k}");
            for idx in 0..2 {
                let Some((o, s)) = descend(&seed, level, idx, gait, &c, &cat) else { break };
                walked += 1;
                let ok = o.reached_landing && o.reached_foot && o.reached_out && !o.fell && o.unfit == 0 && o.end_level == level - 1;
                if !ok {
                    bad.push(format!(
                        "level {level} -> {} {seed} {:?} {idx}: landing {} foot {} out {} fell {} unfit {} :: {}",
                        level - 1,
                        s.shape,
                        o.reached_landing,
                        o.reached_foot,
                        o.reached_out,
                        o.fell,
                        o.unfit,
                        o.why
                    ));
                }
            }
        }
        assert!(walked >= at_least, "level {level}: only {walked} {:?} flights found in {seeds} seeds", gait.shape);
        eprintln!("level {level}: {walked} {:?} descents", gait.shape);
    }
    eprintln!("{:?} descents took {:.1}s", gait.shape, started.elapsed().as_secs_f32());
    bad
}

fn assert_all_walked(bad: Vec<String>) {
    assert!(bad.is_empty(), "{} descents failed:\n{}", bad.len(), bad.join("\n"));
}

/// Every spiral going down, on each level pair (0 to -1, -1 to -2 where the data makes one, -2 to -3 the Inferno), walks
/// from the hall into the level below; in the Inferno the runner then stands past the creature's dwell clock.
#[test]
fn a_spiral_descent_walks_from_the_hall_into_the_level_below_for_every_pair() {
    assert_all_walked(sweep(&[(0, 3), (-1, 0), (-2, 3)], 14, Gait::new(FlightShape::Spiral, 0.0, false)));
}

/// The same with a wandering heading (about 30 degrees off the waypoint, a hand on a mouse) and at a sprint: nothing on
/// the helix, its landing or the way out of its foot holds a runner who does not walk the exact line.
#[test]
fn a_wandering_or_sprinting_runner_still_gets_down_every_spiral() {
    assert_all_walked(sweep(&[(0, 2), (-2, 2)], 8, Gait::new(FlightShape::Spiral, 0.5, false)));
    assert_all_walked(sweep(&[(0, 2), (-2, 2)], 8, Gait::new(FlightShape::Spiral, 0.0, true)));
}

/// The other two shapes on every level pair they exist on, so the fixes are general and not the spiral's alone.
#[test]
fn switchbacks_and_straights_walk_into_the_level_below_too() {
    assert_all_walked(sweep(&[(0, 0), (-1, 2), (-2, 2)], 5, Gait::new(FlightShape::Switchback, 0.0, false)));
    assert_all_walked(sweep(&[(-1, 1), (-2, 1)], 10, Gait::new(FlightShape::Straight, 0.0, false)));
}

/// DRY-START WALK (mark for the test-and-fix agent: if this fails, the message names the seed, spiral and frame). The
/// runner is left where `enter_infinite` puts it on the sewer level (-1, `dry_start`) and walks to each downward spiral's
/// landing by A* over the walking grid, as a player would. A stall at the landing or "body does not fit" here is a real
/// path-following problem near landings; passing means `dry_start` is joined to the flights.
#[test]
fn a_walk_from_the_dry_start_reaches_every_spiral_landing_on_the_sewer_level() {
    let (c, cat) = (Content::load(), Catalog::load());
    let gait = Gait::new(FlightShape::Spiral, 0.0, false);
    let mut bad = vec![];
    let mut walked = 0;
    for k in 0..8 {
        let seed = format!("ROOM-SPIRAL-{k}");
        let mut idx = 0;
        loop {
            let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &seed, -1, &c, &cat);
            g.step(FRAME, &Controls::default(), &c, &cat, &Sounds::default());
            let Some(s) = g
                .fac
                .stairs
                .iter()
                .filter(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs) && s.shape == gait.shape)
                .nth(idx)
                .copied()
            else {
                break;
            };
            idx += 1;
            let o0 = origin(&g);
            let from = Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
            let path = g.fac.find_path(from, s.cell);
            if path.is_empty() {
                // not joined by the grid: the generator-level check in endless.rs covers the start; a landing on another island is not this walk's business
                continue;
            }
            for _ in 0..8 {
                g.keep_stack(&c, &cat);
            }
            let hall: Vec<(f32, f32)> = path.iter().map(|p| (p.center().0 + o0.0 as f32, p.center().1 + o0.1 as f32)).collect();
            let mut o = Out::default();
            walked += 1;
            if !follow(&mut g, &hall, &mut o, 6000, gait, &c, &cat, "dry-start hall") || o.unfit > 0 {
                bad.push(format!("{seed} spiral {idx}: {}", o.why));
            }
        }
    }
    if std::env::var_os("LR_TEST_VERBOSE").is_some() {
        eprintln!("dry-start walks: {walked} spiral landings reached");
    }
    assert!(bad.is_empty(), "{} walks failed:\n{}", bad.len(), bad.join("\n"));
}

/// The secret creature appears on the frame its dwell clock runs out, pushed onto the hunter list from inside the
/// hunter frame: the lists sized per hunter before that must not be indexed past their end (it panicked: 30 seconds
/// into every Inferno).
#[test]
fn the_secret_creature_may_spawn_in_the_middle_of_the_hunter_frame() {
    let (c, cat) = (Content::load(), Catalog::load());
    for (n, difficulty) in [(1, Difficulty::Relaxed), (2, Difficulty::Standard), (3, Difficulty::Nightmare)] {
        let mut g = Game::new_infinite_sim(difficulty, 7, &format!("ROOM-INFERNO-{n}"), INFERNO_LEVEL, &c, &cat);
        let before = g.hunters.len();
        g.step(FRAME, &Controls::default(), &c, &cat, &Sounds::default());
        // one tick short of the dwell: the next hunter frame fires the trigger
        g.inferno_secrets.dwell = crate::hunters::secret_creatures::Rules::get().dwell_seconds - 1e-3;
        g.step(FRAME, &Controls::default(), &c, &cat, &Sounds::default());
        assert!(g.hunters.len() > before, "{difficulty:?}: the creature came out on the frame the clock ran out");
        assert_eq!(g.hunter_aim.len(), g.hunters.len(), "{difficulty:?}: the per-hunter aim list follows the hunter list");
        // and the frames after it
        for _ in 0..120 {
            g.step(FRAME, &Controls::default(), &c, &cat, &Sounds::default());
        }
        assert_eq!(g.hunter_aim.len(), g.hunters.len());
    }
}
