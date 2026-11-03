//! Stage 3b-iv of the Infinite route: the two post passes that `StreamedRouteGenerator._build_chunk` runs on a
//! chunk's region props after the hump pass (streamed_route_generator.gd:1022-1028):
//! - `GeometryResolve.push_out` (geometry_resolve.gd:15-51): a solid prop that a template or a wall-backed shift
//!   left poking into a wall slides out by a few centimetres, through `placement_offset`. Only the offset changes.
//! - `PlacementClearance.clear` (placement_clearance.gd:19-49): the keep-clear zones of every door, then a
//!   walk-through from each door; a prop that blocks a door, or splits off a pocket of 1 m2 or more, is removed.
//!
//! `scale_surface_mounts` (streamed_route_generator.gd:3233, in furnished.rs) runs between the two, in
//! `props::post_passes`. Neither pass draws from the RNG: both read only the grid and the props, so the stage
//! makes no SeededRandom draw. `props::post_passes` calls them in the Godot order.
//!
//! The collision side that `PlacementClearance` reads is `CollisionWorld` (collision_world.gd), ported here as
//! the subset the clean-up uses: the prop boxes (`from_chunk`, 30), the door zones (`_door_zone`, 60),
//! `rect_distance` (82), `focus` (119), `zone_violations` (227) and `_rect_touches` (237). The boxes come from
//! furnished.rs (`PropColliderBoxes.boxes`, `variant_of`), which the furnished props use already.
//!
//! Arithmetic: a Godot `Vector2` is f32 and a plain `float` is f64. A Vector2 operation is done in f32, a
//! scalar expression in f64, and the result is stored back as f32, as the golden needs. Each function says where
//! it departs from that.

use std::collections::HashSet;

use super::furnished::{boxes, interaction_for, is_surface_prop, surface_parent_covers, variant_of};
use super::layout::{Grid, Rect, Room, CHUNK, V2};
use super::props::Prop;

/// `GeometryResolve.MAX_ROUNDS` (geometry_resolve.gd:9).
const MAX_ROUNDS: usize = 4;
/// `GeometryResolve.CLEARANCE` (geometry_resolve.gd:10): the extra push past the penetration.
const CLEARANCE: f32 = 0.01;
/// `GeometryResolve.MAX_PUSH` (geometry_resolve.gd:12): a bigger move leaves the prop where it is.
const MAX_PUSH: f64 = 0.6;

/// `PlacementClearance.POCKET_AREA_LIMIT` (placement_clearance.gd:14), in m2.
const POCKET_AREA_LIMIT: f64 = 1.0;
/// `PlacementClearance.MAX_REMOVALS_PER_DOOR` (placement_clearance.gd:15).
const MAX_REMOVALS_PER_DOOR: usize = 30;

/// `CollisionWorld.MARGIN_RADIUS` (collision_world.gd:11): the walking lane's capsule radius.
const MARGIN_RADIUS: f64 = 0.3;
/// `CollisionWorld.COARSE_STEP` (collision_world.gd:15): the raster step of the generator's clean-up.
const COARSE_STEP: f64 = 0.5;

// ------------------------------------------------------------------------------------------------ vector helpers

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// `Vector2.length()` (f32).
fn length(a: [f32; 2]) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

/// `Vector2.normalized()`: zero stays zero.
fn normalized(a: [f32; 2]) -> [f32; 2] {
    let l = length(a);
    if l == 0.0 {
        [0.0, 0.0]
    } else {
        [a[0] / l, a[1] / l]
    }
}

/// `GeometryAudit._axis_x` (geometry_audit.gd:73): `Vector2(cos(yaw), -sin(yaw))`.
fn axis_x(yaw: f64) -> [f32; 2] {
    [yaw.cos() as f32, (-yaw.sin()) as f32]
}

/// `GeometryAudit._axis_y` (geometry_audit.gd:77): `Vector2(sin(yaw), cos(yaw))`.
fn axis_y(yaw: f64) -> [f32; 2] {
    [yaw.sin() as f32, yaw.cos() as f32]
}

