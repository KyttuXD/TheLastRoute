//! The Infinite route's chunk layout (stage 2): a port of StreamedRouteGenerator's room planning
//! for the maze level, level 0, from
//! `LastRoute Godot/KyttusLabApp/game/world/generation/streamed_route_generator.gd`.
//!
//! What is ported, with the Godot names and line numbers kept in each doc comment:
//! - `_region_layout` (1722) and its three paths that level 0 can reach: `_maze_warren_layout`
//!   (1856, warrens), `_dense_linear_layout` (2042, the dense banks of every dense district) and
//!   `_mixed_room_layout` (2230, apartments only: every other region reaching it is dense).
//! - The seams: `edge_openings` (1374), `wide_seam` (1391), `_carve_edges` (1549).
//! - The great halls and music wing that shape the banks: `large_plan` (1309), `_base_large_plan`
//!   (1329), `_wing_roll` (1354), `hall_trimmed` (1363), with `MusicWing.carve`
//!   (`game/world/regions/music_wing.gd`, 55).
//! - The office and apartment suites: `_carve_suite_cluster` (2375), `_carve_apartment_suite`
//!   (2487), `_connect_room_to_main_hall` (2592).
//! - The level-0 stairwell step of `_build_chunk` (984-1000): `_plan_maze_stairwell` (575),
//!   `_maze_stairwell_fits` (592), `_carve_stairwell` (1081), with `Stairwells.plan`
//!   (`game/world/generation/stairwells.gd`, 46) and the two seed-only sewer draws it reads,
//!   `ramp_between` and `falls_between` (`sewer_route_generator.gd`, 136 and 189).
//!
//! Not ported, because level 0 never reaches them: `_linear_room_layout` (1736, hotel/prison are
//! dense), the stairs paths (`stair_links` is always empty), the mezzanine well and the void
//! (MEZZANINE_ENABLED is false), `_mixed_room_layout` for non-apartment regions, and the sewer and
//! Inferno levels (-1, -2, -3), which belong to SewerRouteGenerator and InfernoCaverns.
//! Grid post-processing (thresholds, hump removal, props, pickups) is stage 3 and not needed for
//! the fields compared here: rooms, corridors, suites, edges and stairs.
//!
//! Every SeededRandom draw is made in the Godot order from the same seed text, using `Rng`
//! (bit-exact with SeededRandom). Ordered Godot dictionaries (warren nodes, tree edges) are
//! ordered `Vec`s here, so pop_at and index draws land on the same entries.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::world::rng::Rng;
use crate::world::route::{fixtures, grid, hunter_spawns, music_boxes, pickups, props, regions};

/// Cells per chunk side (`CHUNK_SIZE`, streamed_route_generator.gd:15 via StreamedRouteGenerator).
pub const CHUNK: i32 = 45;
const N: usize = 45;
/// The chunk's centre row and column (`CHUNK_SIZE / 2`).
const CENTER: i32 = CHUNK / 2;
/// `CORRIDOR_WIDTH` (line 63).
const CORRIDOR_WIDTH: i32 = 3;

/// `grid[z][x]`: 1 solid, 0 open (`_solid_grid` and the carve helpers).
pub type Grid = [[u8; N]; N];

pub fn solid_grid() -> Grid {
    [[1u8; N]; N]
}

/// `DENSE_DISTRICTS` (line 73): the districts laid out as banks along a central cross.
const DENSE_DISTRICTS: [&str; 12] = [
    "hotel", "prison", "maze", "mansion", "beach", "gothic", "carnival", "inferno", "hospital", "lab",
    "greenhouse", "mall",
];
/// `WARREN_REGIONS` (line 1835): the dense districts whose maze chunks may be warrens.
const WARREN_REGIONS: [&str; 11] = [
    "maze", "hospital", "lab", "greenhouse", "mall", "carnival", "hotel", "prison", "mansion", "beach", "gothic",
];
/// `WARREN_ROOM_EXCLUDE` (line 1837).
const WARREN_ROOM_EXCLUDE: [&str; 2] = ["ward", "pod_room"];
/// `WARREN_SHARE` (line 1832).
const WARREN_SHARE: f64 = 0.55;
/// `WARREN_CELL` (line 1839).
const WARREN_CELL: i32 = 6;
/// `MUSIC_WING_REGIONS` (line 118): the hotel only.
const MUSIC_WING_REGIONS: [&str; 1] = ["hotel"];
/// `MUSIC_WING_CHANCE` (line 119).
const MUSIC_WING_CHANCE: f64 = 0.06;
/// `MusicWing.LARGE_TYPE` (music_wing.gd:18).
pub const MUSIC_WING_LARGE_TYPE: &str = "rehearsal_hall";
/// `MALL_SHOP_WIDTH` (line 101).
const MALL_SHOP_WIDTH: i32 = 20;
/// `LARGE_ROOM_SIZE` (line 116).
const LARGE_ROOM_SIZE: i32 = 21;

/// `LARGE_ROOM_CHANCE` (lines 109-110).
fn large_chance(region: &str) -> Option<f64> {
    Some(match region {
        "prison" => 0.30,
        "hotel" => 0.25,
        "mansion" => 0.30,
        "hospital" => 0.25,
        "lab" => 0.30,
        "greenhouse" => 0.45,
        "carnival" => 0.35,
        _ => return None,
    })
}

/// `LARGE_ROOM_TYPES` (lines 111-115).
fn large_types(region: &str) -> &'static [&'static str] {
    match region {
        "maze" => &["restaurant_dining"],
        "prison" => &["prison_cafeteria"],
        "hotel" => &["restaurant_dining"],
        "mansion" => &["dining_room"],
        "hospital" => &["ward"],
        "lab" => &["containment", "server_hall"],
        "greenhouse" => &["dome_atrium", "glasshouse"],
        "carnival" => &["big_top"],
        _ => &[],
    }
}

/// `BANK_BIG_CHANCE` (lines 77-78); a district without an entry draws `0.0`.
fn bank_big_chance(region: &str) -> f64 {
    match region {
        "mansion" => 0.5,
        "carnival" => 0.5,
        "gothic" => 0.35,
        "inferno" => 0.35,
        "beach" => 0.4,
        "maze" => 0.2,
        "hospital" => 0.5,
        "lab" => 0.55,
        "greenhouse" => 0.55,
        _ => 0.0,
    }
}

/// `BANK_BIG_TYPES` (lines 79-87).
fn bank_big_types(region: &str) -> &'static [&'static str] {
    match region {
        "mansion" => &["drawing_room", "dining_room"],
        "carnival" => &["midway", "carnival_stage"],
        "gothic" => &["crypt", "chapel", "archive"],
        "inferno" => &["forge", "thermal_chamber", "lava_gallery", "altar_room"],
        "beach" => &["cove", "tide_pool"],
        "maze" => &["restaurant_dining", "storage", "arcade_room"],
        "hospital" => &["ward", "operating_theatre", "icu", "nurses_station", "imaging"],
        "lab" => &["server_hall", "containment", "clean_room", "chem_lab", "bio_lab", "control_room"],
        "greenhouse" => &["glasshouse", "palm_house"],
        _ => &[],
    }
}

/// `HOSPITAL_ROOMS`: data/regions/hospital.json room_roles (checked by a test).
const HOSPITAL_ROOMS: [&str; 9] = [
    "ward", "nurses_station", "waiting_room", "operating_theatre", "icu", "morgue", "pharmacy", "imaging",
    "supply_closet",
];
/// `MALL_ROOMS`: data/regions/mall.json room_roles (checked by a test).
const MALL_ROOMS: [&str; 14] = [
    "donut_shop", "cinema", "corner_store", "food_court", "bakery", "laundromat", "electronics", "night_diner",
    "drugstore", "florist", "barber", "pawn_shop", "stationery", "music_store",
];
/// `LAB_ROOMS` (lines 92-93).
const LAB_ROOMS: [&str; 9] = [
    "clean_room", "chem_lab", "control_room", "containment", "bio_lab", "decon", "server_hall", "cold_storage",
    "lab_office",
];
/// `GREENHOUSE_ROOMS` (lines 94-95).
const GREENHOUSE_ROOMS: [&str; 6] = [
    "glasshouse", "potting_shed", "palm_house", "cactus_room", "irrigation_room", "overgrown_room",
];

/// `_room_type_choices` (line 2215): the room-type names of a district, duplicates kept.
pub fn room_type_choices(region: &str) -> Vec<&'static str> {
    match region {
        "mansion" => vec!["drawing_room", "bedroom", "dining_room", "luxury_bathroom"],
        "beach" => vec!["cove", "beach_hut", "tide_pool", "boardwalk_room"],
        "inferno" => vec!["thermal_chamber", "forge", "altar_room", "lava_gallery"],
        "gothic" => vec!["crypt", "chapel", "mourning_room", "archive"],
        "apartment" => vec!["bedroom", "kitchen", "living_room", "house_bathroom"],
        "carnival" => vec!["big_top", "midway", "game_stall", "backstage"],
        "maze" => vec!["restaurant_dining", "arcade_room", "public_restroom", "storage"],
        "prison" => vec!["prison_cell", "prison_cafeteria", "prison_cell", "prison_cell"],
        "hospital" => HOSPITAL_ROOMS.to_vec(),
        "lab" => LAB_ROOMS.to_vec(),
        "greenhouse" => GREENHOUSE_ROOMS.to_vec(),
        "mall" => MALL_ROOMS.to_vec(),
        _ => vec!["room"],
    }
}

/// `RoomSizeProfiles.PROFILES` min and max (room_size_profiles.gd), generated from that table.
/// A type with no profile is `None` (`within` is then true).
fn profile_min_max(room_type: &str) -> Option<((i32, i32), (i32, i32))> {
    match room_type {
        "music_stage" => Some(((9, 9), (21, 21))),
        "carnival_stage" => Some(((9, 9), (21, 9))),
        "practice_room" => Some(((9, 9), (13, 13))),
        "instrument_storage" => Some(((9, 9), (11, 11))),
        "recording_booth" => Some(((9, 9), (13, 13))),
        "music_store" => Some(((9, 9), (21, 9))),
        "restaurant_dining" => Some(((11, 8), (21, 21))),
        "prison_cafeteria" => Some(((11, 9), (21, 21))),
        "dining_room" => Some(((9, 9), (21, 21))),
        "family_dining" => Some(((9, 9), (13, 11))),
        "drawing_room" => Some(((9, 9), (21, 9))),
        "big_top" => Some(((9, 9), (21, 21))),
        "midway" => Some(((9, 9), (21, 9))),
        "kitchen" => Some(((9, 9), (13, 13))),
        "archive" => Some(((9, 9), (21, 9))),
        "chapel" => Some(((9, 9), (21, 9))),
        "crypt" => Some(((9, 9), (21, 9))),
        "storage" => Some(((9, 8), (21, 21))),
        "cove" => Some(((9, 9), (21, 9))),
        "tide_pool" => Some(((9, 9), (21, 9))),
        "forge" => Some(((9, 9), (21, 9))),
        "thermal_chamber" => Some(((9, 9), (21, 9))),
        "lava_gallery" => Some(((9, 9), (21, 9))),
        "altar_room" => Some(((9, 9), (21, 9))),
        "guest_room" => Some(((9, 9), (13, 11))),
        "bedroom" => Some(((9, 9), (13, 13))),
        "hotel_bathroom" => Some(((9, 9), (11, 9))),
        "house_bathroom" => Some(((9, 9), (13, 13))),
        "luxury_bathroom" => Some(((9, 9), (13, 13))),
        "public_restroom" => Some(((9, 8), (13, 13))),
        "infirmary" => Some(((9, 9), (13, 13))),
        "prison_cell" => Some(((9, 9), (11, 9))),
        "living_room" => Some(((9, 9), (15, 15))),
        "mourning_room" => Some(((9, 9), (13, 13))),
        "arcade_room" => Some(((9, 8), (21, 21))),
        "game_stall" => Some(((9, 9), (13, 13))),
        "backstage" => Some(((9, 9), (13, 13))),
        "beach_hut" => Some(((9, 9), (13, 13))),
        "boardwalk_room" => Some(((9, 9), (13, 13))),
        "ward" => Some(((9, 9), (21, 21))),
        "operating_theatre" => Some(((9, 9), (21, 9))),
        "nurses_station" => Some(((9, 9), (21, 9))),
        "waiting_room" => Some(((9, 9), (13, 13))),
        "icu" => Some(((9, 9), (21, 9))),
        "morgue" => Some(((9, 9), (13, 13))),
        "pharmacy" => Some(((9, 9), (13, 13))),
        "imaging" => Some(((9, 9), (21, 9))),
        "supply_closet" => Some(((9, 9), (13, 13))),
        "chem_lab" => Some(((9, 9), (21, 9))),
        "pod_room" => Some(((19, 9), (21, 9))),
        "bio_lab" => Some(((9, 9), (21, 9))),
        "decon" => Some(((9, 9), (13, 13))),
        "cold_storage" => Some(((9, 9), (13, 13))),
        "lab_office" => Some(((9, 9), (13, 13))),
        "control_room" => Some(((9, 9), (21, 9))),
        "potting_shed" => Some(((9, 9), (13, 13))),
        "cactus_room" => Some(((9, 9), (13, 13))),
        "irrigation_room" => Some(((9, 9), (13, 13))),
        "overgrown_room" => Some(((9, 9), (13, 13))),
        "clean_room" => Some(((9, 9), (21, 9))),
        "server_hall" => Some(((9, 9), (21, 21))),
        "containment" => Some(((9, 9), (21, 21))),
        "glasshouse" => Some(((9, 9), (21, 21))),
        "palm_house" => Some(((9, 9), (21, 9))),
        "dome_atrium" => Some(((21, 21), (21, 21))),
        "donut_shop" => Some(((9, 9), (21, 9))),
        "cinema" => Some(((9, 9), (21, 9))),
        "corner_store" => Some(((9, 9), (21, 9))),
        "food_court" => Some(((9, 9), (21, 9))),
        "bakery" => Some(((9, 9), (21, 9))),
        "laundromat" => Some(((9, 9), (21, 9))),
        "electronics" => Some(((9, 9), (21, 9))),
        "night_diner" => Some(((9, 9), (21, 9))),
        "drugstore" => Some(((9, 9), (21, 9))),
        "florist" => Some(((9, 9), (21, 9))),
        "barber" => Some(((9, 9), (21, 9))),
        "pawn_shop" => Some(((9, 9), (21, 9))),
        "stationery" => Some(((9, 9), (21, 9))),
        "classroom" => Some(((15, 11), (21, 15))),
        "lecture_hall" => Some(((21, 15), (33, 21))),
        "conference_room" => Some(((13, 11), (21, 13))),
        "open_office" => Some(((21, 15), (33, 21))),
        "lobby" => Some(((15, 11), (33, 21))),
        "ballroom" => Some(((21, 21), (41, 33))),
        "warehouse" => Some(((21, 15), (41, 33))),
        "library" => Some(((21, 15), (33, 21))),
        "gym" => Some(((21, 15), (33, 21))),
        "greenhouse" => Some(((21, 9), (33, 21))),
        "workshop" => Some(((15, 11), (33, 21))),
        _ => None,
    }
}

