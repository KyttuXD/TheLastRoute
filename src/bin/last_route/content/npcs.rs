//! `lr.npcs/1` (data/npcs.json): how many NPC teammates a normal Infinite run starts with, where they stand
//! and how smart they are. Godot has no automatic spawn (only the lobby's NPC TEAMMATES count and the admin
//! page), so this is the Rust port's rule; it is data so a pack or KT can change it without code.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct NpcRules {
    /// NPCs a run starts with, for Relaxed, Standard and Hard.
    pub per_run: [i32; 3],
    /// No run holds more than this many.
    pub max: i32,
    /// An NPC's home is this far from the runner at least, in metres.
    pub min_distance: f32,
    /// ... and at most this far.
    pub max_distance: f32,
    /// A waiting NPC joins the runner once the runner is this close and in sight.
    pub join_radius: f32,
    /// The smartness values an NPC draws from, 0..100.
    pub smart: Vec<i32>,
}

impl Default for NpcRules {
    fn default() -> NpcRules {
        NpcRules {
            per_run: [1, 1, 2],
            max: 4,
            min_distance: 6.0,
            max_distance: 20.0,
            join_radius: 6.0,
            smart: vec![30, 50, 70, 90],
        }
    }
}

impl NpcRules {
    pub fn parse(v: &Value) -> NpcRules {
        let mut r = NpcRules::default();
        let s = &v["spawn"];
        for (i, k) in ["relaxed", "standard", "hard"].iter().enumerate() {
            if let Some(n) = s["per_run"][k].as_i64() {
                r.per_run[i] = n.clamp(0, 64) as i32;
            }
        }
        if let Some(n) = s["max"].as_i64() {
            r.max = n.clamp(0, 64) as i32;
        }
        for (key, slot) in [
            ("min_distance", &mut r.min_distance),
            ("max_distance", &mut r.max_distance),
            ("join_radius", &mut r.join_radius),
        ] {
            if let Some(x) = s[key].as_f64() {
                *slot = (x as f32).clamp(1.0, 200.0);
            }
        }
        if r.max_distance < r.min_distance {
            r.max_distance = r.min_distance;
        }
        let smart: Vec<i32> = v["smart"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_i64()).map(|n| n.clamp(0, 100) as i32).collect())
            .unwrap_or_default();
        if !smart.is_empty() {
            r.smart = smart;
        }
        r
    }

    /// The NPCs a run on `difficulty` starts with.
    pub fn count(&self, difficulty: crate::config::Difficulty) -> i32 {
        let i = match difficulty {
            crate::config::Difficulty::Relaxed => 0,
            crate::config::Difficulty::Standard => 1,
            crate::config::Difficulty::Hard => 2,
        };
        self.per_run[i].min(self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;

    #[test]
    fn the_embedded_file_parses_to_the_defaults() {
        let (_, text) = crate::content::embedded::data_under("npcs.json").next().expect("data/npcs.json is embedded");
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(NpcRules::parse(&v), NpcRules::default());
    }

    #[test]
    fn counts_follow_difficulty_and_the_cap() {
        let v: Value = serde_json::from_str(r#"{"spawn":{"per_run":{"relaxed":0,"standard":3,"hard":9},"max":5}}"#).unwrap();
        let r = NpcRules::parse(&v);
        assert_eq!(r.count(Difficulty::Relaxed), 0);
        assert_eq!(r.count(Difficulty::Standard), 3);
        assert_eq!(r.count(Difficulty::Hard), 5);
    }
}
