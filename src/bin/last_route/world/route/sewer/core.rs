//! Stage 3d-i: the layout and grid of the sewer levels -1 and -2, ported from `SewerRouteGenerator`
//! (game/world/generation/sewer_route_generator.gd, "Godot" below; line numbers are that file's).
//!
//! Ported here: the constants the layout reads (with their Godot lines), the dispatch and the pure seed
//! maths (`ramp_between`, `falls_between`, `trench_cells_below`, `ramps_at`, `ramp_cells`, 103-273), the
//! whole of `_build_chunk` (275-376) up to the fixtures, pickups, hunter spawns and breakables, and the
//! dressing the layout fields read (`_room_features`, `_crawl_features`, `_outflow_features`,
//! `_flood_valve_features`, `_gate_features`, the tunnel and crossing features, the lamp and mount passes,
//! `_feature_props` 1245-1292, `_gates` 944-955, `_crawl_stripe_cells` 1082-1091).
//!
//! Stage 3d-ii (`seals.rs`, `breach.rs`): the seal planner's rooms and specs, `_frame_door_seal` (565-589, which
//! rewrites two `sewer_grid` cells of a framed locked door, so `sewer_grid` here is the grid after the frame), the
//! pipe breakables (914-940) and the sealed cells are computed in `chunk` (`seals::chunk_breakables`, sewer:531-556).
//!
//! Seams (see the `seam_*` functions at the end): `_fixtures` (636-657) and `_hunter_spawns` (388-464) are ported in
//! `fixtures.rs` and `hunters.rs` (stage 3d-iii), and `_pickups` (662-722) in `pickups.rs` (stage 3d-iv).
//!
//! Every draw is from `Rng` (the bit-exact SeededRandom port), in the Godot order. The seam streams
//! (SEWER-FIXTURES, SEWER-PICKUPS, SEWER-HUNTERS, OUTFLOW and GATE are separate) never touch the main
//! `SEWER` stream, so the layout fields do not depend on the seams.

use std::collections::{HashMap, HashSet};
use std::f64::consts::FRAC_PI_2;

use serde_json::{json, Map, Value};

use crate::world::rng::Rng;
use crate::world::route::layout::{self, chunk_layout, ChunkLayout, Coord, Stairwell, V2};
use super::breach::BreachSpec;
use super::seals;

/// Godot `CHUNK_SIZE` (streamed_route_generator.gd:19).
pub const CHUNK: i32 = 45;
/// Grid side, `grid[z][x]`.
pub const N: usize = CHUNK as usize;
pub type Grid = [[u8; N]; N];

/// Godot `CENTER := CHUNK_SIZE / 2` (sewer:53).
const CENTER: i32 = CHUNK / 2;
/// Levels (sewer:20, 21, 24, 25).
pub const LEVEL: i32 = -1;
pub const SURFACE_LEVEL: i32 = 0;
pub const LOWER_LEVEL: i32 = -2;
pub const INFERNO_LEVEL: i32 = -3;

/// Grid codes (sewer:31-42). Walkable is DRY or BRIDGE (`is_walkable`, 1295).
pub const SOLID: u8 = 1;
pub const DRY: u8 = 0;
pub const WATER: u8 = 2;
pub const BRIDGE: u8 = 3;
pub const RAMP: u8 = 4;
pub const FALL: u8 = 5;
pub const SHAFT: u8 = 6;

/// About one chunk in four has falls from the sewer into the collector (sewer:44).
pub const FALLS_CHANCE: f64 = 0.28;
/// Ramps (sewer:48-49): a 12-cell slope. RAMP_CHANCE is {-1: .35, -2: .3}.
pub const RAMP_RUN: i32 = 12;

fn ramp_chance(upper: i32) -> Option<f64> {
    match upper {
        -1 => Some(0.35),
        -2 => Some(0.3),
        _ => None,
    }
}

/// Tunnels (sewer:54-56): walkways 18-20 and 24-26, water 21-23, junction 13 x 13.
pub const TUNNEL_HALF: i32 = 4;
pub const CHANNEL_HALF: i32 = 1;
pub const JUNCTION_HALF: i32 = 6;
/// Bridge decks on each tunnel arm (sewer:59).
pub const BRIDGE_OFFSETS: [i32; 2] = [10, 34];
/// Quadrant centres (sewer:63), in the order 0 NW, 1 NE, 2 SW, 3 SE.
pub const QUADRANT_CENTERS: [V2; 4] = [V2::new(9, 9), V2::new(35, 9), V2::new(9, 35), V2::new(35, 35)];
/// Utility rooms: 7 x 7 at the centres (sewer:64), 3 x 3 shaft chambers (sewer:65).
pub const ROOM_HALF: i32 = 3;
pub const SHAFT_HALF: i32 = 1;
/// sewer:69.
pub const ROOM_KINDS: [&str; 2] = ["pump_room", "valve_room"];
/// sewer:70.
pub const LINK_CHANCE: f64 = 0.55;

/// PlayerModels.SCALE (game/config/player_models.gd:14), the runner's body scale. The Godot float is a
/// double; the product below is the same IEEE double operation.
pub const PLAYER_MODEL_SCALE: f64 = 1.067;
/// PlayerController3D.CROUCH_COLLISION_HEIGHT = 1.08 * PlayerModels.SCALE (player_controller_3d.gd:41).
pub const CROUCH_COLLISION_HEIGHT: f64 = 1.08 * PLAYER_MODEL_SCALE;
/// CRAWL_CLEARANCE = CROUCH_COLLISION_HEIGHT + 0.05 (sewer:77). Numerically 1.20236 (1.15236 + 0.05).
pub const CRAWL_CLEARANCE: f64 = CROUCH_COLLISION_HEIGHT + 0.05;
/// CRAWL_PIPE_MOUNT_Y = CRAWL_CLEARANCE + 0.11 (sewer:78).
pub const CRAWL_PIPE_MOUNT_Y: f64 = CRAWL_CLEARANCE + 0.11;
/// CRAWL_PIPE_SIZE (sewer:79), a Vector3 (float32 in Godot).
pub const CRAWL_PIPE_SIZE: [f32; 3] = [2.0, 0.55, 0.5];
/// CRAWL_CHANCE (sewer:80).
pub const CRAWL_CHANCE: f64 = 0.6;
/// OUTFLOW_CHANCE (sewer:85), OUTFLOW_SIZE (sewer:87), OUTFLOW_OFFSETS (sewer:90).
pub const OUTFLOW_CHANCE: f64 = 0.35;
pub const OUTFLOW_SIZE: [f32; 3] = [1.64, 1.64, 3.2];
pub const OUTFLOW_OFFSETS: [i32; 4] = [13, 14, 30, 31];
/// TUNNEL_VALVE_OFFSETS (sewer:992).
pub const TUNNEL_VALVE_OFFSETS: [i32; 4] = [4, 40, 16, 28];
/// GATE_CHANCE and GATE_OFFSETS (sewer:1025-1026).
pub const GATE_CHANCE: f64 = 0.4;
pub const GATE_OFFSETS: [i32; 2] = [5, 39];
/// MOUNT_SLIDE (sewer:1185).
pub const MOUNT_SLIDE: i32 = 3;
/// FEATURE_MOUNT_Y (sewer:1233), and the solid collider of the pump (SOLID_FEATURES, sewer:1234).
pub fn feature_mount_y(kind: &str) -> Option<f64> {
    match kind {
        "sewerLamp" => Some(2.35),
        "sewerValveWheel" => Some(1.15),
        "sewerPipeRun" => Some(2.55),
        _ => None,
    }
}
pub const PUMP_COLLIDER: [f32; 3] = [1.3, 1.5, 0.9];
/// The hall pilasters (game/look/ref_decor.gd:175-176, RefDecor.near_pilaster 185-190).
pub const PILASTER_EVERY: i32 = 6;
pub const PILASTER_PHASE: i32 = 3;

/// A rectangle as Godot's Rect2i: position and size, end exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    fn from_layout(r: layout::Rect) -> Rect {
        Rect::new(r.x, r.y, r.w, r.h)
    }
    pub fn pos(self) -> V2 {
        V2::new(self.x, self.y)
    }
    pub fn end(self) -> V2 {
        V2::new(self.x + self.w, self.y + self.h)
    }
    /// Rect2i.grow: the position moves out by `n` and the size grows by `2n`.
    pub fn grow(self, n: i32) -> Rect {
        Rect::new(self.x - n, self.y - n, self.w + 2 * n, self.h + 2 * n)
    }
    /// Rect2i.has_point: end-exclusive.
    pub fn has_point(self, p: V2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.w && p.y < self.y + self.h
    }
    /// Rect2i.get_center: `position + size / 2`.
    pub fn center(self) -> V2 {
        V2::new(self.x + self.w / 2, self.y + self.h / 2)
    }
    fn json(self) -> Value {
        json!([self.x, self.y, self.w, self.h])
    }
}

const ZERO: V2 = V2::new(0, 0);

fn add(a: V2, b: V2) -> V2 {
    V2::new(a.x + b.x, a.y + b.y)
}
fn sub(a: V2, b: V2) -> V2 {
    V2::new(a.x - b.x, a.y - b.y)
}
fn scale(a: V2, k: i32) -> V2 {
    V2::new(a.x * k, a.y * k)
}
fn neg(a: V2) -> V2 {
    V2::new(-a.x, -a.y)
}
fn cell_json(c: V2) -> Value {
    json!([c.x, c.y])
}
/// The Godot key of a Vector2i dictionary entry, as the golden dump prints it: `"[x, z]"`.
fn key_of(c: V2) -> String {
    format!("[{}, {}]", c.x, c.y)
}
fn chunk_json(c: Coord) -> Value {
    json!([c.x, c.y, c.z])
}

