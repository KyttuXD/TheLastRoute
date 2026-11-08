//! Idle life for wandering hunters: pauses and peeks at junctions, fidgets, pacing, glances in an Idle pause, and a
//! head turn toward a heard noise before the hunter goes to look. Host only: the brain runs these on the side that
//! simulates hunters; a guest just shows the activity the host sends (activity index and progress).
//!
//! Everything tunable is in data/hunters/activities_idle.json (and the weights in activities.json). This file is
//! rules and pure helpers; hunters/brain.rs calls them at the activity and idle hook points and never lets any of it
//! outlast a chase, an investigation, a search, a peek, a capture or a locker check (the brain ends the activity and
//! clears the turn as soon as its state leaves Wander and Idle).
//!
//! Clips: the renderer reads the activity kind and progress; no dedicated stretch, pace or corner-lean clip exists
//! yet, so these reuse the idle and walk poses with the facing changes below.

use super::activities::{dir_to, region_of, rotate, Activity, ActivityRun, Poi, PoiKind, Rules};
use crate::config::HUNTER_RADIUS;
use crate::world::Facility;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const PATH: &str = "hunters/activities_idle.json";

/// data/hunters/activities_idle.json.
#[derive(Clone, Debug)]
pub struct IdleRules {
    pub fidget_degrees: f32,
    pub fidgets: f32,
    pub pace_length: (f32, f32),
    pub pace_min_length: f32,
    pub pace_speed: f32,
    pub corner_peek_reach: f32,
    pub glance_every: (f32, f32),
    pub glance_degrees: f32,
    pub head_turn_chance: f32,
    pub head_turn_time: (f32, f32),
    pub junction_radius: f32,
    pub junction_min_open: usize,
    pub junction_cap: usize,
    /// personality -> activity -> weight multiplier.
    pub activity_weights: HashMap<String, HashMap<Activity, f32>>,
    /// personality -> multiplier on `head_turn_chance`.
    pub head_turn_scale: HashMap<String, f32>,
}

impl Default for IdleRules {
    fn default() -> Self {
        IdleRules {
            fidget_degrees: 28.0,
            fidgets: 2.5,
            pace_length: (1.4, 2.6),
            pace_min_length: 0.7,
            pace_speed: 0.9,
            corner_peek_reach: 1.0,
            glance_every: (1.4, 3.2),
            glance_degrees: 70.0,
            head_turn_chance: 0.85,
            head_turn_time: (0.45, 1.0),
            junction_radius: 22.0,
            junction_min_open: 3,
            junction_cap: 12,
            activity_weights: HashMap::new(),
            head_turn_scale: HashMap::new(),
        }
    }
}

fn pair(v: &serde_json::Value, d: (f32, f32)) -> (f32, f32) {
    let a = v.as_array();
    let lo = a.and_then(|a| a.first()).and_then(|x| x.as_f64()).map(|x| x as f32);
    let hi = a.and_then(|a| a.get(1)).and_then(|x| x.as_f64()).map(|x| x as f32);
    match (lo, hi) {
        (Some(l), Some(h)) if l >= 0.0 && h >= l => (l, h),
        _ => d,
    }
}

