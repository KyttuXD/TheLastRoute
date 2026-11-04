//! The resident neighbour level's render meshes (docs/STAIRS_CONTINUOUS_DESIGN.md phase 4): one `WorldMeshes` per
//! resident level, drawn in the same pass as the active floor at dy = (level - active) * STOREY, plus the lamp merge
//! and the pure predicates that decide when a neighbour is built or drawn.

use super::shader::{Lamp, MAX_LAMPS};
use super::world_mesh::{self, ChunkFrame, NeighbourScope, Opening, WorldMeshes};
use crate::game::level_stack::LevelStack;
use crate::shared::assets::Assets;
use crate::content::Content;
use crate::world::vertical::STOREY;
use crate::world::Facility;
use macroquad::prelude::*;
use std::collections::HashMap;

/// How far (m, floor plane) from an opening an actor on another level is still drawn: the neighbour cull radius.
/// Relabelling the active level does not touch the hunter mesh cache, because every body keeps its absolute y.
pub fn actor_cull_range() -> f32 {
    world_mesh::stream_table().neighbour_cull_radius
}

/// How many of the room's own nearest lamps are always kept.
const OWN_KEPT: usize = 8;
/// How many ordinary neighbour lamps join the stairwell lamp.
const NEIGHBOUR_KEPT: usize = 2;
/// A neighbour lamp this close (metres, in the floor plane) to a stair's centre is the stairwell lamp.
pub const STAIRWELL_LAMP_RADIUS: f32 = 1.5;

/// The meshes of the resident neighbour levels.
#[derive(Default)]
pub struct NeighbourWorlds {
    pub items: Vec<WorldMeshes>,
    /// The window centre each item was last built for, by level.
    built: HashMap<i32, (i32, i32)>,
}

/// Whether a neighbour is worth building now: the eye is within `reach` metres (in the floor plane) of an opening's
/// centre, or the runner is on a link (flight, ladder or fall).
pub fn wants_neighbour(eye: (f32, f32, f32), openings: &[Opening], reach: f32, on_link: bool) -> bool {
    on_link
        || openings.iter().any(|o| {
            let (dx, dz) = (o.centre.0 - eye.0, o.centre.1 - eye.2);
            dx * dx + dz * dz <= reach * reach
        })
}

/// Whether the eye is between the floors (below the active floor, or above a standing eye on a way up).
pub fn eye_on_flight(eye_y: f32) -> bool {
    eye_y < -0.05 || eye_y > STOREY * 0.5
}

/// The y of the slab between the active level and a neighbour at `dy`.
fn slab_y(dy: f32) -> f32 {
    if dy > 0.0 {
        dy
    } else {
        dy + STOREY
    }
}

/// Whether every box lies on the far side of the slab plane from the eye (the neighbour cannot be seen except
/// through an opening, so with no opening near the eye it is skipped).
pub fn behind_slab(eye_y: f32, dy: f32, bounds: &[(Vec3, Vec3)]) -> bool {
    if bounds.is_empty() {
        return false;
    }
    let plane = slab_y(dy);
    const EPS: f32 = 0.01;
    if dy > 0.0 {
        eye_y < plane && bounds.iter().all(|(lo, _)| lo.y >= plane - EPS)
    } else {
        eye_y > plane && bounds.iter().all(|(_, hi)| hi.y <= plane + EPS)
    }
}

/// The lamps to light the scene with: the 8 nearest own lamps, the stairwell lamp and the 2 nearest other
/// neighbour lamps, then the remaining own lamps up to `cap` (never more than MAX_LAMPS).
pub fn merge_lamps(own: &[Lamp], nb: &[(Lamp, bool)], eye: Vec3, cap: usize) -> Vec<Lamp> {
    let cap = cap.min(MAX_LAMPS);
    let mut own: Vec<Lamp> = own.to_vec();
    own.sort_by(|a, b| a.pos.distance_squared(eye).total_cmp(&b.pos.distance_squared(eye)));
    let mut out: Vec<Lamp> = own.iter().take(OWN_KEPT.min(cap)).copied().collect();
    let mut near: Vec<&(Lamp, bool)> = nb.iter().collect();
    near.sort_by(|a, b| a.0.pos.distance_squared(eye).total_cmp(&b.0.pos.distance_squared(eye)));
    if let Some(w) = near.iter().position(|(_, well)| *well) {
        if out.len() < cap {
            out.push(near.remove(w).0);
        }
    }
    for (l, _) in near.into_iter().filter(|(_, well)| !*well).take(NEIGHBOUR_KEPT) {
        if out.len() < cap {
            out.push(*l);
        }
    }
    for l in own.into_iter().skip(OWN_KEPT) {
        if out.len() >= cap {
            break;
        }
        out.push(l);
    }
    out
}

