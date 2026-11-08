//! Locker inspection with the hider's breath (Godot game/hunters/ai/hunter_inspect.gd and the INSPECT state of
//! hunter_brain.gd 1366-1444), and the hiding capacity lookup.
//!
//! A hunter passing a slatted spot with a runner inside who did NOT see them hide sometimes stops, turns to the slats,
//! leans in and lingers while the runner's breath meter fills, then moves on. The meter fills during lean and linger and
//! drains the rest of the time; it finds the runner only when the data says a full meter does (`reveal_when_full`, off in
//! Standard: she moves on without finding them). A runner who was seen hiding gets the ordinary hide check instead.
//! There is no text and no toast in any of it: the phases are the whole output.
//!
//! Host-only: the side that simulates hunters runs `Inspect::update`; a guest sees the pose through the brain state.
//! Every number is in data/hiding.json (`inspect`, `furniture_capacity`, `default_capacity`).
//!
//! Hooks (brain.rs): a `State::Inspect` that holds an `Inspect`, `Inspect::start` from the pass check, `update` each
//! frame (walk to `stand` while the phase is `Walk`), the facing from `face`, and a reveal handled like `locker_found`.

use crate::config::Difficulty;

pub type Pos = (f32, f32);

/// Embedded data/hiding.json.
const HIDING: &str = include_str!("../data/hiding.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Walking up to the stand point.
    Walk,
    /// Turning to the slats.
    Turn,
    /// Bending in.
    Lean,
    /// Lingering at the slats, breathing.
    Linger,
    /// Straightening up.
    Rise,
}

/// One difficulty's inspection numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DifficultyRow {
    pub chance: f32,
    pub linger: f32,
    pub cooldown: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectRules {
    pub reach: f32,
    pub stand_out: f32,
    pub stand_min: f32,
    pub turn_seconds: f32,
    pub lean_seconds: f32,
    pub rise_seconds: f32,
    pub walk_scale: f32,
    pub walk_limit: f32,
    pub spot_cooldown: f32,
    pub leave_distance: f32,
    pub walk_arrive: f32,
    pub breath_fill_per_second: f32,
    pub breath_drain_per_second: f32,
    pub reveal_when_full: bool,
    default_chance: f32,
    default_stand: f32,
    default_linger_scale: f32,
    /// (hunter id, chance, stand, linger scale)
    per_hunter: Vec<(String, f32, f32, f32)>,
    excluded: Vec<String>,
    /// (difficulty id, row)
    rows: Vec<(String, DifficultyRow)>,
}

impl InspectRules {
    /// The Godot numbers.
    pub fn godot() -> InspectRules {
        InspectRules {
            reach: 3.0,
            stand_out: 1.8,
            stand_min: 1.6,
            turn_seconds: 0.6,
            lean_seconds: 1.6,
            rise_seconds: 1.2,
            walk_scale: 0.6,
            walk_limit: 8.0,
            spot_cooldown: 25.0,
            leave_distance: 4.0,
            walk_arrive: 0.05,
            breath_fill_per_second: 0.27,
            breath_drain_per_second: 0.8,
            reveal_when_full: false,
            default_chance: 0.3,
            default_stand: 1.8,
            default_linger_scale: 1.0,
            per_hunter: vec![("nyxie".to_string(), 0.12, 1.7, 2.0)],
            excluded: ["wynelle", "void_gray", "ilyra", "zellia", "verrine", "ferryn", "javelle"].iter().map(|s| s.to_string()).collect(),
            rows: vec![
                ("standard".to_string(), DifficultyRow { chance: 1.0, linger: 1.5, cooldown: 40.0 }),
                ("hard".to_string(), DifficultyRow { chance: 1.15, linger: 2.0, cooldown: 30.0 }),
                ("nightmare".to_string(), DifficultyRow { chance: 1.3, linger: 2.5, cooldown: 22.0 }),
            ],
        }
    }

