//! Ysolde: timid battery thief. She steals batteries off the floor, backs away when she sees
//! you within 9 cells, and holds her distance instead of closing in (ysolde_gimmick.gd,
//! BEHAVIOUR "avoid").

use super::{Ctx, Event};
use crate::hunters::brain::{cell_of, dist, Brain, State, Target};
use crate::world::Cell;

const KEEP: f32 = 9.0;

#[derive(Default)]
pub struct Ysolde {
    fleeing: bool,
    flee_timer: f32,
    replan: f32,
    flee_point: Option<(f32, f32)>,
}

impl Ysolde {
    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if self.update_flee(b, dt, c, t) {
            return;
        }
        if t.pesticide {
            return;
        }
        let nearest = (0..c.fac.batteries.len())
            // resident hunters (other floors) are stepped with an empty taken list: a battery with no entry is not taken
            .filter(|i| !c.batteries_taken.get(*i).copied().unwrap_or(false))
            .min_by(|p, q| {
                dist(c.fac.batteries[*p].center(), b.pos)
                    .partial_cmp(&dist(c.fac.batteries[*q].center(), b.pos))
                    .unwrap()
            });
        let Some(i) = nearest else { return };
        let bat = c.fac.batteries[i].center();
        b.state = State::Investigate;
        b.last_known = Some(bat);
        if dist(b.pos, bat) <= 0.8 {
            ev.push(Event::TakeBattery(i));
        }
    }

    fn update_flee(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) -> bool {
        self.flee_timer -= dt;
        self.replan -= dt;
        let gap = dist(b.pos, t.pos);
        let sees = !t.hidden && gap < KEEP && c.fac.has_line_of_sight(b.pos, t.pos);
        if sees {
            self.fleeing = true;
            self.flee_timer = 1.5;
        }
        if self.flee_timer <= 0.0 && self.fleeing {
            self.fleeing = false;
            b.last_known = None;
            b.state = State::Wander;
        }
        if !self.fleeing {
            if gap < KEEP * 1.6
                && matches!(b.state, State::Chase | State::Investigate)
                && b.last_known.map(|k| dist(k, t.pos) < 2.5).unwrap_or(false)
            {
                b.state = State::Investigate;
                b.last_known = Some(b.pos);
                return true;
            }
            return false;
        }
        if self.replan <= 0.0
            || self
                .flee_point
                .map(|p| dist(p, b.pos) < 0.8)
                .unwrap_or(true)
        {
            self.replan = 0.6;
            self.flee_point = Some(self.flee_target(b, c, t.pos).unwrap_or(b.pos));
        }
        b.state = State::Investigate;
        b.last_known = self.flee_point;
        true
    }

    fn flee_target(&self, b: &Brain, c: &Ctx, threat: (f32, f32)) -> Option<(f32, f32)> {
        let f = c.fac;
        let here = cell_of(b.pos);
        let gap = dist(b.pos, threat);
        let mut cands: Vec<(f32, Cell)> = vec![];
        let mut r: i32 = 3;
        while r as f32 <= KEEP {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let cell = Cell::new(here.x + dx, here.y + dy);
                    let p = cell.center();
                    if !f.hunter_open(cell) {
                        continue;
                    }
                    let open4 = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .filter(|(a, b)| !f.is_wall_cell(cell.x + a, cell.y + b))
                        .count();
                    if open4 < 2 || dist(p, threat) <= gap + 0.75 {
                        continue;
                    }
                    cands.push((dist(p, threat), cell));
                }
            }
            r += 2;
        }
        cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        for (_, cell) in cands {
            let path = f.find_path(here, cell);
            if !path.is_empty() && path.iter().all(|s| dist(s.center(), threat) >= gap - 0.5) {
                return Some(cell.center());
            }
        }
        None
    }
}
