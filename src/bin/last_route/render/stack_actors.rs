//! Bodies at their absolute height (docs/STAIRS_CONTINUOUS_DESIGN.md phase 8, rule 4 of section 5): hunters, bots and
//! remote players are drawn at `y_abs - floor_y_of(active)`, and those on another level are drawn only when they are
//! on a flight or seen through an opening. Pure functions; `scene.rs` iterates `draw_list`.

use super::world_mesh::{self, Opening};
use super::world_stack::actor_cull_range;
use crate::game::level_stack::floor_y_of;
use crate::game::Game;
use crate::world::vertical::STOREY;
use macroquad::prelude::*;

/// Within this distance (m) of a floor a body counts as standing on it, not on a flight.
const ON_FLOOR_EPS: f32 = 0.1;
/// Height above the feet the sight test aims at (m).
const AIM_HEIGHT: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActorKind {
    Hunter,
    Bot,
}

/// One body to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorRef {
    pub kind: ActorKind,
    /// The level whose list owns the body (the mesh cache key's level).
    pub level: i32,
    /// Index inside that list (the mesh cache key's index).
    pub index: usize,
    pub pos: (f32, f32),
    /// Absolute height of the feet.
    pub y: f32,
}

/// The draw offset of a body at absolute `y_abs` while `active_level` is the drawn frame (0 on the active floor).
pub fn actor_dy(y_abs: f32, active_level: i32) -> f32 {
    y_abs - floor_y_of(active_level)
}

/// Whether a body at absolute `y_abs` is between floors (on a flight, ladder or fall).
pub fn on_link(y_abs: f32) -> bool {
    let r = y_abs / STOREY;
    (r - r.round()).abs() * STOREY > ON_FLOOR_EPS
}

/// The pure rule: an actor on the drawn level is always drawn; one on a link is drawn; otherwise it must be within
/// `range` metres (floor plane) of an opening centre and `clear` (the 3D sight test) must hold.
pub fn visible_rule(
    actor_level: i32,
    active_level: i32,
    y_abs: f32,
    pos: (f32, f32),
    openings: &[Opening],
    range: f32,
    clear: bool,
) -> bool {
    if actor_level == active_level || on_link(y_abs) {
        return true;
    }
    clear
        && openings.iter().any(|o| {
            let (dx, dz) = (o.centre.0 - pos.0, o.centre.1 - pos.1);
            dx * dx + dz * dz <= range * range
        })
}

/// Per-frame culling state, built once: the openings of the drawn floor and of every resident level.
pub struct ActorCull {
    openings: Vec<Opening>,
    range: f32,
}

impl ActorCull {
    pub fn new(g: &Game) -> ActorCull {
        let mut openings = world_mesh::openings(&g.fac);
        if let Some(i) = g.inf.as_ref() {
            for r in &i.stack.residents {
                openings.extend(world_mesh::openings(&r.fac));
            }
        }
        ActorCull { openings, range: actor_cull_range() }
    }

    /// `eye` is in the drawn frame (y relative to the active floor), like `Game::eye`.
    pub fn visible(&self, g: &Game, level: i32, y_abs: f32, pos: (f32, f32), eye: Vec3) -> bool {
        let active = g.fac.level;
        if level == active || on_link(y_abs) {
            return true;
        }
        let target = vec3(pos.0, actor_dy(y_abs, active) + AIM_HEIGHT, pos.1);
        let empty = crate::game::level_stack::LevelStack::EMPTY;
        let stack = g.inf.as_ref().map_or(&empty, |i| &i.stack);
        let floors = crate::hunters::flight_edges::Floors { active: &g.fac, stack };
        let base = floor_y_of(active);
        let clear = crate::hunters::flight_edges::sight_clear_3d(
            &floors,
            [eye.x, eye.z, eye.y + base],
            [target.x, target.z, target.y + base],
        );
        visible_rule(level, active, y_abs, pos, &self.openings, self.range, clear)
    }
}

/// Convenience form of `ActorCull::visible` for one body (builds the openings; use `ActorCull` in a loop). The body
/// is taken to stand on the floor of `level`.
pub fn actor_visible(g: &Game, level: i32, pos: (f32, f32), eye: Vec3) -> bool {
    ActorCull::new(g).visible(g, level, floor_y_of(level), pos, eye)
}

