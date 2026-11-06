//! End-to-end walks of every flight shape, down and back up, through the game's own frame (`Game::step`) with the
//! steered walker (steer.rs): one continuous space, so no y pop, one level change, no fade.

use super::steer::{walk_flight, walk_flight_abs_y, walk_flight_until, FlightWalk};
use crate::audio::Sounds;
use crate::config::Difficulty;
use crate::content::Content;
use crate::game::{pdist, Controls, Game};
use crate::world::catalog::Catalog;
use crate::world::facility::{LinkKind, Stair};
use crate::world::flight::FlightShape;

const FRAMES: usize = 1800;
/// The most the feet may move vertically in one frame (m): a walked flight is smooth, the old swap popped 1.7 to 3 m.
const MAX_STEP_DY: f32 = 0.25;
/// The levels whose downward flights include spirals: the maze (level 0) wells that the layout asks to be spirals, whose
/// landing and foot are the two ends of the doorway (world/flight_links.rs), and the -2 -> -3 bay links, which use the
/// default shape weights. The sewer (-1) has no spirals by its region weights.
const SPIRAL_LEVELS: [i32; 2] = [0, -2];

fn origin(g: &Game) -> (f32, f32) {
    let o = g.inf.as_ref().map_or((0, 0), |i| i.origin());
    (o.0 as f32, o.1 as f32)
}

fn nearest(g: &Game, dir: i32, shape: Option<FlightShape>) -> Option<Stair> {
    let me = g.p.pos;
    g.fac
        .stairs
        .iter()
        .copied()
        .filter(|s| s.dir == dir && matches!(s.kind, LinkKind::Stairs) && shape.map_or(true, |sh| s.shape == sh))
        .min_by(|a, b| pdist(a.cell.center(), me).partial_cmp(&pdist(b.cell.center(), me)).unwrap())
}

/// A window on `level` with a flight going down of `shape`, and that flight. The first frame recentres the window on
/// the body, so the flight is looked up again after it.
fn window_with(shape: FlightShape, level: i32, content: &Content, catalog: &Catalog) -> (Game, Stair) {
    for n in 0..300 {
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-WALK-{n}"), level, content, catalog);
        let Some(first) = nearest(&g, -1, Some(shape)) else {
            continue;
        };
        g.p.pos = first.cell.center();
        g.step(1.0 / 60.0, &Controls::default(), content, catalog, &Sounds::default());
        let s = nearest(&g, -1, Some(shape)).expect("the flight is still in the recentred window");
        return (g, s);
    }
    panic!("no test window on level {level} has a {shape:?} flight going down");
}

/// Walks `s` and checks the contract of one direction.
fn walk_and_check(g: &mut Game, s: Stair, content: &Content, catalog: &Catalog, what: &str) -> FlightWalk {
    let o = origin(g);
    let far = s.far_end().center();
    let far_abs = (far.0 + o.0, far.1 + o.1);
    let w = walk_flight(g, s, content, catalog, FRAMES);
    let o = origin(g);
    let end = (g.p.pos.0 + o.0, g.p.pos.1 + o.1);
    assert!(w.reached_end, "{what}: the walk reached the far end: {w:?}");
    assert_eq!(w.level_changes, 1, "{what}: exactly one level change: {w:?}");
    assert!(w.max_step_dy <= MAX_STEP_DY, "{what}: the feet never pop in y: {w:?}");
    assert!(((w.end_y - w.start_y).abs() - 5.0).abs() <= 0.35, "{what}: the flight rises one storey (5 m): {w:?}");
    assert_eq!(w.end_level, w.start_level + s.dir.signum(), "{what}: the level moved one way: {w:?}");
    assert_eq!(w.fades, 0, "{what}: no fade: {w:?}");
    assert!(pdist(end, far_abs) <= 1.0, "{what}: the body ends at the far end: {end:?} vs {far_abs:?}");
    w
}

