//! Stage 3b-ii of the Infinite route: the region props of a level-0 chunk, a port of
//! `StreamedRouteGenerator._region_props` (streamed_route_generator.gd:2824-2980) and of the helpers
//! that the driver reaches outside a furnished room.
//!
//! Ported, with the Godot names and line numbers in each doc comment:
//! - the driver `region_props` (2824): the door entries first (`_room_door_props`, 5047, ported in
//!   grid.rs), the reserved cells and the solid exclusion (circulation and one-cell shoulders), the
//!   DRESSING stream (2832): a Fisher-Yates shuffle of the rooms, then up to 6 (safe) or 10 props, one
//!   per room, each choice drawn from the region's set. Wall-mounted props take `wall_mount_for_room`
//!   (5028), rugs and frames take the room centre, solid props take `solid_prop_placement` (4969).
//! - `_wall_count` (5607), `_backing_wall_direction` (5618), `_exclude_neighbourhood` (5600),
//!   `_prop_occupied_cells` (5011), `_collider_size` (5007), `_room_facing_yaw` (5290).
//!
//! The furnished rooms (`_furnished_room_props`, 3058) are in furnished.rs (stage 3b-iii part A): a room that
//! gets a template is skipped by the dressing draw, and its props are added before it (2884-2900).
//!
//! Stage 3b-iv: `post_passes` runs the post passes of `_build_chunk` after the hump pass, in the Godot order
//! (streamed_route_generator.gd:1024-1028): `GeometryResolve.push_out`, `scale_surface_mounts` (furnished.rs), then
//! `PlacementClearance.clear` (clearance.rs). `layout::chunk_layout` calls it on the grid after
//! `grid::level0_after_props`.
//! A region with a catalogue row (src/bin/last_route/data/regions/<id>.json) takes its prop set from the row (`prop_set`); the code
//! table in prop_tables.rs is the fallback for a region with no row.
//! Not ported yet: `_stair_door_exclusion` (5177, empty on level 0: stair_links returns []), the dual-level activity
//! entries (2841, behind `_dual_level_pair_active`, false at streamed_route_generator.gd:821), and
//! `_prop_hiding_fixtures` (5307, a consumer after the post passes).
//!
//! RNG: every draw is made in the Godot order from `SeededRandom("<world>:DRESSING:<chunk>")`, through
//! `Rng` (bit-exact with SeededRandom). The float arithmetic follows GDScript: a `Vector2` is `f32`, a
//! plain float expression is `f64`.

use std::collections::{HashMap, HashSet};

use super::clearance;
use super::furnished;
use super::grid::DoorProp;
use super::layout::{Circ, Coord, Grid, Room, Rect, CHUNK, V2};
use super::prop_tables::{lookup, PropSetEntry, PROP_COLLIDER_SIZES, REGION_PROP_SETS, WALL_MOUNTED_TYPES};
use super::regions::region_def;
use crate::content::regions::RegionDef;
use crate::world::rng::Rng;

/// The reserved cells of a chunk (`reserved`, a Dictionary in Godot): `Plain` for doors, thresholds and beds
/// (`true`), `SeatShareable` for the cells of a placed recipe prop (`SEAT_SHAREABLE`, streamed_route_generator.gd:3501).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Plain,
    SeatShareable,
}

pub type Reserved = HashMap<V2, Mark>;

/// One entry of a chunk's `region_props` (the keys this stage makes; `room_template` is stage 3b-iii).
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    /// `<chunk>:PROP:<index>` for a dressing prop, `<chunk>:DOOR:<room index>` for a room door.
    pub id: String,
    pub prop_type: &'static str,
    pub cell: V2,
    /// Radians. `_room_facing_yaw` returns a GDScript float (f64).
    pub yaw: f64,
    pub facing_target: V2,
    pub blocker_size: [f32; 3],
    pub solid: bool,
    /// `occupied_cells`. Wall-mounted props have no such key in Godot, which reads as empty here.
    pub occupied_cells: Vec<V2>,
    /// The door's room (`room_id`); empty for a dressing prop.
    pub room_id: String,
    /// `room_door`: the entry is a room door from `_room_door_props`.
    pub room_door: bool,
    /// Doors only: the cells the door opens, and the wall shoulders set to 1 in the grid.
    pub doorway_cells: Vec<V2>,
    pub threshold_wall_cells: Vec<V2>,
    /// Solid props only: `placement_offset`, `backing_wall`, `open_direction` and `collider_size`.
    pub placement_offset: Option<[f32; 2]>,
    pub backing_wall: Option<V2>,
    pub open_direction: Option<V2>,
    pub collider_size: Option<[f32; 3]>,
    /// Wall-mounted props only: `wall_mounted` and `mount_offset`.
    pub wall_mounted: bool,
    pub mount_offset: Option<[f32; 2]>,
    /// Solid dressing props: `corridor_clear`.
    pub corridor_clear: bool,
    /// Furnished-room props (`_template_prop_entry`): the recipe's `room_template`, the authored `mount_y`
    /// (a surface prop's top), `mount_scaled` and `oriented`, and the recipe index `spec_index` (erased in Godot
    /// before the output; 0 for a dressing prop).
    pub room_template: Option<String>,
    pub mount_y: f64,
    pub mount_scaled: bool,
    pub oriented: bool,
    pub spec_index: usize,
}

