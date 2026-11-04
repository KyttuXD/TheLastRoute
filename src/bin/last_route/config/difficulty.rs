//! DIFFICULTY_PROFILES, HUNTER_ACTIVATION_RELAYS and the tier weights of the hunter draw
//! (game/config/game_config.gd, game/config/hunter_roster.gd). Nightmare is listed but, as in
//! the original's room staging, greyed out.

use super::{TASER_SLOW_FACTOR, TASER_SLOW_SECONDS};
use crate::content::embedded;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Difficulty {
    Relaxed,
    Standard,
    Hard,
    Nightmare,
}

#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub player_speed: f32,
    pub crouch_speed: f32,
    pub sprint_speed: f32,
    pub lamp_drain: f32,
    pub sprint_drain: f32,
    pub hunter_speed: f32,
    pub hunter_sight: f32,
    pub hunter_hearing: f32,
    pub capture_distance: f32,
    pub search_time: f32,
    pub lost_sight_grace: f32,
    pub stamina_drain: f32,
    pub hide_tell: f32,
    pub hide_trail: f32,
    pub hide_cooldown: f32,
    pub hide_random: f32,
    pub taser_slow_factor: f32,
    pub taser_slow_seconds: f32,
    pub taser_cooldown: f32,
    pub escape_stun: f32,
    pub crowbar_stun: f32,
    pub axe_stun: f32,
    pub relay_alert_radius: f32,
    pub relay_alert_chance: f32,
    pub camp_limit: f32,
    pub activation_relays: i32,
}

/// The three difficulties the room wire format round-trips. `net/world_sync.rs` `difficulty_from_name`
/// walks this list and has no NIGHTMARE arm yet, so Nightmare is kept out of it (see TODO.md).
pub const ALL: [Difficulty; 3] = [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard];

/// The lobby's difficulty row, in order: Relaxed, Standard, Hard, Nightmare.
pub const LOBBY: [Difficulty; 4] = [
    Difficulty::Relaxed,
    Difficulty::Standard,
    Difficulty::Hard,
    Difficulty::Nightmare,
];

/// TIER_WEIGHTS_BY_DIFFICULTY["nightmare"]: easy, intermediate, difficult. Kept for callers that
/// name it; the live value is data/hunters/draw_pool.json (test `draw_pool_drives_tier_weights`).
pub const NIGHTMARE_TIER_WEIGHTS: [f64; 3] = [0.175, 0.525, 0.30];

/// Used only when data/hunters/draw_pool.json is missing or unreadable: an even split. The
/// embedded file always loads (see the tests), so this never draws in a shipped build.
const UNIFORM_TIER_WEIGHTS: [f64; 3] = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0];

/// Path of the hunter draw weights inside the embedded data (data/hunters/draw_pool.json).
const DRAW_POOL_PATH: &str = "hunters/draw_pool.json";

/// Parse `tier_weights` from a draw_pool document into id -> [easy, intermediate, difficult].
/// A row with a missing or non-numeric tier is skipped.
pub fn parse_draw_pool(v: &serde_json::Value) -> HashMap<String, [f64; 3]> {
    let mut out = HashMap::new();
    if let Some(rows) = v["tier_weights"].as_object() {
        for (id, row) in rows {
            let w = ["easy", "intermediate", "difficult"].map(|t| row[t].as_f64());
            if let [Some(a), Some(b), Some(c)] = w {
                out.insert(id.clone(), [a, b, c]);
            }
        }
    }
    out
}

/// The draw weights for one difficulty id, from a parsed draw pool.
pub fn weights_from(pool: &HashMap<String, [f64; 3]>, id: &str) -> Option<[f64; 3]> {
    pool.get(id).copied()
}

/// The embedded draw pool, parsed once.
fn draw_pool() -> &'static HashMap<String, [f64; 3]> {
    static POOL: OnceLock<HashMap<String, [f64; 3]>> = OnceLock::new();
    POOL.get_or_init(|| {
        embedded::data_under(DRAW_POOL_PATH)
            .next()
            .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
            .map(|v| parse_draw_pool(&v))
            .unwrap_or_default()
    })
}

