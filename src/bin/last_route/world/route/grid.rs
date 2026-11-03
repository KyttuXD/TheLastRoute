//! The grid-writing passes of the Infinite route's chunk build, stage 3a: the passes of
//! `StreamedRouteGenerator._build_chunk` (`LastRoute Godot/KyttusLabApp/game/world/generation/
//! streamed_route_generator.gd`, 916-1074) that write `grid[z][x]` and read no furniture.
//!
//! Ported here, in the Godot order (`finish_level0_grid`, which `layout::chunk_layout` calls for
//! level 0 after the edge and stairwell carves):
//! - `_restore_single_room_thresholds` (2652, called at 1003 and 1016): seal each room's perimeter
//!   except its doorway, then reopen every doorway and extra doorway.
//! - `_carve_safe_room` (1712), only when `is_safe_room` (1403): the 5x5 square at the centre.
//! - `_room_door_props` (5047) and its helpers `_is_major_room` (5144), `_door_faces_hall_end`
//!   (5153), `_grid_open` (5172), `_find_room_door_threshold` (5190), `_grid_cell_open` (5285),
//!   with the region door tables `_CODE_REGION_ROOM_DOORS` (251) and
//!   `_CODE_REGION_DOUBLE_ROOM_DOORS` (259). It runs inside `_region_props` (2824, step 9 of the
//!   build), before the furniture, and writes shoulders to 1 and doorways to 0. It draws no RNG:
//!   the arch choice is `String.hash()` of the room id (djb2, `route::djb2`).
//! - `_restore_authored_room_door_thresholds` (2728, at 1020): reopen the door cells of the planned
//!   doors, the second leaf of a double door included.
//! - `_remove_wall_humps` (2676, at 1021, `REMOVE_HUMPS` true at line 121): delete wall stubs and
//!   open one cell of each diagonal pinch, never on a room perimeter or a door cell.
//!
//! Not ported, because they never write the grid: `GeometryResolve.push_out`,
//! `scale_surface_mounts`, `PlacementClearance.clear`, the furniture, fixtures, pickups and hunter
//! spawns. Stairs are empty on level 0 (`stair_links` returns []), so `_stair_door_exclusion` is
//! empty and the threshold search omits its `blocked_by_stair` test.

use std::collections::HashSet;

use super::djb2;
use super::layout::{Grid, Room, Rect, CHUNK, V2};

/// The chunk side in cells (`CHUNK_SIZE`).
const SIZE: i32 = CHUNK;

/// `RefLook.HALL_ROOMS` (look/ref_look.gd:368-370): `_is_major_room` keeps these wide.
const REF_HALL_ROOMS: [&str; 20] = [
    "drawing_room", "dining_room", "lobby", "big_top", "chapel", "crypt", "archive", "storage", "forge",
    "thermal_chamber", "lava_gallery", "altar_room", "prison_cafeteria", "midway", "restaurant_dining",
    "ward", "operating_theatre", "server_hall", "containment", "clean_room",
];
/// `RefLook.GREENHOUSE_ROOMS` (look/ref_look.gd:364).
const REF_GREENHOUSE_ROOMS: [&str; 4] = ["cove", "tide_pool", "beach_hut", "boardwalk_room"];
/// `RefLook.GLASSHOUSE_ROOMS` (look/ref_look.gd:367).
const REF_GLASSHOUSE_ROOMS: [&str; 5] = ["glasshouse", "palm_house", "cactus_room", "overgrown_room", "dome_atrium"];

/// `_CODE_REGION_ROOM_DOORS` (streamed_route_generator.gd:251) and `_CODE_REGION_DOUBLE_ROOM_DOORS`
/// (line 259) as (region, single door, double door). The mall's two names come from
/// data/regions/mall.json (`room_door` and `double_room_door`, lines 44-45), which ContentRegistry
/// resolves over the `@data` sentinel. A region not listed here has no room doors, so
/// `room_door_props` makes no door and no grid write for it.
pub const ROOM_DOOR_TABLE: [(&str, &str, &str); 6] = [
    ("maze", "normalRoomDoor", "normalDoubleRoomDoor"),
    ("apartment", "normalRoomDoor", "normalDoubleRoomDoor"),
    ("hotel", "luxuryRoomDoor", "luxuryDoubleRoomDoor"),
    ("mansion", "luxuryRoomDoor", "luxuryDoubleRoomDoor"),
    ("gothic", "gothicRoomDoor", "gothicDoubleRoomDoor"),
    ("mall", "normalRoomDoor", "normalDoubleRoomDoor"),
];