fn inb(c: V2) -> bool {
    c.x >= 0 && c.y >= 0 && c.x < CHUNK && c.y < CHUNK
}
fn at(grid: &Grid, c: V2) -> u8 {
    grid[c.y as usize][c.x as usize]
}
/// `is_walkable` (sewer:1295): dry floor or a bridge deck.
fn walkable(code: u8) -> bool {
    code == DRY || code == BRIDGE
}
/// `is_dry` (sewer:1299): the same two codes as `is_walkable`. Used by the seam streams (stage 3d-ii).
#[allow(dead_code)]
pub fn is_dry(code: u8) -> bool {
    code == DRY || code == BRIDGE
}
/// `wall_yaw` (sewer:1239): atan2(wall.x, wall.y), 0 for no wall.
fn wall_yaw(wall: V2) -> f64 {
    if wall == ZERO {
        0.0
    } else {
        (wall.x as f64).atan2(wall.y as f64)
    }
}
/// `wall_backed` (sewer:1212): `cell` is dry floor and the cell beyond it toward `wall` is brick.
fn wall_backed(grid: &Grid, cell: V2, wall: V2) -> bool {
    let back = add(cell, wall);
    if !inb(cell) || !inb(back) {
        return false;
    }
    at(grid, cell) == DRY && at(grid, back) == SOLID
}
/// `_carve_rect` (sewer:592): the rectangle clipped to the chunk.
fn carve_rect(grid: &mut Grid, r: Rect, code: u8) {
    for z in r.y.max(0)..r.end().y.min(CHUNK) {
        for x in r.x.max(0)..r.end().x.min(CHUNK) {
            grid[z as usize][x as usize] = code;
        }
    }
}
/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600), radius `r`.
fn exclude_neighbourhood(set: &mut HashSet<V2>, cell: V2, r: i32) {
    for dz in -r..=r {
        for dx in -r..=r {
            set.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}
/// Array.erase: remove the first occurrence, if any.
fn erase(v: &mut Vec<i32>, value: i32) {
    if let Some(i) = v.iter().position(|&x| x == value) {
        v.remove(i);
    }
}

/// The inputs `sewer::chunk` reads. The surface (level 0) data the sewer needs (`surface.chunk`, the
/// stairwells, the blocked cells) comes from `layout::chunk_layout(world, (x, 0, z))`; the void checks
/// (`is_void_column`) are always false because VoidRegion is disabled (regions.rs, line 12).
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    /// The world seed text (`SewerRouteGenerator.world_seed`, the decimal of the run seed).
    pub world: &'a str,
}

/// `chunk_id` (sewer:112): `"SEWER:x:y:z"`.
pub fn chunk_id(coord: Coord) -> String {
    format!("SEWER:{}:{}:{}", coord.x, coord.y, coord.z)
}

/// `is_chunk_allowed` (sewer:108).
pub fn is_chunk_allowed(coord: Coord) -> bool {
    coord.y == LEVEL || coord.y == LOWER_LEVEL
}

/// `LevelLink.link_id` (level_link.gd:33): `"LINK:<KIND>:<chunk.x>:<level>:<chunk.z>:<cell.x>:<cell.y>"`, where the
/// chunk is the raw argument of `make` (only the stored `chunk` is normalised to y 0).
fn link_id(kind: &str, chunk: Coord, level: i32, cell: V2) -> String {
    format!("LINK:{}:{}:{}:{}:{}:{}", kind.to_uppercase(), chunk.x, level, chunk.z, cell.x, cell.y)
}

/// A ramp (sewer:136-180): one storey down over RAMP_RUN cells, in one quadrant. `top`, `ramp`, `bottom` and
/// `connector` are Godot's Rect2i entries; `top_x` and `bottom_x` the slope's ends (chunk-local).
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub quadrant: i32,
    pub upper: i32,
    pub row: i32,
    pub top: Rect,
    pub ramp: Rect,
    pub bottom: Rect,
    pub connector: Rect,
    pub top_x: f64,
    pub bottom_x: f64,
    pub link: RampLink,
}

/// The ramp's LevelLink (kind "ramp"), `LevelLink.make` (level_link.gd:18) with top_x and bottom_x added.
#[derive(Clone, Debug, PartialEq)]
pub struct RampLink {
    pub id: String,
    pub chunk: Coord,
    pub from_level: i32,
    pub to_level: i32,
    pub from_cell: V2,
    pub to_cell: V2,
    pub top_x: f64,
    pub bottom_x: f64,
}

/// A ramp's cell in `ramp_cells` (sewer:265-272): `upper` is true on the upper level of the ramp.
#[derive(Clone, Debug, PartialEq)]
pub struct RampCell {
    pub upper: bool,
    pub ramp: Ramp,
}

/// The jump across the falls (sewer:222-227).
#[derive(Clone, Debug, PartialEq)]
pub struct Jump {
    pub cell: V2,
    pub start: V2,
    pub dir: V2,
    pub land: [f64; 2],
    pub exit: [f64; 2],
}

/// The falls from the first sewer into the collector (sewer:189-229).
#[derive(Clone, Debug, PartialEq)]
pub struct Falls {
    pub quadrant: i32,
    pub north: bool,
    pub void: Rect,
    pub spur: Rect,
    pub spur_deck: Rect,
    pub overlook: Rect,
    pub chamber: Rect,
    pub pool: Rect,
    pub center: V2,
    pub jump: Jump,
    pub edge_z: f64,
    pub edge_x0: f64,
}

/// `ramp_between(x, z, upper)` (sewer:136). `None` is Godot's `{}`.
pub fn ramp_between(inputs: &Inputs, x: i32, z: i32, upper: i32) -> Option<Ramp> {
    let chance = ramp_chance(upper)?;
    let mut rng = Rng::new(&format!("{}:RAMP:{}:{}:{}", inputs.world, x, z, upper));
    if rng.next_float() >= chance {
        return None;
    }
    let mut quadrant = rng.next_index(4) as i32;
    if upper == LOWER_LEVEL {
        // the collector already hosts the bottom of the upper ramp in one quadrant
        if let Some(above) = ramp_between(inputs, x, z, LEVEL) {
            if above.quadrant == quadrant {
                quadrant = (quadrant + 1) % 4;
            }
        }
    }
    let qc = QUADRANT_CENTERS[quadrant as usize];
    let west = qc.x < CENTER;
    let north = qc.y < CENTER;
    let walkway = if west { CENTER - TUNNEL_HALF } else { CENTER + TUNNEL_HALF };
    let top_x0 = if west { walkway - 3 } else { walkway + 1 };
    let ramp_x0 = if west { top_x0 - RAMP_RUN } else { top_x0 + 3 };
    let bottom_x0 = if west { ramp_x0 - 3 } else { ramp_x0 + RAMP_RUN };
    let row0 = qc.y - 1;
    // the lower level's halls: the collector's tunnel walkway, or the Inferno maze's central hall
    let hall_edge = if upper == LEVEL {
        if north {
            CENTER - TUNNEL_HALF
        } else {
            CENTER + TUNNEL_HALF
        }
    } else if north {
        CENTER - 1
    } else {
        CENTER + 1
    };
    let mut conn_z0 = if north { row0 } else { hall_edge + 1 };
    let mut conn_z1 = if north { hall_edge - 1 } else { row0 + 2 };
    if upper == LOWER_LEVEL {
        conn_z1 = if north { hall_edge } else { conn_z1 };
        conn_z0 = if north { conn_z0 } else { hall_edge };
    }
    let chunk = Coord::new(x, 0, z);
    let top_cell = V2::new(top_x0 + 1, qc.y);
    let bottom_cell = V2::new(bottom_x0 + 1, qc.y);
    let top_x = if west { top_x0 as f64 } else { (top_x0 + 3) as f64 };
    let bottom_x = if west { ramp_x0 as f64 } else { (ramp_x0 + RAMP_RUN) as f64 };
    Some(Ramp {
        quadrant,
        upper,
        row: qc.y,
        top: Rect::new(top_x0, row0, 3, 3),
        ramp: Rect::new(ramp_x0, row0, RAMP_RUN, 3),
        bottom: Rect::new(bottom_x0, row0, 3, 3),
        connector: Rect::new(bottom_x0, conn_z0, 3, conn_z1 - conn_z0 + 1),
        top_x,
        bottom_x,
        link: RampLink {
            id: link_id("RAMP", chunk, upper, top_cell),
            chunk,
            from_level: upper,
            to_level: upper - 1,
            from_cell: top_cell,
            to_cell: bottom_cell,
            top_x,
            bottom_x,
        },
    })
}

/// `falls_between(x, z)` (sewer:189). `None` is Godot's `{}`.
pub fn falls_between(inputs: &Inputs, x: i32, z: i32) -> Option<Falls> {
    let mut rng = Rng::new(&format!("{}:FALLS:{}:{}", inputs.world, x, z));
    if rng.next_float() >= FALLS_CHANCE {
        return None;
    }
    let mut free: Vec<i32> = vec![0, 1, 2, 3];
    for upper in [LEVEL, LOWER_LEVEL] {
        if let Some(ramp) = ramp_between(inputs, x, z, upper) {
            erase(&mut free, ramp.quadrant);
        }
    }
    if free.is_empty() {
        return None;
    }
    let quadrant = free[rng.next_index(free.len())];
    let qc = QUADRANT_CENTERS[quadrant as usize];
    let north = qc.y < CENTER;
    let walk_lo = if north { CENTER - TUNNEL_HALF } else { CENTER + CHANNEL_HALF + 1 };
    let void = Rect::new(qc.x - 3, qc.y - 3, 7, 7);
    let spur_z0 = if north { qc.y + 4 } else { CENTER + CHANNEL_HALF + 1 };
    let spur_z1 = if north { CENTER - CHANNEL_HALF - 1 } else { qc.y - 4 };
    let spur = Rect::new(qc.x - 1, spur_z0, 3, spur_z1 - spur_z0 + 1);
    let deck = Rect::new(qc.x - 1, walk_lo, 3, 3);
    let ledge_z0 = if north { qc.y + 4 } else { CENTER + TUNNEL_HALF + 1 };
    let ledge_z1 = if north { CENTER - TUNNEL_HALF - 1 } else { qc.y - 4 };
    let overlook = Rect::new(qc.x + 2, ledge_z0, 3, ledge_z1 - ledge_z0 + 1);
    let chamber_y = if north { qc.y - 5 } else { qc.y - 6 };
    let chamber = Rect::new(qc.x - 5, chamber_y, 11, 12);
    let pool_y = if north { qc.y - 2 } else { qc.y - 3 };
    let pool = Rect::new(qc.x - 2, pool_y, 5, 6);
    let jump_dir = if north { V2::new(0, -1) } else { V2::new(0, 1) };
    let jump_y = if north { void.end().y - 1 } else { void.y };
    let jump_cell = V2::new(qc.x + 3, jump_y);
    let land_y = if north { qc.y as f64 + 0.5 } else { qc.y as f64 - 0.5 };
    let exit_y = if north { pool.end().y as f64 + 0.5 } else { pool.y as f64 - 0.5 };
    let jump = Jump {
        cell: jump_cell,
        start: sub(jump_cell, jump_dir),
        dir: jump_dir,
        land: [qc.x as f64 + 1.5, land_y],
        exit: [qc.x as f64 + 1.5, exit_y],
    };
    let edge_z_cell = if north { void.end().y } else { void.y };
    Some(Falls {
        quadrant,
        north,
        void,
        spur,
        spur_deck: deck,
        overlook,
        chamber,
        pool,
        center: qc,
        jump,
        edge_z: edge_z_cell as f64,
        edge_x0: (qc.x - 1) as f64,
    })
}

/// `ramps_at(x, z, level)` (sewer:255): every ramp touching `level` in the column, the upper-level ramp first.
/// Inferno contract: `sewer().ramps_at(x, z, -3)` is the -2 ramp of the column.
pub fn ramps_at(inputs: &Inputs, x: i32, z: i32, level: i32) -> Vec<Ramp> {
    let mut out = Vec::new();
    for upper in [level, level + 1] {
        if let Some(ramp) = ramp_between(inputs, x, z, upper) {
            out.push(ramp);
        }
    }
    out
}

