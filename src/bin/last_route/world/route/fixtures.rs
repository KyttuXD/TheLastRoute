//! Stage 3c part 1 of the Infinite route: the fixtures of a level-0 chunk, a port of
//! `StreamedRouteGenerator._fixtures` (streamed_route_generator.gd:5338-5390) and
//! `_prop_hiding_fixtures` (5307-5335), with the helpers they reach.
//!
//! `_build_chunk` calls them in this order (streamed_route_generator.gd:1029-1031):
//! `fixtures = _fixtures(...)`, then `fixtures += _prop_hiding_fixtures(...)` on the region props after the
//! post passes (`props::post_passes`). `chunk_fixtures` makes both in that order.
//!
//! Helpers, with the Godot names and line numbers in each doc comment:
//! - `_stair_door_exclusion` (5177) and `stair_path_cells` (1655), `STAIR_RUN_CELLS` (62). Stairs are empty on
//!   level 0 (`stair_links`, 1399), so the caller passes no anchors; the port keeps the full-flight rule.
//! - `_exclude_neighbourhood` (5600), `_wall_count` (5607), `_backing_wall_direction` (5618): local copies, since
//!   the `props` versions are private to that module.
//! - `_room_facing_yaw` (5290), reused from `props::room_facing_yaw`.
//! - `HIDING_PROP_STYLES` (5301) and the capacity from `PropModelCatalog.hide_capacity` (`prop_model_catalog.gd`, 504).
//!
//! RNG: `_fixtures` draws from `SeededRandom("<world>:FIXTURES:<chunk>")` (5361). Every candidate popped draws one
//! `next_index` over the pool, including the candidates rejected by the 5-cell spacing (5368-5373), until 6 are
//! accepted (3 on a safe chunk) or the pool is empty. `_prop_hiding_fixtures` draws nothing.
//!
//! The sealed wing (`SealPlanner`, level -3) is not part of this stage: level 0 has no sealed cells.
//! `fixtures.rs` at `src/world/fixtures.rs` is the finite generator's fixtures and is not used here.

use std::collections::HashSet;

use super::layout::{Coord, Grid, Room, CHUNK, V2};
use super::props::{self, Prop};
use crate::world::rng::Rng;

/// `STAIR_RUN_CELLS` (streamed_route_generator.gd:62): the flight length in cells, after the anchor.
const STAIR_RUN_CELLS: i32 = 8;
/// `CHUNK_SIZE / 2`, the chunk's centre row and column.
const CENTER: i32 = CHUNK / 2;

/// `HIDING_PROP_STYLES` (streamed_route_generator.gd:5301-5304): the furniture a runner can hide in, as (prop type, style).
/// The hospital bed (regions lane) has a low base and is not in the table.
pub const HIDING_PROP_STYLES: [(&str, &str); 5] = [
    ("singleBed", "bed"),
    ("doubleBed", "bed"),
    ("gothicDoubleBed", "bed"),
    ("bedroomWardrobe", "wardrobe"),
    ("gothicWardrobe", "wardrobe"),
];

/// The hiding data a prop-based fixture carries (`_prop_hiding_fixtures`, 5307-5335).
#[derive(Clone, Debug, PartialEq)]
pub struct HideSpot {
    pub style: &'static str,
    pub prop_type: &'static str,
    /// `PropModelCatalog.hide_capacity` (prop_model_catalog.gd:504): the prop's `hide_capacity`, clamped to 1..2
    /// (`HideOccupancy.MAX`, hide_occupancy.gd:14). The data sets 2 on `bedroomWardrobe` (data/props/p06.json:165),
    /// `gothicWardrobe` (data/props/p07.json:219) and `bedroomWardrobeMansion`; every other prop is 1.
    pub capacity: i32,
    pub prop_id: String,
    pub prop_cell: V2,
    /// `local_position`: the prop's centre in the chunk (f32, as Godot's Vector2).
    pub local_position: [f32; 2],
    pub blocker_size: [f32; 3],
}