/// `GeometryAudit.obb_push` (geometry_audit.gd:55-70): the translation that moves rectangle A just clear of
/// rectangle B along the axis of least penetration, zero when they do not overlap. Unlike `obb_depth`, it returns
/// the push vector, so it is ported here rather than shared with furnished.rs.
fn obb_push(ca: [f32; 2], ha: [f32; 2], ya: f64, cb: [f32; 2], hb: [f32; 2], yb: f64) -> [f32; 2] {
    let axes = [axis_x(ya), axis_y(ya), axis_x(yb), axis_y(yb)];
    let d = [ca[0] - cb[0], ca[1] - cb[1]];
    let mut best = f64::INFINITY;
    let mut push = [0.0f32, 0.0];
    for n in axes {
        let ra = ha[0] as f64 * dot(n, axis_x(ya)).abs() as f64 + ha[1] as f64 * dot(n, axis_y(ya)).abs() as f64;
        let rb = hb[0] as f64 * dot(n, axis_x(yb)).abs() as f64 + hb[1] as f64 * dot(n, axis_y(yb)).abs() as f64;
        let along = dot(d, n);
        let depth = ra + rb - along.abs() as f64;
        if depth <= 0.0 {
            return [0.0, 0.0];
        }
        if depth < best {
            best = depth;
            let sign = if along >= 0.0 { 1.0 } else { -1.0 };
            push = [n[0] * depth as f32 * sign, n[1] * depth as f32 * sign];
        }
    }
    push
}

// ------------------------------------------------------------------------------------------------ Rect2

/// A `Rect2` (end-exclusive `has_point`, as Godot's).
#[derive(Clone, Copy, Debug)]
struct Area {
    pos: [f32; 2],
    size: [f32; 2],
}

impl Area {
    /// `Rect2(rect)` for a `Rect2i`.
    fn of_rect(r: Rect) -> Area {
        Area { pos: [r.x as f32, r.y as f32], size: [r.w as f32, r.h as f32] }
    }

    /// `Rect2.grow(by)`: every side moves out by `by`.
    fn grow(self, by: f32) -> Area {
        Area { pos: [self.pos[0] - by, self.pos[1] - by], size: [self.size[0] + by * 2.0, self.size[1] + by * 2.0] }
    }

    fn has_point(self, p: [f32; 2]) -> bool {
        p[0] >= self.pos[0]
            && p[1] >= self.pos[1]
            && p[0] < self.pos[0] + self.size[0]
            && p[1] < self.pos[1] + self.size[1]
    }
}

// ------------------------------------------------------------------------------------------------ push_out

/// `GeometryResolve.push_out` (geometry_resolve.gd:15-51). Every solid prop that is neither a room door nor
/// wall-mounted gets up to `MAX_ROUNDS` nudges: each round tests the cells its box reaches (`reach`, a square of
/// `ceil(half length) + 1` cells round the centre), takes the largest push of any closed cell and moves the prop
/// by it plus `CLEARANCE`. A total move above `MAX_PUSH` leaves the prop as it was. Only `placement_offset`
/// changes, so the occupied cells, yaw and type stay as the placement made them.
///
/// Godot returns a new array; a prop it moves is a copy. Here the props are taken by value.
pub fn push_out(grid: &Grid, props: Vec<Prop>) -> Vec<Prop> {
    props
        .into_iter()
        .map(|mut prop| {
            if prop.room_door || !prop.solid || prop.wall_mounted {
                return prop;
            }
            // collider_size, else blocker_size (Vector2(x, z) * .5)
            let collider = prop.collider_size.unwrap_or(prop.blocker_size);
            let half = [collider[0] * 0.5, collider[2] * 0.5];
            let yaw = prop.yaw;
            let cell = prop.cell;
            let base = prop.mount_offset.unwrap_or([0.0, 0.0]);
            let placement = prop.placement_offset.unwrap_or([0.0, 0.0]);
            let mut moved = [0.0f32, 0.0];
            for _round in 0..MAX_ROUNDS {
                // Vector2(cell) + Vector2(.5, .5) + base + placement + moved, left to right
                let centre = [
                    cell.x as f32 + 0.5 + base[0] + placement[0] + moved[0],
                    cell.y as f32 + 0.5 + base[1] + placement[1] + moved[1],
                ];
                // int(ceil(half.length())) + 1
                let reach = length(half).ceil() as i32 + 1;
                let mut fix = [0.0f32, 0.0];
                for dz in -reach..=reach {
                    for dx in -reach..=reach {
                        // Vector2i(floori(centre.x) + dx, floori(centre.y) + dz)
                        let wc = V2::new(centre[0].floor() as i32 + dx, centre[1].floor() as i32 + dz);
                        if is_open(grid, wc) {
                            continue;
                        }
                        let push = obb_push(
                            centre,
                            half,
                            yaw,
                            [wc.x as f32 + 0.5, wc.y as f32 + 0.5],
                            [0.5, 0.5],
                            0.0,
                        );
                        // push.length() > absf(fix.x) + absf(fix.y): a float compare (f64)
                        if length(push) as f64 > fix[0].abs() as f64 + fix[1].abs() as f64 {
                            fix = push;
                        }
                    }
                }
                if fix == [0.0, 0.0] {
                    break;
                }
                let n = normalized(fix);
                moved = [moved[0] + (fix[0] + n[0] * CLEARANCE), moved[1] + (fix[1] + n[1] * CLEARANCE)];
            }
            if moved != [0.0, 0.0] && length(moved) as f64 <= MAX_PUSH {
                prop.placement_offset = Some([placement[0] + moved[0], placement[1] + moved[1]]);
            }
            prop
        })
        .collect()
}

