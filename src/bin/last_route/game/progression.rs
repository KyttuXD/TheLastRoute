//! Relay progression (`lr.progression/1`, data/progression.json): how many signal relays a floor
//! holds, what the lift and the hunters need from them, how many hunters wake and stream in for a
//! relay count, the chase-speed bonus per relay, and when relay rush starts. The rules are data so
//! KT or a pack can retune them without code; Standard values match the Godot reference
//! (game/config/game_config.gd 161, 388, 408-413, 422-428; game/hunters/ai/hunter_director.gd 39-40).

use crate::config::Difficulty;
use serde_json::Value;
use std::sync::OnceLock;

/// The per-difficulty relay gates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DifficultyGate {
    /// Relays needed before the first hunter wakes (HUNTER_ACTIVATION_RELAYS).
    pub activation_relays: i32,
    /// Hunters awake at the activation relay in an Infinite run (STREAMED_HUNTER_BASE, default 1).
    pub streamed_base: i32,
    /// Relays between each extra streamed hunter (STREAMED_HUNTER_RELAY_STEP).
    pub streamed_step: i32,
}

/// Something the run unlocks once the runner has collected `relays` relays.
#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    pub relays: i32,
    pub unlocks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Progression {
    /// Relays on a floor (FACILITY_RELAY_COUNT).
    pub relay_count: i32,
    /// Score for one recovered relay (RELAY_SCORE).
    pub relay_score: i32,
    /// The lift waits for every relay on the floor (true) or for `relay_count` (false).
    pub lift_needs_all: bool,
    /// Relaxed, Standard, Hard, Nightmare.
    pub gates: [DifficultyGate; 4],
    /// Chase-speed bonus per collected relay (HUNTER_RELAY_SPEED_BONUS).
    pub chase_speed_bonus_per_relay: f32,
    /// Every hunter's chase speed (the `chase` + relay/depth bonus, before the sight-line dash, winded, slow and
    /// difficulty scales) is multiplied by this, one value per difficulty (Relaxed, Standard, Hard, Nightmare). 1.0 is
    /// the original roster; the per-hunter `chase` values stay intact. Wander, investigate and search speeds are not
    /// touched. Only Nightmare is tuned high enough to let a clear-line dash out-run the runner's sprint.
    pub chase_speed_scale: [f32; 4],
    /// Hunter categories (`category` in data/hunters/*.json) that keep their authored chase speed, e.g. the secret
    /// wolf spiders.
    pub chase_speed_scale_skip: Vec<String>,
    /// Relay rush starts at this many relays (RELAY_RUSH_COUNT) ...
    pub rush_relays: i32,
    /// ... once that has held this long, in seconds (RELAY_RUSH_DELAY).
    pub rush_delay: f32,
    /// Hunters in Search that are at least `rush_search_clear` cells from every runner can also
    /// be sent to a relay (relay_rush.include_searching).
    pub rush_search: bool,
    /// Minimum runner gap, in cells, for a searching hunter to join the rush (relay_rush.search_clear).
    pub rush_search_clear: f32,
    pub milestones: Vec<Milestone>,
}

impl Default for Progression {
    fn default() -> Progression {
        let gate = |activation_relays, streamed_base, streamed_step| DifficultyGate {
            activation_relays,
            streamed_base,
            streamed_step,
        };
        Progression {
            relay_count: 5,
            relay_score: 100,
            lift_needs_all: true,
            // Relaxed, Standard, Hard, Nightmare
            gates: [gate(3, 1, 3), gate(1, 1, 2), gate(0, 1, 1), gate(0, 2, 1)],
            chase_speed_bonus_per_relay: 0.1,
            // Relaxed, Standard, Hard, Nightmare (see data/progression.json for why Hard is 1.0)
            chase_speed_scale: [1.06, 1.06, 1.0, 1.09],
            chase_speed_scale_skip: vec!["secret".into()],
            rush_relays: 7,
            rush_delay: 45.0,
            rush_search: true,
            rush_search_clear: 12.0,
            milestones: vec![
                Milestone { relays: 5, unlocks: vec!["service_lift".into()] },
                Milestone { relays: 7, unlocks: vec!["relay_rush".into()] },
            ],
        }
    }
}