/// `_region_props` (streamed_route_generator.gd:2824) for a level-0 chunk. `grid` is the grid after the room
/// door planner (`level0_before_props`, grid.rs), which the furniture reads; `doors` are its planned doors;
/// `circulation` is the layout's circulation mask. `stairs` is always empty on level 0.
///
/// The region's prop set comes from the code table, or from src/bin/last_route/data/regions for the mall and hospital (`prop_set`).
/// Returns `Some(vec![])` for a chunk with no rooms, as Godot does (2826-2827). These are the props before the
/// post passes (`post_passes`).
pub fn region_props(
    world: &str,
    c: Coord,
    region: &str,
    rooms: &[Room],
    grid: &Grid,
    doors: &[DoorProp],
    safe: bool,
    circulation: &Circ,
) -> Option<Vec<Prop>> {
    if rooms.is_empty() {
        return Some(Vec::new());
    }
    let choices = prop_set(region);
    if choices.is_empty() {
        return Some(Vec::new());
    }
    let tag = chunk_tag(c);
    // 2832: SeededRandom("<world_seed>:DRESSING:<chunk_id>")
    let mut rng = Rng::new(&format!("{world}:DRESSING:{tag}"));
    // 2837: _room_door_props (the door entries come first)
    let mut result = door_props(&tag, rooms, doors);
    // 2841-2866: the dual-level activity entries need `_dual_level_pair_active`, which is false (821).
    // 2867-2872: reserved is the door cells and their threshold shoulders.
    let mut reserved: Reserved = HashMap::new();
    for door in &result {
        reserved.insert(door.cell, Mark::Plain);
        for shoulder in &door.threshold_wall_cells {
            reserved.insert(*shoulder, Mark::Plain);
        }
    }
    // _stair_door_exclusion(stairs) is empty on level 0, so the wall mounts have no stair exclusion.
    let wall_mount_exclusion: HashSet<V2> = HashSet::new();
    // 2873-2882: the solid exclusion is the circulation, then one-cell shoulders of the traffic and reserved cells.
    let mut solid_exclusion: HashSet<V2> = circulation.clone();
    for traffic in circulation.iter() {
        exclude_neighbourhood(&mut solid_exclusion, *traffic, 1);
    }
    for reserved_cell in reserved.keys() {
        exclude_neighbourhood(&mut solid_exclusion, *reserved_cell, 1);
    }
    // 2884-2900: the furnished rooms. Stairwells, suite parents and two-floor rooms are skipped (no
    // two-floor rooms exist on level 0). A room with a template is skipped by the dressing below.
    let mut templated: HashSet<String> = HashSet::new();
    for room in rooms {
        if room.room_type == "stairwell" || room.suite_parent {
            continue;
        }
        let furnishing = furnished::furnished_room_props(world, c, region, room, grid, &mut reserved);
        if furnishing.is_empty() {
            continue;
        }
        templated.insert(room.id.clone());
        result.extend(furnishing);
    }
    // 2902-2906: the shuffle of the rooms, Fisher-Yates from the end (n - 1 draws)
    let mut order: Vec<&Room> = rooms.iter().collect();
    rng.shuffle(&mut order);
    // 2908: at most 6 props in a safe chunk, 10 otherwise
    let prop_count = (if safe { 6 } else { 10 }).min(order.len());
    for index in 0..prop_count {
        let room = order[index];
        // prison cells are owned by the cell-block dresser; stairwells, suite parents and templated rooms
        // are not dressed here
        if room.room_type == "stairwell" || room.room_type == "prison_cell" || room.suite_parent || templated.contains(&room.id) {
            continue;
        }
        let rect = room.rect;
        if rect.w < 3 || rect.h < 3 {
            continue;
        }
        // 2927: one draw for the choice
        let choice = choices[rng.next_index(choices.len())];
        let prop_type = choice.prop_type;
        let id = format!("{tag}:PROP:{index}");
        if WALL_MOUNTED_TYPES.contains(&prop_type) {
            // 2929: _wall_mount_for_room (one draw if any candidate)
            let Some((mount_cell, backing_wall)) = wall_mount_for_room(rect, grid, &reserved, &wall_mount_exclusion, &mut rng) else {
                continue;
            };
            let open_direction = backing_wall.neg();
            result.push(Prop {
                id,
                prop_type,
                cell: mount_cell,
                yaw: room_facing_yaw(mount_cell, mount_cell.add(open_direction)),
                facing_target: mount_cell.add(open_direction),
                blocker_size: choice.size,
                solid: false,
                occupied_cells: Vec::new(),
                room_id: String::new(),
                room_door: false,
                doorway_cells: Vec::new(),
                threshold_wall_cells: Vec::new(),
                placement_offset: None,
                backing_wall: Some(backing_wall),
                open_direction: Some(open_direction),
                collider_size: None,
                wall_mounted: true,
                // Vector2(backing_wall) * .44: the thin model moves just in front of the wall face
                mount_offset: Some([backing_wall.x as f32 * 0.44, backing_wall.y as f32 * 0.44]),
                corridor_clear: false, room_template: None, mount_y: 0.0, mount_scaled: false, oriented: false, spec_index: 0,
            });
            reserved.insert(mount_cell, Mark::Plain);
            continue;
        }
        let blocker_size = choice.size;
        // 2953: the room centre, rect.position + rect.size / 2 (`_room_entry`, 2744)
        let room_center = V2::new(rect.x + rect.w / 2, rect.y + rect.h / 2);
        if !choice.solid {
            // rugs and frames sit at the room centre and take no room
            result.push(Prop {
                id,
                prop_type,
                cell: room_center,
                yaw: 0.0,
                facing_target: room_center,
                blocker_size,
                solid: false,
                occupied_cells: Vec::new(),
                room_id: String::new(),
                room_door: false,
                doorway_cells: Vec::new(),
                threshold_wall_cells: Vec::new(),
                placement_offset: None,
                backing_wall: None,
                open_direction: None,
                collider_size: None,
                wall_mounted: false,
                mount_offset: None,
                corridor_clear: false, room_template: None, mount_y: 0.0, mount_scaled: false, oriented: false, spec_index: 0,
            });
            reserved.insert(room_center, Mark::Plain);
            continue;
        }
        // 2965: _solid_prop_placement (one draw if any candidate)
        let Some(placement) = solid_prop_placement(rect, grid, &solid_exclusion, &reserved, blocker_size, &mut rng) else {
            continue;
        };
        let cell = placement.cell;
        let open_direction = placement.open_direction;
        result.push(Prop {
            id,
            prop_type,
            cell,
            yaw: room_facing_yaw(cell, cell.add(open_direction)),
            facing_target: cell.add(open_direction),
            blocker_size,
            solid: true,
            occupied_cells: placement.occupied.clone(),
            room_id: String::new(),
            room_door: false,
            doorway_cells: Vec::new(),
            threshold_wall_cells: Vec::new(),
            placement_offset: Some(placement.placement_offset),
            backing_wall: Some(open_direction.neg()),
            open_direction: Some(open_direction),
            collider_size: Some(collider_size(prop_type, blocker_size)),
            wall_mounted: false,
            mount_offset: None,
            corridor_clear: true, room_template: None, mount_y: 0.0, mount_scaled: false, oriented: false, spec_index: 0,
        });
        for occupied in &placement.occupied {
            reserved.insert(*occupied, Mark::Plain);
            exclude_neighbourhood(&mut solid_exclusion, *occupied, 1);
        }
    }
    Some(result)
}