/// `GeometryAudit.is_open_cell` (geometry_audit.gd:369) for the chunk grid: inside the chunk and floor.
fn is_open(grid: &Grid, cell: V2) -> bool {
    cell.x >= 0 && cell.y >= 0 && cell.x < CHUNK && cell.y < CHUNK && grid[cell.y as usize][cell.x as usize] == 0
}

// ------------------------------------------------------------------------------------------------ CollisionWorld

/// One box of a solid prop: `rects` in CollisionWorld (`id`, `type`, `center`, `half`, `yaw`, `height`); the
/// height is not read here.
#[derive(Clone, Debug)]
struct Box2 {
    id: String,
    prop_type: &'static str,
    center: [f32; 2],
    half: [f32; 2],
    yaw: f64,
}

/// A door's keep-clear zone (`door_zones`, its rect; the door id is not read by the clean-up).
struct Zone {
    area: Area,
}

/// The walk-through view of a chunk (`CollisionWorld.from_chunk` with the grid and the props). `local` is
/// `local_rects` after `focus`: indices into `rects`.
struct World<'g> {
    grid: &'g Grid,
    rects: Vec<Box2>,
    doors: Vec<Prop>,
    zones: Vec<Zone>,
    local: Vec<usize>,
    step: f64,
}

/// `PlacementClearance._world` (placement_clearance.gd:67) with `CollisionWorld.from_chunk` (collision_world.gd:30):
/// a room door becomes a door and a keep-clear zone; a solid prop gets one box per `PropColliderBoxes.boxes`,
/// placed at `cell + .5 + mount_offset + placement_offset` and turned by its yaw. Non-solid props are not boxes.
/// The raster step is `COARSE_STEP`, as the generator sets it.
fn build_world<'g>(grid: &'g Grid, props: &[Prop]) -> World<'g> {
    let mut world = World { grid, rects: Vec::new(), doors: Vec::new(), zones: Vec::new(), local: Vec::new(), step: COARSE_STEP };
    for prop in props {
        if prop.room_door {
            world.doors.push(prop.clone());
            world.zones.push(Zone { area: door_zone(prop) });
            continue;
        }
        if !prop.solid {
            continue;
        }
        let size = prop.collider_size.unwrap_or(prop.blocker_size);
        let mount = prop.mount_offset.unwrap_or([0.0, 0.0]);
        let placement = prop.placement_offset.unwrap_or([0.0, 0.0]);
        let offset = [mount[0] + placement[0], mount[1] + placement[1]];
        let at = [prop.cell.x as f32 + 0.5 + offset[0], prop.cell.y as f32 + 0.5 + offset[1]];
        for (local, half) in boxes(prop.prop_type, size, variant_of(prop)) {
            world.rects.push(Box2 {
                id: prop.id.clone(),
                prop_type: prop.prop_type,
                center: world_centre(at, local, prop.yaw),
                half,
                yaw: prop.yaw,
            });
        }
    }
    world
}

