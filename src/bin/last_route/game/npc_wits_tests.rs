//! The bots' wits on small hand-built floors: cover when they flee, the memory of a hunter they saw, the revive that
//! waits for a hunter to leave, and the way out of a jam on a plain walk. A child of `game::npc` for its private
//! functions (`flee_route`, `spot_safe`, `revive_walk`, `follow`, `walk_toward`).

use super::*;

/// A floor of walls with these rectangles (x, y, width, height in cells) opened.
fn floor(open: &[(i32, i32, i32, i32)]) -> Facility {
    let mut f = Facility::blank("BOT-WITS", 35, 35);
    for &(x, y, w, h) in open {
        f.open_cells(x, y, w, h);
    }
    f.rebuild_grids();
    f
}

/// One open room, 15 cells square, from cell (5, 5).
fn room() -> Facility {
    floor(&[(5, 5, 15, 15)])
}

/// A corridor along x (cells 5 to 19 on row 10) with a side corridor going off it at x 15 (rows 11 to 16).
fn corner() -> Facility {
    floor(&[(5, 10, 15, 1), (15, 11, 1, 6)])
}

/// A joined bot standing at `at` on level 0 that has not moved yet.
fn bot(at: (f32, f32)) -> Npc {
    Npc {
        id: 0, name: "B-00001".to_string(), slot: 1, at, level: 0, y: 0.0, walk: None, edge_clock: 0.0, hop: None, facing: 0.0, turn_v: 0.0, heading: (0.0, 0.0),
        path: vec![], path_age: 0.0, walking: false, stride: 0.0, smart: 50, key: None, joined: true,
        join_radius: 0.0, door: None, hidden: None, errand: None, dwell: 0.0, visited: vec![], clear_time: 0.0,
        claim: None,
        searched: vec![], last_door: None, deep_cells: vec![], fleeing: false, heard: None, goal: GoalState::default(), flee: FleeState::default(), life: LifeState::default(),
    }
}

/// A scene of `fac` with no hunter, no spot and nobody down.
fn scene(fac: &Facility) -> Scene<'_> {
    Scene { fac, disabled: &[], leader: (0.0, 0.0), seen: &[], searching: &[], runner_level: 0, nav: None, loot: &[], spots: &[], search_all: &[], downed: &[], peers: &[] }
}

fn relay_at(c: Cell) -> Spot {
    Spot { kind: SpotKind::Relay, level: 0, cell: c, pos: c.center(), claim: None }
}

#[test]
fn a_fleeing_bot_ducks_round_the_corner_instead_of_running_down_the_hunters_corridor() {
    let fac = corner();
    let t = Tuning::get();
    let n = bot((15.5, 10.5));
    let hunter = (9.5, 10.5);
    assert!(fac.has_line_of_sight(hunter, n.at), "the hunter looks down the corridor at the bot");
    let route = flee_route(&n, &fac, &[hunter], &t.flee).expect("a way to run");
    let end = *route.last().unwrap();
    let end_at = Cell::new(end.0, end.1).center();
    assert_eq!(end.0, 15, "it runs into the side corridor: {route:?}");
    assert!(!fac.has_line_of_sight(hunter, end_at), "the flight ends out of the hunter's sight");
    assert!(pdist(end_at, hunter) > pdist(n.at, hunter), "and farther from the hunter than it began");
    // without the cover rule the farthest cell wins: the corridor's end, in plain sight
    let mut plain = t.flee.clone();
    plain.cover_bonus = 0.0;
    let old = flee_route(&n, &fac, &[hunter], &plain).expect("a way to run");
    let old_end = *old.last().unwrap();
    assert_eq!(old_end, (19, 10), "the old rule runs to the corridor's end: {old:?}");
    assert!(fac.has_line_of_sight(hunter, Cell::new(old_end.0, old_end.1).center()));
}

#[test]
fn the_cover_rule_tests_no_more_cells_than_the_data_allows() {
    let fac = corner();
    let t = Tuning::get();
    let n = bot((15.5, 10.5));
    let hunter = (9.5, 10.5);
    // no cell may be tested: the route is the old one
    let mut none = t.flee.clone();
    none.cover_checks = 0;
    let route = flee_route(&n, &fac, &[hunter], &none).expect("a way to run");
    assert_eq!(route.last().copied(), Some((19, 10)));
}