/// `REGION_PROP_SETS.get(region, REGION_PROP_SETS["maze"])` (streamed_route_generator.gd:2828). A region with a
/// catalogue row (src/bin/last_route/data/regions/<id>.json, read through ContentRegistry) takes its set from the row; a region with
/// no row keeps the code table.
fn prop_set(region: &str) -> Vec<PropSetEntry> {
    if let Some(def) = region_def(region) {
        return prop_set_of(def);
    }
    lookup(REGION_PROP_SETS, region).unwrap_or_else(|| lookup(REGION_PROP_SETS, "maze").unwrap_or(&[])).to_vec()
}

/// The prop set of a catalogue row: one `PropSetEntry` per `prop_set` item, in file order (the draw indexes this
/// list, so the order is part of the seed's output). Every entry is solid unless its `solid` is false.
pub(super) fn prop_set_of(def: &'static RegionDef) -> Vec<PropSetEntry> {
    def.prop_set
        .iter()
        .map(|p| PropSetEntry { prop_type: p.prop_type.as_str(), size: p.size, solid: p.solid })
        .collect()
}

/// The post passes of `_build_chunk` after the hump pass (streamed_route_generator.gd:1024-1028), in the Godot
/// order: `GeometryResolve.push_out` (1024), `scale_surface_mounts` (1025), then `PlacementClearance.clear` when
/// `clear_walkways` (1027-1028). `grid` is the grid after `_remove_wall_humps`. Neither pass draws from the RNG.
pub fn post_passes(grid: &Grid, rooms: &[Room], props: Vec<Prop>) -> Vec<Prop> {
    let mut props = clearance::push_out(grid, props);
    furnished::scale_surface_mounts(&mut props);
    if CLEAR_WALKWAYS {
        clearance::clear(grid, rooms, props)
    } else {
        props
    }
}

