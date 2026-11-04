//! The finite floor generator, ported draw-for-draw from
//! game/world/generation/facility_generator.gd: a 3x3 sector grid around an 11x11 lobby,
//! catalogue room types, a Kruskal graph of three-wide halls with up to four loops, seeded
//! room props, then the service lift, safe zone, relays, batteries and hiding spots. The same
//! seed text gives the same floor as the original (Godot's float32 Vector2 maths and its
//! unstable sort are reproduced where they decide placement).

use super::catalog::Catalog;
use super::flight::{flight_surface_y, flights, lane_dividers, walk_path as flight_walk_path, well_footprint, Flight, FlightKind, FlightShape, Footprint, Segment};
use super::godot_sort::sort_custom;
use super::vertical::STOREY;
use super::infinite::ChunkKey;
use super::route::catwalk_links::{self, CatwalkLink};
use super::route::{catwalk, maze_regions};
use crate::content::regions::RegionDef;
use super::rng::Rng;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

pub const HALL_WIDTH: i32 = 3;
pub const LOBBY_SIZE: i32 = 11;
pub const SECTOR_GAP: i32 = 2;
pub const SECTOR_MARGIN: i32 = 2;
pub const MIN_AUTHORED_ROOM_SIZE: i32 = 10;
pub const MAX_AUTHORED_ROOM_SIZE: i32 = 26;

/// The room side range (min, max) in cells for a region: its `geometry` block's `room_min` and `room_max`, or
/// the game-wide `MIN_AUTHORED_ROOM_SIZE` and `MAX_AUTHORED_ROOM_SIZE` when the region has no block (or no region).
pub fn room_sizes_for(def: Option<&RegionDef>) -> (i32, i32) {
    match def.and_then(|d| d.geometry) {
        Some(g) => (g.room_min, g.room_max),
        None => (MIN_AUTHORED_ROOM_SIZE, MAX_AUTHORED_ROOM_SIZE),
    }
}
pub const ROOM_SECTOR_FILL: f64 = 0.86;
pub const DEFAULT_AGENT_RADIUS: f64 = 0.23;
/// Half extents of a hiding locker's solid body (estimated from the locker model).
pub const LOCKER_HALF: f32 = 0.34;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

impl Cell {
    pub const fn new(x: i32, y: i32) -> Cell {
        Cell { x, y }
    }
    pub fn center(self) -> (f32, f32) {
        (self.x as f32 + 0.5, self.y as f32 + 0.5)
    }
    /// Vector2(a - b).length() in float32, as the original measures cell spacing.
    pub fn len_to(self, o: Cell) -> f32 {
        let dx = (self.x - o.x) as f32;
        let dy = (self.y - o.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }
}

#[derive(Clone, Debug)]
pub struct Room {
    pub id: String,
    pub sector: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub center: Cell,
    pub rtype: String,
    pub label: String,
    pub region: String,
    pub landmark: bool,
    pub prop_types: Vec<String>,
}

impl Room {
    pub fn lobby(&self) -> bool {
        self.id == "room-lobby"
    }
}

#[derive(Clone, Debug)]
pub struct Prop {
    pub id: String,
    pub cell: Cell,
    pub ptype: String,
    pub room: usize,
    pub rotation: i32,
    /// Height of the parent surface for desk/table-top props.
    pub mount_y: Option<f64>,
    pub solid: bool,
    /// Rotated half extents (f32, as Godot's Vector2).
    pub half: (f32, f32),
    /// The prop's centre is `cell + .5 + offset` (Godot's `placement_offset`: a wall-backed bed is pushed
    /// flush to its wall). The drawn model and the footprint both use it, as Godot's `_spawn_authored_prop`
    /// and `CollisionWorld` do. Zero for a prop with no offset.
    pub offset: (f32, f32),
    /// A room door's doorway (draw only: the span and the centre offset from `cell`); `None` for other props.
    pub door: Option<DoorDraw>,
}

/// How a room door is drawn: the doorway's width in cells (`door_span`) and the offset of the doorway's centre
/// from the threshold `cell`, which is `placement_offset` for a double door (door.rs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorDraw {
    pub span: i32,
    pub offset: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Hiding {
    pub cell: Cell,
    /// From the backing wall into the room: the locker's front.
    pub facing: (i32, i32),
    pub room: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct RoomLight {
    pub room: usize,
    pub center: Cell,
    pub switch_cell: Cell,
    pub color: (f32, f32, f32),
    pub range: f32,
    pub energy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pickup {
    Taser,
    Lockpick,
    Crowbar,
    Axe,
    Battery,
    Stim,
    ReviveKit,
    BackpackSmall,
    BackpackMedium,
    BackpackLarge,
}

/// PlayerController3D.COLLECTIBLE_ITEMS minus the starting flashlight, in order.
pub const PICKUP_ORDER: [Pickup; 10] = [
    Pickup::Taser,
    Pickup::Lockpick,
    Pickup::Crowbar,
    Pickup::Axe,
    Pickup::Battery,
    Pickup::Stim,
    Pickup::ReviveKit,
    Pickup::BackpackSmall,
    Pickup::BackpackMedium,
    Pickup::BackpackLarge,
];

/// What a cell's walk surface is: the ground floor, or a catwalk deck raised over it (Godot's Mezzanine decks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeckKind {
    #[default]
    Floor,
    Catwalk,
}

/// The walk surface of one cell: its kind and its height above the level's floor, in metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WalkSurface {
    pub kind: DeckKind,
    pub height: f32,
}

#[derive(Clone)]
pub struct Facility {
    pub seed_text: String,
    pub cols: i32,
    pub rows: i32,
    /// grid[y][x]: true = wall
    pub wall: Vec<Vec<bool>>,
    pub rooms: Vec<Room>,
    pub props: Vec<Prop>,
    pub start: Cell,
    pub exit: Cell,
    pub safe_zone: Option<Cell>,
    pub relays: Vec<Cell>,
    pub batteries: Vec<Cell>,
    pub hiding: Vec<Hiding>,
    pub pickups: Vec<(Cell, Pickup)>,
    pub room_lights: Vec<RoomLight>,
    pub hunter_spawn: Cell,
    /// Whether this floor has the service lift (the fixed route); Infinite moves by stairs.
    pub has_lift: bool,
    /// Stair links: (top cell of the flight, -1 down / +1 up, direction the flight runs).
    pub stairs: Vec<Stair>,
    /// Region of each cell's chunk (Infinite districts); halls take their textures from it.
    pub cell_region: Vec<u8>,
    pub region_names: Vec<String>,
    /// Absolute cell of this grid's (0, 0) (the Infinite window's top-left; 0 on a finite floor),
    /// so surface picks stay put when the window recentres.
    pub origin: (i32, i32),
    /// Stairwell cells: open to the level below (or above), not walkable.
    hole: Vec<bool>,
    hunter_open: Vec<bool>,
    random: Rng,
    /// Solid props by cell, for fast footprint tests.
    prop_grid: Vec<Vec<usize>>,
    /// The swing of each room door, keyed by its threshold cell. A door with no entry has not been ticked yet
    /// and counts as open, so a floor nobody calls `update_doors` on stays walkable. Nothing here opens a door
    /// for the runner or a player bot (NPC teammate): only `toggle_door` (the interact press) does; hunters pull the doors they reach.
    door_state: HashMap<Cell, DoorState>,
    /// The walk surface of each cell that is not plain ground floor; a cell with no entry is `Floor` at 0.0 m.
    walk_surface: HashMap<Cell, WalkSurface>,
    /// The stairs and ladders up to this chunk's catwalk decks (`apply_catwalks`); empty on a chunk with none.
    pub catwalk_links: Vec<CatwalkLink>,
    /// The id of the level 0 maze variant that dresses this chunk (`apply_maze_variant`); `None` is the plain maze.
    pub maze_variant: Option<String>,
    /// The level this facility is (0 ground, -1 sewer); its floor sits at `level * STOREY` in absolute y
    /// (`body_fits_at_y`). Set by the game when it loads a window; 0 until then.
    pub level: i32,
    /// Waterfall voids: cells with no floor at all (open air down to the level below), but no wall either. Sorted;
    /// set by `set_drop_cells`, indexed into `drop` by `rebuild_grids`.
    drop_list: Vec<Cell>,
    /// Cells with no ceiling because a drop on the level above falls through them (`set_air_above_cells`). Sorted.
    air_list: Vec<Cell>,
    drop: Vec<bool>,
    air: Vec<bool>,
}

/// How far a door is open (0 closed, 1 open) and whether it is meant to be open. `open` chases `want_open` at
/// `DOOR_SWING_RATE`, except that a leaf never swings shut with a body in its sweep (`update_doors`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorState {
    pub open: f32,
    pub want_open: bool,
    /// The doorway's width in metres (its span in cells): the leaf's length.
    span: f32,
    centre: (f32, f32),
    half: (f32, f32),
}

/// A hunter this close to a door's edge pulls it open and leaves it open (the Godot hunter's
/// `_operate_hunter_room_door` reaches 1.15 m from the door's centre; a wider doorway adds half its span).
pub const DOOR_OPENER_REACH: f32 = 1.15;
/// Open amount gained or lost per second.
pub const DOOR_SWING_RATE: f32 = 4.0;
/// Half the thickness of a closed door's blocking slab, in metres.
const DOOR_HALF_DEPTH: f32 = 0.1;
/// How far (metres) a body's walk height may be from a flight's surface for it to stand on the flight.
const FLIGHT_FIT_TOL: f32 = 0.6;
/// A door passes a body once the gap the swung leaf leaves is this much wider than the body.
const DOOR_GAP_MARGIN: f32 = 0.04;
/// How far past the door's slab, on every side, a body stands in the leaf's sweep: the leaf's clearance plus a body
/// radius. A leaf does not swing shut while any body is that near.
const DOOR_SWEEP: f32 = 0.45;

/// What pressing interact on a door did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorToggle {
    Opened,
    Closed,
    /// A body stands in the leaf's sweep, so the door would not close.
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stair {
    /// dir < 0 (lives on the UPPER level): the landing cell A where the runner steps on. dir > 0 (lives on the LOWER
    /// level): the foot cell B where the flight ends. A ladder: the hatch.
    pub cell: Cell,
    /// -1 leads down a level, +1 up.
    pub dir: i32,
    /// The flight's own run (A toward the well), the same on both levels. A ladder: the unit step from the hatch to the
    /// one clear step cell beside it, the same on both levels (`flight_links::ladder_flight`).
    pub run: (i32, i32),
    pub kind: LinkKind,
    /// How the flight is laid out (a ladder: Switchback, unused).
    pub shape: FlightShape,
    /// Spiral only: which way it turns.
    pub clockwise: bool,
}

impl Stair {
    pub fn is_ladder(&self) -> bool {
        matches!(self.kind, LinkKind::Ladder(_))
    }

    /// A stair record of a walked flight; `cell` is A for dir < 0 and B for dir > 0.
    pub fn stairs(cell: Cell, dir: i32, run: (i32, i32), shape: FlightShape, clockwise: bool) -> Stair {
        Stair { cell, dir, run, kind: LinkKind::Stairs, shape, clockwise }
    }

    /// A ladder record: `cell` is the hatch, stepping off to the north (`(0, -1)`). Use `ladder_with_run` for the
    /// step direction `flight_links::ladder_flight` picked.
    pub fn ladder(cell: Cell, dir: i32, cover: Cover) -> Stair {
        Stair::ladder_with_run(cell, dir, cover, (0, -1))
    }

    /// A ladder record: `cell` is the hatch and `run` the unit step to the one clear step cell beside it.
    pub fn ladder_with_run(cell: Cell, dir: i32, cover: Cover, run: (i32, i32)) -> Stair {
        Stair {
            cell,
            dir,
            run,
            kind: LinkKind::Ladder(cover),
            shape: FlightShape::Switchback,
            clockwise: false,
        }
    }

    /// The flight this record is one end of, seen from `level`, the level of the facility holding it. A ladder is a
    /// `FlightKind::Ladder` flight: the hatch is its anchor and `run` leads to the step cell.
    pub fn flight(&self, level: i32) -> Option<Flight> {
        let kind = if self.is_ladder() { FlightKind::Ladder } else { FlightKind::Stairs };
        let run = if self.is_ladder() && self.run == (0, 0) { (0, -1) } else { self.run };
        Some(if self.dir < 0 {
            Flight::new(kind, self.shape, level, self.cell, run).with_clockwise(self.clockwise)
        } else {
            Flight::from_foot_with(flights(), kind, self.shape, level, self.cell, run, self.clockwise)
        })
    }

    /// The footprint of a stair (the cells do not depend on the level number).
    fn footprint(&self) -> Option<Footprint> {
        self.flight(if self.dir < 0 { 0 } else { -1 }).map(|f| well_footprint(&f))
    }

    /// The cells that are a hole on the upper level and open ceiling on the lower. A ladder: its one hatch cell, the
    /// same from both records (the lower level's foot is an ordinary floor cell under it, see `rebuild_grids`).
    pub fn well_cells(&self) -> Vec<Cell> {
        self.footprint().map_or_else(|| vec![self.cell], |fp| fp.upper_hole)
    }

    /// The walls between the lanes of this flight (grid coordinates; the same list from the upper and the lower record,
    /// `level` is the level of the facility holding this record). Empty for straight and spiral flights and ladders.
    pub fn dividers(&self, level: i32) -> Vec<Segment> {
        self.flight(level).map(|f| lane_dividers(&f)).unwrap_or_default()
    }

    /// The landing A (upper level). A ladder: its step cell, the floor cell beside the hatch.
    pub fn landing(&self) -> Cell {
        self.footprint().map_or(self.cell, |fp| fp.landing)
    }

    /// The foot B (lower level). A ladder: its hatch cell (the floor under the shaft).
    pub fn foot(&self) -> Cell {
        self.footprint().map_or(self.cell, |fp| fp.foot)
    }

    /// A ladder's one clear step cell beside the hatch (the same cell as `landing()`); for a stair, its landing A.
    pub fn step_cell(&self) -> Cell {
        self.landing()
    }

    /// The side of a ladder's cell its rails stand on: opposite the step cell, so a climber steps off away from the
    /// rails. `(0, 0)` for a stair.
    pub fn rails_side(&self) -> (i32, i32) {
        if self.is_ladder() {
            let run = if self.run == (0, 0) { (0, -1) } else { self.run };
            (-run.0, -run.1)
        } else {
            (0, 0)
        }
    }

    /// The end of the flight on the other level: the foot for an upper stair, the landing for a lower one.
    pub fn far_end(&self) -> Cell {
        if self.dir < 0 {
            self.foot()
        } else {
            self.landing()
        }
    }

    /// Every cell the flight needs kept clear: the well, both ends and their margins (sorted, no repeats).
    pub fn carve_cells(&self) -> Vec<Cell> {
        let Some(fp) = self.footprint() else { return vec![self.cell] };
        let mut v = fp.upper_hole;
        v.push(fp.landing);
        v.push(fp.foot);
        v.extend(fp.upper_clear);
        v.extend(fp.lower_clear);
        v.sort();
        v.dedup();
        v
    }

    /// The walking height at `(x, z)` above the floor of `level`, the level of the facility holding this record: the
    /// absolute surface minus `level * STOREY`, so it is continuous across the level change. `Some` only over the
    /// well and this stair's own end (A for dir < 0, B for dir > 0), never on the other end.
    pub fn surface_rel(&self, level: i32, x: f32, z: f32) -> Option<f32> {
        // a ladder is climbed, not walked: it has no tread surface
        if self.is_ladder() {
            return None;
        }
        let flight = self.flight(level)?;
        let fp = well_footprint(&flight);
        // The cell holding the point, by the flight's own frame (each cell is half-open towards the well: u in
        // [k-1, k)), so a point exactly on a cell edge belongs to the same cell whichever way the run points. Floor of
        // the world coordinate would put it in the far-end cell for a negative run and drop the walker off the flight.
        let (rx, rz) = flight.run;
        let (sx, sz) = (rz.abs(), rx.abs());
        let (ex, ez) = (
            flight.anchor_cell.x as f32 + (rx > 0) as i32 as f32,
            flight.anchor_cell.y as f32 + (rz > 0) as i32 as f32,
        );
        let u = rx as f32 * (x - ex) + rz as f32 * (z - ez);
        let v = sx as f32 * (x - ex) + sz as f32 * (z - ez);
        let (k, j) = (u.floor() as i32 + 1, v.floor() as i32);
        let cell_k = |k: i32| {
            Cell::new(flight.anchor_cell.x + rx * k + sx * j, flight.anchor_cell.y + rz * k + sz * j)
        };
        let mut here = cell_k(k);
        let (own, other) = if self.dir < 0 { (fp.landing, fp.foot) } else { (fp.foot, fp.landing) };
        // On the edge between the last well cell and the far end (u a whole number) the point is still the flight's:
        // the edge belongs to the well, never to the other storey's cell.
        if here == other && u.fract() == 0.0 && fp.upper_hole.contains(&cell_k(k - 1)) {
            here = cell_k(k - 1);
        }
        // The same for a far end off the run axis (a spiral exits sideways): a point exactly on the edge it shares with a
        // well cell stays the well's.
        if here == other {
            let west = Cell::new(here.x - 1, here.y);
            let north = Cell::new(here.x, here.y - 1);
            if x.fract() == 0.0 && fp.upper_hole.contains(&west) {
                here = west;
            } else if z.fract() == 0.0 && fp.upper_hole.contains(&north) {
                here = north;
            }
        }
        if here == other || !(here == own || fp.upper_hole.contains(&here)) {
            return None;
        }
        flight_surface_y(&flight, x, z).map(|y| y - level as f32 * STOREY)
    }