/// `RoomSizeProfiles.within` (room_size_profiles.gd:128): `size` (either orientation) lies between the
/// profile's min and max.
fn within(room_type: &str, size: (i32, i32)) -> bool {
    let Some((lo, hi)) = profile_min_max(room_type) else {
        return true;
    };
    let short = size.0.min(size.1);
    let long = size.0.max(size.1);
    short >= lo.0.min(lo.1) && long >= lo.0.max(lo.1) && short <= hi.0.min(hi.1) && long <= hi.0.max(hi.1)
}

/// A cell or a size in Godot's Vector2i.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct V2 {
    pub x: i32,
    pub y: i32,
}

impl V2 {
    pub const fn new(x: i32, y: i32) -> V2 {
        V2 { x, y }
    }
    pub fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
    /// Vector2i unary minus.
    pub fn neg(self) -> V2 {
        V2::new(-self.x, -self.y)
    }
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
    fn mul(self, k: i32) -> V2 {
        V2::new(self.x * k, self.y * k)
    }
    /// Vector2i.sign(): the sign of each component.
    fn sign(self) -> V2 {
        V2::new(self.x.signum(), self.y.signum())
    }
    fn json(self) -> Value {
        json!([self.x, self.y])
    }
}

/// Godot's Rect2i: a position and a size, end-exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
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
    fn size(self) -> V2 {
        V2::new(self.w, self.h)
    }
    fn end(self) -> V2 {
        V2::new(self.x + self.w, self.y + self.h)
    }
    fn area(self) -> i32 {
        self.w * self.h
    }
    /// Rect2i.has_area: a positive width and height.
    fn has_area(self) -> bool {
        self.w > 0 && self.h > 0
    }
    /// Rect2i.get_center: `position + size / 2` (integer division).
    fn center(self) -> V2 {
        V2::new(self.x + self.w / 2, self.y + self.h / 2)
    }
    /// Rect2i.has_point: end-exclusive.
    fn has_point(self, p: V2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.w && p.y < self.y + self.h
    }
    /// Rect2i.intersects: the four separating-axis tests of Godot's implementation, so a zero-size
    /// rectangle still counts when it lies strictly inside the other.
    fn intersects(self, o: Rect) -> bool {
        if o.x >= self.x + self.w {
            return false;
        }
        if o.x + o.w <= self.x {
            return false;
        }
        if o.y >= self.y + self.h {
            return false;
        }
        if o.y + o.h <= self.y {
            return false;
        }
        true
    }
    /// Rect2i.grow(n): grows every side by n (negative shrinks).
    fn grow(self, n: i32) -> Rect {
        Rect::new(self.x - n, self.y - n, self.w + 2 * n, self.h + 2 * n)
    }
    /// Rect2i.merge (Godot 4): the smallest rectangle holding both.
    fn merge(self, o: Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let ex = (o.x + o.w).max(self.x + self.w);
        let ey = (o.y + o.h).max(self.y + self.h);
        Rect::new(x, y, ex - x, ey - y)
    }
    /// Rect2i.intersection: an empty rectangle when they do not intersect.
    fn intersection(self, o: Rect) -> Rect {
        if !self.intersects(o) {
            return Rect::default();
        }
        let x = o.x.max(self.x);
        let y = o.y.max(self.y);
        let ex = (o.x + o.w).min(self.x + self.w);
        let ey = (o.y + o.h).min(self.y + self.h);
        Rect::new(x, y, ex - x, ey - y)
    }
    fn json(self) -> Value {
        json!([self.x, self.y, self.w, self.h])
    }
}

/// A chunk coordinate: `x`, the level `y` (0 is the maze), and `z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Coord {
    pub const fn new(x: i32, y: i32, z: i32) -> Coord {
        Coord { x, y, z }
    }
    fn step(self, dx: i32, dz: i32) -> Coord {
        Coord::new(self.x + dx, self.y, self.z + dz)
    }
}

/// `chunk_id` (line 1524): `"x:y:z"`.
fn chunk_id(c: Coord) -> String {
    format!("{}:{}:{}", c.x, c.y, c.z)
}

fn cell(grid: &Grid, x: i32, z: i32) -> u8 {
    grid[z as usize][x as usize]
}

fn set(grid: &mut Grid, x: i32, z: i32, value: u8) {
    grid[z as usize][x as usize] = value;
}

/// A room of the chunk: the Godot `_room_entry` dictionary (line 2744) plus the keys written later.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub id: String,
    pub region: &'static str,
    pub room_type: String,
    pub rect: Rect,
    pub doorway: Option<V2>,
    /// Set by the big-top hall and by apartment children; `None` when Godot never wrote the key.
    pub extra_doorways: Option<Vec<V2>>,
    pub great_hall: bool,
    pub preset_room: bool,
    pub suite_id: Option<String>,
    pub suite_kind: Option<String>,
    pub suite_role: Option<String>,
    pub suite_parent: bool,
}

impl Room {
    fn json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("region".into(), json!(self.region));
        m.insert("room_type".into(), json!(self.room_type));
        m.insert("rect".into(), self.rect.json());
        m.insert("size".into(), self.rect.size().json());
        m.insert("center".into(), self.rect.center().json());
        if let Some(d) = self.doorway {
            m.insert("doorway".into(), d.json());
        }
        if let Some(extra) = &self.extra_doorways {
            m.insert("extra_doorways".into(), Value::Array(extra.iter().map(|d| d.json()).collect()));
        }
        if self.great_hall {
            m.insert("great_hall".into(), json!(true));
        }
        if self.preset_room {
            m.insert("preset_room".into(), json!(true));
        }
        if let Some(s) = &self.suite_id {
            m.insert("suite_id".into(), json!(s));
        }
        if let Some(s) = &self.suite_kind {
            m.insert("suite_kind".into(), json!(s));
        }
        if let Some(s) = &self.suite_role {
            m.insert("suite_role".into(), json!(s));
        }
        if self.suite_parent {
            m.insert("suite_parent".into(), json!(true));
        }
        Value::Object(m)
    }
}

/// `_room_entry` (streamed_route_generator.gd:2744).
fn room_entry(c: Coord, index: i32, region: &'static str, rect: Rect, room_type: &str) -> Room {
    Room {
        id: format!("{}:ROOM:{}", chunk_id(c), index),
        region,
        room_type: room_type.to_string(),
        rect,
        doorway: None,
        extra_doorways: None,
        great_hall: false,
        preset_room: false,
        suite_id: None,
        suite_kind: None,
        suite_role: None,
        suite_parent: false,
    }
}

/// A corridor dictionary (`corridors` entries). Only the keys Godot wrote are `Some`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Corridor {
    pub id: String,
    pub axis: &'static str,
    pub width: i32,
    pub rect: Option<Rect>,
    pub length: Option<i32>,
    pub branch: Option<bool>,
    pub from_main_hall: Option<bool>,
    pub serves_many_rooms: Option<bool>,
    pub doorway: Option<V2>,
    pub suite_id: Option<String>,
    pub from_perimeter: Option<bool>,
    pub room_id: Option<String>,
}

impl Corridor {
    fn json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("axis".into(), json!(self.axis));
        m.insert("width".into(), json!(self.width));
        if let Some(r) = self.rect {
            m.insert("rect".into(), r.json());
        }
        if let Some(l) = self.length {
            m.insert("length".into(), json!(l));
        }
        if let Some(b) = self.branch {
            m.insert("branch".into(), json!(b));
        }
        if let Some(b) = self.from_main_hall {
            m.insert("from_main_hall".into(), json!(b));
        }
        if let Some(b) = self.serves_many_rooms {
            m.insert("serves_many_rooms".into(), json!(b));
        }
        if let Some(d) = self.doorway {
            m.insert("doorway".into(), d.json());
        }
        if let Some(s) = &self.suite_id {
            m.insert("suite_id".into(), json!(s));
        }
        if let Some(b) = self.from_perimeter {
            m.insert("from_perimeter".into(), json!(b));
        }
        if let Some(s) = &self.room_id {
            m.insert("room_id".into(), json!(s));
        }
        Value::Object(m)
    }
}

/// `_carve_stairwell` drops (`MAX_*` below) and `Stairwells.plan` results, as the fields the layout
/// keeps: the well's `box`, `door`, `north`, and its walk cells (the landing and the door row).
#[derive(Clone, Debug, PartialEq)]
pub struct Stairwell {
    pub kind: &'static str,
    pub north: bool,
    pub quadrant: i32,
    pub bbox: Rect,
    pub door: Rect,
    pub walk: Vec<V2>,
}

/// A suite (the `suites` entries, streamed_route_generator.gd:2375 and 2487).
#[derive(Clone, Debug, PartialEq)]
pub struct Suite {
    pub id: String,
    pub kind: String,
    pub region: &'static str,
    pub rect: Rect,
    pub entry_doorway: V2,
    pub parent_room_id: String,
    pub room_ids: Vec<String>,
}

impl Suite {
    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "region": self.region,
            "rect": self.rect.json(),
            "entry_doorway": self.entry_doorway.json(),
            "parent_room_id": self.parent_room_id,
            "room_ids": self.room_ids,
        })
    }
}

/// The circulation mask of a chunk (`circulation` in the Godot layout dictionary): the central halls,
/// room connectors, chunk seams and stairwell lanes that solid dressing must never take. Only the
/// cells matter (Godot stores them as `true`).
pub type Circ = HashSet<V2>;

/// `_mark_rect_cells` (streamed_route_generator.gd:2818): every cell of `rect`, clipped to 0..CHUNK_SIZE.
pub fn mark_rect(circ: &mut Circ, rect: Rect) {
    for z in rect.y.max(0)..(rect.y + rect.h).min(CHUNK) {
        for x in rect.x.max(0)..(rect.x + rect.w).min(CHUNK) {
            circ.insert(V2::new(x, z));
        }
    }
}

/// `_mark_corridor_cell` (streamed_route_generator.gd:2807): a `width` square round `cell`, clipped to
/// 0..CHUNK_SIZE (not to the open interior, unlike `_carve_corridor_cell`).
pub fn mark_corridor_cell(circ: &mut Circ, cell_at: V2, width: i32) {
    let half = width / 2;
    for dz in -half..=half {
        for dx in -half..=half {
            let at = cell_at.add(V2::new(dx, dz));
            if at.x >= 0 && at.y >= 0 && at.x < CHUNK && at.y < CHUNK {
                circ.insert(at);
            }
        }
    }
}

/// `_mark_corridor_cells` (streamed_route_generator.gd:2795): the walk of `_carve_corridor`, marked.
pub fn mark_corridor_cells(circ: &mut Circ, from: V2, to: V2, width: i32) {
    let mut cursor = from;
    while cursor.x != to.x {
        mark_corridor_cell(circ, cursor, width);
        cursor.x += if cursor.x > to.x { -1 } else { 1 };
    }
    while cursor.y != to.y {
        mark_corridor_cell(circ, cursor, width);
        cursor.y += if cursor.y > to.y { -1 } else { 1 };
    }
    mark_corridor_cell(circ, to, width);
}