    /// Parses data/hiding.json's `inspect` object; a missing value keeps the Godot one.
    pub fn parse(text: &str) -> InspectRules {
        let mut r = InspectRules::godot();
        let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else { return r };
        let v = &root["inspect"];
        let f = |k: &str, into: &mut f32| {
            if let Some(x) = v[k].as_f64() {
                *into = x as f32;
            }
        };
        f("reach", &mut r.reach);
        f("stand_out", &mut r.stand_out);
        f("stand_min", &mut r.stand_min);
        f("turn_seconds", &mut r.turn_seconds);
        f("lean_seconds", &mut r.lean_seconds);
        f("rise_seconds", &mut r.rise_seconds);
        f("walk_scale", &mut r.walk_scale);
        f("walk_limit", &mut r.walk_limit);
        f("spot_cooldown", &mut r.spot_cooldown);
        f("leave_distance", &mut r.leave_distance);
        f("walk_arrive", &mut r.walk_arrive);
        f("breath_fill_per_second", &mut r.breath_fill_per_second);
        f("breath_drain_per_second", &mut r.breath_drain_per_second);
        if let Some(b) = v["reveal_when_full"].as_bool() {
            r.reveal_when_full = b;
        }
        if let Some(x) = v["default"]["chance"].as_f64() {
            r.default_chance = x as f32;
        }
        if let Some(x) = v["default"]["stand"].as_f64() {
            r.default_stand = x as f32;
        }
        if let Some(x) = v["default"]["linger_scale"].as_f64() {
            r.default_linger_scale = x as f32;
        }
        if let Some(m) = v["per_hunter"].as_object() {
            r.per_hunter = m
                .iter()
                .map(|(id, o)| {
                    (
                        id.clone(),
                        o["chance"].as_f64().map_or(r.default_chance, |x| x as f32),
                        o["stand"].as_f64().map_or(r.default_stand, |x| x as f32),
                        o["linger_scale"].as_f64().map_or(r.default_linger_scale, |x| x as f32),
                    )
                })
                .collect();
        }
        if let Some(a) = v["excluded"].as_array() {
            r.excluded = a.iter().filter_map(|s| s.as_str().map(String::from)).collect();
        }
        if let Some(m) = v["difficulty"].as_object() {
            r.rows = m
                .iter()
                .map(|(id, o)| {
                    let base = DifficultyRow { chance: 1.0, linger: 1.5, cooldown: 40.0 };
                    (
                        id.clone(),
                        DifficultyRow {
                            chance: o["chance"].as_f64().map_or(base.chance, |x| x as f32),
                            linger: o["linger"].as_f64().map_or(base.linger, |x| x as f32),
                            cooldown: o["cooldown"].as_f64().map_or(base.cooldown, |x| x as f32),
                        },
                    )
                })
                .collect();
        }
        r
    }

    /// The difficulty's row; Standard's (or the plain default) for an id with no row.
    pub fn row(&self, difficulty_id: &str) -> DifficultyRow {
        self.rows
            .iter()
            .find(|(k, _)| k == difficulty_id)
            .or_else(|| self.rows.iter().find(|(k, _)| k == "standard"))
            .map(|(_, r)| *r)
            .unwrap_or(DifficultyRow { chance: 1.0, linger: 1.5, cooldown: 40.0 })
    }

    fn hunter(&self, hunter_id: &str) -> (f32, f32, f32) {
        self.per_hunter
            .iter()
            .find(|(id, ..)| id == hunter_id)
            .map(|(_, c, s, l)| (*c, *s, *l))
            .unwrap_or((self.default_chance, self.default_stand, self.default_linger_scale))
    }

    /// Chance per qualifying pass; 0 for a hunter whose own rules drive it.
    pub fn chance(&self, hunter_id: &str, difficulty_id: &str) -> f32 {
        if self.excluded.iter().any(|e| e == hunter_id) {
            return 0.0;
        }
        (self.hunter(hunter_id).0 * self.row(difficulty_id).chance).clamp(0.0, 1.0)
    }

    /// Seconds at the slats.
    pub fn linger(&self, hunter_id: &str, difficulty_id: &str) -> f32 {
        self.row(difficulty_id).linger * self.hunter(hunter_id).2
    }

    /// Seconds before this hunter inspects anything again.
    pub fn cooldown(&self, difficulty_id: &str) -> f32 {
        self.row(difficulty_id).cooldown
    }