/// TIER_WEIGHTS_BY_DIFFICULTY.get(id, standard): the hunter draw weights for any difficulty id,
/// Nightmare included; an unknown id draws as Standard.
pub fn tier_weights_for(id: &str) -> [f64; 3] {
    LOBBY
        .iter()
        .find(|d| d.id() == id)
        .map_or(Difficulty::Standard.tier_weights(), |d| d.tier_weights())
}

/// DIFFICULTY_PROFILES["nightmare"] (game/config/game_config.gd:50-72). The Godot block has no
/// activation_relays entry, so activation is 0 (`Difficulty::profile` returns this for Nightmare).
pub fn nightmare_profile() -> Profile {
    Profile {
        player_speed: 0.94,
        crouch_speed: 0.94,
        sprint_speed: 0.96,
        lamp_drain: 1.35,
        sprint_drain: 1.20,
        hunter_speed: 1.17,
        hunter_sight: 1.22,
        hunter_hearing: 1.25,
        capture_distance: 1.08,
        search_time: 5.0,
        lost_sight_grace: 4.5,
        stamina_drain: 10.0,
        hide_tell: 1.0,
        hide_trail: 4.0,
        hide_cooldown: 12.0,
        hide_random: 0.45,
        taser_slow_factor: TASER_SLOW_FACTOR,
        taser_slow_seconds: TASER_SLOW_SECONDS,
        taser_cooldown: 10.0,
        escape_stun: 2.2,
        crowbar_stun: 3.3,
        axe_stun: 4.4,
        relay_alert_radius: 30.0,
        relay_alert_chance: 1.0,
        camp_limit: 15.0,
        activation_relays: 0,
    }
}

/// SAFE_ZONE_LIMITS_BY_DIFFICULTY["nightmare"].
pub const NIGHTMARE_SAFE_ZONE_LIMIT: i32 = 0;

impl Difficulty {
    pub fn id(self) -> &'static str {
        match self {
            Difficulty::Relaxed => "relaxed",
            Difficulty::Standard => "standard",
            Difficulty::Hard => "hard",
            Difficulty::Nightmare => "nightmare",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Relaxed => "RELAXED",
            Difficulty::Standard => "STANDARD",
            Difficulty::Hard => "HARD",
            Difficulty::Nightmare => "NIGHTMARE",
        }
    }

    /// The staging blurb under the difficulty buttons (room_lobby.gd).
    pub fn blurb(self) -> &'static str {
        match self {
            Difficulty::Relaxed => "Room to breathe: hunters wake late and tire early.",
            Difficulty::Standard => "The route as meant: hunters work together, so keep moving.",
            Difficulty::Hard => "Hunters wake at once and hear more. Spend your light well.",
            Difficulty::Nightmare => "Hunters are faster and sharper, and the torch burns down quicker.",
        }
    }

    pub fn profile(self) -> Profile {
        match self {
            Difficulty::Nightmare => nightmare_profile(),
            Difficulty::Relaxed => Profile {
                player_speed: 1.08,
                crouch_speed: 1.06,
                sprint_speed: 1.08,
                lamp_drain: 0.70,
                sprint_drain: 0.80,
                hunter_speed: 0.82,
                hunter_sight: 0.86,
                hunter_hearing: 0.86,
                capture_distance: 0.90,
                search_time: 3.2,
                lost_sight_grace: 3.0,
                stamina_drain: 14.0,
                hide_tell: 1.6,
                hide_trail: 3.0,
                hide_cooldown: 20.0,
                hide_random: 0.0,
                taser_slow_factor: TASER_SLOW_FACTOR,
                taser_slow_seconds: TASER_SLOW_SECONDS,
                taser_cooldown: 8.0,
                escape_stun: 3.0,
                crowbar_stun: 4.5,
                axe_stun: 6.0,
                relay_alert_radius: 0.0,
                relay_alert_chance: 0.0,
                camp_limit: 6.0,
                activation_relays: 3,
            },
            Difficulty::Standard => Profile {
                player_speed: 1.0,
                crouch_speed: 1.0,
                sprint_speed: 1.0,
                lamp_drain: 1.0,
                sprint_drain: 1.0,
                hunter_speed: 1.0,
                hunter_sight: 1.02,
                hunter_hearing: 1.03,
                capture_distance: 1.0,
                search_time: 3.4,
                lost_sight_grace: 3.2,
                stamina_drain: 13.5,
                hide_tell: 1.5,
                hide_trail: 3.2,
                hide_cooldown: 19.0,
                hide_random: 0.05,
                taser_slow_factor: TASER_SLOW_FACTOR,
                taser_slow_seconds: TASER_SLOW_SECONDS,
                taser_cooldown: 8.5,
                escape_stun: 2.9,
                crowbar_stun: 4.35,
                axe_stun: 5.8,
                relay_alert_radius: 8.0,
                relay_alert_chance: 0.25,
                camp_limit: 9.0,
                activation_relays: 1,
            },
            Difficulty::Hard => Profile {
                player_speed: 0.97,
                crouch_speed: 0.97,
                sprint_speed: 0.98,
                lamp_drain: 1.175,
                sprint_drain: 1.10,
                hunter_speed: 1.085,
                hunter_sight: 1.1,
                hunter_hearing: 1.12,
                capture_distance: 1.04,
                search_time: 4.0,
                lost_sight_grace: 3.6,
                stamina_drain: 12.0,
                hide_tell: 1.3,
                hide_trail: 3.5,
                hide_cooldown: 16.0,
                hide_random: 0.25,
                taser_slow_factor: TASER_SLOW_FACTOR,
                taser_slow_seconds: TASER_SLOW_SECONDS,
                taser_cooldown: 9.0,
                escape_stun: 2.6,
                crowbar_stun: 3.9,
                axe_stun: 5.2,
                relay_alert_radius: 18.0,
                relay_alert_chance: 1.0,
                camp_limit: 12.0,
                activation_relays: 0,
            },
        }
    }

    /// TIER_WEIGHTS_BY_DIFFICULTY: easy, intermediate, difficult. Read from data/hunters/draw_pool.json.
    pub fn tier_weights(self) -> [f64; 3] {
        weights_from(draw_pool(), self.id()).unwrap_or(UNIFORM_TIER_WEIGHTS)
    }

    /// SAFE_ZONE_LIMITS_BY_DIFFICULTY: >0 puts the safe zone at the spawn.
    pub fn safe_zone_limit(self) -> i32 {
        match self {
            Difficulty::Relaxed => 3,
            Difficulty::Standard => 2,
            Difficulty::Hard => 1,
            Difficulty::Nightmare => NIGHTMARE_SAFE_ZONE_LIMIT,
        }
    }
}