/// The rooms, corridors, suites and circulation a region layout adds to one chunk.
#[derive(Default, Clone, Debug)]
pub struct Parts {
    pub rooms: Vec<Room>,
    pub corridors: Vec<Corridor>,
    pub suites: Vec<Suite>,
    pub circulation: Circ,
}

/// One chunk's layout: the Godot chunk dictionary's `rooms`, `corridors`, `suites`, `edges` and
/// `stairs` (`stairs` is always empty on the single maze level: stair_links returns []).
#[derive(Clone, Debug)]
pub struct ChunkLayout {
    pub coord: Coord,
    pub region: &'static str,
    pub rooms: Vec<Room>,
    pub corridors: Vec<Corridor>,
    pub suites: Vec<Suite>,
    /// Openings per side, in the order west, east, north, south (`edge_openings`).
    pub edges: [Vec<i32>; 4],
    /// The stairwells carved into the chunk (level 0 to -1). The Godot `stairwells` key, not compared.
    pub stairwells: Vec<Stairwell>,
    /// The final grid (after the hump pass), the Godot chunk's `grid`.
    pub grid: Grid,
    /// The circulation mask (`circulation` in `_region_layout`), which `_region_props` reads.
    pub circulation: Circ,
    /// The region props of the chunk after the post passes (`_region_props`, streamed_route_generator.gd:2824,
    /// then push_out, scale_surface_mounts and PlacementClearance.clear at 1024-1028: `props::post_passes`).
    /// `None` only if `_region_props` has no set for the region, which does not happen in this port.
    pub region_props: Option<Vec<props::Prop>>,
    /// The fixtures of the chunk, `_fixtures` then `_prop_hiding_fixtures` (streamed_route_generator.gd:1029-1031):
    /// `fixtures::chunk_fixtures`. Empty when the chunk has no region props.
    pub fixtures: Vec<fixtures::Fixture>,
    /// The pickups of the chunk, `_pickups` (streamed_route_generator.gd:1038) then `_ensure_pickups_reachable`
    /// (809, last): `pickups::chunk_pickups` and `pickups::ensure_pickups_reachable`. Empty on a safe chunk.
    pub pickups: Vec<pickups::Pickup>,
    /// The music boxes of the chunk, `_music_boxes` (streamed_route_generator.gd:1039): `music_boxes::chunk_music_boxes`.
    /// Read the pickups before the ensure pass. Empty on a safe chunk.
    pub music_boxes: Vec<music_boxes::MusicBox>,
    /// The hunter spawns of the chunk, `_hunter_spawns` (streamed_route_generator.gd:1040): `hunter_spawns::chunk_hunter_spawns`.
    /// Empty on a safe chunk.
    pub hunter_spawns: Vec<hunter_spawns::HunterSpawn>,
}

impl ChunkLayout {
    /// The compared fields in the Godot chunk dictionary's shape.
    pub fn to_json(&self) -> Value {
        const SIDES: [&str; 4] = ["west", "east", "north", "south"];
        let mut edges = serde_json::Map::new();
        for (i, side) in SIDES.iter().enumerate() {
            edges.insert((*side).into(), json!(self.edges[i]));
        }
        json!({
            "coord": [self.coord.x, self.coord.y, self.coord.z],
            "region": self.region,
            "rooms": self.rooms.iter().map(Room::json).collect::<Vec<_>>(),
            "corridors": self.corridors.iter().map(Corridor::json).collect::<Vec<_>>(),
            "suites": self.suites.iter().map(Suite::json).collect::<Vec<_>>(),
            "edges": Value::Object(edges),
            "stairs": [],
        })
    }
}

/// Godot `Rect2i` carve helper (`_carve_rect`, line 2767): the rectangle, clipped to the open interior 1..44.
fn carve_rect(grid: &mut Grid, rect: Rect) {
    let (x0, x1) = (rect.x.max(1), (rect.x + rect.w).min(CHUNK - 1));
    let (z0, z1) = (rect.y.max(1), (rect.y + rect.h).min(CHUNK - 1));
    for z in z0..z1 {
        for x in x0..x1 {
            set(grid, x, z, 0);
        }
    }
}

/// `_carve_corridor_cell` (line 2785): a `width` square round `cell`, inside 1..44.
fn carve_corridor_cell(grid: &mut Grid, cell_at: V2, width: i32) {
    let half = width / 2;
    for dz in -half..=half {
        for dx in -half..=half {
            let at = cell_at.add(V2::new(dx, dz));
            if at.x > 0 && at.y > 0 && at.x < CHUNK - 1 && at.y < CHUNK - 1 {
                set(grid, at.x, at.y, 0);
            }
        }
    }
}

/// `_carve_corridor` (line 2773): an L-shaped corridor, x first then y.
fn carve_corridor(grid: &mut Grid, from: V2, to: V2, width: i32) {
    let mut cursor = from;
    while cursor.x != to.x {
        carve_corridor_cell(grid, cursor, width);
        cursor.x += if cursor.x > to.x { -1 } else { 1 };
    }
    while cursor.y != to.y {
        carve_corridor_cell(grid, cursor, width);
        cursor.y += if cursor.y > to.y { -1 } else { 1 };
    }
    carve_corridor_cell(grid, to, width);
}

/// `_carve_corridor_segment` (line 1607): a three-wide segment, marked as circulation.
fn carve_corridor_segment(grid: &mut Grid, circ: &mut Circ, from: V2, to: V2) {
    carve_corridor(grid, from, to, CORRIDOR_WIDTH);
    mark_corridor_cells(circ, from, to, CORRIDOR_WIDTH);
}

/// `_seal_room_perimeter_except` (line 2641): a solid wall round the rectangle, the doorway open.
fn seal_except(grid: &mut Grid, rect: Rect, doorway: V2) {
    for x in rect.x..rect.x + rect.w {
        set(grid, x, rect.y, 1);
        set(grid, x, rect.y + rect.h - 1, 1);
    }
    for z in rect.y..rect.y + rect.h {
        set(grid, rect.x, z, 1);
        set(grid, rect.x + rect.w - 1, z, 1);
    }
    set(grid, doorway.x, doorway.y, 0);
}

/// `_carve_warren_lane` (line 2031): an L of three-wide lanes between two cells, clipped to 1..44,
/// each leg marked as circulation with the same clipped rectangle.
fn carve_warren_lane(grid: &mut Grid, circ: &mut Circ, from: V2, to: V2) {
    let corner = V2::new(to.x, from.y);
    for (a, b) in [(from, corner), (corner, to)] {
        let rect = Rect::new(
            a.x.min(b.x) - 1,
            a.y.min(b.y) - 1,
            (a.x - b.x).abs() + 3,
            (a.y - b.y).abs() + 3,
        )
        .intersection(Rect::new(1, 1, CHUNK - 2, CHUNK - 2));
        carve_rect(grid, rect);
        mark_rect(circ, rect);
    }
}

/// `_safe_room_door_coordinate` (line 2632): clamp a door to leave a wall shoulder (`clampi`).
fn safe_door_coord(start: i32, finish: i32, preferred: i32) -> i32 {
    let shoulder = if finish - start >= 5 { 2 } else { 1 };
    let lo = start + shoulder;
    let hi = finish - shoulder - 1;
    if preferred < lo {
        lo
    } else if preferred > hi {
        hi
    } else {
        preferred
    }
}

/// The result of `_connect_room_to_main_hall` (line 2592).
struct Entrance {
    axis: &'static str,
    length: i32,
    doorway: V2,
}

/// `_connect_room_to_main_hall` (streamed_route_generator.gd:2592): a corridor from the room's door to
/// the centre cross, along the longer axis (or the hint, or a coin on a tie).
fn connect_room_to_main_hall(grid: &mut Grid, circ: &mut Circ, rect: Rect, sector_index: i32, rng: &mut Rng, axis_hint: &str) -> Entrance {
    let center = CENTER;
    let room_center = rect.center();
    let horizontal_door = V2::new(
        if sector_index == 0 || sector_index == 2 { rect.end().x - 1 } else { rect.x },
        safe_door_coord(rect.y, rect.end().y, room_center.y),
    );
    let vertical_door = V2::new(
        safe_door_coord(rect.x, rect.end().x, room_center.x),
        if sector_index == 0 || sector_index == 1 { rect.end().y - 1 } else { rect.y },
    );
    let horizontal_length = (horizontal_door.x - center).abs();
    let vertical_length = (vertical_door.y - center).abs();
    let connect_horizontally = if axis_hint == "east_west" {
        true
    } else if axis_hint == "north_south" {
        false
    } else if horizontal_length == vertical_length {
        rng.next_float() < 0.5
    } else {
        horizontal_length > vertical_length
    };
    let (start, target) = if connect_horizontally {
        (horizontal_door, V2::new(center, horizontal_door.y))
    } else {
        (vertical_door, V2::new(vertical_door.x, center))
    };
    carve_corridor_segment(grid, circ, start, target);
    seal_except(grid, rect, start);
    Entrance {
        axis: if connect_horizontally { "east_west" } else { "north_south" },
        length: (start.x - target.x).abs() + (start.y - target.y).abs(),
        doorway: start,
    }
}

/// `_carve_edges` (streamed_route_generator.gd:1549): the seams' lanes into the centre cross. Sides in
/// the order west, east, north, south; `wide` is `wide_seams` in that order.
fn carve_edges(grid: &mut Grid, circ: &mut Circ, edges: &[Vec<i32>; 4], dense: bool, wide: &[bool; 4]) -> Vec<Corridor> {
    const SIDES: [&str; 4] = ["west", "east", "north", "south"];
    let center = CENTER;
    let mut corridors = Vec::new();
    for (si, side) in SIDES.iter().enumerate() {
        for &offset in &edges[si] {
            let start = match si {
                0 => V2::new(1, offset),
                1 => V2::new(CHUNK - 2, offset),
                2 => V2::new(offset, 1),
                _ => V2::new(offset, CHUNK - 2),
            };
            let horizontal_side = si == 0 || si == 1;
            if dense && horizontal_side && offset != center {
                // a dense chunk's side halls already reach the border: just open the seam, no lane
                for spread in -1..=1 {
                    let x = if si == 0 { 0 } else { CHUNK - 1 };
                    set(grid, x, offset + spread, 0);
                }
                continue;
            }
            let reach = if wide[si] && offset == center { 2 } else { 1 };
            for spread in -reach..=reach {
                match si {
                    0 => set(grid, 0, offset + spread, 0),
                    1 => set(grid, CHUNK - 1, offset + spread, 0),
                    2 => set(grid, offset + spread, 0, 0),
                    _ => set(grid, offset + spread, CHUNK - 1, 0),
                }
            }
            let mut lane = start;
            let junction = V2::new(center, center);
            if horizontal_side {
                lane.x = if si == 0 { 2 } else { CHUNK - 3 };
                let hall_entry = V2::new(lane.x, center);
                carve_corridor_segment(grid, circ, start, lane);
                carve_corridor_segment(grid, circ, lane, hall_entry);
                carve_corridor_segment(grid, circ, hall_entry, junction);
            } else {
                lane.y = if si == 2 { 2 } else { CHUNK - 3 };
                let hall_entry = V2::new(center, lane.y);
                carve_corridor_segment(grid, circ, start, lane);
                carve_corridor_segment(grid, circ, lane, hall_entry);
                carve_corridor_segment(grid, circ, hall_entry, junction);
            }
            corridors.push(Corridor {
                id: format!("EDGE:{}:{}", side, offset),
                axis: side,
                width: CORRIDOR_WIDTH,
                length: Some((start.x - junction.x).abs() + (start.y - junction.y).abs()),
                from_perimeter: Some(true),
                room_id: Some(String::new()),
                ..Corridor::default()
            });
        }
    }
    corridors
}

/// The music wing's rooms and hall (`MusicWing.carve`, music_wing.gd:55).
struct WingRoom {
    room_type: &'static str,
    rect: Rect,
    doorway: V2,
}

struct Wing {
    rooms: Vec<WingRoom>,
    hall: Rect,
    /// `circulation` of `MusicWing.carve`: the door, and for the "rooms" part the hall cells and room doorways.
    circulation: Vec<V2>,
}

