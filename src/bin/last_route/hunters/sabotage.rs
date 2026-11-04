//! Hunter sabotage: a hunter that lingers near a facility relay can knock it out of service.
//!
//! Godot reference: the relay saboteur tuning, `BRISELLE_SABOTAGE_DWELL` (5.0 s) and `RELAY_RESECURE_SECONDS`
//! (2.6 s), and `BRISELLE_ROOM_CHANCE` (0.5) in game/config/game_config.gd:525-534. Godot's dwell is tied to room
//! entry; the near radius (`relay.near_cells` = 3) has no Godot source and is this port's own choice. The sabotaged-relay state is
//! `GameState.relays_sabotaged` (game/autoload/game_state.gd:27) and `sabotaged_relays` in the run save
//! (game/autoload/run_saves.gd:41). In the current Godot build nothing rolls a relay sabotage (the Briselle saboteur
//! was replaced by her relay tracker), so every difficulty ships disabled and Standard matches the current game.
//!
//! All tuning lives in `data/hunters/sabotage.json` (`lr.hunter_sabotage/1`): the shared relay timings and, per
//! difficulty, whether sabotage is on, its target and its chance per dwell. Unknown keys and unknown targets are
//! errors, so a typo never silently changes the game.
//!
//! Floor effect: a sabotaged relay is offline (blocked) for `relay.resecure_s` seconds, then comes back by itself.
//! Godot's `RELAY_RESECURE_SECONDS` (game_config.gd:534) is a hold-to-restore by the runner; this port has no hold
//! yet, so the timer runs out on its own. `Tracker::is_blocked` tells the floor which relays are offline.
//! Not yet ported: lights, doors and the lift have no Godot sabotage to copy.

use crate::world::facility::Cell;
use crate::world::rng::Rng;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.hunter_sabotage/1";

/// What a sabotage can take offline. Only relays have a Godot reference so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Relay,
}

impl Target {
    fn parse(name: &str) -> Option<Target> {
        match name {
            "relay" => Some(Target::Relay),
            _ => None,
        }
    }
}

/// One difficulty's sabotage rule, with the shared relay timings copied in.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub enabled: bool,
    pub target: Target,
    /// Chance of a sabotage each time a hunter finishes a dwell.
    pub chance: f64,
    /// Seconds a hunter must stay within `near_cells` of a standing relay before a roll.
    pub dwell: f32,
    /// Cells (Chebyshev distance) from a relay that count as near it.
    pub near_cells: i32,
    /// Seconds for a runner to resecure a sabotaged relay.
    pub resecure: f32,
}

/// The parsed `lr.hunter_sabotage/1` file: one rule per difficulty id.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    by_difficulty: HashMap<String, Rule>,
}

/// What a roll did. The relay named is blocked on the floor for `resecure` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    RelaySabotaged { cell: Cell, resecure: f32 },
}

fn cheb(a: Cell, b: Cell) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

const RELAY_KEYS: [&str; 3] = ["near_cells", "dwell_s", "resecure_s"];
const DIFFICULTY_KEYS: [&str; 3] = ["enabled", "target", "chance"];

impl Rules {
    /// Parse and check the data file. Every key must be known and every value in range.
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let obj = v.as_object().ok_or("sabotage data must be an object")?;
        let schema = obj.get("schema").and_then(|s| s.as_str()).unwrap_or("");
        if schema != SCHEMA {
            return Err(format!("unknown schema {schema:?}"));
        }
        check_keys(obj.keys().map(|k| k.as_str()), &["schema", "relay", "difficulties"])?;

        let relay = obj
            .get("relay")
            .and_then(|r| r.as_object())
            .ok_or("missing relay block")?;
        check_keys(relay.keys().map(|k| k.as_str()), &RELAY_KEYS)?;
        let near_cells = int(relay, "near_cells")?;
        let dwell = num(relay, "dwell_s")? as f32;
        let resecure = num(relay, "resecure_s")? as f32;
        if near_cells < 0 || dwell < 0.0 || resecure < 0.0 {
            return Err("relay timings must not be negative".into());
        }