#[test]
fn a_bot_sees_the_hunter_in_front_of_it_and_not_one_behind_or_round_a_corner() {
    let fac = corner();
    let t = Tuning::get();
    let mut n = bot((12.5, 10.5));
    n.heading = (1.0, 0.0);
    let ahead = [((18.5, 10.5), false)];
    assert_eq!(seen_hunter(&n, &fac, &ahead, &t.flee), Some((18.5, 10.5)));
    let behind = [((6.5, 10.5), false)];
    assert!(pdist(n.at, behind[0].0) > t.flee.sense_range);
    assert_eq!(seen_hunter(&n, &fac, &behind, &t.flee), None, "outside the view cone and the sense range");
    let hidden = [((15.5, 15.5), false)];
    assert_eq!(seen_hunter(&n, &fac, &hidden, &t.flee), None, "round the corner, no line of sight");
    let both = [((18.5, 10.5), false), ((14.5, 10.5), true)];
    assert_eq!(seen_hunter(&n, &fac, &both, &t.flee), Some((14.5, 10.5)), "the nearest one in view");
}

#[test]
fn a_bot_keeps_off_the_spot_where_it_just_saw_a_hunter_until_the_memory_fades() {
    let fac = room();
    let t = Tuning::get();
    let mut n = bot((6.5, 6.5));
    let dt = 1.0 / 60.0;
    let spots = [relay_at(Cell::new(16, 16))];
    let sc = Scene { spots: &spots, ..scene(&fac) };
    assert_eq!(pick_goal(&n, &sc, t).map(|s| s.cell), Some(Cell::new(16, 16)), "nothing seen: the relay is wanted");
    // a hunter is seen beside the relay, then goes out of view
    note_sighting(&mut n, Some((15.5, 15.5)), &t.flee, dt);
    note_sighting(&mut n, None, &t.flee, dt);
    assert!(!spot_safe(&n, &sc, spots[0].pos, t), "the relay is where the hunter was");
    assert!(spot_safe(&n, &sc, (6.5, 16.5), t), "a spot well away from the sighting is fine");
    assert_eq!(pick_goal(&n, &sc, t), None, "the bot does not walk back to the hunter it saw");
    for _ in 0..((t.flee.memory + 0.5) / dt) as usize {
        note_sighting(&mut n, None, &t.flee, dt);
    }
    assert!(spot_safe(&n, &sc, spots[0].pos, t), "the memory has faded");
    assert!(pick_goal(&n, &sc, t).is_some());
    // a change of floor frame drops the memory at once
    note_sighting(&mut n, Some((15.5, 15.5)), &t.flee, dt);
    n.shift((1.0, 0.0));
    assert!(spot_safe(&n, &sc, spots[0].pos, t));
}

#[test]
fn a_bot_leaves_a_downed_teammate_while_a_hunter_stands_over_it_and_goes_once_it_has_left() {
    let fac = room();
    let t = Tuning::get();
    let dt = 1.0 / 60.0;
    let mut presses = vec![];
    let body = (12.5, 10.5);
    let downed = [(0, body, None::<u32>)];
    let mut n = bot((8.5, 10.5));
    assert!(pdist(n.at, body) <= t.follow_radius * t.revive_range_factor);
    let over_it = [((13.5, 10.5), false)];
    let sc = Scene { downed: &downed, seen: &over_it, ..scene(&fac) };
    assert!(!revive_walk(&mut n, &sc, t, dt, &mut presses), "a hunter 1 m from the body: the bot stays away");
    assert_eq!(n.at, (8.5, 10.5));
    let gone = [((18.5, 18.5), false)];
    assert!(pdist(gone[0].0, body) > t.revive_safe_range);
    let sc = Scene { downed: &downed, seen: &gone, ..scene(&fac) };
    assert!(revive_walk(&mut n, &sc, t, dt, &mut presses), "the hunter has left: the bot goes");
    // a hunter the bot saw at the body a moment ago keeps it away too, though it is out of sight now
    let mut wary = bot((8.5, 10.5));
    note_sighting(&mut wary, Some((13.0, 10.5)), &t.flee, dt);
    let sc = Scene { downed: &downed, ..scene(&fac) };
    assert!(!revive_walk(&mut wary, &sc, t, dt, &mut presses));
    // the range is data
    let mut bold = t.clone();
    bold.revive_safe_range = 0.0;
    let sc = Scene { downed: &downed, seen: &over_it, ..scene(&fac) };
    assert!(revive_walk(&mut bot((8.5, 10.5)), &sc, &bold, dt, &mut presses), "a safe range of zero walks in as before");
}