/// `WIDE_DOUBLE_DOOR_SPAN` (streamed_route_generator.gd:270).
pub const WIDE_DOUBLE_DOOR_SPAN: i32 = 3;

/// The planned room door of `_room_door_props` (5047), the fields stage 3b and the grid passes read.
/// The Godot `room_door` entry, without the furniture-side keys.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorProp {
    /// `room.id`: the door belongs to this room.
    pub room_id: String,
    /// The door's `type` (`roomArch`, or the region's single or double door).
    pub door_type: &'static str,
    pub cell: V2,
    pub door_span: i32,
    pub arch: bool,
    pub inward: V2,
    pub outward: V2,
    pub across: V2,
    /// `doorway_cells`: the cells the door opens (the canonical opening and its second leaf).
    pub doorway_cells: Vec<V2>,
    /// `threshold_wall_cells`: the wall shoulders, set to 1 in the grid.
    pub threshold_wall_cells: Vec<V2>,
}

/// The single and double door names of a region with room doors (`REGION_ROOM_DOORS` and
/// `REGION_DOUBLE_ROOM_DOORS`). `None` when the region has no room door (`_room_door_props` returns []).
fn door_table(region: &str) -> Option<(&'static str, &'static str)> {
    ROOM_DOOR_TABLE
        .iter()
        .find(|(name, _, _)| *name == region)
        .map(|(_, single, double)| (*single, *double))
}

/// `_build_chunk` (streamed_route_generator.gd:1003-1022) from the restore after the stairwell carve
/// to the hump pass, for a level-0 chunk. `safe` is `is_safe_room(coord)` (1005). The planned doors
/// are returned for stage 3b (the furniture pass reads them); the grid is the result.
pub fn finish_level0_grid(grid: &mut Grid, rooms: &[Room], region: &str, safe: bool) -> Vec<DoorProp> {
    let doors = level0_before_props(grid, rooms, region, safe);
    level0_after_props(grid, rooms, &doors);
    doors
}

/// `_build_chunk` (streamed_route_generator.gd:1003-1007), the grid as `_region_props` first sees it: the
/// rooms' thresholds, the safe room's opening, then `_room_door_props` (the door planner, which writes the
/// shoulders and the doorways). Returns the planned doors; the grid is the one the furniture reads.
pub fn level0_before_props(grid: &mut Grid, rooms: &[Room], region: &str, safe: bool) -> Vec<DoorProp> {
    // 1003: the rooms' thresholds, before the safe room and the door planner
    restore_single_room_thresholds(grid, rooms);
    // 1005-1006: the safe room's central opening
    if safe {
        carve_safe_room(grid);
    }
    // 1007: `_region_props` -> `_room_door_props`, the door planner, which writes shoulders and doorways
    room_door_props(grid, region, rooms)
}

/// `_build_chunk` (streamed_route_generator.gd:1016-1022), after `_region_props`: the second restore, the
/// authored doorway cells of the planned doors, and the hump pass.
pub fn level0_after_props(grid: &mut Grid, rooms: &[Room], doors: &[DoorProp]) {
    // 1016: the second restore, after the dressing
    restore_single_room_thresholds(grid, rooms);
    // 1020: the authored doorway cells of the planned doors (both leaves of a double door)
    restore_authored_room_door_thresholds(grid, doors);
    // 1021-1022: REMOVE_HUMPS
    remove_wall_humps(grid, rooms, doors);
}

fn is_open(grid: &Grid, c: V2) -> bool {
    c.x >= 0 && c.y >= 0 && c.x < SIZE && c.y < SIZE && grid[c.y as usize][c.x as usize] == 0
}