        let diffs = obj
            .get("difficulties")
            .and_then(|d| d.as_object())
            .ok_or("missing difficulties block")?;
        let mut by_difficulty = HashMap::new();
        for (id, body) in diffs {
            let body = body.as_object().ok_or_else(|| format!("{id}: must be an object"))?;
            check_keys(body.keys().map(|k| k.as_str()), &DIFFICULTY_KEYS)?;
            let target_name = body
                .get("target")
                .and_then(|t| t.as_str())
                .ok_or_else(|| format!("{id}: missing target"))?;
            let target = Target::parse(target_name)
                .ok_or_else(|| format!("{id}: unknown target {target_name:?}"))?;
            let enabled = body
                .get("enabled")
                .and_then(|e| e.as_bool())
                .ok_or_else(|| format!("{id}: missing enabled"))?;
            let chance = num(body, "chance")?;
            if !(0.0..=1.0).contains(&chance) {
                return Err(format!("{id}: chance must be between 0 and 1"));
            }
            by_difficulty.insert(
                id.clone(),
                Rule { enabled, target, chance, dwell, near_cells, resecure },
            );
        }
        Ok(Rules { by_difficulty })
    }

    /// The rule for a difficulty id. A difficulty with no entry is off.
    pub fn for_difficulty(&self, id: &str) -> Option<&Rule> {
        self.by_difficulty.get(id)
    }
}

fn check_keys<'a>(keys: impl Iterator<Item = &'a str>, known: &[&str]) -> Result<(), String> {
    for k in keys {
        if !known.contains(&k) {
            return Err(format!("unknown key {k:?}"));
        }
    }
    Ok(())
}

fn num(obj: &serde_json::Map<String, Value>, key: &str) -> Result<f64, String> {
    obj.get(key)
        .and_then(|x| x.as_f64())
        .ok_or_else(|| format!("missing or non-numeric {key}"))
}

fn int(obj: &serde_json::Map<String, Value>, key: &str) -> Result<i32, String> {
    obj.get(key)
        .and_then(|x| x.as_i64())
        .map(|n| n as i32)
        .ok_or_else(|| format!("missing or non-integer {key}"))
}

/// The embedded rules (data/hunters/sabotage.json), read once. Off everywhere when the file is missing or broken.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("hunters/sabotage.json")
            .next()
            .and_then(|(_, text)| Rules::parse(text).ok())
            .unwrap_or(Rules { by_difficulty: HashMap::new() })
    })
}

/// Per-floor sabotage state: the dwell clock and the seeded roll stream. Built once per floor by the hunting loop.
#[derive(Clone)]
pub struct Tracker {
    depth: i32,
    rule: Option<Rule>,
    rng: Rng,
    dwell: f32,
    last: Option<Outcome>,
    /// Relays offline from a sabotage, each with the seconds left before it is resecured.
    blocked: Vec<(Cell, f32)>,
}

impl Tracker {
    /// The tracker for one floor, seeded from the depth so a replay draws the same rolls.
    pub fn for_floor(depth: i32, difficulty_id: &str) -> Tracker {
        Tracker::with_rules(rules(), depth, difficulty_id)
    }

    pub fn with_rules(rules: &Rules, depth: i32, difficulty_id: &str) -> Tracker {
        let rule = rules.for_difficulty(difficulty_id).filter(|r| r.enabled).cloned();
        Tracker {
            depth,
            rule,
            rng: Rng::new(&format!("{depth}:HUNTER-SABOTAGE")),
            dwell: 0.0,
            last: None,
            blocked: Vec::new(),
        }
    }

    /// Whether this relay is offline from a sabotage that has not been resecured yet.
    pub fn is_blocked(&self, cell: Cell) -> bool {
        self.blocked.iter().any(|(c, _)| *c == cell)
    }

    /// The relays offline now, each with the seconds left before it is resecured.
    pub fn blocked(&self) -> &[(Cell, f32)] {
        &self.blocked
    }

    pub fn depth(&self) -> i32 {
        self.depth
    }

    pub fn last(&self) -> Option<Outcome> {
        self.last
    }