/// Joins the lists into one draw list: active hunters of the drawn level, then the resident lists, then bots.
/// One Vec, O(bodies).
pub fn build_list(
    active_level: i32,
    active: impl Iterator<Item = (usize, (f32, f32), f32)>,
    residents: impl Iterator<Item = (i32, usize, (f32, f32), f32)>,
    bots: impl Iterator<Item = (usize, i32, (f32, f32), f32)>,
) -> Vec<ActorRef> {
    let mut out = Vec::new();
    out.extend(active.map(|(index, pos, y)| ActorRef { kind: ActorKind::Hunter, level: active_level, index, pos, y }));
    out.extend(residents.map(|(level, index, pos, y)| ActorRef { kind: ActorKind::Hunter, level, index, pos, y }));
    out.extend(bots.map(|(index, level, pos, y)| ActorRef { kind: ActorKind::Bot, level, index, pos, y }));
    out
}

/// Every body to consider this frame (the caller applies `ActorCull::visible` to those off the drawn level).
pub fn draw_list(g: &Game) -> Vec<ActorRef> {
    build_list(
        g.fac.level,
        g.hunters.iter().enumerate().filter(|(_, h)| h.active).map(|(i, h)| (i, h.pos, h.y)),
        g.resident_hunters.levels().into_iter().flat_map(|l| {
            g.resident_hunters.list(l).iter().enumerate().map(move |(i, s)| (l, i, s.brain.pos, s.brain.y))
        }),
        g.npcs.iter().enumerate().map(|(i, n)| (i, n.level, n.at, n.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opening(x: f32, z: f32) -> Opening {
        Opening { cells: vec![], centre: (x, z), upper_level: 1 }
    }

    #[test]
    fn actor_dy_is_zero_on_the_active_floor_and_five_per_storey() {
        for l in [-2, 0, 3] {
            assert_eq!(actor_dy(floor_y_of(l), l), 0.0);
            assert!((actor_dy(floor_y_of(l + 1), l) - STOREY).abs() < 1e-5);
            assert!((actor_dy(floor_y_of(l - 1), l) + STOREY).abs() < 1e-5);
        }
        assert!((STOREY - 5.0).abs() < 1e-5);
    }

    #[test]
    fn a_hunter_on_a_flight_is_drawn_between_the_floors() {
        let mid = floor_y_of(0) + STOREY * 0.5;
        assert!(on_link(mid));
        assert!(visible_rule(0, 0, mid, (50.0, 50.0), &[], 14.0, false));
        assert!(visible_rule(1, 0, mid, (50.0, 50.0), &[], 14.0, false));
        assert!(!on_link(floor_y_of(2)));
        let dy = actor_dy(mid, 0);
        assert!(dy > 0.0 && dy < STOREY);
    }

    #[test]
    fn a_hunter_on_the_next_floor_is_hidden_behind_a_slab_and_shown_through_a_well() {
        let ops = [opening(10.0, 10.0)];
        let y = floor_y_of(1);
        assert!(!visible_rule(1, 0, y, (10.0, 10.0), &ops, 14.0, false), "slab blocks sight");
        assert!(visible_rule(1, 0, y, (11.0, 10.0), &ops, 14.0, true), "seen through the well");
        assert!(!visible_rule(1, 0, y, (60.0, 10.0), &ops, 14.0, true), "far from every opening");
        assert!(!visible_rule(1, 0, y, (10.0, 10.0), &[], 14.0, true), "no opening at all");
    }

    #[test]
    fn the_single_level_draw_list_equals_the_old_active_list() {
        let hs = [((1.0, 2.0), 0.0f32), ((3.0, 4.0), 0.0)];
        let list = build_list(0, hs.iter().enumerate().map(|(i, (p, y))| (i, *p, *y)), std::iter::empty(), std::iter::empty());
        assert_eq!(list.len(), 2);
        for (i, a) in list.iter().enumerate() {
            assert_eq!((a.kind, a.level, a.index, a.pos, a.y), (ActorKind::Hunter, 0, i, hs[i].0, 0.0));
            assert_eq!(actor_dy(a.y, 0), 0.0);
        }
    }

    #[test]
    fn residents_and_bots_follow_the_active_hunters() {
        let list = build_list(
            0,
            std::iter::once((0, (0.0, 0.0), 0.0)),
            std::iter::once((1, 0, (1.0, 1.0), STOREY)),
            std::iter::once((0, -1, (2.0, 2.0), -STOREY)),
        );
        let kinds: Vec<_> = list.iter().map(|a| (a.kind, a.level)).collect();
        assert_eq!(kinds, vec![(ActorKind::Hunter, 0), (ActorKind::Hunter, 1), (ActorKind::Bot, -1)]);
    }
}