impl IdleRules {
    pub fn parse(v: &serde_json::Value) -> IdleRules {
        let d = IdleRules::default();
        let num = |k: &str, d: f32| v["params"][k].as_f64().map(|x| x as f32).filter(|x| x.is_finite()).unwrap_or(d);
        let mut activity_weights = HashMap::new();
        if let Some(o) = v["activity_weights"].as_object() {
            for (pers, m) in o {
                let mut map = HashMap::new();
                if let Some(mo) = m.as_object() {
                    for (n, w) in mo {
                        if let (Some(a), Some(w)) = (Activity::from_name(n), w.as_f64()) {
                            map.insert(a, w.max(0.0) as f32);
                        }
                    }
                }
                activity_weights.insert(pers.clone(), map);
            }
        }
        let head_turn_scale = v["head_turn_chance_scale"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?.max(0.0) as f32))).collect())
            .unwrap_or_default();
        IdleRules {
            fidget_degrees: num("fidget_degrees", d.fidget_degrees).clamp(0.0, 180.0),
            fidgets: num("fidgets", d.fidgets).max(0.0),
            pace_length: pair(&v["params"]["pace_length"], d.pace_length),
            pace_min_length: num("pace_min_length", d.pace_min_length).max(0.1),
            pace_speed: num("pace_speed", d.pace_speed).max(0.1),
            corner_peek_reach: num("corner_peek_reach", d.corner_peek_reach).max(0.5),
            glance_every: pair(&v["params"]["glance_every"], d.glance_every),
            glance_degrees: num("glance_degrees", d.glance_degrees).clamp(0.0, 180.0),
            head_turn_chance: num("head_turn_chance", d.head_turn_chance).clamp(0.0, 1.0),
            head_turn_time: pair(&v["params"]["head_turn_time"], d.head_turn_time),
            junction_radius: num("junction_radius", d.junction_radius).max(0.0),
            junction_min_open: v["params"]["junction_min_open"].as_u64().map_or(d.junction_min_open, |n| (n as usize).clamp(2, 4)),
            junction_cap: v["params"]["junction_cap"].as_u64().map_or(d.junction_cap, |n| n as usize),
            activity_weights,
            head_turn_scale,
        }
    }

    pub fn get() -> &'static IdleRules {
        static RULES: OnceLock<IdleRules> = OnceLock::new();
        RULES.get_or_init(|| {
            crate::content::embedded::data_under(PATH)
                .next()
                .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
                .map(|v| IdleRules::parse(&v))
                .unwrap_or_default()
        })
    }

    /// The weight multiplier a personality gives an activity (1.0 when the data does not say).
    pub fn activity_scale(&self, personality: &str, a: Activity) -> f32 {
        self.activity_weights.get(personality).and_then(|m| m.get(&a)).copied().unwrap_or(1.0)
    }

    /// The chance this personality turns its head to a noise before investigating.
    pub fn head_turn_odds(&self, personality: &str) -> f32 {
        (self.head_turn_chance * self.head_turn_scale.get(personality).copied().unwrap_or(1.0)).clamp(0.0, 1.0)
    }
}

/// The activity done at `poi`, with the hunter's personality scaling each activity's weight.
pub fn pick_activity(rules: &Rules, idle: &IdleRules, poi: &Poi, door_shut: bool, personality: &str, roll: f64) -> Activity {
    rules.pick_activity_scaled(poi, door_shut, roll, |a| idle.activity_scale(personality, a))
}

const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

fn open_at(fac: &Facility, p: (f32, f32)) -> bool {
    fac.body_fits(p.0, p.1, HUNTER_RADIUS)
}

/// Whether the whole pace line from `from` to `to` is ground the runner can stand on too (sampled every 0.1 m), so a
/// pacing hunter never creeps under a pipe or against a wall the runner could not reach.
fn pace_line_clear(fac: &Facility, from: (f32, f32), to: (f32, f32)) -> bool {
    let d = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
    let steps = ((d / 0.1).ceil() as i32).max(1);
    (0..=steps).all(|i| {
        let t = i as f32 / steps as f32;
        crate::game::player::body_fits(fac, from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, 0.0)
    })
}

