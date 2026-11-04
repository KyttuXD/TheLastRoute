//! `lr.public_roster/1` (data/public_roster.json): the hunter lineup of the persistent PUBLIC route.
//! Godot twin: game/config/public_roster.gd (WEIGHTS L9, FEATURED_COUNT L10, PLAYERS_PER_EXTRA_HUNTER L11,
//! CAP L12, day_key L16, target_count L22, draw L34, featured L54, lineup L60).
//!
//! The UTC day picks a stable featured pair (the same for every client that day). The rest of the lineup is a
//! tier-weighted draw without replacement, mixed with the day. The hunter count is 1 + one per two players,
//! clamped to 1..cap. Every weight and cap is data, so a pack or KT can change them without code.
//!
//! The "patrol" block is the PUBLIC route patrol around a live runner (Godot twin: game/hunters/ai/hunter_brain.gd
//! PATROL_RADIUS L142 and _pick_wander_target L1120-1126; game/infinite_route/infinite_route_test.gd
//! PUBLIC_PATROL_DWELL_MSEC L5869 and _assign_public_patrol L5874). The behaviour lives in app/public_route.rs.

use crate::world::rng::Rng;
use serde_json::Value;
use std::collections::HashMap;

/// Patrol around a live runner: how far from the anchor a wandering hunter stays, the wider fallback radius, how
/// long one runner is patrolled before the choice is re-rolled, and a speed multiplier for patrolling hunters.
#[derive(Clone, Debug, PartialEq)]
pub struct PublicPatrol {
    /// Wander candidates within this many cells of the runner are preferred (Godot: PATROL_RADIUS 30.0).
    pub radius: f32,
    /// If no candidate is inside `radius`, this multiple of it is tried next (Godot: PATROL_RADIUS * 2.0).
    pub fallback_radius_scale: f32,
    /// Milliseconds one runner stays the anchor before the hunter re-rolls (Godot: PUBLIC_PATROL_DWELL_MSEC 20000).
    pub dwell_msec: u64,
    /// Multiplier on a hunter's wander speed while it patrols an anchor (Standard 1.0: no change).
    pub speed_scale: f32,
}

impl Default for PublicPatrol {
    fn default() -> PublicPatrol {
        PublicPatrol { radius: 30.0, fallback_radius_scale: 2.0, dwell_msec: 20_000, speed_scale: 1.0 }
    }
}