/// One fixture of a chunk: the Godot dictionary's `id`, `kind`, `cell`, `yaw`, `backing_wall` and `open_direction`,
/// plus `hide` for the prop-based hiding spots.
#[derive(Clone, Debug, PartialEq)]
pub struct Fixture {
    pub id: String,
    /// "hiding_spot" or "vent".
    pub kind: &'static str,
    pub cell: V2,
    pub yaw: f64,
    pub backing_wall: V2,
    pub open_direction: V2,
    pub hide: Option<HideSpot>,
}

/// `chunk_fixtures`: the fixtures of a level-0 chunk in the Godot order (`_build_chunk` 1029-1031).
/// `grid` is the final grid, `rooms` the chunk's rooms (after the stairwell carve), `stairs` the stair anchor cells
/// (always empty on level 0), `region_props` the region props after the post passes, and `safe` the safe-room flag.
pub fn chunk_fixtures(
    world: &str,
    c: Coord,
    grid: &Grid,
    rooms: &[Room],
    stairs: &[V2],
    region_props: &[Prop],
    safe: bool,
) -> Vec<Fixture> {
    let mut fixtures = fixtures(world, c, grid, rooms, stairs, region_props, safe);
    fixtures.extend(prop_hiding_fixtures(c, region_props, safe));
    fixtures
}

/// `_fixtures` (streamed_route_generator.gd:5338-5390): the vents and the wall hiding spots drawn from the pool of
/// room-floor cells, with a `FIXTURES` draw per candidate popped.
pub fn fixtures(
    world: &str,
    c: Coord,
    grid: &Grid,
    rooms: &[Room],
    stairs: &[V2],
    region_props: &[Prop],
    safe: bool,
) -> Vec<Fixture> {
    // Exclude the complete flight (5343), then the prop neighbourhoods (5344-5345) and a 9x9 round the centre on
    // a safe chunk (5346-5347).
    let excluded = excluded_cells(stairs, region_props, safe);
    let mut candidates = pool(grid, rooms, &excluded);
    let tag = props::chunk_tag(c);
    let mut rng = Rng::new(&format!("{world}:FIXTURES:{tag}"));
    let target_count = if safe { 3 } else { 6 };
    let mut result: Vec<Fixture> = Vec::new();
    while result.len() < target_count && !candidates.is_empty() {
        // pop_at(rng.next_index(size)): one draw per candidate, even one the spacing test rejects (5367-5373).
        let index = rng.next_index(candidates.len());
        let cell = candidates.remove(index);
        let separated = result.iter().all(|existing| distance(cell, existing.cell) >= 5.0);
        if !separated {
            continue;
        }
        // `"vent" if result.size() % 3 == 2`: the third and sixth fixtures are vents (5375).
        let kind = if result.len() % 3 == 2 { "vent" } else { "hiding_spot" };
        let backing_wall = backing_wall_direction(grid, cell);
        let open_direction = backing_wall.neg();
        let number = result.len();
        result.push(Fixture {
            id: format!("{tag}:FIXTURE:{}:{number}", kind.to_uppercase()),
            kind,
            cell,
            // Authored interactables face local -Z: point that front at the open floor (5384-5386).
            yaw: props::room_facing_yaw(cell, cell.add(open_direction)),
            backing_wall,
            open_direction,
            hide: None,
        });
    }
    result
}

/// The cells `_fixtures` keeps its wall fixtures off (5343-5347): the stair flights, the prop neighbourhoods, and a 9x9
/// round the centre on a safe chunk.
fn excluded_cells(stairs: &[V2], region_props: &[Prop], safe: bool) -> HashSet<V2> {
    let mut excluded = stair_door_exclusion(stairs);
    for prop in region_props {
        exclude_neighbourhood(&mut excluded, prop.cell, 1);
    }
    if safe {
        exclude_neighbourhood(&mut excluded, V2::new(CENTER, CENTER), 4);
    }
    excluded
}