/// A bot following its runner across the room, held on its start point (a body wedged on a corner) until it steps
/// aside: whether it stepped aside, its jam count, and where it stood after 40 s.
fn wedged_follow() -> (bool, u32, (f32, f32)) {
    let fac = room();
    let t = Tuning::get();
    let (start, leader) = ((6.5, 12.5), (14.5, 12.5));
    let mut n = bot(start);
    let dt = 1.0 / 60.0;
    let mut presses = vec![];
    let mut stepped = false;
    for _ in 0..(40 * 60) {
        follow(&mut n, &fac, leader, t, dt, &mut presses);
        if n.goal.jam.aside > 0.0 {
            stepped = true;
        } else if !stepped {
            n.at = start;
        }
    }
    (stepped, n.goal.jam.count, n.at)
}

#[test]
fn a_following_bot_wedged_in_place_steps_aside_and_then_reaches_its_runner() {
    let t = Tuning::get();
    let (stepped, jams, at) = wedged_follow();
    assert!(stepped, "after a fresh route and a door check did not help, it stepped aside");
    assert_eq!(jams, 3, "one rung of the ladder each stuck window");
    let n = bot(at);
    assert!(pdist(at, (14.5, 12.5)) <= follow_range(&n, t) + 0.01, "and walked on to its runner: {at:?}");
}

#[test]
fn a_wedged_follow_is_the_same_on_every_run() {
    assert_eq!(wedged_follow(), wedged_follow());
}

#[test]
fn a_bot_that_is_not_stuck_never_steps_aside() {
    let fac = room();
    let t = Tuning::get();
    let leader = (14.5, 12.5);
    let mut n = bot((6.5, 12.5));
    let dt = 1.0 / 60.0;
    let mut presses = vec![];
    for _ in 0..(20 * 60) {
        follow(&mut n, &fac, leader, t, dt, &mut presses);
        assert_eq!(n.goal.jam.aside, 0.0);
    }
    assert_eq!(n.goal.jam.count, 0, "a free walk and a bot standing in its slot are not jams");
    assert!(pdist(n.at, leader) <= follow_range(&n, t) + 0.01);
}

#[test]
fn a_step_blocked_again_right_after_a_fresh_route_does_not_search_a_route_again() {
    let fac = room();
    let t = Tuning::get();
    let dt = 1.0 / 60.0;
    // a quarter of a cell from the centre of an edge cell toward the wall, with a route that leads into the wall
    let start = (5.25, 10.5);
    let goal = (15.5, 10.5);
    let mut n = bot(start);
    n.path = vec![(4, 10)];
    let mut presses = vec![];
    walk_toward(&mut n, &fac, goal, false, t, dt, &mut presses);
    assert_eq!(n.at, start);
    assert_eq!(n.path_age, 0.0, "the first blocked step planned a route");
    assert!(!n.path.is_empty() && !n.path.contains(&(4, 10)));
    // the same blocked step on the next frame: no second search, the bot waits
    n.path = vec![(4, 10)];
    walk_toward(&mut n, &fac, goal, false, t, dt, &mut presses);
    assert_eq!(n.path, vec![(4, 10)], "no route was searched on the frame after one");
    assert!(n.path_age > 0.0 && !n.walking);
    // the next repath interval plans as always, and the bot walks off
    for _ in 0..60 {
        walk_toward(&mut n, &fac, goal, false, t, dt, &mut presses);
    }
    assert!(pdist(n.at, start) > 0.5, "it walked on: {:?}", n.at);
}
