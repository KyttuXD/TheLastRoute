//! The walkable surface of a level-0 hunter's world: the footprint rules of Godot's `StreamedHunterNavigation`
//! (game/hunters/ai/streamed_hunter_navigation.gd) that the live window's `Facility` grid is built from.
//!
//! The grid is `Facility::hunter_open` over the runner's 3x3 window. `world::route::live::from_layout` sets its walls
//! and its solid props, and takes each prop's footprint from the rules here. The footprint is the prop's collider
//! (`collider_size`, else `blocker_size`: `_collider_rects` and `StreamedHunterNavigation._collect_solid_prop_cells`
//! read the collider the same way), and a prop no taller than `WALK_ON_HEIGHT` is floor to the runner and to the
//! hunters (`blocks_nothing`), so it is not solid.

/// `StreamedHunterNavigation.WALK_ON_HEIGHT` (streamed_hunter_navigation.gd): a solid prop whose collider is no taller
/// than this is walked over.
pub const WALK_ON_HEIGHT: f32 = 0.25;

/// `StreamedHunterNavigation.NAV_RADIUS` (the capsule radius a cell's centre is tested with): `Facility::DEFAULT_AGENT_RADIUS`.
pub const NAV_RADIUS: f32 = 0.23;

/// The collider a prop's footprint is read from: `collider_size`, else the blocker
/// (`prop.get("collider_size", prop.get("blocker_size", Vector3.ONE))`).
pub fn footprint(collider: Option<[f32; 3]>, blocker: [f32; 3]) -> [f32; 3] {
    collider.unwrap_or(blocker)
}

/// `StreamedHunterNavigation.blocks_nothing`: true for a solid prop the runner and the hunters walk over. The raised
/// collider rule (`collider_y0`) is not on the Rust prop, so it counts as zero.
pub fn walk_over(size: [f32; 3]) -> bool {
    size[1] <= WALK_ON_HEIGHT
}

/// The half extents of a prop's footprint on the plan, for a rotation in quarter turns: `Facility::can_occupy` reads one
/// axis-aligned box per prop, so x and z swap on an odd turn.
pub fn half_extents(size: [f32; 3], quarter_turns: i32) -> (f32, f32) {
    let [bx, _, bz] = size;
    if quarter_turns.rem_euclid(4) % 2 == 1 {
        (bz / 2.0, bx / 2.0)
    } else {
        (bx / 2.0, bz / 2.0)
    }
}

/// The one footprint rule for a body on a flight at absolute height `y` (`Facility::hunter_fits_at_y`): the brain's
/// edge walk and the gimmicks share it, so a hunter standing on a flight cell off an edge is legal exactly when it is
/// legal on one.
pub fn on_flight(fac: &crate::world::Facility, x: f32, z: f32, y: f32) -> bool {
    fac.hunter_fits_at_y(x, z, y, super::flight_edges::rules().edge_fit_radius)
}

// -- route following and stuck handling (data/hunters/nav_tuning.json) ---------------------------------------------------

/// The embedded tuning file; the content loader skips it as a hunter definition (hook in content/mod.rs).
pub const TUNING_PATH: &str = "hunters/nav_tuning.json";
/// The search-by-difficulty table (also skipped by the loader).
pub const SEARCH_DIFFICULTY_PATH: &str = "hunters/search_difficulty.json";

/// Pursuit and route following (hunter_brain.gd WAYPOINT_DISTANCE, PATH_LOOKAHEAD_NODES, SWEEP_STEP, ARRIVAL_DISTANCE).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pursuit {
    pub waypoint_distance: f32,
    pub path_lookahead_nodes: usize,
    pub sweep_step: f32,
    pub arrival_distance: f32,
}

/// Stuck handling (hunter_brain.gd `_track_progress`): no real progress for `center_after` seconds steers to the cell
/// centres for `center_steer_seconds`; still none after `repath_after` clears the path and snaps the body out of a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckRules {
    /// Moving less than this share of what was wanted counts as no progress.
    pub progress_ratio: f32,
    /// A wanted step no longer than this is not trying to move.
    pub min_wanted: f32,
    pub center_after: f32,
    pub repath_after: f32,
    pub center_steer_seconds: f32,
}