#[cfg(test)]
mod nightmare_tests {
    use super::*;

    /// The Godot nightmare block (game_config.gd:50-72) must match the port numbers exactly.
    #[test]
    fn nightmare_profile_matches_godot_numbers() {
        let p = nightmare_profile();
        assert_eq!(p.player_speed, 0.94);
        assert_eq!(p.crouch_speed, 0.94);
        assert_eq!(p.sprint_speed, 0.96);
        assert_eq!(p.lamp_drain, 1.35);
        assert_eq!(p.sprint_drain, 1.20);
        assert_eq!(p.hunter_speed, 1.17);
        assert_eq!(p.hunter_sight, 1.22);
        assert_eq!(p.hunter_hearing, 1.25);
        assert_eq!(p.capture_distance, 1.08);
        assert_eq!(p.search_time, 5.0);
        assert_eq!(p.lost_sight_grace, 4.5);
        assert_eq!(p.stamina_drain, 10.0);
        assert_eq!(p.hide_tell, 1.0);
        assert_eq!(p.hide_trail, 4.0);
        assert_eq!(p.hide_cooldown, 12.0);
        assert_eq!(p.hide_random, 0.45);
        assert_eq!(p.taser_cooldown, 10.0);
        assert_eq!(p.escape_stun, 2.2);
        assert_eq!(p.crowbar_stun, 3.3);
        assert_eq!(p.axe_stun, 4.4);
        assert_eq!(p.relay_alert_radius, 30.0);
        assert_eq!(p.relay_alert_chance, 1.0);
        assert_eq!(p.camp_limit, 15.0);
        assert_eq!(NIGHTMARE_SAFE_ZONE_LIMIT, 0);
    }

    /// Standard and Hard profiles are untouched by the Nightmare addition.
    #[test]
    fn standard_and_hard_profiles_are_unchanged() {
        assert_eq!(Difficulty::Standard.profile().hunter_speed, 1.0);
        assert_eq!(Difficulty::Standard.profile().search_time, 3.4);
        assert_eq!(Difficulty::Hard.profile().hunter_speed, 1.085);
        assert_eq!(Difficulty::Hard.profile().camp_limit, 12.0);
    }

