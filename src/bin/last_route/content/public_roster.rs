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
//!
//! The "messages" block holds the player-facing PUBLIC route notices, and the "limits" block the join and
//! session timings that mirror the server (lr_server rooms/public_route.rs PUBLIC_JOIN_GRACE_MS,
//! JOIN_LIMIT_PER_MINUTE, PUBLIC_PLAYER_TTL_MS, PUBLIC_RESUME_TTL_MS). The server stays authoritative; these
//! values let the client show the same notices and pace its own requests.

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

/// The player-facing PUBLIC route notices (the "messages" block). Shown as plain title-screen notices.
#[derive(Clone, Debug, PartialEq)]
pub struct PublicMessages {
    /// Shown when the PUBLIC ROUTE button cannot start the route in this build.
    pub unavailable: String,
    /// Shown while the client joins the persistent route.
    pub joining: String,
    /// Shown once the route has been started or joined.
    pub started: String,
    /// Shown when the server refuses a join for its per-minute limit (the server's own 429 text).
    pub join_rate_limited: String,
}

impl Default for PublicMessages {
    fn default() -> PublicMessages {
        PublicMessages {
            unavailable: "PUBLIC ROUTE IS NOT AVAILABLE IN THIS BUILD YET".to_string(),
            joining: "JOINING PUBLIC ROUTE".to_string(),
            started: "PUBLIC ROUTE STARTED".to_string(),
            join_rate_limited: "TOO MANY JOIN REQUESTS".to_string(),
        }
    }
}

impl PublicMessages {
    /// Reads the "messages" object. A missing or non-text key keeps its Standard text.
    pub fn parse(v: &Value) -> PublicMessages {
        let mut m = PublicMessages::default();
        if let Some(s) = v["unavailable"].as_str() {
            m.unavailable = s.to_string();
        }
        if let Some(s) = v["joining"].as_str() {
            m.joining = s.to_string();
        }
        if let Some(s) = v["started"].as_str() {
            m.started = s.to_string();
        }
        if let Some(s) = v["join_rate_limited"].as_str() {
            m.join_rate_limited = s.to_string();
        }
        m
    }
}

/// Join and session timings for the PUBLIC route (the "limits" block). Mirrors the server constants.
#[derive(Clone, Debug, PartialEq)]
pub struct PublicLimits {
    /// Milliseconds a freshly joined runner is protected before it can be dropped (server: 10 000).
    pub join_grace_msec: u64,
    /// Join requests allowed per minute before the server refuses (server: 20).
    pub join_per_minute: u32,
    /// Milliseconds without a heartbeat before a runner is dropped from the route (server: 20 000).
    pub player_ttl_msec: u64,
    /// Milliseconds a departed runner can resume the same place on the route (server: 6 hours).
    pub resume_ttl_msec: u64,
}

impl Default for PublicLimits {
    fn default() -> PublicLimits {
        PublicLimits {
            join_grace_msec: 10_000,
            join_per_minute: 20,
            player_ttl_msec: 20_000,
            resume_ttl_msec: 6 * 60 * 60 * 1000,
        }
    }
}