/// `PropColliderBoxes.world_centre` (prop_collider_boxes.gd): the box centre turned by the prop's yaw.
fn world_centre(at: [f32; 2], local: [f32; 2], yaw: f64) -> [f32; 2] {
    let c = yaw.cos();
    let s = yaw.sin();
    let (lx, ly) = (local[0] as f64, local[1] as f64);
    [at[0] + (lx * c + ly * s) as f32, at[1] + (-lx * s + ly * c) as f32]
}

/// `CollisionWorld._door_zone` (collision_world.gd:60): the keep-clear zone of a door, 1.5 m deep on each side of
/// the door and as wide as its opening, centred on the mean of its doorway cells.
fn door_zone(door: &Prop) -> Area {
    // inward = facing_target - cell. A door's outward cell is its negation (door_planner: inward = -outward).
    let inward = [door.facing_target.x - door.cell.x, door.facing_target.y - door.cell.y];
    let across = [inward[1].abs() as f32, inward[0].abs() as f32];
    let along = [inward[0].abs() as f32, inward[1].abs() as f32];
    let (centre, span) = if !door.doorway_cells.is_empty() {
        let n = door.doorway_cells.len() as f32;
        let mut sum = [0.0f32, 0.0];
        for cell in &door.doorway_cells {
            sum[0] += cell.x as f32 + 0.5;
            sum[1] += cell.y as f32 + 0.5;
        }
        ([sum[0] / n, sum[1] / n], n)
    } else {
        // Godot reads "door_span" (default 1) and the placement offset; a room door always has doorway cells
        // (grid.rs writes them), so this branch is the fallback only.
        let cell = door.cell;
        ([cell.x as f32 + 0.5, cell.y as f32 + 0.5], 1.0)
    };
    let k = (span as f64 * 0.5 + 0.1) as f32;
    let half = [across[0] * k + along[0] * 1.5, across[1] * k + along[1] * 1.5];
    Area { pos: [centre[0] - half[0], centre[1] - half[1]], size: [half[0] * 2.0, half[1] * 2.0] }
}

/// `CollisionWorld.rect_distance` (collision_world.gd:82): the distance from a point to a turned box, 0 inside.
fn rect_distance(point: [f32; 2], b: &Box2) -> f32 {
    let d = [point[0] - b.center[0], point[1] - b.center[1]];
    let c = b.yaw.cos();
    let s = b.yaw.sin();
    let (dx, dz) = (d[0] as f64, d[1] as f64);
    let local = [(dx * c - dz * s) as f32, (dx * s + dz * c) as f32];
    let q = [
        ((local[0].abs() as f64) - b.half[0] as f64).max(0.0) as f32,
        ((local[1].abs() as f64) - b.half[1] as f64).max(0.0) as f32,
    ];
    length(q)
}

/// `CollisionWorld._rect_touches` (collision_world.gd:237): a box touches a zone when one of its sampled points
/// (a 5 by 5 grid over the box, corners and edge midpoints included) is inside the zone. The zone is first grown
/// by the box's half length, so a far box is skipped.
fn rect_touches(prop: &Box2, zone: Area) -> bool {
    let half = prop.half;
    let centre = prop.center;
    let reach = length(half);
    let grown = zone.grow(reach);
    if !grown.has_point(centre) {
        return false;
    }
    let c = prop.yaw.cos();
    let s = prop.yaw.sin();
    for sx in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
        for sy in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            let local = [half[0] * sx, half[1] * sy];
            let (lx, lz) = (local[0] as f64, local[1] as f64);
            let world = [centre[0] + (lx * c + lz * s) as f32, centre[1] + (-lx * s + lz * c) as f32];
            if zone.has_point(world) {
                return true;
            }
        }
    }
    false
}