/// `StreamedRouteGenerator.clear_walkways` (streamed_route_generator.gd:123): `static var ... := true`, never
/// reassigned.
const CLEAR_WALKWAYS: bool = true;

/// `_room_door_props` entries (streamed_route_generator.gd:5047-5139) from the planned doors of grid.rs.
/// The id uses the room's index in `rooms`, as Godot's `room_index` does.
fn door_props(tag: &str, rooms: &[Room], doors: &[DoorProp]) -> Vec<Prop> {
    doors
        .iter()
        .map(|door| {
            let room_index = rooms.iter().position(|room| room.id == door.room_id).unwrap_or(0);
            let inward = door.inward;
            Prop {
                id: format!("{tag}:DOOR:{room_index}"),
                prop_type: door.door_type,
                cell: door.cell,
                // face perpendicular to the wall: the yaw from the threshold cell to the inward cell (5127)
                yaw: room_facing_yaw(door.cell, door.cell.add(inward)),
                facing_target: door.cell.add(inward),
                // Vector3(door_span + .15, 2.6, .2)
                blocker_size: [door.door_span as f32 + 0.15, 2.6, 0.2],
                solid: false,
                occupied_cells: door.doorway_cells.clone(),
                room_id: door.room_id.clone(),
                room_door: true,
                doorway_cells: door.doorway_cells.clone(),
                threshold_wall_cells: door.threshold_wall_cells.clone(),
                placement_offset: None,
                backing_wall: None,
                open_direction: None,
                collider_size: None,
                wall_mounted: false,
                mount_offset: None,
                corridor_clear: false, room_template: None, mount_y: 0.0, mount_scaled: false, oriented: false, spec_index: 0,
            }
        })
        .collect()
}

/// `_wall_mount_for_room` (streamed_route_generator.gd:5028): a random open cell of the room that has one
/// wall, with an open cell on the other side. One draw when there is any candidate.
fn wall_mount_for_room(
    rect: Rect,
    grid: &Grid,
    reserved: &Reserved,
    excluded: &HashSet<V2>,
    rng: &mut Rng,
) -> Option<(V2, V2)> {
    let mut candidates: Vec<(V2, V2)> = Vec::new();
    for z in rect.y..rect.y + rect.h {
        for x in rect.x..rect.x + rect.w {
            let cell = V2::new(x, z);
            if reserved.contains_key(&cell) || excluded.contains(&cell) || !grid_cell_open(grid, cell) || wall_count(grid, cell) != 1 {
                continue;
            }
            let backing_wall = backing_wall_direction(grid, cell);
            if !grid_cell_open(grid, cell.add(backing_wall.neg())) {
                continue;
            }
            candidates.push((cell, backing_wall));
        }
    }
    if candidates.is_empty() {
        return None;
    }
    let index = rng.next_index(candidates.len());
    Some(candidates[index])
}

/// The result of `_solid_prop_placement`.
struct Placement {
    cell: V2,
    open_direction: V2,
    placement_offset: [f32; 2],
    occupied: Vec<V2>,
}