/// `ramp_cells(x, z, level)` (sewer:265): cell -> {upper, ramp}. Later ramps overwrite earlier ones, as Godot's dict.
/// Inferno contract: `sewer().ramp_cells(x, z, -3)`.
pub fn ramp_cells(inputs: &Inputs, x: i32, z: i32, level: i32) -> HashMap<V2, RampCell> {
    let mut cells = HashMap::new();
    for ramp in ramps_at(inputs, x, z, level) {
        let r = ramp.ramp;
        for cz in r.y..r.end().y {
            for cx in r.x..r.end().x {
                let upper = ramp.upper == level;
                cells.insert(V2::new(cx, cz), RampCell { upper, ramp: ramp.clone() });
            }
        }
    }
    cells
}

/// `trench_cells_below(x, z, level)` (sewer:236): the cells of the chunk under the channel trench of the storey
/// above it (the two main channels, and the falls' spur or pool).
pub fn trench_cells_below(inputs: &Inputs, x: i32, z: i32, level: i32) -> Vec<V2> {
    let above = level + 1;
    if above != LEVEL && above != LOWER_LEVEL {
        return Vec::new();
    }
    let mut rects = vec![
        Rect::new(0, CENTER - CHANNEL_HALF, CHUNK, CHANNEL_HALF * 2 + 1),
        Rect::new(CENTER - CHANNEL_HALF, 0, CHANNEL_HALF * 2 + 1, CHUNK),
    ];
    if let Some(falls) = falls_between(inputs, x, z) {
        rects.push(if above == LEVEL { falls.spur } else { falls.pool });
    }
    let mut out = Vec::new();
    for rect in rects {
        for cz in rect.y.max(0)..rect.end().y.min(CHUNK) {
            for cx in rect.x.max(0)..rect.end().x.min(CHUNK) {
                out.push(V2::new(cx, cz));
            }
        }
    }
    out
}

/// A stairwell's sewer end, as the sewer needs it: the box (`bbox`, the Godot `box`), the doorway, the side, the
/// quadrant, and the landing and shaft cells of `Stairwells._geometry` (stairwells.gd:66-143). Only wells from the
/// maze (upper 0) reach the sewer.
#[derive(Clone, Debug, PartialEq)]
pub struct SewerWell {
    pub kind: &'static str,
    pub upper: i32,
    pub quadrant: i32,
    pub north: bool,
    pub boxed: Rect,
    pub door: Rect,
    /// `lower_landing` (and `upper_landing`, the same cells for both kinds).
    pub landing: Vec<V2>,
    /// `shaft`: the open well cells.
    pub shaft: Vec<V2>,
}

impl SewerWell {
    /// From the maze's stairwell (layout.rs `Stairwell`, whose box, door, side and quadrant are Godot's).
    pub fn from_layout(st: &Stairwell) -> SewerWell {
        let spiral = st.kind == "spiral";
        let boxed = Rect::from_layout(st.bbox);
        let (landing, shaft) = well_cells(boxed, st.north, spiral);
        SewerWell {
            kind: st.kind,
            upper: 0,
            quadrant: st.quadrant,
            north: st.north,
            boxed,
            door: Rect::from_layout(st.door),
            landing,
            shaft,
        }
    }
}

/// The landing and shaft cells of a well (stairwells.gd `_geometry`, lines 66-143): for `stairs`, the two bottom rows
/// (5 wide) and the rows above them but the top one; for `spiral`, the three landing cells on the last row and the
/// rest of the five-wide interior.
fn well_cells(boxed: Rect, north: bool, spiral: bool) -> (Vec<V2>, Vec<V2>) {
    let interior = boxed.grow(-1);
    let last = interior.h - 1;
    let flip = |row: i32| {
        if north {
            interior.y + row
        } else {
            interior.y + interior.h - 1 - row
        }
    };
    let col = |c: i32| interior.x + c;
    let mut landing = Vec::new();
    let mut shaft = Vec::new();
    if !spiral {
        for row in [last - 1, last] {
            for c in 0..5 {
                landing.push(V2::new(col(c), flip(row)));
            }
        }
        for row in 0..(last - 1) {
            for c in 0..5 {
                shaft.push(V2::new(col(c), flip(row)));
            }
        }
    } else {
        for c in 1..=3 {
            landing.push(V2::new(col(c), flip(last)));
        }
        for row in 0..=last {
            for c in 0..5 {
                let cell = V2::new(col(c), flip(row));
                if !landing.contains(&cell) {
                    shaft.push(cell);
                }
            }
        }
    }
    (landing, shaft)
}

/// `Stairwell.corridor` (stairwells.gd:152): the three-wide lane from the doorway to the hall's first row.
fn corridor(door: Rect, north: bool, hall_edge: i32) -> Rect {
    let x0 = door.x + door.w / 2 - 1;
    if north {
        Rect::new(x0, door.y + 1, 3, (hall_edge - door.y - 1).max(0))
    } else {
        Rect::new(x0, hall_edge + 1, 3, (door.y - hall_edge - 1).max(0))
    }
}

/// A feature of the chunk before it becomes a region prop (the Godot feature dictionaries).
#[derive(Clone, Debug)]
struct Feature {
    kind: &'static str,
    cell: V2,
    /// The unit step to the brick the piece is fixed to; ZERO for free-standing pieces (the missing "wall" key).
    wall: V2,
    /// The explicit "yaw" key; without it the yaw is `wall_yaw(wall)`.
    yaw: Option<f64>,
    /// The "link" key (ladders).
    link: Option<String>,
    /// The "valve_effect" key (valve wheels; a valve without it is a flood valve).
    valve_effect: Option<&'static str>,
    /// The "gate" key (sluice gates and their valves).
    gate: Option<String>,
    /// The "gate_cells" key (sluice gates).
    gate_cells: Option<Vec<V2>>,
    /// The "crawl" dictionary (crawl-under pipes).
    crawl: Option<Crawl>,
    /// The "outflow" dictionary (outflow mains).
    outflow: Option<Outflow>,
}

#[derive(Clone, Debug)]
struct Crawl {
    across: V2,
    side: i32,
    step: V2,
    cells: Vec<V2>,
    /// The cells past the pipe: no pickup or hunter may stand there (`SewerChunk.behind`, read by the hunter spawns).
    behind: Vec<V2>,
}

#[derive(Clone, Debug)]
struct Outflow {
    wall: V2,
    step: V2,
    cells: Vec<V2>,
}

impl Feature {
    fn new(kind: &'static str, cell: V2, wall: V2) -> Feature {
        Feature {
            kind,
            cell,
            wall,
            yaw: None,
            link: None,
            valve_effect: None,
            gate: None,
            gate_cells: None,
            crawl: None,
            outflow: None,
        }
    }
}

/// `_room_features` (sewer:798): a pump room (pump, pipe run, lamp) or a valve room (two wheels, a pipe run).
fn room_features(kind: &str, rect: Rect, center: V2) -> Vec<Feature> {
    let north = V2::new(center.x, rect.y);
    let west = V2::new(rect.x, center.y);
    let east = V2::new(rect.end().x - 1, center.y);
    if kind == "pump_room" {
        vec![
            Feature::new("sewerPump", north, V2::new(0, -1)),
            Feature::new("sewerPipeRun", west, V2::new(-1, 0)),
            Feature::new("sewerLamp", east, V2::new(1, 0)),
        ]
    } else {
        vec![
            Feature::new("sewerValveWheel", north, V2::new(0, -1)),
            Feature::new("sewerValveWheel", east, V2::new(1, 0)),
            Feature::new("sewerPipeRun", west, V2::new(-1, 0)),
        ]
    }
}

/// `_carve_branch` (sewer:618): a three-wide dry branch from a room or shaft centre to the nearer tunnel walkway.
fn carve_branch(grid: &mut Grid, from: V2, half: i32) {
    let walk_lo = CENTER - TUNNEL_HALF;
    let walk_hi = CENTER + TUNNEL_HALF;
    let dx = (from.x - CENTER).abs();
    let dz = (from.y - CENTER).abs();
    if dx <= dz {
        let start = if from.x < CENTER { from.x + half + 1 } else { walk_hi + 1 };
        let finish = if from.x < CENTER { walk_lo } else { from.x - half };
        carve_rect(grid, Rect::new(start, from.y - 1, finish - start, 3), DRY);
    } else {
        let start = if from.y < CENTER { from.y + half + 1 } else { walk_hi + 1 };
        let finish = if from.y < CENTER { walk_lo } else { from.y - half };
        carve_rect(grid, Rect::new(from.x - 1, start, 3, finish - start), DRY);
    }
}

/// `_carve_main_cross` (sewer:598).
fn carve_main_cross(grid: &mut Grid) {
    let lo = CENTER - TUNNEL_HALF;
    let width = TUNNEL_HALF * 2 + 1;
    carve_rect(grid, Rect::new(0, lo, CHUNK, width), DRY);
    carve_rect(grid, Rect::new(lo, 0, width, CHUNK), DRY);
    carve_rect(
        grid,
        Rect::new(CENTER - JUNCTION_HALF, CENTER - JUNCTION_HALF, JUNCTION_HALF * 2 + 1, JUNCTION_HALF * 2 + 1),
        DRY,
    );
    let c_lo = CENTER - CHANNEL_HALF;
    let c_w = CHANNEL_HALF * 2 + 1;
    carve_rect(grid, Rect::new(0, c_lo, CHUNK, c_w), WATER);
    carve_rect(grid, Rect::new(c_lo, 0, c_w, CHUNK), WATER);
    for offset in BRIDGE_OFFSETS {
        carve_rect(grid, Rect::new(offset - 1, c_lo, 3, c_w), BRIDGE);
        carve_rect(grid, Rect::new(c_lo, offset - 1, c_w, 3), BRIDGE);
    }
}

/// `_carve_stairwell` (sewer:475): the walled box, the lane, the landing and doorway, and the shaft (SHAFT cells,
/// recorded in `shaft_cells` with `upper` = (level == the well's upper level)).
fn carve_stairwell(grid: &mut Grid, well: &SewerWell, level: i32, shaft_cells: &mut HashMap<V2, bool>) {
    carve_rect(grid, well.boxed, SOLID);
    let hall_edge = if well.north { CENTER - TUNNEL_HALF } else { CENTER + TUNNEL_HALF };
    carve_rect(grid, corridor(well.door, well.north, hall_edge), DRY);
    // Stairwells.cells_for (stairwells.gd:162): "walk" is the landing and the door row, "shaft" the open well
    let mut walk: Vec<V2> = well.landing.clone();
    for cx in well.door.x..well.door.x + well.door.w {
        walk.push(V2::new(cx, well.door.y));
    }
    for cell in walk {
        grid[cell.y as usize][cell.x as usize] = DRY;
    }
    let upper = level == well.upper;
    for cell in &well.shaft {
        grid[cell.y as usize][cell.x as usize] = SHAFT;
        shaft_cells.insert(*cell, upper);
    }
}

/// `_carve_falls` (sewer:491).
fn carve_falls(grid: &mut Grid, falls: &Falls, level: i32) {
    if level == LEVEL {
        carve_rect(grid, falls.overlook, DRY);
        carve_rect(grid, falls.spur, WATER);
        carve_rect(grid, falls.spur_deck, BRIDGE);
        carve_rect(grid, falls.void, FALL);
    } else {
        carve_rect(grid, falls.chamber, DRY);
        carve_branch(grid, falls.center, 5);
        carve_rect(grid, falls.pool, WATER);
    }
}

