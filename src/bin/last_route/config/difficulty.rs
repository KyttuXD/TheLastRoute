//! DIFFICULTY_PROFILES, HUNTER_ACTIVATION_RELAYS and the tier weights of the hunter draw
//! (game/config/game_config.gd, game/config/hunter_roster.gd). Nightmare is listed but, as in
//! the original's room staging, greyed out.

use super::{TASER_SLOW_FACTOR, TASER_SLOW_SECONDS};

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

/// TIER_WEIGHTS_BY_DIFFICULTY["nightmare"]: easy, intermediate, difficult.
pub const NIGHTMARE_TIER_WEIGHTS: [f64; 3] = [0.175, 0.525, 0.30];

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

    /// TIER_WEIGHTS_BY_DIFFICULTY: easy, intermediate, difficult.
    pub fn tier_weights(self) -> [f64; 3] {
        match self {
            Difficulty::Relaxed => [0.70, 0.25, 0.05],
            Difficulty::Standard => [0.35, 0.50, 0.15],
            Difficulty::Hard => [0.2625, 0.5125, 0.225],
            Difficulty::Nightmare => NIGHTMARE_TIER_WEIGHTS,
        }
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