fn is_solid(grid: &Grid, x: i32, z: i32) -> bool {
    x < 0 || z < 0 || x >= SIZE || z >= SIZE || grid[z as usize][x as usize] != 0
}

fn put(grid: &mut Grid, c: V2, value: u8) {
    grid[c.y as usize][c.x as usize] = value;
}

fn add(a: V2, b: V2) -> V2 {
    V2::new(a.x + b.x, a.y + b.y)
}

fn scale(v: V2, k: i32) -> V2 {
    V2::new(v.x * k, v.y * k)
}

fn neg(v: V2) -> V2 {
    V2::new(-v.x, -v.y)
}

/// `Rect2i.has_point`: end-exclusive.
fn has_point(rect: Rect, p: V2) -> bool {
    p.x >= rect.x && p.y >= rect.y && p.x < rect.x + rect.w && p.y < rect.y + rect.h
}

/// `Rect2i.get_center` as `_build_chunk` uses it: `position + size / 2` (line 2748).
fn center(rect: Rect) -> V2 {
    V2::new(rect.x + rect.w / 2, rect.y + rect.h / 2)
}

/// `_seal_room_perimeter_except` (streamed_route_generator.gd:2641): a solid wall round the rectangle,
/// the doorway open.
fn seal_room_perimeter_except(grid: &mut Grid, rect: Rect, doorway: V2) {
    for x in rect.x..rect.x + rect.w {
        put(grid, V2::new(x, rect.y), 1);
        put(grid, V2::new(x, rect.y + rect.h - 1), 1);
    }
    for z in rect.y..rect.y + rect.h {
        put(grid, V2::new(rect.x, z), 1);
        put(grid, V2::new(rect.x + rect.w - 1, z), 1);
    }
    put(grid, doorway, 0);
}

/// `_restore_single_room_thresholds` (streamed_route_generator.gd:2652), called at 1003 and 1016.
/// Pass 1 reseals each ordinary room (not `stairwell`, with a doorway and a size) and opens its
/// doorway. Pass 2 reopens every primary and extra doorway, because rooms that share walls reseal
/// each other's doors.
pub fn restore_single_room_thresholds(grid: &mut Grid, rooms: &[Room]) {
    for room in rooms {
        if room.room_type == "stairwell" {
            continue;
        }
        let rect = room.rect;
        let Some(doorway) = room.doorway else { continue };
        if rect.w <= 0 || rect.h <= 0 {
            continue;
        }
        seal_room_perimeter_except(grid, rect, doorway);
    }
    for room in rooms {
        if let Some(primary) = room.doorway {
            if room.rect.w > 0 {
                put(grid, primary, 0);
            }
        }
        for extra in room.extra_doorways.iter().flatten() {
            put(grid, *extra, 0);
        }
    }
}

/// `_carve_safe_room` (streamed_route_generator.gd:1712): `grid[z][x] = 0` for the 5x5 square
/// `center - 2 .. center + 2` (center 22). Called only when `is_safe_room(coord)`.
pub fn carve_safe_room(grid: &mut Grid) {
    let center = SIZE / 2;
    for z in center - 2..center + 3 {
        for x in center - 2..center + 3 {
            put(grid, V2::new(x, z), 0);
        }
    }
}

/// `_is_major_room` (streamed_route_generator.gd:5144): the big named halls, the greenhouses and any
/// room of 13 x 13 or more keep a wide opening.
fn is_major_room(room: &Room) -> bool {
    let t = room.room_type.as_str();
    REF_HALL_ROOMS.contains(&t)
        || REF_GREENHOUSE_ROOMS.contains(&t)
        || REF_GLASSHOUSE_ROOMS.contains(&t)
        || (room.rect.w >= 13 && room.rect.h >= 13)
}