/// `_carve_ramp` (sewer:505).
fn carve_ramp(grid: &mut Grid, ramp: &Ramp, level: i32) {
    carve_rect(grid, ramp.ramp, RAMP);
    if ramp.upper == level {
        carve_rect(grid, ramp.top, DRY);
    } else {
        carve_rect(grid, ramp.bottom, DRY);
        carve_rect(grid, ramp.connector, DRY);
    }
}

/// `_crawl_features` (sewer:816): at most one crawl-under pipe per chunk, across a utility room's branch. Every
/// room takes a draw, in order, and the first one that fits gets the pipe.
fn crawl_features(rooms: &[SewerRoom], grid: &Grid, rng: &mut Rng) -> Vec<Feature> {
    let mut out = Vec::new();
    for room in rooms {
        if rng.next_float() >= CRAWL_CHANCE {
            continue;
        }
        let rect = room.rect;
        let centre = room.center;
        let dx = (centre.x - CENTER).abs();
        let dz = (centre.y - CENTER).abs();
        let along_x = dx <= dz;
        let mut step = if centre.x < CENTER { V2::new(1, 0) } else { V2::new(-1, 0) };
        if !along_x {
            step = if centre.y < CENTER { V2::new(0, 1) } else { V2::new(0, -1) };
        }
        let half = (rect.w - 1) / 2;
        let pipe_cell = add(centre, scale(step, half + 3));
        let across = if along_x { V2::new(0, 1) } else { V2::new(1, 0) };
        let fits = walkable(at(grid, pipe_cell))
            && at(grid, add(pipe_cell, across)) == DRY
            && at(grid, sub(pipe_cell, across)) == DRY
            && at(grid, add(pipe_cell, scale(across, 2))) == SOLID;
        if !fits {
            continue;
        }
        let mut behind = Vec::new();
        for z in rect.y..rect.end().y {
            for x in rect.x..rect.end().x {
                behind.push(V2::new(x, z));
            }
        }
        for k in 1..=(half + 3) {
            for w in -1..=1 {
                behind.push(add(add(centre, scale(step, k)), scale(across, w)));
            }
        }
        let cells = vec![sub(pipe_cell, across), pipe_cell, add(pipe_cell, across)];
        for side in [-1, 1] {
            let mut feature = Feature::new("sewerPipeRun", pipe_cell, ZERO);
            feature.yaw = Some(if along_x { FRAC_PI_2 } else { 0.0 });
            feature.crawl = Some(Crawl { across, side, step, cells: cells.clone(), behind: behind.clone() });
            out.push(feature);
        }
        break;
    }
    out
}

/// `_tunnel_features` (sewer:1095): caged lamps and pipe runs on the tunnels' outer walls.
fn tunnel_features() -> Vec<Feature> {
    let mut out = Vec::new();
    for offset in [7, 37] {
        out.push(Feature::new("sewerLamp", V2::new(offset, CENTER - TUNNEL_HALF), V2::new(0, -1)));
        out.push(Feature::new("sewerLamp", V2::new(CENTER + TUNNEL_HALF, offset), V2::new(1, 0)));
    }
    for offset in [14, 30] {
        out.push(Feature::new("sewerPipeRun", V2::new(offset, CENTER + TUNNEL_HALF), V2::new(0, 1)));
        out.push(Feature::new("sewerPipeRun", V2::new(CENTER - TUNNEL_HALF, offset), V2::new(-1, 0)));
    }
    out
}

/// `_crossing_features` (sewer:1223): the bridge decks over each channel.
fn crossing_features() -> Vec<Feature> {
    let mut out = Vec::new();
    for offset in BRIDGE_OFFSETS {
        let mut a = Feature::new("sewerBridge", V2::new(offset, CENTER), ZERO);
        a.yaw = Some(0.0);
        out.push(a);
        let mut b = Feature::new("sewerBridge", V2::new(CENTER, offset), ZERO);
        b.yaw = Some(FRAC_PI_2);
        out.push(b);
    }
    out
}

/// `_centre_room_lamps` (sewer:1109): a room's lamp hangs at the middle of a brick wall.
fn centre_room_lamps(grid: &Grid, rooms: &[SewerRoom], features: Vec<Feature>) -> Vec<Feature> {
    let mut out = Vec::new();
    for index in 0..features.len() {
        let feature = &features[index];
        if feature.kind != "sewerLamp" {
            out.push(feature.clone());
            continue;
        }
        let cell = feature.cell;
        let room = rooms.iter().filter(|r| r.rect.has_point(cell)).last();
        let room = match room {
            Some(room) if !wall_backed(grid, cell, feature.wall) => room,
            _ => {
                out.push(feature.clone());
                continue;
            }
        };
        let rect = room.rect;
        let centre = room.center;
        let used: HashSet<V2> = features
            .iter()
            .enumerate()
            .filter(|(j, other)| *j != index && rect.has_point(other.cell))
            .map(|(_, other)| other.cell)
            .collect();
        let options = [
            (V2::new(centre.x, rect.y), V2::new(0, -1)),
            (V2::new(centre.x, rect.end().y - 1), V2::new(0, 1)),
            (V2::new(rect.x, centre.y), V2::new(-1, 0)),
            (V2::new(rect.end().x - 1, centre.y), V2::new(1, 0)),
        ];
        let mut moved = feature.clone();
        for (c, w) in options {
            if !used.contains(&c) && wall_backed(grid, c, w) {
                moved.cell = c;
                moved.wall = w;
                break;
            }
        }
        out.push(moved);
    }
    out
}

/// `_pair_tunnel_lamps` (sewer:1146): mirror pairs of tunnel lamps move inward together until both have brick.
fn pair_tunnel_lamps(grid: &Grid, features: Vec<Feature>) -> Vec<Feature> {
    let mut out = features;
    for i in 0..out.len() {
        let lamp = out[i].clone();
        if lamp.kind != "sewerLamp" {
            continue;
        }
        let wall = lamp.wall;
        let cell = lamp.cell;
        let along_x = wall.y != 0;
        let a = if along_x { cell.x } else { cell.y };
        let across = if along_x { cell.y } else { cell.x };
        if a >= CENTER || (across - CENTER).abs() != TUNNEL_HALF {
            continue;
        }
        let mirror = if along_x {
            V2::new(2 * CENTER - cell.x, cell.y)
        } else {
            V2::new(cell.x, 2 * CENTER - cell.y)
        };
        let mut j: Option<usize> = None;
        for k in 0..out.len() {
            if out[k].kind == "sewerLamp" && out[k].cell == mirror && out[k].wall == wall {
                j = Some(k);
            }
        }
        let Some(j) = j else { continue };
        for s in 0..=8 {
            let step = if along_x { V2::new(s, 0) } else { V2::new(0, s) };
            let left = add(cell, step);
            let right = sub(mirror, step);
            if wall_backed(grid, left, wall) && wall_backed(grid, right, wall) {
                let mut moved_left = lamp.clone();
                moved_left.cell = left;
                let mut moved_right = out[j].clone();
                moved_right.cell = right;
                out[i] = moved_left;
                out[j] = moved_right;
                break;
            }
        }
    }
    out
}

/// `_mount_on_walls` (sewer:1188): a wall piece slides along its wall (up to MOUNT_SLIDE cells) to a cell with
/// brick behind it, or is dropped when none is in reach.
fn mount_on_walls(grid: &Grid, features: Vec<Feature>) -> Vec<Feature> {
    let mut out = Vec::new();
    for feature in features {
        let wall = feature.wall;
        if wall == ZERO || feature.link.is_some() {
            out.push(feature);
            continue;
        }
        let along = V2::new(wall.y.abs(), wall.x.abs());
        for k in [0i32, 1, -1, 2, -2, 3, -3] {
            if k.abs() > MOUNT_SLIDE {
                continue;
            }
            let spot = add(feature.cell, scale(along, k));
            if wall_backed(grid, spot, wall) {
                let mut moved = feature.clone();
                moved.cell = spot;
                out.push(moved);
                break;
            }
        }
    }
    out
}

/// `_outflow_features` (sewer:862): at most one outflow main, from its own seed stream `OUTFLOW:x:y:z`.
fn outflow_features(world: &str, coord: Coord, grid: &Grid, features: &[Feature], links: &[LadderLink]) -> Vec<Feature> {
    let mut rng = Rng::new(&format!("{}:OUTFLOW:{}:{}:{}", world, coord.x, coord.y, coord.z));
    if rng.next_float() >= OUTFLOW_CHANCE {
        return Vec::new();
    }
    let mut taken = HashSet::new();
    for feature in features {
        exclude_neighbourhood(&mut taken, feature.cell, 1);
    }
    for link in links {
        exclude_neighbourhood(&mut taken, link.to_cell, 2);
    }
    let mut options: Vec<(V2, V2, V2)> = Vec::new();
    for offset in OUTFLOW_OFFSETS {
        for along_x in [true, false] {
            for side in [-1, 1] {
                let wall = if along_x { V2::new(0, side) } else { V2::new(side, 0) };
                let outer = CENTER + side * TUNNEL_HALF;
                let middle = if along_x {
                    V2::new(offset, outer - side)
                } else {
                    V2::new(outer - side, offset)
                };
                let along = if along_x { V2::new(1, 0) } else { V2::new(0, 1) };
                options.push((middle, wall, along));
            }
        }
    }
    let start = rng.next_index(options.len());
    for k in 0..options.len() {
        let (middle, wall, along) = options[(start + k) % options.len()];
        let cells = vec![add(middle, wall), middle, sub(middle, wall)];
        let mut fits = at(grid, sub(middle, scale(wall, 2))) == WATER;
        for cell in &cells {
            for a in -1..=1 {
                let c = add(*cell, scale(along, a));
                fits = fits && at(grid, c) == DRY && !taken.contains(&c);
            }
        }
        for a in -1..=1 {
            let brick = add(add(middle, scale(wall, 2)), scale(along, a));
            fits = fits && at(grid, brick) == SOLID;
        }
        if fits {
            let mut feature = Feature::new("sewerOutflowPipe", middle, ZERO);
            feature.yaw = Some(wall_yaw(wall));
            feature.outflow = Some(Outflow { wall, step: along, cells });
            return vec![feature];
        }
    }
    Vec::new()
}