/// The pool of `_fixtures` (5349-5365): every room's rectangle in order, skipping suite parents, the excluded cells,
/// the non-floor cells and the cells that do not have exactly one wall side. A cell is listed once.
fn pool(grid: &Grid, rooms: &[Room], excluded: &HashSet<V2>) -> Vec<V2> {
    let mut candidates: Vec<V2> = Vec::new();
    let mut seen: HashSet<V2> = HashSet::new();
    for room in rooms {
        if room.suite_parent {
            continue;
        }
        let rect = room.rect;
        for z in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let cell = V2::new(x, z);
                if seen.contains(&cell) || excluded.contains(&cell) || !in_chunk(cell) {
                    continue;
                }
                if grid[z as usize][x as usize] != 0 || wall_count(grid, cell) != 1 {
                    continue;
                }
                seen.insert(cell);
                candidates.push(cell);
            }
        }
    }
    candidates
}

/// Door clearance for the live floor (`world::door_clearance`). The Godot pool lists every wall-backed cell of a room's
/// rectangle, which includes the outer cells of a wide doorway and the cells in front of a door; a locker or a vent on
/// one is in the way of the door. A wall fixture whose cell `blocked(kind, cell)` refuses (kind "hiding_spot" or
/// "vent") moves to the nearest pool cell within `radius` that `blocked` allows, that no prop neighbourhood, `avoid`
/// cell or other fixture (less than 5 cells away) takes; with none it is dropped.
/// Nothing is drawn from the RNG, so the same chunk always gives the same floor. Prop-based hiding spots (`hide`) stay.
/// Fixtures that need no move keep their cell, order and ids.
pub fn clear_of_doors(
    fixtures: &[Fixture],
    grid: &Grid,
    rooms: &[Room],
    stairs: &[V2],
    region_props: &[Prop],
    safe: bool,
    avoid: &[V2],
    radius: i32,
    blocked: &dyn Fn(&str, V2) -> bool,
) -> Vec<Fixture> {
    let mut excluded = excluded_cells(stairs, region_props, safe);
    excluded.extend(avoid.iter().copied());
    let candidates = pool(grid, rooms, &excluded);
    let mut out: Vec<Fixture> = fixtures.to_vec();
    let mut dropped: Vec<usize> = Vec::new();
    for i in 0..out.len() {
        if out[i].hide.is_some() || !blocked(out[i].kind, out[i].cell) {
            continue;
        }
        let from = out[i].cell;
        let kind = out[i].kind;
        let mut best: Option<(i32, V2)> = None;
        for &cell in &candidates {
            let (dx, dz) = (cell.x - from.x, cell.y - from.y);
            if dx.abs() > radius || dz.abs() > radius || blocked(kind, cell) {
                continue;
            }
            let apart = out
                .iter()
                .enumerate()
                .all(|(j, other)| j == i || dropped.contains(&j) || distance(cell, other.cell) >= 5.0);
            if !apart {
                continue;
            }
            let d = dx * dx + dz * dz;
            if best.map_or(true, |(bd, bc)| d < bd || (d == bd && (cell.y, cell.x) < (bc.y, bc.x))) {
                best = Some((d, cell));
            }
        }
        match best {
            Some((_, cell)) => {
                let backing_wall = backing_wall_direction(grid, cell);
                let open_direction = backing_wall.neg();
                out[i].cell = cell;
                out[i].backing_wall = backing_wall;
                out[i].open_direction = open_direction;
                out[i].yaw = props::room_facing_yaw(cell, cell.add(open_direction));
            }
            None => dropped.push(i),
        }
    }
    out.into_iter().enumerate().filter(|(i, _)| !dropped.contains(i)).map(|(_, f)| f).collect()
}