fn index(d: Difficulty) -> usize {
    match d {
        Difficulty::Relaxed => 0,
        Difficulty::Standard => 1,
        Difficulty::Hard => 2,
        Difficulty::Nightmare => 3,
    }
}

const DIFFICULTY_KEYS: [&str; 4] = ["relaxed", "standard", "hard", "nightmare"];

impl Progression {
    /// Reads `lr.progression/1`. Missing or malformed fields keep the Standard default.
    pub fn parse(v: &Value) -> Progression {
        let mut p = Progression::default();
        if let Some(n) = v["relays"]["count"].as_i64() {
            p.relay_count = n.clamp(1, 64) as i32;
        }
        if let Some(n) = v["relays"]["score"].as_i64() {
            p.relay_score = n.clamp(0, 1_000_000) as i32;
        }
        if let Some(b) = v["relays"]["lift_needs_all"].as_bool() {
            p.lift_needs_all = b;
        }
        for (i, k) in DIFFICULTY_KEYS.iter().enumerate() {
            let d = &v["difficulty"][k];
            if let Some(n) = d["activation_relays"].as_i64() {
                p.gates[i].activation_relays = n.clamp(0, 64) as i32;
            }
            if let Some(n) = d["streamed_base"].as_i64() {
                p.gates[i].streamed_base = n.clamp(0, 64) as i32;
            }
            if let Some(n) = d["streamed_step"].as_i64() {
                p.gates[i].streamed_step = n.clamp(1, 64) as i32;
            }
        }
        if let Some(x) = v["chase_speed_bonus_per_relay"].as_f64() {
            p.chase_speed_bonus_per_relay = (x as f32).clamp(0.0, 10.0);
        }
        // a number is one scale for every difficulty; an object names them (a missing or junk one keeps its default)
        if let Some(x) = v["chase_speed_scale"].as_f64() {
            p.chase_speed_scale = [(x as f32).clamp(0.5, 2.0); 4];
        } else if v["chase_speed_scale"].is_object() {
            for (i, k) in DIFFICULTY_KEYS.iter().enumerate() {
                if let Some(x) = v["chase_speed_scale"][k].as_f64() {
                    p.chase_speed_scale[i] = (x as f32).clamp(0.5, 2.0);
                }
            }
        }
        if let Some(a) = v["chase_speed_scale_skip"].as_array() {
            p.chase_speed_scale_skip = a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        }
        if let Some(n) = v["relay_rush"]["relays"].as_i64() {
            p.rush_relays = n.clamp(0, 64) as i32;
        }
        if let Some(x) = v["relay_rush"]["delay_seconds"].as_f64() {
            p.rush_delay = (x as f32).clamp(0.0, 3600.0);
        }
        if let Some(b) = v["relay_rush"]["include_searching"].as_bool() {
            p.rush_search = b;
        }
        if let Some(x) = v["relay_rush"]["search_clear"].as_f64() {
            p.rush_search_clear = (x as f32).clamp(0.0, 1000.0);
        }
        if let Some(list) = v["milestones"].as_array() {
            p.milestones = list
                .iter()
                .filter_map(|m| {
                    let relays = m["relays"].as_i64()?.clamp(0, 64) as i32;
                    let unlocks = m["unlocks"]
                        .as_array()?
                        .iter()
                        .filter_map(|u| u.as_str().map(str::to_string))
                        .collect();
                    Some(Milestone { relays, unlocks })
                })
                .collect();
        }
        p
    }