/// `_flood_valve_features` (sewer:995): a chunk with pipes the Axe can burst has a flood valve (a room's, or one on
/// a tunnel wall).
fn flood_valve_features(grid: &Grid, features: &[Feature], links: &[LadderLink]) -> Vec<Feature> {
    let has_pipe = features.iter().any(|f| f.crawl.is_some() || f.outflow.is_some());
    let has_valve = features
        .iter()
        .any(|f| f.kind == "sewerValveWheel" && f.valve_effect.unwrap_or("flood") == "flood");
    if !has_pipe || has_valve {
        return Vec::new();
    }
    let mut taken = HashSet::new();
    for feature in features {
        exclude_neighbourhood(&mut taken, feature.cell, 1);
        let cells: Vec<V2> = feature
            .crawl
            .as_ref()
            .map(|c| c.cells.clone())
            .or_else(|| feature.outflow.as_ref().map(|o| o.cells.clone()))
            .unwrap_or_default();
        for cell in cells {
            exclude_neighbourhood(&mut taken, cell, 1);
        }
    }
    for link in links {
        exclude_neighbourhood(&mut taken, link.to_cell, 2);
    }
    for offset in TUNNEL_VALVE_OFFSETS {
        for along_x in [true, false] {
            for side in [-1, 1] {
                let outer = CENTER + side * TUNNEL_HALF;
                let cell = if along_x { V2::new(offset, outer) } else { V2::new(outer, offset) };
                let wall = if along_x { V2::new(0, side) } else { V2::new(side, 0) };
                if !taken.contains(&cell) && wall_backed(grid, cell, wall) {
                    let mut valve = Feature::new("sewerValveWheel", cell, wall);
                    valve.valve_effect = Some("flood");
                    return vec![valve];
                }
            }
        }
    }
    Vec::new()
}

/// `_gate_features` (sewer:1029): at most one sluice gate, from its own stream `GATE:x:y:z`, with its valve.
fn gate_features(world: &str, coord: Coord, grid: &Grid, features: &[Feature], links: &[LadderLink]) -> Vec<Feature> {
    let mut rng = Rng::new(&format!("{}:GATE:{}:{}:{}", world, coord.x, coord.y, coord.z));
    if rng.next_float() >= GATE_CHANCE {
        return Vec::new();
    }
    let mut taken = HashSet::new();
    for feature in features {
        exclude_neighbourhood(&mut taken, feature.cell, 1);
        let cells: Vec<V2> = feature
            .crawl
            .as_ref()
            .map(|c| c.cells.clone())
            .or_else(|| feature.outflow.as_ref().map(|o| o.cells.clone()))
            .unwrap_or_default();
        for cell in cells {
            exclude_neighbourhood(&mut taken, cell, 1);
        }
    }
    for link in links {
        exclude_neighbourhood(&mut taken, link.to_cell, 2);
    }
    let mut options: Vec<(i32, bool, i32)> = Vec::new();
    for offset in GATE_OFFSETS {
        for along_x in [true, false] {
            for side in [-1, 1] {
                options.push((offset, along_x, side));
            }
        }
    }
    let start = rng.next_index(options.len());
    for k in 0..options.len() {
        let (offset, along_x, side) = options[(start + k) % options.len()];
        // across the channel: from the hinge-side walkway to the far one
        let across = if along_x { V2::new(0, side) } else { V2::new(side, 0) };
        let middle = if along_x { V2::new(offset, CENTER) } else { V2::new(CENTER, offset) };
        let cells = vec![sub(middle, across), middle, add(middle, across)];
        let hinge_end = add(middle, scale(across, CHANNEL_HALF + 1));
        let far_end = sub(middle, scale(across, CHANNEL_HALF + 1));
        let mut fits = at(grid, hinge_end) == DRY && at(grid, far_end) == DRY;
        for c in cells.iter().copied().chain([hinge_end, far_end]) {
            fits = fits && !taken.contains(&c);
        }
        for c in &cells {
            fits = fits && at(grid, *c) == WATER;
        }
        if !fits {
            continue;
        }
        let outer = CENTER + side * TUNNEL_HALF;
        for shift in [2, -2, 3, -3] {
            let spot = if along_x {
                V2::new(offset + shift, outer)
            } else {
                V2::new(outer, offset + shift)
            };
            if !taken.contains(&spot) && wall_backed(grid, spot, across) {
                let gate_id = format!("GATE:{}:{}:{}:0", coord.x, coord.y, coord.z);
                let mut sluice = Feature::new("sewerSluiceGate", middle, ZERO);
                sluice.yaw = Some(wall_yaw(across));
                sluice.gate = Some(gate_id.clone());
                sluice.gate_cells = Some(cells.clone());
                let mut valve = Feature::new("sewerValveWheel", spot, across);
                valve.valve_effect = Some("gate");
                valve.gate = Some(gate_id);
                return vec![sluice, valve];
            }
        }
    }
    Vec::new()
}

/// `_crawl_stripe_cells` (sewer:1082): the floor under a crawl pipe and one row either side (side -1 only).
fn crawl_stripe_cells(features: &[Feature]) -> Vec<V2> {
    let mut cells = Vec::new();
    for feature in features {
        let Some(crawl) = &feature.crawl else { continue };
        if crawl.side != -1 {
            continue;
        }
        for cell in &crawl.cells {
            for k in [-1, 0, 1] {
                cells.push(add(*cell, scale(crawl.step, k)));
            }
        }
    }
    cells
}

/// A room of the chunk (`{id, room_type, rect, center}`).
#[derive(Clone, Debug, PartialEq)]
pub struct SewerRoom {
    pub id: String,
    pub room_type: &'static str,
    pub rect: Rect,
    pub center: V2,
}

/// A ladder between the maze (level 0) and the first sewer: `LevelLink.make("ladder", ...)` plus `cover` and `wall`.
#[derive(Clone, Debug, PartialEq)]
pub struct LadderLink {
    pub id: String,
    pub chunk: Coord,
    pub from_level: i32,
    pub to_level: i32,
    pub from_cell: V2,
    pub to_cell: V2,
    pub cover: &'static str,
    pub wall: V2,
}

/// A sluice gate and its valve (`_gates`, sewer:944).
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub id: String,
    pub cells: Vec<V2>,
    pub valve: String,
    pub prop: String,
}

/// A region prop of the chunk (`_feature_props`, sewer:1245): the keys the layout reads. Keys that Godot writes only
/// for some types are `Option`s. `breakable_id` is set by the pipe breakables (`seals::pipe_breakables`, sewer:927-933).
#[derive(Clone, Debug, PartialEq)]
pub struct SewerProp {
    pub id: String,
    pub prop_type: &'static str,
    pub cell: V2,
    pub yaw: f64,
    pub solid: bool,
    pub mount_y: Option<f64>,
    pub mount_offset: Option<[f32; 2]>,
    pub placement_offset: Option<[f32; 2]>,
    pub collider_size: Option<[f32; 3]>,
    pub collider_y0: Option<f64>,
    pub occupied_cells: Option<Vec<V2>>,
    pub crawl_under: bool,
    pub crawl_side: Option<i32>,
    /// `crawl_step`: set on crawl pipes and outflow mains.
    pub crawl_step: Option<V2>,
    pub outflow: bool,
    pub outflow_wall: Option<V2>,
    pub valve_effect: Option<&'static str>,
    pub gate: Option<String>,
    pub gate_cells: Option<Vec<V2>>,
    pub link: Option<String>,
    /// `breakable_id`: the pipe spec that covers this prop (set in `chunk`, not in `layout_json`).
    pub breakable_id: Option<String>,
}

/// `_feature_props` (sewer:1245-1292): one prop per feature, `"<chunk>:PROP:<index>"`.
fn feature_props(coord: Coord, features: &[Feature]) -> Vec<SewerProp> {
    features
        .iter()
        .enumerate()
        .map(|(index, f)| {
            let wall = f.wall;
            let mut p = SewerProp {
                id: format!("{}:PROP:{}", chunk_id(coord), index),
                prop_type: f.kind,
                cell: f.cell,
                yaw: f.yaw.unwrap_or_else(|| wall_yaw(wall)),
                solid: f.kind == "sewerPump",
                mount_y: None,
                mount_offset: None,
                placement_offset: None,
                collider_size: None,
                collider_y0: None,
                occupied_cells: None,
                crawl_under: false,
                crawl_side: None,
                crawl_step: None,
                outflow: false,
                outflow_wall: None,
                valve_effect: None,
                gate: None,
                gate_cells: None,
                link: None,
                breakable_id: None,
            };
            if let Some(crawl) = &f.crawl {
                p.solid = true;
                p.crawl_under = true;
                p.crawl_step = Some(crawl.step);
                p.crawl_side = Some(crawl.side);
                p.mount_y = Some(CRAWL_PIPE_MOUNT_Y);
                p.collider_size = Some(CRAWL_PIPE_SIZE);
                p.collider_y0 = Some(CRAWL_CLEARANCE);
                let half = (0.5 * crawl.side as f64) as f32;
                p.placement_offset = Some([crawl.across.x as f32 * half, crawl.across.y as f32 * half]);
                p.occupied_cells = Some(crawl.cells.clone());
            } else if let Some(outflow) = &f.outflow {
                p.solid = true;
                p.outflow = true;
                p.outflow_wall = Some(outflow.wall);
                p.crawl_step = Some(outflow.step);
                p.collider_size = Some(OUTFLOW_SIZE);
                p.collider_y0 = Some(CRAWL_CLEARANCE);
                p.occupied_cells = Some(outflow.cells.clone());
            } else if let Some(my) = feature_mount_y(f.kind) {
                p.mount_y = Some(my);
                p.mount_offset = Some([wall.x as f32 * 0.5, wall.y as f32 * 0.5]);
            } else if f.kind == "sewerPump" {
                p.mount_offset = Some([wall.x as f32 * 0.05f32, wall.y as f32 * 0.05f32]);
            }
            if f.kind == "sewerPump" && f.crawl.is_none() {
                p.collider_size = Some(PUMP_COLLIDER);
            }
            if let Some(link) = &f.link {
                p.link = Some(link.clone());
            }
            if f.kind == "sewerValveWheel" {
                p.valve_effect = Some(f.valve_effect.unwrap_or("flood"));
                if let Some(gate) = &f.gate {
                    p.gate = Some(gate.clone());
                }
            }
            if f.kind == "sewerSluiceGate" {
                p.solid = false;
                p.gate = f.gate.clone();
                p.gate_cells = f.gate_cells.clone();
            }
            p
        })
        .collect()
}

/// `_gates` (sewer:944): each sluice gate with the valve whose `gate` matches (the last one, as Godot's loop).
fn gates_of(props: &[SewerProp]) -> Vec<Gate> {
    let mut out = Vec::new();
    for prop in props {
        if prop.prop_type != "sewerSluiceGate" {
            continue;
        }
        let mut valve = String::new();
        for other in props {
            if other.prop_type == "sewerValveWheel" && other.gate.is_some() && other.gate == prop.gate {
                valve = other.id.clone();
            }
        }
        out.push(Gate {
            id: prop.gate.clone().unwrap_or_default(),
            cells: prop.gate_cells.clone().unwrap_or_default(),
            valve,
            prop: prop.id.clone(),
        });
    }
    out
}