/// Down a flight of `shape`, then back up the flight beneath it.
fn down_and_up(shape: FlightShape, level: i32) {
    let (content, catalog) = (Content::load(), Catalog::load());
    let (mut g, s) = window_with(shape, level, &content, &catalog);
    let down = walk_and_check(&mut g, s, &content, &catalog, "down");
    assert_eq!(down.end_level, level - 1);
    assert!(down.end_y < down.start_y, "down goes down");
    let up_stair = nearest(&g, 1, Some(shape)).expect("the flight below leads back up with the same shape");
    let up = walk_and_check(&mut g, up_stair, &content, &catalog, "up");
    assert_eq!(up.end_level, level);
    assert!(up.end_y > up.start_y, "up goes up");
    assert!((up.end_y - down.start_y).abs() <= 1.0, "up ends about where down started");
}

#[test]
fn walks_a_switchback_down_and_up() {
    down_and_up(FlightShape::Switchback, 0);
}

#[test]
fn walks_a_spiral_down_and_up() {
    for level in SPIRAL_LEVELS {
        down_and_up(FlightShape::Spiral, level);
    }
}

#[test]
fn walks_a_straight_down_and_up() {
    down_and_up(FlightShape::Straight, -1);
}

/// The swap at the midpoint is invisible in absolute y: the frame that changes the level moves the feet no more than
/// any other frame of the walk. The walk is deterministic, so the same walk one frame shorter shows the feet just before.
#[test]
fn the_swap_keeps_absolute_y_continuous() {
    let (content, catalog) = (Content::load(), Catalog::load());
    for (shape, level) in [(FlightShape::Switchback, 0), (FlightShape::Spiral, 0), (FlightShape::Spiral, -2)] {
        let (mut g, s) = window_with(shape, level, &content, &catalog);
        let at_swap = walk_flight_until(&mut g, s, &content, &catalog, FRAMES, true);
        assert_eq!(at_swap.level_changes, 1, "{shape:?}: the walk changed the level");
        let (mut g, s) = window_with(shape, level, &content, &catalog);
        let before = walk_flight(&mut g, s, &content, &catalog, at_swap.frames - 1);
        assert_eq!(before.level_changes, 0, "{shape:?}: the frame before the swap is still the old level");
        assert!(
            (at_swap.end_y - before.end_y).abs() < MAX_STEP_DY,
            "{shape:?}: absolute y across the swap: {} -> {}",
            before.end_y,
            at_swap.end_y
        );
        assert!(at_swap.max_step_dy < MAX_STEP_DY, "{shape:?}: no frame before the swap pops: {at_swap:?}");
    }
}

/// The absolute y trace of a whole walk is continuous (no frame above MAX_STEP_DY, no NaN) and ends one storey from
/// where it started, down and back up.
#[test]
fn the_abs_y_trace_is_continuous_down_and_up() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let (mut g, s) = window_with(FlightShape::Switchback, 0, &content, &catalog);
    let down = walk_flight_abs_y(&mut g, s, &content, &catalog, FRAMES);
    assert!(down.len() > 10, "the walk stepped frames");
    assert!(down.iter().all(|y| y.is_finite()));
    for w in down.windows(2) {
        assert!((w[1] - w[0]).abs() <= MAX_STEP_DY, "y pops {} -> {}", w[0], w[1]);
    }
    let (first, last) = (down[0], *down.last().unwrap());
    assert!(((first - last).abs() - 5.0).abs() <= 0.35, "one storey: {first} -> {last}");
    let up_stair = nearest(&g, 1, Some(FlightShape::Switchback)).expect("the flight below leads back up");
    let up = walk_flight_abs_y(&mut g, up_stair, &content, &catalog, FRAMES);
    for w in up.windows(2) {
        assert!((w[1] - w[0]).abs() <= MAX_STEP_DY, "y pops going up {} -> {}", w[0], w[1]);
    }
    assert!(*up.last().unwrap() > up[0] + 4.0, "up gains a storey");
}