    /// Advance one frame. `hunters` are their positions, `relays` the relays still standing. Offline relays count
    /// down their resecure time first and are not candidates. A hunter near an online relay builds dwell; each full
    /// dwell rolls the chance once against the nearest such relay and then starts again.
    pub fn tick(
        &mut self,
        dt: f32,
        hunters: impl Iterator<Item = (f32, f32)>,
        relays: &[Cell],
    ) -> Option<Outcome> {
        let rule = self.rule.clone()?;
        self.blocked.retain_mut(|(_, left)| {
            *left -= dt;
            *left > 0.0
        });
        let online: Vec<Cell> = relays.iter().copied().filter(|r| !self.is_blocked(*r)).collect();
        let mut hit: Option<Cell> = None;
        for p in hunters {
            let c = crate::hunters::brain::cell_of(p);
            let nearest = online
                .iter()
                .copied()
                .filter(|r| cheb(c, *r) <= rule.near_cells)
                .min_by_key(|r| cheb(c, *r));
            if nearest.is_some() {
                hit = nearest;
                break;
            }
        }
        let Some(cell) = hit else {
            self.dwell = 0.0;
            return None;
        };
        self.dwell += dt;
        if self.dwell < rule.dwell {
            return None;
        }
        self.dwell = 0.0;
        if self.rng.next_float() < rule.chance {
            let outcome = match rule.target {
                Target::Relay => Outcome::RelaySabotaged { cell, resecure: rule.resecure },
            };
            self.blocked.push((cell, rule.resecure));
            self.last = Some(outcome);
            return Some(outcome);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_rules() -> Rules {
        Rules::parse(include_str!("../data/hunters/sabotage.json")).expect("sabotage.json parses")
    }

    /// A copy of the data with one difficulty's enabled/chance replaced.
    fn tweaked(id: &str, enabled: bool, chance: f64) -> String {
        let mut v: Value = serde_json::from_str(include_str!("../data/hunters/sabotage.json")).unwrap();
        v["difficulties"][id]["enabled"] = Value::Bool(enabled);
        v["difficulties"][id]["chance"] = Value::from(chance);
        v.to_string()
    }

    /// Frames until the first outcome, with one hunter parked beside the relay.
    fn first_outcome(t: &mut Tracker, relays: &[Cell], frames: usize, dt: f32) -> Option<usize> {
        let at = (relays[0].x as f32 + 0.5, relays[0].y as f32 + 0.5);
        (0..frames).find(|_| t.tick(dt, std::iter::once(at), relays).is_some())
    }

    #[test]
    fn standard_matches_the_current_game_and_is_off() {
        let r = std_rules();
        for id in ["relaxed", "standard", "hard", "nightmare"] {
            assert!(!r.for_difficulty(id).unwrap().enabled, "{id} should ship off");
        }
        // Godot numbers: BRISELLE_SABOTAGE_DWELL 5.0, RELAY_RESECURE_SECONDS 2.6 (game_config.gd:533-534).
        let relay = r.for_difficulty("standard").unwrap();
        assert_eq!(relay.dwell, 5.0);
        assert_eq!(relay.resecure, 2.6);
        assert_eq!(relay.chance, 0.5);
        let mut t = Tracker::with_rules(&r, 3, "standard");
        let relays = [Cell { x: 10, y: 10 }];
        assert_eq!(first_outcome(&mut t, &relays, 2000, 0.1), None);
    }

    #[test]
    fn enabling_a_difficulty_in_data_changes_behaviour() {
        let r = Rules::parse(&tweaked("hard", true, 1.0)).unwrap();
        let mut t = Tracker::with_rules(&r, 3, "hard");
        let relays = [Cell { x: 10, y: 10 }];
        let frames = first_outcome(&mut t, &relays, 200, 0.5).expect("sabotage fires");
        // dwell 5.0 s at 0.5 s a frame: the first roll lands on the 10th frame (index 9).
        assert_eq!(frames, 9);
        assert_eq!(t.last(), Some(Outcome::RelaySabotaged { cell: Cell { x: 10, y: 10 }, resecure: 2.6 }));
    }

    /// Standard enabled at chance 1.0 with a given resecure_s.
    fn with_resecure(resecure: f64) -> String {
        let mut v: Value = serde_json::from_str(include_str!("../data/hunters/sabotage.json")).unwrap();
        v["difficulties"]["standard"]["enabled"] = Value::Bool(true);
        v["difficulties"]["standard"]["chance"] = Value::from(1.0);
        v["relay"]["resecure_s"] = Value::from(resecure);
        v.to_string()
    }

    /// Far from every relay, so nothing rolls and no dwell builds.
    const FAR: (f32, f32) = (50.5, 50.5);

    #[test]
    fn sabotaged_relay_is_blocked_until_its_resecure_time() {
        let r = Rules::parse(&tweaked("standard", true, 1.0)).unwrap();
        let mut t = Tracker::with_rules(&r, 3, "standard");
        let relays = [Cell { x: 10, y: 10 }];
        assert_eq!(first_outcome(&mut t, &relays, 200, 0.5), Some(9));
        assert!(t.is_blocked(relays[0]));
        // resecure 2.6 s at 0.5 s a frame: still blocked after 5 frames (2.5 s), free on the 6th
        for _ in 0..5 {
            t.tick(0.5, std::iter::once(FAR), &relays);
        }
        assert!(t.is_blocked(relays[0]));
        t.tick(0.5, std::iter::once(FAR), &relays);
        assert!(!t.is_blocked(relays[0]));
    }

    #[test]
    fn a_blocked_relay_is_not_sabotaged_again_while_offline() {
        let r = Rules::parse(&tweaked("standard", true, 1.0)).unwrap();
        let mut t = Tracker::with_rules(&r, 3, "standard");
        let relays = [Cell { x: 10, y: 10 }];
        assert_eq!(first_outcome(&mut t, &relays, 200, 0.5), Some(9));
        // 2.0 s is inside the 2.6 s window, and the only relay is offline: no new dwell, no new roll
        for _ in 0..4 {
            assert_eq!(t.tick(0.5, std::iter::once((10.5, 10.5)), &relays), None);
        }
    }

    #[test]
    fn sabotage_hits_the_relay_nearest_the_dwelling_hunter() {
        let r = Rules::parse(&tweaked("standard", true, 1.0)).unwrap();
        let mut t = Tracker::with_rules(&r, 3, "standard");
        let relays = [Cell { x: 10, y: 10 }, Cell { x: 13, y: 10 }];
        let near_second = (13.5, 10.5);
        let frames = (0..200).find(|_| t.tick(0.5, std::iter::once(near_second), &relays).is_some());
        assert_eq!(frames, Some(9));
        assert_eq!(
            t.last(),
            Some(Outcome::RelaySabotaged { cell: Cell { x: 13, y: 10 }, resecure: 2.6 })
        );
        assert!(t.is_blocked(Cell { x: 13, y: 10 }));
        assert!(!t.is_blocked(Cell { x: 10, y: 10 }));
    }

    #[test]
    fn resecure_time_in_data_sets_how_long_a_relay_stays_blocked() {
        let r = Rules::parse(&with_resecure(1.0)).unwrap();
        let mut t = Tracker::with_rules(&r, 3, "standard");
        let relays = [Cell { x: 10, y: 10 }];
        assert_eq!(first_outcome(&mut t, &relays, 200, 0.5), Some(9));
        assert_eq!(t.last(), Some(Outcome::RelaySabotaged { cell: relays[0], resecure: 1.0 }));
        // 1.0 s at 0.5 s a frame: blocked after one far frame, free after two
        t.tick(0.5, std::iter::once(FAR), &relays);
        assert!(t.is_blocked(relays[0]));
        t.tick(0.5, std::iter::once(FAR), &relays);
        assert!(!t.is_blocked(relays[0]));
    }

    #[test]
    fn standard_resecure_is_2_6_seconds_in_the_shipped_data() {
        let r = std_rules();
        assert_eq!(r.for_difficulty("standard").unwrap().resecure, 2.6);
    }

    #[test]
    fn chance_in_data_is_used_for_the_roll() {
        let never = Rules::parse(&tweaked("standard", true, 0.0)).unwrap();
        let mut t = Tracker::with_rules(&never, 3, "standard");
        assert_eq!(first_outcome(&mut t, &[Cell { x: 0, y: 0 }], 5000, 0.1), None);
    }

    #[test]
    fn hunter_far_from_every_relay_builds_no_dwell() {
        let r = Rules::parse(&tweaked("standard", true, 1.0)).unwrap();
        let mut t = Tracker::with_rules(&r, 3, "standard");
        let relays = [Cell { x: 0, y: 0 }];
        let far = (50.5, 50.5);
        for _ in 0..1000 {
            assert_eq!(t.tick(0.1, std::iter::once(far), &relays), None);
        }
    }

    #[test]
    fn near_radius_comes_from_data() {
        let relays = [Cell { x: 10, y: 10 }];
        // a hunter two cells off the relay: inside the radius of 3, outside a radius of 0
        let two_off = (12.5, 10.5);
        let mut v: Value = serde_json::from_str(&tweaked("standard", true, 1.0)).unwrap();
        v["relay"]["near_cells"] = Value::from(0);
        let tight = Rules::parse(&v.to_string()).unwrap();
        let mut t = Tracker::with_rules(&tight, 3, "standard");
        for _ in 0..500 {
            assert_eq!(t.tick(0.5, std::iter::once(two_off), &relays), None);
        }
        let wide = Rules::parse(&tweaked("standard", true, 1.0)).unwrap();
        let mut t = Tracker::with_rules(&wide, 3, "standard");
        assert!((0..20).any(|_| t.tick(0.5, std::iter::once(two_off), &relays).is_some()));
    }

    #[test]
    fn same_depth_gives_the_same_rolls() {
        let r = Rules::parse(&tweaked("standard", true, 0.5)).unwrap();
        let relays = [Cell { x: 4, y: 4 }];
        let run = |depth| {
            let mut t = Tracker::with_rules(&r, depth, "standard");
            (0..40).map(|_| first_outcome(&mut t, &relays, 100, 0.1)).collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
    }

    #[test]
    fn unknown_keys_and_targets_are_errors() {
        let mut v: Value = serde_json::from_str(include_str!("../data/hunters/sabotage.json")).unwrap();
        v["difficulties"]["standard"]["target"] = Value::from("lift");
        assert!(Rules::parse(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(include_str!("../data/hunters/sabotage.json")).unwrap();
        v["relay"]["dwel_s"] = Value::from(1.0);
        assert!(Rules::parse(&v.to_string()).is_err());
    }
}