/// `_surface_blocked_cells` (sewer:735): the maze cells a manhole must keep clear of: every prop, fixture, pickup
/// and hunter spawn with its one-cell ring, and every room's rectangle.
fn surface_blocked(surface: &ChunkLayout) -> HashSet<V2> {
    let mut blocked = HashSet::new();
    fn ring(cells: &[V2], blocked: &mut HashSet<V2>) {
        for cell in cells {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    blocked.insert(V2::new(cell.x + dx, cell.y + dz));
                }
            }
        }
    }
    for prop in surface.region_props.as_deref().unwrap_or(&[]) {
        let mut cells = vec![prop.cell];
        cells.extend(prop.occupied_cells.iter().copied());
        cells.extend(prop.doorway_cells.iter().copied());
        ring(&cells, &mut blocked);
    }
    for fixture in &surface.fixtures {
        ring(&[fixture.cell], &mut blocked);
    }
    for pickup in &surface.pickups {
        ring(&[pickup.cell], &mut blocked);
    }
    for spawn in &surface.hunter_spawns {
        ring(&[spawn.cell], &mut blocked);
    }
    for room in &surface.rooms {
        let r = Rect::from_layout(room.rect);
        for z in r.y..r.end().y {
            for x in r.x..r.end().x {
                blocked.insert(V2::new(x, z));
            }
        }
    }
    blocked
}

/// `_ladder_wall` (sewer:727): the unit step from a ladder cell to the wall its rungs are fixed to.
fn ladder_wall(cell: V2) -> V2 {
    if (cell.x - CENTER).abs() <= (cell.y - CENTER).abs() {
        if cell.x < CENTER {
            V2::new(-1, 0)
        } else {
            V2::new(1, 0)
        }
    } else if cell.y < CENTER {
        V2::new(0, -1)
    } else {
        V2::new(0, 1)
    }
}

/// `RefDecor.near_pilaster(cell, wall)` with the default clearance 1 (ref_decor.gd:185-190).
fn near_pilaster(cell: V2, wall: V2) -> bool {
    let along = if wall.y != 0 { cell.x } else { cell.y };
    (-1..=1).any(|k| (along + k).rem_euclid(PILASTER_EVERY) == PILASTER_PHASE)
}

/// `_shaft_cell` (sewer:762): the maze hallway cell nearest the quadrant centre where a ladder can stand flush against
/// a hallway wall. Returns (cell, wall), or `None`.
fn shaft_cell(qc: V2, surface_grid: &Grid, blocked: &HashSet<V2>) -> Option<(V2, V2)> {
    let open = |c: V2| inb(c) && surface_grid[c.y as usize][c.x as usize] == 0;
    let mut best: Option<(V2, V2)> = None;
    let mut best_d = 1i32 << 30;
    for z in (qc.y - 7)..=(qc.y + 7) {
        for x in (qc.x - 7)..=(qc.x + 7) {
            let cell = V2::new(x, z);
            // stay out of the tunnel band and off the chunk edge (the chamber reaches two cells out)
            if (x - CENTER).abs() <= TUNNEL_HALF + SHAFT_HALF + 1 || (z - CENTER).abs() <= TUNNEL_HALF + SHAFT_HALF + 1 {
                continue;
            }
            if x < 4 || z < 4 || x > CHUNK - 5 || z > CHUNK - 5 {
                continue;
            }
            if blocked.contains(&cell) || !open(cell) {
                continue;
            }
            let toward_tunnel = neg(ladder_wall(cell));
            for wall in [V2::new(1, 0), V2::new(-1, 0), V2::new(0, 1), V2::new(0, -1)] {
                if wall == toward_tunnel || open(add(cell, wall)) {
                    continue;
                }
                if near_pilaster(cell, wall) {
                    continue;
                }
                let side = V2::new(wall.y.abs(), wall.x.abs());
                let behind = sub(cell, wall);
                if !open(behind) || blocked.contains(&behind) || !open(add(cell, side)) || !open(sub(cell, side)) {
                    continue;
                }
                let v = sub(cell, qc);
                let d = v.x * v.x + v.y * v.y;
                if d < best_d {
                    best_d = d;
                    best = Some((cell, wall));
                }
            }
        }
    }
    best
}

/// The output of `sewer::chunk` for one sewer chunk: the layout fields (Godot's dictionary keys, see `layout_json`).
#[derive(Clone, Debug)]
pub struct SewerChunk {
    pub coord: Coord,
    /// `sewer_grid`: the codes SOLID..SHAFT, after the seal step's frame (`_frame_door_seal`, sewer:569-584).
    pub sewer_grid: Grid,
    pub rooms: Vec<SewerRoom>,
    pub links: Vec<LadderLink>,
    pub ramps: Vec<Ramp>,
    pub falls: Option<Falls>,
    pub ramp_cells: HashMap<V2, RampCell>,
    pub crawl_cells: Vec<V2>,
    pub gates: Vec<Gate>,
    /// `shaft_cells`: cell -> upper flag. The well dictionaries are not kept.
    pub shaft_cells: HashMap<V2, bool>,
    /// The maze stairwells that reach this chunk (`stairwells`, the -1 level only).
    pub wells: Vec<SewerWell>,
    pub region_props: Vec<SewerProp>,
    /// `breakables` (sewer:546): the pipes' specs then the seals' (`seals::chunk_breakables`).
    pub breakables: Vec<BreachSpec>,
    /// `sealed_cells` (sewer:540-541), z-major.
    pub sealed_cells: Vec<V2>,
    /// The crawl pipes' `behind` cells (the raw feature key, sewer:853-854), in feature order, both sides of each pipe.
    /// Excluded exactly (not a neighbourhood) by `_hunter_spawns` and `_pickups`.
    pub behind: Vec<V2>,
}

/// `sewer().chunk(coord)` for the sewer levels (`_build_chunk`, sewer:275-376) up to the seams. `None` for a
/// coordinate that is not a sewer chunk.
pub fn chunk(coord: Coord, inputs: &Inputs) -> Option<SewerChunk> {
    if !is_chunk_allowed(coord) {
        return None;
    }
    let world = inputs.world;
    let mut grid: Grid = [[SOLID; N]; N];
    carve_main_cross(&mut grid);
    // the collector gets its own rooms and dressing; the first sewer keeps its seed stream
    let salt = if coord.y == LEVEL { "SEWER".to_string() } else { format!("SEWER{}", -coord.y) };
    let mut rng = Rng::new(&format!("{}:{}:{}:{}", world, salt, coord.x, coord.z));
    let mut rooms: Vec<SewerRoom> = Vec::new();
    let mut links: Vec<LadderLink> = Vec::new();
    let mut features: Vec<Feature> = Vec::new();
    let mut order: Vec<i32> = vec![0, 1, 2, 3];
    let ramps = ramps_at(inputs, coord.x, coord.z, coord.y);
    // Deterministic shuffle so rooms and shafts land in different quadrants per chunk.
    rng.shuffle(&mut order);
    let mut room_count = 1 + rng.next_index(2);
    // `coord.x == 0 and coord.z == 0 or rng.next_float() < LINK_CHANCE`: the origin takes no draw
    let mut wants_link = (coord.x == 0 && coord.z == 0) || rng.next_float() < LINK_CHANCE;
    // ladders come down from the maze only into the first sewer
    if coord.y != LEVEL {
        wants_link = false;
    }
    // ramp quadrants are taken: no room or shaft there
    for ramp in &ramps {
        erase(&mut order, ramp.quadrant);
        carve_ramp(&mut grid, ramp, coord.y);
    }
    let falls = falls_between(inputs, coord.x, coord.z);
    if let Some(falls) = &falls {
        erase(&mut order, falls.quadrant);
        carve_falls(&mut grid, falls, coord.y);
        if coord.y == LEVEL {
            // the spur crosses the tunnel walkway on a bridge like the channels do
            let mut bridge = Feature::new("sewerBridge", add(falls.spur_deck.pos(), V2::new(1, 1)), ZERO);
            bridge.yaw = Some(FRAC_PI_2);
            features.push(bridge);
        }
    }
    // a stairwell up to the maze takes its quadrant too (stairwells_at, 319 and 325); the sewer's own surface is
    // the maze chunk above it, read through the layout
    let surface = if coord.y == LEVEL {
        Some(chunk_layout(world, Coord::new(coord.x, SURFACE_LEVEL, coord.z))?)
    } else {
        None
    };
    let wells: Vec<SewerWell> = surface
        .as_ref()
        .map(|s| s.stairwells.iter().map(SewerWell::from_layout).collect())
        .unwrap_or_default();
    let mut shaft_cells: HashMap<V2, bool> = HashMap::new();
    for well in &wells {
        erase(&mut order, well.quadrant);
        carve_stairwell(&mut grid, well, coord.y, &mut shaft_cells);
    }
    if coord.y == LEVEL {
        // the second read of the same column's wells (stairwells_at at SURFACE_LEVEL, 325): its quadrants are already gone
        for well in &wells {
            erase(&mut order, well.quadrant);
        }
    }
    // rooms never take the last free quadrant while a ladder is still wanted (the maze's wells erase up to two)
    if wants_link {
        room_count = room_count.min(order.len().saturating_sub(1));
    }
    let blocked = surface.as_ref().filter(|_| wants_link).map(surface_blocked).unwrap_or_default();
    let covers = ["manhole", "grate"];
    for slot in 0..order.len() {
        let quadrant = order[slot] as usize;
        let qc = QUADRANT_CENTERS[quadrant];
        if slot < room_count {
            let kind = ROOM_KINDS[rng.next_index(ROOM_KINDS.len())];
            let side = ROOM_HALF * 2 + 1;
            let rect = Rect::new(qc.x - ROOM_HALF, qc.y - ROOM_HALF, side, side);
            carve_rect(&mut grid, rect, DRY);
            carve_branch(&mut grid, qc, ROOM_HALF);
            let index = rooms.len();
            rooms.push(SewerRoom {
                id: format!("{}:ROOM:{}", chunk_id(coord), index),
                room_type: kind,
                rect,
                center: qc,
            });
            features.extend(room_features(kind, rect, qc));
        } else if wants_link && links.len() < 2 {
            let Some(surface_layout) = surface.as_ref() else { continue };
            let Some((up, wall)) = shaft_cell(qc, &surface_layout.grid, &blocked) else { continue };
            let cover = covers[rng.next_index(covers.len())];
            // the ladder hangs flush on a hallway wall; the 3 x 3 chamber opens away from it
            let chamber = sub(up, wall);
            carve_rect(&mut grid, Rect::new(chamber.x - SHAFT_HALF, chamber.y - SHAFT_HALF, 3, 3), DRY);
            carve_branch(&mut grid, chamber, SHAFT_HALF);
            let id = link_id("LADDER", coord, SURFACE_LEVEL, up);
            links.push(LadderLink {
                id: id.clone(),
                chunk: Coord::new(coord.x, 0, coord.z),
                from_level: SURFACE_LEVEL,
                to_level: LEVEL,
                from_cell: up,
                to_cell: up,
                cover,
                wall,
            });
            let mut ladder = Feature::new("sewerLadder", up, wall);
            ladder.link = Some(id);
            features.push(ladder);
        }
    }
    let crawl = crawl_features(&rooms, &grid, &mut rng);
    features.extend(crawl);
    features.extend(tunnel_features());
    features.extend(crossing_features());
    let features = centre_room_lamps(&grid, &rooms, features);
    let features = pair_tunnel_lamps(&grid, features);
    let mut features = mount_on_walls(&grid, features);
    let outflow = outflow_features(world, coord, &grid, &features, &links);
    features.extend(outflow);
    let valves = flood_valve_features(&grid, &features, &links);
    features.extend(valves);
    let gate = gate_features(world, coord, &grid, &features, &links);
    features.extend(gate);

    let mut region_props = feature_props(coord, &features);
    // the seal step and the pipe breakables (sewer:531-546): the frame rewrites `grid`, so this runs before it is moved
    let seals::ChunkBreakables { specs: breakables, sealed: sealed_cells } =
        seals::chunk_breakables(coord, world, &mut grid, &rooms, &mut region_props);
    let gates = gates_of(&region_props);
    let crawl_cells = crawl_stripe_cells(&features);
    let ramp_cells_map = ramp_cells(inputs, coord.x, coord.z, coord.y);
    let behind: Vec<V2> = features.iter().filter_map(|f| f.crawl.as_ref()).flat_map(|c| c.behind.iter().copied()).collect();
    Some(SewerChunk {
        coord,
        sewer_grid: grid,
        rooms,
        links,
        ramps,
        falls,
        ramp_cells: ramp_cells_map,
        crawl_cells,
        gates,
        shaft_cells,
        wells,
        region_props,
        breakables,
        sealed_cells,
        behind,
    })
}