/// `MusicWing.carve` (game/world/regions/music_wing.gd:55) for the "stage" or "rooms" part of the slot
/// `rect` (21 x 21), entered at `door`. Rooms and the hall are carved; the slot is solid first.
fn music_wing_carve(grid: &mut Grid, rect: Rect, door: V2, part: &str) -> Wing {
    const SLOT: i32 = 21;
    let flip = door.x == rect.x;
    for z in rect.y..rect.y + rect.h {
        for x in rect.x..rect.x + rect.w {
            set(grid, x, z, 1);
        }
    }
    let place = |local: Rect| -> Rect {
        let x = rect.x + if flip { SLOT - (local.x + local.w) } else { local.x };
        Rect::new(x, rect.y + local.y, local.w, local.h)
    };
    let place_cell = |local: V2| -> V2 {
        V2::new(rect.x + if flip { SLOT - 1 - local.x } else { local.x }, rect.y + local.y)
    };
    // (type, local rect, door side) per LAYOUTS (music_wing.gd:22)
    let entries: &[(&'static str, Rect, &str)] = match part {
        "stage" => &[("music_stage", Rect::new(6, 4, 15, 13), "east")],
        _ => &[
            ("practice_room", Rect::new(0, 0, 11, 9), "south"),
            ("practice_room", Rect::new(10, 0, 11, 9), "south"),
            ("instrument_storage", Rect::new(0, 12, 11, 9), "north"),
            ("recording_booth", Rect::new(10, 12, 11, 9), "north"),
        ],
    };
    let mut rooms = Vec::new();
    for &(room_type, local, side) in entries {
        let room_rect = place(local);
        let door_local = match side {
            "east" => V2::new(local.x + local.w - 1, 10),
            "south" => V2::new(local.x + local.w / 2, local.y + local.h - 1),
            _ => V2::new(local.x + local.w / 2, local.y),
        };
        let doorway = place_cell(door_local);
        carve_rect(grid, room_rect);
        seal_except(grid, room_rect, doorway);
        rooms.push(WingRoom { room_type, rect: room_rect, doorway });
    }
    let mut hall = Rect::default();
    let mut circulation = vec![door];
    if part == "rooms" {
        hall = place(Rect::new(1, 9, 19, 3));
        carve_rect(grid, hall);
        set(grid, door.x, door.y, 0);
        for z in hall.y..hall.y + hall.h {
            for x in hall.x..hall.x + hall.w {
                circulation.push(V2::new(x, z));
            }
        }
        for room in &rooms {
            circulation.push(room.doorway);
        }
    }
    Wing { rooms, hall, circulation }
}

/// `_carve_suite_cluster` (streamed_route_generator.gd:2375) returns the suite and its parts.
struct SuiteOut {
    rooms: Vec<Room>,
    corridors: Vec<Corridor>,
    suite: Suite,
}

/// The sewer's `ramp_between(x, z, upper)` (sewer_route_generator.gd:136), reduced to the quadrant it
/// takes. `upper` is -1 (the ramp from the maze's sewer) or -2 (the collector).
fn sewer_ramp_quadrant(world: &str, x: i32, z: i32, upper: i32) -> Option<i32> {
    // RAMP_CHANCE (sewer_route_generator.gd:49): -1 .35, -2 .3
    let chance = match upper {
        -1 => 0.35,
        -2 => 0.3,
        _ => return None,
    };
    let mut rng = Rng::new(&format!("{}:RAMP:{}:{}:{}", world, x, z, upper));
    if rng.next_float() >= chance {
        return None;
    }
    let mut quadrant = rng.next_index(4) as i32;
    if upper == -2 {
        // the collector already hosts the bottom of the upper ramp in one quadrant
        if let Some(above) = sewer_ramp_quadrant(world, x, z, -1) {
            if above == quadrant {
                quadrant = (quadrant + 1) % 4;
            }
        }
    }
    Some(quadrant)
}

/// The sewer's `falls_between(x, z)` (sewer_route_generator.gd:189), reduced to its quadrant.
fn falls_quadrant(world: &str, x: i32, z: i32) -> Option<i32> {
    // FALLS_CHANCE .28
    let mut rng = Rng::new(&format!("{}:FALLS:{}:{}", world, x, z));
    if rng.next_float() >= 0.28 {
        return None;
    }
    let mut free: Vec<i32> = vec![0, 1, 2, 3];
    for upper in [-1, -2] {
        if let Some(q) = sewer_ramp_quadrant(world, x, z, upper) {
            free.retain(|&f| f != q);
        }
    }
    if free.is_empty() {
        return None;
    }
    Some(free[rng.next_index(free.len())])
}

/// `Stairwells.MAZE_AREAS` (stairwells.gd:31): the outer room bands of the maze, 9 rows deep.
const MAZE_AREAS: [Rect; 8] = [
    Rect::new(2, 0, 7, 9),
    Rect::new(11, 0, 7, 9),
    Rect::new(27, 0, 7, 9),
    Rect::new(36, 0, 7, 9),
    Rect::new(2, 36, 7, 9),
    Rect::new(11, 36, 7, 9),
    Rect::new(27, 36, 7, 9),
    Rect::new(36, 36, 7, 9),
];

/// `Stairwells.quadrant_of` (stairwells.gd:146).
fn quadrant_of(area: Rect) -> i32 {
    let c = area.center();
    (if c.x > 22 { 1 } else { 0 }) + (if c.y > 22 { 2 } else { 0 })
}

/// `Stairwells._geometry` (stairwells.gd:66), for the fields the layout uses: the box, the door, the
/// doorway side and the walk cells (upper landing plus door row). `spiral` picks the kind.
fn stairwell_geometry(area: Rect, spiral: bool) -> Stairwell {
    let wide = 7;
    let deep = if spiral { 7 } else { 9 };
    let north = area.y < 22;
    let bbox = Rect::new(
        area.x + (area.w - wide) / 2,
        area.y + if north { area.h - deep } else { 0 },
        wide,
        deep,
    );
    let interior = bbox.grow(-1);
    let flip = |row: i32| -> i32 {
        if north {
            interior.y + row
        } else {
            interior.y + interior.h - 1 - row
        }
    };
    let col = |c: i32| -> i32 { interior.x + c };
    let last = interior.h - 1;
    let door_z = if north { bbox.y + bbox.h - 1 } else { bbox.y };
    let door = Rect::new(if spiral { col(1) } else { col(0) }, door_z, if spiral { 3 } else { 5 }, 1);
    let mut walk = Vec::new();
    if spiral {
        for c in [1, 2, 3] {
            walk.push(V2::new(col(c), flip(last)));
        }
    } else {
        for row in [last - 1, last] {
            for c in 0..5 {
                walk.push(V2::new(col(c), flip(row)));
            }
        }
    }
    for cx in door.x..door.x + door.w {
        walk.push(V2::new(cx, door.y));
    }
    Stairwell {
        kind: if spiral { "spiral" } else { "stairs" },
        north,
        quadrant: quadrant_of(area),
        bbox,
        door,
        walk,
    }
}

/// `Stairwells.plan(world, x, z, 0, avoid, fits, MAZE_AREAS)` (stairwells.gd:46) for the maze's
/// 0 <-> -1 well: a chance draw, a spiral draw, then the free areas tried in seeded order until one fits.
fn stairwell_plan<F: Fn(&Stairwell) -> bool>(world: &str, x: i32, z: i32, avoid: &[i32], fits: F) -> Option<Stairwell> {
    // CHANCE[0] .12, SPIRAL_SHARE[0] .5
    let mut rng = Rng::new(&format!("{}:STAIRS:{}:{}:{}", world, x, z, 0));
    if rng.next_float() >= 0.12 {
        return None;
    }
    let mut free: Vec<Rect> = MAZE_AREAS
        .iter()
        .copied()
        .filter(|area| !avoid.contains(&quadrant_of(*area)))
        .collect();
    let spiral = rng.next_float() < 0.5;
    while !free.is_empty() {
        let area = free.remove(rng.next_index(free.len()));
        let well = stairwell_geometry(area, spiral);
        if fits(&well) {
            return Some(well);
        }
    }
    None
}

/// `Stairwell.corridor` (stairwells.gd:152) with `hall_edge` = the door's row +-1: the lane carved out
/// of the doorway (zero-height at level 0).
fn stairwell_lane(well: &Stairwell) -> Rect {
    let door = well.door;
    let hall_edge = if well.north { door.y + 1 } else { door.y - 1 };
    let x0 = door.x + door.w / 2 - 1;
    if well.north {
        Rect::new(x0, door.y + 1, 3, 0.max(hall_edge - door.y - 1))
    } else {
        Rect::new(x0, hall_edge + 1, 3, 0.max(door.y - hall_edge - 1))
    }
}

/// `_narrow_open_cells` (streamed_route_generator.gd:632): open cells of `area` (outside `bx`) whose
/// straight open run is under three on either axis, `wall` counted as solid.
fn narrow_open_cells(grid: &Grid, area: Rect, bx: Rect, wall: Rect) -> i32 {
    let mut count = 0;
    for z in area.y..area.y + area.h {
        for x in area.x..area.x + area.w {
            if cell(grid, x, z) != 0 || bx.has_point(V2::new(x, z)) {
                continue;
            }
            let mut runs = [0i32; 2];
            for (k, step) in [V2::new(1, 0), V2::new(0, 1)].iter().enumerate() {
                let mut run = 1;
                for sign in [1, -1] {
                    let mut at = V2::new(x, z).add(step.mul(sign));
                    while run < 3
                        && at.x >= 0
                        && at.y >= 0
                        && at.x < CHUNK
                        && at.y < CHUNK
                        && cell(grid, at.x, at.y) == 0
                        && !wall.has_point(at)
                    {
                        run += 1;
                        at = at.add(step.mul(sign));
                    }
                }
                runs[k] = run;
            }
            if runs[0].min(runs[1]) < 3 {
                count += 1;
            }
        }
    }
    count
}

/// `_open_components` (streamed_route_generator.gd:653): the number of open regions of the chunk.
fn open_components(grid: &Grid) -> i32 {
    let mut seen: HashSet<V2> = HashSet::new();
    let mut count = 0;
    for z in 0..CHUNK {
        for x in 0..CHUNK {
            if cell(grid, x, z) != 0 || seen.contains(&V2::new(x, z)) {
                continue;
            }
            count += 1;
            seen.insert(V2::new(x, z));
            let mut queue = vec![V2::new(x, z)];
            while let Some(at) = queue.pop() {
                for step in [V2::new(1, 0), V2::new(-1, 0), V2::new(0, 1), V2::new(0, -1)] {
                    let n = at.add(step);
                    if n.x < 0 || n.y < 0 || n.x >= CHUNK || n.y >= CHUNK || seen.contains(&n) || cell(grid, n.x, n.y) != 0 {
                        continue;
                    }
                    seen.insert(n);
                    queue.push(n);
                }
            }
        }
    }
    count
}

/// `_maze_stairwell_fits` (streamed_route_generator.gd:592): a well fits when no suite or main hall
/// touches its box, it drops at most 100 cells of plain rooms (220 when `relaxed`), no open run beside
/// it narrows, and the carve adds no pocket.
fn maze_stairwell_fits(grid: &Grid, layout: &Parts, well: &Stairwell, relaxed: bool) -> bool {
    let reach = well.bbox.grow(1);
    for suite in &layout.suites {
        if suite.rect.intersects(reach) {
            return false;
        }
    }
    for corridor in &layout.corridors {
        if corridor.id.contains(":HALL:") {
            if let Some(rect) = corridor.rect {
                if rect.intersects(well.bbox) {
                    return false;
                }
            }
        }
    }
    let mut trial_grid = *grid;
    let mut trial_rooms = layout.rooms.clone();
    // the trial works on a copy of the circulation in Godot (`circulation.duplicate()`): a throwaway here
    carve_stairwell(&mut trial_grid, &mut Circ::new(), &mut trial_rooms, well);
    let mut dropped = 0;
    for room in &layout.rooms {
        if !trial_rooms.contains(room) {
            if room.suite_id.as_deref().is_some_and(|s| !s.is_empty()) {
                return false;
            }
            dropped += room.rect.area();
        }
    }
    let limit = if relaxed { 220 } else { 100 };
    if dropped > limit {
        return false;
    }
    let around = reach.grow(3).intersection(Rect::new(0, 0, CHUNK, CHUNK));
    if narrow_open_cells(&trial_grid, around, well.bbox, Rect::default())
        > narrow_open_cells(grid, around, well.bbox, Rect::default())
    {
        return false;
    }
    for z in around.y..around.y + around.h {
        for x in around.x..around.x + around.w {
            let one = Rect::new(x, z, 1, 1);
            if narrow_open_cells(&trial_grid, one, well.bbox, well.bbox) > 0
                && (cell(grid, x, z) != 0 || narrow_open_cells(grid, one, Rect::default(), Rect::default()) == 0)
            {
                return false;
            }
        }
    }
    open_components(&trial_grid) <= open_components(grid)
}

/// `_plan_maze_stairwell` (streamed_route_generator.gd:575) for the level-0 step of `_build_chunk`:
/// the maze <-> sewer well, clear of the sewer's ramp and falls quadrants.
fn plan_maze_stairwell(world: &str, x: i32, z: i32, grid: &Grid, layout: &Parts) -> Option<Stairwell> {
    let mut avoid = Vec::new();
    if let Some(q) = sewer_ramp_quadrant(world, x, z, -1) {
        avoid.push(q);
    }
    if let Some(q) = falls_quadrant(world, x, z) {
        avoid.push(q);
    }
    stairwell_plan(world, x, z, &avoid, |well| maze_stairwell_fits(grid, layout, well, false))
}

/// `_carve_stairwell` (streamed_route_generator.gd:1081) at level 0: drops the rooms under the box, the
/// lane and the room in its way, carves the box and the walk cells, then the corridor to the hall.
fn carve_stairwell(grid: &mut Grid, circ: &mut Circ, rooms: &mut Vec<Room>, well: &Stairwell) {
    let bx = well.bbox;
    let mut lane = stairwell_lane(well);
    let mut kept = Vec::new();
    for room in rooms.drain(..) {
        let rect = room.rect;
        if rect.grow(1).intersects(bx) || rect.intersects(lane) {
            for z in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    set(grid, x, z, 1);
                }
            }
        } else {
            kept.push(room);
        }
    }
    *rooms = kept;
    for z in bx.y..bx.y + bx.h {
        for x in bx.x..bx.x + bx.w {
            set(grid, x, z, 1);
        }
    }
    for at in &well.walk {
        set(grid, at.x, at.y, 0);
    }
    let outward = if well.north { 1 } else { -1 };
    let mut row = if outward > 0 { lane.y + lane.h } else { lane.y - 1 };
    while row > 0 && row < CHUNK - 1 {
        let step = Rect::new(lane.x, row, lane.w, 1);
        let mut still = Vec::new();
        for room in rooms.drain(..) {
            let rect = room.rect;
            if rect.grow(1).intersects(step) {
                for z in rect.y..rect.y + rect.h {
                    for x in rect.x..rect.x + rect.w {
                        set(grid, x, z, 1);
                    }
                }
            } else {
                still.push(room);
            }
        }
        *rooms = still;
        let mut open = false;
        for x in step.x..step.x + step.w {
            if cell(grid, x, row) == 0 {
                open = true;
            }
        }
        if open {
            carve_rect(grid, step);
            mark_rect(circ, step);
            break;
        }
        lane = lane.merge(step);
        row += outward;
    }
    carve_rect(grid, lane);
    mark_rect(circ, lane);
}