    /// Whether this pass qualifies (the dice are the caller's): near the spot, off cooldown, and the runner was not seen
    /// going in.
    pub fn qualifies(&self, distance_to_spot: f32, saw_hide: bool, own_cooldown: f32, spot_cooldown: f32) -> bool {
        !saw_hide && distance_to_spot <= self.reach && own_cooldown <= 0.0 && spot_cooldown <= 0.0
    }

    /// Where she stands: out through the spot's door at her stand distance, nearer (down to `stand_min`) in a tighter
    /// hall; None when no stand clears her body (she skips the spot). `fits` is the body check at a point.
    pub fn stand_point(&self, hunter_id: &str, spot: Pos, door: Pos, fits: impl Fn(Pos) -> bool) -> Option<Pos> {
        let len = (door.0 * door.0 + door.1 * door.1).sqrt();
        if len < 1e-6 {
            return None;
        }
        let (dx, dy) = (door.0 / len, door.1 / len);
        let mut out = self.hunter(hunter_id).1;
        while out >= self.stand_min - 0.001 {
            let at = (spot.0 + dx * out, spot.1 + dy * out);
            if fits(at) {
                return Some(at);
            }
            out -= 0.1;
        }
        None
    }
}

/// The embedded rules (the Godot numbers when the file is broken).
pub fn rules() -> &'static InspectRules {
    static R: std::sync::OnceLock<InspectRules> = std::sync::OnceLock::new();
    R.get_or_init(|| InspectRules::parse(HIDING))
}

/// What one frame of an inspection did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Continue,
    /// The meter filled and the data says that finds the runner.
    Reveal,
    /// The inspection is over; the hunter walks on.
    Done,
}

/// One inspection in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspect {
    pub phase: Phase,
    pub timer: f32,
    pub stand: Pos,
    /// The direction she faces at the slats (from the stand toward the spot).
    pub face: Pos,
    /// The hider's breath, 0..1.
    pub breath: f32,
    linger: f32,
}

impl Inspect {
    pub fn start(rules: &InspectRules, hunter_id: &str, difficulty_id: &str, stand: Pos, face: Pos) -> Inspect {
        Inspect { phase: Phase::Walk, timer: 0.0, stand, face, breath: 0.0, linger: rules.linger(hunter_id, difficulty_id) }
    }

    /// True while she is bent in at the slats (the lean pose shows, the breath plays).
    pub fn leaning(&self) -> bool {
        matches!(self.phase, Phase::Lean | Phase::Linger)
    }

    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.timer = 0.0;
    }

    /// One frame. `distance_to_stand` is how far she still is from the stand point while walking. The breath fills in the
    /// lean and the linger and drains otherwise; `Reveal` only when `reveal_when_full` is on and the meter is full.
    pub fn update(&mut self, rules: &InspectRules, dt: f32, distance_to_stand: f32) -> Step {
        self.timer += dt;
        if self.leaning() {
            self.breath = (self.breath + rules.breath_fill_per_second * dt).min(1.0);
        } else {
            self.breath = (self.breath - rules.breath_drain_per_second * dt).max(0.0);
        }
        if rules.reveal_when_full && self.breath >= 1.0 {
            return Step::Reveal;
        }
        match self.phase {
            Phase::Walk => {
                if distance_to_stand <= rules.walk_arrive || self.timer >= rules.walk_limit {
                    self.next(Phase::Turn);
                }
            }
            Phase::Turn => {
                if self.timer >= rules.turn_seconds {
                    self.next(Phase::Lean);
                }
            }
            Phase::Lean => {
                if self.timer >= rules.lean_seconds {
                    self.next(Phase::Linger);
                }
            }
            Phase::Linger => {
                if self.timer >= self.linger {
                    self.next(Phase::Rise);
                }
            }
            Phase::Rise => {
                if self.timer >= rules.rise_seconds {
                    return Step::Done;
                }
            }
        }
        Step::Continue
    }
}

// -- hiding capacity (data/hiding.json) ------------------------------------------------------------------------------------