impl<'g> World<'g> {
    /// `CollisionWorld.zone_violations` (collision_world.gd:227): the ids of the boxes that touch a door zone.
    fn zone_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for zone in &self.zones {
            for prop in &self.rects {
                if rect_touches(prop, zone.area) {
                    out.push(prop.id.clone());
                }
            }
        }
        out
    }

    /// `PlacementClearance._has_props_in` (placement_clearance.gd:81): a box centre within 1 m of the room.
    fn has_props_in(&self, rect: Rect) -> bool {
        let area = Area::of_rect(rect).grow(1.0);
        self.rects.iter().any(|prop| area.has_point(prop.center))
    }

    /// `CollisionWorld.focus` (collision_world.gd:119): the boxes near `area` (within 1.5 m, and reaching it),
    /// which the raster of `blocking_prop` reads (`local_rects`).
    fn focus(&mut self, area: Area) {
        self.local.clear();
        let grown = area.grow(1.5);
        for (i, b) in self.rects.iter().enumerate() {
            // maxf(half.x, half.y) * 1.5
            let reach = b.half[0].max(b.half[1]) as f64 * 1.5;
            if grown.grow(reach as f32).has_point(b.center) {
                self.local.push(i);
            }
        }
    }

    /// `PlacementClearance._blocking_prop` (placement_clearance.gd:175): the id of the box to drop so that the
    /// door's room becomes walkable, or "" when it already is (or the pocket is under 1 m2). Calls `focus`.
    fn blocking_prop(&mut self, door: &Prop, rect: Rect) -> String {
        let radius = MARGIN_RADIUS;
        let cell = door.cell;
        // door.inward = facing_target - cell; outward = -inward
        let inward = [door.facing_target.x - cell.x, door.facing_target.y - cell.y];
        let outward = [-inward[0], -inward[1]];
        let area = Area {
            pos: [rect.x as f32 - 1.5, rect.y as f32 - 1.5],
            size: [rect.w as f32 + 3.0, rect.h as f32 + 3.0],
        };
        self.focus(area);
        let outside = [cell.x as f32 + 0.5 + outward[0] as f32, cell.y as f32 + 0.5 + outward[1] as f32];
        let inside = [cell.x as f32 + 0.5 + inward[0] as f32, cell.y as f32 + 0.5 + inward[1] as f32];
        let raster = self.build_raster(area, radius);
        let n = (raster.width * raster.height) as usize;
        let mut reached = vec![false; n];
        if let Some(seed) = free_near(&raster, raster.key_of(outside)) {
            reached[seed as usize] = true;
            let mut frontier = vec![seed];
            let (w, h) = (raster.width, raster.height);
            while let Some(current) = frontier.pop() {
                let cx = current % w;
                let cy = current / w;
                if cx > 0 {
                    visit(&raster, &mut reached, &mut frontier, current - 1);
                }
                if cx < w - 1 {
                    visit(&raster, &mut reached, &mut frontier, current + 1);
                }
                if cy > 0 {
                    visit(&raster, &mut reached, &mut frontier, current - w);
                }
                if cy < h - 1 {
                    visit(&raster, &mut reached, &mut frontier, current + w);
                }
            }
        }
        let mut unreached: Vec<[f32; 2]> = Vec::new();
        let mut reached_samples: Vec<[f32; 2]> = Vec::new();
        let room_area = Area::of_rect(rect);
        for key in 0..n {
            if raster.blocked[key] {
                continue;
            }
            let p = raster.point_of(key as i32);
            if !room_area.has_point(p) {
                continue;
            }
            if reached[key] {
                reached_samples.push(p);
            } else {
                unreached.push(p);
            }
        }
        let inside_key = raster.key_of(inside);
        let door_blocked = !reached[inside_key as usize] && !near_reached(&raster, &reached, inside_key);
        // float(unreached.size()) * step * step < POCKET_AREA_LIMIT
        if !door_blocked && (unreached.len() as f64) * self.step * self.step < POCKET_AREA_LIMIT {
            return String::new();
        }
        let probe: Vec<[f32; 2]> = if door_blocked || unreached.is_empty() { vec![inside] } else { unreached };
        let reach = radius + 0.55;
        let mut best = String::new();
        let mut best_score: i64 = -1;
        for prop in &self.rects {
            let near = |p: &[f32; 2]| rect_distance(*p, prop) as f64 <= reach;
            let near_blocked = probe.iter().filter(|p| near(p)).count() as i64;
            if near_blocked == 0 {
                continue;
            }
            let near_open = reached_samples.iter().filter(|p| near(p)).count() as i64;
            // mini(near_blocked, maxi(near_open, 1)) * 1000 + near_blocked
            let mut score = near_blocked.min(near_open.max(1)) * 1000 + near_blocked;
            if door_blocked {
                if rect_distance(inside, prop) as f64 <= reach {
                    score += 100_000;
                }
            } else if interaction_for(prop.prop_type) == "lay" {
                // int / int: GDScript integer division
                score /= 1000;
            }
            if score > best_score {
                best_score = score;
                best = prop.id.clone();
            }
        }
        best
    }

    /// `PlacementClearance._build_raster` (placement_clearance.gd:125): the open cells of `area`, with every wall
    /// cell and every box of `local` stamped out by `radius`, on a raster of `world.step`.
    fn build_raster(&self, area: Area, radius: f64) -> Raster {
        let mut raster = Raster::new(area, self.step);
        let x0 = area.pos[0].floor() as i32;
        let y0 = area.pos[1].floor() as i32;
        let x1 = (area.pos[0] + area.size[0]).ceil() as i32;
        let y1 = (area.pos[1] + area.size[1]).ceil() as i32;
        for cy in y0..y1 {
            for cx in x0..x1 {
                if !is_open(self.grid, V2::new(cx, cy)) {
                    raster.stamp(
                        [(cx as f64 - radius) as f32, (cy as f64 - radius) as f32],
                        [(cx as f64 + 1.0 + radius) as f32, (cy as f64 + 1.0 + radius) as f32],
                    );
                }
            }
        }
        for &i in &self.local {
            let b = &self.rects[i];
            let mut half = b.half;
            let quarter = (b.yaw * 2.0).sin().abs() < 0.02;
            if quarter {
                if b.yaw.sin().abs() > 0.7 {
                    half = [half[1], half[0]];
                }
            } else {
                // a tilted box: its bounding square
                let reach = length(half);
                half = [reach, reach];
            }
            let r = radius as f32;
            raster.stamp(
                [b.center[0] - half[0] - r, b.center[1] - half[1] - r],
                [b.center[0] + half[0] + r, b.center[1] + half[1] + r],
            );
        }
        raster
    }
}