/// Patrol anchor (hunter_brain.gd PATROL_RADIUS and the doubled fallback).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatrolRules {
    pub radius: f32,
    pub fallback_radius_scale: f32,
    /// On a public route an idle hunter's anchor is a live runner.
    pub live_runner_anchor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavTuning {
    pub pursuit: Pursuit,
    pub stuck: StuckRules,
    pub patrol: PatrolRules,
}

impl NavTuning {
    /// The Godot constants.
    pub const GODOT: NavTuning = NavTuning {
        pursuit: Pursuit { waypoint_distance: 0.18, path_lookahead_nodes: 4, sweep_step: 0.15, arrival_distance: 0.65 },
        stuck: StuckRules { progress_ratio: 0.35, min_wanted: 0.0001, center_after: 0.25, repath_after: 0.7, center_steer_seconds: 0.8 },
        patrol: PatrolRules { radius: 30.0, fallback_radius_scale: 2.0, live_runner_anchor: true },
    };

    /// Parses the file; a missing or out-of-range value keeps the Godot one.
    pub fn parse(text: &str) -> NavTuning {
        let mut t = NavTuning::GODOT;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return t };
        let num = |o: &serde_json::Value, k: &str, lo: f64, hi: f64| {
            o.get(k).and_then(|x| x.as_f64()).filter(|x| *x >= lo && *x <= hi).map(|x| x as f32)
        };
        let p = &v["pursuit"];
        if let Some(x) = num(p, "waypoint_distance", 0.01, 5.0) {
            t.pursuit.waypoint_distance = x;
        }
        if let Some(x) = num(p, "path_lookahead_nodes", 1.0, 64.0) {
            t.pursuit.path_lookahead_nodes = x as usize;
        }
        if let Some(x) = num(p, "sweep_step", 0.01, 5.0) {
            t.pursuit.sweep_step = x;
        }
        if let Some(x) = num(p, "arrival_distance", 0.05, 10.0) {
            t.pursuit.arrival_distance = x;
        }
        let s = &v["stuck"];
        if let Some(x) = num(s, "progress_ratio", 0.0, 1.0) {
            t.stuck.progress_ratio = x;
        }
        if let Some(x) = num(s, "min_wanted", 0.0, 1.0) {
            t.stuck.min_wanted = x;
        }
        if let Some(x) = num(s, "center_after", 0.0, 30.0) {
            t.stuck.center_after = x;
        }
        if let Some(x) = num(s, "repath_after", 0.0, 60.0) {
            t.stuck.repath_after = x;
        }
        if let Some(x) = num(s, "center_steer_seconds", 0.0, 30.0) {
            t.stuck.center_steer_seconds = x;
        }
        let q = &v["patrol"];
        if let Some(x) = num(q, "radius", 1.0, 1000.0) {
            t.patrol.radius = x;
        }
        if let Some(x) = num(q, "fallback_radius_scale", 1.0, 100.0) {
            t.patrol.fallback_radius_scale = x;
        }
        if let Some(b) = q.get("live_runner_anchor").and_then(|x| x.as_bool()) {
            t.patrol.live_runner_anchor = b;
        }
        t
    }
}

/// The embedded tuning (the Godot values when the file is missing or broken).
pub fn tuning() -> &'static NavTuning {
    static T: std::sync::OnceLock<NavTuning> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        crate::content::embedded::data_under(TUNING_PATH)
            .next()
            .map(|(_, text)| NavTuning::parse(text))
            .unwrap_or(NavTuning::GODOT)
    })
}