    /// The walked route from the centre of A to the centre of B, always in that order.
    pub fn walk_path(&self, level: i32, step: f32) -> Vec<(f32, f32)> {
        match self.flight(level) {
            Some(f) => flight_walk_path(&f, step),
            None => vec![self.cell.center()],
        }
    }

    /// The same record moved by `(dx, dy)` cells.
    pub fn shifted(&self, dx: i32, dy: i32) -> Stair {
        Stair { cell: Cell::new(self.cell.x + dx, self.cell.y + dy), ..*self }
    }
}

/// What a level link is (Godot `LevelLink.kind`): a stair flight, or a sewer ladder under a manhole or a floor grate
/// (`world::level_links`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Stairs,
    Ladder(Cover),
}

/// A ladder's `cover` at the maze end (docs/SEWERS.md "Ladders").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Manhole,
    Grate,
}

pub struct Options {
    pub cols: i32,
    pub rows: i32,
    pub relay_count: usize,
    pub battery_count: usize,
    pub room_count: usize,
    pub safe_zone_count: i32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            cols: 45,
            rows: 45,
            relay_count: 5,
            battery_count: 7,
            room_count: 9,
            safe_zone_count: 2,
        }
    }
}

impl Facility {
    pub fn generate(seed_text: &str, opt: &Options, cat: &Catalog) -> Facility {
        let cols = opt.cols.clamp(35, 121);
        let rows = opt.rows.clamp(35, 121);
        let mut f = Facility {
            seed_text: seed_text.to_string(),
            cols,
            rows,
            wall: vec![vec![true; cols as usize]; rows as usize],
            rooms: vec![],
            props: vec![],
            start: Cell::new(cols / 2, rows / 2),
            exit: Cell::new(cols / 2, rows / 2),
            safe_zone: None,
            relays: vec![],
            batteries: vec![],
            hiding: vec![],
            pickups: vec![],
            room_lights: vec![],
            hunter_spawn: Cell::new(0, 0),
            has_lift: true,
            stairs: vec![],
            cell_region: vec![0; (cols * rows) as usize],
            region_names: vec!["maze".into()],
            origin: (0, 0),
            hole: vec![false; (cols * rows) as usize],
            hunter_open: vec![],
            random: Rng::new(seed_text),
            prop_grid: vec![vec![]; (cols * rows) as usize],
            door_state: HashMap::new(),
            walk_surface: HashMap::new(),
            catwalk_links: vec![],
            maze_variant: None,
            level: 0,
            drop_list: vec![],
            air_list: vec![],
            drop: vec![],
            air: vec![],
        };
        let room_count = opt.room_count.max(5);
        f.carve_facility_rooms(room_count, cat);
        f.start = f.rooms[0].center;
        f.connect_facility_rooms();
        f.populate_rooms(cat);
        let from_start = f.distance_map(f.start);
        f.exit = f.pick_service_lift(&from_start, cat);
        if opt.safe_zone_count.clamp(0, 4) > 0 {
            // safe_zone_at_spawn = safe_zone_count > 0 (RoomState.generator_options)
            f.safe_zone = Some(f.start);
        }
        f.place_relays(&from_start, opt.relay_count.max(1), cat);
        f.place_batteries(&from_start, opt.battery_count.max(1), cat);
        // FacilityView3D's on-demand placements, each on its own seeded stream.
        f.place_hiding_spots(6, cat);
        f.place_room_lights();
        f.place_pickups();
        f.build_hunter_grid();
        f.hunter_spawn = f.place_hunter_spawn(&from_start);
        f
    }

    /// Replace the finite-floor sector layout with the room-bank topology used by
    /// StreamedRouteGenerator in the Godot build.  Infinite chunks are still backed
    /// by Facility so collision, props and objectives share one implementation, but
    /// their walls must be authored as halls plus bounded rooms rather than as the
    /// finite floor's Kruskal graph.
    pub fn apply_godot_streamed_layout(&mut self, seed_text: &str, region: &str, cat: &Catalog) {
        self.wall = vec![vec![true; self.cols as usize]; self.rows as usize];
        self.rooms.clear();
        self.props.clear();
        self.relays.clear();
        self.batteries.clear();
        self.hiding.clear();
        self.pickups.clear();
        self.room_lights.clear();
        self.safe_zone = None;
        self.prop_grid = vec![vec![]; (self.cols * self.rows) as usize];
        self.hunter_open.clear();
        self.random = Rng::new(&format!("{seed_text}:ROOMS"));

        let center = self.cols / 2;
        let maze_wide = matches!(region, "maze" | "mall");
        let half = if maze_wide { 2 } else { 1 };
        self.carve_rect(center - half, 1, 1 + half * 2, self.rows - 2);
        self.carve_rect(1, center - half, self.cols - 2, 1 + half * 2);

        let dense = !matches!(region, "apartment" | "beach");
        let room_region = match region {
            "maze" => "arcade",
            "hotel" | "apartment" => "mansion",
            "prison" => "prison",
            "beach" => "beach",
            "gothic" | "mansion" => "mansion",
            "carnival" => "circus",
            "hospital" => "hospital",
            "lab" => "lab",
            "greenhouse" => "greenhouse",
            "mall" => "mall",
            "inferno" => "nether",
            _ => "maintenance",
        };
        let candidates: Vec<usize> = cat
            .types
            .iter()
            .enumerate()
            .filter(|(_, t)| t.region == room_region && !t.streamed_only && !t.props.is_empty())
            .map(|(i, _)| i)
            .collect();
        let fallback = cat
            .types
            .iter()
            .position(|t| !t.streamed_only && !t.props.is_empty())
            .unwrap_or(0);
        let pick_type = |n: usize| -> usize {
            candidates
                .get(n % candidates.len().max(1))
                .copied()
                .unwrap_or(fallback)
        };
        let add_room = |f: &mut Facility, x: i32, y: i32, w: i32, h: i32, door: Cell, n: usize| {
            let ti = pick_type(n);
            let t = &cat.types[ti];
            f.carve_rect(x, y, w, h);
            // Godot rooms have one canonical threshold into a shared hall. Restore
            // the perimeter after carving so a branch never dissolves its neighbours.
            for xx in x..x + w {
                for yy in [y, y + h - 1] {
                    if yy >= 0 && yy < f.rows && xx >= 0 && xx < f.cols && Cell::new(xx, yy) != door
                    {
                        f.wall[yy as usize][xx as usize] = true;
                    }
                }
            }
            for yy in y..y + h {
                for xx in [x, x + w - 1] {
                    if yy >= 0 && yy < f.rows && xx >= 0 && xx < f.cols && Cell::new(xx, yy) != door
                    {
                        f.wall[yy as usize][xx as usize] = true;
                    }
                }
            }
            let rect_center = Cell::new(x + w / 2, y + h / 2);
            f.rooms.push(Room {
                id: format!("room-streamed-{n}"),
                sector: format!("bank-{n}"),
                x,
                y,
                w,
                h,
                center: rect_center,
                rtype: t.id.clone(),
                label: t.label.clone(),
                region: region.to_string(),
                landmark: t.landmark,
                prop_types: t.props_sorted.clone(),
            });
        };

        if dense {
            // The Godot dense layout is four room banks around three east-west
            // halls and the central north-south hall. Maze/mall reserve a wider
            // centre spine, leaving usable room depths on both sides.
            for (row, y) in [1, 12, 24, 35].into_iter().enumerate() {
                let h = if row == 3 { 8 } else { 8 };
                let left_w = if maze_wide { 19 } else { 20 };
                let right_x = if maze_wide {
                    center + half + 1
                } else {
                    center + half + 1
                };
                let right_w = self.cols - right_x - 1;
                let left_x = 1;
                let left_door = Cell::new(
                    left_x + left_w / 2,
                    if row % 2 == 0 { y + h - 1 } else { y },
                );
                let right_door = Cell::new(
                    right_x + right_w / 2,
                    if row % 2 == 0 { y + h - 1 } else { y },
                );
                add_room(self, left_x, y, left_w, h, left_door, row * 2);
                add_room(self, right_x, y, right_w, h, right_door, row * 2 + 1);
                // Re-open the doorway and the adjacent hall cell; the branch is
                // intentionally one cell wide just like the Godot threshold.
                self.wall[left_door.y as usize][left_door.x as usize] = false;
                self.wall[right_door.y as usize][right_door.x as usize] = false;
            }
        } else {
            // Mixed districts keep four spacious bounded rooms, one per sector,
            // attached to the same canonical cross.
            let sectors = [
                (3, 3),
                (center + half + 2, 3),
                (3, center + half + 2),
                (center + half + 2, center + half + 2),
            ];
            for (n, (x, y)) in sectors.into_iter().enumerate() {
                let w = (center - half - 3).max(9).min(self.cols - x - 2);
                let h = (center - half - 3).max(9).min(self.rows - y - 2);
                let door = if n < 2 {
                    Cell::new(x + w / 2, y + h - 1)
                } else {
                    Cell::new(x + w / 2, y)
                };
                add_room(self, x, y, w, h, door, n);
                self.wall[door.y as usize][door.x as usize] = false;
            }
        }

        self.start = Cell::new(center, center);
        self.exit = self.start;
        self.rebuild_grids();
        self.populate_rooms(cat);
        let from_start = self.distance_map(self.start);
        self.place_relays(&from_start, 3, cat);
        self.place_batteries(&from_start, 4, cat);
        self.place_hiding_spots(4, cat);
        self.place_room_lights();
        self.place_pickups();
        self.build_hunter_grid();
        self.hunter_spawn = self.place_hunter_spawn(&from_start);
    }

    // -- Infinite: stitching chunks ---------------------------------------------------------

    /// Opens cells including the outer border (seams between chunks).
    pub fn open_cells(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for row in y.max(0)..(y + h).min(self.rows) {
            for col in x.max(0)..(x + w).min(self.cols) {
                self.wall[row as usize][col as usize] = false;
            }
        }
    }

    /// Drops props whose cell falls in the rectangle (a stairwell cut through a room).
    pub fn clear_props_in(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.props
            .retain(|p| !(p.cell.x >= x && p.cell.x < x + w && p.cell.y >= y && p.cell.y < y + h));
        let inside = |c: Cell| c.x >= x && c.x < x + w && c.y >= y && c.y < y + h;
        self.relays.retain(|c| !inside(*c));
        self.batteries.retain(|c| !inside(*c));
        self.hiding.retain(|h| !inside(h.cell));
        self.pickups.retain(|p| !inside(p.0));
        self.room_lights.retain(|l| !inside(l.switch_cell));
    }

    /// Removes the props of cell `c` and every solid prop whose footprint reaches into it, so a body fills the cell
    /// (a ladder's hatch and step cell, a catwalk link). Room doors are never removed, and relays, batteries, pickups
    /// and lockers stay: they are objectives, not furniture. Call `rebuild_grids` afterwards.
    pub fn clear_props_at(&mut self, c: Cell) {
        self.props.retain(|p| {
            if p.door.is_some() {
                return true;
            }
            let (px, py) = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
            let reaches = p.solid
                && (c.x as f32 + 0.5 - px).abs() < p.half.0 + 0.5
                && (c.y as f32 + 0.5 - py).abs() < p.half.1 + 0.5;
            p.cell != c && !reaches
        });
    }

    /// Recomputes the solid-prop index and the hunters' walkable grid after edits.
    pub fn rebuild_grids(&mut self) {
        self.hole = vec![false; (self.cols * self.rows) as usize];
        for s in self.stairs.clone() {
            // a ladder's foot (dir > 0) is an ordinary floor cell: the hatch above it is the hole
            if s.is_ladder() && s.dir > 0 {
                continue;
            }
            for c in s.well_cells() {
                if c.x >= 0 && c.y >= 0 && c.x < self.cols && c.y < self.rows {
                    self.hole[(c.y * self.cols + c.x) as usize] = true;
                }
            }
        }
        self.index_drops();
        self.prop_grid = vec![vec![]; (self.cols * self.rows) as usize];
        for i in 0..self.props.len() {
            if self.props[i].solid {
                for ci in self.prop_index_cells(&self.props[i]) {
                    self.prop_grid[ci].push(i);
                }
            }
        }
        self.build_hunter_grid();
    }

    /// Joins chunks into one window grid (Infinite's 3x3 live window). Each part is placed at its
    /// cell offset; rooms, props and objectives keep their own data, shifted.
    pub fn merge(
        parts: Vec<(Facility, i32, i32, usize)>,
        cols: i32,
        rows: i32,
        region_names: Vec<String>,
        seed_text: &str,
    ) -> Facility {
        let mut f = Facility {
            seed_text: seed_text.to_string(),
            cols,
            rows,
            wall: vec![vec![true; cols as usize]; rows as usize],
            rooms: vec![],
            props: vec![],
            start: Cell::new(cols / 2, rows / 2),
            exit: Cell::new(-10000, -10000),
            safe_zone: None,
            relays: vec![],
            batteries: vec![],
            hiding: vec![],
            pickups: vec![],
            room_lights: vec![],
            hunter_spawn: Cell::new(cols / 2, rows / 2),
            has_lift: false,
            stairs: vec![],
            cell_region: vec![0; (cols * rows) as usize],
            region_names,
            origin: (0, 0),
            hole: vec![false; (cols * rows) as usize],
            hunter_open: vec![],
            random: Rng::new(seed_text),
            prop_grid: vec![],
            door_state: HashMap::new(),
            walk_surface: HashMap::new(),
            catwalk_links: vec![],
            maze_variant: None,
            level: parts.first().map_or(0, |p| p.0.level),
            drop_list: vec![],
            air_list: vec![],
            drop: vec![],
            air: vec![],
        };
        for (p, ox, oy, region) in parts {
            let sh = |c: Cell| Cell::new(c.x + ox, c.y + oy);
            for y in 0..p.rows {
                for x in 0..p.cols {
                    let (gx, gy) = (x + ox, y + oy);
                    if gx < 0 || gy < 0 || gx >= cols || gy >= rows {
                        continue;
                    }
                    f.wall[gy as usize][gx as usize] = p.wall[y as usize][x as usize];
                    f.cell_region[(gy * cols + gx) as usize] = region as u8;
                }
            }
            let room_base = f.rooms.len();
            for r in p.rooms {
                f.rooms.push(Room {
                    id: format!("{}@{},{}", r.id, ox, oy),
                    x: r.x + ox,
                    y: r.y + oy,
                    center: sh(r.center),
                    ..r
                });
            }
            for pr in p.props {
                f.props.push(Prop {
                    cell: sh(pr.cell),
                    room: pr.room + room_base,
                    ..pr
                });
            }
            f.relays.extend(p.relays.into_iter().map(sh));
            f.batteries.extend(p.batteries.into_iter().map(sh));
            f.hiding.extend(p.hiding.into_iter().map(|h| Hiding {
                cell: sh(h.cell),
                room: h.room.map(|r| r + room_base),
                ..h
            }));
            f.pickups
                .extend(p.pickups.into_iter().map(|(c, k)| (sh(c), k)));
            f.room_lights
                .extend(p.room_lights.into_iter().map(|l| RoomLight {
                    room: l.room + room_base,
                    center: sh(l.center),
                    switch_cell: sh(l.switch_cell),
                    ..l
                }));
            f.stairs.extend(p.stairs.into_iter().map(|s| s.shifted(ox, oy)));
            f.drop_list.extend(p.drop_list.iter().map(|c| sh(*c)));
            f.air_list.extend(p.air_list.iter().map(|c| sh(*c)));
            // catwalk decks and their links ride along with the chunk they belong to
            for (c, s) in p.walk_surface {
                f.walk_surface.insert(sh(c), s);
            }
            f.catwalk_links.extend(p.catwalk_links.into_iter().map(|l| CatwalkLink { cell: sh(l.cell), ..l }));
            if let Some(s) = p.safe_zone {
                f.safe_zone = Some(sh(s));
            }
        }
        f.rebuild_grids();
        f
    }

    // -- sectors and rooms ------------------------------------------------------------------