impl<'a> Gen<'a> {
    /// `_maze_warren_layout` (streamed_route_generator.gd:1856): the five-wide spine across the chunk and
    /// four quadrants of 3x3 macro cells on a seeded spanning tree, with loops, joins and at most one
    /// 12x12 room per quadrant.
    fn warren(&self, c: Coord, grid: &mut Grid, region: &'static str) -> Parts {
        let id = chunk_id(c);
        let mut rooms: Vec<Room> = Vec::new();
        let mut corridors: Vec<Corridor> = Vec::new();
        let mut circ = Circ::new();
        let mut rng = Rng::new(&format!("{}:WARRENLAYOUT:{}", self.world, id));
        let spine = [Rect::new(1, 20, CHUNK - 2, 5), Rect::new(20, 1, 5, CHUNK - 2)];
        for (index, hall) in spine.iter().enumerate() {
            carve_rect(grid, *hall);
            mark_rect(&mut circ, *hall);
            corridors.push(Corridor {
                id: format!("{}:HALL:{}", id, if index == 0 { 1 } else { 3 }),
                rect: Some(*hall),
                width: CORRIDOR_WIDTH,
                axis: if index == 0 { "east_west" } else { "north_south" },
                serves_many_rooms: Some(true),
                ..Corridor::default()
            });
        }
        // the region's own rooms that fit a 12 x 12 corner (RoomSizeProfiles.within)
        let choices: Vec<&'static str> = if region == "hotel" {
            vec!["guest_room", "hotel_bathroom"]
        } else {
            room_type_choices(region)
        };
        let mut room_types: Vec<&'static str> = Vec::new();
        for type_name in choices {
            if within(type_name, (WARREN_CELL * 2, WARREN_CELL * 2))
                && !room_types.contains(&type_name)
                && !WARREN_ROOM_EXCLUDE.contains(&type_name)
            {
                room_types.push(type_name);
            }
        }
        for quadrant in 0..4i32 {
            let east = quadrant % 2 == 1;
            let south = quadrant >= 2;
            let origin = V2::new(if east { 25 } else { 1 }, if south { 25 } else { 1 });
            // the macro cell on the seam row (row 10 or 34) at the chunk's outer side keeps its corridor
            let seam_node = V2::new(if east { 2 } else { 0 }, 1);
            let spine_x = if east { 0 } else { 2 };
            let spine_z = if south { 0 } else { 2 };
            let mut room_corner = V2::new(-1, -1);
            if rng.next_float() < 0.65 && !room_types.is_empty() {
                let mut options: Vec<V2> = Vec::new();
                for cx in 0..2 {
                    for cz in 0..2 {
                        let corner = V2::new(cx, cz);
                        let covers_seam = Rect::new(corner.x, corner.y, 2, 2).has_point(seam_node);
                        if !covers_seam {
                            options.push(corner);
                        }
                    }
                }
                if !options.is_empty() {
                    room_corner = options[rng.next_index(options.len())];
                }
            }
            // nodes in Godot dictionary insertion order: i outer, j inner
            let mut nodes: Vec<V2> = Vec::new();
            for i in 0..3 {
                for j in 0..3 {
                    let p = V2::new(i, j);
                    if room_corner.x >= 0 && Rect::new(room_corner.x, room_corner.y, 2, 2).has_point(p) {
                        continue;
                    }
                    nodes.push(p);
                }
            }
            // spanning tree: randomised depth-first from the seam node
            let mut tree: Vec<(V2, V2)> = Vec::new();
            let mut tree_keys: HashSet<(V2, V2)> = HashSet::new();
            let edge_key = |a: V2, b: V2| -> (V2, V2) {
                if a.x * 99 + a.y < b.x * 99 + b.y {
                    (a, b)
                } else {
                    (b, a)
                }
            };
            let start = seam_node;
            let mut seen: HashSet<V2> = HashSet::new();
            seen.insert(start);
            let mut stack = vec![start];
            while let Some(&at) = stack.last() {
                let mut options: Vec<V2> = Vec::new();
                for step in [V2::new(1, 0), V2::new(-1, 0), V2::new(0, 1), V2::new(0, -1)] {
                    let next = at.add(step);
                    if nodes.contains(&next) && !seen.contains(&next) {
                        options.push(next);
                    }
                }
                if options.is_empty() {
                    stack.pop();
                    continue;
                }
                let chosen = options[rng.next_index(options.len())];
                seen.insert(chosen);
                let key = edge_key(at, chosen);
                if tree_keys.insert(key) {
                    tree.push((at, chosen));
                }
                stack.push(chosen);
            }
            // the edges not in the tree, in node order
            let mut extra: Vec<(V2, V2)> = Vec::new();
            for node in &nodes {
                for step in [V2::new(1, 0), V2::new(0, 1)] {
                    let next = node.add(step);
                    if nodes.contains(&next) && !tree_keys.contains(&edge_key(*node, next)) {
                        extra.push((*node, next));
                    }
                }
            }
            // one or two loops, each drawn from the remaining extra edges
            let loops = (if rng.next_float() < 0.6 { 1 } else { 0 }) + (if rng.next_float() < 0.25 { 1 } else { 0 });
            for _ in 0..loops {
                if extra.is_empty() {
                    break;
                }
                let pick = extra.remove(rng.next_index(extra.len()));
                if tree_keys.insert(edge_key(pick.0, pick.1)) {
                    tree.push(pick);
                }
            }
            let centre_of = |node: V2| -> V2 { origin.add(node.mul(WARREN_CELL)).add(V2::new(3, 3)) };
            for &(a, b) in &tree {
                carve_warren_lane(grid, &mut circ, centre_of(a), centre_of(b));
            }
            for node in &nodes {
                let cc = centre_of(*node);
                carve_warren_lane(grid, &mut circ, cc, cc);
            }
            // joins to the spine: the nodes on the spine side, one always, a second sometimes
            let mut spine_nodes: Vec<V2> = nodes
                .iter()
                .copied()
                .filter(|n| n.x == spine_x || n.y == spine_z)
                .collect();
            spine_nodes.sort_by_key(|a| a.x * 7 + a.y);
            let joins = 1 + if rng.next_float() < 0.5 { 1 } else { 0 };
            let join_count = joins.min(spine_nodes.len());
            for join in 0..join_count {
                let node = spine_nodes.remove(rng.next_index(spine_nodes.len()));
                let cc = centre_of(node);
                let toward = if node.x == spine_x && (node.y != spine_z || rng.next_float() < 0.5) {
                    V2::new(22, cc.y)
                } else {
                    V2::new(cc.x, 22)
                };
                carve_warren_lane(grid, &mut circ, cc, toward);
                corridors.push(Corridor {
                    id: format!("{}:WARREN-JOIN:{}:{}", id, quadrant, join),
                    axis: if toward.y == cc.y { "east_west" } else { "north_south" },
                    width: CORRIDOR_WIDTH,
                    length: Some((toward.x - cc.x).abs() + (toward.y - cc.y).abs()),
                    branch: Some(true),
                    from_main_hall: Some(true),
                    serves_many_rooms: Some(true),
                    ..Corridor::default()
                });
            }
            // the seam row out to the chunk border (rows 10 / 34): the neighbour's hall comes through there
            let seam_c = centre_of(seam_node);
            carve_warren_lane(grid, &mut circ, seam_c, V2::new(if east { CHUNK - 2 } else { 1 }, seam_c.y));
            // a plaza in a quadrant with no room
            if room_corner.x < 0 && rng.next_float() < 0.5 && nodes.contains(&V2::new(1, 1)) {
                let plaza = Rect::new(origin.x + WARREN_CELL + 1, origin.y + WARREN_CELL + 1, WARREN_CELL - 1, WARREN_CELL - 1);
                carve_rect(grid, plaza);
                mark_rect(&mut circ, plaza);
                corridors.push(Corridor {
                    id: format!("{}:PLAZA:{}", id, quadrant),
                    rect: Some(plaza),
                    width: CORRIDOR_WIDTH,
                    axis: "east_west",
                    serves_many_rooms: Some(true),
                    ..Corridor::default()
                });
            }
            // the room: a 12 x 12 corner, its door through a stub onto the nearest corridor beside it
            if room_corner.x >= 0 {
                let rect = Rect::new(
                    origin.x + room_corner.x * WARREN_CELL,
                    origin.y + room_corner.y * WARREN_CELL,
                    WARREN_CELL * 2,
                    WARREN_CELL * 2,
                );
                carve_rect(grid, rect);
                let mut door_options: Vec<(V2, V2)> = Vec::new();
                for step in [V2::new(1, 0), V2::new(-1, 0), V2::new(0, 1), V2::new(0, -1)] {
                    for along in 0..2 {
                        let inside = room_corner.add(V2::new(
                            if step.x == 0 {
                                along
                            } else if step.x > 0 {
                                1
                            } else {
                                0
                            },
                            if step.y == 0 {
                                along
                            } else if step.y > 0 {
                                1
                            } else {
                                0
                            },
                        ));
                        let outside = inside.add(step);
                        if nodes.contains(&outside) {
                            door_options.push((inside, outside));
                        }
                    }
                }
                if door_options.is_empty() {
                    door_options.push((room_corner, room_corner));
                }
                let (inside_node, outside_node) = door_options[rng.next_index(door_options.len())];
                let target = centre_of(outside_node);
                let inner = centre_of(inside_node);
                // the door sits mid-side (a door by a corner leaves a one-cell wall stub), the stub bends to the corridor
                let middle = rect.center();
                let doorway = if target.x != inner.x {
                    V2::new(if target.x < inner.x { rect.x } else { rect.x + rect.w - 1 }, middle.y)
                } else {
                    V2::new(middle.x, if target.y < inner.y { rect.y } else { rect.y + rect.h - 1 })
                };
                seal_except(grid, rect, doorway);
                carve_warren_lane(grid, &mut circ, doorway.add(target.sub(inner).sign()), target);
                mark_corridor_cell(&mut circ, doorway, 1);
                corridors.push(Corridor {
                    id: format!("{}:ROOM-BRANCH:{}", id, rooms.len()),
                    axis: "north_south",
                    width: CORRIDOR_WIDTH,
                    length: Some(1),
                    branch: Some(true),
                    from_main_hall: Some(false),
                    serves_many_rooms: Some(false),
                    doorway: Some(doorway),
                    ..Corridor::default()
                });
                let pick = room_types[rng.next_index(room_types.len())];
                let mut entry = room_entry(c, rooms.len() as i32, region, rect, pick);
                entry.doorway = Some(doorway);
                rooms.push(entry);
            }
        }
        Parts { rooms, corridors, suites: Vec::new(), circulation: circ }
    }
}

/// The generator context for one world: the Godot `StreamedRouteGenerator` queries that decide a
/// chunk's layout, with `world` the decimal world seed (`str(run_seed)`).
pub struct Gen<'a> {
    world: &'a str,
}

/// `is_chunk_allowed` (streamed_route_generator.gd:1436) with the mezzanine off: levels 0 to -3.
fn allowed(c: Coord) -> bool {
    c.y <= 0 && c.y >= -3
}

fn is_dense(region: &str) -> bool {
    DENSE_DISTRICTS.contains(&region)
}

