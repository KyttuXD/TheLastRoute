//! DIFFICULTY_PROFILES, HUNTER_ACTIVATION_RELAYS and the tier weights of the hunter draw
//! (game/config/game_config.gd, game/config/hunter_roster.gd). Nightmare is listed but, as in
//! the original's room staging, greyed out.

use super::{TASER_SLOW_FACTOR, TASER_SLOW_SECONDS};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Difficulty {
    Relaxed,
    Standard,
    Hard,
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

pub const ALL: [Difficulty; 3] = [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard];

impl Difficulty {
    pub fn id(self) -> &'static str {
        match self {
            Difficulty::Relaxed => "relaxed",
            Difficulty::Standard => "standard",
            Difficulty::Hard => "hard",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Relaxed => "RELAXED",
            Difficulty::Standard => "STANDARD",
            Difficulty::Hard => "HARD",
        }
    }

    /// The staging blurb under the difficulty buttons (room_lobby.gd).
    pub fn blurb(self) -> &'static str {
        match self {
            Difficulty::Relaxed => "Room to breathe: hunters wake late and tire early.",
            Difficulty::Standard => "The route as meant: hunters work together, so keep moving.",
            Difficulty::Hard => "Hunters wake at once and hear more. Spend your light well.",
        }
    }

    pub fn profile(self) -> Profile {
        match self {
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
        }
    }

    /// SAFE_ZONE_LIMITS_BY_DIFFICULTY: >0 puts the safe zone at the spawn.
    pub fn safe_zone_limit(self) -> i32 {
        match self {
            Difficulty::Relaxed => 3,
            Difficulty::Standard => 2,
            Difficulty::Hard => 1,
        }
    }
}
