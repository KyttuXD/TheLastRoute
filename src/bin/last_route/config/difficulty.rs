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

// ---------------------------------------------------------------------------------------------------------------
// Scaling profile and per-level danger (data/danger.json, `lr.danger/1`). Host-side numbers only.

/// Path of the danger data inside the embedded data.
const DANGER_PATH: &str = "danger.json";

/// What a difficulty adds on top of its `Profile`: bigger director teams, peek and inspect odds, the pounce table.
#[derive(Clone, Debug, PartialEq)]
pub struct Scaling {
    /// Extra hunters a director call may send (added to data/hunters/director.json `team`).
    pub team_bonus: i32,
    /// Scale on the peek odds (the `chance` of data/peek.json `difficulty`).
    pub peek_chance: f32,
    /// Scale on the odds a hunter inspects a spot (a locker, a hiding place).
    pub inspect_chance: f32,
    /// Id of the pounce row in data/hard_abilities.json; empty when the difficulty has no hard pounce.
    pub pounce_table: String,
}

/// Danger at one level number of a run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Danger {
    /// Multiplier on the hunter's speed.
    pub speed_scale: f32,
    /// Hunters added to the floor's lineup.
    pub extra_hunters: i32,
    /// Chance a floor offers a safe room, 0..=1.
    pub safe_room_odds: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct DepthRow {
    speed_per_level: f32,
    speed_cap: f32,
    extra_every: i32,
    max_extra: i32,
    safe_room_base: f32,
    safe_room_per_level: f32,
    safe_room_min: f32,
}

/// The HUD danger meter's tunables (data/danger.json `meter`). The route base is a depth/difficulty
/// floor (`Difficulty::route_danger`) a nearby alert hunter spikes toward 100% (`Game::danger`). The
/// `Default` is the shipped block, so an absent/broken `meter` leaves the meter's behaviour unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    /// Depth at which the pure-depth ramp reaches 1.0.
    pub depth_full: f32,
    /// Max route base, leaving headroom for a hunter spike.
    pub base_cap: f32,
    /// Floor so the meter is never empty on a normal run.
    pub base_min: f32,
    pub depth_weight: f32,
    pub speed_weight: f32,
    /// speed_scale that maps to a full speed term.
    pub speed_ref: f32,
    pub hunters_weight: f32,
    /// extra_hunters that maps to a full hunters term.
    pub hunters_ref: f32,
    pub safe_weight: f32,
    /// safe_room_odds treated as the baseline (lost odds = more danger).
    pub safe_ref: f32,
    /// Spike state weight for a Chase hunter.
    pub spike_chase: f32,
    /// Spike state weight for an Investigate/Search hunter.
    pub spike_hunt: f32,
    /// Spike state weight for any other hunter state.
    pub spike_idle: f32,
    /// Cells over which the spike proximity term falls to zero.
    pub spike_range: f32,
    /// Extra spike a hunter at zero distance adds on top of its state weight.
    pub spike_prox: f32,
}

impl Default for Meter {
    fn default() -> Self {
        Meter {
            depth_full: 24.0,
            base_cap: 0.85,
            base_min: 0.05,
            depth_weight: 0.45,
            speed_weight: 0.35,
            speed_ref: 1.25,
            hunters_weight: 0.12,
            hunters_ref: 3.0,
            safe_weight: 0.08,
            safe_ref: 0.6,
            spike_chase: 0.75,
            spike_hunt: 0.4,
            spike_idle: 0.1,
            spike_range: 16.0,
            spike_prox: 0.36,
        }
    }
}

#[derive(Default)]
struct DangerData {
    scaling: HashMap<String, Scaling>,
    depth: HashMap<String, DepthRow>,
    meter: Meter,
}

fn parse_danger(text: &str) -> Result<DangerData, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if v["schema"].as_str() != Some("lr.danger/1") {
        return Err("schema must be lr.danger/1".into());
    }
    let mut data = DangerData::default();
    for (id, r) in v["scaling"].as_object().ok_or("missing scaling")? {
        let f = |k: &str| r[k].as_f64().ok_or_else(|| format!("scaling.{id}.{k}: must be a number"));
        data.scaling.insert(
            id.clone(),
            Scaling {
                team_bonus: f("team_bonus")?.max(0.0) as i32,
                peek_chance: 0.0,
                inspect_chance: f("inspect_chance")?.max(0.0) as f32,
                pounce_table: r["pounce_table"].as_str().unwrap_or("").to_string(),
            },
        );
    }
    for (id, r) in v["depth"].as_object().ok_or("missing depth")? {
        let f = |k: &str| r[k].as_f64().ok_or_else(|| format!("depth.{id}.{k}: must be a number"));
        data.depth.insert(
            id.clone(),
            DepthRow {
                speed_per_level: f("speed_per_level")? as f32,
                speed_cap: f("speed_cap")? as f32,
                extra_every: f("extra_every")?.max(0.0) as i32,
                max_extra: f("max_extra")?.max(0.0) as i32,
                safe_room_base: f("safe_room_base")? as f32,
                safe_room_per_level: f("safe_room_per_level")? as f32,
                safe_room_min: f("safe_room_min")? as f32,
            },
        );
    }
    // `meter` is optional: a missing block, or any missing/non-numeric field, keeps that field's shipped default.
    if let Some(m) = v["meter"].as_object() {
        let d = Meter::default();
        let g = |k: &str, fallback: f32| m.get(k).and_then(|x| x.as_f64()).map_or(fallback, |x| x as f32);
        data.meter = Meter {
            depth_full: g("depth_full", d.depth_full),
            base_cap: g("base_cap", d.base_cap),
            base_min: g("base_min", d.base_min),
            depth_weight: g("depth_weight", d.depth_weight),
            speed_weight: g("speed_weight", d.speed_weight),
            speed_ref: g("speed_ref", d.speed_ref),
            hunters_weight: g("hunters_weight", d.hunters_weight),
            hunters_ref: g("hunters_ref", d.hunters_ref),
            safe_weight: g("safe_weight", d.safe_weight),
            safe_ref: g("safe_ref", d.safe_ref),
            spike_chase: g("spike_chase", d.spike_chase),
            spike_hunt: g("spike_hunt", d.spike_hunt),
            spike_idle: g("spike_idle", d.spike_idle),
            spike_range: g("spike_range", d.spike_range),
            spike_prox: g("spike_prox", d.spike_prox),
        };
    }
    Ok(data)
}