impl PublicPatrol {
    /// Reads the "patrol" object. A missing key keeps its Standard value.
    pub fn parse(v: &Value) -> PublicPatrol {
        let mut p = PublicPatrol::default();
        if let Some(r) = v["radius"].as_f64() {
            p.radius = r.max(0.0) as f32;
        }
        if let Some(s) = v["fallback_radius_scale"].as_f64() {
            p.fallback_radius_scale = s.max(1.0) as f32;
        }
        if let Some(ms) = v["dwell_msec"].as_u64() {
            p.dwell_msec = ms;
        }
        if let Some(s) = v["speed_scale"].as_f64() {
            p.speed_scale = s.max(0.0) as f32;
        }
        p
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublicRoster {
    /// Draw weight per hunter tier name ("easy", "intermediate", "difficult").
    pub weights: HashMap<String, f64>,
    /// Weight for a tier that has no entry in `weights` (Godot: `.3`).
    pub default_weight: f64,
    /// Hunters fixed to the day, at the front of the lineup (Godot: FEATURED_COUNT).
    pub featured_count: usize,
    /// One extra hunter per this many players (Godot: PLAYERS_PER_EXTRA_HUNTER).
    pub players_per_extra_hunter: usize,
    /// No lineup holds more than this many hunters (Godot: CAP).
    pub cap: usize,
    /// Patrol around a live runner (the "patrol" block of data/public_roster.json).
    pub patrol: PublicPatrol,
}

impl Default for PublicRoster {
    fn default() -> PublicRoster {
        let weights = [("easy", 0.5), ("intermediate", 0.4), ("difficult", 0.1)]
            .iter()
            .map(|(k, w)| (k.to_string(), *w))
            .collect();
        PublicRoster {
            weights,
            default_weight: 0.3,
            featured_count: 2,
            players_per_extra_hunter: 2,
            cap: 8,
            patrol: PublicPatrol::default(),
        }
    }
}

impl PublicRoster {
    /// Reads data/public_roster.json. A missing key keeps its Standard value.
    pub fn parse(v: &Value) -> PublicRoster {
        let mut r = PublicRoster::default();
        if let Some(obj) = v["weights"].as_object() {
            r.weights = obj
                .iter()
                .filter_map(|(k, x)| x.as_f64().map(|w| (k.clone(), w.max(0.0))))
                .collect();
        }
        if let Some(w) = v["default_weight"].as_f64() {
            r.default_weight = w.max(0.0);
        }
        if let Some(n) = v["featured_count"].as_u64() {
            r.featured_count = n.min(64) as usize;
        }
        if let Some(n) = v["players_per_extra_hunter"].as_u64() {
            r.players_per_extra_hunter = n.clamp(1, 64) as usize;
        }
        if let Some(n) = v["cap"].as_u64() {
            r.cap = n.clamp(1, 64) as usize;
        }
        if v["patrol"].is_object() {
            r.patrol = PublicPatrol::parse(&v["patrol"]);
        }
        r
    }

    /// The embedded data/public_roster.json, or the Standard values if it is missing or broken.
    pub fn load() -> PublicRoster {
        crate::content::embedded::data_under("public_roster.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
            .map(|v| PublicRoster::parse(&v))
            .unwrap_or_default()
    }

    /// The draw weight of one tier.
    pub fn weight_of(&self, tier: &str) -> f64 {
        self.weights.get(tier).copied().unwrap_or(self.default_weight)
    }

    /// 1 + floor((players - 1) / per), clamped to 1..=cap: 1-2 players run 1 hunter, 3-4 run 2.
    pub fn target_count(&self, players: usize) -> usize {
        (1 + (players.max(1) - 1) / self.players_per_extra_hunter).clamp(1, self.cap)
    }

    /// Weighted draw without replacement from `pool`, seeded by `seed` (same as Godot draw, L34).
    fn draw<'a>(
        &self,
        seed: &str,
        pool: &[&'a str],
        count: usize,
        tier_of: &dyn Fn(&str) -> &'static str,
    ) -> Vec<&'a str> {
        let mut rng = Rng::new(seed);
        let mut remaining: Vec<&'a str> = pool.to_vec();
        let mut picked: Vec<&'a str> = Vec::new();
        while picked.len() < count && !remaining.is_empty() {
            let weights: Vec<f64> = remaining.iter().map(|id| self.weight_of(tier_of(*id))).collect();
            let total: f64 = weights.iter().sum();
            let mut roll = rng.next_float() * total;
            let mut chosen = remaining.len() - 1;
            for (index, w) in weights.iter().enumerate() {
                roll -= w;
                if roll <= 0.0 {
                    chosen = index;
                    break;
                }
            }
            picked.push(remaining.remove(chosen));
        }
        picked
    }

    /// The day's featured pair. Depends on the date alone, so every client agrees (Godot featured, L54).
    pub fn featured<'a>(&self, day: &str, pool: &[&'a str], tier_of: &dyn Fn(&str) -> &'static str) -> Vec<&'a str> {
        self.draw(&format!("PUBLIC-FEATURED:{day}"), pool, self.featured_count, tier_of)
    }

    /// The full lineup: the featured pair first, then the day-mixed weighted draw of everyone else (Godot lineup, L60).
    pub fn lineup<'a>(
        &self,
        seed: &str,
        day: &str,
        pool: &[&'a str],
        tier_of: &dyn Fn(&str) -> &'static str,
    ) -> Vec<&'a str> {
        let first = self.featured(day, pool, tier_of);
        let rest: Vec<&'a str> = pool.iter().copied().filter(|id| !first.contains(id)).collect();
        let count = rest.len();
        let mut out = first;
        out.extend(self.draw(&format!("{seed}:DAY-{day}:LINEUP"), &rest, count, tier_of));
        out
    }

    /// The hunters a public route with `players` online runs: the first `target_count` of the lineup.
    pub fn roster<'a>(
        &self,
        seed: &str,
        day: &str,
        players: usize,
        pool: &[&'a str],
        tier_of: &dyn Fn(&str) -> &'static str,
    ) -> Vec<&'a str> {
        let n = self.target_count(players);
        self.lineup(seed, day, pool, tier_of).into_iter().take(n).collect()
    }
}

