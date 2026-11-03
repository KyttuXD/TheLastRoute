//! Ravelle: tripwire trapper. Two wires across doorway-shaped cells, re-laid every 30 s. Walking
//! into one stuns you 1.6 s, alerts every hunter and permanently adds 8% to her speed
//! (ravelle_gimmick.gd). The port draws the wires faintly, as the newer design does.

use super::{dist_to_segment, Ctx, Event};
use crate::hunters::brain::{Brain, Target};
use crate::world::Cell;

#[derive(Clone)]
pub struct Wire {
    pub a: (f32, f32),
    pub b: (f32, f32),
    pub triggered: bool,
}

#[derive(Default)]
pub struct Ravelle {
    pub wires: Vec<Wire>,
    has_wires: bool,
    refresh: f32,
    window_triggered: bool,
    triggered: i32,
}

impl Ravelle {
    pub fn speed_scale(&self) -> f32 {
        1.0 + self.triggered as f32 * 0.08
    }

    fn place(&mut self, b: &mut Brain, c: &Ctx) {
        let f = c.fac;
        let open = |x: i32, y: i32| !f.is_wall_cell(x, y);
        let mut cands: Vec<Cell> = f
            .floor_cells()
            .into_iter()
            .filter(|p| {
                let (l, r, u, d) = (
                    open(p.x - 1, p.y),
                    open(p.x + 1, p.y),
                    open(p.x, p.y - 1),
                    open(p.x, p.y + 1),
                );
                (l && r && (!u || !d)) || (u && d && (!l || !r))
            })
            .collect();
        self.wires.clear();
        while self.wires.len() < 2 && !cands.is_empty() {
            let cell = cands.remove(b.next_index(cands.len()));
            let centre = cell.center();
            let across = if open(cell.x - 1, cell.y) && open(cell.x + 1, cell.y) {
                (0.0, 1.42)
            } else {
                (1.42, 0.0)
            };
            self.wires.push(Wire {
                a: (centre.0 - across.0, centre.1 - across.1),
                b: (centre.0 + across.0, centre.1 + across.1),
                triggered: false,
            });
        }
        self.has_wires = true;
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        self.refresh -= dt;
        if !self.has_wires || self.refresh <= 0.0 {
            if !self.has_wires || !self.window_triggered {
                self.place(b, c);
            }
            self.refresh = 30.0;
            self.window_triggered = false;
        }
        if t.hidden || t.in_safe_zone {
            return;
        }
        for w in self.wires.iter_mut() {
            if w.triggered || dist_to_segment(t.pos, w.a, w.b) > 0.8 {
                continue;
            }
            w.triggered = true;
            self.window_triggered = true;
            self.triggered += 1;
            ev.push(Event::Stun(1.6));
            ev.push(Event::FacilityAlert {
                at: t.pos,
                except_self: false,
            });
            b.chase(t.pos);
        }
    }
}