// ------------------------------------------------------------------------------------------------ Raster

/// `PlacementClearance.Raster` (placement_clearance.gd:92): a grid of `step` metres over `area`, with blocked
/// points.
struct Raster {
    origin: [f32; 2],
    width: i32,
    height: i32,
    blocked: Vec<bool>,
    step: f64,
}

impl Raster {
    fn new(area: Area, step: f64) -> Raster {
        let width = (area.size[0] as f64 / step).ceil() as i32 + 1;
        let height = (area.size[1] as f64 / step).ceil() as i32 + 1;
        Raster {
            origin: area.pos,
            width,
            height,
            blocked: vec![false; (width * height) as usize],
            step,
        }
    }

    /// `Raster.stamp`: block the raster points inside [min, max].
    fn stamp(&mut self, min: [f32; 2], max: [f32; 2]) {
        let (ox, oy) = (self.origin[0] as f64, self.origin[1] as f64);
        let x0 = 0.max((((min[0] as f64) - ox) / self.step).floor() as i32 + 1);
        let x1 = (self.width - 1).min((((max[0] as f64) - ox) / self.step).ceil() as i32 - 1);
        let y0 = 0.max((((min[1] as f64) - oy) / self.step).floor() as i32 + 1);
        let y1 = (self.height - 1).min((((max[1] as f64) - oy) / self.step).ceil() as i32 - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.blocked[(y * self.width + x) as usize] = true;
            }
        }
    }

    /// `Raster.key_of`: the nearest raster point, clamped to the raster.
    fn key_of(&self, p: [f32; 2]) -> i32 {
        let x = ((((p[0] as f64) - self.origin[0] as f64) / self.step).round() as i32).clamp(0, self.width - 1);
        let y = ((((p[1] as f64) - self.origin[1] as f64) / self.step).round() as i32).clamp(0, self.height - 1);
        y * self.width + x
    }

    /// `Raster.point_of`: the position of a raster key.
    fn point_of(&self, key: i32) -> [f32; 2] {
        let step = self.step as f32;
        [
            self.origin[0] + (key % self.width) as f32 * step,
            self.origin[1] + (key / self.width) as f32 * step,
        ]
    }
}