/// `_door_faces_hall_end` (streamed_route_generator.gd:5153): the hall arrives straight at the door
/// (at most three open cells across the first cell outside, at least three open cells straight out).
fn door_faces_hall_end(grid: &Grid, t: &Threshold) -> bool {
    let first = add(t.cell, t.outward);
    let mut run_across = 1;
    for sign in [-1, 1] {
        let mut k = 1;
        while k < 8 && is_open(grid, add(first, scale(t.across, sign * k))) {
            run_across += 1;
            k += 1;
        }
    }
    let mut run_out = 0;
    let mut j = 0;
    while j < 8 && is_open(grid, add(first, scale(t.outward, j))) {
        run_out += 1;
        j += 1;
    }
    run_across <= 3 && run_out >= 3
}

/// One candidate door of `_find_room_door_threshold` (streamed_route_generator.gd:5190).
#[derive(Clone, Debug)]
struct Threshold {
    cell: V2,
    inward: V2,
    outward: V2,
    across: V2,
    doorway_cells: Vec<V2>,
    wall_cells: Vec<V2>,
    score: f64,
}

/// `_find_room_door_threshold` (streamed_route_generator.gd:5190): the best door on the four sides of
/// `rect` for `requested_span` cells (1 to 3). A side must have an open doorway cell, open cells
/// inside, at the outward cell and two beyond it, and a wall shoulder at both ends. The lowest score
/// (squared distance from the doorway's centre to the room centre) wins, ties by y then x (5271-5279).
/// The stair exclusion is omitted: stairs are empty on level 0.
fn find_room_door_threshold(rect: Rect, room_center: V2, grid: &Grid, requested_span: i32) -> Option<Threshold> {
    let span = requested_span.clamp(1, 3);
    // (outward, across) for UP, DOWN, LEFT, RIGHT (Vector2i.UP etc.)
    let sides = [
        (V2::new(0, -1), V2::new(1, 0)),
        (V2::new(0, 1), V2::new(1, 0)),
        (V2::new(-1, 0), V2::new(0, 1)),
        (V2::new(1, 0), V2::new(0, 1)),
    ];
    let start = V2::new(rect.x + 1, rect.y + 1);
    let finish = V2::new(rect.x + rect.w - 1, rect.y + rect.h - 1);
    let end = V2::new(rect.x + rect.w, rect.y + rect.h);
    let mut best: Option<Threshold> = None;
    for (outward, across) in sides {
        let inward = neg(outward);
        let mut cells: Vec<V2> = Vec::new();
        if outward == V2::new(0, -1) {
            for x in start.x..finish.x {
                cells.push(V2::new(x, rect.y));
            }
        } else if outward == V2::new(0, 1) {
            for x in start.x..finish.x {
                cells.push(V2::new(x, end.y - 1));
            }
        } else if outward == V2::new(-1, 0) {
            for z in start.y..finish.y {
                cells.push(V2::new(rect.x, z));
            }
        } else {
            for z in start.y..finish.y {
                cells.push(V2::new(end.x - 1, z));
            }
        }
        for cell in cells {
            let doorway_cells: Vec<V2> = (0..span).map(|i| add(cell, scale(across, i))).collect();
            // at least one cell is the route's existing opening
            if !doorway_cells.iter().any(|at| is_open(grid, *at)) {
                continue;
            }
            let mut lanes_clear = true;
            for doorway in &doorway_cells {
                let inward_cell = add(*doorway, inward);
                let outside_cell = add(*doorway, outward);
                let corridor_cell = add(*doorway, scale(outward, 2));
                if !has_point(rect, *doorway)
                    || !has_point(rect, inward_cell)
                    || has_point(rect, outside_cell)
                    || !is_open(grid, inward_cell)
                    || !is_open(grid, outside_cell)
                    || !is_open(grid, corridor_cell)
                {
                    lanes_clear = false;
                    break;
                }
            }
            if !lanes_clear {
                continue;
            }
            let mut wall_cells: Vec<V2> = Vec::new();
            for offset in 1..3 {
                for shoulder in [add(cell, scale(across, -offset)), add(cell, scale(across, span - 1 + offset))] {
                    if has_point(rect, shoulder) {
                        wall_cells.push(shoulder);
                    }
                }
            }
            // a real doorway needs a wall shoulder immediately outside both ends
            if !wall_cells.contains(&add(cell, neg(across))) || !wall_cells.contains(&add(cell, scale(across, span))) {
                continue;
            }
            let mid = cell.x as f64 + across.x as f64 * (span - 1) as f64 * 0.5;
            let mid_y = cell.y as f64 + across.y as f64 * (span - 1) as f64 * 0.5;
            let dx = mid - room_center.x as f64;
            let dy = mid_y - room_center.y as f64;
            let candidate = Threshold {
                cell,
                inward,
                outward,
                across,
                doorway_cells,
                wall_cells,
                score: dx * dx + dy * dy,
            };
            let better = match &best {
                None => true,
                Some(b) => {
                    candidate.score < b.score
                        || (candidate.score == b.score
                            && (candidate.cell.y < b.cell.y
                                || (candidate.cell.y == b.cell.y && candidate.cell.x < b.cell.x)))
                }
            };
            if better {
                best = Some(candidate);
            }
        }
    }
    best
}

