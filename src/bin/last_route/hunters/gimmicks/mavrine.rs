//! Mavrine: lattice mark. She cycles cooldown 4.2 s -> survey 0.9 s (standing still) ->
//! project 6 s (an invisible mark somewhere near her) -> ping; crossing the mark sends her
//! there (mavrine_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{cell_of, dist, Brain, State, Target};
use crate::world::Cell;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Cooldown,
    Survey,
    Project,
    Ping,
}

pub struct Mavrine {
    phase: Phase,
    timer: f32,
    mark: Option<(f32, f32)>,
    last_mark: Option<Cell>,
}

impl Mavrine {
    /// The window moved by `d`.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.mark = self.mark.map(|m| (m.0 + d.0, m.1 + d.1));
        self.last_mark = None;
    }

    pub fn new() -> Mavrine {
        Mavrine {
            phase: Phase::Cooldown,
            timer: 4.2,
            mark: None,
            last_mark: None,
        }
    }

    pub fn holds_position(&self) -> bool {
        self.phase == Phase::Survey
    }

    /// The invisible mark she projected (tests stand the runner on it).
    #[cfg(test)]
    pub fn mark(&self) -> Option<(f32, f32)> {
        self.mark
    }

    /// The mechanic pose she is in: standing still taking her bearings (the survey), then the ping when a mark is crossed.
    pub fn mechanic(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Survey => Some("survey"),
            Phase::Ping => Some("ping_mark"),
            _ => None,
        }
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        if b.state == State::Chase && matches!(self.phase, Phase::Cooldown | Phase::Survey) {
            self.timer = self.timer.max(6.2 * 0.35);
            return;
        }
        self.timer -= dt;
        if self.timer <= 0.0 {
            match self.phase {
                Phase::Cooldown => {
                    self.phase = Phase::Survey;
                    self.timer = 0.9;
                }
                Phase::Survey => {
                    self.mark = None;
                    self.place_mark(b, c);
                    self.phase = Phase::Project;
                    self.timer = 6.0;
                }
                Phase::Project | Phase::Ping => {
                    self.mark = None;
                    self.phase = Phase::Cooldown;
                    self.timer = 4.2;
                }
            }
        }
        if self.phase != Phase::Project || t.hidden || t.in_safe_zone {
            return;
        }
        let Some(m) = self.mark else { return };
        if dist(t.pos, m) > 0.62 {
            return;
        }
        self.phase = Phase::Ping;
        self.timer = 0.45;
        b.investigate(m);
    }

    fn place_mark(&mut self, b: &mut Brain, c: &Ctx) {
        let f = c.fac;
        let start = f.start.center();
        let exit = f.exit.center();
        let mut cands: Vec<Cell> = f
            .floor_cells()
            .into_iter()
            .filter(|cell| {
                let p = cell.center();
                Some(*cell) != self.last_mark
                    && dist(p, start) >= 4.0
                    && dist(p, exit) >= 2.5
                    && dist(p, b.pos) >= 3.0
                    && dist(p, b.pos) <= 30.0
                    && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .filter(|(dx, dy)| !f.is_wall_cell(cell.x + dx, cell.y + dy))
                        .count()
                        >= 3
                    && f.can_occupy(p.0 as f64, p.1 as f64, 0.16)
                    && f.relays.iter().all(|r| dist(r.center(), p) >= 1.25)
                    && f.hiding.iter().all(|h| dist(h.cell.center(), p) >= 1.25)
            })
            .collect();
        for i in (1..cands.len()).rev() {
            let j = b.next_index(i + 1);
            cands.swap(i, j);
        }
        let here = cell_of(b.pos);
        for cell in cands.iter().take(48) {
            if !f.find_path(here, *cell).is_empty() {
                self.mark = Some(cell.center());
                self.last_mark = Some(*cell);
                return;
            }
        }
    }
}