impl<'a> Gen<'a> {
    /// `region_for` (line 1210) at the chunk's own level.
    fn region(&self, c: Coord) -> &'static str {
        regions::region_for(self.world, c.x, c.z, c.y)
    }

    /// `is_safe_room` (line 1403) on the private route (`public_route` false): a seeded chance scaled by
    /// the level's `safe_room_bias` (line 1290). The spawn chunk is always safe.
    fn is_safe_room(&self, c: Coord) -> bool {
        if c == Coord::new(0, 0, 0) {
            return true;
        }
        let mut rng = Rng::new(&format!("{}:SAFE:{}:{}:{}", self.world, c.x, c.y, c.z));
        rng.next_float() < super::danger::DangerProfile::for_level(c.y).safe_room_chance()
    }

    /// `is_pod_room_chunk` (line 2204): the lab's dive pod room, one chunk per lab district.
    fn is_pod_room_chunk(&self, c: Coord) -> bool {
        if c.y != 0 || self.region(c) != "lab" {
            return false;
        }
        let mut rng = Rng::new(&format!(
            "{}:PODROOM:{}:{}",
            self.world,
            c.x.div_euclid(4),
            c.z.div_euclid(4)
        ));
        if c.x.rem_euclid(4) as usize != rng.next_index(4) {
            return false;
        }
        c.z.rem_euclid(4) as usize == rng.next_index(4)
    }

    /// `is_warren_chunk` (line 1844): a maze chunk that is a warren, WARREN_SHARE of the time.
    fn is_warren(&self, c: Coord) -> bool {
        if c.y != 0
            || !WARREN_REGIONS.contains(&self.region(c))
            || self.is_safe_room(c)
            || self.is_pod_room_chunk(c)
        {
            return false;
        }
        Rng::new(&format!("{}:WARREN:{}", self.world, chunk_id(c))).next_float() < WARREN_SHARE
    }

    /// `_wing_roll` (line 1354): the music wing's own roll for a hotel chunk.
    fn wing_roll(&self, c: Coord) -> bool {
        if !allowed(c)
            || !MUSIC_WING_REGIONS.contains(&self.region(c))
            || self.is_safe_room(c)
            || self.is_warren(c)
        {
            return false;
        }
        Rng::new(&format!("{}:MUSICWING:{}", self.world, chunk_id(c))).next_float() < MUSIC_WING_CHANCE
    }

    /// `_base_large_plan` (line 1329): the ordinary great-hall slots, [W, E] ("" for none).
    fn base_large_plan(&self, c: Coord) -> [&'static str; 2] {
        let mut plan = ["", ""];
        let region = if allowed(c) { self.region(c) } else { "" };
        let Some(chance) = large_chance(region) else {
            return plan;
        };
        if self.is_safe_room(c) || self.is_warren(c) {
            return plan;
        }
        let mut rng = Rng::new(&format!("{}:LARGE:{}", self.world, chunk_id(c)));
        let types = large_types(region);
        for side in 0..2usize {
            let roll = rng.next_float();
            let type_index = rng.next_index(types.len());
            if roll >= chance {
                continue;
            }
            if region == "maze" && side == 1 {
                continue;
            }
            let neighbour = c.step(if side == 0 { -1 } else { 1 }, 0);
            if !allowed(neighbour) || !is_dense(self.region(neighbour)) {
                continue;
            }
            plan[side] = types[type_index];
        }
        plan
    }

    /// `large_plan` (line 1309): the great-hall slots of a chunk, with the music wing taking both
    /// slots when the wing rolls and both neighbours can give their slot.
    fn large_plan(&self, c: Coord) -> [&'static str; 2] {
        let mut plan = self.base_large_plan(c);
        if self.wing_roll(c) {
            let mut both = true;
            for dx in [-1, 1] {
                let next = c.step(dx, 0);
                both = both
                    && allowed(next)
                    && is_dense(self.region(next))
                    && !self.wing_roll(next)
                    && self.base_large_plan(next)[if dx < 0 { 1 } else { 0 }].is_empty();
            }
            if both {
                plan = [MUSIC_WING_LARGE_TYPE; 2];
            }
        }
        plan
    }

    /// `hall_trimmed` (line 1363): the middle east-west hall is cut short on this chunk's `dx` end.
    fn hall_trimmed(&self, c: Coord, dx: i32) -> bool {
        if !is_dense(self.region(c)) {
            return false;
        }
        let neighbour = c.step(dx, 0);
        let mine = self.large_plan(c)[if dx < 0 { 0 } else { 1 }];
        let mut theirs = "";
        if allowed(neighbour) && is_dense(self.region(neighbour)) {
            theirs = self.large_plan(neighbour)[if dx < 0 { 1 } else { 0 }];
        }
        !mine.is_empty() || !theirs.is_empty()
    }

    /// `edge_openings` (line 1374): the offsets this chunk's side opens onto its neighbour.
    fn edge_openings(&self, c: Coord, dx: i32, dz: i32) -> Vec<i32> {
        let neighbour = c.step(dx, dz);
        if !allowed(c) || !allowed(neighbour) {
            return Vec::new();
        }
        if is_dense(self.region(c)) && is_dense(self.region(neighbour)) && dx != 0 {
            let mut openings = vec![10, CENTER, 34];
            if self.hall_trimmed(c, dx) {
                openings.retain(|&o| o != CENTER);
            }
            return openings;
        }
        vec![CENTER]
    }

    /// `wide_seam` (line 1391): both sides maze.
    fn wide_seam(&self, c: Coord, dx: i32, dz: i32) -> bool {
        let neighbour = c.step(dx, dz);
        allowed(c) && allowed(neighbour) && self.region(c) == "maze" && self.region(neighbour) == "maze"
    }

    /// `_dense_linear_layout` (streamed_route_generator.gd:2042): the banks of a dense district, with
    /// the three halls, the rows of rooms, the great halls (and the music wing) and the maze office suite.
    fn dense(&self, c: Coord, region: &'static str, grid: &mut Grid) -> Parts {
        let id = chunk_id(c);
        let mut rooms: Vec<Room> = Vec::new();
        let mut corridors: Vec<Corridor> = Vec::new();
        let mut suites: Vec<Suite> = Vec::new();
        let mut circ = Circ::new();
        let mut rng = Rng::new(&format!("{}:BANKS:{}", self.world, id));
        let center = CENTER;
        // maze keeps its office suite in the south-east quarter: the banks leave that quarter alone
        let office_quarter = region == "maze";
        // the maze's two main halls are five wide
        let wide = region == "maze";
        let half = if wide { 2 } else { 1 };
        let plan = self.large_plan(c);
        let middle_start = if self.hall_trimmed(c, -1) { center - half } else { 1 };
        let middle_end = if self.hall_trimmed(c, 1) {
            if office_quarter {
                37
            } else {
                center + 1 + half
            }
        } else {
            CHUNK - 1
        };
        let mall = region == "mall";
        let main_hall = if mall || wide {
            Rect::new(center - 2, 1, 5, CHUNK - 2)
        } else {
            Rect::new(center - 1, 1, 3, CHUNK - 2)
        };
        let halls = [
            Rect::new(1, 9, CHUNK - 2, 3),
            Rect::new(middle_start, center - half, middle_end - middle_start, 1 + 2 * half),
            Rect::new(1, 33, if office_quarter { 22 } else { CHUNK - 2 }, 3),
            main_hall,
        ];
        for (hall_index, hall) in halls.iter().enumerate() {
            carve_rect(grid, *hall);
            mark_rect(&mut circ, *hall);
            corridors.push(Corridor {
                id: format!("{}:HALL:{}", id, hall_index),
                rect: Some(*hall),
                width: CORRIDOR_WIDTH,
                axis: if hall_index == 3 { "north_south" } else { "east_west" },
                serves_many_rooms: Some(true),
                ..Corridor::default()
            });
        }
        // rows of rooms: (top y, height, door on the top wall)
        let mut rows = [(0, 9, false), (12, 9, true), (24, 9, false), (36, 9, true)];
        if wide {
            // the five-wide middle hall (rows 20-24) takes a row from each middle bank
            rows[1].1 = 8;
            rows[2].0 = 25;
            rows[2].1 = 8;
        }
        let side_x: [i32; 2] = if mall || wide { [0, 25] } else { [0, 24] };
        // a bank side runs from the border to the centre hall: 21 cells beside a three-wide hall, 20 beside a five-wide one
        let bank_w = if wide { 20 } else { 21 };
        let big_types = bank_big_types(region);
        let choices = room_type_choices(region);
        let mut small_types: Vec<&'static str> = choices
            .iter()
            .copied()
            .filter(|t| !big_types.contains(t) && !(region == "carnival" && *t == "big_top"))
            .collect();
        if small_types.is_empty() {
            small_types = choices.clone();
        }
        let mut pod_pending = region == "lab" && self.is_pod_room_chunk(c);
        let mut room_index: i32 = 0;
        // a mall chunk holds eight shops: its cycle through the kinds starts at a per-chunk offset
        let type_offset = if mall { rng.next_index(small_types.len()) } else { 0 };
        // great halls first: two middle banks and the hall between them become one 21x21 room
        let mut merged = [false; 2];
        for side in 0..2usize {
            let hall_type = plan[side];
            if hall_type.is_empty() {
                continue;
            }
            merged[side] = true;
            let big = Rect::new(side_x[side], 12, if wide { bank_w } else { LARGE_ROOM_SIZE }, LARGE_ROOM_SIZE);
            carve_rect(grid, big);
            let big_door = V2::new(if side == 0 { big.end().x - 1 } else { big.x }, center);
            seal_except(grid, big, big_door);
            mark_corridor_cell(&mut circ, big_door, 1);
            corridors.push(Corridor {
                id: format!("{}:ROOM-BRANCH:{}", id, room_index),
                axis: "east_west",
                width: CORRIDOR_WIDTH,
                length: Some(1),
                branch: Some(true),
                from_main_hall: Some(true),
                serves_many_rooms: Some(false),
                doorway: Some(big_door),
                ..Corridor::default()
            });
            if hall_type == "big_top" {
                // the merry-go-round hall opens onto the vertical hall three cells wide
                for door_row in big_door.y - 1..big_door.y + 2 {
                    set(grid, big_door.x, door_row, 0);
                }
            }
            if hall_type == MUSIC_WING_LARGE_TYPE {
                let part = if side == 0 { "stage" } else { "rooms" };
                let wing = music_wing_carve(grid, big, big_door, part);
                for wing_cell in &wing.circulation {
                    mark_corridor_cell(&mut circ, *wing_cell, 1);
                }
                for wing_room in wing.rooms {
                    let mut entry = room_entry(c, room_index, region, wing_room.rect, wing_room.room_type);
                    entry.doorway = Some(wing_room.doorway);
                    entry.preset_room = true;
                    rooms.push(entry);
                    room_index += 1;
                }
                if wing.hall.has_area() {
                    // the wing hall is three wide, so its entrance is a three-wide opening
                    for door_row in big_door.y - 1..big_door.y + 2 {
                        set(grid, big_door.x, door_row, 0);
                    }
                    corridors.push(Corridor {
                        id: format!("{}:WING-HALL:{}", id, side),
                        rect: Some(wing.hall),
                        axis: "east_west",
                        width: CORRIDOR_WIDTH,
                        branch: Some(true),
                        serves_many_rooms: Some(true),
                        ..Corridor::default()
                    });
                }
                continue;
            }
            let mut big_entry = room_entry(c, room_index, region, big, hall_type);
            big_entry.doorway = Some(big_door);
            big_entry.great_hall = true;
            if hall_type == "big_top" {
                big_entry.extra_doorways = Some(vec![
                    V2::new(big_door.x, big_door.y - 1),
                    V2::new(big_door.x, big_door.y + 1),
                ]);
            }
            rooms.push(big_entry);
            room_index += 1;
        }
        for &(row_y, row_h, door_top) in rows.iter() {
            for side in 0..2usize {
                if office_quarter && side == 1 && row_y >= 24 {
                    continue;
                }
                if merged[side] && row_y > 0 && row_y < 36 {
                    continue;
                }
                // the maze's 8-deep banks cannot seat a restaurant's two rows of six
                let shallow = row_h < 9;
                let mut specs: Vec<(i32, i32, String)> = Vec::new();
                if pod_pending {
                    pod_pending = false;
                    specs.push((side_x[side], 21, "pod_room".to_string()));
                } else if mall {
                    specs.push((side_x[side], MALL_SHOP_WIDTH, String::new()));
                } else if !big_types.is_empty() && rng.next_float() < bank_big_chance(region) {
                    let mut big_type = big_types[rng.next_index(big_types.len())];
                    if shallow && big_type == "restaurant_dining" {
                        big_type = "storage";
                    }
                    specs.push((side_x[side], bank_w, big_type.to_string()));
                } else if wide {
                    // two rooms sharing a wall, the one nearer the centre hall a cell narrower
                    let outer = if side == 0 { 0 } else { 9 };
                    specs.push((side_x[side] + outer, 11, String::new()));
                    specs.push((side_x[side] + if side == 0 { 10 } else { 0 }, 10, String::new()));
                } else {
                    specs.push((side_x[side], 11, String::new()));
                    specs.push((side_x[side] + 10, 11, String::new()));
                }
                for (x, w, ty) in specs {
                    let rect = Rect::new(x, row_y, w, row_h);
                    carve_rect(grid, rect);
                    let doorway = V2::new(
                        rect.x + rect.w / 2,
                        if door_top { rect.y } else { rect.y + rect.h - 1 },
                    );
                    seal_except(grid, rect, doorway);
                    mark_corridor_cell(&mut circ, doorway, 1);
                    corridors.push(Corridor {
                        id: format!("{}:ROOM-BRANCH:{}", id, room_index),
                        axis: "north_south",
                        width: CORRIDOR_WIDTH,
                        length: Some(1),
                        branch: Some(true),
                        from_main_hall: Some(true),
                        serves_many_rooms: Some(false),
                        doorway: Some(doorway),
                        ..Corridor::default()
                    });
                    let room_type: String = if !ty.is_empty() {
                        ty
                    } else if region == "hotel" {
                        (if room_index % 4 == 3 { "hotel_bathroom" } else { "guest_room" }).to_string()
                    } else if region == "prison" {
                        (if room_index % 5 == 2 {
                            "prison_cafeteria"
                        } else if room_index % 5 == 4 {
                            "infirmary"
                        } else {
                            "prison_cell"
                        })
                        .to_string()
                    } else {
                        let n = small_types.len();
                        let mut t = small_types[(room_index as usize + type_offset) % n];
                        if shallow && t == "restaurant_dining" {
                            t = small_types[(room_index as usize + type_offset + 1) % n];
                        }
                        t.to_string()
                    };
                    let mut entry = room_entry(c, room_index, region, rect, &room_type);
                    entry.doorway = Some(doorway);
                    rooms.push(entry);
                    room_index += 1;
                }
            }
        }
        if office_quarter && self.edge_openings(c, 1, 0).contains(&34) {
            // the lower hall stops short of the east border here: a full three-wide bay for the seam
            carve_rect(grid, Rect::new(CHUNK - 3, 33, 3, 3));
            mark_rect(&mut circ, Rect::new(CHUNK - 3, 33, 3, 3));
        }
        if office_quarter {
            let suite = self.suite_cluster(c, region, grid, &mut circ, Rect::new(25, 25, 17, 19), 3, "office", &mut rng);
            rooms.extend(suite.rooms);
            corridors.extend(suite.corridors);
            suites.push(suite.suite);
        }
        Parts { rooms, corridors, suites, circulation: circ }
    }

    /// `_carve_suite_cluster` (streamed_route_generator.gd:2375): a suite in one 17x17 sector, the office
    /// (four rooms round a three-wide cross) or, through `_carve_apartment_suite`, an apartment.
    fn suite_cluster(
        &self,
        c: Coord,
        region: &'static str,
        grid: &mut Grid,
        circ: &mut Circ,
        rect: Rect,
        sector_index: i32,
        suite_kind: &str,
        rng: &mut Rng,
    ) -> SuiteOut {
        if suite_kind == "apartment" {
            return self.apartment_suite(c, region, grid, circ, rect, sector_index);
        }
        let suite_id = format!("{}:SUITE:{}:{}", chunk_id(c), sector_index, suite_kind.to_uppercase());
        carve_rect(grid, rect);
        let entrance = connect_room_to_main_hall(grid, circ, rect, sector_index, rng, "");
        let mut suite_corridors = vec![Corridor {
            id: format!("{}:ENTRY-HALL", suite_id),
            axis: entrance.axis,
            width: CORRIDOR_WIDTH,
            length: Some(entrance.length),
            branch: Some(true),
            from_main_hall: Some(true),
            serves_many_rooms: Some(true),
            doorway: Some(entrance.doorway),
            suite_id: Some(suite_id.clone()),
            ..Corridor::default()
        }];
        let mut parent = room_entry(c, sector_index, region, rect, &format!("{}_suite", suite_kind));
        parent.id = format!("{}:ENTRY", suite_id);
        parent.doorway = Some(entrance.doorway);
        parent.suite_id = Some(suite_id.clone());
        parent.suite_kind = Some(suite_kind.to_string());
        parent.suite_parent = true;
        parent.preset_room = true;
        let mut suite_rooms = vec![parent];
        // four rooms round a three-wide cross
        let room_w = (rect.w - 3) / 2;
        let room_h = (rect.h - 3) / 2;
        let far_x = room_w + 3;
        let far_z = room_h + 3;
        let hall_x = Rect::new(rect.x + room_w, rect.y + 1, 3, rect.h - 2);
        let hall_z = Rect::new(rect.x + 1, rect.y + room_h, rect.w - 2, 3);
        for hall in [hall_x, hall_z] {
            carve_rect(grid, hall);
            mark_rect(circ, hall);
        }
        // (rect, door on the west side)
        let child_specs = [
            (Rect::new(rect.x, rect.y, room_w, room_h), false),
            (Rect::new(rect.x + far_x, rect.y, room_w, room_h), true),
            (Rect::new(rect.x, rect.y + far_z, room_w, room_h), false),
            (Rect::new(rect.x + far_x, rect.y + far_z, room_w, room_h), true),
        ];
        let roles = ["office_reception", "cubicle_room", "office_job", "private_office"];
        let mut child_ids = Vec::new();
        for (ci, (crect, door_west)) in child_specs.iter().enumerate() {
            let mut doorway = V2::new(crect.x + crect.w - 1, crect.y + crect.h / 2);
            if *door_west {
                doorway.x = crect.x;
            }
            seal_except(grid, *crect, doorway);
            mark_corridor_cell(circ, doorway, 1);
            let role = roles[ci];
            let mut child = room_entry(c, ci as i32, region, *crect, role);
            child.id = format!("{}:ROOM:{}:{}", suite_id, ci, role.to_uppercase());
            child.doorway = Some(doorway);
            child.suite_id = Some(suite_id.clone());
            child.suite_kind = Some(suite_kind.to_string());
            child.suite_role = Some(role.to_string());
            child.preset_room = true;
            child_ids.push(child.id.clone());
            suite_rooms.push(child);
        }
        for (inner, axis) in [(hall_x, "north_south"), (hall_z, "east_west")] {
            suite_corridors.push(Corridor {
                id: format!("{}:INNER:{}", suite_id, axis),
                rect: Some(inner),
                axis,
                width: CORRIDOR_WIDTH,
                branch: Some(true),
                from_main_hall: Some(false),
                serves_many_rooms: Some(true),
                suite_id: Some(suite_id.clone()),
                ..Corridor::default()
            });
        }
        SuiteOut {
            rooms: suite_rooms,
            corridors: suite_corridors,
            suite: Suite {
                id: suite_id.clone(),
                kind: suite_kind.to_string(),
                region,
                rect,
                entry_doorway: entrance.doorway,
                parent_room_id: format!("{}:ENTRY", suite_id),
                room_ids: child_ids,
            },
        }
    }

    /// `_carve_apartment_suite` (streamed_route_generator.gd:2487): four rooms sharing walls, joined by
    /// single doors, in one 17x17 sector entered on its west wall (mirrored for sectors 0 and 2).
    fn apartment_suite(&self, c: Coord, region: &'static str, grid: &mut Grid, circ: &mut Circ, rect: Rect, sector_index: i32) -> SuiteOut {
        let suite_id = format!("{}:SUITE:{}:APARTMENT", chunk_id(c), sector_index);
        let flip = sector_index == 0 || sector_index == 2;
        let size = rect.w;
        let to_cell = |local: V2| -> V2 {
            let x = if flip { size - 1 - local.x } else { local.x };
            V2::new(rect.x + x, rect.y + local.y)
        };
        let to_rect = |local: Rect| -> Rect {
            let x = if flip { size - (local.x + local.w) } else { local.x };
            Rect::new(rect.x + x, rect.y + local.y, local.w, local.h)
        };
        carve_rect(grid, rect);
        let entry_cell = to_cell(V2::new(0, 12));
        let junction = V2::new(CENTER, entry_cell.y);
        carve_corridor_segment(grid, circ, entry_cell, junction);
        // (role, local rect, local doorway)
        let plan: [(&str, Rect, V2); 4] = [
            ("bedroom", Rect::new(0, 0, 9, 9), V2::new(4, 8)),
            ("house_bathroom", Rect::new(8, 0, 9, 9), V2::new(8, 4)),
            ("living_room", Rect::new(0, 8, 11, 9), V2::new(0, 12)),
            ("kitchen", Rect::new(10, 8, 7, 9), V2::new(10, 15)),
        ];
        for (_, local, _) in plan.iter() {
            seal_except(grid, to_rect(*local), entry_cell);
            set(grid, entry_cell.x, entry_cell.y, 1);
        }
        let door_cells = [V2::new(4, 8), V2::new(8, 4), V2::new(10, 15), V2::new(0, 12)];
        for door in door_cells {
            let world_door = to_cell(door);
            set(grid, world_door.x, world_door.y, 0);
            mark_corridor_cell(circ, world_door, 1);
        }
        let mut parent = room_entry(c, sector_index, region, rect, "apartment_suite");
        parent.id = format!("{}:ENTRY", suite_id);
        parent.doorway = Some(entry_cell);
        parent.suite_id = Some(suite_id.clone());
        parent.suite_kind = Some("apartment".to_string());
        parent.suite_parent = true;
        parent.preset_room = true;
        let mut suite_rooms = vec![parent.clone()];
        let mut child_ids = Vec::new();
        for (child_index, (role, local, door)) in plan.iter().enumerate() {
            let world_rect = to_rect(*local);
            let mut child = room_entry(c, child_index as i32, region, world_rect, role);
            child.id = format!("{}:ROOM:{}:{}", suite_id, child_index, role.to_uppercase());
            let doorway = to_cell(*door);
            child.doorway = Some(doorway);
            let mut extra = Vec::new();
            for d in door_cells {
                let world_door = to_cell(d);
                if world_rect.has_point(world_door) && world_door != doorway {
                    extra.push(world_door);
                }
            }
            child.extra_doorways = Some(extra);
            child.suite_id = Some(suite_id.clone());
            child.suite_kind = Some("apartment".to_string());
            child.suite_role = Some(role.to_string());
            child.preset_room = true;
            child_ids.push(child.id.clone());
            suite_rooms.push(child);
        }
        SuiteOut {
            rooms: suite_rooms,
            corridors: vec![Corridor {
                id: format!("{}:ENTRY-HALL", suite_id),
                axis: "east_west",
                width: CORRIDOR_WIDTH,
                length: Some((entry_cell.x - junction.x).abs()),
                branch: Some(true),
                from_main_hall: Some(true),
                serves_many_rooms: Some(true),
                doorway: Some(entry_cell),
                suite_id: Some(suite_id.clone()),
                ..Corridor::default()
            }],
            suite: Suite {
                id: suite_id.clone(),
                kind: "apartment".to_string(),
                region,
                rect,
                entry_doorway: entry_cell,
                parent_room_id: format!("{}:ENTRY", suite_id),
                room_ids: child_ids,
            },
        }
    }

    /// `_mixed_room_layout` (streamed_route_generator.gd:2230) for the apartment district: a three-wide
    /// cross and four apartment suites, one per sector (RNG-free). The other regions of this path are
    /// never reached on level 0 (they are dense).
    fn mixed_apartment(&self, c: Coord, region: &'static str, grid: &mut Grid) -> Parts {
        let id = chunk_id(c);
        let mut rng = Rng::new(&format!("{}:ROOMS:{}", self.world, id));
        let mut rooms: Vec<Room> = Vec::new();
        let mut corridors: Vec<Corridor> = Vec::new();
        let mut suites: Vec<Suite> = Vec::new();
        let horizontal = Rect::new(1, CENTER - 1, CHUNK - 2, 3);
        let vertical = Rect::new(CENTER - 1, 1, 3, CHUNK - 2);
        let mut circ = Circ::new();
        for hall in [horizontal, vertical] {
            carve_rect(grid, hall);
            mark_rect(&mut circ, hall);
        }
        corridors.push(Corridor {
            id: format!("{}:HALL:MAIN-X", id),
            rect: Some(horizontal),
            width: CORRIDOR_WIDTH,
            axis: "east_west",
            serves_many_rooms: Some(true),
            ..Corridor::default()
        });
        corridors.push(Corridor {
            id: format!("{}:HALL:MAIN-Z", id),
            rect: Some(vertical),
            width: CORRIDOR_WIDTH,
            axis: "north_south",
            serves_many_rooms: Some(true),
            ..Corridor::default()
        });
        let sectors = [
            Rect::new(4, 4, 17, 17),
            Rect::new(24, 4, 17, 17),
            Rect::new(4, 24, 17, 17),
            Rect::new(24, 24, 17, 17),
        ];
        for (index, sector) in sectors.iter().enumerate() {
            let suite = self.suite_cluster(c, region, grid, &mut circ, *sector, index as i32, "apartment", &mut rng);
            rooms.extend(suite.rooms);
            corridors.extend(suite.corridors);
            suites.push(suite.suite);
        }
        Parts { rooms, corridors, suites, circulation: circ }
    }
}