/// Junction goals within `junction_radius` of `from`: corridor cells with at least `junction_min_open` open sides,
/// nearest first, at most `junction_cap`. Run once per goal pick (never per frame). Only when the rules list the kind.
pub fn junction_pois(fac: &Facility, rules: &Rules, idle: &IdleRules, from: (f32, f32)) -> Vec<Poi> {
    if !rules.kinds.contains_key(&PoiKind::Junction) || idle.junction_cap == 0 {
        return vec![];
    }
    let r = idle.junction_radius;
    let (x0, x1) = (((from.0 - r).floor() as i32).max(1), ((from.0 + r).ceil() as i32).min(fac.cols - 2));
    let (y0, y1) = (((from.1 - r).floor() as i32).max(1), ((from.1 + r).ceil() as i32).min(fac.rows - 2));
    let mut found: Vec<(f32, Poi)> = Vec::new();
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            let c = (cx as f32 + 0.5, cy as f32 + 0.5);
            let d = ((c.0 - from.0).powi(2) + (c.1 - from.1).powi(2)).sqrt();
            if d > r || !open_at(fac, c) {
                continue;
            }
            let open = DIRS.iter().filter(|(dx, dy)| open_at(fac, (c.0 + *dx as f32, c.1 + *dy as f32))).count();
            if open < idle.junction_min_open {
                continue;
            }
            let room = fac.room_at(c.0 as f64, c.1 as f64);
            found.push((
                d,
                Poi {
                    kind: PoiKind::Junction,
                    stand: c,
                    focus: c,
                    room,
                    rtype: room.and_then(|i| fac.rooms.get(i)).map(|r| r.rtype.clone()).unwrap_or_default(),
                    region: region_of(fac, c, room),
                    locker: None,
                    door: None,
                },
            ));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.truncate(idle.junction_cap);
    found.into_iter().map(|(_, p)| p).collect()
}

/// Prepare a run of an idle-life activity at `pos` facing `run.base`. `roll` is 0..1. Pace stores its two ends in
/// `loop_points` (both open spots; empty when there is no room to pace, which then fidgets); a corner peek stores the
/// unit directions of the open sides, starting from a rolled side.
pub fn setup(run: &mut ActivityRun, fac: &Facility, pos: (f32, f32), idle: &IdleRules, roll: f64) {
    run.loop_points.clear();
    run.leg = 0;
    match run.kind {
        Activity::Pace => {
            let roll = roll.clamp(0.0, 1.0) as f32;
            let mut len = idle.pace_length.0 + (idle.pace_length.1 - idle.pace_length.0) * roll;
            while len >= idle.pace_min_length {
                let end = (pos.0 + run.base.0 * len, pos.1 + run.base.1 * len);
                let mid = (pos.0 + run.base.0 * len * 0.5, pos.1 + run.base.1 * len * 0.5);
                if open_at(fac, end) && open_at(fac, mid) && pace_line_clear(fac, pos, end) {
                    run.loop_points = vec![pos, end];
                    return;
                }
                len *= 0.7;
            }
        }
        Activity::CornerPeek => {
            let start = ((roll.clamp(0.0, 0.999) * 4.0) as usize).min(3);
            for i in 0..4 {
                let (dx, dy) = DIRS[(i + start) % 4];
                let d = (dx as f32, dy as f32);
                let reach = idle.corner_peek_reach;
                if open_at(fac, (pos.0 + d.0 * reach, pos.1 + d.1 * reach)) {
                    run.loop_points.push(d);
                }
            }
        }
        _ => {}
    }
}

/// What an idle-life activity asks of the brain this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    /// Set the facing to this.
    pub facing: Option<(f32, f32)>,
    /// Creep toward this spot at this speed (m/s); the brain moves with its own collision, so the hunter never
    /// leaves ground it can stand on.
    pub step_to: Option<((f32, f32), f32)>,
}

/// One frame of Fidget, Pace or CornerPeek at progress `p` (0..1).
pub fn tick(run: &mut ActivityRun, p: f32, pos: (f32, f32), idle: &IdleRules) -> Step {
    match run.kind {
        Activity::Pace if run.loop_points.len() == 2 => {
            let goal = run.loop_points[run.leg % 2];
            if ((goal.0 - pos.0).powi(2) + (goal.1 - pos.1).powi(2)).sqrt() < 0.12 {
                run.leg += 1;
            }
            let goal = run.loop_points[run.leg % 2];
            Step { facing: Some(dir_to(pos, goal, run.base)), step_to: Some((goal, idle.pace_speed)) }
        }
        Activity::CornerPeek if !run.loop_points.is_empty() => {
            let n = run.loop_points.len();
            let i = ((p * n as f32) as usize).min(n - 1);
            Step { facing: Some(run.loop_points[i]), step_to: None }
        }
        _ => {
            let a = (p * idle.fidgets * std::f32::consts::TAU).sin() * idle.fidget_degrees.to_radians() / 2.0;
            Step { facing: Some(rotate(run.base, a)), step_to: None }
        }
    }
}

