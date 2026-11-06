//! Zellia: the merchant. She settles in the room nearest the facility's centre and trades for
//! coins: cool, warm, lamp repair, pesticide. Every completed trade carries a small chance the
//! deal costs your life (zellia_gimmick.gd, main_3d _perform_zellia_trade).

use super::{open_near, Ctx};
use crate::config::Difficulty;
use crate::hunters::brain::{Brain, State, Target};

pub struct Zellia {
    settled: bool,
    pub death_chance: f32,
    pub shop_open: bool,
}

pub const OFFERS: [(&str, i32); 4] = [("COOL", 2), ("WARM", 2), ("REPAIR", 3), ("PESTICIDE", 4)];

impl Zellia {
    /// game_config.gd ZELLIA_DEATH_CHANCE["nightmare"] = 0.12 (Standard 0.05, Hard 0.075).
    pub const NIGHTMARE_DEATH_CHANCE: f32 = 0.12;

    /// Zellia on Nightmare: a 12 percent death chance per trade.
    pub fn nightmare() -> Zellia {
        Zellia {
            settled: false,
            death_chance: Zellia::NIGHTMARE_DEATH_CHANCE,
            shop_open: false,
        }
    }

    pub fn new(diff: Difficulty) -> Zellia {
        let death_chance = match diff {
            Difficulty::Relaxed => 0.02,
            Difficulty::Standard => 0.05,
            Difficulty::Hard => 0.075,
            Difficulty::Nightmare => Zellia::NIGHTMARE_DEATH_CHANCE,
        };
        Zellia {
            settled: false,
            death_chance,
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

    /// game_config.gd ZELLIA_DEATH_CHANCE nightmare = 0.12.
    #[test]
    fn nightmare_death_chance_is_twelve_percent() {
        assert_eq!(Zellia::nightmare().death_chance, 0.12);
        assert_eq!(Zellia::new(Difficulty::Standard).death_chance, 0.05);
        assert_eq!(Zellia::new(Difficulty::Hard).death_chance, 0.075);
    }
}