impl NeighbourWorlds {
    /// Keeps one item per resident level, builds those the eye is near an opening of (or on a flight), and drops the
    /// rest. Parts already built are kept when the eye is far; only the building stops.
    pub fn sync(
        &mut self,
        active: &Facility,
        stack: &LevelStack,
        eye: (f32, f32, f32),
        assets: &Assets,
        content: &Content,
    ) {
        let levels = stack.levels();
        self.items.retain(|m| levels.contains(&m.level));
        self.built.retain(|l, _| levels.contains(l));
        let table = world_mesh::stream_table();
        let reach = crate::game::transition::links().prefetch_distance;
        let on_flight = eye_on_flight(eye.1);
        let own_openings = world_mesh::openings(active);
        for r in &stack.residents {
            let dy = (r.level - active.level) as f32 * STOREY;
            let idx = match self.items.iter().position(|m| m.level == r.level) {
                Some(i) => i,
                None => {
                    let mut m = WorldMeshes::default();
                    m.level = r.level;
                    m.dy = dy;
                    self.items.push(m);
                    self.items.len() - 1
                }
            };
            // an item that arrived by a relabel is a whole floor already built for the shared centre
            if !self.built.contains_key(&r.level) && !self.items[idx].meshes.is_empty() {
                self.built.insert(r.level, r.center);
            }
            let mut openings = world_mesh::openings(&r.fac);
            openings.extend(own_openings.iter().map(|o| Opening {
                cells: o.cells.clone(),
                centre: o.centre,
                upper_level: o.upper_level,
            }));
            let m = &mut self.items[idx];
            let stale = self.built.get(&r.level) != Some(&r.center) || m.has_pending();
            if stale && wants_neighbour(eye, &openings, reach, on_flight) {
                let scope = NeighbourScope {
                    dy,
                    openings,
                    radius: table.neighbour_cull_radius,
                    max_parts: table.neighbour_max_parts,
                    chunks_per_frame: table.neighbour_chunks_per_frame,
                };
                m.level = r.level;
                m.dy = dy;
                m.rebuild_neighbour(&r.fac, ChunkFrame { level: r.level, center: r.center }, &scope, assets, content);
                self.built.insert(r.level, r.center);
            }
        }
    }

    /// Swaps the active meshes with the item of level `to` and shifts both so the new active sits at dy 0 and the old
    /// active at its true offset. Never rebuilds. False when there is no such item.
    pub fn relabel(&mut self, active: &mut WorldMeshes, to: i32) -> bool {
        let Some(i) = self.items.iter().position(|m| m.level == to) else {
            return false;
        };
        let from_level = active.level;
        std::mem::swap(active, &mut self.items[i]);
        // `active` now holds the neighbour's meshes at its own dy; bring it to 0. The old active moves by the same delta (the stack shifts rigidly)
        let shift = -active.dy;
        active.shift_y(shift);
        self.items[i].shift_y(shift);
        active.level = to;
        self.items[i].level = from_level;
        self.built.remove(&to);
        self.built.remove(&from_level);
        true
    }