/// `_room_door_props` (streamed_route_generator.gd:5047), the grid writes only. Walks the rooms in
/// order (at most 20 accepted doors, lobbies and stairwells skipped), picks each room's door, claims
/// its doorway cells (a doorway shared with an earlier room is skipped), sets the shoulders to 1 and
/// the doorway cells to 0. The door's size and arch follow 5068-5097; a double door on a
/// WIDE_DOUBLE_DOOR_REGIONS region is three cells wide, two when no three-cell threshold fits.
pub fn room_door_props(grid: &mut Grid, region: &str, rooms: &[Room]) -> Vec<DoorProp> {
    let Some((single, double)) = door_table(region) else {
        return Vec::new();
    };
    let mut result: Vec<DoorProp> = Vec::new();
    let mut claimed: HashSet<V2> = HashSet::new();
    for room in rooms {
        // one preset suite owns an exterior door plus four internal doors (dense: 16 rooms a chunk)
        if result.len() >= 20 {
            break;
        }
        if room.room_type == "lobby" || room.room_type == "stairwell" {
            continue;
        }
        let rect = room.rect;
        let suite_child = room.suite_id.is_some() && !room.suite_parent;
        let major = is_major_room(room);
        let mut use_double = room.suite_parent || (!suite_child && (rect.w >= 4 || rect.h >= 4));
        // Roomy major rooms open onto the hall through a three-cell arch on hash % 100 < 65
        let mut arch = major
            && room.suite_id.is_none()
            && rect.w >= 7
            && rect.h >= 7
            && (djb2(&room.id) % 100) < 65;
        let mut door_span = if arch {
            3
        } else if use_double {
            2
        } else {
            1
        };
        // every double-door region fills the three-wide hall; `wide_double` is never an arch
        let wide_double = use_double && !arch;
        if wide_double {
            door_span = WIDE_DOUBLE_DOOR_SPAN;
        }
        let room_center = center(rect);
        let mut threshold = find_room_door_threshold(rect, room_center, grid, door_span);
        if threshold.is_none() && wide_double {
            door_span = 2;
            threshold = find_room_door_threshold(rect, room_center, grid, door_span);
        }
        let Some(mut t) = threshold else { continue };
        // a hallway is three wide; a room at the END of one gets a single door
        if door_span > 1 && door_faces_hall_end(grid, &t) {
            arch = false;
            use_double = false;
            door_span = 1;
            match find_room_door_threshold(rect, room_center, grid, 1) {
                Some(single_door) => t = single_door,
                None => continue,
            }
        }
        let door_type = if arch {
            "roomArch"
        } else if use_double {
            double
        } else {
            single
        };
        if t.doorway_cells.iter().any(|c| claimed.contains(c)) {
            continue;
        }
        for c in &t.doorway_cells {
            claimed.insert(*c);
        }
        for shoulder in &t.wall_cells {
            put(grid, *shoulder, 1);
        }
        for doorway in &t.doorway_cells {
            put(grid, *doorway, 0);
        }
        result.push(DoorProp {
            room_id: room.id.clone(),
            door_type,
            cell: t.cell,
            door_span,
            arch,
            inward: t.inward,
            outward: t.outward,
            across: t.across,
            doorway_cells: t.doorway_cells,
            threshold_wall_cells: t.wall_cells,
        });
    }
    result
}