    /// Nightmare is in the lobby row and its profile is the Godot block when chosen.
    #[test]
    fn nightmare_is_selectable_and_applies_its_profile() {
        assert!(LOBBY.contains(&Difficulty::Nightmare));
        assert_eq!(Difficulty::Nightmare.id(), "nightmare");
        assert_eq!(Difficulty::Nightmare.name(), "NIGHTMARE");
        assert_eq!(Difficulty::Nightmare.profile().player_speed, 0.94);
        assert_eq!(Difficulty::Nightmare.profile().hunter_speed, 1.17);
        assert_eq!(Difficulty::Nightmare.profile().activation_relays, 0);
        assert_eq!(Difficulty::Nightmare.safe_zone_limit(), 0);
        assert_eq!(Difficulty::Nightmare.tier_weights(), [0.175, 0.525, 0.30]);
        assert_eq!(tier_weights_for("nightmare"), [0.175, 0.525, 0.30]);
    }

    /// The room wire format (`ALL`) still lists three difficulties; Nightmare is lobby-only until
    /// net/world_sync.rs learns its name.
    #[test]
    fn room_wire_list_excludes_nightmare_until_net_maps_it() {
        assert_eq!(ALL.len(), 3);
        assert!(!ALL.contains(&Difficulty::Nightmare));
    }
}

#[cfg(test)]
mod draw_pool_tests {
    use super::*;
    use serde_json::json;

    fn embedded_pool() -> HashMap<String, [f64; 3]> {
        draw_pool().clone()
    }

    /// The shipped data file loads, so the even-split fallback never draws in a real build.
    #[test]
    fn embedded_draw_pool_loads_every_difficulty() {
        let pool = embedded_pool();
        for d in LOBBY {
            assert!(pool.contains_key(d.id()), "{} missing from draw_pool.json", d.id());
        }
    }

    /// Standard values in the data match the game (Godot hunter_roster.gd 171-174).
    #[test]
    fn standard_and_relaxed_weights_match_the_game() {
        assert_eq!(Difficulty::Standard.tier_weights(), [0.35, 0.50, 0.15]);
        assert_eq!(Difficulty::Relaxed.tier_weights(), [0.70, 0.25, 0.05]);
        assert_eq!(Difficulty::Hard.tier_weights(), [0.2625, 0.5125, 0.225]);
        assert_eq!(Difficulty::Nightmare.tier_weights(), NIGHTMARE_TIER_WEIGHTS);
    }

    /// Changing a value in the data changes the weights the draw uses.
    #[test]
    fn changing_a_data_value_changes_the_weights() {
        let doc = json!({ "tier_weights": {
            "standard": { "easy": 0.2, "intermediate": 0.3, "difficult": 0.5 }
        }});
        let pool = parse_draw_pool(&doc);
        assert_eq!(weights_from(&pool, "standard"), Some([0.2, 0.3, 0.5]));
        assert_eq!(weights_from(&pool, "hard"), None, "an id absent from the data has no row");
        assert_ne!(
            weights_from(&pool, "standard"),
            Some(Difficulty::Standard.tier_weights()),
            "a retuned file must not fall back to the shipped numbers"
        );
    }

    /// A row with a missing or non-numeric tier is dropped rather than read as zero.
    #[test]
    fn broken_rows_are_skipped() {
        let doc = json!({ "tier_weights": {
            "relaxed": { "easy": 1.0, "intermediate": 0.0 },
            "hard": { "easy": "x", "intermediate": 0.5, "difficult": 0.5 }
        }});
        let pool = parse_draw_pool(&doc);
        assert!(pool.is_empty());
    }

    /// The Nightmare const kept for callers matches its data row.
    #[test]
    fn draw_pool_drives_tier_weights() {
        assert_eq!(
            weights_from(&embedded_pool(), "nightmare"),
            Some(NIGHTMARE_TIER_WEIGHTS)
        );
        for d in ALL {
            assert_eq!(d.tier_weights(), weights_from(&embedded_pool(), d.id()).unwrap());
        }
    }
}