fn danger_data() -> &'static DangerData {
    static DATA: OnceLock<DangerData> = OnceLock::new();
    DATA.get_or_init(|| {
        let text = embedded::data_under(DANGER_PATH).next().map(|(_, t)| t).unwrap_or("");
        parse_danger(text).unwrap_or_else(|e| {
            eprintln!("{DANGER_PATH}: {e}");
            DangerData::default()
        })
    })
}

/// The HUD danger meter's tunables (data/danger.json `meter`), parsed once.
pub fn meter() -> &'static Meter {
    &danger_data().meter
}

fn danger_at(row: &DepthRow, depth: i32) -> Danger {
    let levels = (depth - 1).max(0) as f32;
    let speed_scale = (1.0 + row.speed_per_level * levels).min(row.speed_cap.max(1.0));
    let extra_hunters = if row.extra_every > 0 { ((depth - 1).max(0) / row.extra_every).min(row.max_extra) } else { 0 };
    let safe_room_odds = (row.safe_room_base + row.safe_room_per_level * levels).max(row.safe_room_min).clamp(0.0, 1.0);
    Danger { speed_scale, extra_hunters, safe_room_odds }
}

impl Difficulty {
    /// Team size, peek and inspect odds and the pounce table for this difficulty (data/danger.json; peek odds from data/peek.json).
    pub fn scaling(self) -> Scaling {
        let mut s = danger_data().scaling.get(self.id()).cloned().unwrap_or(Scaling {
            team_bonus: 0,
            peek_chance: 1.0,
            inspect_chance: 1.0,
            pounce_table: String::new(),
        });
        s.peek_chance = crate::hunters::peek::rules().scale(self).chance;
        s
    }

    /// The danger at a level number (1 = the first level, which is the unscaled base). A difficulty with no data row is flat.
    pub fn danger(self, depth: i32) -> Danger {
        danger_data()
            .depth
            .get(self.id())
            .map_or(Danger { speed_scale: 1.0, extra_hunters: 0, safe_room_odds: 0.0 }, |r| danger_at(r, depth))
    }

    /// The HUD danger meter's ROUTE BASE at a depth: a 0..1 floor that rises with descent and with the
    /// difficulty's scaling (`Self::danger`), reusing the shipped `Danger` curve and the `meter()` weights.
    /// It never reaches 0 (the `base_min` floor) and never fills past `base_cap`, leaving room for a hunter
    /// spike. `Game::danger` blends this base with the nearest-hunter spike (Infinite mode only).
    pub fn route_danger(self, depth: i32) -> f32 {
        let m = meter();
        let d = self.danger(depth);
        let levels = (depth - 1).max(0) as f32;
        let depth_t = (levels / (m.depth_full - 1.0)).clamp(0.0, 1.0);
        let speed_t = ((d.speed_scale - 1.0) / (m.speed_ref - 1.0)).clamp(0.0, 1.0);
        let hunters_t = (d.extra_hunters as f32 / m.hunters_ref).clamp(0.0, 1.0);
        let safe_t = ((m.safe_ref - d.safe_room_odds) / m.safe_ref).clamp(0.0, 1.0);
        let route = (m.depth_weight * depth_t
            + m.speed_weight * speed_t
            + m.hunters_weight * hunters_t
            + m.safe_weight * safe_t)
            .clamp(0.0, 1.0);
        (route * m.base_cap).max(m.base_min)
    }
}

#[cfg(test)]
mod danger_tests {
    use super::*;

    #[test]
    fn the_shipped_file_has_every_difficulty() {
        assert!(!danger_data().scaling.is_empty() && !danger_data().depth.is_empty(), "danger.json loads");
        for d in LOBBY {
            assert!(danger_data().scaling.contains_key(d.id()), "scaling.{}", d.id());
            assert!(danger_data().depth.contains_key(d.id()), "depth.{}", d.id());
        }
    }