    /// Every neighbour lamp, flagged when within STAIRWELL_LAMP_RADIUS of a stair centre of `active`.
    pub fn lamps(&self, active: &Facility) -> Vec<(Lamp, bool)> {
        let centres: Vec<(f32, f32)> = active.stairs.iter().map(|s| s.cell.center()).collect();
        self.items
            .iter()
            .flat_map(|m| m.lamps.iter())
            .map(|l| {
                let well = centres.iter().any(|c| {
                    let (dx, dz) = (l.pos.x - c.0, l.pos.z - c.1);
                    dx * dx + dz * dz <= STAIRWELL_LAMP_RADIUS * STAIRWELL_LAMP_RADIUS
                });
                (*l, well)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(x: f32, y: f32, z: f32) -> Lamp {
        Lamp { pos: vec3(x, y, z), range: 8.0, color: WHITE, energy: 1.0 }
    }

    fn own_row(n: usize) -> Vec<Lamp> {
        (0..n).map(|i| lamp(i as f32 + 1.0, 1.0, 0.0)).collect()
    }

    #[test]
    fn merge_lamps_keeps_the_eight_nearest_own_lamps() {
        let own = own_row(20);
        let nb: Vec<(Lamp, bool)> = (0..5).map(|i| (lamp(i as f32, 6.0, 0.0), false)).collect();
        let out = merge_lamps(&own, &nb, vec3(0.0, 1.0, 0.0), MAX_LAMPS);
        for i in 0..8 {
            assert!(out.iter().any(|l| l.pos.x == i as f32 + 1.0 && l.pos.y == 1.0), "own lamp {i} kept");
        }
    }

    #[test]
    fn merge_lamps_adds_the_stairwell_lamp_and_two_nearest() {
        let own = own_row(20);
        let well = lamp(90.0, 6.0, 0.0);
        let nb = vec![
            (lamp(1.0, 6.0, 0.0), false),
            (well, true),
            (lamp(2.0, 6.0, 0.0), false),
            (lamp(3.0, 6.0, 0.0), false),
        ];
        let out = merge_lamps(&own, &nb, vec3(0.0, 1.0, 0.0), MAX_LAMPS);
        assert!(out.iter().any(|l| l.pos == well.pos), "the far stairwell lamp is kept");
        assert!(out.iter().any(|l| l.pos == nb[0].0.pos) && out.iter().any(|l| l.pos == nb[2].0.pos), "the two nearest others are kept");
        assert!(!out.iter().any(|l| l.pos == nb[3].0.pos), "a third ordinary neighbour lamp is not");
    }

    #[test]
    fn merge_lamps_never_exceeds_cap() {
        let own = own_row(40);
        let nb: Vec<(Lamp, bool)> = (0..10).map(|i| (lamp(i as f32, 6.0, 0.0), i == 3)).collect();
        for cap in [0, 3, 8, 12, MAX_LAMPS, 100] {
            let out = merge_lamps(&own, &nb, Vec3::ZERO, cap);
            assert!(out.len() <= cap.min(MAX_LAMPS), "cap {cap}");
        }
        assert_eq!(merge_lamps(&own, &nb, Vec3::ZERO, 100).len(), MAX_LAMPS);
    }

    fn crafted(level: i32, dy: f32) -> WorldMeshes {
        let mut m = WorldMeshes::default();
        m.level = level;
        m.dy = dy;
        m.bounds = vec![(vec3(0.0, dy, 0.0), vec3(4.0, dy + 3.0, 4.0))];
        m.lamps = vec![lamp(1.0, dy + 2.0, 1.0)];
        m
    }

    #[test]
    fn relabel_swaps_levels_and_leaves_new_active_at_dy_zero() {
        let mut nb = NeighbourWorlds::default();
        nb.items.push(crafted(1, STOREY));
        let mut active = crafted(0, 0.0);
        assert!(nb.relabel(&mut active, 1));
        assert_eq!(active.level, 1);
        assert!(active.dy.abs() < 1e-5, "the new active sits at dy 0");
        assert!((active.bounds[0].0.y - 0.0).abs() < 1e-5);
        assert!((active.lamps[0].pos.y - 2.0).abs() < 1e-5);
        let old = &nb.items[0];
        assert_eq!(old.level, 0);
        assert!((old.dy + STOREY).abs() < 1e-5, "the old active is one storey below");
        assert!((old.bounds[0].0.y + STOREY).abs() < 1e-5);
        assert!(!nb.relabel(&mut active, 7), "no item, no change");
    }

    #[test]
    fn neighbour_cull_skips_far_meshes() {
        let near = (vec3(0.0, STOREY, 0.0), vec3(2.0, STOREY + 3.0, 2.0));
        let far = (vec3(500.0, STOREY, 0.0), vec3(502.0, STOREY + 3.0, 2.0));
        let eye = vec3(1.0, 1.6, 1.0);
        let reach = 80.0;
        let gap = |b: &(Vec3, Vec3)| (b.0 - eye).max(eye - b.1).max(Vec3::ZERO).length();
        assert!(gap(&near) < reach);
        assert!(gap(&far) >= reach);
        // above the slab with the eye below it, nothing of the neighbour is on the eye's side
        assert!(behind_slab(1.6, STOREY, &[near, far]));
        assert!(!behind_slab(STOREY + 1.0, STOREY, &[near]));
        assert!(behind_slab(1.6, -STOREY, &[(vec3(0.0, -STOREY, 0.0), vec3(1.0, -0.5, 1.0))]));
    }

    #[test]
    fn neighbour_not_built_when_far_from_every_opening() {
        let o = Opening { cells: Vec::new(), centre: (10.0, 10.0), upper_level: 1 };
        assert!(wants_neighbour((10.0, 1.6, 12.0), std::slice::from_ref(&o), 8.0, false));
        assert!(!wants_neighbour((40.0, 1.6, 10.0), std::slice::from_ref(&o), 8.0, false));
        assert!(wants_neighbour((40.0, -2.0, 10.0), std::slice::from_ref(&o), 8.0, true), "on a flight it builds");
        assert!(!wants_neighbour((10.0, 1.6, 10.0), &[], 8.0, false), "no openings, no neighbour");
        assert!(eye_on_flight(-1.0) && eye_on_flight(STOREY - 1.0) && !eye_on_flight(1.6));
    }
}
