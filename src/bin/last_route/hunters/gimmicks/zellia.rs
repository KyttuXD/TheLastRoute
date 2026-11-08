//! Zellia: the merchant. She settles in the room nearest the facility's centre and trades for
//! coins: cool, warm, lamp repair, pesticide. Every completed trade carries a small chance the
//! deal costs your life (zellia_gimmick.gd, main_3d _perform_zellia_trade). The odds and the
//! repair branch come from data/gimmicks/zellia.json.

use super::{open_near, Ctx};
use crate::config::Difficulty;
use crate::hunters::brain::{Brain, State, Target};

pub struct Zellia {
    settled: bool,
    pub death_chance: f32,
    pub shop_open: bool,
}

pub const OFFERS: [(&str, i32); 4] = [("COOL", 2), ("WARM", 2), ("REPAIR", 3), ("PESTICIDE", 4)];

/// The odds and the repair branch, from data/gimmicks/zellia.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Odds {
    pub by_difficulty: Vec<(String, f32)>,
    pub default_chance: f32,
    pub repair_mends_relay: bool,
}

impl Odds {
    /// Parse the data text; a bad or missing value keeps the Godot number (Standard 5 percent).
    pub fn parse(text: &str) -> Odds {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let ok = |x: &serde_json::Value| x.as_f64().filter(|n| (0.0..=1.0).contains(n)).map(|n| n as f32);
        let by_difficulty = v
            .get("death_chance")
            .and_then(|d| d.as_object())
            .map(|o| o.iter().filter_map(|(k, x)| ok(x).map(|n| (k.clone(), n))).collect())
            .unwrap_or_default();
        Odds {
            by_difficulty,
            default_chance: v.get("death_chance_default").and_then(ok).unwrap_or(0.05),
            repair_mends_relay: v.get("repair_mends_relay").and_then(|x| x.as_bool()).unwrap_or(true),
        }
    }

    /// The death chance per completed trade on `diff`.
    pub fn death_chance(&self, diff: Difficulty) -> f32 {
        self.by_difficulty.iter().find(|(k, _)| k == diff.id()).map_or(self.default_chance, |(_, n)| *n)
    }
}

/// The shipped odds, parsed once.
pub fn odds() -> &'static Odds {
    static O: std::sync::OnceLock<Odds> = std::sync::OnceLock::new();
    O.get_or_init(|| Odds::parse(include_str!("../../data/gimmicks/zellia.json")))
}

/// What the REPAIR offer mends: a sabotaged relay when one exists (and the data allows it), else the lamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairTarget {
    Relay,
    Lamp,
}

pub fn repair_target(sabotaged_relay_exists: bool) -> RepairTarget {
    if sabotaged_relay_exists && odds().repair_mends_relay {
        RepairTarget::Relay
    } else {
        RepairTarget::Lamp
    }
}

impl Zellia {
    /// Zellia on Nightmare (game_config.gd ZELLIA_DEATH_CHANCE nightmare, from data).
    pub fn nightmare() -> Zellia {
        Zellia::new(Difficulty::Nightmare)
    }

    pub fn new(diff: Difficulty) -> Zellia {
        Zellia {
            settled: false,
            death_chance: odds().death_chance(diff),
            shop_open: false,
        }
    }

    pub fn holds_position(&self) -> bool {
        true
    }

    pub fn trading(&self) -> bool {
        true
    }

    /// The mechanic pose she is in: both hands out over the counter while the runner has her shop open.
    pub fn mechanic(&self) -> Option<&'static str> {
        self.shop_open.then_some("trade_offer")
    }

    pub fn after(&mut self, b: &mut Brain, _dt: f32, c: &Ctx, t: &Target) {
        if !self.settled {
            self.settled = true;
            let rooms = &c.fac.rooms;
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for r in rooms {
                let (x, y) = (r.center.x as f32, r.center.y as f32);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            let mid = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            if let Some(r) = rooms.iter().min_by(|a, b| {
                let da = (a.center.x as f32 - mid.0).powi(2) + (a.center.y as f32 - mid.1).powi(2);
                let db = (b.center.x as f32 - mid.0).powi(2) + (b.center.y as f32 - mid.1).powi(2);
                da.partial_cmp(&db).unwrap()
            }) {
                b.pos = open_near(c.fac, r.center.center());
            }
        }
        b.state = State::Idle;
        b.idle_timer = 999.0;
        b.last_known = None;
        b.lost_sight = 0.0;
        if self.shop_open && crate::hunters::brain::dist(b.pos, t.pos) > 1.35 + 0.25 {
            self.shop_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// game_config.gd ZELLIA_DEATH_CHANCE: relaxed 2, standard 5, hard 7.5, nightmare 12 percent.
    #[test]
    fn shipped_death_chances_match_the_game() {
        assert_eq!(Zellia::nightmare().death_chance, 0.12);
        assert_eq!(Zellia::new(Difficulty::Relaxed).death_chance, 0.02);
        assert_eq!(Zellia::new(Difficulty::Standard).death_chance, 0.05);
        assert_eq!(Zellia::new(Difficulty::Hard).death_chance, 0.075);
    }

    /// A data edit changes the odds; an out-of-range entry is ignored; a missing difficulty uses the default.
    #[test]
    fn data_edits_change_the_odds() {
        let o = Odds::parse(r#"{"death_chance": {"hard": 0.5, "standard": 9}, "death_chance_default": 0.3}"#);
        assert_eq!(o.death_chance(Difficulty::Hard), 0.5);
        assert_eq!(o.death_chance(Difficulty::Standard), 0.3);
        assert_eq!(o.death_chance(Difficulty::Nightmare), 0.3);
        assert_eq!(Odds::parse("nope").death_chance(Difficulty::Hard), 0.05);
    }

    /// REPAIR mends a sabotaged relay when there is one, else the lamp.
    #[test]
    fn repair_prefers_a_sabotaged_relay() {
        assert_eq!(repair_target(true), RepairTarget::Relay);
        assert_eq!(repair_target(false), RepairTarget::Lamp);
    }
}