/// `_solid_prop_placement` (streamed_route_generator.gd:4969): the cells of the room with exactly one wall
/// behind them, whose footprint (from `_prop_occupied_cells`) is open, unexcluded and unreserved. One draw
/// when there is any candidate.
fn solid_prop_placement(
    rect: Rect,
    grid: &Grid,
    excluded: &HashSet<V2>,
    reserved: &Reserved,
    blocker_size: [f32; 3],
    rng: &mut Rng,
) -> Option<Placement> {
    let mut candidates: Vec<Placement> = Vec::new();
    for z in rect.y..rect.y + rect.h {
        for x in rect.x..rect.x + rect.w {
            let cell = V2::new(x, z);
            if excluded.contains(&cell) || reserved.contains_key(&cell) || !grid_cell_open(grid, cell) || wall_count(grid, cell) != 1 {
                continue;
            }
            let backing_wall = backing_wall_direction(grid, cell);
            let open_direction = backing_wall.neg();
            if !grid_cell_open(grid, cell.add(open_direction)) {
                continue;
            }
            // the rear face sits 5 cm off the wall; any excess depth moves into the room. maxf(0.0, z * .5 - .45)
            // is a GDScript float, then Vector2(open) * depth is a Vector2 (f32)
            let depth_shift = (blocker_size[2] as f64 * 0.5 - 0.45).max(0.0);
            let placement_offset = [
                open_direction.x as f32 * depth_shift as f32,
                open_direction.y as f32 * depth_shift as f32,
            ];
            let occupied = prop_occupied_cells(cell, open_direction, blocker_size, placement_offset);
            let mut valid = !occupied.is_empty();
            for at in &occupied {
                if !has_point(rect, *at) || excluded.contains(at) || reserved.contains_key(at) || !grid_cell_open(grid, *at) {
                    valid = false;
                    break;
                }
            }
            if valid {
                candidates.push(Placement { cell, open_direction, placement_offset, occupied });
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    let index = rng.next_index(candidates.len());
    Some(candidates.swap_remove(index))
}

/// `_collider_size` (streamed_route_generator.gd:5007): the model's collider box, else the authored size.
pub(super) fn collider_size(prop_type: &str, authored: [f32; 3]) -> [f32; 3] {
    lookup(PROP_COLLIDER_SIZES, prop_type).unwrap_or(authored)
}

/// `_prop_occupied_cells` (streamed_route_generator.gd:5011): the cells under a footprint of `blocker_size`
/// centred on the cell, shifted by `placement_offset`. Order: z outer, x inner.
pub(super) fn prop_occupied_cells(cell: V2, open_direction: V2, blocker_size: [f32; 3], placement_offset: [f32; 2]) -> Vec<V2> {
    // center := Vector2(cell) + Vector2(.5, .5) + placement_offset: Vector2 arithmetic, so f32
    let center = [
        cell.x as f32 + 0.5 + placement_offset[0],
        cell.y as f32 + 0.5 + placement_offset[1],
    ];
    // extent := Vector2(size.x, size.z) * .5, with x and z swapped when the open direction is along x
    let extent: [f32; 2] = if open_direction.x != 0 {
        [blocker_size[2] * 0.5, blocker_size[0] * 0.5]
    } else {
        [blocker_size[0] * 0.5, blocker_size[2] * 0.5]
    };
    // floori(center - extent +- .001): GDScript float arithmetic on the f32 components
    let min_x = (center[0] as f64 - extent[0] as f64 + 0.001).floor() as i32;
    let min_z = (center[1] as f64 - extent[1] as f64 + 0.001).floor() as i32;
    let max_x = (center[0] as f64 + extent[0] as f64 - 0.001).floor() as i32;
    let max_z = (center[1] as f64 + extent[1] as f64 - 0.001).floor() as i32;
    let mut result = Vec::new();
    for z in min_z..=max_z {
        for x in min_x..=max_x {
            result.push(V2::new(x, z));
        }
    }
    result
}

/// `_room_facing_yaw` (streamed_route_generator.gd:5290): the yaw that turns the prop's -Z forward toward
/// `target`, snapped to PI/4. Zero when the two cells coincide.
pub(super) fn room_facing_yaw(cell: V2, target: V2) -> f64 {
    let dx = (target.x - cell.x) as f64;
    let dz = (target.y - cell.y) as f64;
    if dx * dx + dz * dz < 0.01 {
        return 0.0;
    }
    // snappedf(atan2(-x, -z), PI * .25) = floor(x / step + .5) * step
    let step = std::f64::consts::PI * 0.25;
    let raw = (-dx).atan2(-dz);
    (raw / step + 0.5).floor() * step
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600): the square of `radius` round `cell`.
fn exclude_neighbourhood(excluded: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            excluded.insert(cell.add(V2::new(dx, dz)));
        }
    }
}

/// `_wall_count` (streamed_route_generator.gd:5607): the sides of `cell` that are wall or outside the chunk.
fn wall_count(grid: &Grid, cell: V2) -> i32 {
    const SIDES: [V2; 4] = [V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)];
    SIDES.iter().filter(|side| is_wall_or_outside(grid, cell.add(**side))).count() as i32
}