/// `_prop_hiding_fixtures` (streamed_route_generator.gd:5307-5335): every solid room-template bed and wardrobe becomes
/// a hiding spot whose cell is the open floor in front of it. No RNG draws. Empty on a safe chunk (5308).
pub fn prop_hiding_fixtures(c: Coord, region_props: &[Prop], safe: bool) -> Vec<Fixture> {
    if safe {
        return Vec::new();
    }
    let tag = props::chunk_tag(c);
    let mut result: Vec<Fixture> = Vec::new();
    for prop in region_props {
        let Some(style) = hiding_style(prop.prop_type) else {
            continue;
        };
        if !prop.solid || prop.room_template.as_deref().unwrap_or("").is_empty() {
            continue;
        }
        let cell = prop.cell;
        let offset = prop.placement_offset.unwrap_or([0.0, 0.0]);
        let open_direction = prop.open_direction.unwrap_or(V2::new(0, 1));
        let size = prop.blocker_size;
        // `local = Vector2(cell) + Vector2(.5, .5) + offset` (Vector2, f32)
        let local = [cell.x as f32 + 0.5 + offset[0], cell.y as f32 + 0.5 + offset[1]];
        // `front = local + Vector2(open_direction) * (size.z * .5 + .45)`: the scalar is a double, the vector f32
        let reach = (size[2] as f64 * 0.5 + 0.45) as f32;
        let front = [
            local[0] + open_direction.x as f32 * reach,
            local[1] + open_direction.y as f32 * reach,
        ];
        // The fixture's cell is the open floor in front of the furniture (5318-5322).
        let approach = V2::new(front[0].floor() as i32, front[1].floor() as i32);
        let number = result.len();
        let capacity = hide_capacity(prop.prop_type);
        result.push(Fixture {
            id: format!("{tag}:FIXTURE:HIDE_{}:{number}", style.to_uppercase()),
            kind: "hiding_spot",
            cell: approach,
            yaw: prop.yaw,
            backing_wall: open_direction.neg(),
            open_direction,
            hide: Some(HideSpot {
                style,
                prop_type: prop.prop_type,
                capacity,
                prop_id: prop.id.clone(),
                prop_cell: cell,
                local_position: local,
                blocker_size: size,
            }),
        });
    }
    result
}

/// `HIDING_PROP_STYLES.get(type)` (5301-5304): the style of a hideable prop type, or `None`.
fn hiding_style(prop_type: &str) -> Option<&'static str> {
    HIDING_PROP_STYLES
        .iter()
        .find(|(t, _)| *t == prop_type)
        .map(|(_, style)| *style)
        // the data kinds (data/hiding.json prop_styles): cell bunks, planters. Not under the Godot goldens
        // (`layout::with_godot_regions`): Godot hides only in its own table, so a Godot bunk bed or mall planter is no spot
        .or_else(|| if super::layout::godot_regions() { None } else { crate::world::fixtures::hide_rules().prop_style(prop_type) })
}

/// `PropModelCatalog.hide_capacity` for the hideable types (see `HideSpot::capacity`).
fn hide_capacity(prop_type: &str) -> i32 {
    match prop_type {
        "bedroomWardrobe" | "gothicWardrobe" => 2,
        // a data kind takes its capacity from data/hiding.json; the Godot beds stay 1
        _ if super::layout::godot_regions() => 1,
        other => {
            let rules = crate::world::fixtures::hide_rules();
            rules.prop_style(other).and_then(|s| rules.capacity_of(s)).map_or(1, |c| c as i32)
        }
    }
}

/// `_stair_door_exclusion` (streamed_route_generator.gd:5177-5187): the full flight of every stair, each cell with
/// radius 3 of clearance.
fn stair_door_exclusion(stairs: &[V2]) -> HashSet<V2> {
    let mut excluded = HashSet::new();
    for anchor in stairs {
        for path_cell in stair_path_cells(*anchor) {
            exclude_neighbourhood(&mut excluded, path_cell, 3);
        }
    }
    excluded
}

/// `stair_path_cells` (streamed_route_generator.gd:1655-1663): the anchor and the eight cells that follow it, running
/// toward the chunk centre along the longer axis (`signi` of the delta, the x axis on a tie).
fn stair_path_cells(cell: V2) -> Vec<V2> {
    let dx = CENTER - cell.x;
    let dz = CENTER - cell.y;
    let direction = if dx.abs() >= dz.abs() {
        V2::new(dx.signum(), 0)
    } else {
        V2::new(0, dz.signum())
    };
    (0..=STAIR_RUN_CELLS)
        .map(|index| V2::new(cell.x + direction.x * index, cell.y + direction.y * index))
        .collect()
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600-5604): the square of `radius` round `cell`.
fn exclude_neighbourhood(excluded: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            excluded.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}