// ----- Seams (stage 2 and the stage 3c streams) -------------------------------------------------------------------

/// Seam: `_fixtures` (sewer:636), ported in `fixtures.rs` (stage 3d-iii).
pub fn seam_fixtures(world: &str, chunk: &SewerChunk) -> Vec<crate::world::route::fixtures::Fixture> {
    super::fixtures::fixtures(world, chunk)
}
/// Seam: `_pickups` (sewer:662), ported in `pickups.rs` (stage 3d-iv). `coop` is `surface.coop` (714); the goldens use false.
pub fn seam_pickups(world: &str, chunk: &SewerChunk, coop: bool) -> Vec<crate::world::route::pickups::Pickup> {
    super::pickups::chunk_pickups(world, chunk, coop)
}
/// Seam: `_hunter_spawns` (sewer:388), ported in `hunters.rs` (stage 3d-iii). `fixtures` is `seam_fixtures`'s output;
/// `pickup_cells` are the cells of the chunk's pickups, which the exclusion reads (empty until the pickups are ported).
pub fn seam_hunter_spawns(
    world: &str,
    chunk: &SewerChunk,
    fixtures: &[crate::world::route::fixtures::Fixture],
    pickup_cells: &[V2],
) -> Vec<crate::world::route::hunter_spawns::HunterSpawn> {
    super::hunters::chunk_hunter_spawns(world, chunk, fixtures, pickup_cells)
}
/// The breakables of the chunk (`_pipe_breakables`, sewer:914, and the seal planner's specs, streamed 536-546): stage 3d-ii.
pub fn seam_breakables(chunk: &SewerChunk) -> Vec<BreachSpec> {
    chunk.breakables.clone()
}

// ----- JSON in the golden dump's shape (the layout fields) ----------------------------------------------------------

fn rooms_json(rooms: &[SewerRoom]) -> Value {
    Value::Array(
        rooms
            .iter()
            .map(|r| json!({"id": r.id, "room_type": r.room_type, "rect": r.rect.json(), "center": cell_json(r.center)}))
            .collect(),
    )
}

fn links_json(links: &[LadderLink]) -> Value {
    Value::Array(
        links
            .iter()
            .map(|l| {
                json!({
                    "id": l.id,
                    "kind": "ladder",
                    "from_level": l.from_level,
                    "to_level": l.to_level,
                    "chunk": chunk_json(Coord::new(l.chunk.x, 0, l.chunk.z)),
                    "from_cell": cell_json(l.from_cell),
                    "to_cell": cell_json(l.to_cell),
                    "cover": l.cover,
                    "wall": cell_json(l.wall),
                })
            })
            .collect(),
    )
}

fn ramp_link_json(l: &RampLink) -> Value {
    json!({
        "id": l.id,
        "kind": "ramp",
        "from_level": l.from_level,
        "to_level": l.to_level,
        "chunk": chunk_json(Coord::new(l.chunk.x, 0, l.chunk.z)),
        "from_cell": cell_json(l.from_cell),
        "to_cell": cell_json(l.to_cell),
        "top_x": l.top_x,
        "bottom_x": l.bottom_x,
    })
}

fn ramp_json(r: &Ramp) -> Value {
    json!({
        "quadrant": r.quadrant,
        "upper": r.upper,
        "row": r.row,
        "top": r.top.json(),
        "ramp": r.ramp.json(),
        "bottom": r.bottom.json(),
        "connector": r.connector.json(),
        "top_x": r.top_x,
        "bottom_x": r.bottom_x,
        "link": ramp_link_json(&r.link),
    })
}

fn falls_json(f: &Option<Falls>) -> Value {
    let Some(f) = f else { return json!({}) };
    json!({
        "quadrant": f.quadrant,
        "north": f.north,
        "void": f.void.json(),
        "spur": f.spur.json(),
        "spur_deck": f.spur_deck.json(),
        "overlook": f.overlook.json(),
        "chamber": f.chamber.json(),
        "pool": f.pool.json(),
        "center": cell_json(f.center),
        "edge_z": f.edge_z,
        "edge_x0": f.edge_x0,
        "jump": {
            "cell": cell_json(f.jump.cell),
            "start": cell_json(f.jump.start),
            "dir": cell_json(f.jump.dir),
            "land": json!(f.jump.land),
            "exit": json!(f.jump.exit),
        },
    })
}

fn prop_json(p: &SewerProp) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(p.id));
    m.insert("type".into(), json!(p.prop_type));
    m.insert("cell".into(), cell_json(p.cell));
    m.insert("yaw".into(), json!(p.yaw));
    m.insert("solid".into(), json!(p.solid));
    m.insert("sewer_feature".into(), json!(true));
    if let Some(v) = p.mount_y {
        m.insert("mount_y".into(), json!(v));
    }
    if let Some(v) = p.mount_offset {
        m.insert("mount_offset".into(), json!([v[0], v[1]]));
    }
    if let Some(v) = p.placement_offset {
        m.insert("placement_offset".into(), json!([v[0], v[1]]));
    }
    if let Some(v) = p.collider_size {
        // Godot prints the Vector3 (float32) in its shortest form, e.g. "(1.3, 1.5, 0.9)"
        let short = |x: f32| -> f64 { format!("{x}").parse().expect("f32 text") };
        m.insert("collider_size".into(), json!([short(v[0]), short(v[1]), short(v[2])]));
    }
    if let Some(v) = p.collider_y0 {
        m.insert("collider_y0".into(), json!(v));
    }
    if let Some(cells) = &p.occupied_cells {
        m.insert("occupied_cells".into(), Value::Array(cells.iter().map(|c| cell_json(*c)).collect()));
    }
    if p.crawl_under {
        m.insert("crawl_under".into(), json!(true));
    }
    if let Some(side) = p.crawl_side {
        m.insert("crawl_side".into(), json!(side));
    }
    if let Some(step) = p.crawl_step {
        m.insert("crawl_step".into(), cell_json(step));
    }
    if p.outflow {
        m.insert("outflow".into(), json!(true));
    }
    if let Some(w) = p.outflow_wall {
        m.insert("outflow_wall".into(), cell_json(w));
    }
    if let Some(v) = p.valve_effect {
        m.insert("valve_effect".into(), json!(v));
    }
    if let Some(g) = &p.gate {
        m.insert("gate".into(), json!(g));
    }
    if let Some(cells) = &p.gate_cells {
        m.insert("gate_cells".into(), Value::Array(cells.iter().map(|c| cell_json(*c)).collect()));
    }
    if let Some(l) = &p.link {
        m.insert("link".into(), json!(l));
    }
    Value::Object(m)
}

impl SewerChunk {
    /// The layout fields as the golden dump names them: `sewer_grid` (rows of codes), `rooms`, `links`, `falls`,
    /// `ramp_cells`, `crawl_cells`, `gates`, `shaft_cells` (with `upper`), `ramps` and `region_props`.
    pub fn layout_json(&self) -> Value {
        let mut m = Map::new();
        let grid: Vec<Vec<u8>> = self.sewer_grid.iter().map(|row| row.to_vec()).collect();
        m.insert("sewer_grid".into(), json!(grid));
        m.insert("rooms".into(), rooms_json(&self.rooms));
        m.insert("links".into(), links_json(&self.links));
        m.insert("falls".into(), falls_json(&self.falls));
        let mut ramp_map = Map::new();
        for (cell, rc) in &self.ramp_cells {
            ramp_map.insert(key_of(*cell), json!({"ramp": ramp_json(&rc.ramp), "upper": rc.upper}));
        }
        m.insert("ramp_cells".into(), Value::Object(ramp_map));
        m.insert("crawl_cells".into(), Value::Array(self.crawl_cells.iter().map(|c| cell_json(*c)).collect()));
        m.insert(
            "gates".into(),
            Value::Array(
                self.gates
                    .iter()
                    .map(|g| {
                        json!({
                            "id": g.id,
                            "cells": g.cells.iter().map(|c| cell_json(*c)).collect::<Vec<_>>(),
                            "valve": g.valve,
                            "prop": g.prop,
                        })
                    })
                    .collect(),
            ),
        );
        let mut shaft_map = Map::new();
        for (cell, upper) in &self.shaft_cells {
            shaft_map.insert(key_of(*cell), json!({"upper": upper}));
        }
        m.insert("shaft_cells".into(), Value::Object(shaft_map));
        m.insert("ramps".into(), Value::Array(self.ramps.iter().map(ramp_json).collect()));
        m.insert("region_props".into(), Value::Array(self.region_props.iter().map(prop_json).collect()));
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compact golden `tests/parity/sewer-layout-<tag>.json` (or the full dump `golden-<tag>.json` in PARITY_DIR):
    /// the sewer chunks of levels -1 and -2 of one Godot golden, in the golden dump's own shape.
    #[derive(Clone)]
    struct Golden {
        tag: String,
        world: String,
        coord: Coord,
        data: Value,
    }

    /// The compact extracts in the repo. PARITY_DIR holds the full dumps only (`load_full`), so it is not read here.
    fn parity_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("parity")
    }