    fn carve_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for row in y.max(1)..(y + h).min(self.rows - 1) {
            for col in x.max(1)..(x + w).min(self.cols - 1) {
                self.wall[row as usize][col as usize] = false;
            }
        }
    }

    fn divisions(size: i32) -> i32 {
        if size >= 110 {
            7
        } else if size >= 66 {
            5
        } else {
            3
        }
    }

    fn bands(size: i32, n: i32) -> Vec<(i32, i32)> {
        let usable = size - SECTOR_MARGIN * 2 - SECTOR_GAP * (n - 1);
        let band = usable / n;
        let rem = usable - band * n;
        let centre = n / 2;
        let mut pos = SECTOR_MARGIN;
        (0..n)
            .map(|i| {
                let s = band + if i == centre { rem } else { 0 };
                let out = (pos, s);
                pos += s + SECTOR_GAP;
                out
            })
            .collect()
    }

    fn room_types_for_count(&mut self, count: usize, cat: &Catalog) -> Vec<usize> {
        let mut chosen = vec![];
        let mut ordinary = vec![];
        for (i, t) in cat.types.iter().enumerate() {
            if t.streamed_only {
                continue;
            }
            if t.special > 0 {
                if t.special as usize <= count && (!t.large_only || self.cols >= 61) {
                    chosen.push(i);
                }
            } else {
                ordinary.push(i);
            }
        }
        self.random.shuffle(&mut ordinary);
        let mut k = 0;
        while chosen.len() < count && !ordinary.is_empty() {
            chosen.push(ordinary[k % ordinary.len()]);
            k += 1;
        }
        self.random.shuffle(&mut chosen);
        chosen
    }

    fn size_range(dim: i32, r: (f64, f64), (min, max): (i32, i32)) -> (i32, i32) {
        let clamp = |v: i32| {
            let top = max.min(dim);
            min.min(top).max(top.min(v))
        };
        let lo = clamp((dim as f64 * r.0).floor() as i32);
        let hi = lo.max(clamp((dim as f64 * r.1).floor() as i32));
        (lo, hi)
    }

    fn carve_facility_rooms(&mut self, room_count: usize, cat: &Catalog) {
        let (cx, cy) = (self.cols / 2, self.rows / 2);
        let off = LOBBY_SIZE / 2;
        self.rooms.push(Room {
            id: "room-lobby".into(),
            sector: "lobby".into(),
            x: cx - off,
            y: cy - off,
            w: LOBBY_SIZE,
            h: LOBBY_SIZE,
            center: Cell::new(cx, cy),
            rtype: "lobby".into(),
            label: String::new(),
            region: String::new(),
            landmark: false,
            prop_types: vec![],
        });
        self.carve_rect(cx - off, cy - off, LOBBY_SIZE, LOBBY_SIZE);

        let nx = Self::divisions(self.cols);
        let ny = Self::divisions(self.rows);
        let hb = Self::bands(self.cols, nx);
        let vb = Self::bands(self.rows, ny);
        let (scx, scy) = (nx / 2, ny / 2);
        let mut cardinal = vec![];
        let mut others = vec![];
        for row in 0..ny {
            for col in 0..nx {
                if col == scx && row == scy {
                    continue;
                }
                let id = if col == scx && row == scy - 1 {
                    "north".to_string()
                } else if col == scx && row == scy + 1 {
                    "south".to_string()
                } else if col == scx - 1 && row == scy {
                    "west".to_string()
                } else if col == scx + 1 && row == scy {
                    "east".to_string()
                } else {
                    format!("sector-{col}-{row}")
                };
                let s = (
                    id.clone(),
                    hb[col as usize].0,
                    vb[row as usize].0,
                    hb[col as usize].1,
                    vb[row as usize].1,
                );
                if ["north", "south", "west", "east"].contains(&id.as_str()) {
                    cardinal.push(s)
                } else {
                    others.push(s)
                }
            }
        }
        self.random.shuffle(&mut others);
        let mut sectors = cardinal;
        sectors.extend(others);
        let outer = sectors.len().min(room_count.saturating_sub(1));
        let types = self.room_types_for_count(outer, cat);
        for index in 0..outer {
            let (sid, sx, sy, sw, sh) = sectors[index].clone();
            let t = cat.types[types[index]].clone();
            let smallest = sw.min(sh);
            let class = if t.landmark {
                (0.84, 1.0)
            } else if smallest >= 16 {
                let roll = self.random.next_float();
                if roll < 0.32 {
                    (0.5, 0.64)
                } else if roll < 0.82 {
                    (0.64, 0.8)
                } else {
                    (0.78, 0.92)
                }
            } else if self.random.next_float() < 0.38 {
                (0.5, 0.64)
            } else {
                (0.64, 0.8)
            };
            // the room's region sets its side range (its geometry block), else the game-wide range
            let sizes = room_sizes_for(crate::content::regions::table().def(&t.region));
            let (room_min, _) = sizes;
            let (wl, wh) = Self::size_range(sw, class, sizes);
            let (hl, hh) = Self::size_range(sh, class, sizes);
            let dw = wl + self.random.next_index((wh - wl + 1) as usize) as i32;
            let dh = hl + self.random.next_index((hh - hl + 1) as usize) as i32;
            let wc = if t.landmark {
                sw
            } else {
                room_min
                    .min(sw)
                    .max((sw as f64 * ROOM_SECTOR_FILL).floor() as i32)
            };
            let hc = if t.landmark {
                sh
            } else {
                room_min
                    .min(sh)
                    .max((sh as f64 * ROOM_SECTOR_FILL).floor() as i32)
            };
            let w = wc.min(dw);
            let h = hc.min(dh);
            let x = sx + self.random.next_index((sw - w).max(0) as usize + 1) as i32;
            let y = sy + self.random.next_index((sh - h).max(0) as usize + 1) as i32;
            let center = Cell::new(
                (x as f64 + w as f64 / 2.0) as i32,
                (y as f64 + h as f64 / 2.0) as i32,
            );
            self.rooms.push(Room {
                id: format!("room-{index}"),
                sector: sid,
                x,
                y,
                w,
                h,
                center,
                rtype: t.id.clone(),
                label: t.label.clone(),
                region: t.region.clone(),
                landmark: t.landmark,
                prop_types: t.props_sorted.clone(),
            });
            self.carve_rect(x, y, w, h);
        }
    }

    // -- halls ------------------------------------------------------------------------------

    fn connect_facility_rooms(&mut self) {
        let n = self.rooms.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut c = i;
            while p[c] != c {
                c = p[c];
            }
            let mut k = i;
            while p[k] != k {
                let nx = p[k];
                p[k] = c;
                k = nx;
            }
            c
        }
        let mut selected: Vec<(usize, usize, u8)> = vec![]; // kind: 0 main, 1 connector, 2 service
        let mut keys: HashSet<(usize, usize)> = HashSet::new();
        let select = |a: usize,
                          b: usize,
                          kind: u8,
                          parent: &mut Vec<usize>,
                          keys: &mut HashSet<(usize, usize)>,
                          sel: &mut Vec<(usize, usize, u8)>| {
            let key = (a.min(b), a.max(b));
            if keys.contains(&key) {
                return;
            }
            keys.insert(key);
            sel.push((a, b, kind));
            let (ra, rb) = (find(parent, a), find(parent, b));
            if ra != rb {
                parent[rb] = ra;
            }
        };
        for card in ["north", "south", "west", "east"] {
            if let Some(i) = self.rooms.iter().position(|r| r.sector == card) {
                select(0, i, 0, &mut parent, &mut keys, &mut selected);
            }
        }
        // (from, to, manhattan distance, score)
        let mut cands: Vec<(usize, usize, f64, f64)> = vec![];
        for a in 0..n {
            for b in (a + 1)..n {
                let (ca, cb) = (self.rooms[a].center, self.rooms[b].center);
                let d = ((ca.x - cb.x).abs() + (ca.y - cb.y).abs()) as f64;
                let score = d + self.random.next_float() * 3.5;
                cands.push((a, b, d, score));
            }
        }
        sort_custom(&mut cands, |p, q| p.3 < q.3);
        for &(a, b, _, _) in &cands {
            if find(&mut parent, a) == find(&mut parent, b) {
                continue;
            }
            select(a, b, 1, &mut parent, &mut keys, &mut selected);
        }
        let mut loops: Vec<_> = cands
            .iter()
            .filter(|c| !keys.contains(&(c.0, c.1)) && c.2 <= 24.0)
            .cloned()
            .collect();
        self.random.shuffle(&mut loops);
        sort_custom(
            &mut loops,
            |p, q| if p.2 != q.2 { p.2 < q.2 } else { p.3 < q.3 },
        );
        for c in loops.iter().take(4) {
            select(c.0, c.1, 2, &mut parent, &mut keys, &mut selected);
        }
        for (a, b, kind) in selected {
            let from = self.rooms[a].center;
            let to = self.rooms[b].center;
            let dx = (from.x - to.x).abs();
            let dy = (from.y - to.y).abs();
            let mut horizontal_first = None;
            if kind == 0 {
                let s = if ["north", "south", "west", "east"]
                    .contains(&self.rooms[a].sector.as_str())
                {
                    self.rooms[a].sector.clone()
                } else {
                    self.rooms[b].sector.clone()
                };
                if s == "north" || s == "south" {
                    horizontal_first = Some(false);
                } else if s == "west" || s == "east" {
                    horizontal_first = Some(true);
                }
            }
            let horizontal_first = match horizontal_first {
                Some(h) => h,
                None if dx as f64 > dy as f64 * 1.5 => true,
                None if dy as f64 > dx as f64 * 1.5 => false,
                None => self.random.next_float() < 0.5,
            };
            let off = (HALL_WIDTH - 1) / 2;
            let bend = if horizontal_first {
                Cell::new(to.x, from.y)
            } else {
                Cell::new(from.x, to.y)
            };
            let mut seg = |p: Cell, q: Cell, horizontal: bool| {
                if horizontal {
                    self.carve_rect(p.x.min(q.x), p.y - off, (q.x - p.x).abs() + 1, HALL_WIDTH);
                } else {
                    self.carve_rect(p.x - off, p.y.min(q.y), HALL_WIDTH, (q.y - p.y).abs() + 1);
                }
            };
            seg(from, bend, horizontal_first);
            seg(bend, to, !horizontal_first);
        }
    }

    // -- grid queries -----------------------------------------------------------------------

    pub fn is_floor(&self, x: f64, y: f64) -> bool {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        cx >= 0
            && cy >= 0
            && cx < self.cols as i64
            && cy < self.rows as i64
            && !self.wall[cy as usize][cx as usize]
    }

    pub fn is_wall_cell(&self, cx: i32, cy: i32) -> bool {
        cx < 0
            || cy < 0
            || cx >= self.cols
            || cy >= self.rows
            || self.wall[cy as usize][cx as usize]
    }

    fn valid_open_cell(&self, c: Cell) -> bool {
        !self.is_wall_cell(c.x, c.y)
    }

    /// The generator's can_occupy: five footprint samples on floor, clear of solid props.
    pub fn can_occupy(&self, x: f64, y: f64, r: f64) -> bool {
        if !self.drop_list.is_empty() {
            // a drop has no floor to stand on: nothing is placed, spawned or routed over one
            for (a, b) in [(x, y), (x - r, y - r), (x + r, y - r), (x - r, y + r), (x + r, y + r)] {
                if self.is_drop(Cell::new(a.floor() as i32, b.floor() as i32)) {
                    return false;
                }
            }
        }
        self.clear_of_solids(x, y, r)
    }

    /// `can_occupy` without the drop rule: five footprint samples on floor (a drop is floor-less but wall-less, so a
    /// body may enter it), clear of solid props. What `fits` builds on.
    fn clear_of_solids(&self, x: f64, y: f64, r: f64) -> bool {
        for (a, b) in [
            (x, y),
            (x - r, y - r),
            (x + r, y - r),
            (x - r, y + r),
            (x + r, y + r),
        ] {
            if !self.is_floor(a, b) {
                return false;
            }
        }
        if self.props.is_empty() {
            return true;
        }
        // props are at most ~1.2 cells from their own cell; check the neighbourhood
        let (cx, cy) = (x.floor() as i32, y.floor() as i32);
        for yy in (cy - 2).max(0)..=(cy + 2).min(self.rows - 1) {
            for xx in (cx - 2).max(0)..=(cx + 2).min(self.cols - 1) {
                for &pi in &self.prop_grid[(yy * self.cols + xx) as usize] {
                    let p = &self.props[pi];
                    let (px, py) = (p.cell.x as f64 + 0.5 + p.offset.0 as f64, p.cell.y as f64 + 0.5 + p.offset.1 as f64);
                    if (x - px).abs() < p.half.0 as f64 + r && (y - py).abs() < p.half.1 as f64 + r
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Whether the cell has no floor: a stairwell or ladder hatch (not walkable except on its flight) or a drop (open
    /// air a body may enter and fall through).
    pub fn is_hole(&self, x: i32, y: i32) -> bool {
        self.is_stair_hole(x, y) || self.is_drop(Cell::new(x, y))
    }

    /// A stairwell or ladder-hatch cell: the hole rule keeps a walking body out of it unless it is on the flight.
    fn is_stair_hole(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.cols
            && y < self.rows
            && self
                .hole
                .get((y * self.cols + x) as usize)
                .copied()
                .unwrap_or(false)
    }

    fn index_drops(&mut self) {
        let n = (self.cols * self.rows) as usize;
        let (cols, rows) = (self.cols, self.rows);
        let index = |list: &[Cell]| {
            let mut v = vec![false; n];
            for c in list {
                if c.x >= 0 && c.y >= 0 && c.x < cols && c.y < rows {
                    v[(c.y * cols + c.x) as usize] = true;
                }
            }
            v
        };
        self.drop = index(&self.drop_list);
        self.air = index(&self.air_list);
    }

    /// Makes `cells` this level's drops (a waterfall void): no floor and no wall, open air down to the level below.
    /// A body at or above the floor may enter one (`fits`), then falls; hunters never walk in (`hunter_open`), and
    /// nothing is placed on one (`can_occupy`, `floor_cells`). Replaces any earlier drops.
    pub fn set_drop_cells(&mut self, cells: &[Cell]) {
        self.drop_list = cells.to_vec();
        self.drop_list.sort();
        self.drop_list.dedup();
        self.index_drops();
    }

    /// Makes `cells` this level's air-above cells: no ceiling, because a drop on the level above falls through them.
    pub fn set_air_above_cells(&mut self, cells: &[Cell]) {
        self.air_list = cells.to_vec();
        self.air_list.sort();
        self.air_list.dedup();
        self.index_drops();
    }

    /// Whether `c` is a drop cell (open air, no floor).
    pub fn is_drop(&self, c: Cell) -> bool {
        c.x >= 0
            && c.y >= 0
            && c.x < self.cols
            && c.y < self.rows
            && self.drop.get((c.y * self.cols + c.x) as usize).copied().unwrap_or(false)
    }

    /// Every drop cell (sorted).
    pub fn drop_cells(&self) -> &[Cell] {
        &self.drop_list
    }

    /// Whether a drop cell lies within `radius` cells of the point `(x, z)` (grid cells): a cheap test for whether the
    /// level below must be resident (`keep_stack`). The distance is taken to the nearest point of the cell.
    pub fn drop_within(&self, x: f32, z: f32, radius: f32) -> bool {
        self.drop_list.iter().any(|c| {
            let dx = (x - (c.x as f32 + 0.5)).abs() - 0.5;
            let dz = (z - (c.y as f32 + 0.5)).abs() - 0.5;
            dx.max(0.0).hypot(dz.max(0.0)) <= radius
        })
    }

    /// Whether `c` is open to the sky of the level below it: a drop on the level above falls through it, so it has no
    /// ceiling (the pool under a waterfall).
    pub fn is_air_above(&self, c: Cell) -> bool {
        c.x >= 0
            && c.y >= 0
            && c.x < self.cols
            && c.y < self.rows
            && self.air.get((c.y * self.cols + c.x) as usize).copied().unwrap_or(false)
    }

    /// Every air-above cell (sorted).
    pub fn air_above_cells(&self) -> &[Cell] {
        &self.air_list
    }

    /// The region name of a cell's chunk.
    pub fn region_at(&self, x: i32, y: i32) -> &str {
        let i = if x >= 0 && y >= 0 && x < self.cols && y < self.rows {
            self.cell_region[(y * self.cols + x) as usize] as usize
        } else {
            0
        };
        self.region_names
            .get(i)
            .map(|s| s.as_str())
            .unwrap_or("maze")
    }

    /// Whether (x, y) is a tread of a stair flight. A flight's well cells are holes for a body that is not
    /// walking it; the flight itself is walkable at the height stair_surface_y gives for that point.
    fn on_stair_flight(&self, x: f32, y: f32) -> bool {
        let cell = Cell::new(x.floor() as i32, y.floor() as i32);
        self.stairs.iter().any(|s| {
            s.well_cells().contains(&cell) && s.surface_rel(self.level, x, y).is_some()
        })
    }

    pub fn body_fits(&self, x: f32, y: f32, r: f32) -> bool {
        self.body_fits_at(x, y, r, 0.0)
    }

    /// Every cell of every stair's well (sorted, no repeats).
    pub fn well_cells_all(&self) -> Vec<Cell> {
        let mut v: Vec<Cell> = self.stairs.iter().flat_map(|s| s.well_cells()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Whether the cell has no ceiling: part of a stair's well or a ladder's shaft that is open to the level above (a
    /// lower-level record, dir > 0; a ladder's foot is its hatch cell), or air under a drop (`is_air_above`).
    pub fn open_above(&self, c: Cell) -> bool {
        self.is_air_above(c) || self.stairs.iter().any(|s| s.dir > 0 && s.well_cells().contains(&c))
    }

    /// Whether the cell has no floor: part of a stair's well or a ladder's hatch that is open to the level below (an
    /// upper-level record, dir < 0), or a drop.
    pub fn open_below(&self, c: Cell) -> bool {
        self.is_drop(c) || self.stairs.iter().any(|s| s.dir < 0 && s.well_cells().contains(&c))
    }

    /// Whether a body of radius `r` at `(x, z)` overlaps the wall between the lanes of any switchback.
    pub fn divider_blocks(&self, x: f32, z: f32, r: f32) -> bool {
        if self.stairs.is_empty() {
            return false;
        }
        let reach = r + flights().divider_thickness * 0.5;
        self.stairs.iter().any(|s| {
            s.dividers(self.level).iter().any(|seg| {
                let (dx, dz) = (seg.b.0 - seg.a.0, seg.b.1 - seg.a.1);
                let len2 = dx * dx + dz * dz;
                let t = if len2 > 0.0 { (((x - seg.a.0) * dx + (z - seg.a.1) * dz) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (px, pz) = (seg.a.0 + dx * t, seg.a.1 + dz * t);
                let (ex, ez) = (x - px, z - pz);
                ex * ex + ez * ez < reach * reach
            })
        })
    }

    /// Whether `(x, z)` is on a flight of this facility with its surface within `FLIGHT_FIT_TOL` of the walk height
    /// `height` (relative to this facility's floor).
    fn on_flight_at(&self, x: f32, z: f32, height: f32) -> bool {
        self.stairs
            .iter()
            .any(|s| s.surface_rel(self.level, x, z).is_some_and(|rel| (rel - height).abs() <= FLIGHT_FIT_TOL))
    }

    /// `body_fits_at` for a body whose feet are at absolute height `y_abs`: the walk height is `y_abs` minus this
    /// facility's own floor (`level * STOREY`), so the result equals `body_fits_at` at that relative height. Over a
    /// stair well the hole rule holds unless the body is on the flight at its surface (within `FLIGHT_FIT_TOL`), so a
    /// body at the wrong height over a well is over a hole.
    pub fn body_fits_at_y(&self, x: f32, z: f32, y_abs: f32, r: f32) -> bool {
        let floor_y = self.level as f32 * crate::world::vertical::STOREY;
        self.fits(x, z, r, y_abs - floor_y, true)
    }

    /// `body_fits` for a body standing at walk height `height`: every cell the body touches must have a
    /// walk surface at that same height, so a body on the floor cannot enter a deck and a body on a deck
    /// cannot step onto the floor.
    pub fn body_fits_at(&self, x: f32, y: f32, r: f32, height: f32) -> bool {
        self.fits(x, y, r, height, false)
    }

    /// The shared body of `body_fits_at` and `body_fits_at_y`. `by_height`: a corner on a flight at the body's height is
    /// exempt from the deck-height and hole rules; otherwise any tread of a flight is exempt from the hole rule.
    fn fits(&self, x: f32, y: f32, r: f32, height: f32, by_height: bool) -> bool {
        // a drop has no wall, so a body may enter it (the hole rule below does not apply to a drop either)
        if !self.clear_of_solids(x as f64, y as f64, r as f64) {
            return false;
        }
        if self.divider_blocks(x, y, r) {
            return false;
        }
        if !self.walk_surface.is_empty() || height != 0.0 {
            for (sx, sy) in [(x - r, y - r), (x + r, y - r), (x - r, y + r), (x + r, y + r)] {
                let cell = Cell::new(sx.floor() as i32, sy.floor() as i32);
                // open air holds a body at any height: only the cells around it say whether it is below the floor
                if self.is_drop(cell) {
                    continue;
                }
                if (self.height_at(cell) - height).abs() > 0.01 && !(by_height && self.on_flight_at(sx, sy, height)) {
                    return false;
                }
            }
        }
        for (sx, sy) in [
            (x - r, y - r),
            (x + r, y - r),
            (x - r, y + r),
            (x + r, y + r),
            (x, y),
        ] {
            if self.is_stair_hole(sx.floor() as i32, sy.floor() as i32) {
                let on = if by_height { self.on_flight_at(sx, sy, height) } else { self.on_stair_flight(sx, sy) };
                if !on {
                    return false;
                }
            }
        }
        if self.door_blocks(x, y, r) {
            return false;
        }
        self.hiding.iter().all(|h| {
            let (lx, ly) = self.locker_center(h);
            (x - lx).abs() >= LOCKER_HALF + r || (y - ly).abs() >= LOCKER_HALF + r
        })
    }

    /// The centre of a room door and the half extents of its blocking slab (long side along the wall).
    fn door_slab(p: &Prop, d: DoorDraw) -> ((f32, f32), (f32, f32)) {
        let c = (p.cell.x as f32 + 0.5 + d.offset[0], p.cell.y as f32 + 0.5 + d.offset[1]);
        let along = d.span as f32 * 0.5;
        let half = if p.rotation.rem_euclid(2) == 0 {
            (along, DOOR_HALF_DEPTH)
        } else {
            (DOOR_HALF_DEPTH, along)
        };
        (c, half)
    }

    /// The width of the opening a door leaves: its leaf, `span` metres long and hinged at one jamb, has swung
    /// `open` of a quarter turn, so it covers `span * cos` of the doorway along the wall.
    fn door_gap(open: f32, span: f32) -> f32 {
        span * (1.0 - (open.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).cos())
    }

    /// Whether a door's slab overlaps a body of radius `r` at `(x, y)` while the opening it leaves is narrower
    /// than the body (so a half-swung leaf is solid across the whole doorway: a body never clips through it).
    fn door_blocks(&self, x: f32, y: f32, r: f32) -> bool {
        self.door_in_way(x, y, r).is_some()
    }

    /// The threshold cell of the room door that blocks a body of radius `r` at `(x, y)` (see `door_blocks`).
    pub fn door_in_way(&self, x: f32, y: f32, r: f32) -> Option<Cell> {
        self.door_state
            .iter()
            .find(|(_, s)| {
                Self::door_gap(s.open, s.span) < 2.0 * r + DOOR_GAP_MARGIN
                    && (x - s.centre.0).abs() < s.half.0 + r
                    && (y - s.centre.1).abs() < s.half.1 + r
            })
            .map(|(c, _)| *c)
    }

    /// The centre of the room door whose threshold cell is `cell` (None for a door not yet ticked).
    pub fn door_centre(&self, cell: Cell) -> Option<(f32, f32)> {
        self.door_state.get(&cell).map(|s| s.centre)
    }

    /// The point of the slab of the door at `cell` nearest to `from` (None for a door not yet ticked).
    pub fn door_nearest_to(&self, cell: Cell, from: (f32, f32)) -> Option<(f32, f32)> {
        let s = self.door_state.get(&cell)?;
        Some((from.0.clamp(s.centre.0 - s.half.0, s.centre.0 + s.half.0), from.1.clamp(s.centre.1 - s.half.1, s.centre.1 + s.half.1)))
    }

    /// The open amount of the room door whose threshold cell is `cell`: 0 (shut, the pose the world meshes hold it in) for a door not yet ticked.
    pub fn door_open_amount(&self, cell: Cell) -> f32 {
        self.door_state.get(&cell).map_or(0.0, |s| s.open)
    }

    /// Whether the door at `cell` is meant to be open (a door not yet ticked is open).
    pub fn door_is_open(&self, cell: Cell) -> bool {
        self.door_state.get(&cell).map_or(true, |s| s.want_open)
    }

    /// The prop index and centre of every room door.
    pub fn doors(&self) -> impl Iterator<Item = (usize, (f32, f32))> + '_ {
        self.props
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.door.map(|d| (i, Self::door_slab(p, d).0)))
    }

    /// The point of door `i`'s slab nearest to `from` (what the runner faces when standing flush against the leaf).
    pub fn door_nearest(&self, i: usize, from: (f32, f32)) -> Option<(f32, f32)> {
        let p = self.props.get(i)?;
        let (c, h) = Self::door_slab(p, p.door?);
        Some(((from.0).clamp(c.0 - h.0, c.0 + h.0), (from.1).clamp(c.1 - h.1, c.1 + h.1)))
    }

    /// Whether a body at `b` stands in the sweep of a door slab.
    fn in_door_sweep(centre: (f32, f32), half: (f32, f32), b: (f32, f32)) -> bool {
        (b.0 - centre.0).abs() < half.0 + DOOR_SWEEP && (b.1 - centre.1).abs() < half.1 + DOOR_SWEEP
    }

    /// An interact press (the runner's or a player bot's) on the door at threshold `cell`. An open door closes unless a body in `bodies` stands in
    /// its sweep (`Blocked`: the leaf would swing through them); a closed door always opens. `None` for a cell that
    /// is not a ticked room door.
    pub fn toggle_door(&mut self, cell: Cell, bodies: &[(f32, f32)]) -> Option<DoorToggle> {
        let s = self.door_state.get_mut(&cell)?;
        if !s.want_open {
            s.want_open = true;
            return Some(DoorToggle::Opened);
        }
        if bodies.iter().any(|b| Self::in_door_sweep(s.centre, s.half, *b)) {
            return Some(DoorToggle::Blocked);
        }
        s.want_open = false;
        Some(DoorToggle::Closed)
    }

    /// The walk surface of `cell`: plain `Floor` at 0.0 m unless it was set with `set_walk_surface`.
    pub fn walk_surface_at(&self, cell: Cell) -> WalkSurface {
        self.walk_surface.get(&cell).copied().unwrap_or_default()
    }

    /// The height of `cell`'s walk surface in metres above the level's floor.
    pub fn height_at(&self, cell: Cell) -> f32 {
        self.walk_surface_at(cell).height
    }

    /// Sets the walk surface of `cell` alone; every other cell keeps its surface.
    pub fn set_walk_surface(&mut self, cell: Cell, kind: DeckKind, height: f32) {
        let surface = WalkSurface { kind, height };
        if surface == WalkSurface::default() {
            self.walk_surface.remove(&cell);
        } else {
            self.walk_surface.insert(cell, surface);
        }
    }

    /// Dresses a level 0 maze chunk with its region variant (`route::maze_regions`): records the variant id and thins
    /// the chunk's loose pickups by the variant's `loot_scale` and its props by `prop_density` (a scale of 1 or more
    /// changes nothing; relays, batteries and doors are never thinned). The floors, room roles and prop set of the
    /// variant are read by the renderer through `maze_variant`. Returns the variant, `None` for the plain maze.
    pub fn apply_maze_variant(&mut self, seed: &str, key: ChunkKey) -> Option<&'static maze_regions::MazeVariant> {
        self.maze_variant = None;
        let v = maze_regions::variant_at(seed, key.cx, key.cy, key.level)?;
        self.maze_variant = Some(v.id.clone());
        let tag = format!("{},{}", key.cx, key.cy);
        let loot = v.spawn_rules.loot_scale;
        let mut n = 0usize;
        self.pickups.retain(|_| {
            n += 1;
            maze_regions::keeps(loot, seed, &format!("{tag}:loot"), n)
        });
        let density = v.spawn_rules.prop_density;
        if density < 1.0 {
            let mut n = 0usize;
            self.props.retain(|p| {
                n += 1;
                p.door.is_some() || maze_regions::keeps(density, seed, &format!("{tag}:prop"), n)
            });
            self.rebuild_grids();
        }
        Some(v)
    }

    /// Raises catwalk decks over the level 0 floor of a chunk whose region the data allows (`catwalk_links.json`
    /// `regions`) and links them to the ground by stairs and ladders. The deck grid is `route::catwalk`'s; a deck cell
    /// stands only on an open, clear ground cell, and only where lifting it leaves the cells around it still
    /// connected, so the ground floor stays walkable. Only the decks a link reaches are kept. Returns the number of
    /// deck cells set (0 when the chunk has no catwalk).
    pub fn apply_catwalks(&mut self, seed: &str, key: ChunkKey, region: &str) -> usize {
        self.apply_catwalks_with(seed, key, region, &catwalk_links::rules())
    }

    /// `apply_catwalks` with explicit rules. The link clearance (`rules.link_clearance`, the half side of the footprint
    /// that must be free of solid props on a link's ground cell and its deck landing) decides which catwalks survive.
    pub(crate) fn apply_catwalks_with(
        &mut self,
        seed: &str,
        key: ChunkKey,
        region: &str,
        rules: &catwalk_links::CatwalkRules,
    ) -> usize {
        self.catwalk_links.clear();
        if key.level != 0 || !catwalk_links::allowed_in(region) {
            return 0;
        }
        let params = catwalk::params();
        let grid = catwalk::generate(&format!("{seed}:L0:C{},{}", key.cx, key.cy));
        let n = self.cols;
        if self.rows != n || grid.size as i32 != n {
            return 0;
        }
        let idx = |c: Cell| (c.y * n + c.x) as usize;
        let mut ground = vec![false; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                ground[idx(Cell::new(x, y))] =
                    !self.is_hole(x, y) && self.can_occupy(x as f64 + 0.5, y as f64 + 0.5, 0.25);
            }
        }
        let open = ground.clone();
        let mut keep_clear = self.taken_cells();
        for s in &self.stairs {
            for dy in -3..=3 {
                for dx in -3..=3 {
                    keep_clear.insert(Cell::new(s.cell.x + dx, s.cell.y + dy));
                }
            }
            for c in s.carve_cells() {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        keep_clear.insert(Cell::new(c.x + dx, c.y + dy));
                    }
                }
            }
        }
        for p in self.props.iter().filter(|p| p.door.is_some()) {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    keep_clear.insert(Cell::new(p.cell.x + dx, p.cell.y + dy));
                }
            }
        }
        let mut deck: HashSet<Cell> = HashSet::new();
        for y in 0..n {
            for x in 0..n {
                let c = Cell::new(x, y);
                if grid.walkable(x, y) && ground[idx(c)] && !keep_clear.contains(&c) && Self::lifts_cleanly(&ground, n, c) {
                    ground[idx(c)] = false;
                    deck.insert(c);
                }
            }
        }
        // A link needs its own ground cell and the deck cell it steps onto clear of solid props (the deck grid ignores
        // props): a cell whose body-sized footprint touches a solid prop is refused.
        let clear = |c: Cell| self.clear_of_solids(c.x as f64 + 0.5, c.y as f64 + 0.5, rules.link_clearance);
        let mut candidates: Vec<Cell> = Vec::new();
        for c in &deck {
            if !clear(*c) {
                continue;
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let g = Cell::new(c.x + dx, c.y + dy);
                if g.x >= 0 && g.y >= 0 && g.x < n && g.y < n && ground[idx(g)] && !keep_clear.contains(&g) && clear(g) {
                    candidates.push(g);
                }
            }
        }
        let mut links = catwalk_links::links_for(seed, key, &candidates, Some(params.height), rules);
        if links.is_empty() {
            return 0;
        }
        // keep the deck cells joined to a link
        let mut kept: HashSet<Cell> = HashSet::new();
        let mut queue: VecDeque<Cell> = VecDeque::new();
        for l in &links {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let c = Cell::new(l.cell.x + dx, l.cell.y + dy);
                if deck.contains(&c) && kept.insert(c) {
                    queue.push_back(c);
                }
            }
        }
        while let Some(c) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let m = Cell::new(c.x + dx, c.y + dy);
                if deck.contains(&m) && kept.insert(m) {
                    queue.push_back(m);
                }
            }
        }
        // the ground must not be cut in two by what is kept
        let mut after = open.clone();
        for c in &kept {
            after[idx(*c)] = false;
        }
        if Self::ground_pieces(&after, n) > Self::ground_pieces(&open, n) {
            return 0;
        }
        // each link steps from its ground cell onto an adjacent kept deck cell with no solid prop on it (the deck landing)
        for l in &mut links {
            let landing = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .find(|(dx, dy)| {
                    let d = Cell::new(l.cell.x + dx, l.cell.y + dy);
                    kept.contains(&d) && clear(d)
                })
                .unwrap_or((1, 0));
            l.run = landing;
        }
        for c in &kept {
            self.set_walk_surface(*c, DeckKind::Catwalk, params.height);
        }
        // the two cells of a link hold no prop at all (a rug or lamp the clearance test let through)
        for l in &links {
            self.clear_props_in(l.cell.x, l.cell.y, 1, 1);
            self.clear_props_in(l.cell.x + l.run.0, l.cell.y + l.run.1, 1, 1);
        }
        self.catwalk_links = links;
        self.rebuild_grids();
        kept.len()
    }

    /// The flight of a catwalk link, for its cells only: a ladder whose foot is the link's ground cell and whose landing
    /// is the deck cell it steps onto (`CatwalkLink::run`), with the footprint's clear margins. Its heights are not
    /// the flight's: a deck is `link.deck_height` above the same floor, never a storey.
    pub fn catwalk_flight(link: &CatwalkLink) -> Flight {
        let run = if link.run == (0, 0) { (1, 0) } else { link.run };
        Flight::new(FlightKind::Ladder, FlightShape::Switchback, 1, link.cell, run)
    }

    /// Whether closing `c` leaves its open 4-neighbours joined through the ring of cells around it.
    fn lifts_cleanly(ground: &[bool], n: i32, c: Cell) -> bool {
        const RING: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];
        let at = |(dx, dy): (i32, i32)| {
            let (x, y) = (c.x + dx, c.y + dy);
            x >= 0 && y >= 0 && x < n && y < n && ground[(y * n + x) as usize]
        };
        let o: Vec<bool> = RING.iter().map(|d| at(*d)).collect();
        let edges_open = (1..8).step_by(2).filter(|i| o[*i]).count();
        if edges_open <= 1 {
            return true;
        }
        let Some(start) = o.iter().position(|x| !*x) else {
            return true;
        };
        let (mut run, mut prev_open) = (0usize, false);
        let mut edge_run: Option<usize> = None;
        for step in 1..=8 {
            let i = (start + step) % 8;
            if o[i] {
                if !prev_open {
                    run += 1;
                }
                if i % 2 == 1 {
                    match edge_run {
                        None => edge_run = Some(run),
                        Some(r) if r != run => return false,
                        _ => {}
                    }
                }
            }
            prev_open = o[i];
        }
        true
    }

    /// The number of 4-connected pieces of open ground in `ground` (an n x n grid).
    fn ground_pieces(ground: &[bool], n: i32) -> usize {
        let mut seen = vec![false; ground.len()];
        let mut pieces = 0;
        for s in 0..ground.len() {
            if !ground[s] || seen[s] {
                continue;
            }
            pieces += 1;
            seen[s] = true;
            let mut q = VecDeque::from([s as i32]);
            while let Some(i) = q.pop_front() {
                let (x, y) = (i % n, i / n);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= n || ny >= n {
                        continue;
                    }
                    let j = (ny * n + nx) as usize;
                    if ground[j] && !seen[j] {
                        seen[j] = true;
                        q.push_back(j as i32);
                    }
                }
            }
        }
        pieces
    }

    /// Swings every room door toward its wanted state. `openers` (the hunters, never the runner or a player bot) open a
    /// door they come within `DOOR_OPENER_REACH` of and leave it open. `bodies` is everyone with a body: a leaf
    /// that is not shut never swings closed with one of them in its sweep, it swings back open until they clear.
    /// The first call registers every door closed (open, when a body already stands in its sweep).
    pub fn update_doors(&mut self, openers: &[(f32, f32)], bodies: &[(f32, f32)], dt: f32) {
        for p in &self.props {
            let Some(d) = p.door else { continue };
            let (c, half) = Self::door_slab(p, d);
            let s = self.door_state.entry(p.cell).or_insert_with(|| {
                let held = bodies.iter().any(|b| Self::in_door_sweep(c, half, *b));
                DoorState { open: if held { 1.0 } else { 0.0 }, want_open: held, span: d.span as f32, centre: c, half }
            });
            let reach = DOOR_OPENER_REACH + d.span as f32 * 0.5;
            if !s.want_open && openers.iter().any(|a| (a.0 - c.0).hypot(a.1 - c.1) < reach) {
                s.want_open = true;
            }
            let held = s.open > 0.0 && bodies.iter().any(|b| Self::in_door_sweep(c, half, *b));
            if s.want_open || held {
                s.open = (s.open + DOOR_SWING_RATE * dt).min(1.0);
            } else {
                s.open = (s.open - DOOR_SWING_RATE * dt).max(0.0);
            }
        }
    }

    pub fn locker_center(&self, h: &Hiding) -> (f32, f32) {
        let (cx, cy) = h.cell.center();
        (cx - h.facing.0 as f32 * 0.18, cy - h.facing.1 as f32 * 0.18)
    }

    pub fn floor_cells(&self) -> Vec<Cell> {
        let mut v = vec![];
        for y in 1..self.rows - 1 {
            for x in 1..self.cols - 1 {
                if !self.wall[y as usize][x as usize] && !self.is_drop(Cell::new(x, y)) {
                    v.push(Cell::new(x, y));
                }
            }
        }
        v
    }

    pub fn distance_map(&self, s: Cell) -> HashMap<Cell, i32> {
        let mut d = HashMap::new();
        d.insert(s, 0);
        let mut q = VecDeque::from([s]);
        while let Some(c) = q.pop_front() {
            let base = d[&c];
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if d.contains_key(&n)
                    || !self.can_occupy(n.x as f64 + 0.5, n.y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
                {
                    continue;
                }
                d.insert(n, base + 1);
                q.push_back(n);
            }
        }
        d
    }

    /// nearest_open_position: BFS over floor cells from the point's cell to the first cell a
    /// body of `r` fits in.
    pub fn nearest_open(&self, x: f32, y: f32, r: f32) -> Option<(f32, f32)> {
        let s = Cell::new(x.floor() as i32, y.floor() as i32);
        let mut seen = HashSet::from([s]);
        let mut q = VecDeque::from([s]);
        while let Some(c) = q.pop_front() {
            let (wx, wy) = c.center();
            if self.body_fits(wx, wy, r) {
                return Some((wx, wy));
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if seen.contains(&n) || !self.is_floor(n.x as f64 + 0.5, n.y as f64 + 0.5) {
                    continue;
                }
                seen.insert(n);
                q.push_back(n);
            }
        }
        None
    }

    pub fn nearest_open_or(&self, x: f32, y: f32, r: f32) -> (f32, f32) {
        self.nearest_open(x, y, r).unwrap_or((x, y))
    }

    /// room_at: rooms in order (lobby first), half-open bounds.
    pub fn room_at(&self, x: f64, y: f64) -> Option<usize> {
        self.rooms.iter().position(|r| {
            x >= r.x as f64 && x < (r.x + r.w) as f64 && y >= r.y as f64 && y < (r.y + r.h) as f64
        })
    }

    fn in_walkable_structure_room(&self, c: Cell, cat: &Catalog) -> bool {
        self.room_at(c.x as f64 + 0.5, c.y as f64 + 0.5)
            .map(|i| cat.room_has_walkable_structure(&self.rooms[i].rtype))
            .unwrap_or(false)
    }

    // -- room props -------------------------------------------------------------------------

    /// The solid-prop index cells of a prop: every cell its footprint reaches, plus one cell of margin for the
    /// body radius. A prop can reach past its own cell (a flush bed sits up to ~1.2 cells toward its wall), and
    /// the footprint test reads only the cells two either side of the point.
    fn prop_index_cells(&self, p: &Prop) -> Vec<usize> {
        let cx = p.cell.x as f64 + 0.5 + p.offset.0 as f64;
        let cy = p.cell.y as f64 + 0.5 + p.offset.1 as f64;
        let x0 = ((cx - p.half.0 as f64).floor() as i32 - 1).max(0);
        let x1 = ((cx + p.half.0 as f64).floor() as i32 + 1).min(self.cols - 1);
        let y0 = ((cy - p.half.1 as f64).floor() as i32 - 1).max(0);
        let y1 = ((cy + p.half.1 as f64).floor() as i32 + 1).min(self.rows - 1);
        let mut cells = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                cells.push((y * self.cols + x) as usize);
            }
        }
        cells
    }

    fn add_prop(&mut self, p: Prop) {
        let idx = self.props.len();
        if p.solid {
            for ci in self.prop_index_cells(&p) {
                self.prop_grid[ci].push(idx);
            }
        }
        self.props.push(p);
    }

    fn populate_rooms(&mut self, cat: &Catalog) {
        for ri in 0..self.rooms.len() {
            let room = self.rooms[ri].clone();
            if room.prop_types.is_empty() || room.w < 4 || room.h < 4 {
                continue;
            }
            let mut cands = vec![];
            for y in (room.y + 1)..(room.y + room.h - 1) {
                for x in (room.x + 1)..(room.x + room.w - 1) {
                    cands.push(Cell::new(x, y));
                }
            }
            if !cat.is_centerpiece_room(&room.rtype) {
                let c = room.center;
                cands.retain(|cell| (cell.x - c.x).abs() > 1 && (cell.y - c.y).abs() > 1);
            }
            self.random.shuffle(&mut cands);
            let mut used: HashSet<Cell> = HashSet::new();
            let mut used_surfaces: HashSet<String> = HashSet::new();
            for (index, t) in room.prop_types.iter().enumerate() {
                let id = format!("{}-prop-{}", room.id, index);
                if cat.is_surface_prop(t) {
                    let parents = cat.surface_parent_types(t);
                    let parent = self
                        .props
                        .iter()
                        .find(|o| {
                            o.room == ri
                                && parents.contains(&o.ptype)
                                && !used_surfaces.contains(&o.id)
                        })
                        .cloned();
                    if let Some(p) = parent {
                        used_surfaces.insert(p.id.clone());
                        let half = cat.footprint(t, p.rotation);
                        self.add_prop(Prop {
                            id,
                            cell: p.cell,
                            ptype: t.clone(),
                            room: ri,
                            rotation: p.rotation,
                            mount_y: Some(cat.surface_height(&p.ptype)),
                            solid: cat.is_solid(t),
                            half,
                            offset: (0.0, 0.0),
                            door: None,
                        });
                    }
                    continue;
                }
                let rotation_start = self.random.next_index(4) as i32;
                let mut best: Option<(Cell, i32)> = None;
                let mut best_score = f64::INFINITY;
                for &cell in &cands {
                    if used.contains(&cell) {
                        continue;
                    }
                    for offset in 0..4 {
                        let rotation = (rotation_start + offset) % 4;
                        if !self.room_prop_fits(&room, cell, t, rotation, cat) {
                            continue;
                        }
                        let score = Self::room_prop_preference(&room, cell, t, rotation, cat);
                        if score < best_score {
                            best_score = score;
                            best = Some((cell, rotation));
                        }
                    }
                }
                let Some((cell, rotation)) = best else {
                    continue;
                };
                used.insert(cell);
                self.add_prop(Prop {
                    id,
                    cell,
                    ptype: t.clone(),
                    room: ri,
                    rotation,
                    mount_y: None,
                    solid: cat.is_solid(t),
                    half: cat.footprint(t, rotation),
                    offset: (0.0, 0.0),
                    door: None,
                });
            }
        }
    }

    fn room_prop_fits(
        &self,
        room: &Room,
        cell: Cell,
        t: &str,
        rotation: i32,
        cat: &Catalog,
    ) -> bool {
        if !self.valid_open_cell(cell) || cell == self.start || cell == self.exit {
            return false;
        }
        let fp = cat.footprint(t, rotation);
        let (fx, fy) = (fp.0 as f64, fp.1 as f64);
        let (cx, cy) = (cell.x as f64 + 0.5, cell.y as f64 + 0.5);
        let clearance = 0.12;
        if cx - fx - clearance < room.x as f64 + 1.0
            || cx + fx + clearance > (room.x + room.w) as f64 - 1.0
            || cy - fy - clearance < room.y as f64 + 1.0
            || cy + fy + clearance > (room.y + room.h) as f64 - 1.0
        {
            return false;
        }
        let (sx, sy) = (self.start.x as f32 + 0.5, self.start.y as f32 + 0.5);
        let d = ((cx as f32 - sx) * (cx as f32 - sx) + (cy as f32 - sy) * (cy as f32 - sy)).sqrt()
            as f64;
        if d < 1.2f64.max(fx.max(fy) + DEFAULT_AGENT_RADIUS + 0.5) {
            return false;
        }
        for o in &self.props {
            if !o.solid {
                continue;
            }
            let (ox, oy) = (o.cell.x as f64 + 0.5 + o.offset.0 as f64, o.cell.y as f64 + 0.5 + o.offset.1 as f64);
            if (cx - ox).abs() < fx + o.half.0 as f64 + clearance
                && (cy - oy).abs() < fy + o.half.1 as f64 + clearance
            {
                return false;
            }
        }
        true
    }

    fn room_prop_preference(room: &Room, cell: Cell, t: &str, rotation: i32, cat: &Catalog) -> f64 {
        let c = room.center;
        let dsq = (((cell.x - c.x) * (cell.x - c.x)) + ((cell.y - c.y) * (cell.y - c.y))) as f64;
        if ["carnivalCarousel", "giantTree", "diningTable"].contains(&t)
            && cat.is_centerpiece_room(&room.rtype)
        {
            return dsq;
        }
        if cat.is_walkable_structure(t) {
            return dsq;
        }
        let left = cell.x - (room.x + 1);
        let right = room.x + room.w - 2 - cell.x;
        let top = cell.y - (room.y + 1);
        let bottom = room.y + room.h - 2 - cell.y;
        let edge = left.min(right).min(top.min(bottom));
        if !cat.is_wall_backed(t) {
            return edge as f64 * 20.0;
        }
        let desired = if bottom == edge {
            2
        } else if left == edge {
            1
        } else if right == edge {
            3
        } else {
            0
        };
        edge as f64 * 20.0 + if rotation == desired { 0.0 } else { 1000.0 }
    }

    // -- objectives -------------------------------------------------------------------------

    fn pick_service_lift(&self, from: &HashMap<Cell, i32>, cat: &Catalog) -> Cell {
        let mut lr = Rng::new(&format!("{}:SERVICE-LIFT", self.seed_text));
        let reachable: Vec<Cell> = self
            .floor_cells()
            .into_iter()
            .filter(|c| {
                *c != self.start
                    && !self.in_walkable_structure_room(*c, cat)
                    && from.contains_key(c)
                    && self.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
            })
            .collect();
        if reachable.is_empty() {
            return self.start;
        }
        let min_steps = (self.cols + self.rows) as f64 * 0.35;
        let half_c = self.cols as f64 / 2.0;
        let half_r = self.rows as f64 / 2.0;
        let mut choices: Vec<Vec<Cell>> = vec![];
        for room in self.rooms.iter().filter(|r| !r.lobby()) {
            let cells: Vec<Cell> = reachable
                .iter()
                .cloned()
                .filter(|c| {
                    c.x >= room.x + (room.w as f64 * 0.62).floor() as i32
                        && c.x < room.x + room.w - 1
                        && c.y > room.y
                        && c.y <= room.y + ((room.h as f64 * 0.38).floor() as i32).max(1)
                        && c.x as f64 >= half_c
                        && c.y as f64 <= half_r
                        && from.get(c).copied().unwrap_or(0) as f64 >= min_steps
                })
                .collect();
            if !cells.is_empty() {
                choices.push(cells);
            }
        }
        if !choices.is_empty() {
            let sel = &choices[lr.next_index(choices.len())];
            return sel[lr.next_index(sel.len())];
        }
        let top_right: Vec<Cell> = reachable
            .iter()
            .cloned()
            .filter(|c| c.x as f64 >= half_c && c.y as f64 <= half_r)
            .collect();
        let far = |v: &Vec<Cell>| -> Vec<Cell> {
            v.iter()
                .cloned()
                .filter(|c| from.get(c).copied().unwrap_or(0) as f64 >= min_steps)
                .collect()
        };
        let trf = far(&top_right);
        let all_far = far(&reachable);
        let mut pool = if !trf.is_empty() {
            trf
        } else if !top_right.is_empty() {
            top_right
        } else if !all_far.is_empty() {
            all_far
        } else {
            reachable
        };
        sort_custom(&mut pool, |a, b| (a.x - a.y) > (b.x - b.y));
        let band = ((pool.len() as f64 * 0.12).ceil() as usize).max(1);
        pool[lr.next_index(band)]
    }

    fn far_from_all(c: Cell, zones: &[Cell], min: f32) -> bool {
        zones.iter().all(|z| c.len_to(*z) > min)
    }

    fn place_relays(&mut self, from: &HashMap<Cell, i32>, count: usize, cat: &Catalog) {
        let safe: Vec<Cell> = self.safe_zone.into_iter().collect();
        let mut cands: Vec<Cell> = self
            .floor_cells()
            .into_iter()
            .filter(|c| {
                *c != self.start
                    && !self.in_walkable_structure_room(*c, cat)
                    && from.get(c).copied().unwrap_or(0) > 14
                    && self.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
                    && c.len_to(self.exit) > 4.0
                    && Self::far_from_all(*c, &safe, 5.0)
            })
            .collect();
        self.random.shuffle(&mut cands);
        let mut chosen: Vec<Cell> = vec![];
        for c in cands {
            if chosen.iter().all(|o| c.len_to(*o) >= 7.0) {
                chosen.push(c);
                if chosen.len() == count {
                    break;
                }
            }
        }
        self.relays = chosen;
    }

    fn place_batteries(&mut self, from: &HashMap<Cell, i32>, count: usize, cat: &Catalog) {
        let safe: Vec<Cell> = self.safe_zone.into_iter().collect();
        let mut cands: Vec<Cell> = self
            .floor_cells()
            .into_iter()
            .filter(|c| {
                *c != self.start
                    && !self.in_walkable_structure_room(*c, cat)
                    && from.get(c).copied().unwrap_or(-1) > 5
                    && self.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
                    && c.len_to(self.exit) > 2.0
                    && Self::far_from_all(*c, &self.relays, 2.5)
                    && Self::far_from_all(*c, &safe, 2.5)
            })
            .collect();
        self.random.shuffle(&mut cands);
        let mut chosen: Vec<Cell> = vec![];
        for c in cands {
            if chosen.iter().all(|o| c.len_to(*o) >= 4.5) {
                chosen.push(c);
                if chosen.len() == count {
                    break;
                }
            }
        }
        self.batteries = chosen;
    }

    fn walls_around(&self, c: Cell) -> i32 {
        [(0, 1), (0, -1), (1, 0), (-1, 0)]
            .iter()
            .filter(|(dx, dy)| self.is_wall_cell(c.x + dx, c.y + dy))
            .count() as i32
    }

    fn place_hiding_spots(&mut self, count: usize, cat: &Catalog) {
        let mut rng = Rng::new(&format!("{}:HIDING-SPOTS", self.seed_text));
        let mut taken: HashSet<Cell> = HashSet::from([self.exit, self.start]);
        taken.extend(self.relays.iter());
        taken.extend(self.batteries.iter());
        taken.extend(self.safe_zone.iter());
        let mut in_room = vec![];
        let mut in_corridor = vec![];
        for y in 1..self.rows - 1 {
            for x in 1..self.cols - 1 {
                let c = Cell::new(x, y);
                if !self.can_occupy(x as f64 + 0.5, y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
                    || taken.contains(&c)
                {
                    continue;
                }
                if self.in_walkable_structure_room(c, cat) || self.walls_around(c) != 1 {
                    continue;
                }
                if self.room_at(x as f64, y as f64).is_none() {
                    in_corridor.push(c)
                } else {
                    in_room.push(c)
                }
            }
        }
        let spacing = 4.0f64.max(self.cols.min(self.rows) as f64 / count.max(1) as f64);
        let mut spots: Vec<Cell> = vec![];
        // room buckets in insertion order, keyed by room id
        let mut keys_in_order: Vec<String> = vec![];
        let mut buckets: HashMap<String, Vec<Cell>> = HashMap::new();
        for c in &in_room {
            let r = self.rooms[self.room_at(c.x as f64, c.y as f64).unwrap()]
                .id
                .clone();
            if !buckets.contains_key(&r) {
                keys_in_order.push(r.clone());
            }
            buckets.entry(r).or_default().push(*c);
        }
        let mut room_keys = keys_in_order;
        room_keys.sort();
        for i in (1..room_keys.len()).rev() {
            let j = rng.next_index(i + 1);
            room_keys.swap(i, j);
        }
        let far =
            |pick: Cell, spots: &[Cell]| spots.iter().all(|s| (pick.len_to(*s) as f64) >= spacing);
        while spots.len() < count && !room_keys.is_empty() {
            for rk in room_keys.clone() {
                if spots.len() >= count {
                    break;
                }
                let bucket = buckets.get_mut(&rk).unwrap();
                if bucket.is_empty() {
                    room_keys.retain(|k| *k != rk);
                    continue;
                }
                let pick = bucket[rng.next_index(bucket.len())];
                let ok = far(pick, &spots);
                bucket.retain(|c| *c != pick);
                if ok {
                    spots.push(pick);
                }
            }
        }
        while spots.len() < count && !in_corridor.is_empty() {
            let pick = in_corridor[rng.next_index(in_corridor.len())];
            let ok = far(pick, &spots);
            in_corridor.retain(|c| *c != pick);
            if ok {
                spots.push(pick);
            }
        }
        self.hiding = spots
            .into_iter()
            .map(|c| {
                let wall = [(0, 1), (0, -1), (1, 0), (-1, 0)]
                    .into_iter()
                    .find(|(dx, dy)| self.is_wall_cell(c.x + dx, c.y + dy))
                    .unwrap_or((0, -1));
                Hiding {
                    cell: c,
                    facing: (-wall.0, -wall.1),
                    room: self.room_at(c.x as f64, c.y as f64),
                }
            })
            .collect();
    }

    /// FacilityView3D._spawn_room_lights: three seeded rooms get a switchable ceiling light.
    fn place_room_lights(&mut self) {
        let mut cands: Vec<usize> = (0..self.rooms.len())
            .filter(|i| !self.rooms[*i].lobby() && self.rooms[*i].w >= 5 && self.rooms[*i].h >= 5)
            .collect();
        let mut rng = Rng::new(&format!("{}:TOGGLEABLE-ROOM-LIGHTS", self.seed_text));
        let count = cands.len().min(3);
        for _ in 0..count {
            let chosen = rng.next_index(cands.len());
            let ri = cands.remove(chosen);
            let r = &self.rooms[ri];
            let color = match r.region.as_str() {
                "maintenance" => (0.941, 0.651, 0.353),
                "arcade" | "circus" => (0.729, 0.514, 0.910),
                "garden" => (0.533, 0.827, 0.604),
                "nether" => (0.882, 0.416, 0.322),
                _ => (0.851, 0.890, 0.831),
            };
            let mut switch_cell = Cell::new(r.x + 1, r.y + 1);
            if self.taken_cells().contains(&switch_cell) {
                switch_cell = r.center;
            }
            self.room_lights.push(RoomLight {
                room: ri,
                center: r.center,
                switch_cell,
                color,
                range: (r.w.min(r.h) as f32 * 0.55).clamp(3.5, 6.0),
                energy: 1.25,
            });
        }
    }

    fn taken_cells(&self) -> HashSet<Cell> {
        let mut t: HashSet<Cell> = HashSet::from([self.start, self.exit]);
        t.extend(self.relays.iter());
        t.extend(self.batteries.iter());
        t.extend(self.hiding.iter().map(|h| h.cell));
        t.extend(self.room_lights.iter().map(|l| l.switch_cell));
        t.extend(self.pickups.iter().map(|p| p.0));
        t
    }

    /// FacilityView3D._spawn_tool_pickups: one of every collectible but the flashlight.
    fn place_pickups(&mut self) {
        let taken = self.taken_cells();
        let mut avail: Vec<Cell> = self
            .floor_cells()
            .into_iter()
            .filter(|c| {
                !taken.contains(c)
                    && c.len_to(self.start) >= 3.0
                    && self.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
            })
            .collect();
        let mut rng = Rng::new(&format!("{}:EQUIPMENT-PICKUPS", self.seed_text));
        for item in PICKUP_ORDER {
            if avail.is_empty() {
                break;
            }
            let i = rng.next_index(avail.len());
            self.pickups.push((avail.remove(i), item));
        }
    }

    fn build_hunter_grid(&mut self) {
        let mut open = vec![false; (self.cols * self.rows) as usize];
        for y in 0..self.rows {
            for x in 0..self.cols {
                // a hunter never walks into a drop: it has no floor
                open[(y * self.cols + x) as usize] = !self.is_drop(Cell::new(x, y))
                    && self.body_fits(x as f32 + 0.5, y as f32 + 0.5, DEFAULT_AGENT_RADIUS as f32);
            }
        }
        self.hunter_open = open;
    }

    /// place_hunter_spawn: >20 steps from the start, in the start's open component, 7 cells
    /// from safe zones and 4 from the lift; drawn from the main stream.
    fn place_hunter_spawn(&mut self, from: &HashMap<Cell, i32>) -> Cell {
        let safe: Vec<Cell> = self.safe_zone.into_iter().collect();
        let comp = self.component(self.start);
        let cands: Vec<Cell> = self
            .floor_cells()
            .into_iter()
            .filter(|c| {
                *c != self.start
                    && from.get(c).copied().unwrap_or(0) > 20
                    && comp.contains(c)
                    && self.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.25)
                    && Self::far_from_all(*c, &safe, 7.0)
                    && c.len_to(self.exit) > 4.0
            })
            .collect();
        if cands.is_empty() {
            return self.exit;
        }
        cands[self.random.next_index(cands.len())]
    }

    fn component(&self, origin: Cell) -> HashSet<Cell> {
        let mut seen = HashSet::new();
        let Some(s) = self.nearest_open(origin.x as f32 + 0.5, origin.y as f32 + 0.5, 0.25) else {
            return seen;
        };
        let s = Cell::new(s.0 as i32, s.1 as i32);
        seen.insert(s);
        let mut q = VecDeque::from([s]);
        while let Some(c) = q.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if !seen.contains(&n) && self.can_occupy(n.x as f64 + 0.5, n.y as f64 + 0.5, 0.25) {
                    seen.insert(n);
                    q.push_back(n);
                }
            }
        }
        seen
    }

    // -- navigation -------------------------------------------------------------------------

    pub fn hunter_open(&self, c: Cell) -> bool {
        c.x >= 0
            && c.y >= 0
            && c.x < self.cols
            && c.y < self.rows
            && self.hunter_open[(c.y * self.cols + c.x) as usize]
    }

    /// 8-way A* with an octile heuristic and no corner cutting (AStarGrid2D,
    /// DIAGONAL_MODE_ONLY_IF_NO_OBSTACLES).
    pub fn find_path(&self, from: Cell, to: Cell) -> Vec<Cell> {
        if !self.hunter_open(from) || !self.hunter_open(to) {
            return vec![];
        }
        #[derive(PartialEq)]
        struct Node(f32, u32);
        impl Eq for Node {}
        impl PartialOrd for Node {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Node {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.0.partial_cmp(&self.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        }
        let w = self.cols;
        let h = |c: Cell| {
            let dx = (c.x - to.x).abs() as f32;
            let dy = (c.y - to.y).abs() as f32;
            dx.max(dy) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dy)
        };
        let n = (self.cols * self.rows) as usize;
        let mut g = vec![f32::INFINITY; n];
        let mut came = vec![u32::MAX; n];
        let idx = |c: Cell| (c.y * w + c.x) as usize;
        let mut open = BinaryHeap::new();
        g[idx(from)] = 0.0;
        open.push(Node(h(from), idx(from) as u32));
        while let Some(Node(_, ci)) = open.pop() {
            let c = Cell::new(ci as i32 % w, ci as i32 / w);
            if c == to {
                let mut path = vec![c];
                let mut i = ci;
                while came[i as usize] != u32::MAX {
                    i = came[i as usize];
                    path.push(Cell::new(i as i32 % w, i as i32 / w));
                }
                path.reverse();
                return path;
            }
            let gc = g[ci as usize];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nb = Cell::new(c.x + dx, c.y + dy);
                    if !self.hunter_open(nb) {
                        continue;
                    }
                    if dx != 0
                        && dy != 0
                        && (!self.hunter_open(Cell::new(c.x + dx, c.y))
                            || !self.hunter_open(Cell::new(c.x, c.y + dy)))
                    {
                        continue;
                    }
                    let ng = gc
                        + if dx != 0 && dy != 0 {
                            std::f32::consts::SQRT_2
                        } else {
                            1.0
                        };
                    if ng < g[idx(nb)] {
                        g[idx(nb)] = ng;
                        came[idx(nb)] = ci;
                        open.push(Node(ng + h(nb), idx(nb) as u32));
                    }
                }
            }
        }
        vec![]
    }

    /// has_line_of_sight: samples the segment at ceil(distance*6) points; walls only.
    pub fn has_line_of_sight(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        let steps = ((d * 6.0).ceil() as i32).max(1);
        (0..=steps).all(|i| {
            let t = i as f32 / steps as f32;
            self.is_floor(
                (a.0 + (b.0 - a.0) * t) as f64,
                (a.1 + (b.1 - a.1) * t) as f64,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::regions::parse_file;

    fn door_floor() -> Facility {
        let cat = Catalog::load();
        let mut f = Facility::generate("DOOR-1", &Options::default(), &cat);
        let cell = f.floor_cells()[0];
        f.props.push(Prop {
            id: "door".into(),
            cell,
            ptype: "normalRoomDoor".into(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (0.5, 0.1),
            offset: (0.0, 0.0),
            door: Some(DoorDraw { span: 1, offset: [0.0, 0.0] }),
        });
        f
    }

    /// A door nobody ticked is open floor; once ticked it is closed and blocks. A body beside it (the runner) does
    /// not open it; only `toggle_door` does, and it closes again the same way.
    #[test]
    fn a_closed_door_blocks_until_it_is_toggled_open() {
        let mut f = door_floor();
        let cell = f.props.last().unwrap().cell;
        let (cx, cy) = cell.center();
        let far = (cx + 30.0, cy + 30.0);
        assert!(!f.door_blocks(cx, cy, 0.2), "an unticked door is open");
        assert_eq!(f.door_open_amount(cell), 0.0, "an unticked door is drawn shut until the first tick");
        assert_eq!(f.toggle_door(cell, &[]), None, "an unticked door cannot be toggled");
        f.update_doors(&[], &[far], 0.0);
        assert_eq!(f.door_open_amount(cell), 0.0);
        assert!(!f.door_is_open(cell));
        assert!(f.door_blocks(cx, cy, 0.2), "a closed door blocks its slab");
        assert!(!f.door_blocks(cx, cy + 0.6, 0.2), "clear of the slab");
        // the runner stands at the door for a second: nothing opens
        let runner = (cx, cy + 0.8);
        for _ in 0..10 {
            f.update_doors(&[], &[runner], 0.1);
        }
        assert_eq!(f.door_open_amount(cell), 0.0, "the runner never opens a door by walking up");
        assert!(f.door_blocks(cx, cy, 0.2));
        assert_eq!(f.toggle_door(cell, &[runner]), Some(DoorToggle::Opened));
        f.update_doors(&[], &[runner], 0.1);
        assert!(f.door_open_amount(cell) > 0.0);
        f.update_doors(&[], &[runner], 0.5);
        assert_eq!(f.door_open_amount(cell), 1.0);
        assert!(!f.door_blocks(cx, cy, 0.2), "an open door is passable");
        for _ in 0..30 {
            f.update_doors(&[], &[runner], 0.1);
        }
        assert_eq!(f.door_open_amount(cell), 1.0, "an open door stays open");
        assert_eq!(f.toggle_door(cell, &[runner]), Some(DoorToggle::Closed));
        f.update_doors(&[], &[runner], 0.5);
        assert_eq!(f.door_open_amount(cell), 0.0);
        assert!(f.door_blocks(cx, cy, 0.2));
    }

    /// A half-swung leaf is solid until the opening it leaves is wider than the body: a fat hunter needs the door
    /// further open than the runner does.
    #[test]
    fn a_half_open_door_blocks_until_the_gap_fits_the_body() {
        let mut f = door_floor();
        let cell = f.props.last().unwrap().cell;
        let (cx, cy) = cell.center();
        f.update_doors(&[], &[], 0.0);
        f.toggle_door(cell, &[]);
        let (runner_r, hunter_r) = (0.24, 0.33);
        f.update_doors(&[], &[], 0.1); // open 0.4: the leaf covers most of the doorway
        assert!(f.door_open_amount(cell) < 0.5);
        assert!(f.door_blocks(cx, cy, runner_r), "a barely open door still blocks the runner");
        f.update_doors(&[], &[], 0.075); // open 0.7: the gap fits the runner, not the hunter
        assert!(!f.door_blocks(cx, cy, runner_r), "the runner fits once the gap is wide enough");
        assert!(f.door_blocks(cx, cy, hunter_r), "the hunter needs more of a gap");
        f.update_doors(&[], &[], 0.5);
        assert!(!f.door_blocks(cx, cy, hunter_r), "a fully open door passes the hunter");
    }

    /// A leaf never swings shut through a body: pressing close with someone in its sweep is refused, and a leaf
    /// closing when someone steps into its sweep swings back open until they clear.
    #[test]
    fn a_door_never_closes_through_a_body_in_its_sweep() {
        let mut f = door_floor();
        let cell = f.props.last().unwrap().cell;
        let (cx, cy) = cell.center();
        f.update_doors(&[], &[], 0.0);
        f.toggle_door(cell, &[]);
        f.update_doors(&[], &[], 1.0);
        assert_eq!(f.door_open_amount(cell), 1.0);
        let in_doorway = (cx, cy + 0.2);
        assert_eq!(f.toggle_door(cell, &[in_doorway]), Some(DoorToggle::Blocked));
        assert!(f.door_is_open(cell), "a refused close leaves it open");
        let clear = (cx, cy + 1.0);
        assert_eq!(f.toggle_door(cell, &[clear]), Some(DoorToggle::Closed));
        f.update_doors(&[], &[clear], 0.1);
        let swung = f.door_open_amount(cell);
        assert!(swung < 1.0 && swung > 0.0, "the leaf is mid swing");
        // someone steps into the sweep: the leaf reverses
        f.update_doors(&[], &[in_doorway], 0.1);
        assert!(f.door_open_amount(cell) > swung);
        f.update_doors(&[], &[in_doorway], 1.0);
        assert_eq!(f.door_open_amount(cell), 1.0);
        // once they clear, the close the runner asked for carries on
        f.update_doors(&[], &[clear], 1.0);
        assert_eq!(f.door_open_amount(cell), 0.0);
    }

    /// Hunters (the openers) pull a closed door open when they reach it and leave it open.
    #[test]
    fn an_opener_at_the_door_opens_it_and_leaves_it_open() {
        let mut f = door_floor();
        let cell = f.props.last().unwrap().cell;
        let (cx, cy) = cell.center();
        let far = (cx + 30.0, cy + 30.0);
        f.update_doors(&[far], &[far], 0.0);
        assert_eq!(f.door_open_amount(cell), 0.0);
        let hunter = (cx, cy + 1.0);
        f.update_doors(&[hunter], &[hunter], 0.5);
        assert_eq!(f.door_open_amount(cell), 1.0);
        f.update_doors(&[far], &[far], 2.0);
        assert_eq!(f.door_open_amount(cell), 1.0, "it stays open behind them");
    }

    /// A door first ticked with a body in its sweep starts open, so nobody is born inside the leaf.
    #[test]
    fn a_door_registered_around_a_body_starts_open() {
        let mut f = door_floor();
        let cell = f.props.last().unwrap().cell;
        let (cx, cy) = cell.center();
        f.update_doors(&[], &[(cx, cy)], 0.0);
        assert_eq!(f.door_open_amount(cell), 1.0);
        assert!(f.door_is_open(cell));
    }

    /// A temporary region with a geometry block sets its room side range; without one the game-wide range stays.
    #[test]
    fn a_region_geometry_block_sets_the_room_sizes() {
        let text = r#"{"schema":"lr.region/1","id":"tmp_tall","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[],"geometry":{"room_min":8,"room_max":14}}"#;
        let tall = parse_file("tmp_tall.json", text).unwrap();
        assert_eq!(room_sizes_for(Some(&tall)), (8, 14));
        assert_eq!(room_sizes_for(None), (MIN_AUTHORED_ROOM_SIZE, MAX_AUTHORED_ROOM_SIZE));
        // a 40-cell sector in the 0.5..0.64 class: the geometry range pins the side to 14, the constants give 20..=25
        assert_eq!(Facility::size_range(40, (0.5, 0.64), room_sizes_for(Some(&tall))), (14, 14));
        assert_eq!(Facility::size_range(40, (0.5, 0.64), room_sizes_for(None)), (20, 25));
    }

    /// A new floor is all ground floor at 0.0 m; a catwalk set on one cell changes that cell only.
    #[test]
    fn a_new_facility_is_all_floor_and_a_catwalk_moves_only_its_own_cell() {
        let cat = Catalog::load();
        let mut f = Facility::generate("DECK-1", &Options::default(), &cat);
        let cells: Vec<Cell> = (0..f.rows)
            .flat_map(|y| (0..f.cols).map(move |x| Cell::new(x, y)))
            .collect();
        for &c in &cells {
            assert_eq!(f.walk_surface_at(c), WalkSurface { kind: DeckKind::Floor, height: 0.0 });
            assert_eq!(f.height_at(c), 0.0);
        }
        let deck = f.floor_cells()[0];
        f.set_walk_surface(deck, DeckKind::Catwalk, 4.0);
        assert_eq!(f.walk_surface_at(deck), WalkSurface { kind: DeckKind::Catwalk, height: 4.0 });
        assert_eq!(f.height_at(deck), 4.0);
        for &c in cells.iter().filter(|&&c| c != deck) {
            assert_eq!(f.walk_surface_at(c), WalkSurface { kind: DeckKind::Floor, height: 0.0 }, "{c:?} moved");
        }
        f.set_walk_surface(deck, DeckKind::Floor, 0.0);
        assert_eq!(f.walk_surface_at(deck), WalkSurface::default());
    }

    fn deck_fixture() -> (Facility, Cell, Cell) {
        let cat = Catalog::load();
        let mut f = Facility::generate("DECK-2", &Options::default(), &cat);
        let cells = f.floor_cells();
        let deck = cells
            .iter()
            .copied()
            .find(|&c| {
                let (x, y) = c.center();
                f.body_fits(x, y, 0.2)
            })
            .expect("a free floor cell");
        let floor = cells
            .iter()
            .copied()
            .find(|&c| c != deck && f.body_fits(c.center().0, c.center().1, 0.2))
            .expect("a second free floor cell");
        f.set_walk_surface(deck, DeckKind::Catwalk, 4.0);
        (f, deck, floor)
    }

    #[test]
    fn a_body_on_the_floor_is_blocked_by_a_deck_cell() {
        let (f, deck, floor) = deck_fixture();
        assert!(!f.body_fits_at(deck.center().0, deck.center().1, 0.2, 0.0));
        assert!(f.body_fits_at(floor.center().0, floor.center().1, 0.2, 0.0));
    }

    #[test]
    fn a_body_on_the_deck_is_not_blocked_by_deck_cells() {
        let (f, deck, _) = deck_fixture();
        assert!(f.body_fits_at(deck.center().0, deck.center().1, 0.2, 4.0));
    }

    #[test]
    fn a_body_on_the_deck_is_blocked_by_a_floor_cell_at_zero() {
        let (f, _, floor) = deck_fixture();
        assert!(!f.body_fits_at(floor.center().0, floor.center().1, 0.2, 4.0));
    }

    const WORLD: &str = "1426541270";

    /// The level 0 maze chunks around the origin, each built the way the live route builds it.
    fn maze_chunks() -> Vec<(ChunkKey, Facility)> {
        let cat = Catalog::load();
        let mut out = vec![];
        for cx in -8..8 {
            for cy in -8..8 {
                let key = ChunkKey { cx, cy, level: 0 };
                if crate::world::route::regions::region_for(WORLD, cx, cy, 0) != "maze" {
                    continue;
                }
                if let Some((f, _)) = crate::world::route::live::chunk(WORLD, key, &cat) {
                    out.push((key, f));
                }
            }
        }
        out
    }

    fn open_ground(f: &Facility) -> Vec<bool> {
        let mut g = vec![false; (f.cols * f.rows) as usize];
        for y in 0..f.rows {
            for x in 0..f.cols {
                g[(y * f.cols + x) as usize] = !f.is_hole(x, y) && f.can_occupy(x as f64 + 0.5, y as f64 + 0.5, 0.25);
            }
        }
        g
    }

    #[test]
    fn catwalk_decks_stand_on_clear_ground_and_every_link_touches_a_deck() {
        let height = catwalk::params().height;
        let mut with_decks = 0;
        for (key, mut f) in maze_chunks() {
            let before = open_ground(&f);
            let set = f.apply_catwalks(WORLD, key, "maze");
            if set == 0 {
                assert!(f.catwalk_links.is_empty());
                continue;
            }
            with_decks += 1;
            assert!(!f.catwalk_links.is_empty() && f.catwalk_links.len() <= catwalk_links::rules().per_chunk);
            let decks: Vec<Cell> = (0..f.rows)
                .flat_map(|y| (0..f.cols).map(move |x| Cell::new(x, y)))
                .filter(|c| f.walk_surface_at(*c).kind == DeckKind::Catwalk)
                .collect();
            assert_eq!(decks.len(), set);
            for c in &decks {
                assert!(before[(c.y * f.cols + c.x) as usize], "a deck over a wall or prop at {c:?}");
                assert_eq!(f.height_at(*c), height);
                // a runner up on the deck fits there; one on the floor does not
                assert!(f.body_fits_at(c.center().0, c.center().1, 0.2, height));
                assert!(!f.body_fits_at(c.center().0, c.center().1, 0.2, 0.0));
            }
            for l in &f.catwalk_links {
                assert_eq!(f.walk_surface_at(l.cell).kind, DeckKind::Floor, "a link starts on the ground");
                assert!(decks.iter().any(|d| (d.x - l.cell.x).abs() + (d.y - l.cell.y).abs() == 1));
                assert_eq!(l.deck_height, height);
            }
            // lifting the decks cut no ground floor in two
            let mut after = before.clone();
            for c in &decks {
                after[(c.y * f.cols + c.x) as usize] = false;
            }
            assert!(Facility::ground_pieces(&after, f.cols) <= Facility::ground_pieces(&before, f.cols));
        }
        assert!(with_decks > 0, "no maze chunk got a catwalk");
    }

    #[test]
    fn catwalks_are_deterministic_and_follow_the_data_regions() {
        let (key, f) = maze_chunks().into_iter().find(|(k, f)| {
            let mut g = f.clone();
            g.apply_catwalks(WORLD, *k, "maze") > 0
        }).expect("a chunk with a catwalk");
        let (mut a, mut b) = (f.clone(), f.clone());
        assert_eq!(a.apply_catwalks(WORLD, key, "maze"), b.apply_catwalks(WORLD, key, "maze"));
        assert_eq!(a.catwalk_links, b.catwalk_links);
        // a region the data does not list, and any level but 0, get none
        // (live::chunk already decked this chunk as a maze one, so start from the bare chunk)
        let mut c = f.clone();
        c.walk_surface.clear();
        assert_eq!(c.apply_catwalks(WORLD, key, "mansion"), 0);
        assert!(c.walk_surface.is_empty());
        assert_eq!(c.apply_catwalks(WORLD, ChunkKey { level: -1, ..key }, "maze"), 0);
    }

    #[test]
    fn the_link_clearance_is_data_and_changing_it_changes_which_catwalks_survive() {
        let std = catwalk_links::rules();
        assert_eq!(std.link_clearance, 0.45, "the shipped value");
        let run = |clearance: f64| -> Vec<(ChunkKey, usize, Vec<Cell>)> {
            let rules = catwalk_links::CatwalkRules { link_clearance: clearance, ..std };
            maze_chunks()
                .into_iter()
                .map(|(k, mut f)| {
                    f.walk_surface.clear();
                    let decks = f.apply_catwalks_with(WORLD, k, "maze", &rules);
                    (k, decks, f.catwalk_links.iter().map(|l| l.cell).collect())
                })
                .collect()
        };
        let shipped = run(std.link_clearance);
        assert_eq!(shipped, run(std.link_clearance), "deterministic for one value");
        assert!(shipped.iter().any(|(_, d, _)| *d > 0), "no maze chunk got a catwalk");
        assert_ne!(shipped, run(0.0), "no clearance admits cells the shipped value refuses");
        assert_ne!(shipped, run(0.5), "a whole-cell clearance refuses cells the shipped value admits");
    }

    #[test]
    fn merging_chunks_keeps_their_decks_and_links_shifted() {
        let (key, mut f) = maze_chunks().into_iter().find(|(k, f)| {
            let mut g = f.clone();
            g.apply_catwalks(WORLD, *k, "maze") > 0
        }).expect("a chunk with a catwalk");
        f.apply_catwalks(WORLD, key, "maze");
        let deck = (0..f.rows)
            .flat_map(|y| (0..f.cols).map(move |x| Cell::new(x, y)))
            .find(|c| f.walk_surface_at(*c).kind == DeckKind::Catwalk)
            .unwrap();
        let link = f.catwalk_links[0];
        let m = Facility::merge(vec![(f.clone(), 45, 45, 0)], 135, 135, vec!["maze".into()], WORLD);
        assert_eq!(m.walk_surface_at(Cell::new(deck.x + 45, deck.y + 45)), f.walk_surface_at(deck));
        assert_eq!(m.walk_surface_at(deck).kind, DeckKind::Floor);
        assert!(m.catwalk_links.iter().any(|l| l.cell == Cell::new(link.cell.x + 45, link.cell.y + 45)));
    }

    #[test]
    fn a_maze_variant_is_recorded_and_thins_only_what_the_data_says() {
        let mut seen_variant = false;
        for (key, mut f) in maze_chunks() {
            let doors = f.props.iter().filter(|p| p.door.is_some()).count();
            let (pickups, relays, batteries) = (f.pickups.len(), f.relays.len(), f.batteries.len());
            let v = f.apply_maze_variant(WORLD, key);
            assert_eq!(f.maze_variant.as_deref(), v.map(|v| v.id.as_str()));
            assert_eq!(f.maze_variant.as_deref(), maze_regions::variant_at(WORLD, key.cx, key.cy, 0).map(|v| v.id.as_str()));
            assert_eq!(f.props.iter().filter(|p| p.door.is_some()).count(), doors);
            assert_eq!((f.relays.len(), f.batteries.len()), (relays, batteries));
            match v {
                None => assert_eq!(f.pickups.len(), pickups),
                Some(v) => {
                    seen_variant = true;
                    assert!(f.pickups.len() <= pickups);
                    if v.spawn_rules.loot_scale >= 1.0 {
                        assert_eq!(f.pickups.len(), pickups);
                    }
                }
            }
        }
        assert!(seen_variant, "no maze chunk got a variant");
        // not level 0: never a variant
        let cat = Catalog::load();
        let mut f = Facility::generate("V", &Options::default(), &cat);
        assert!(f.apply_maze_variant(WORLD, ChunkKey { cx: 0, cy: 0, level: -1 }).is_none());
        assert!(f.maze_variant.is_none());
    }

    #[test]
    fn body_fits_at_y_matches_body_fits_at_on_level_zero_and_minus_one() {
        use crate::world::vertical::STOREY;
        let cat = Catalog::load();
        for level in [0, -1] {
            let mut f = Facility::generate("FITY", &Options::default(), &cat);
            f.level = level;
            let floor_y = level as f32 * STOREY;
            let mut checked = 0;
            for cy in 0..f.rows.min(24) {
                for cx in 0..f.cols.min(24) {
                    let (x, z) = (cx as f32 + 0.5, cy as f32 + 0.5);
                    for h in [0.0_f32, 0.75] {
                        assert_eq!(
                            f.body_fits_at_y(x, z, floor_y + h, 0.3),
                            f.body_fits_at(x, z, 0.3, h),
                            "level {level} cell ({cx},{cy}) h {h}"
                        );
                        checked += 1;
                    }
                }
            }
            assert!(checked > 0);
        }
    }

    /// A generated facility on `level` with one stair of `shape` (run (0, 1)) placed in a wall-free block, and the stair.
    fn with_stair(level: i32, dir: i32, shape: crate::world::flight::FlightShape) -> (Facility, Stair) {
        let cat = Catalog::load();
        let mut f = Facility::generate("DIV-1", &Options::default(), &cat);
        f.level = level;
        // A generated maze has no wall-free block of a flight's size, so carve one: the flight's block becomes open
        // floor with no props, and the rest of the facility stays as generated.
        let (ax, az) = (8, 8);
        for z in az - 2..az + 12 {
            for x in ax - 2..ax + 8 {
                f.wall[z as usize][x as usize] = false;
            }
        }
        f.props.retain(|p| !((ax - 3..ax + 9).contains(&p.cell.x) && (az - 3..az + 13).contains(&p.cell.y)));
        let upper = Stair::stairs(Cell::new(ax, az), -1, (0, 1), shape, false);
        let s = if dir < 0 {
            upper
        } else {
            Stair::stairs(upper.foot(), 1, (0, 1), shape, false)
        };
        f.stairs.push(s);
        f.rebuild_grids();
        (f, s)
    }

    #[test]
    fn divider_blocks_only_the_switchback_divider() {
        use crate::world::flight::FlightShape::*;
        let (f, s) = with_stair(0, -1, Switchback);
        let seg = s.dividers(0)[0];
        let mid = ((seg.a.0 + seg.b.0) * 0.5, (seg.a.1 + seg.b.1) * 0.5);
        assert!(f.divider_blocks(mid.0, mid.1, 0.2));
        // half a metre to either side is a lane, not the divider (the divider runs along z for run (0, 1))
        assert!(!f.divider_blocks(mid.0 - 0.5, mid.1, 0.2));
        assert!(!f.divider_blocks(mid.0 + 0.5, mid.1, 0.2));
        assert!(!f.body_fits(mid.0, mid.1, 0.2) && !f.body_fits_at_y(mid.0, mid.1, 0.0, 0.2));
        // a facility without stairs, and the other shapes, have no divider
        let cat = Catalog::load();
        let plain = Facility::generate("DIV-1", &Options::default(), &cat);
        assert!(!plain.divider_blocks(mid.0, mid.1, 0.2));
        for shape in [Straight, Spiral] {
            let (g, t) = with_stair(0, -1, shape);
            assert!(t.dividers(0).is_empty());
            let c = t.cell.center();
            for (dx, dz) in [(0.0, 0.0), (0.5, 1.0), (1.0, 2.0), (1.0, 3.0)] {
                assert!(!g.divider_blocks(c.0 + dx, c.1 + dz, 0.2), "{shape:?} {dx} {dz}");
            }
        }
    }

    #[test]
    fn a_body_cannot_cross_between_lanes() {
        use crate::world::flight::FlightShape::Switchback;
        for (level, dir) in [(0, -1), (-1, 1)] {
            let (f, s) = with_stair(level, dir, Switchback);
            let seg = s.dividers(level)[0];
            let (dx, dz) = (seg.b.0 - seg.a.0, seg.b.1 - seg.a.1);
            let len = (dx * dx + dz * dz).sqrt();
            let (nx, nz) = (-dz / len, dx / len);
            let r = 0.05;
            let mut crossings = 0;
            for i in 0..=20 {
                let t = i as f32 / 20.0;
                let (px, pz) = (seg.a.0 + dx * t, seg.a.1 + dz * t);
                // a sideways walk from one lane to the other in 0.02 m steps: some step must be blocked
                let blocked = (-25..=25).any(|k| {
                    let d = k as f32 * 0.02;
                    f.divider_blocks(px + nx * d, pz + nz * d, r)
                });
                assert!(blocked, "level {level} u-sample {i}: a body crossed the divider");
                // and the blocked samples form one unbroken band, so it cannot hop over
                let band: Vec<bool> = (-25..=25).map(|k| f.divider_blocks(px + nx * k as f32 * 0.02, pz + nz * k as f32 * 0.02, r)).collect();
                let first = band.iter().position(|b| *b).unwrap();
                let last = band.iter().rposition(|b| *b).unwrap();
                assert!(band[first..=last].iter().all(|b| *b), "level {level} u-sample {i}: gap in the band");
                crossings += 1;
            }
            assert_eq!(crossings, 21);
        }
    }

    #[test]
    fn body_at_wrong_height_over_a_well_is_blocked() {
        use crate::world::flight::FlightShape::Switchback;
        use crate::world::vertical::STOREY;
        let (f, s) = with_stair(0, -1, Switchback);
        let mut checked = 0;
        for (x, z) in s.walk_path(0, 0.25) {
            let cell = Cell::new(x.floor() as i32, z.floor() as i32);
            let Some(rel) = s.surface_rel(0, x, z) else { continue };
            if !f.is_hole(cell.x, cell.y) {
                continue;
            }
            assert!(!f.body_fits_at_y(x, z, rel + 2.0, 0.2), "({x}, {z}) 2 m above the surface is over a hole");
            assert!(!f.body_fits_at_y(x, z, rel - 2.0, 0.2), "({x}, {z}) 2 m below the surface is over a hole");
            let _ = STOREY;
            checked += 1;
        }
        assert!(checked > 3, "only {checked} well samples");
    }

    #[test]
    fn body_on_the_flight_at_its_surface_fits() {
        use crate::world::flight::FlightShape::{Spiral, Straight, Switchback};
        for shape in [Switchback, Straight, Spiral] {
            for (level, dir) in [(0, -1), (-1, 1)] {
                let (f, s) = with_stair(level, dir, shape);
                let mut checked = 0;
                for (x, z) in s.walk_path(level, 0.25) {
                    let Some(rel) = s.surface_rel(level, x, z) else { continue };
                    let cell = Cell::new(x.floor() as i32, z.floor() as i32);
                    if !f.well_cells_all().contains(&cell) {
                        continue;
                    }
                    // The far end of the walk is the other storey's floor: that body stands in the neighbour's foot cell
                    // (the label flips there), which this facility does not own, so it is not this level's fit to check.
                    if rel.abs() > crate::world::vertical::STOREY - FLIGHT_FIT_TOL {
                        continue;
                    }
                    let y_abs = level as f32 * crate::world::vertical::STOREY + rel;
                    assert!(f.body_fits_at_y(x, z, y_abs, 0.1), "{shape:?} level {level}: ({x}, {z}) at {rel} on the flight");
                    checked += 1;
                }
                assert!(checked > 3, "{shape:?} level {level}: only {checked} samples");
            }
        }
    }

    #[test]
    fn well_cells_all_matches_each_stairs_cells() {
        use crate::world::flight::FlightShape::*;
        let (mut f, s) = with_stair(0, -1, Switchback);
        let mut want = s.well_cells();
        want.sort();
        want.dedup();
        assert_eq!(f.well_cells_all(), want);
        for c in &want {
            assert!(f.open_below(*c) && !f.open_above(*c));
        }
        assert!(!f.open_below(Cell::new(0, 0)));
        // a second stair on the same facility adds its cells
        let other = Stair::stairs(Cell::new(s.cell.x + 4, s.cell.y), -1, (0, 1), Straight, false);
        f.stairs.push(other);
        let mut both: Vec<Cell> = s.well_cells().into_iter().chain(other.well_cells()).collect();
        both.sort();
        both.dedup();
        assert_eq!(f.well_cells_all(), both);
        // the lower level's record opens upwards
        let (g, t) = with_stair(-1, 1, Switchback);
        for c in t.well_cells() {
            assert!(g.open_above(c) && !g.open_below(c));
        }
    }

    #[test]
    fn a_stair_opens_its_well_on_both_levels_and_merge_keeps_level_and_shifts_it() {
        use crate::world::flight::FlightShape;
        let cat = Catalog::load();
        for (level, dir) in [(0, -1), (-1, 1)] {
            let mut part = Facility::generate("STAIR-1", &Options::default(), &cat);
            part.level = level;
            let (cell, run) = (Cell::new(20, 20), (0, 1));
            let upper = Stair::stairs(cell, -1, run, FlightShape::Switchback, false);
            let foot = upper.foot();
            let s = if dir < 0 { upper } else { Stair::stairs(foot, 1, run, FlightShape::Switchback, false) };
            part.stairs.push(s);
            part.rebuild_grids();
            for c in s.well_cells() {
                assert!(part.is_hole(c.x, c.y), "level {level}: {c:?} is a hole");
            }
            let (x, y) = s.well_cells()[0].center();
            assert!(part.on_stair_flight(x, y), "level {level}: the well is a tread for the flight");
            let merged = Facility::merge(vec![(part, 3, 2, 0)], 60, 60, vec!["maze".into()], "STAIR-1");
            assert_eq!(merged.level, level);
            assert_eq!(merged.stairs, vec![s.shifted(3, 2)]);
        }
    }

    /// A generated facility with a 24x24 block of plain open floor (cells 4..28) and no props, lockers or walls in it.
    fn open_block() -> Facility {
        let cat = Catalog::load();
        let mut f = Facility::generate("DROP-1", &Options::default(), &cat);
        for z in 4..28usize {
            for x in 4..28usize {
                f.wall[z][x] = false;
            }
        }
        let inside = |c: Cell| (3..29).contains(&c.x) && (3..29).contains(&c.y);
        f.props.retain(|p| !inside(p.cell));
        f.hiding.retain(|h| !inside(h.cell));
        f.rebuild_grids();
        f
    }

    /// The `n` by `n` block of cells from `(x, y)`, sorted as `set_drop_cells` stores them.
    fn square(x: i32, y: i32, n: i32) -> Vec<Cell> {
        let mut v: Vec<Cell> = (0..n).flat_map(|dy| (0..n).map(move |dx| Cell::new(x + dx, y + dy))).collect();
        v.sort();
        v
    }

    #[test]
    fn a_drop_cell_is_enterable_but_not_walkable_ground() {
        use crate::world::vertical::STOREY;
        let mut f = open_block();
        f.level = -1;
        let drops = square(14, 14, 2);
        f.set_drop_cells(&drops);
        f.rebuild_grids();
        assert_eq!(f.drop_cells(), &drops[..], "sorted, as set");
        for c in &drops {
            assert!(f.is_drop(*c) && f.is_hole(c.x, c.y) && f.open_below(*c), "{c:?} is a hole");
            assert!(!f.is_wall_cell(c.x, c.y), "{c:?}: a drop is open air, not a wall");
            assert!(!f.open_above(*c) && !f.is_air_above(*c));
        }
        assert!(!f.is_drop(Cell::new(13, 14)) && !f.is_hole(13, 14));
        // a body at or above the floor may walk in: the hole rule does not keep it out, no wall does either
        let (x, z) = Cell::new(14, 14).center();
        assert!(f.body_fits_at(x, z, 0.3, 0.0));
        assert!(f.body_fits_at_y(x, z, f.level as f32 * STOREY, 0.3));
        let edge = Cell::new(13, 14).center();
        assert!(f.body_fits_at(edge.0 + 0.45, edge.1, 0.3, 0.0), "straddling the lip of the drop");
        // once below the floor it is inside the void and stays in it: only the drop cells let it be there
        let below = f.level as f32 * STOREY - 2.0;
        assert!(f.body_fits_at_y(x, z, below, 0.3), "falling through the void");
        let (gx, gz) = Cell::new(10, 10).center();
        assert!(!f.body_fits_at_y(gx, gz, below, 0.3), "a body that deep beside the void is inside the ground");
        assert!(!f.body_fits_at_y(edge.0 + 0.45, edge.1, below, 0.3), "half in the void, half in the ground");
        // nothing stands, spawns or is placed on it
        assert!(!f.can_occupy(x as f64, z as f64, 0.2));
        assert!(f.can_occupy(gx as f64, gz as f64, 0.2));
        assert!(!f.floor_cells().contains(&Cell::new(14, 14)) && f.floor_cells().contains(&Cell::new(13, 14)));
        // the residency test: near a drop within the radius, far from it not
        assert!(f.drop_within(14.5, 14.5, 0.1));
        assert!(f.drop_within(17.0, 14.5, 1.6) && !f.drop_within(17.0, 14.5, 0.9));
        assert!(!f.drop_within(24.5, 24.5, 3.0));
        // and the cells of a merge keep their place
        let merged = Facility::merge(vec![(f.clone(), 45, 45, 0)], 135, 135, vec!["maze".into()], "DROP-1");
        assert!(merged.is_drop(Cell::new(59, 59)) && !merged.is_drop(Cell::new(14, 14)));
        assert_eq!(merged.drop_cells().len(), drops.len());
    }

    #[test]
    fn air_above_cells_open_the_ceiling_only() {
        let mut f = open_block();
        f.level = -2;
        let air = square(14, 14, 3);
        f.set_air_above_cells(&air);
        f.rebuild_grids();
        for c in &air {
            assert!(f.is_air_above(*c) && f.open_above(*c), "{c:?} has no ceiling");
            assert!(!f.open_below(*c) && !f.is_drop(*c) && !f.is_hole(c.x, c.y), "{c:?} is still ordinary floor");
            let (x, z) = c.center();
            assert!(f.body_fits(x, z, 0.3) && f.hunter_open(*c));
        }
        assert!(!f.is_air_above(Cell::new(13, 13)) && !f.open_above(Cell::new(13, 13)));
        assert_eq!(f.air_above_cells(), &air[..]);
    }

    #[test]
    fn hunters_never_path_into_a_drop() {
        let mut f = open_block();
        f.level = -1;
        let drops = square(14, 14, 3);
        f.set_drop_cells(&drops);
        f.rebuild_grids();
        for c in &drops {
            assert!(!f.hunter_open(*c), "{c:?}: a hunter cannot stand in a drop");
        }
        // there is a way round the void, and none of it goes through the void
        let (from, to) = (Cell::new(10, 15), Cell::new(20, 15));
        assert!(f.hunter_open(from) && f.hunter_open(to));
        let path = f.find_path(from, to);
        assert!(!path.is_empty(), "the void can be walked round");
        for c in &path {
            assert!(!f.is_drop(*c), "the path crosses the drop at {c:?}");
        }
        // a goal inside the void is unreachable
        assert!(f.find_path(from, Cell::new(15, 15)).is_empty());
        // and the reachable set of the start never includes a drop
        let reach = f.distance_map(from);
        assert!(drops.iter().all(|c| !reach.contains_key(c)));
    }

    #[test]
    fn a_ladder_hatch_is_a_hole_and_its_step_cell_is_not() {
        let hatch = Cell::new(15, 15);
        for run in [(0, -1), (0, 1), (1, 0), (-1, 0)] {
            let step = Cell::new(hatch.x + run.0, hatch.y + run.1);
            // the upper level: the hatch is a hole a walking body cannot enter, the step cell is plain floor
            let mut up = open_block();
            up.level = 0;
            let top = Stair::ladder_with_run(hatch, -1, Cover::Manhole, run);
            up.stairs.push(top);
            up.rebuild_grids();
            assert_eq!(top.step_cell(), step);
            assert!(up.is_hole(hatch.x, hatch.y) && up.open_below(hatch) && !up.open_above(hatch), "{run:?}");
            assert!(!up.is_hole(step.x, step.y) && !up.open_below(step), "{run:?}: the step cell is no hole");
            assert!(!up.body_fits(hatch.center().0, hatch.center().1, 0.3), "{run:?}: walking into the hatch is refused");
            assert!(up.body_fits(step.center().0, step.center().1, 0.3), "{run:?}: the step cell is walkable");
            assert!(!up.hunter_open(hatch) && up.hunter_open(step));
            // the lower level: the foot is an ordinary cell with no ceiling over it, the step cell beside it too
            let mut lo = open_block();
            lo.level = -1;
            let foot = Stair::ladder_with_run(hatch, 1, Cover::Manhole, run);
            lo.stairs.push(foot);
            lo.rebuild_grids();
            assert!(!lo.is_hole(hatch.x, hatch.y), "{run:?}: the foot is an ordinary floor cell");
            assert!(lo.open_above(hatch) && !lo.open_below(hatch), "{run:?}: the shaft is open above the foot");
            assert!(lo.body_fits(hatch.center().0, hatch.center().1, 0.3), "{run:?}: a body stands on the foot");
            assert!(lo.hunter_open(hatch) && lo.body_fits(step.center().0, step.center().1, 0.3));
            assert!(!lo.open_above(step));
            assert_eq!(top.well_cells(), foot.well_cells());
        }
    }

    #[test]
    fn every_catwalk_link_steps_onto_a_kept_deck_cell_with_no_prop_on_it() {
        let mut seen = 0;
        for (key, mut f) in maze_chunks() {
            if f.apply_catwalks(WORLD, key, "maze") == 0 {
                continue;
            }
            for l in f.catwalk_links.clone() {
                seen += 1;
                assert_eq!(l.run.0.abs() + l.run.1.abs(), 1, "{:?}: run {:?} is a unit step", l.cell, l.run);
                let deck = Cell::new(l.cell.x + l.run.0, l.cell.y + l.run.1);
                assert_eq!(f.walk_surface_at(deck).kind, DeckKind::Catwalk, "{:?} leads to a deck cell", l.cell);
                assert_eq!(f.walk_surface_at(l.cell).kind, DeckKind::Floor);
                for c in [l.cell, deck] {
                    let (x, z) = c.center();
                    assert!(f.can_occupy(x as f64, z as f64, 0.3), "{c:?}: a solid prop on the link");
                }
                // the link as a flight, for its cells: foot on the ground cell, landing on the deck cell
                let fp = well_footprint(&Facility::catwalk_flight(&l));
                assert_eq!((fp.foot, fp.landing), (l.cell, deck));
                assert_eq!(fp.upper_hole, vec![l.cell]);
            }
        }
        assert!(seen > 0, "no catwalk link in the sweep");
    }

    #[test]
    fn the_stair_record_of_a_ladder_is_the_ladder_flight() {
        let hatch = Cell::new(20, 20);
        for run in [(0, -1), (0, 1), (1, 0), (-1, 0)] {
            let top = Stair::ladder_with_run(hatch, -1, Cover::Grate, run);
            let flight = top.flight(0).expect("ladders are flights");
            assert_eq!((flight.kind, flight.anchor_cell, flight.run, flight.upper_level), (FlightKind::Ladder, hatch, run, 0));
            let foot = Stair::ladder_with_run(hatch, 1, Cover::Grate, run);
            assert_eq!(foot.flight(-1), Some(flight), "{run:?}");
            assert_eq!(top.rails_side(), (-run.0, -run.1));
            assert_eq!(top.step_cell(), Cell::new(hatch.x + run.0, hatch.y + run.1));
        }
        // the plain constructor steps north, and a flight has no rails side
        assert_eq!(Stair::ladder(hatch, -1, Cover::Manhole).run, (0, -1));
        let stairs = Stair::stairs(hatch, -1, (0, 1), FlightShape::Switchback, false);
        assert_eq!(stairs.rails_side(), (0, 0));
    }
}