/// Waypoint look-ahead (hunter_brain.gd `_pursue`): the furthest of the first `lookahead` path points the body can
/// reach in one clean sweep, walking the points in order and stopping at the first it cannot. `fits(i, p)` answers for
/// path point `i` at `p`. Returns the index and the point (the first point when none passes), None for an empty path.
pub fn look_ahead(
    path: &[(f32, f32)],
    lookahead: usize,
    mut fits: impl FnMut(usize, (f32, f32)) -> bool,
) -> Option<(usize, (f32, f32))> {
    let first = *path.first()?;
    let mut best = (0, first);
    for (i, &c) in path.iter().enumerate().take(lookahead.max(1)) {
        if !fits(i, c) {
            break;
        }
        best = (i, c);
    }
    Some(best)
}

/// How many path points to look ahead: one (the next cell centre, no corner cutting) while centre-steering.
pub fn lookahead_for(steering: bool, base: usize) -> usize {
    if steering {
        1
    } else {
        base.max(1)
    }
}

/// What the stuck tracker asks for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StuckAction {
    None,
    /// Steer to the cell centres of the path (no corner cutting) for the data time.
    CenterSteer,
    /// Clear the path, plan again and snap the body out of a wall if it sits in one.
    Repath,
}

/// The escalation of hunter_brain.gd `_track_progress`: centre-steer, then repath with a snap-back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StuckTracker {
    stuck_time: f32,
    steer_left: f32,
}

impl StuckTracker {
    /// True while the body is steering to cell centres.
    pub fn steering(&self) -> bool {
        self.steer_left > 0.0
    }

    pub fn stuck_time(&self) -> f32 {
        self.stuck_time
    }

    /// One frame: the body `moved` this far when it `wanted` to move `wanted`.
    pub fn track(&mut self, rules: &StuckRules, moved: f32, wanted: f32, dt: f32) -> StuckAction {
        self.steer_left = (self.steer_left - dt).max(0.0);
        if wanted > rules.min_wanted && moved < wanted * rules.progress_ratio {
            self.stuck_time += dt;
        } else {
            self.stuck_time = 0.0;
            return StuckAction::None;
        }
        let mut action = StuckAction::None;
        if self.stuck_time >= rules.center_after && self.steer_left <= 0.0 {
            self.steer_left = rules.center_steer_seconds;
            action = StuckAction::CenterSteer;
        }
        if self.stuck_time >= rules.repath_after {
            self.stuck_time = 0.0;
            action = StuckAction::Repath;
        }
        action
    }

    /// A goal change or a teleport: forget the progress.
    pub fn reset(&mut self) {
        *self = StuckTracker::default();
    }
}

/// The wedge snap-back: a body whose footprint does not fit where it stands moves to the nearest open spot; None when
/// it fits already or no open spot is near. `nearest` is `Facility::nearest_open` at the body's radius.
pub fn snap_back(
    pos: (f32, f32),
    fits: impl Fn((f32, f32)) -> bool,
    nearest: impl Fn((f32, f32)) -> Option<(f32, f32)>,
) -> Option<(f32, f32)> {
    if fits(pos) {
        None
    } else {
        nearest(pos)
    }
}

// -- searching by difficulty (data/hunters/search_difficulty.json) --------------------------------------------------------

/// One difficulty's search habits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchHabits {
    /// Chance a search step checks the hiding spot nearest the last sighting.
    pub locker_check_rate: f32,
    /// Chance a step heads for a room or dead end not yet searched this chase.
    pub unchecked_room_rate: f32,
    /// The first step goes to the last-seen spot.
    pub last_seen_first: bool,
}

/// What one search step does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStep {
    LastSeen,
    Locker,
    UncheckedRoom,
    Offset,
}

#[derive(Clone, Debug)]
pub struct SearchDifficulty {
    default: SearchHabits,
    rows: Vec<(String, SearchHabits)>,
}