/// `_restore_authored_room_door_thresholds` (streamed_route_generator.gd:2728), at 1020: each planned
/// door's `doorway_cells` (the fallback `cell`) open again. Cells outside the chunk are skipped.
pub fn restore_authored_room_door_thresholds(grid: &mut Grid, doors: &[DoorProp]) {
    for door in doors {
        let cells: Vec<V2> = if door.doorway_cells.is_empty() {
            vec![door.cell]
        } else {
            door.doorway_cells.clone()
        };
        for c in cells {
            if c.x < 0 || c.y < 0 || c.x >= SIZE || c.y >= SIZE {
                continue;
            }
            put(grid, c, 0);
        }
    }
}

/// `_remove_wall_humps` (streamed_route_generator.gd:2676), at 1021 with `REMOVE_HUMPS` true. Protected
/// cells are every room perimeter cell and every door doorway cell. Three passes over `z, x` in
/// `2..43`, in place, each cell tested live against the grid:
/// (a) a solid, unprotected cell with exactly one solid 4-neighbour, and solid cells on both sides of
///     that neighbour, is opened (a one-cell stub);
/// (b) a solid cell with a solid diagonal `(+1,+1)` or `(-1,+1)`, whose two orthogonal cells are open,
///     opens itself if unprotected, else the diagonal cell if that is unprotected.
/// Stops after a pass with no change.
pub fn remove_wall_humps(grid: &mut Grid, rooms: &[Room], doors: &[DoorProp]) {
    let mut protected: HashSet<V2> = HashSet::new();
    for room in rooms {
        let rect = room.rect;
        for x in rect.x..rect.x + rect.w {
            protected.insert(V2::new(x, rect.y));
            protected.insert(V2::new(x, rect.y + rect.h - 1));
        }
        for z in rect.y..rect.y + rect.h {
            protected.insert(V2::new(rect.x, z));
            protected.insert(V2::new(rect.x + rect.w - 1, z));
        }
    }
    for door in doors {
        for c in &door.doorway_cells {
            protected.insert(*c);
        }
    }
    const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for _pass in 0..3 {
        let mut changed = false;
        for z in 2..SIZE - 2 {
            for x in 2..SIZE - 2 {
                let here = is_solid(grid, x, z);
                let me = V2::new(x, z);
                if here && !protected.contains(&me) {
                    let attached: Vec<(i32, i32)> = DIRS
                        .iter()
                        .copied()
                        .filter(|(dx, dz)| is_solid(grid, x + dx, z + dz))
                        .collect();
                    if attached.len() == 1 {
                        let (dx, dz) = attached[0];
                        let (nx, nz) = (x + dx, z + dz);
                        // the side pair is (d.y, d.x)
                        let (sx, sz) = (dz, dx);
                        if is_solid(grid, nx + sx, nz + sz) && is_solid(grid, nx - sx, nz - sz) {
                            put(grid, me, 0);
                            changed = true;
                            continue;
                        }
                    }
                }
                if here && is_solid(grid, x + 1, z + 1) && !is_solid(grid, x + 1, z) && !is_solid(grid, x, z + 1) {
                    if !protected.contains(&me) {
                        put(grid, me, 0);
                        changed = true;
                    } else if !protected.contains(&V2::new(x + 1, z + 1)) {
                        put(grid, V2::new(x + 1, z + 1), 0);
                        changed = true;
                    }
                } else if here
                    && is_solid(grid, x - 1, z + 1)
                    && !is_solid(grid, x - 1, z)
                    && !is_solid(grid, x, z + 1)
                {
                    if !protected.contains(&me) {
                        put(grid, me, 0);
                        changed = true;
                    } else if !protected.contains(&V2::new(x - 1, z + 1)) {
                        put(grid, V2::new(x - 1, z + 1), 0);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::layout::chunk_layout;
    use crate::world::route::layout::Coord;
    use serde_json::Value;

    /// `tests/parity/grid-<tag>.json`: the level-0 grids of one Godot golden, 45 rows of 45 characters
    /// (`1` solid, `0` open), per chunk, with the golden's world seed.
    fn load_grid_golden(tag: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("parity")
            .join(format!("grid-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("grid golden json")
    }

    fn rows_of(grid: &Grid) -> Vec<String> {
        grid.iter()
            .map(|row| row.iter().map(|v| if *v == 0 { '0' } else { '1' }).collect())
            .collect()
    }

    /// The first cell (x, z) where the two grids differ: (x, z, ours, golden).
    fn first_mismatch(ours: &[String], golden: &[String]) -> Option<(usize, usize, char, char)> {
        for (z, (a, b)) in ours.iter().zip(golden).enumerate() {
            for (x, (p, q)) in a.chars().zip(b.chars()).enumerate() {
                if p != q {
                    return Some((x, z, p, q));
                }
            }
        }
        None
    }

    #[test]
    fn region_door_table_matches_the_mall_data_file() {
        let mall: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/regions/mall.json"))).unwrap();
        let mall_row = ROOM_DOOR_TABLE.iter().find(|(name, _, _)| *name == "mall").unwrap();
        assert_eq!(mall["room_door"].as_str(), Some(mall_row.1));
        assert_eq!(mall["double_room_door"].as_str(), Some(mall_row.2));
        assert_eq!(door_table("lab"), None);
        assert_eq!(door_table("hotel"), Some(("luxuryRoomDoor", "luxuryDoubleRoomDoor")));
    }

    /// Every level-0 chunk of the three Godot goldens: the Rust grid after the stage-3a passes is
    /// compared cell by cell with the golden grid. A mismatch is reported with its first cell.
    #[test]
    fn level0_grids_match_godot_goldens() {
        let mut failures: Vec<String> = Vec::new();
        let mut report = String::new();
        for tag in ["parity", "k7qxm", "demo"] {
            let golden = load_grid_golden(tag);
            let world = golden["world_seed"].as_str().expect("world_seed").to_string();
            let (mut matched, mut mismatched, mut no_layout) = (0usize, 0usize, 0usize);
            report.push_str(&format!("seed {tag} (world {world}):\n"));
            for chunk in golden["chunks"].as_array().expect("chunks") {
                let a = chunk["coord"].as_array().expect("coord");
                let coord = Coord::new(
                    a[0].as_i64().unwrap() as i32,
                    a[1].as_i64().unwrap() as i32,
                    a[2].as_i64().unwrap() as i32,
                );
                let region = chunk["region"].as_str().unwrap_or("?");
                let want: Vec<String> = chunk["grid"]
                    .as_array()
                    .expect("grid")
                    .iter()
                    .map(|r| r.as_str().expect("grid row").to_string())
                    .collect();
                let Some(layout) = chunk_layout(&world, coord) else {
                    no_layout += 1;
                    failures.push(format!("{tag} {:?} {region}: no layout", (coord.x, coord.z)));
                    continue;
                };
                let got = rows_of(&layout.grid);
                match first_mismatch(&got, &want) {
                    None => {
                        matched += 1;
                        report.push_str(&format!("  ({:>3},{:>3}) {region:<9} match\n", coord.x, coord.z));
                    }
                    Some((x, z, ours, theirs)) => {
                        mismatched += 1;
                        report.push_str(&format!("  ({:>3},{:>3}) {region:<9} MISMATCH\n", coord.x, coord.z));
                        failures.push(format!(
                            "{tag} chunk ({},{},{}) {region}: first mismatch at x={x} z={z}: ours {ours} golden {theirs}",
                            coord.x, coord.y, coord.z
                        ));
                    }
                }
            }
            report.push_str(&format!(
                "  => {matched} matched, {mismatched} mismatched, {no_layout} without a layout\n"
            ));
        }
        eprintln!("{report}");
        for line in failures.iter().take(20) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} level-0 grids differ from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }
}