/// `_backing_wall_direction` (streamed_route_generator.gd:5618): the first side (up, right, down, else left)
/// that is a wall. The candidate cells are interior, so the reads stay in the chunk.
fn backing_wall_direction(grid: &Grid, cell: V2) -> V2 {
    if cell_value(grid, cell.x, cell.y - 1) != 0 {
        return V2::new(0, -1);
    }
    if cell_value(grid, cell.x + 1, cell.y) != 0 {
        return V2::new(1, 0);
    }
    if cell_value(grid, cell.x, cell.y + 1) != 0 {
        return V2::new(0, 1);
    }
    V2::new(-1, 0)
}

/// `_grid_cell_open` (streamed_route_generator.gd:5285): inside the chunk and floor.
fn grid_cell_open(grid: &Grid, cell: V2) -> bool {
    cell.x >= 0 && cell.y >= 0 && cell.x < CHUNK && cell.y < CHUNK && grid[cell.y as usize][cell.x as usize] == 0
}

/// A grid value, with the outside of the chunk read as wall (the `_wall_count` convention).
fn cell_value(grid: &Grid, x: i32, z: i32) -> u8 {
    if x < 0 || z < 0 || x >= CHUNK || z >= CHUNK {
        1
    } else {
        grid[z as usize][x as usize]
    }
}

fn is_wall_or_outside(grid: &Grid, cell: V2) -> bool {
    cell_value(grid, cell.x, cell.y) != 0
}

/// `Rect2i.has_point`: end-exclusive.
fn has_point(rect: Rect, p: V2) -> bool {
    p.x >= rect.x && p.y >= rect.y && p.x < rect.x + rect.w && p.y < rect.y + rect.h
}