    /// Nightmare scales team size, peek and inspect odds above Standard and names its pounce table.
    #[test]
    fn nightmare_profile_scales_above_standard() {
        let (n, s) = (Difficulty::Nightmare.scaling(), Difficulty::Standard.scaling());
        assert!(n.team_bonus > s.team_bonus);
        assert!(n.peek_chance > s.peek_chance);
        assert!(n.inspect_chance > s.inspect_chance);
        assert_eq!(n.pounce_table, "nightmare");
        assert_eq!(Difficulty::Relaxed.scaling().pounce_table, "");
        assert_eq!(s.team_bonus, 0, "Standard keeps the shipped director teams");
    }

    #[test]
    fn danger_grows_with_depth_and_is_flat_on_level_one() {
        for d in LOBBY {
            let base = d.danger(1);
            assert_eq!((base.speed_scale, base.extra_hunters), (1.0, 0), "{}", d.id());
            let deep = d.danger(40);
            assert!(deep.speed_scale >= base.speed_scale && deep.safe_room_odds <= base.safe_room_odds, "{}", d.id());
        }
        assert!(Difficulty::Nightmare.danger(20).speed_scale > Difficulty::Standard.danger(20).speed_scale);
        assert_eq!(Difficulty::Relaxed.danger(30).speed_scale, 1.0);
    }

    /// Standard plays as before at every depth: no speed-up, no extra hunters, the shipped safe-room odds.
    #[test]
    fn standard_is_flat_at_every_depth() {
        for depth in [1, 2, 9, 17, 40, 200] {
            let d = Difficulty::Standard.danger(depth);
            assert_eq!((d.speed_scale, d.extra_hunters), (1.0, 0), "depth {depth}");
            assert!((f64::from(d.safe_room_odds) - crate::world::route::danger::SAFE_ROOM_BASE_CHANCE).abs() < 1e-6, "depth {depth}");
        }
    }

    #[test]
    fn danger_is_capped_and_odds_stay_in_range() {
        let row = DepthRow { speed_per_level: 0.1, speed_cap: 1.2, extra_every: 2, max_extra: 3, safe_room_base: 0.5, safe_room_per_level: -0.2, safe_room_min: 0.05 };
        let d = danger_at(&row, 500);
        assert_eq!(d.speed_scale, 1.2);
        assert_eq!(d.extra_hunters, 3);
        assert_eq!(d.safe_room_odds, 0.05);
    }

    #[test]
    fn a_changed_data_value_changes_the_danger() {
        let text = r#"{"schema":"lr.danger/1","scaling":{},"depth":{"standard":{"speed_per_level":0.5,"speed_cap":2.0,"extra_every":0,"max_extra":0,"safe_room_base":0.4,"safe_room_per_level":0.0,"safe_room_min":0.0}}}"#;
        let data = parse_danger(text).unwrap();
        assert_eq!(danger_at(&data.depth["standard"], 3).speed_scale, 2.0);
        assert!(parse_danger(r#"{"schema":"x"}"#).is_err());
    }

    /// The route base is never empty: at least `base_min` for every lobby difficulty on the first floor.
    #[test]
    fn route_danger_never_zero() {
        let floor = meter().base_min;
        for d in LOBBY {
            assert!(d.route_danger(1) >= floor, "{} d1 below base_min", d.id());
        }
    }

    /// The route base rises with descent for every difficulty (deeper is never safer).
    #[test]
    fn route_danger_rises_with_depth() {
        for d in LOBBY {
            assert!(d.route_danger(10) > d.route_danger(1), "{} did not rise with depth", d.id());
        }
    }

    /// A harder difficulty reads a higher route base at the same depth.
    #[test]
    fn route_danger_harder_reads_higher() {
        assert!(Difficulty::Nightmare.route_danger(12) > Difficulty::Standard.route_danger(12));
        assert!(Difficulty::Hard.route_danger(12) > Difficulty::Standard.route_danger(12));
    }

    /// An absent `meter` block loads the shipped defaults (no back-compat break on the old schema).
    #[test]
    fn meter_block_defaults_when_absent() {
        let text = r#"{"schema":"lr.danger/1","scaling":{},"depth":{}}"#;
        let data = parse_danger(text).unwrap();
        assert_eq!(data.meter, Meter::default());
        // and the shipped file's block parses (meter() reads real data, not the fallback default alone).
        assert!(meter().base_min > 0.0 && meter().base_cap <= 1.0);
    }

    /// A field present in `meter` overrides just that field; the rest keep their defaults.
    #[test]
    fn meter_partial_override_keeps_other_defaults() {
        let text = r#"{"schema":"lr.danger/1","scaling":{},"depth":{},"meter":{"base_min":0.2}}"#;
        let data = parse_danger(text).unwrap();
        assert_eq!(data.meter.base_min, 0.2);
        assert_eq!(data.meter.base_cap, Meter::default().base_cap);
        assert_eq!(data.meter.spike_chase, Meter::default().spike_chase);
    }
}