/// `PlacementClearance._visit` (placement_clearance.gd:151): mark and queue an open, unreached key.
fn visit(raster: &Raster, reached: &mut [bool], frontier: &mut Vec<i32>, key: i32) {
    let k = key as usize;
    if !raster.blocked[k] && !reached[k] {
        reached[k] = true;
        frontier.push(key);
    }
}

/// `PlacementClearance._free_near` (placement_clearance.gd:157): the key itself when open, else an open
/// neighbour (the eight around it, in Godot's order), else none.
fn free_near(raster: &Raster, key: i32) -> Option<i32> {
    if !raster.blocked[key as usize] {
        return Some(key);
    }
    let w = raster.width;
    let n = raster.width * raster.height;
    for d in [1, -1, w, -w, w + 1, w - 1, -w + 1, -w - 1] {
        let k = key + d;
        if k >= 0 && k < n && !raster.blocked[k as usize] {
            return Some(k);
        }
    }
    None
}

/// `PlacementClearance._near_reached` (placement_clearance.gd:167): the key or one of its eight neighbours is
/// reached.
fn near_reached(raster: &Raster, reached: &[bool], key: i32) -> bool {
    let w = raster.width;
    let n = raster.width * raster.height;
    for d in [0, 1, -1, w, -w, w + 1, w - 1, -w + 1, -w - 1] {
        let k = key + d;
        if k >= 0 && k < n && reached[k as usize] {
            return true;
        }
    }
    false
}

// ------------------------------------------------------------------------------------------------ clear

/// `PlacementClearance.clear` (placement_clearance.gd:19-49), with the Godot order and steps:
/// 1. the keep-clear zones: every box that touches a door zone is removed (all boxes of that prop, by id);
/// 2. the walk-through, door by door: while a door's room has props and `_blocking_prop` names a culprit, remove
///    it (at most `MAX_REMOVALS_PER_DOOR`);
/// then `_drop_orphans` when anything was removed. Nothing is moved, nothing is drawn.
///
/// The grid is the one after `_remove_wall_humps`, as `_build_chunk` runs the clean-up last.
pub fn clear(grid: &Grid, rooms: &[Room], region_props: Vec<Prop>) -> Vec<Prop> {
    let original_len = region_props.len();
    let mut props = region_props;
    let mut world = build_world(grid, &props);
    // 1. keep-clear zones
    let doomed: HashSet<String> = world.zone_violations().into_iter().collect();
    if !doomed.is_empty() {
        props = without(props, &doomed);
        world = build_world(grid, &props);
    }
    // 2. walk-through, door by door (the doors of the world as it stood after step 1)
    let doors = world.doors.clone();
    for door in &doors {
        // room_by_id: the last room with the id wins, as the Godot dictionary assignment does
        let Some(room) = rooms.iter().rev().find(|r| r.id == door.room_id) else {
            continue;
        };
        let rect = room.rect;
        if !world.has_props_in(rect) {
            continue;
        }
        for _round in 0..MAX_REMOVALS_PER_DOOR {
            let culprit = world.blocking_prop(door, rect);
            if culprit.is_empty() {
                break;
            }
            let mut one = HashSet::new();
            one.insert(culprit);
            props = without(props, &one);
            world = build_world(grid, &props);
        }
    }
    if props.len() != original_len {
        drop_orphans(props)
    } else {
        props
    }
}

/// `PlacementClearance._without` (placement_clearance.gd:73): the props whose id is not doomed.
fn without(props: Vec<Prop>, doomed: &HashSet<String>) -> Vec<Prop> {
    props.into_iter().filter(|p| !doomed.contains(&p.id)).collect()
}

/// `PlacementClearance._drop_orphans` (placement_clearance.gd:51): a surface prop that rests on a parent (its
/// `mount_y` above zero) is dropped when no prop in the chunk still covers it.
fn drop_orphans(props: Vec<Prop>) -> Vec<Prop> {
    let mut kept = Vec::new();
    for prop in &props {
        if is_surface_prop(prop.prop_type) && prop.mount_y > 0.0 {
            let seated = props.iter().any(|other| surface_parent_covers(other, prop));
            if !seated {
                continue;
            }
        }
        kept.push(prop.clone());
    }
    kept
}