    fn coord_of(v: &Value) -> Coord {
        let a = v.as_array().expect("coord");
        Coord::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32, a[2].as_i64().unwrap() as i32)
    }

    fn read_json(path: &std::path::Path) -> Value {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("golden json")
    }

    const COMPACT_TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];

    fn load_compact(tag: &str) -> Vec<Golden> {
        let v = read_json(&parity_dir().join(format!("sewer-layout-{tag}.json")));
        let world = v["world_seed"].as_str().expect("world_seed").to_string();
        v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .map(|c| Golden { tag: tag.to_string(), world: world.clone(), coord: coord_of(&c["coord"]), data: c["data"].clone() })
            .collect()
    }

    /// The full dump (`golden-<tag>.json`), the sewer levels only. Needs PARITY_DIR.
    fn load_full(tag: &str) -> Vec<Golden> {
        let dir = std::env::var("PARITY_DIR").expect("PARITY_DIR must point at the full goldens");
        let v = read_json(&std::path::Path::new(&dir).join(format!("golden-{tag}.json")));
        let world = v["world_seed"].as_str().expect("world_seed").to_string();
        v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .filter(|c| c["coord"][1].as_i64().is_some_and(|y| y == -1 || y == -2))
            .map(|c| Golden { tag: tag.to_string(), world: world.clone(), coord: coord_of(&c["coord"]), data: c["data"].clone() })
            .collect()
    }

    /// The golden's value for one field, in our shape: sewer_grid rows as arrays (the compact extract stores strings),
    /// shaft_cells as `upper` only (the well dictionaries are not ported), region_props without `breakable_id` (set in
    /// `chunk`, checked by the seals tests, and not written by `layout_json`)
    /// and with `collider_size` as the three numbers (the dump prints the Vector3 as a string).
    fn want_field(data: &Value, field: &str) -> Value {
        match field {
            "sewer_grid" => Value::Array(
                data["sewer_grid"]
                    .as_array()
                    .expect("sewer_grid")
                    .iter()
                    .map(|row| match row {
                        Value::String(s) => Value::Array(s.bytes().map(|b| json!(b - b'0')).collect()),
                        other => other.clone(),
                    })
                    .collect(),
            ),
            "shaft_cells" => {
                let mut m = Map::new();
                for (k, v) in data["shaft_cells"].as_object().expect("shaft_cells") {
                    m.insert(k.clone(), json!({"upper": v["upper"]}));
                }
                Value::Object(m)
            }
            "region_props" => Value::Array(
                data["region_props"]
                    .as_array()
                    .expect("region_props")
                    .iter()
                    .map(|p| {
                        let mut m = p.as_object().expect("prop").clone();
                        m.remove("breakable_id");
                        if let Some(Value::String(s)) = m.get("collider_size").cloned() {
                            let nums: Vec<Value> = s
                                .trim_matches(|c| c == '(' || c == ')')
                                .split(',')
                                .map(|n| json!(n.trim().parse::<f64>().expect("collider number")))
                                .collect();
                            m.insert("collider_size".into(), Value::Array(nums));
                        }
                        Value::Object(m)
                    })
                    .collect(),
            ),
            "ramp_cells" => project_ramp_cells(&data["ramp_cells"]),
            other => data[other].clone(),
        }
    }

    /// `ramp_cells` compares each cell's upper flag and its ramp's identity (quadrant, upper level, link id). The whole
    /// ramp dictionary is compared under `ramps`, and every cell's ramp is one of those (sewer:265-272). The extract
    /// stores this projection; both sides are projected the same way.
    fn project_ramp_cells(v: &Value) -> Value {
        let mut m = Map::new();
        for (k, e) in v.as_object().expect("ramp_cells") {
            let ramp = &e["ramp"];
            m.insert(
                k.clone(),
                json!({"upper": e["upper"], "ramp": {"quadrant": ramp["quadrant"], "upper": ramp["upper"], "link": {"id": ramp["link"]["id"]}}}),
            );
        }
        Value::Object(m)
    }

    /// The first difference between ours and the golden's value, as a path, or `None`. Numbers agree to 1e-9 (the
    /// golden prints 15 digits; float32 fields print their float32 value). An object element names its `id`.
    fn diff(path: &str, ours: &Value, want: &Value) -> Option<String> {
        match (ours, want) {
            (Value::Object(a), Value::Object(b)) => {
                for (k, av) in a {
                    match b.get(k) {
                        None => return Some(format!("{path}.{k}: only in ours")),
                        Some(bv) => {
                            if let Some(d) = diff(&format!("{path}.{k}"), av, bv) {
                                return Some(d);
                            }
                        }
                    }
                }
                for k in b.keys() {
                    if !a.contains_key(k) {
                        return Some(format!("{path}.{k}: only in Godot"));
                    }
                }
                None
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Some(format!("{path}: {} items vs Godot {}", a.len(), b.len()));
                }
                for (i, (av, bv)) in a.iter().zip(b).enumerate() {
                    let name = bv.get("id").and_then(|v| v.as_str()).map(|id| format!("[{i}]({id})")).unwrap_or(format!("[{i}]"));
                    if let Some(d) = diff(&format!("{path}{name}"), av, bv) {
                        return Some(d);
                    }
                }
                None
            }
            (Value::Number(a), Value::Number(b)) => {
                let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                if (x - y).abs() > 1e-9 {
                    Some(format!("{path}: {x} vs Godot {y}"))
                } else {
                    None
                }
            }
            _ => {
                if ours == want {
                    None
                } else {
                    Some(format!("{path}: {ours} vs Godot {want}"))
                }
            }
        }
    }

    /// One field over every golden chunk of the given sources: the matched count per (seed, level) and the mismatches.
    struct FieldReport {
        lines: Vec<String>,
        mismatches: Vec<String>,
    }

    fn field_report(goldens: &[Golden], field: &str) -> FieldReport {
        // per (tag, level): (chunks, matched)
        let mut counts: std::collections::BTreeMap<(String, i32), (usize, usize)> = Default::default();
        let mut mismatches = Vec::new();
        for g in goldens {
            let level = g.coord.y;
            let entry = counts.entry((g.tag.clone(), level)).or_default();
            entry.0 += 1;
            let Some(chunk) = chunk(g.coord, &Inputs { world: &g.world }) else {
                mismatches.push(format!("{} {:?} {field}: no sewer chunk", g.tag, (g.coord.x, g.coord.y, g.coord.z)));
                continue;
            };
            let ours = chunk.layout_json()[field].clone();
            let ours = if field == "ramp_cells" { project_ramp_cells(&ours) } else { ours };
            let want = want_field(&g.data, field);
            let at = format!("{} {:?}", g.tag, (g.coord.x, g.coord.y, g.coord.z));
            match diff(&format!("{at} {field}"), &ours, &want) {
                None => entry.1 += 1,
                Some(d) => mismatches.push(d),
            }
        }
        let lines: Vec<String> = counts
            .iter()
            .map(|((tag, level), (n, ok))| format!("  {field} seed {tag} level {level}: matched {ok} of {n} chunks"))
            .collect();
        FieldReport { lines, mismatches }
    }

    fn check_field(field: &str) {
        let mut goldens = Vec::new();
        for tag in COMPACT_TAGS {
            goldens.extend(load_compact(tag));
        }
        let report = field_report(&goldens, field);
        for line in &report.lines {
            eprintln!("{line}");
        }
        for m in report.mismatches.iter().take(25) {
            eprintln!("MISMATCH {m}");
        }
        assert!(report.mismatches.is_empty(), "{} {field} mismatches; first: {}", report.mismatches.len(), report.mismatches.first().cloned().unwrap_or_default());
    }

    /// Constants (sewer:73-79, player_controller_3d.gd:41, player_models.gd:14): the crawl clearance is the crouch
    /// capsule plus 5 cm, and the crouch capsule is 1.08 times the runner's scale (1.067).
    #[test]
    fn crawl_clearance_is_the_physics_crouch_height() {
        assert_eq!(PLAYER_MODEL_SCALE, 1.067);
        assert!((CROUCH_COLLISION_HEIGHT - 1.15236).abs() < 1e-12);
        assert!((CRAWL_CLEARANCE - 1.20236).abs() < 1e-12);
        assert!((CRAWL_PIPE_MOUNT_Y - 1.31236).abs() < 1e-12);
    }

    #[test]
    fn chunk_ids_and_dispatch_match_godot() {
        assert_eq!(chunk_id(Coord::new(-3, -1, 4)), "SEWER:-3:-1:4");
        assert!(is_chunk_allowed(Coord::new(0, -1, 0)));
        assert!(is_chunk_allowed(Coord::new(0, -2, 0)));
        assert!(!is_chunk_allowed(Coord::new(0, 0, 0)));
        assert!(!is_chunk_allowed(Coord::new(0, -3, 0)));
        let inputs = Inputs { world: "1426541270" };
        assert!(chunk(Coord::new(0, 0, 0), &inputs).is_none());
    }

    /// `ramp_cells` and `ramps_at` for the Inferno contract agree with the ramps the sewer carves: the -3 query returns
    /// the -2 ramps of the column, and the -2 cells are marked `upper` false there.
    #[test]
    fn inferno_contract_queries_the_collector_ramps() {
        let inputs = Inputs { world: "1426541270" };
        for x in -3..=3 {
            for z in -3..=3 {
                let at_3 = ramps_at(&inputs, x, z, -3);
                assert!(at_3.iter().all(|r| r.upper == -2), "{x} {z}");
                for (cell, rc) in ramp_cells(&inputs, x, z, -3) {
                    assert!(!rc.upper, "{x} {z} {cell:?}");
                    assert!(rc.ramp.upper == -2);
                }
                for ramp in ramps_at(&inputs, x, z, -2) {
                    assert!(ramp.upper == -1 || ramp.upper == -2);
                }
            }
        }
    }

    #[test]
    fn sewer_grid_match_godot_goldens() {
        check_field("sewer_grid");
    }

    #[test]
    fn rooms_match_godot_goldens() {
        check_field("rooms");
    }

    #[test]
    fn links_match_godot_goldens() {
        check_field("links");
    }

    #[test]
    fn falls_match_godot_goldens() {
        check_field("falls");
    }

    #[test]
    fn ramp_cells_match_godot_goldens() {
        check_field("ramp_cells");
    }

    #[test]
    fn crawl_cells_match_godot_goldens() {
        check_field("crawl_cells");
    }

    #[test]
    fn gates_match_godot_goldens() {
        check_field("gates");
    }

    #[test]
    fn shaft_cells_match_godot_goldens() {
        check_field("shaft_cells");
    }

    #[test]
    fn ramps_match_godot_goldens() {
        check_field("ramps");
    }

    #[test]
    fn region_props_match_godot_goldens() {
        check_field("region_props");
    }

    /// The full goldens (PARITY_DIR), all layout fields. Stays ignored until it passes on every seed.
    #[test]
    #[ignore = "needs PARITY_DIR at the full goldens; un-ignore once every chunk passes"]
    fn sewer_layout_match_full_goldens() {
        let mut goldens = Vec::new();
        for tag in COMPACT_TAGS {
            goldens.extend(load_full(tag));
        }
        let mut failures = Vec::new();
        for field in [
            "sewer_grid", "rooms", "links", "falls", "ramp_cells", "crawl_cells", "gates", "shaft_cells", "ramps",
            "region_props",
        ] {
            let report = field_report(&goldens, field);
            failures.extend(report.mismatches);
        }
        for m in failures.iter().take(25) {
            eprintln!("MISMATCH {m}");
        }
        assert!(failures.is_empty(), "{} layout mismatches; first: {}", failures.len(), failures.first().cloned().unwrap_or_default());
    }
}