impl SearchDifficulty {
    pub fn parse(text: &str) -> SearchDifficulty {
        let base = SearchHabits { locker_check_rate: 0.0, unchecked_room_rate: 0.0, last_seen_first: false };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
            return SearchDifficulty { default: base, rows: vec![] };
        };
        let read = |o: &serde_json::Value, b: SearchHabits| {
            let mut h = b;
            if let Some(x) = o["locker_check_rate"].as_f64() {
                h.locker_check_rate = (x as f32).clamp(0.0, 1.0);
            }
            if let Some(x) = o["unchecked_room_rate"].as_f64() {
                h.unchecked_room_rate = (x as f32).clamp(0.0, 1.0);
            }
            if let Some(x) = o["last_seen_first"].as_bool() {
                h.last_seen_first = x;
            }
            h
        };
        let default = read(&v["default"], base);
        let rows = v["difficulty"]
            .as_object()
            .map(|m| m.iter().map(|(k, o)| (k.clone(), read(o, default))).collect())
            .unwrap_or_default();
        SearchDifficulty { default, rows }
    }

    /// The habits for a difficulty id ("standard", "hard" ...); the default row for an unknown one.
    pub fn habits(&self, difficulty_id: &str) -> SearchHabits {
        self.rows.iter().find(|(k, _)| k == difficulty_id).map(|(_, h)| *h).unwrap_or(self.default)
    }

    /// The next step of a search. `first` is the first step of this search; `roll_a` decides a locker check and
    /// `roll_b` an unchecked room, both in 0..1. The last-seen spot comes first when the difficulty asks for it.
    pub fn next_step(&self, difficulty_id: &str, first: bool, roll_a: f32, roll_b: f32) -> SearchStep {
        let h = self.habits(difficulty_id);
        if first && h.last_seen_first {
            return SearchStep::LastSeen;
        }
        if roll_a < h.locker_check_rate {
            SearchStep::Locker
        } else if roll_b < h.unchecked_room_rate {
            SearchStep::UncheckedRoom
        } else {
            SearchStep::Offset
        }
    }
}