/// Idle-pause and noise-reaction state kept by the brain.
#[derive(Clone, Debug, Default)]
pub struct IdleState {
    /// Seconds of head turn left before the hunter goes to look at a noise.
    turn: Option<f32>,
    glance_in: f32,
}

impl IdleState {
    pub fn turning(&self) -> bool {
        self.turn.is_some()
    }

    /// A noise was heard: maybe start a head turn. Returns whether one started (the brain then holds the hunter still
    /// and calls `tick_turn`). `roll_chance` and `roll_time` are 0..1.
    pub fn begin_turn(&mut self, idle: &IdleRules, personality: &str, roll_chance: f64, roll_time: f64) -> bool {
        if self.turn.is_some() || roll_chance as f32 >= idle.head_turn_odds(personality) {
            return false;
        }
        let (lo, hi) = idle.head_turn_time;
        self.turn = Some(lo + (hi - lo) * roll_time.clamp(0.0, 1.0) as f32);
        true
    }

    /// Count the turn down; true on the frame it finishes (the brain then investigates).
    pub fn tick_turn(&mut self, dt: f32) -> bool {
        match self.turn {
            Some(left) if left - dt > 0.0 => {
                self.turn = Some(left - dt);
                false
            }
            Some(_) => {
                self.turn = None;
                true
            }
            None => false,
        }
    }

    /// The brain left its Idle pause (a chase, investigation, search, peek, or a wander set by the director or a gimmick): drop the turn at once, so a stale turn never makes it deaf to the next noise.
    pub fn cancel(&mut self) {
        self.turn = None;
    }