/// `_wall_count` (streamed_route_generator.gd:5607-5615): the sides of `cell` that are wall or outside the chunk.
fn wall_count(grid: &Grid, cell: V2) -> i32 {
    const SIDES: [V2; 4] = [V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)];
    SIDES
        .iter()
        .filter(|side| cell_value(grid, cell.x + side.x, cell.y + side.y) != 0)
        .count() as i32
}

/// `_backing_wall_direction` (streamed_route_generator.gd:5618-5625): the first side (up, right, down, else left) that
/// is a wall.
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

/// A grid value, with the outside of the chunk read as wall (the `_wall_count` convention).
fn cell_value(grid: &Grid, x: i32, z: i32) -> u8 {
    if in_chunk(V2::new(x, z)) {
        grid[z as usize][x as usize]
    } else {
        1
    }
}

fn in_chunk(cell: V2) -> bool {
    cell.x >= 0 && cell.y >= 0 && cell.x < CHUNK && cell.y < CHUNK
}

/// `Vector2i.distance_to` as a float.
fn distance(a: V2, b: V2) -> f64 {
    let dx = (a.x - b.x) as f64;
    let dz = (a.y - b.y) as f64;
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stair_flight_runs_toward_the_centre_on_the_longer_axis() {
        // from (0, 22) the x delta (22) is the longer one: nine cells along +x
        let path = stair_path_cells(V2::new(0, CENTER));
        assert_eq!(path.len(), 9);
        assert_eq!(path[0], V2::new(0, CENTER));
        assert_eq!(path[8], V2::new(8, CENTER));
        // a tie goes to x, as `abs(delta.x) >= abs(delta.y)`
        let tie = stair_path_cells(V2::new(0, 0));
        assert_eq!(tie[1], V2::new(1, 0));
    }

    #[test]
    fn hiding_styles_and_capacities_follow_the_godot_tables() {
        assert_eq!(hiding_style("doubleBed"), Some("bed"));
        assert_eq!(hiding_style("gothicWardrobe"), Some("wardrobe"));
        assert_eq!(hiding_style("hospitalBed"), None);
        assert_eq!(hide_capacity("bedroomWardrobe"), 2);
        assert_eq!(hide_capacity("singleBed"), 1);
    }

    #[test]
    fn data_prop_kinds_take_style_and_capacity_from_hiding_json() {
        let rules = crate::world::fixtures::hide_rules();
        assert_eq!(hiding_style("prisonBunk"), Some("bunk"));
        assert_eq!(hiding_style("raisedPlanter"), Some("planter"));
        assert_eq!(hide_capacity("prisonBunk") as usize, rules.capacity_of("bunk").unwrap());
        assert_eq!(hide_capacity("raisedPlanter") as usize, rules.capacity_of("planter").unwrap());
        // the Godot-parity entries are untouched
        assert_eq!(hiding_style("singleBed"), Some("bed"));
    }

    #[test]
    fn spacing_rejects_fixtures_closer_than_five_cells() {
        // the separation test is `distance_to < 5.0` (5369): exactly 5.0 is separated
        assert!(distance(V2::new(0, 0), V2::new(4, 3)) >= 5.0);
        assert!(distance(V2::new(0, 0), V2::new(3, 3)) < 5.0);
    }
}