/// The embedded search-by-difficulty table.
pub fn search_difficulty() -> &'static SearchDifficulty {
    static S: std::sync::OnceLock<SearchDifficulty> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        crate::content::embedded::data_under(SEARCH_DIFFICULTY_PATH)
            .next()
            .map(|(_, t)| SearchDifficulty::parse(t))
            .unwrap_or_else(|| SearchDifficulty::parse("{}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_collider_is_the_footprint_when_there_is_one() {
        assert_eq!(footprint(Some([2.0, 1.0, 0.5]), [1.0, 1.0, 1.0]), [2.0, 1.0, 0.5]);
        assert_eq!(footprint(None, [1.0, 1.0, 1.0]), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_prop_no_taller_than_the_walk_on_height_is_floor() {
        assert!(walk_over([1.0, WALK_ON_HEIGHT, 1.0]));
        assert!(walk_over([1.0, 0.1, 1.0]));
        assert!(!walk_over([1.0, 0.26, 1.0]));
    }

    #[test]
    fn half_extents_swap_on_an_odd_turn() {
        assert_eq!(half_extents([2.0, 1.0, 4.0], 0), (1.0, 2.0));
        assert_eq!(half_extents([2.0, 1.0, 4.0], 1), (2.0, 1.0));
        assert_eq!(half_extents([2.0, 1.0, 4.0], 2), (1.0, 2.0));
        assert_eq!(half_extents([2.0, 1.0, 4.0], -1), (2.0, 1.0));
    }

    // -- route following, stuck handling and searches by difficulty ----------------------------------------------------

    #[test]
    fn the_tuning_file_is_the_godot_constants_and_a_changed_value_applies() {
        let t = NavTuning::parse(include_str!("../data/hunters/nav_tuning.json"));
        assert_eq!(t, NavTuning::GODOT);
        assert_eq!(*tuning(), NavTuning::GODOT);
        let c = NavTuning::parse(r#"{"stuck":{"center_after":0.5},"pursuit":{"path_lookahead_nodes":2},"patrol":{"radius":10}}"#);
        assert_eq!(c.stuck.center_after, 0.5);
        assert_eq!(c.pursuit.path_lookahead_nodes, 2);
        assert_eq!(c.patrol.radius, 10.0);
        assert_eq!(c.stuck.repath_after, NavTuning::GODOT.stuck.repath_after);
        assert_eq!(NavTuning::parse("not json"), NavTuning::GODOT);
    }

    #[test]
    fn the_pursuit_data_equals_the_walk_data() {
        let walk: serde_json::Value = serde_json::from_str(include_str!("../data/hunter_walk.json")).unwrap();
        let p = tuning().pursuit;
        assert_eq!(walk["waypoint_distance"].as_f64().unwrap() as f32, p.waypoint_distance);
        assert_eq!(walk["path_lookahead_nodes"].as_u64().unwrap() as usize, p.path_lookahead_nodes);
        assert_eq!(walk["sweep_step"].as_f64().unwrap() as f32, p.sweep_step);
    }

    #[test]
    fn look_ahead_takes_the_furthest_clean_point_and_stops_at_the_first_blocked() {
        let path = [(0.5, 0.5), (1.5, 0.5), (2.5, 0.5), (3.5, 0.5), (4.5, 0.5), (5.5, 0.5)];
        assert_eq!(look_ahead(&path, 4, |_, _| true), Some((3, (3.5, 0.5))));
        assert_eq!(look_ahead(&path, 4, |i, _| i != 2), Some((1, (1.5, 0.5))), "later clean points do not count");
        assert_eq!(look_ahead(&path, 4, |_, _| false), Some((0, (0.5, 0.5))), "the first point is the floor");
        assert_eq!(look_ahead(&path[..2], 4, |_, _| true), Some((1, (1.5, 0.5))));
        assert_eq!(look_ahead(&[], 4, |_, _| true), None);
        assert_eq!(look_ahead(&path, lookahead_for(true, 4), |_, _| true), Some((0, (0.5, 0.5))), "centre-steer takes one");
        assert_eq!(lookahead_for(false, 4), 4);
    }

    /// A body on an L-shaped route round a solid cell: the cell [1,2]x[1,2] with the body's radius round it is shut.
    fn wall_fits(p: (f32, f32)) -> bool {
        !(p.0 > 0.7 && p.0 < 2.2 && p.1 > 0.7 && p.1 < 2.3)
    }

    /// Walks the route (1.5,0.5) -> (2.5,0.5) -> (2.5,1.5) at 3 m/s in 60 fps frames and returns the seconds taken, or
    /// None when the body is still wedged after ten seconds. `centre_steer` switches the escalation on.
    fn walk_round_the_corner(centre_steer: bool) -> Option<f32> {
        let rules = tuning().stuck;
        let mut tracker = StuckTracker::default();
        let mut path = vec![(2.5f32, 0.5f32), (2.5, 1.5)];
        let mut pos = (1.69f32, 0.69f32); // wedged: the corner cut already carried it to the wall's edge
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        while t < 10.0 {
            t += dt;
            while path.len() > 1 && ((path[0].0 - pos.0).powi(2) + (path[0].1 - pos.1).powi(2)).sqrt() <= 0.18 {
                path.remove(0);
            }
            let steering = centre_steer && tracker.steering();
            let (_, wp) = look_ahead(&path, lookahead_for(steering, 2), |_, _| true).unwrap();
            let (dx, dy) = (wp.0 - pos.0, wp.1 - pos.1);
            let d = (dx * dx + dy * dy).sqrt();
            if d < 0.2 && path.len() == 1 {
                return Some(t);
            }
            let wanted = (3.0 * dt).min(d);
            let next = (pos.0 + dx / d * wanted, pos.1 + dy / d * wanted);
            let moved = if wall_fits(next) {
                pos = next;
                wanted
            } else {
                0.0
            };
            if centre_steer {
                tracker.track(&rules, moved, wanted, dt);
            }
        }
        None
    }

    #[test]
    fn centre_steer_frees_a_wedged_body() {
        assert_eq!(walk_round_the_corner(false), None, "without the escalation the cut keeps it wedged");
        let took = walk_round_the_corner(true).expect("centre-steer frees it");
        let r = tuning().stuck;
        assert!(took >= r.center_after, "not before the data time: {took}");
        assert!(took <= r.repath_after + 1.0, "freed within about a second: {took}");
    }

    #[test]
    fn the_tracker_escalates_to_centre_steer_then_repath_and_resets_on_progress() {
        let r = tuning().stuck;
        let mut t = StuckTracker::default();
        let dt = 0.05;
        let (mut steered, mut repathed) = (None, None);
        for frame in 0..40 {
            match t.track(&r, 0.0, 0.1, dt) {
                StuckAction::CenterSteer if steered.is_none() => steered = Some(frame as f32 * dt),
                StuckAction::Repath if repathed.is_none() => repathed = Some(frame as f32 * dt),
                _ => {}
            }
        }
        let (s, p) = (steered.expect("centre-steer"), repathed.expect("repath"));
        assert!(s >= r.center_after - dt && s < p, "centre-steer {s} before repath {p}");
        assert!(p >= r.repath_after - dt && p <= r.repath_after + 2.0 * dt, "repath {p}");
        // moving freely resets the clock, and a body that does not want to move is not stuck
        let mut t = StuckTracker::default();
        t.track(&r, 0.0, 0.1, 0.2);
        assert!(t.stuck_time() > 0.0);
        assert_eq!(t.track(&r, 0.1, 0.1, 0.2), StuckAction::None);
        assert_eq!(t.stuck_time(), 0.0);
        assert_eq!(t.track(&r, 0.0, 0.0, 5.0), StuckAction::None);
        // a changed threshold changes when it fires
        let slow = StuckRules { center_after: 2.0, ..r };
        let mut t = StuckTracker::default();
        assert_eq!(t.track(&slow, 0.0, 0.1, 0.5), StuckAction::None);
    }

    #[test]
    fn snap_back_moves_only_a_body_inside_a_wall() {
        let nearest = |_p: (f32, f32)| Some((0.5, 0.5));
        assert_eq!(snap_back((1.5, 1.5), wall_fits, nearest), Some((0.5, 0.5)));
        assert_eq!(snap_back((0.2, 0.2), wall_fits, nearest), None, "a body that fits stays put");
        assert_eq!(snap_back((1.5, 1.5), wall_fits, |_| None), None, "no open spot near: nothing to snap to");
    }

    #[test]
    fn searches_by_difficulty_order_their_steps() {
        let s = SearchDifficulty::parse(include_str!("../data/hunters/search_difficulty.json"));
        // Standard keeps the Godot behaviour: never a locker, never last-seen-first
        for roll in [0.0f32, 0.1, 0.5, 0.99] {
            assert_eq!(s.next_step("standard", true, roll, roll), SearchStep::Offset);
            assert_eq!(s.next_step("relaxed", false, roll, roll), SearchStep::Offset);
        }
        // Hard and Nightmare check the last-seen spot first
        assert_eq!(s.next_step("hard", true, 0.99, 0.99), SearchStep::LastSeen);
        assert_eq!(s.next_step("nightmare", true, 0.99, 0.99), SearchStep::LastSeen);
        assert_eq!(s.next_step("nightmare", false, 0.1, 0.9), SearchStep::Locker);
        assert_eq!(s.next_step("nightmare", false, 0.9, 0.5), SearchStep::UncheckedRoom);
        assert_eq!(s.next_step("nightmare", false, 0.9, 0.95), SearchStep::Offset);
        // the harder the difficulty, the more lockers get checked
        let lockers = |id: &str| (0..100).filter(|i| s.next_step(id, false, *i as f32 / 100.0, 0.99) == SearchStep::Locker).count();
        assert!(lockers("nightmare") > lockers("hard") && lockers("hard") > lockers("standard"));
        assert_eq!(lockers("standard"), 0);
        assert_eq!(s.habits("nonsense"), s.habits("standard"), "an unknown difficulty uses the default row");
    }

    #[test]
    fn a_changed_search_rate_changes_the_steps() {
        let s = SearchDifficulty::parse(r#"{"difficulty":{"standard":{"locker_check_rate":1.0}}}"#);
        assert_eq!(s.next_step("standard", false, 0.99, 0.99), SearchStep::Locker);
        assert_eq!(SearchDifficulty::parse("broken").next_step("hard", true, 0.0, 0.0), SearchStep::Offset);
    }
}