/// The chunk id of `_build_chunk` (`chunk_id`): "x:y:z".
pub(super) fn chunk_tag(c: Coord) -> String {
    format!("{}:{}:{}", c.x, c.y, c.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The draw reads the row it is given: a changed size and solid flag in the data reach the entries, in file order.
    #[test]
    fn prop_set_of_reads_the_row_it_is_given() {
        let text = r#"{"schema":"lr.region/1","id":"prison","hall_floor":["concrete0"],"hall_runner":false,"room_default_floor":["concrete0"],"room_roles":["prison_cell"],"prop_set":[{"type":"prisonBunk","size":[0.6,1.0,1.8]},{"type":"ceilingLight","size":[0.75,0.3,0.75],"solid":false}]}"#;
        let def: &'static RegionDef = Box::leak(Box::new(crate::content::regions::parse_file("prison.json", text).unwrap()));
        let set = prop_set_of(def);
        assert_eq!(set.len(), 2);
        assert_eq!(set[0].prop_type, "prisonBunk");
        assert_eq!(set[0].size, [0.6, 1.0, 1.8]);
        assert!(set[0].solid);
        assert_eq!(set[1].prop_type, "ceilingLight");
        assert!(!set[1].solid);
    }

    /// A region with no catalogue row (office) keeps the code table's fallback, the maze set.
    #[test]
    fn prop_set_falls_back_to_the_maze_table_without_a_row() {
        let maze = lookup(REGION_PROP_SETS, "maze").expect("maze row");
        let office = prop_set("office");
        assert_eq!(office.len(), maze.len());
        assert_eq!(office[0].prop_type, maze[0].prop_type);
    }

    #[test]
    fn facing_yaw_snaps_to_eighths_of_a_turn() {
        let o = V2::new(5, 5);
        assert_eq!(room_facing_yaw(o, o), 0.0);
        // a target one cell to the north (-Z): atan2(-0, 1) = 0
        assert_eq!(room_facing_yaw(o, V2::new(5, 4)), 0.0);
        // one cell to the south (+Z): atan2(-0, -1) = -PI
        assert!((room_facing_yaw(o, V2::new(5, 6)) + std::f64::consts::PI).abs() < 1e-12);
        // one cell to the east: atan2(-1, -0) = -PI/2
        assert!((room_facing_yaw(o, V2::new(6, 5)) + std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        // a diagonal snaps to a quarter turn
        assert!((room_facing_yaw(o, V2::new(6, 4)) + std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    }

    #[test]
    fn occupied_cells_follow_the_footprint_and_run_z_outer() {
        // a 1.4 x 0.8 crate centred on cell (10, 10) and open toward +Z: x extent 0.7, z extent 0.4
        // centre (10.5, 10.5): x from floor(9.801) to floor(11.199), z from floor(10.101) to floor(10.899)
        let cells = prop_occupied_cells(V2::new(10, 10), V2::new(0, 1), [1.4, 1.0, 0.8], [0.0, 0.0]);
        assert_eq!(cells, vec![V2::new(9, 10), V2::new(10, 10), V2::new(11, 10)]);
        // open toward +X swaps the extents: a box 2.0 deep (z) and 1.0 wide (x) runs along x, extent 1.0 and 0.5
        // centre (10.5, 10.5): x from floor(9.501) to floor(11.499), z from floor(10.001) to floor(10.999)
        let across = prop_occupied_cells(V2::new(10, 10), V2::new(1, 0), [1.0, 1.0, 2.0], [0.0, 0.0]);
        assert_eq!(across, vec![V2::new(9, 10), V2::new(10, 10), V2::new(11, 10)]);
    }

    #[test]
    fn wall_count_treats_the_outside_as_wall() {
        let mut grid: Grid = [[1u8; 45]; 45];
        grid[10][10] = 0;
        grid[10][11] = 0;
        assert_eq!(wall_count(&grid, V2::new(10, 10)), 3);
        assert_eq!(wall_count(&grid, V2::new(0, 0)), 4);
        assert_eq!(backing_wall_direction(&grid, V2::new(10, 10)), V2::new(0, -1));
    }

    /// The compact golden `tests/parity/props-<tag>.json`: the level-0 chunks of a Godot golden, with the props
    /// that have no `room_template` (the fields `id`, `type`, `cell`, `yaw`, `occupied_cells`, `doorway_cells`),
    /// and `template_rooms`, the ids of the rooms whose furnishing (a room_template prop) Godot made.
    fn load_golden(tag: &str) -> serde_json::Value {
        // PARITY_DIR overrides the directory of the compact goldens (tests/parity by default)
        let dir = std::env::var("PARITY_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("parity"));
        let path = dir.join(format!("props-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("props golden json")
    }

    fn cell_of(v: &serde_json::Value) -> V2 {
        let a = v.as_array().expect("cell");
        V2::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32)
    }

    fn cells_of(v: &serde_json::Value) -> Vec<V2> {
        v.as_array().map_or(Vec::new(), |list| list.iter().map(cell_of).collect())
    }

    /// The first field that differs between our prop and the Godot entry, or `None` when they match.
    /// Compared: type, cell, yaw (f64, within 1e-9: the golden prints 15 digits), occupied_cells and
    /// doorway_cells, both in order. An absent `occupied_cells` or `doorway_cells` is the empty list.
    fn prop_difference(ours: &Prop, want: &serde_json::Value) -> Option<String> {
        if want["type"].as_str() != Some(ours.prop_type) {
            return Some(format!("type {} vs Godot {}", ours.prop_type, want["type"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs Godot {}", (ours.cell.x, ours.cell.y), want["cell"]));
        }
        let want_yaw = want["yaw"].as_f64().unwrap_or(0.0);
        if (ours.yaw - want_yaw).abs() > 1e-9 {
            return Some(format!("yaw {} vs Godot {}", ours.yaw, want_yaw));
        }
        let want_occupied = cells_of(&want["occupied_cells"]);
        if ours.occupied_cells != want_occupied {
            return Some(format!("occupied_cells {} vs Godot {} cells", ours.occupied_cells.len(), want_occupied.len()));
        }
        let want_doorway = cells_of(&want["doorway_cells"]);
        if ours.doorway_cells != want_doorway {
            return Some(format!("doorway_cells {} vs Godot {} cells", ours.doorway_cells.len(), want_doorway.len()));
        }
        None
    }

    /// Every level-0 chunk of the seven goldens, from `chunk_layout`, against the Godot props after the post passes
    /// (push_out, scale_surface_mounts, PlacementClearance.clear). Each golden prop is matched by id on type, cell,
    /// yaw, occupied cells and doorway cells, and every prop made here must be in the golden. A chunk where Godot
    /// furnished a room is counted apart (furnished_chunks). Prints the counts per seed and the first mismatches.
    /// PARITY_TEMPLATES=1 also compares the furnished props (room_template); PARITY_TAGS narrows the seeds and
    /// PARITY_DIR points at compact goldens elsewhere than tests/parity.
    #[test]
    fn region_props_match_godot_goldens() {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        const ALL_TAGS: [&str; 7] = ["parity", "k7qxm", "demo", "mansion1", "apt1", "beach1", "gothic1"];
        // PARITY_TAGS=apt1,demo narrows the run to some seeds (the goldens of the other seeds may be missing)
        let only: Vec<String> = std::env::var("PARITY_TAGS").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default();
        let tags: Vec<&str> = ALL_TAGS.iter().copied().filter(|t| only.is_empty() || only.iter().any(|o| o == t)).collect();
        // PARITY_TEMPLATES=1: the compact goldens list the furnished props too, and they are compared as well
        let with_templates = std::env::var("PARITY_TEMPLATES").is_ok();
        let mut report = String::new();
        let mut failures: Vec<String> = Vec::new();
        for tag in tags {
            let golden = load_golden(tag);
            let world = golden["world_seed"].as_str().expect("world_seed").to_string();
            let (mut chunks, mut matched_chunks, mut no_layout, mut data_backed) = (0usize, 0usize, 0usize, 0usize);
            let (mut furnished_chunks, mut furnished_matched) = (0usize, 0usize);
            let (mut props_godot, mut props_matched, mut props_ours) = (0usize, 0usize, 0usize);
            let (mut not_made, mut field_diffs, mut extra_furnished, mut extra_plain) = (0usize, 0usize, 0usize, 0usize);
            let mut by_region: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
            for chunk in golden["chunks"].as_array().expect("chunks") {
                chunks += 1;
                let at = chunk["coord"].as_array().expect("coord");
                let c = Coord::new(
                    at[0].as_i64().unwrap() as i32,
                    at[1].as_i64().unwrap() as i32,
                    at[2].as_i64().unwrap() as i32,
                );
                let region = chunk["region"].as_str().unwrap_or("?").to_string();
                let furnished = chunk["template_rooms"].as_array().is_some_and(|r| !r.is_empty());
                if furnished {
                    furnished_chunks += 1;
                }
                let entry = by_region.entry(region.clone()).or_default();
                entry.0 += 1;
                let Some(layout) = crate::world::route::layout::chunk_layout_godot(&world, c) else {
                    no_layout += 1;
                    continue;
                };
                let Some(ours) = layout.region_props else {
                    data_backed += 1;
                    continue;
                };
                let by_id: std::collections::HashMap<&str, &Prop> = ours.iter().map(|p| (p.id.as_str(), p)).collect();
                let wanted: Vec<&str> = chunk["props"].as_array().expect("props").iter().map(|p| p["id"].as_str().unwrap_or("")).collect();
                let mut chunk_ok = true;
                for want in chunk["props"].as_array().expect("props") {
                    props_godot += 1;
                    let id = want["id"].as_str().unwrap_or("");
                    match by_id.get(id) {
                        Some(prop) => match prop_difference(prop, want) {
                            None => props_matched += 1,
                            Some(why) => {
                                chunk_ok = false;
                                field_diffs += 1;
                                failures.push(format!("{tag} {region} {id}: {why}"));
                            }
                        },
                        None => {
                            chunk_ok = false;
                            not_made += 1;
                            failures.push(format!("{tag} {region} {id}: not made (Godot {} at {})", want["type"], want["cell"]));
                        }
                    }
                }
                // the golden lists the dressing and door props; a furnished prop (room_template) is stage 3b-iii's
                for prop in ours.iter().filter(|p| with_templates || p.room_template.is_none()) {
                    props_ours += 1;
                    if !wanted.contains(&prop.id.as_str()) {
                        chunk_ok = false;
                        if furnished {
                            extra_furnished += 1;
                            failures.push(format!("{tag} {region} {}: made here, not in Godot (its chunk has furnished rooms)", prop.id));
                        } else {
                            extra_plain += 1;
                            failures.push(format!("{tag} {region} {}: made here, not in Godot (no furnished room)", prop.id));
                        }
                    }
                }
                if chunk_ok {
                    matched_chunks += 1;
                    entry.1 += 1;
                    if furnished {
                        furnished_matched += 1;
                    }
                }
            }
            report.push_str(&format!(
                "seed {tag} (world {world}): level-0 chunks {chunks}, matched {matched_chunks}, not laid out {no_layout}, \
                 data-backed (mall/hospital) {data_backed}; Godot furnished rooms in {furnished_chunks} chunks \
                 ({furnished_matched} of them matched)\n  props without template: Godot {props_godot}, matched {props_matched}, \
                 made here {props_ours}; not made {not_made}, field differences {field_diffs}, extra in furnished chunks \
                 {extra_furnished}, extra in plain chunks {extra_plain}\n  matched chunks by region (chunks, matched): {by_region:?}\n"
            ));
        }
        eprintln!("{report}");
        for line in failures.iter().take(25) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} prop differences from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }
}