/// "YYYY-MM-DD" in UTC for a Unix time in seconds (Godot day_key, L16). The lineup reshuffles at 00:00 UTC.
pub fn day_key(unix_seconds: i64) -> String {
    let (y, m, d) = civil_from_days(unix_seconds.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Today's UTC day key from the system clock.
pub fn today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    day_key(now)
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The streamed hunter pool and each one's tier, from data/hunters/*.json (the "tier" field).
    const POOL: &[&str] = crate::world::route::prop_tables::STREAMED_HUNTER_IDS;

    fn tier_of(id: &str) -> &'static str {
        match id {
            "elise" | "umbriel" => "difficult",
            "xivvy" | "thessine" | "porcelle" | "verrine" => "intermediate",
            _ => "easy",
        }
    }

    fn parse(text: &str) -> PublicRoster {
        PublicRoster::parse(&serde_json::from_str::<Value>(text).unwrap())
    }

    #[test]
    fn the_embedded_file_parses_to_the_standard_values() {
        let (_, text) = crate::content::embedded::data_under("public_roster.json")
            .next()
            .expect("data/public_roster.json is embedded");
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(PublicRoster::parse(&v), PublicRoster::default());
        assert_eq!(PublicRoster::load(), PublicRoster::default());
    }

    #[test]
    fn standard_weights_and_caps_match_godot() {
        let r = PublicRoster::default();
        assert_eq!(r.weight_of("easy"), 0.5);
        assert_eq!(r.weight_of("intermediate"), 0.4);
        assert_eq!(r.weight_of("difficult"), 0.1);
        assert_eq!(r.weight_of("unknown-tier"), 0.3);
        assert_eq!(r.featured_count, 2);
        assert_eq!(r.players_per_extra_hunter, 2);
        assert_eq!(r.cap, 8);
    }

    #[test]
    fn standard_patrol_matches_godot() {
        let p = PublicRoster::default().patrol;
        assert_eq!(p.radius, 30.0);
        assert_eq!(p.fallback_radius_scale, 2.0);
        assert_eq!(p.dwell_msec, 20_000);
        assert_eq!(p.speed_scale, 1.0);
    }

    #[test]
    fn changing_the_patrol_block_changes_the_parsed_values() {
        let r = parse(r#"{"patrol":{"radius":12.5,"fallback_radius_scale":3,"dwell_msec":5000,"speed_scale":0.5}}"#);
        assert_eq!(r.patrol.radius, 12.5);
        assert_eq!(r.patrol.fallback_radius_scale, 3.0);
        assert_eq!(r.patrol.dwell_msec, 5_000);
        assert_eq!(r.patrol.speed_scale, 0.5);
        // The rest of the roster keeps its Standard values.
        assert_eq!(r.cap, 8);
        // A missing or broken key keeps the Standard patrol.
        assert_eq!(parse(r#"{"patrol":"nope"}"#).patrol, PublicPatrol::default());
    }

    #[test]
    fn hunter_count_is_one_per_two_players_capped() {
        let r = PublicRoster::default();
        assert_eq!(r.target_count(0), 1);
        assert_eq!(r.target_count(1), 1);
        assert_eq!(r.target_count(2), 1);
        assert_eq!(r.target_count(3), 2);
        assert_eq!(r.target_count(4), 2);
        assert_eq!(r.target_count(15), 8);
        assert_eq!(r.target_count(100), 8);
    }

    #[test]
    fn changing_the_cap_or_the_ratio_changes_the_count() {
        let r = parse(r#"{"cap":3,"players_per_extra_hunter":1}"#);
        assert_eq!(r.target_count(1), 1);
        assert_eq!(r.target_count(2), 2);
        assert_eq!(r.target_count(10), 3);
    }

    #[test]
    fn changing_a_weight_changes_who_is_drawn() {
        // Only "difficult" can be drawn: the two difficult hunters must lead the lineup.
        let only_difficult = parse(r#"{"weights":{"easy":0,"intermediate":0,"difficult":1}}"#);
        let mut featured = only_difficult.featured("2026-11-03", POOL, &tier_of);
        featured.sort();
        assert_eq!(featured, vec!["elise", "umbriel"]);
        let standard = PublicRoster::default().featured("2026-11-03", POOL, &tier_of);
        assert_ne!(standard, featured);
    }

    #[test]
    fn changing_the_default_weight_changes_the_draw_for_unlisted_tiers() {
        let r = parse(r#"{"weights":{},"default_weight":0.0}"#);
        assert_eq!(r.weight_of("easy"), 0.0);
        assert_eq!(r.weight_of("difficult"), 0.0);
        assert_eq!(PublicRoster::default().weight_of("easy"), 0.5);
    }

    #[test]
    fn the_featured_pair_leads_the_lineup_and_is_the_same_for_every_seed() {
        let r = PublicRoster::default();
        let day = "2026-11-03";
        let featured = r.featured(day, POOL, &tier_of);
        assert_eq!(featured.len(), 2);
        let a = r.lineup("seed-a", day, POOL, &tier_of);
        let b = r.lineup("seed-b", day, POOL, &tier_of);
        assert_eq!(&a[..2], featured.as_slice());
        assert_eq!(&b[..2], featured.as_slice());
    }

    #[test]
    fn the_lineup_is_a_full_permutation_of_the_pool() {
        let r = PublicRoster::default();
        let lineup = r.lineup("seed", "2026-11-03", POOL, &tier_of);
        assert_eq!(lineup.len(), POOL.len());
        let mut sorted = lineup.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), POOL.len());
    }

    #[test]
    fn the_same_seed_and_day_give_the_same_roster() {
        let r = PublicRoster::default();
        let a = r.roster("lobby-1", "2026-11-03", 5, POOL, &tier_of);
        let b = r.roster("lobby-1", "2026-11-03", 5, POOL, &tier_of);
        assert_eq!(a, b);
        assert_eq!(a.len(), r.target_count(5));
    }

    #[test]
    fn a_new_day_reshuffles_the_non_featured_draw() {
        let r = PublicRoster::default();
        let today = r.lineup("lobby-1", "2026-11-03", POOL, &tier_of);
        let tomorrow = r.lineup("lobby-1", "2026-11-04", POOL, &tier_of);
        assert_ne!(today, tomorrow);
    }

    #[test]
    fn day_key_is_the_utc_date() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(day_key(951_782_400), "2000-02-29");
        assert_eq!(day_key(1_793_664_000), "2026-11-03");
        assert_eq!(day_key(1_793_663_999), "2026-11-02");
    }
}