/// The parity checks of `fixtures` against the Godot goldens. `fixtures_match_godot_compact_extracts` reads the
/// committed extracts `tests/parity/fields-fixtures-<tag>.json` (the level-0 chunks of each golden); the full
/// goldens `golden-<tag>.json` in PARITY_DIR are checked by the ignored `fixtures_match_godot_goldens`.
#[cfg(test)]
mod parity_tests {
    use super::*;
    use crate::world::route::layout::chunk_layout;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];
    /// Tolerance for the f32 fields (`local_position`, `blocker_size`): the goldens print about six digits.
    const F32_TOLERANCE: f64 = 1e-5;

    /// The level-0 chunks of one golden: the world seed, and each chunk's coord with its Godot fixture entries.
    struct Golden {
        world: String,
        chunks: Vec<(Coord, Vec<Value>)>,
    }

    fn coord_of(v: &Value) -> Coord {
        let a = v.as_array().expect("coord");
        Coord::new(
            a[0].as_i64().unwrap() as i32,
            a[1].as_i64().unwrap() as i32,
            a[2].as_i64().unwrap() as i32,
        )
    }

    fn load_compact(tag: &str) -> Golden {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("parity")
            .join(format!("fields-fixtures-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: Value = serde_json::from_str(&text).expect("fixtures extract json");
        Golden {
            world: v["world_seed"].as_str().expect("world_seed").to_string(),
            chunks: v["chunks"]
                .as_array()
                .expect("chunks")
                .iter()
                .map(|c| (coord_of(&c["coord"]), c["fixtures"].as_array().expect("fixtures").clone()))
                .collect(),
        }
    }

    /// The full Godot golden `golden-<tag>.json` in PARITY_DIR (required). Only its level-0 chunks are read.
    fn load_full(tag: &str) -> Golden {
        let dir = std::env::var("PARITY_DIR").expect("PARITY_DIR must point at the Godot goldens");
        let path = PathBuf::from(dir).join(format!("golden-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: Value = serde_json::from_str(&text).expect("golden json");
        let chunks = v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .map(|c| (coord_of(&c["coord"]), c["data"]["fixtures"].as_array().expect("fixtures").clone()))
            .filter(|(c, _)| c.y == 0)
            .collect();
        Golden { world: v["world_seed"].as_str().expect("world_seed").to_string(), chunks }
    }

    fn cell_of(v: &Value) -> V2 {
        let a = v.as_array().expect("cell");
        V2::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32)
    }

    /// A number triple or pair as the goldens write it: a JSON array, or a Godot `str()` string such as "(1.9, 0.92, 2.7)".
    fn numbers(v: &Value) -> Vec<f64> {
        match v {
            Value::Array(a) => a.iter().map(|x| x.as_f64().expect("number")).collect(),
            Value::String(s) => s
                .trim_matches(|c| c == '(' || c == ')')
                .split(',')
                .map(|p| p.trim().parse::<f64>().expect("number"))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn close(ours: &[f64], want: &[f64]) -> bool {
        ours.len() == want.len() && ours.iter().zip(want).all(|(a, b)| (a - b).abs() <= F32_TOLERANCE)
    }

    /// The first field that differs between our fixture and the Godot entry, or `None` when they match. The
    /// hiding fields are compared when either side has them.
    fn fixture_difference(ours: &Fixture, want: &Value) -> Option<String> {
        if want["kind"].as_str() != Some(ours.kind) {
            return Some(format!("kind {} vs Godot {}", ours.kind, want["kind"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs Godot {}", (ours.cell.x, ours.cell.y), want["cell"]));
        }
        let want_yaw = want["yaw"].as_f64().unwrap_or(0.0);
        if (ours.yaw - want_yaw).abs() > 1e-9 {
            return Some(format!("yaw {} vs Godot {}", ours.yaw, want_yaw));
        }
        if cell_of(&want["backing_wall"]) != ours.backing_wall {
            return Some(format!("backing_wall {:?} vs Godot {}", (ours.backing_wall.x, ours.backing_wall.y), want["backing_wall"]));
        }
        if cell_of(&want["open_direction"]) != ours.open_direction {
            return Some(format!("open_direction {:?} vs Godot {}", (ours.open_direction.x, ours.open_direction.y), want["open_direction"]));
        }
        match (&ours.hide, want.get("style")) {
            (None, None) => None,
            (None, Some(_)) => Some("hiding fields in Godot, none made here".to_string()),
            (Some(_), None) => Some("hiding fields made here, none in Godot".to_string()),
            (Some(h), Some(style)) => {
                if style.as_str() != Some(h.style) {
                    return Some(format!("style {} vs Godot {}", h.style, style));
                }
                if want["prop_type"].as_str() != Some(h.prop_type) {
                    return Some(format!("prop_type {} vs Godot {}", h.prop_type, want["prop_type"]));
                }
                if want["capacity"].as_i64() != Some(i64::from(h.capacity)) {
                    return Some(format!("capacity {} vs Godot {}", h.capacity, want["capacity"]));
                }
                if want["prop_id"].as_str() != Some(h.prop_id.as_str()) {
                    return Some(format!("prop_id {} vs Godot {}", h.prop_id, want["prop_id"]));
                }
                if cell_of(&want["prop_cell"]) != h.prop_cell {
                    return Some(format!("prop_cell {:?} vs Godot {}", (h.prop_cell.x, h.prop_cell.y), want["prop_cell"]));
                }
                let local: Vec<f64> = h.local_position.iter().map(|v| f64::from(*v)).collect();
                if !close(&local, &numbers(&want["local_position"])) {
                    return Some(format!("local_position {:?} vs Godot {}", h.local_position, want["local_position"]));
                }
                let size: Vec<f64> = h.blocker_size.iter().map(|v| f64::from(*v)).collect();
                if !close(&size, &numbers(&want["blocker_size"])) {
                    return Some(format!("blocker_size {:?} vs Godot {}", h.blocker_size, want["blocker_size"]));
                }
                None
            }
        }
    }

    /// Our fixtures of one chunk against the Godot list, by id. Returns the differences (empty when the chunk
    /// matches) and how many Godot entries matched.
    fn compare_chunk(ours: &[Fixture], wanted: &[Value]) -> (Vec<String>, usize) {
        let by_id: HashMap<&str, &Fixture> = ours.iter().map(|f| (f.id.as_str(), f)).collect();
        let mut diffs = Vec::new();
        let mut matched = 0;
        for want in wanted {
            let id = want["id"].as_str().unwrap_or("");
            match by_id.get(id) {
                Some(f) => match fixture_difference(f, want) {
                    None => matched += 1,
                    Some(why) => diffs.push(format!("{id}: {why}")),
                },
                None => diffs.push(format!("{id}: not made (Godot {} at {})", want["kind"], want["cell"])),
            }
        }
        let wanted_ids: HashSet<&str> = wanted.iter().filter_map(|w| w["id"].as_str()).collect();
        for f in ours {
            if !wanted_ids.contains(f.id.as_str()) {
                diffs.push(format!("{}: made here, not in Godot", f.id));
            }
        }
        (diffs, matched)
    }

    fn check(tags: &[&str], load: fn(&str) -> Golden) {
        let mut report = String::new();
        let mut failures: Vec<String> = Vec::new();
        for tag in tags {
            let golden = load(tag);
            let (mut chunks, mut matched_chunks, mut no_layout) = (0usize, 0usize, 0usize);
            let (mut entries, mut matched_entries, mut made, mut prop_made) = (0usize, 0usize, 0usize, 0usize);
            for (c, wanted) in &golden.chunks {
                chunks += 1;
                entries += wanted.len();
                let Some(layout) = crate::world::route::layout::chunk_layout_godot(&golden.world, *c) else {
                    no_layout += 1;
                    failures.push(format!("{tag} [{},{},{}]: no layout", c.x, c.y, c.z));
                    continue;
                };
                made += layout.fixtures.len();
                prop_made += layout.fixtures.iter().filter(|f| f.hide.is_some()).count();
                let (diffs, matched) = compare_chunk(&layout.fixtures, wanted);
                matched_entries += matched;
                if diffs.is_empty() {
                    matched_chunks += 1;
                }
                for diff in diffs {
                    failures.push(format!("{tag} [{},{},{}] {diff}", c.x, c.y, c.z));
                }
            }
            report.push_str(&format!(
                "seed {tag} (world {}): level-0 chunks {chunks}, matched {matched_chunks}, not laid out {no_layout}; \
                 Godot entries {entries}, matched {matched_entries}; made here {made} (prop-based {prop_made})\n",
                golden.world
            ));
        }
        eprintln!("{report}");
        for line in failures.iter().take(25) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} fixture differences from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }

    #[test]
    fn fixtures_match_godot_compact_extracts() {
        crate::world::route::layout::with_godot_regions(|| {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        let _godot = crate::world::route::furnished::godot_layout();
        check(&TAGS, load_compact);
            });
    }

    #[test]
    #[ignore = "full Godot goldens: set PARITY_DIR to the golden directory and run with --ignored"]
    fn fixtures_match_godot_goldens() {
        check(&TAGS, load_full);
    }
}