impl PublicLimits {
    /// Reads the "limits" object. A missing or non-numeric key keeps its Standard value.
    pub fn parse(v: &Value) -> PublicLimits {
        let mut l = PublicLimits::default();
        if let Some(ms) = v["join_grace_msec"].as_u64() {
            l.join_grace_msec = ms;
        }
        if let Some(n) = v["join_per_minute"].as_u64() {
            l.join_per_minute = n.min(u32::MAX as u64) as u32;
        }
        if let Some(ms) = v["player_ttl_msec"].as_u64() {
            l.player_ttl_msec = ms;
        }
        if let Some(ms) = v["resume_ttl_msec"].as_u64() {
            l.resume_ttl_msec = ms;
        }
        l
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
    /// Player-facing PUBLIC route notices (the "messages" block of data/public_roster.json).
    pub messages: PublicMessages,
    /// Join and session timings (the "limits" block of data/public_roster.json).
    pub limits: PublicLimits,
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
            messages: PublicMessages::default(),
            limits: PublicLimits::default(),
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
        if v["messages"].is_object() {
            r.messages = PublicMessages::parse(&v["messages"]);
        }
        if v["limits"].is_object() {
            r.limits = PublicLimits::parse(&v["limits"]);
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
    let now = crate::shared::clock::SystemTime::now()
        .duration_since(crate::shared::clock::UNIX_EPOCH)
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
        assert!((r.weight_of("easy") - 0.5_f64).abs() < 1e-4);
        assert!((r.weight_of("intermediate") - 0.4_f64).abs() < 1e-4);
        assert!((r.weight_of("difficult") - 0.1_f64).abs() < 1e-4);
        assert!((r.weight_of("unknown-tier") - 0.3_f64).abs() < 1e-4);
        assert_eq!(r.featured_count, 2);
        assert_eq!(r.players_per_extra_hunter, 2);
        assert_eq!(r.cap, 8);
    }

    #[test]
    fn standard_patrol_matches_godot() {
        let p = PublicRoster::default().patrol;
        assert!((p.radius - 30.0_f32).abs() < 1e-4);
        assert!((p.fallback_radius_scale - 2.0_f32).abs() < 1e-4);
        assert_eq!(p.dwell_msec, 20_000);
        assert!((p.speed_scale - 1.0_f32).abs() < 1e-4);
    }

    #[test]
    fn changing_the_patrol_block_changes_the_parsed_values() {
        let r = parse(r#"{"patrol":{"radius":12.5,"fallback_radius_scale":3,"dwell_msec":5000,"speed_scale":0.5}}"#);
        assert!((r.patrol.radius - 12.5_f32).abs() < 1e-4);
        assert!((r.patrol.fallback_radius_scale - 3.0_f32).abs() < 1e-4);
        assert_eq!(r.patrol.dwell_msec, 5_000);
        assert!((r.patrol.speed_scale - 0.5_f32).abs() < 1e-4);
        // The rest of the roster keeps its Standard values.
        assert_eq!(r.cap, 8);
        // A missing or broken key keeps the Standard patrol.
        assert_eq!(parse(r#"{"patrol":"nope"}"#).patrol, PublicPatrol::default());
    }

    #[test]
    fn standard_messages_match_the_title_and_server_text() {
        let m = PublicRoster::default().messages;
        assert_eq!(m.unavailable, "PUBLIC ROUTE IS NOT AVAILABLE IN THIS BUILD YET");
        assert_eq!(m.joining, "JOINING PUBLIC ROUTE");
        assert_eq!(m.started, "PUBLIC ROUTE STARTED");
        assert_eq!(m.join_rate_limited, "TOO MANY JOIN REQUESTS");
    }

    #[test]
    fn changing_a_message_changes_only_that_text() {
        let r = parse(r#"{"messages":{"joining":"CONNECTING","started":""}}"#);
        assert_eq!(r.messages.joining, "CONNECTING");
        assert_eq!(r.messages.started, "");
        assert_eq!(r.messages.unavailable, PublicMessages::default().unavailable);
        assert_eq!(r.messages.join_rate_limited, PublicMessages::default().join_rate_limited);
        // A non-text value keeps the Standard text.
        assert_eq!(parse(r#"{"messages":{"joining":5}}"#).messages.joining, "JOINING PUBLIC ROUTE");
        // A broken block keeps every Standard message.
        assert_eq!(parse(r#"{"messages":"nope"}"#).messages, PublicMessages::default());
    }

    #[test]
    fn standard_limits_match_the_server_constants() {
        let l = PublicRoster::default().limits;
        assert_eq!(l.join_grace_msec, 10_000);
        assert_eq!(l.join_per_minute, 20);
        assert_eq!(l.player_ttl_msec, 20_000);
        assert_eq!(l.resume_ttl_msec, 6 * 60 * 60 * 1000);
    }

    #[test]
    fn changing_a_limit_changes_the_parsed_value() {
        let r = parse(r#"{"limits":{"join_per_minute":5,"join_grace_msec":0,"resume_ttl_msec":1000}}"#);
        assert_eq!(r.limits.join_per_minute, 5);
        assert_eq!(r.limits.join_grace_msec, 0);
        assert_eq!(r.limits.resume_ttl_msec, 1_000);
        // Untouched limits keep their Standard values.
        assert_eq!(r.limits.player_ttl_msec, 20_000);
        assert_eq!(parse(r#"{"limits":{"join_per_minute":"many"}}"#).limits, PublicLimits::default());
        assert_eq!(parse(r#"{"limits":[1,2]}"#).limits, PublicLimits::default());
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
        assert!((r.weight_of("easy") - 0.0_f64).abs() < 1e-4);
        assert!((r.weight_of("difficult") - 0.0_f64).abs() < 1e-4);
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