    /// The embedded data/progression.json (the Standard defaults if it is missing or broken).
    pub fn load() -> &'static Progression {
        static RULES: OnceLock<Progression> = OnceLock::new();
        RULES.get_or_init(|| {
            crate::content::embedded::data_under("progression.json")
                .next()
                .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
                .map(|v| Progression::parse(&v))
                .unwrap_or_default()
        })
    }

    pub fn gate(&self, d: Difficulty) -> DifficultyGate {
        self.gates[index(d)]
    }

    /// Relays before the first hunter wakes on this difficulty.
    pub fn activation_relays(&self, d: Difficulty) -> i32 {
        self.gate(d).activation_relays
    }

    /// Hunters awake in an Infinite run after `relays_collected` relays (Godot
    /// `GameConfig.streamed_hunter_population`): none before the activation relay, then the base
    /// lineup plus one more every `streamed_step` relays.
    pub fn streamed_hunters(&self, d: Difficulty, relays_collected: i32) -> i32 {
        let g = self.gate(d);
        if relays_collected < g.activation_relays {
            return 0;
        }
        g.streamed_base + (relays_collected - g.activation_relays) / g.streamed_step.max(1)
    }

    /// Chase-speed bonus from the relays collected so far.
    pub fn chase_speed_bonus(&self, relays_collected: i32) -> f32 {
        relays_collected.max(0) as f32 * self.chase_speed_bonus_per_relay
    }

    /// The chase-speed multiplier for a hunter of this category on `d`: `chase_speed_scale` for that difficulty, or
    /// 1.0 for a skipped category.
    pub fn chase_scale_for(&self, d: Difficulty, category: &str) -> f32 {
        if self.chase_speed_scale_skip.iter().any(|c| c == category) {
            1.0
        } else {
            self.chase_speed_scale[index(d)]
        }
    }

    /// Relay rush is on once `relays_collected` reaches the threshold and it has held `held_seconds`.
    pub fn relay_rush_active(&self, relays_collected: i32, held_seconds: f32) -> bool {
        relays_collected >= self.rush_relays && held_seconds >= self.rush_delay
    }

    /// The lift is open when the runner has the relays it needs on this floor.
    pub fn lift_open(&self, relays_collected: i32, relays_on_floor: i32) -> bool {
        let need = if self.lift_needs_all { relays_on_floor } else { self.relay_count };
        relays_collected >= need
    }

    /// What unlocks at exactly this many relays.
    pub fn unlocks_at(&self, relays_collected: i32) -> Vec<&str> {
        self.milestones
            .iter()
            .filter(|m| m.relays == relays_collected)
            .flat_map(|m| m.unlocks.iter().map(String::as_str))
            .collect()
    }
}


/// When the run saves itself (`autosave` in data/progression.json). `triggers` lists the events that save at all,
/// `always` the ones that ignore `min_interval_s` (a pickup burst is rate limited, a quit never is).
#[derive(Clone, Debug, PartialEq)]
pub struct Autosave {
    pub triggers: Vec<String>,
    pub always: Vec<String>,
    pub keep_backup: bool,
    pub store_position: bool,
    pub min_interval_s: f32,
}

impl Default for Autosave {
    fn default() -> Autosave {
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect();
        Autosave {
            triggers: s(&["boundary", "pause_quit", "pickup", "safe_room"]),
            always: s(&["boundary", "pause_quit", "safe_room"]),
            keep_backup: true,
            store_position: true,
            min_interval_s: 5.0,
        }
    }
}

impl Autosave {
    /// Reads the root of progression.json; a missing section keeps the defaults.
    pub fn parse(root: &Value) -> Autosave {
        let d = Autosave::default();
        let a = &root["autosave"];
        let list = |k: &str, dv: Vec<String>| {
            a[k].as_array().map(|x| x.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or(dv)
        };
        Autosave {
            triggers: list("triggers", d.triggers),
            always: list("always", d.always),
            keep_backup: a["keep_backup"].as_bool().unwrap_or(d.keep_backup),
            store_position: a["store_position"].as_bool().unwrap_or(d.store_position),
            min_interval_s: a["min_interval_s"].as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as f32).unwrap_or(d.min_interval_s),
        }
    }