/// The layout of one chunk of the Infinite route at level 0 (the maze). `None` for the levels this
/// port does not lay out: the sewer levels -1 and -2 (SewerRouteGenerator), the Inferno -3 (no rooms),
/// and the mezzanine +1 (removed from play).
///
/// This is `_build_chunk` (streamed_route_generator.gd:916) for the room planning, in its order:
/// `_region_layout` (934), the seam carve `_carve_edges` (978), the level-0 stairwell step (984-1000).
/// Stairs are always empty on this route (stair_links, line 1399).
pub fn chunk_layout(world: &str, c: Coord) -> Option<ChunkLayout> {
    if c.y != 0 {
        return None;
    }
    let gen = Gen { world };
    let region = gen.region(c);
    let mut grid = solid_grid();
    let mut parts = if gen.is_warren(c) {
        gen.warren(c, &mut grid, region)
    } else if is_dense(region) {
        gen.dense(c, region, &mut grid)
    } else if region == "apartment" {
        gen.mixed_apartment(c, region, &mut grid)
    } else {
        return None;
    };
    // (dx, dz) of west, east, north, south: `edges` in _build_chunk (lines 957-962)
    let dirs = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    let edges: [Vec<i32>; 4] = std::array::from_fn(|i| gen.edge_openings(c, dirs[i].0, dirs[i].1));
    let wide: [bool; 4] = std::array::from_fn(|i| gen.wide_seam(c, dirs[i].0, dirs[i].1));
    let edge_corridors = carve_edges(&mut grid, &mut parts.circulation, &edges, is_dense(region), &wide);
    parts.corridors.extend(edge_corridors);
    // the level-0 stairwell (_build_chunk 984): one well down to the sewer, or none
    let mut stairwells = Vec::new();
    if let Some(down) = plan_maze_stairwell(world, c.x, c.z, &grid, &parts) {
        stairwells.push(down);
    }
    for well in &stairwells {
        carve_stairwell(&mut grid, &mut parts.circulation, &mut parts.rooms, well);
    }
    // _build_chunk 1003-1007 (grid.rs): the thresholds, the safe room and the door planner, which
    // writes the grid that the furniture reads (stage 3a).
    let safe = gen.is_safe_room(c);
    let doors = grid::level0_before_props(&mut grid, &parts.rooms, region, safe);
    // 1007: _region_props (props.rs, stage 3b-ii)
    let region_props = props::region_props(world, c, region, &parts.rooms, &grid, &doors, safe, &parts.circulation);
    // 1016-1022 (grid.rs): the second restore, the authored doorways and the hump pass
    grid::level0_after_props(&mut grid, &parts.rooms, &doors);
    // 1024-1028 (props.rs, stage 3b-iv): push_out, scale_surface_mounts and PlacementClearance.clear, on the grid
    // after the hump pass
    let region_props = region_props.map(|props| props::post_passes(&grid, &parts.rooms, props));
    // 1029-1031 (fixtures.rs, stage 3c-i): _fixtures, then _prop_hiding_fixtures on the props after the post passes.
    // Stairs are empty on level 0 (stair_links, 1399) and the sealed wing is level -3 only (1032-1037).
    let fixtures = fixtures::chunk_fixtures(
        world,
        c,
        &grid,
        &parts.rooms,
        &[],
        region_props.as_deref().unwrap_or(&[]),
        safe,
    );
    // 1038 (pickups.rs, stage 3c-ii): _pickups on the same final grid, props and fixtures; the sealed set is empty on
    // level 0, so no sealed cells are passed (1032-1037 skip the sealed wing).
    let props_now: &[props::Prop] = region_props.as_deref().unwrap_or(&[]);
    let mut pickups = pickups::chunk_pickups(world, c, &grid, region, props_now, &fixtures, &[], safe);
    // 1039 (music_boxes.rs, stage 3c-iii): _music_boxes reads the fixtures and the pickups before the ensure pass,
    // so it runs here, on the pickups as _pickups placed them.
    let music = music_boxes::chunk_music_boxes(world, c, &grid, props_now, &fixtures, &pickups, &parts.rooms, safe);
    // 1040 (hunter_spawns.rs, stage 3c-iii): _hunter_spawns on the region props after the post passes. Stairs are
    // empty on level 0.
    let hunters = hunter_spawns::chunk_hunter_spawns(world, c, &grid, &[], props_now, safe);
    // 809 (pickups.rs): _ensure_pickups_reachable runs last, after the music boxes and hunters in Godot.
    pickups::ensure_pickups_reachable(&grid, props_now, &mut pickups);
    Some(ChunkLayout {
        coord: c,
        region,
        rooms: parts.rooms,
        corridors: parts.corridors,
        suites: parts.suites,
        edges,
        stairwells,
        grid,
        circulation: parts.circulation,
        region_props,
        fixtures,
        pickups,
        music_boxes: music,
        hunter_spawns: hunters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn load_golden(tag: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("parity")
            .join(format!("layout-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("layout golden json")
    }

    fn coord_of(v: &Value) -> Coord {
        let a = v["coord"].as_array().expect("coord");
        Coord::new(
            a[0].as_i64().unwrap() as i32,
            a[1].as_i64().unwrap() as i32,
            a[2].as_i64().unwrap() as i32,
        )
    }

    fn cut(v: &Value) -> String {
        let s = v.to_string();
        if s.len() > 160 {
            format!("{}...", &s[..160])
        } else {
            s
        }
    }

    /// The first place two JSON values differ: the path, and both values cut short.
    fn first_difference(got: &Value, want: &Value, path: &str) -> String {
        match (got, want) {
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return format!("{path}: {} entries, Godot has {}", a.len(), b.len());
                }
                for (i, (x, y)) in a.iter().zip(b).enumerate() {
                    if x != y {
                        return first_difference(x, y, &format!("{path}[{i}]"));
                    }
                }
                format!("{path}: equal")
            }
            (Value::Object(a), Value::Object(b)) => {
                for key in b.keys().chain(a.keys()) {
                    match (a.get(key), b.get(key)) {
                        (Some(x), Some(y)) if x != y => return first_difference(x, y, &format!("{path}.{key}")),
                        (Some(_), None) => return format!("{path}.{key}: only in ours"),
                        (None, Some(_)) => return format!("{path}.{key}: only in Godot"),
                        _ => {}
                    }
                }
                format!("{path}: equal")
            }
            _ => format!("{path}: ours {} != Godot {}", cut(got), cut(want)),
        }
    }

    #[test]
    fn profile_table_bounds() {
        assert_eq!(profile_min_max("guest_room"), Some(((9, 9), (13, 11))));
        assert_eq!(profile_min_max("prison_cafeteria"), Some(((11, 9), (21, 21))));
        assert_eq!(profile_min_max("no_such_room"), None);
        // a 12 x 12 corner: only the rooms that reach 12 on both sides
        assert!(within("prison_cafeteria", (12, 12)));
        assert!(!within("prison_cell", (12, 12)));
        assert!(within("no_such_room", (12, 12)));
    }

    #[test]
    fn district_lists_match_the_data_files() {
        let hospital: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/regions/hospital.json"))).unwrap();
        let mall: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/regions/mall.json"))).unwrap();
        let names = |v: &Value| -> Vec<String> {
            v["room_roles"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect()
        };
        assert_eq!(names(&hospital), HOSPITAL_ROOMS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(names(&mall), MALL_ROOMS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }

    #[test]
    fn rect_semantics_match_godot() {
        // Rect2i.intersects is end-exclusive and counts a zero-height rect strictly inside the other
        let a = Rect::new(0, 0, 5, 5);
        assert!(!a.intersects(Rect::new(5, 0, 2, 2)));
        assert!(a.intersects(Rect::new(4, 4, 2, 2)));
        assert!(a.intersects(Rect::new(1, 2, 3, 0)));
        assert!(!a.intersects(Rect::new(1, 5, 3, 0)));
        assert_eq!(Rect::new(1, 9, 3, 0).merge(Rect::new(1, 9, 3, 1)), Rect::new(1, 9, 3, 1));
        assert_eq!(a.intersection(Rect::new(3, 3, 9, 9)), Rect::new(3, 3, 2, 2));
        assert!(!a.has_point(V2::new(5, 0)));
        assert_eq!(Rect::new(0, 0, 17, 17).center(), V2::new(8, 8));
    }

    /// Every level-0 chunk of the three Godot goldens is compared field by field (rooms, corridors,
    /// suites, edges, stairs). Levels -1 and -2 (sewer) and -3 (Inferno, no rooms) are skipped and
    /// counted; they are not matched.
    #[test]
    fn layout_matches_godot_goldens() {
        let mut failures: Vec<String> = Vec::new();
        let mut report = String::new();
        for tag in ["parity", "k7qxm", "demo"] {
            let golden = load_golden(tag);
            let world = golden["world_seed"].as_str().expect("world_seed").to_string();
            // level -> (matched, mismatched, skipped)
            let mut per_level: BTreeMap<i32, (usize, usize, usize)> = BTreeMap::new();
            let mut skipped_regions: BTreeMap<String, usize> = BTreeMap::new();
            let mut region_matched: BTreeMap<String, usize> = BTreeMap::new();
            let mut features: BTreeMap<&str, usize> = BTreeMap::new();
            let mut totals = (0usize, 0usize, 0usize);
            // (chunks where Godot or we carved a well, chunks where the boxes agree)
            let mut wells = (0usize, 0usize);
            for chunk in golden["chunks"].as_array().expect("chunks") {
                let c = coord_of(chunk);
                let entry = per_level.entry(c.y).or_default();
                if c.y != 0 {
                    entry.2 += 1;
                    *skipped_regions
                        .entry(format!("{} (level {})", chunk["region"].as_str().unwrap_or("?"), c.y))
                        .or_default() += 1;
                    continue;
                }
                let Some(layout) = chunk_layout(&world, c) else {
                    entry.1 += 1;
                    failures.push(format!("{tag} {:?}: no layout for region {}", (c.x, c.y, c.z), chunk["region"]));
                    continue;
                };
                let got = layout.to_json();
                let mut bad: Vec<String> = Vec::new();
                for key in ["rooms", "corridors", "suites", "edges", "stairs"] {
                    if got[key] != chunk[key] {
                        bad.push(first_difference(&got[key], &chunk[key], key));
                    }
                }
                let ours: Vec<Value> = layout.stairwells.iter().map(|w| w.bbox.json()).collect();
                let theirs: Vec<Value> = chunk["stairwell_boxes"].as_array().cloned().unwrap_or_default();
                if !theirs.is_empty() || !ours.is_empty() {
                    wells.0 += 1;
                    if ours == theirs {
                        wells.1 += 1;
                    }
                }
                if bad.is_empty() {
                    entry.0 += 1;
                    let branch = if (Gen { world: &world }).is_warren(c) { "warren" } else if is_dense(layout.region) { "banks" } else { "apartment" };
                    *region_matched.entry(format!("{}/{}", layout.region, branch)).or_default() += 1;
                    totals.0 += chunk["rooms"].as_array().map_or(0, |a| a.len());
                    totals.1 += chunk["corridors"].as_array().map_or(0, |a| a.len());
                    totals.2 += chunk["suites"].as_array().map_or(0, |a| a.len());
                    for room in chunk["rooms"].as_array().into_iter().flatten() {
                        if room["great_hall"] == json!(true) { *features.entry("great-hall rooms").or_default() += 1; }
                        if room.get("suite_id").is_some() { *features.entry("suite rooms").or_default() += 1; }
                        if room.get("extra_doorways").is_some() { *features.entry("rooms with extra doorways").or_default() += 1; }
                    }
                    for corridor in chunk["corridors"].as_array().into_iter().flatten() {
                        let id = corridor["id"].as_str().unwrap_or("");
                        for (tag, label) in [(":WARREN-JOIN:", "warren joins"), (":PLAZA:", "warren plazas"), (":WING-HALL:", "music-wing halls"), (":INNER:", "suite inner halls"), (":ENTRY-HALL", "suite entry halls"), ("EDGE:", "edge lanes")] {
                            if id.contains(tag) { *features.entry(label).or_default() += 1; }
                        }
                    }
                } else {
                    entry.1 += 1;
                    failures.push(format!("{tag} {:?} {}: {}", (c.x, c.y, c.z), layout.region, bad.join(" | ")));
                }
            }
            report.push_str(&format!("seed {tag} (world {world}):\n"));
            for (level, (ok, bad, skipped)) in &per_level {
                report.push_str(&format!("  level {level:>2}: matched {ok}, mismatched {bad}, skipped {skipped}\n"));
            }
            report.push_str(&format!("  level-0 matched by region/branch: {region_matched:?}
"));
            report.push_str(&format!("  compared in matched chunks: {} rooms, {} corridors, {} suites
", totals.0, totals.1, totals.2));
            report.push_str(&format!("  features in matched chunks: {features:?}\n"));
            report.push_str(&format!("  skipped (not laid out here): {skipped_regions:?}\n"));
            report.push_str(&format!(
                "  stairwells: {} chunks with a well on either side, {} with the same box\n",
                wells.0, wells.1
            ));
        }
        eprintln!("{report}");
        for line in failures.iter().take(20) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} level-0 chunks differ from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }
}