    /// While standing in an Idle pause: now and then glance to a side. Returns the new facing on those frames.
    /// One float decrement per frame; the rolls are only drawn when a glance fires.
    pub fn glance(&mut self, dt: f32, idle: &IdleRules, facing: (f32, f32), mut roll: impl FnMut() -> f64) -> Option<(f32, f32)> {
        self.glance_in -= dt;
        if self.glance_in > 0.0 {
            return None;
        }
        let (lo, hi) = idle.glance_every;
        self.glance_in = lo + (hi - lo) * roll().clamp(0.0, 1.0) as f32;
        let a = (roll().clamp(0.0, 1.0) as f32 * 2.0 - 1.0) * idle.glance_degrees.to_radians();
        Some(rotate(facing, a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;
    use crate::world::rng::Rng;

    fn fac() -> Facility {
        Facility::generate("IDLE-1", &Options::default(), &Catalog::load())
    }

    fn poi(kind: PoiKind) -> Poi {
        Poi {
            kind,
            stand: (5.5, 5.5),
            focus: (5.5, 5.5),
            room: None,
            rtype: String::new(),
            region: "maze".into(),
            locker: None,
            door: None,
        }
    }

    fn run(kind: Activity, base: (f32, f32)) -> ActivityRun {
        ActivityRun {
            kind,
            duration: 3.0,
            elapsed: 0.0,
            base,
            focus: (0.0, 0.0),
            locker: None,
            door: None,
            fired: false,
            leg: 0,
            loop_points: vec![],
            hearing_applied: 1.0,
        }
    }

    #[test]
    fn the_embedded_idle_data_loads_and_every_kind_and_activity_is_listed() {
        let i = IdleRules::get();
        assert!(i.pace_speed > 0.0 && i.glance_every.0 > 0.0 && i.head_turn_time.1 >= i.head_turn_time.0);
        assert!(i.head_turn_chance > 0.0 && i.head_turn_chance <= 1.0);
        let r = Rules::get();
        assert!(r.kinds.contains_key(&PoiKind::Junction));
        for a in [Activity::Fidget, Activity::Pace, Activity::CornerPeek] {
            assert!(r.durations.contains_key(&a), "{}", a.name());
            assert!(r.kinds.values().any(|k| k.activities.iter().any(|(x, w)| *x == a && *w > 0.0)), "{}", a.name());
        }
        for name in ["stalker", "lingerer"] {
            assert!(r.personalities.contains_key(name) && i.activity_weights.contains_key(name), "{name}");
        }
        assert_eq!(r.personality_of("ferryn", 1), "stalker");
        assert_eq!(r.personality_of("porcelle", 1), "lingerer");
    }

    #[test]
    fn an_idle_hunter_picks_varied_activities_over_time() {
        let rules = Rules::get();
        let idle = IdleRules::get();
        let p = poi(PoiKind::RoomCentre);
        let mut rng = Rng::new("VARIETY");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..600 {
            seen.insert(pick_activity(rules, idle, &p, false, "steady", rng.next_float()));
        }
        assert!(seen.len() >= 5, "only {seen:?}");
        assert!(seen.contains(&Activity::Fidget) && seen.contains(&Activity::Pace));
    }

    #[test]
    fn personality_weights_from_data_change_the_activity_mix() {
        let rules = Rules::get();
        let idle = IdleRules::get();
        let p = poi(PoiKind::RoomCentre);
        let share = |pers: &str, a: Activity| {
            let mut rng = Rng::new("MIX");
            (0..3000).filter(|_| pick_activity(rules, idle, &p, false, pers, rng.next_float()) == a).count() as f32 / 3000.0
        };
        assert!(share("lingerer", Activity::Fidget) > share("stalker", Activity::Fidget) * 2.0);
        assert!(share("stalker", Activity::CornerPeek) > share("lingerer", Activity::CornerPeek) * 1.5);
        // a changed data value changes the pick
        let mut changed = idle.clone();
        changed.activity_weights.insert("x".into(), HashMap::from([(Activity::Fidget, 0.0)]));
        let mut rng = Rng::new("MIX");
        for _ in 0..500 {
            assert_ne!(pick_activity(rules, &changed, &p, false, "x", rng.next_float()), Activity::Fidget);
        }
        let parsed = IdleRules::parse(&serde_json::json!({
            "params": { "pace_speed": 2.0, "head_turn_chance": 0.5 },
            "activity_weights": { "q": { "PACE": 3 } },
            "head_turn_chance_scale": { "q": 0.5 }
        }));
        assert_eq!(parsed.pace_speed, 2.0);
        assert_eq!(parsed.activity_scale("q", Activity::Pace), 3.0);
        assert_eq!(parsed.activity_scale("q", Activity::Sniff), 1.0);
        assert!((parsed.head_turn_odds("q") - 0.25).abs() < 1e-6);
    }

    #[test]
    fn junctions_are_open_stands_with_enough_open_sides_near_the_hunter() {
        let f = fac();
        let (rules, idle) = (Rules::get(), IdleRules::get());
        let from = f.rooms.iter().find(|r| !r.lobby()).map(|r| r.center.center()).unwrap();
        let js = junction_pois(&f, rules, idle, from);
        assert!(js.len() <= idle.junction_cap);
        for j in &js {
            assert_eq!(j.kind, PoiKind::Junction);
            assert!(f.body_fits(j.stand.0, j.stand.1, HUNTER_RADIUS));
            let d = ((j.stand.0 - from.0).powi(2) + (j.stand.1 - from.1).powi(2)).sqrt();
            assert!(d <= idle.junction_radius + 0.01);
        }
        assert!(junction_pois(&f, &Rules::default(), idle, from).is_empty());
    }

    #[test]
    fn pace_and_corner_peek_only_use_ground_the_body_fits() {
        let f = fac();
        let idle = IdleRules::get();
        let pois = crate::hunters::activities::points_of_interest(&f, Rules::get());
        let mut tried = 0;
        for p in pois.iter().take(40) {
            for (i, base) in DIRS.iter().enumerate() {
                let b = (base.0 as f32, base.1 as f32);
                let mut r = run(Activity::Pace, b);
                setup(&mut r, &f, p.stand, idle, i as f64 / 4.0);
                for pt in &r.loop_points {
                    assert!(f.body_fits(pt.0, pt.1, HUNTER_RADIUS));
                }
                let mut c = run(Activity::CornerPeek, b);
                setup(&mut c, &f, p.stand, idle, i as f64 / 4.0);
                for d in &c.loop_points {
                    let (x, y) = (p.stand.0 + d.0 * idle.corner_peek_reach, p.stand.1 + d.1 * idle.corner_peek_reach);
                    assert!(f.body_fits(x, y, HUNTER_RADIUS));
                }
                tried += 1;
            }
        }
        assert!(tried > 0);
    }

    #[test]
    fn ticks_turn_the_hunter_within_the_data_ranges_and_pace_goes_back_and_forth() {
        let idle = IdleRules::get();
        let mut r = run(Activity::Fidget, (1.0, 0.0));
        for i in 0..=50 {
            let s = tick(&mut r, i as f32 / 50.0, (0.0, 0.0), idle);
            let f = s.facing.unwrap();
            assert!(f.0 >= (idle.fidget_degrees / 2.0).to_radians().cos() - 1e-3);
            assert!(s.step_to.is_none());
        }
        let mut r = run(Activity::Pace, (1.0, 0.0));
        r.loop_points = vec![(0.0, 0.0), (2.0, 0.0)];
        // leg 0 is the start spot, which it is standing on, so it moves on to the far end
        let s = tick(&mut r, 0.1, (0.0, 0.0), idle);
        assert_eq!(s.step_to.unwrap().0, (2.0, 0.0));
        let _ = tick(&mut r, 0.2, (2.0, 0.0), idle);
        assert_eq!(r.leg, 2, "reached the far end after the start: back toward the start");
        assert_eq!(tick(&mut r, 0.3, (2.0, 0.0), idle).step_to.unwrap().0, (0.0, 0.0));
        // a pace with no room falls back to a fidget, a corner peek walks through its sides
        let mut np = run(Activity::Pace, (1.0, 0.0));
        assert!(tick(&mut np, 0.5, (0.0, 0.0), idle).step_to.is_none());
        let mut cp = run(Activity::CornerPeek, (1.0, 0.0));
        cp.loop_points = vec![(0.0, -1.0), (1.0, 0.0)];
        assert_eq!(tick(&mut cp, 0.0, (0.0, 0.0), idle).facing, Some((0.0, -1.0)));
        assert_eq!(tick(&mut cp, 0.99, (0.0, 0.0), idle).facing, Some((1.0, 0.0)));
    }

    #[test]
    fn a_head_turn_runs_its_data_time_and_a_cancel_drops_it_at_once() {
        let idle = IdleRules::get();
        let mut s = IdleState::default();
        assert!(!s.begin_turn(idle, "steady", 0.999_999, 0.0), "a high roll skips the turn");
        assert!(s.begin_turn(idle, "steady", 0.0, 0.0));
        assert!(s.turning());
        assert!(!s.begin_turn(idle, "steady", 0.0, 0.0), "not twice");
        let mut t = 0.0;
        while !s.tick_turn(0.05) {
            t += 0.05;
            assert!(t < idle.head_turn_time.1 + 0.2);
        }
        assert!(t >= idle.head_turn_time.0 - 0.1 && !s.turning());
        assert!(s.begin_turn(idle, "steady", 0.0, 1.0));
        s.cancel();
        assert!(!s.turning() && !s.tick_turn(10.0));
    }

    #[test]
    fn glances_fire_on_the_data_clock_and_stay_within_the_glance_angle() {
        let idle = IdleRules::get();
        let mut s = IdleState::default();
        let mut rng = Rng::new("GLANCE");
        let (mut n, mut t) = (0, 0.0);
        while t < 30.0 {
            if let Some(f) = s.glance(0.016, idle, (1.0, 0.0), || rng.next_float()) {
                n += 1;
                assert!(f.0 >= idle.glance_degrees.to_radians().cos() - 1e-3);
            }
            t += 0.016;
        }
        let max = 30.0 / idle.glance_every.0;
        let min = 30.0 / (idle.glance_every.1 + 0.1);
        assert!((n as f32) <= max + 2.0 && (n as f32) >= min - 1.0, "{n} glances");
    }
}