    pub fn load() -> &'static Autosave {
        static A: OnceLock<Autosave> = OnceLock::new();
        A.get_or_init(|| {
            crate::content::embedded::data_under("progression.json")
                .next()
                .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
                .map(|v| Autosave::parse(&v))
                .unwrap_or_default()
        })
    }

    /// Whether `trigger` saves now, given the seconds since the last save.
    pub fn due(&self, trigger: &str, since_last_s: f32) -> bool {
        if !self.triggers.iter().any(|t| t == trigger) {
            return false;
        }
        self.always.iter().any(|t| t == trigger) || since_last_s >= self.min_interval_s
    }
}

#[cfg(test)]
mod autosave_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_embedded_autosave_matches_the_defaults() {
        let (_, text) = crate::content::embedded::data_under("progression.json").next().unwrap();
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(Autosave::parse(&v), Autosave::default());
        assert_eq!(Autosave::load(), &Autosave::default());
    }

    #[test]
    fn pickups_are_rate_limited_but_quit_and_boundary_always_save() {
        let a = Autosave::load();
        assert!(!a.due("pickup", 0.5));
        assert!(a.due("pickup", a.min_interval_s));
        assert!(a.due("pause_quit", 0.0) && a.due("boundary", 0.0));
        assert!(!a.due("unknown", 100.0));
    }

    #[test]
    fn changed_data_changes_behaviour() {
        let a = Autosave::parse(&json!({"autosave": {"triggers": ["pickup"], "always": [], "min_interval_s": 30.0}}));
        assert!(!a.due("pause_quit", 100.0));
        assert!(!a.due("pickup", 29.0) && a.due("pickup", 30.0));
        assert_eq!(Autosave::parse(&json!({})), Autosave::default());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FACILITY_RELAY_COUNT, HUNTER_RELAY_SPEED_BONUS, RELAY_SCORE};
    use serde_json::json;

    #[test]
    fn the_embedded_file_matches_the_standard_values() {
        let p = Progression::load();
        assert_eq!(p.relay_count as usize, FACILITY_RELAY_COUNT);
        assert_eq!(p.relay_score, RELAY_SCORE);
        assert_eq!(p.chase_speed_bonus_per_relay, HUNTER_RELAY_SPEED_BONUS);
        // Standard values from the old director consts (Godot hunter_director.gd 39-40).
        assert_eq!(p.rush_relays, 7);
        assert_eq!(p.rush_delay, 45.0);
        assert_eq!(crate::hunters::director::relay_rush_count(), p.rush_relays);
        assert_eq!(crate::hunters::director::relay_rush_delay(), p.rush_delay);
        assert_eq!(p.activation_relays(Difficulty::Relaxed), 3);
        assert_eq!(p.activation_relays(Difficulty::Standard), 1);
        assert_eq!(p.activation_relays(Difficulty::Hard), 0);
        assert_eq!(p.activation_relays(Difficulty::Nightmare), 0);
        assert_eq!(p.gate(Difficulty::Nightmare).streamed_base, 2);
        assert_eq!(p.gate(Difficulty::Relaxed).streamed_step, 3);
        assert_eq!(p.gate(Difficulty::Standard).streamed_step, 2);
        assert_eq!(p.gate(Difficulty::Hard).streamed_step, 1);
        assert_eq!(p.gate(Difficulty::Nightmare).streamed_step, 1);
        assert_eq!(*p, Progression::parse(&json!({})), "an empty file keeps the defaults");
    }

    #[test]
    fn repeated_loads_return_the_cached_rules() {
        // load() is called every director tick; it must parse the embedded JSON once and hand
        // back the same cached value, with the same numbers, on every call.
        let first = Progression::load();
        let second = Progression::load();
        assert!(std::ptr::eq(first, second), "load() should return the one cached instance");
        assert_eq!(first, second);
        assert_eq!(first.rush_relays, second.rush_relays);
        assert_eq!(first.rush_delay, second.rush_delay);
        assert_eq!(first.relay_score, second.relay_score);
    }

    #[test]
    fn standard_activation_matches_the_difficulty_profile() {
        for d in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            assert_eq!(Progression::load().activation_relays(d), d.profile().activation_relays, "{d:?}");
        }
    }

    #[test]
    fn streamed_hunters_follow_the_godot_formula() {
        let p = Progression::load();
        assert_eq!(p.streamed_hunters(Difficulty::Standard, 0), 0);
        assert_eq!(p.streamed_hunters(Difficulty::Standard, 1), 1);
        assert_eq!(p.streamed_hunters(Difficulty::Standard, 3), 2);
        assert_eq!(p.streamed_hunters(Difficulty::Relaxed, 2), 0);
        assert_eq!(p.streamed_hunters(Difficulty::Relaxed, 3), 1);
        assert_eq!(p.streamed_hunters(Difficulty::Relaxed, 6), 2);
        assert_eq!(p.streamed_hunters(Difficulty::Nightmare, 0), 2);
        assert_eq!(p.streamed_hunters(Difficulty::Nightmare, 3), 5);
    }

    #[test]
    fn changing_the_data_changes_the_behaviour() {
        let v = json!({
            "relays": { "count": 3, "lift_needs_all": false },
            "difficulty": { "standard": { "activation_relays": 2, "streamed_base": 1, "streamed_step": 4 } },
            "chase_speed_bonus_per_relay": 0.5,
            "relay_rush": { "relays": 2, "delay_seconds": 10.0 },
            "milestones": [ { "relays": 2, "unlocks": ["x"] } ]
        });
        let p = Progression::parse(&v);
        assert_eq!(p.activation_relays(Difficulty::Standard), 2);
        assert_eq!(p.streamed_hunters(Difficulty::Standard, 1), 0);
        assert_eq!(p.streamed_hunters(Difficulty::Standard, 6), 2);
        assert_eq!(p.chase_speed_bonus(4), 2.0);
        assert!(p.relay_rush_active(2, 10.0));
        assert!(!p.relay_rush_active(2, 9.9));
        assert!(p.lift_open(3, 5), "lift waits for relay_count, not the floor's relays");
        assert!(!p.lift_open(2, 2));
        assert_eq!(p.unlocks_at(2), vec!["x"]);
        assert!(p.unlocks_at(5).is_empty());
    }

    #[test]
    fn the_lift_needs_every_relay_on_the_floor_by_default() {
        let p = Progression::load();
        assert!(!p.lift_open(4, 5));
        assert!(p.lift_open(5, 5));
    }

    #[test]
    fn changing_the_rush_and_score_data_changes_the_consumers() {
        let v = json!({
            "relays": { "score": 250 },
            "difficulty": { "hard": { "activation_relays": 2 } },
            "relay_rush": { "relays": 4, "delay_seconds": 12.5 },
            "chase_speed_bonus_per_relay": 0.25
        });
        let p = Progression::parse(&v);
        assert_eq!(p.relay_score, 250, "interact adds this per relay");
        assert_eq!(p.activation_relays(Difficulty::Hard), 2, "interact gates the first hunter on this");
        assert_eq!(p.chase_speed_bonus(4), 1.0, "brain adds this to the chase speed");
        assert!(p.relay_rush_active(4, 12.5), "director rush uses these values");
        assert!(!p.relay_rush_active(3, 99.0));
    }

    #[test]
    fn milestones_unlock_at_their_relay_count() {
        let p = Progression::load();
        assert_eq!(p.unlocks_at(5), vec!["service_lift"]);
        assert_eq!(p.unlocks_at(7), vec!["relay_rush"]);
        assert!(p.unlocks_at(2).is_empty());
    }

    #[test]
    fn relay_rush_waits_for_count_and_delay() {
        let p = Progression::load();
        assert!(!p.relay_rush_active(6, 100.0));
        assert!(!p.relay_rush_active(7, 44.0));
        assert!(p.relay_rush_active(7, 45.0));
    }
}