/// Bodies a hiding spot of `kind` holds: the kind's own `capacity` in `kinds`, else `furniture_capacity`, else
/// `default_capacity`. An empty kind is a locker. Always at least one.
pub fn capacity_in(text: &str, kind: &str) -> usize {
    let kind = if kind.is_empty() { "locker" } else { kind };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return 1 };
    v["kinds"][kind]["capacity"]
        .as_u64()
        .or_else(|| v["furniture_capacity"][kind].as_u64())
        .or_else(|| v["default_capacity"].as_u64())
        .unwrap_or(1)
        .max(1) as usize
}

/// [`capacity_in`] over the embedded data.
pub fn capacity_of(kind: &str) -> usize {
    capacity_in(HIDING, kind)
}

/// The lowest free slot of a spot (0 the front, 1 beside) given the slots already held; None when it is full.
pub fn free_slot(capacity: usize, held: &[usize]) -> Option<usize> {
    (0..capacity.max(1)).find(|k| !held.contains(k))
}

/// The difficulty id the inspection tables are keyed by.
pub fn difficulty_key(d: Difficulty) -> &'static str {
    d.id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_to_end(r: &InspectRules, hunter: &str, diff: &str) -> (Inspect, Vec<Phase>, Step, f32) {
        let mut i = Inspect::start(r, hunter, diff, (0.0, 0.0), (0.0, 1.0));
        let mut seen = vec![i.phase];
        let mut peak = 0.0f32;
        let dt = 0.05;
        for _ in 0..2000 {
            let s = i.update(r, dt, 0.0);
            peak = peak.max(i.breath);
            if seen.last() != Some(&i.phase) {
                seen.push(i.phase);
            }
            if s != Step::Continue {
                return (i, seen, s, peak);
            }
        }
        (i, seen, Step::Continue, peak)
    }

    #[test]
    fn the_data_is_the_godot_inspection() {
        let r = InspectRules::parse(HIDING);
        let g = InspectRules::godot();
        for id in ["standard", "hard", "nightmare"] {
            assert_eq!(r.row(id), g.row(id), "{id}");
        }
        assert_eq!((r.reach, r.stand_out, r.stand_min, r.walk_limit), (g.reach, g.stand_out, g.stand_min, g.walk_limit));
        assert_eq!((r.breath_fill_per_second, r.reveal_when_full), (g.breath_fill_per_second, g.reveal_when_full));
        assert_eq!(rules().reach, g.reach);
        assert_eq!(r.chance("aki", "standard"), 0.3);
        assert!((r.chance("nyxie", "standard") - 0.12).abs() < 1e-6);
        assert_eq!(r.chance("javelle", "nightmare"), 0.0, "a hunter whose rules drive it never inspects");
        assert_eq!(r.linger("nyxie", "standard"), 3.0);
        assert!(r.cooldown("nightmare") < r.cooldown("standard"));
    }

    #[test]
    fn the_phases_run_in_order_and_end() {
        let r = InspectRules::godot();
        let (_, seen, step, _) = run_to_end(&r, "aki", "standard");
        assert_eq!(seen, vec![Phase::Walk, Phase::Turn, Phase::Lean, Phase::Linger, Phase::Rise]);
        assert_eq!(step, Step::Done);
    }

    #[test]
    fn the_breath_fills_while_leaning_and_never_reveals_in_standard() {
        let r = InspectRules::godot();
        let (i, _, step, peak) = run_to_end(&r, "aki", "nightmare");
        assert_eq!(step, Step::Done, "she moves on without finding the runner");
        assert!(peak > 0.5, "the meter filled in the lean and the linger: {peak}");
        assert!(i.breath < peak, "it drains as she rises");
        let mut walking = Inspect::start(&r, "aki", "standard", (0.0, 0.0), (0.0, 1.0));
        for _ in 0..10 {
            walking.update(&r, 0.05, 5.0); // still walking to the stand
        }
        assert_eq!(walking.breath, 0.0, "no breath while she walks up");
    }

    #[test]
    fn a_full_meter_reveals_only_when_the_data_says_so() {
        let strict = InspectRules::godot();
        assert_eq!(run_to_end(&strict, "aki", "nightmare").2, Step::Done);
        let reveal = InspectRules { reveal_when_full: true, ..InspectRules::godot() };
        // nightmare lingers long enough to fill the meter; standard does not
        assert_eq!(run_to_end(&reveal, "aki", "nightmare").2, Step::Reveal);
        assert_eq!(run_to_end(&reveal, "aki", "standard").2, Step::Done);
        // a faster fill rate in data fills it in Standard too
        let fast = InspectRules { reveal_when_full: true, breath_fill_per_second: 2.0, ..InspectRules::godot() };
        assert_eq!(run_to_end(&fast, "aki", "standard").2, Step::Reveal);
    }

    #[test]
    fn a_blocked_walk_turns_after_the_walk_limit() {
        let r = InspectRules::godot();
        let mut i = Inspect::start(&r, "aki", "standard", (0.0, 0.0), (0.0, 1.0));
        let mut t = 0.0;
        while i.phase == Phase::Walk && t < 20.0 {
            i.update(&r, 0.1, 3.0);
            t += 0.1;
        }
        assert_eq!(i.phase, Phase::Turn);
        assert!(t >= r.walk_limit - 0.2 && t <= r.walk_limit + 0.3, "{t}");
    }

    #[test]
    fn only_a_pass_that_did_not_see_the_hide_qualifies() {
        let r = InspectRules::godot();
        assert!(r.qualifies(2.0, false, 0.0, 0.0));
        assert!(!r.qualifies(2.0, true, 0.0, 0.0), "she saw it: the hide check runs instead");
        assert!(!r.qualifies(3.5, false, 0.0, 0.0), "too far");
        assert!(!r.qualifies(2.0, false, 5.0, 0.0), "on cooldown");
        assert!(!r.qualifies(2.0, false, 0.0, 5.0), "this spot was just inspected");
    }

    #[test]
    fn the_stand_point_backs_off_in_a_tight_hall_and_skips_a_shut_one() {
        let r = InspectRules::godot();
        let open = r.stand_point("aki", (0.0, 0.0), (0.0, 2.0), |_| true).unwrap();
        assert!((open.1 - 1.8).abs() < 1e-4 && open.0.abs() < 1e-4);
        let tight = r.stand_point("aki", (0.0, 0.0), (0.0, 1.0), |p| p.1 <= 1.65).unwrap();
        assert!(tight.1 <= 1.65 && tight.1 >= r.stand_min - 0.001);
        assert_eq!(r.stand_point("aki", (0.0, 0.0), (0.0, 1.0), |p| p.1 <= 1.0), None);
        assert_eq!(r.stand_point("aki", (0.0, 0.0), (0.0, 0.0), |_| true), None, "no door, no stand");
    }

    #[test]
    fn the_inspection_has_no_text_to_toast() {
        // the whole output of an inspection is a phase, a meter and a step: nothing carries a message
        let r = InspectRules::godot();
        let mut i = Inspect::start(&r, "aki", "hard", (0.0, 0.0), (0.0, 1.0));
        let s: Step = i.update(&r, 0.1, 0.0);
        assert_eq!(s, Step::Continue);
        assert!(matches!(i.phase, Phase::Walk | Phase::Turn));
    }

    #[test]
    fn capacity_comes_from_data_with_a_default() {
        assert_eq!(capacity_of("wardrobe"), 2);
        assert_eq!(capacity_of("closet"), 1);
        assert_eq!(capacity_of(""), 2, "an empty kind is a locker");
        assert_eq!(capacity_of("locker"), 2);
        assert_eq!(capacity_of("bed"), 2);
        assert_eq!(capacity_of("no such kind"), capacity_in(HIDING, "no such kind"));
        assert_eq!(capacity_of("no such kind"), 1, "the data default");
        let changed = HIDING.replace("\"furniture_capacity\": { \"locker\": 2", "\"furniture_capacity\": { \"locker\": 3");
        assert_eq!(capacity_in(&changed, "locker"), 3);
        assert_eq!(capacity_in("not json", "locker"), 1);
    }

    #[test]
    fn the_free_slot_is_the_lowest_unheld_one() {
        assert_eq!(free_slot(2, &[]), Some(0));
        assert_eq!(free_slot(2, &[0]), Some(1));
        assert_eq!(free_slot(2, &[1]), Some(0));
        assert_eq!(free_slot(2, &[0, 1]), None);
        assert_eq!(free_slot(1, &[0]), None);
    }
}
