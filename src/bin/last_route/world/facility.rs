//! The finite floor generator, ported draw-for-draw from
//! game/world/generation/facility_generator.gd: a 3x3 sector grid around an 11x11 lobby,
//! catalogue room types, a Kruskal graph of three-wide halls with up to four loops, seeded
//! room props, then the service lift, safe zone, relays, batteries and hiding spots. The same
//! seed text gives the same floor as the original (Godot's float32 Vector2 maths and its
//! unstable sort are reproduced where they decide placement).

use super::catalog::Catalog;
use super::flight::{divider_width, flight_walk_y_with, flights, landing_row_with, lane_dividers, spiral_post, walk_path as flight_walk_path, well_footprint, Flight, FlightKind, FlightShape, Footprint, Segment};
use super::godot_sort::sort_custom;
use super::vertical::STOREY;
use super::infinite::ChunkKey;
use super::route::maze_regions;
use crate::content::regions::RegionDef;
use super::inferno_terrain::Terrain;
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
    /// A sewer valve wheel's or sluice gate's keys; `None` for every other prop.
    pub sewer: Option<SewerKeys>,
    /// A pipe hung at crawl height (the sewer's crawl-under pipe, underside `collider_y0` above the floor). `solid` stays
    /// false so the floor footprint is open, and `crawl_stops` blocks a body whose top reaches that underside: a standing
    /// runner or a bot is stopped, a crouched runner passes under, and a hunter is always stopped (`hunter_fits`).
    pub crawl_under: bool,
}

/// The prop type of a room arch (`_room_door_props`, streamed_route_generator.gd:5101): a three-cell doorway with no
/// leaf. Godot spawns a door only for `PropModelCatalog.ROOM_DOOR_TYPES`, which leaves the arch out, so it is an open
/// way: it has no model, never swings, never blocks and has no prompt.
pub const ROOM_ARCH: &str = "roomArch";

impl Prop {
    /// The doorway of a room door that has a leaf (it swings, blocks and is drawn); `None` for a room arch, which
    /// keeps `door` for its doorway (door clearance) only, and for every other prop.
    pub fn leaf_door(&self) -> Option<DoorDraw> {
        self.door.filter(|_| self.ptype != ROOM_ARCH)
    }
}

/// The prop type of a sewer sluice gate (`world::route::sewer`).
pub const SLUICE_GATE: &str = "sewerSluiceGate";

/// What a sewer valve wheel or sluice gate prop carries from the sewer chunk (`valve_effect` and `gate`): the effect
/// name is one of data/pipes.json `effects`, the gate id ties a gate valve to its sluice (`GATE:x:y:z:n`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SewerKeys {
    pub effect: String,
    pub gate: String,
}

/// How a room door is drawn: the doorway's width in cells (`door_span`) and the offset of the doorway's centre
/// from the threshold `cell`, which is `placement_offset` for a double door (door.rs).
/// Why a route was not found (admin HUNTER VIEW).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathFail {
    NoRoute,
    DoorClosed,
    PropCollider,
}

impl PathFail {
    /// The upper-case label the readout shows.
    pub fn label(self) -> &'static str {
        match self {
            PathFail::NoRoute => "NO ROUTE",
            PathFail::DoorClosed => "DOOR CLOSED",
            PathFail::PropCollider => "PROP COLLIDER",
        }
    }
}

/// find_path_stats' readout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathStats {
    pub length: usize,
    pub cost: f32,
    pub nodes_searched: usize,
    pub elapsed_us: u64,
    pub fail: Option<PathFail>,
}

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
    /// Hiding kind from data/hiding.json (wardrobe, stall ...); empty means a locker.
    pub kind: String,
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
    /// The music box (game/music_box.rs). Not in `PICKUP_ORDER`: the equipment draw of a finite floor does not place it.
    MusicBox,
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

/// The railing sides of a walkway cell (`Facility::set_rails`): north is -y, east +x, south +y, west -x.
pub const RAIL_N: u8 = 1;
pub const RAIL_E: u8 = 2;
pub const RAIL_S: u8 = 4;
pub const RAIL_W: u8 = 8;

/// The side bit of a unit step to a neighbouring cell, 0 for anything that is not one of the four.
pub fn rail_side_bit(step: (i32, i32)) -> u8 {
    match step {
        (0, -1) => RAIL_N,
        (1, 0) => RAIL_E,
        (0, 1) => RAIL_S,
        (-1, 0) => RAIL_W,
        _ => 0,
    }
}

/// The unit step a side bit faces.
fn rail_side_step(bit: u8) -> (i32, i32) {
    match bit {
        RAIL_N => (0, -1),
        RAIL_E => (1, 0),
        RAIL_S => (0, 1),
        _ => (-1, 0),
    }
}

/// Test level for the railed channel: a walkway row y = 10 (x 8..=14) over a walled pool of channel water at y = 11, fenced
/// on its south side except at the gap x = 11. The pool's cells are the water (`set_water_cells`), so a body in them is in
/// the water and the grid closes them behind the railing.
#[cfg(test)]
pub(crate) fn railed_pool() -> Facility {
    let mut f = Facility::open_test_level(0);
    f.wall[11][7] = true;
    f.wall[11][15] = true;
    for x in 8..=14usize {
        f.wall[12][x] = true;
    }
    let edges: Vec<(Cell, u8)> = (8..=14).filter(|x| *x != 11).map(|x| (Cell::new(x, 10), RAIL_S)).collect();
    f.set_rails(&edges, 0.06, 0.07);
    let water: Vec<Cell> = (8..=14).map(|x| Cell::new(x, 11)).collect();
    f.set_water_cells(&water);
    f.rebuild_grids();
    f
}

/// The solid railings of a level: a thin wall along a cell edge, on the walkway side of it (the sewer's channel
/// railings, `render::sewer_rails`). `edges` is each (cell, one side bit); `grid` the side bits per cell, indexed by
/// `Facility::index_rails`. A railing runs the whole edge, `inset` in from it, and is `half` thick on either side of that line.
#[derive(Clone, Debug, Default)]
struct Rails {
    edges: Vec<(Cell, u8)>,
    grid: Vec<u8>,
    inset: f32,
    half: f32,
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
    /// Indices into `props` of the crawl-under pipes (`Prop::crawl_under`): the few props only a hunter is stopped by.
    crawl_props: Vec<usize>,
    random: Rng,
    /// Solid props by cell, for fast footprint tests.
    prop_grid: Vec<Vec<usize>>,
    /// The swing of each room door, keyed by its threshold cell. A door with no entry has not been ticked yet
    /// and counts as open, so a floor nobody calls `update_doors` on stays walkable. Nothing here opens a door
    /// for the runner or a player bot (NPC teammate): only `toggle_door` (the interact press) does; hunters pull the doors they reach.
    door_state: HashMap<Cell, DoorState>,
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
    /// The channel water cells of a sewer window (`set_water_cells`): sorted, and indexed into `water` by `rebuild_grids`.
    /// The maps tint these; nothing in the walk, the swim or the hunters' grid reads them (those read the sewer chunk grid).
    water_list: Vec<Cell>,
    water: Vec<bool>,
    /// The solid railings along the water's edge (`set_rails`); empty on every level but the sewers.
    rails: Rails,
    /// The noisy floor and ceiling of an Inferno window (`world/inferno_terrain.rs`); `None` on every other level. Set by
    /// `set_terrain`.
    terrain: Option<Terrain>,
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

/// A snapshot of door swing: threshold cell, open amount, wanted-open.
pub type DoorSwing = Vec<(Cell, f32, bool)>;

/// A hunter this close to a door's edge pulls it open and leaves it open (the Godot hunter's
/// `_operate_hunter_room_door` reaches 1.15 m from the door's centre; a wider doorway adds half its span).
pub const DOOR_OPENER_REACH: f32 = 1.15;
/// Open amount gained or lost per second.
pub const DOOR_SWING_RATE: f32 = 4.0;
/// Half the thickness of a closed door's blocking slab, in metres.
const DOOR_HALF_DEPTH: f32 = 0.1;
/// How far (metres) a body's walk height may be from a flight's surface for it to stand on the flight.
pub(crate) const FLIGHT_FIT_TOL: f32 = 0.6;
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
        let hole = fp.upper_hole.clone();
        let mut v = fp.upper_hole;
        v.push(fp.landing);
        v.push(fp.foot);
        v.extend(fp.upper_clear);
        v.extend(fp.lower_clear);
        if self.shape == FlightShape::Spiral {
            // both ends' lanes, so the upper and the lower level carve the same cells (as flight_links::carve_cells does)
            // (none in the Godot parity scope: vertical::spiral_keepout)
            v.extend(crate::world::vertical::spiral_keepout(&hole, fp.landing, fp.foot));
        }
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
        // the well, this stair's own end and the rest of the landing row (a body sinking onto the ramp has its back
        // corners over it, so it carries the level's flat floor)
        if here == other || !(here == own || fp.upper_hole.contains(&here) || landing_row_with(flights(), &flight).contains(&here)) {
            return None;
        }
        // what a body of this record's level walks on: the lower level sees a spiral's slab as overhead and the wedge
        // under it as plain floor (the walker arriving at the helicoid's foot steps off onto it)
        flight_walk_y_with(flights(), &flight, x, z, self.dir > 0).map(|y| y - level as f32 * STOREY)
    }

    /// Whether the point lies within `r` of a drop in the flight's walking surface: a spiral's end radial, across which the
    /// slab of upper floor stands a storey over the lowest step. The route crosses it where the walker changes level, and a
    /// body is kept off it (it is the rail): a test that fits a body along the route skips those samples.
    #[cfg(test)]
    pub(crate) fn near_cliff(&self, level: i32, x: f32, z: f32, r: f32) -> bool {
        let Some(here) = self.surface_rel(level, x, z) else { return false };
        (0..8).any(|i| {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            self.surface_rel(level, x + r * a.cos(), z + r * a.sin()).is_some_and(|h| (h - here).abs() > FLIGHT_FIT_TOL)
        })
    }

    /// The post a spiral stands round (centre, radius; grid coordinates) as held by `level`. `None` for any other flight.
    pub fn post(&self, level: i32) -> Option<((f32, f32), f32)> {
        if self.is_ladder() || self.shape != FlightShape::Spiral {
            return None;
        }
        self.flight(level).and_then(|f| spiral_post(&f))
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
    /// An unpopulated floor with the same initialized grids as `generate`. Streamed chunks
    /// replace this shell with their authored layout, so they must not first generate a
    /// disposable finite floor and all of its rooms, props, relays and navigation data.
    pub fn blank(seed_text: &str, cols: i32, rows: i32) -> Facility {
        let cols = cols.clamp(35, 121);
        let rows = rows.clamp(35, 121);
        Facility {
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
            crawl_props: vec![],
            random: Rng::new(seed_text),
            prop_grid: vec![vec![]; (cols * rows) as usize],
            door_state: HashMap::new(),
            level: 0,
            drop_list: vec![],
            air_list: vec![],
            drop: vec![],
            air: vec![],
            water_list: vec![],
            water: vec![],
            rails: Rails::default(),
            terrain: None,
        }
    }

    pub fn generate(seed_text: &str, opt: &Options, cat: &Catalog) -> Facility {
        let mut f = Facility::blank(seed_text, opt.cols, opt.rows);
        let room_count = opt.room_count.max(5);
        f.carve_facility_rooms(room_count, cat);
        f.start = f.rooms[0].center;
        f.connect_facility_rooms();
        f.populate_rooms(cat);
        super::door_clearance::drop_blocked(&mut f);
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
        // last, so every seeded placement above saw the same props as Godot's: the removed types (the mirrors) go
        if f.remove_listed_props(cat) > 0 {
            f.rebuild_grids();
        }
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
        super::door_clearance::drop_blocked(self);
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

    /// `clear_props_in`, and also every solid prop whose FOOTPRINT (its centre cell plus offset, grown by its half extents)
    /// overlaps the rectangle, not only those whose centre cell is inside it. A wide prop (a sewer outflow pipe is 3.2 m)
    /// centred one or two cells outside a stair's bay otherwise reaches into the ring round the well or the landing and
    /// pins the runner against the drop. Room doors stay (they are the doorways). Call `rebuild_grids` afterwards.
    pub fn clear_props_reaching(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.clear_props_in(x, y, w, h);
        let (x0, y0, x1, y1) = (x as f32, y as f32, (x + w) as f32, (y + h) as f32);
        let mut swept: Vec<(f32, f32, f32, f32)> = vec![];
        self.props.retain(|p| {
            // a sewer valve or sluice gate belongs to a channel puzzle (game/pipes.rs): never swept away by reach
            if p.door.is_some() || !p.solid || p.sewer.is_some() {
                return true;
            }
            let (px, py) = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
            // strictly overlapping: a prop that only touches the rectangle's edge (a shelf flush against its wall) stays
            let overlaps = px + p.half.0 > x0 + 1e-3 && px - p.half.0 < x1 - 1e-3 && py + p.half.1 > y0 + 1e-3 && py - p.half.1 < y1 - 1e-3;
            if overlaps {
                swept.push((px, py, p.half.0, p.half.1));
            }
            !overlaps
        });
        // what stood on a swept table or desk (a `mount_y` prop whose centre is over its footprint) goes with it, or it would
        // hang in the air above the cleared floor
        if !swept.is_empty() {
            self.props.retain(|p| {
                if p.mount_y.is_none() || p.door.is_some() {
                    return true;
                }
                let (px, py) = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
                !swept.iter().any(|(sx, sy, hx, hy)| (px - sx).abs() <= *hx && (py - sy).abs() <= *hy)
            });
        }
    }

    /// Removes the props of cell `c` and every solid prop whose footprint reaches into it, so a body fills the cell
    /// (a ladder's hatch and step cell). Room doors are never removed, and relays, batteries, pickups
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

    /// The removed-props post-pass: drops every prop whose type the catalogue lists as removed (data/rooms/catalog.json
    /// `removed_props`: the mirrors) and returns how many went. It runs when a floor's or chunk's props become live,
    /// AFTER the generators have placed everything, so the seeded placement (checked against Godot) is unchanged and
    /// only the listed props are missing. Nothing else refers to these props (they are not doors, valves, seats, hiding
    /// places or surfaces), and the prop indices are rebuilt by `rebuild_grids`: call it afterwards.
    pub fn remove_listed_props(&mut self, cat: &Catalog) -> usize {
        let before = self.props.len();
        self.props.retain(|p| !cat.is_removed_prop(&p.ptype));
        before - self.props.len()
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
        self.index_rails();
        self.prop_grid = vec![vec![]; (self.cols * self.rows) as usize];
        for i in 0..self.props.len() {
            if self.props[i].solid {
                for ci in self.prop_index_cells(&self.props[i]) {
                    self.prop_grid[ci].push(i);
                }
            }
        }
        self.crawl_props = (0..self.props.len()).filter(|i| self.props[*i].crawl_under).collect();
        self.build_hunter_grid();
    }

    /// A sewer sluice gate is a wall across its channel cells while closed and floor while open (its valve turned an odd
    /// number of times). Sets every gate prop named `gate` and rebuilds the solid index and the hunters' grid when a gate
    /// changed. Returns whether one did.
    pub fn set_gate_open(&mut self, gate: &str, open: bool) -> bool {
        let mut changed = false;
        for p in self.props.iter_mut() {
            if p.ptype == SLUICE_GATE && p.sewer.as_ref().is_some_and(|k| k.gate == gate) && p.solid == open {
                p.solid = !open;
                changed = true;
            }
        }
        if changed {
            self.rebuild_grids();
        }
        changed
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
        let _window_merge_perf = crate::app::perf::section("window merge");
        // Window generation merges the same nine cached chunks repeatedly as
        // the runner crosses chunk boundaries. Reserve the final collection
        // sizes up front so repeated resident/active builds do not grow and
        // copy these vectors several times.
        let room_cap: usize = parts.iter().map(|(p, _, _, _)| p.rooms.len()).sum();
        let prop_cap: usize = parts.iter().map(|(p, _, _, _)| p.props.len()).sum();
        let relay_cap: usize = parts.iter().map(|(p, _, _, _)| p.relays.len()).sum();
        let battery_cap: usize = parts.iter().map(|(p, _, _, _)| p.batteries.len()).sum();
        let hiding_cap: usize = parts.iter().map(|(p, _, _, _)| p.hiding.len()).sum();
        let pickup_cap: usize = parts.iter().map(|(p, _, _, _)| p.pickups.len()).sum();
        let light_cap: usize = parts.iter().map(|(p, _, _, _)| p.room_lights.len()).sum();
        let stair_cap: usize = parts.iter().map(|(p, _, _, _)| p.stairs.len()).sum();
        let drop_cap: usize = parts.iter().map(|(p, _, _, _)| p.drop_list.len()).sum();
        let air_cap: usize = parts.iter().map(|(p, _, _, _)| p.air_list.len()).sum();
        let water_cap: usize = parts.iter().map(|(p, _, _, _)| p.water_list.len()).sum();
        let mut f = Facility {
            seed_text: seed_text.to_string(),
            cols,
            rows,
            wall: vec![vec![true; cols as usize]; rows as usize],
            rooms: Vec::with_capacity(room_cap),
            props: Vec::with_capacity(prop_cap),
            start: Cell::new(cols / 2, rows / 2),
            exit: Cell::new(-10000, -10000),
            safe_zone: None,
            relays: Vec::with_capacity(relay_cap),
            batteries: Vec::with_capacity(battery_cap),
            hiding: Vec::with_capacity(hiding_cap),
            pickups: Vec::with_capacity(pickup_cap),
            room_lights: Vec::with_capacity(light_cap),
            hunter_spawn: Cell::new(cols / 2, rows / 2),
            has_lift: false,
            stairs: Vec::with_capacity(stair_cap),
            cell_region: vec![0; (cols * rows) as usize],
            region_names,
            origin: (0, 0),
            hole: vec![false; (cols * rows) as usize],
            hunter_open: vec![],
            crawl_props: vec![],
            random: Rng::new(seed_text),
            prop_grid: vec![],
            door_state: HashMap::new(),
            level: parts.first().map_or(0, |p| p.0.level),
            drop_list: Vec::with_capacity(drop_cap),
            air_list: Vec::with_capacity(air_cap),
            drop: vec![],
            air: vec![],
            water_list: Vec::with_capacity(water_cap),
            water: vec![],
            rails: Rails::default(),
            terrain: None,
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
            f.water_list.extend(p.water_list.iter().map(|c| sh(*c)));
            // the railings along the water ride along with their chunk (all chunks of a window share one shape)
            if !p.rails.edges.is_empty() {
                f.rails.edges.extend(p.rails.edges.iter().map(|(c, bit)| (sh(*c), *bit)));
                (f.rails.inset, f.rails.half) = (p.rails.inset, p.rails.half);
            }
            if let Some(s) = p.safe_zone {
                f.safe_zone = Some(sh(s));
            }
        }
        // A chunk with props and no room of its own (the sewer props find no room and carry 0) had `room_base` added to that
        // 0: the first room of a later chunk, or one past the last room of the window. Every index names a room of the
        // window again (a window with no room at all keeps none; readers use `rooms.get`).
        if let Some(last) = f.rooms.len().checked_sub(1) {
            for p in &mut f.props {
                p.room = p.room.min(last);
            }
        }
        drop(_window_merge_perf);
        let _window_grid_perf = crate::app::perf::section("window grid");
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
        if self.rail_blocks(x, y, r) {
            return false;
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
        self.water = index(&self.water_list);
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

    /// Marks `cells` as this level's channel water (a sewer chunk's WATER cells), for the maps. Replaces any earlier water;
    /// call `rebuild_grids` afterwards. It changes nothing about walking, swimming or the hunters' grid.
    pub fn set_water_cells(&mut self, cells: &[Cell]) {
        self.water_list = cells.to_vec();
        self.water_list.sort();
        self.water_list.dedup();
        self.index_drops();
    }

    /// Whether `c` is a channel water cell (the sewer's pools and channels).
    pub fn is_water(&self, c: Cell) -> bool {
        c.x >= 0
            && c.y >= 0
            && c.x < self.cols
            && c.y < self.rows
            && self.water.get((c.y * self.cols + c.x) as usize).copied().unwrap_or(false)
    }

    /// Every channel water cell (sorted); empty off the sewers.
    pub fn water_cells(&self) -> &[Cell] {
        &self.water_list
    }

    /// Makes `cells` this level's air-above cells: no ceiling, because a drop on the level above falls through them.
    pub fn set_air_above_cells(&mut self, cells: &[Cell]) {
        self.air_list = cells.to_vec();
        self.air_list.sort();
        self.air_list.dedup();
        self.index_drops();
    }

    /// Stands a solid railing on the given sides of walkway cells: `edges` is (cell, side bit `RAIL_N`..`RAIL_W`), `inset`
    /// how far in from the cell edge the rail line sits and `thickness` how thick the wall is (metres). The railing is a
    /// wall for every body (`clear_of_solids`: the runner, the bots, the hunters, a knock-back) and the cell it faces is closed
    /// to the hunters' grid (`build_hunter_grid`), so no route crosses it. Replaces any earlier railings; call
    /// `rebuild_grids` afterwards.
    pub fn set_rails(&mut self, edges: &[(Cell, u8)], inset: f32, thickness: f32) {
        let mut list: Vec<(Cell, u8)> = edges.iter().copied().filter(|(_, bit)| matches!(*bit, RAIL_N | RAIL_E | RAIL_S | RAIL_W)).collect();
        list.sort();
        list.dedup();
        self.rails = Rails { edges: list, grid: vec![], inset, half: thickness * 0.5 };
    }

    fn index_rails(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        let mut grid = vec![0u8; if self.rails.edges.is_empty() { 0 } else { (cols * rows) as usize }];
        for (c, bit) in &self.rails.edges {
            if c.x >= 0 && c.y >= 0 && c.x < cols && c.y < rows {
                grid[(c.y * cols + c.x) as usize] |= *bit;
            }
        }
        self.rails.grid = grid;
    }

    /// Every railing as (walkway cell, side bit), sorted.
    pub fn rail_edges(&self) -> &[(Cell, u8)] {
        &self.rails.edges
    }

    /// The side bits of the railings standing on walkway cell `c` (0 for none).
    pub fn rail_sides(&self, c: Cell) -> u8 {
        if c.x < 0 || c.y < 0 || c.x >= self.cols || c.y >= self.rows {
            return 0;
        }
        self.rails.grid.get((c.y * self.cols + c.x) as usize).copied().unwrap_or(0)
    }

    /// Whether a railing stands on the edge between the orthogonal neighbours `a` and `b` (on either one's side of it).
    pub fn rail_between(&self, a: Cell, b: Cell) -> bool {
        let step = (b.x - a.x, b.y - a.y);
        let (there, back) = (rail_side_bit(step), rail_side_bit((-step.0, -step.1)));
        there != 0 && (self.rail_sides(a) & there != 0 || self.rail_sides(b) & back != 0)
    }

    /// Whether a body of radius `r` at `(x, y)` overlaps a railing. A railing is the whole edge long and `Rails::half`
    /// thick either side of its line, `Rails::inset` in from the edge on the walkway cell's side; the height is no
    /// factor, so a jump does not clear it (there is no vault).
    fn rail_blocks(&self, x: f64, y: f64, r: f64) -> bool {
        if self.rails.grid.is_empty() {
            return false;
        }
        let (x0, x1) = ((x - r).floor() as i32, (x + r).floor() as i32);
        let (y0, y1) = ((y - r).floor() as i32, (y + r).floor() as i32);
        let off = 0.5 - self.rails.inset as f64;
        let half = self.rails.half as f64;
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let bits = self.rail_sides(Cell::new(cx, cy));
                if bits == 0 {
                    continue;
                }
                let (mx, my) = (cx as f64 + 0.5, cy as f64 + 0.5);
                for bit in [RAIL_N, RAIL_E, RAIL_S, RAIL_W] {
                    if bits & bit == 0 {
                        continue;
                    }
                    let (sx, sy) = rail_side_step(bit);
                    let (bx, by) = (mx + sx as f64 * off, my + sy as f64 * off);
                    // long across the cell on the edge's own axis, thin on the other
                    let (hx, hy) = if sx != 0 { (half, 0.5) } else { (0.5, half) };
                    if (x - bx).abs() < hx + r && (y - by).abs() < hy + r {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The farthest point of the straight line from `from` to `to` that a body of radius `r` reaches before something
    /// solid stops it (`to` itself when the line is clear): a knock-back or a shove never carries a body through a wall
    /// or a railing. A `from` the body does not fit at stays put.
    pub fn clear_toward(&self, from: (f32, f32), to: (f32, f32), r: f32) -> (f32, f32) {
        let d = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
        let steps = ((d / 0.05).ceil() as i32).max(1);
        let mut last = from;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let p = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            if !self.body_fits_tall(p.0, p.1, r) {
                break;
            }
            last = p;
        }
        last
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

    /// Whether a body of radius `r` standing on this level's floor fits at `(x, y)`. Walls, solid props, doors, lockers and
    /// flight dividers are tested for the body's own radius. The stair wells and ladder hatches are tested by the runner's
    /// own rule at floor height (`stair_holes_fit`, `PLAYER_RADIUS`): a cell of a well holds a floor-walking body only where
    /// the flight's surface is at the floor. A hunter or a bot is never carried across a well (over the drop, under the
    /// ramp) that the runner could not cross; the landing where a flight starts still holds the wider hunter, as it holds
    /// the runner.
    ///
    /// A drop cell has no floor: a hunter-sized body (radius `HUNTER_RADIUS` or more) never stands on one, by the same
    /// five-sample test as `can_occupy`. The runner and the thinner bots may still enter a drop to fall (the deliberate
    /// gap of a falls chunk). Free on a floor without drops.
    pub fn body_fits(&self, x: f32, y: f32, r: f32) -> bool {
        if r >= crate::config::HUNTER_RADIUS && !self.drop_list.is_empty() {
            for (a, b) in [(x, y), (x - r, y - r), (x + r, y - r), (x - r, y + r), (x + r, y + r)] {
                if self.is_drop(Cell::new(a.floor() as i32, b.floor() as i32)) {
                    return false;
                }
            }
        }
        self.fits(x, y, r, 0.0, false)
            && (self.stairs.is_empty() || self.stair_holes_fit(x, y, r.min(crate::config::PLAYER_RADIUS), 0.0, true))
    }

    /// Whether a crawl-under pipe's footprint covers a body of radius `r` at `(x, y)`. Height is `crawl_stops`.
    pub fn crawl_blocks(&self, x: f32, y: f32, r: f32) -> bool {
        self.crawl_props.iter().any(|&i| {
            let p = &self.props[i];
            let (px, py) = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
            (x - px).abs() < p.half.0 + r && (y - py).abs() < p.half.1 + r
        })
    }

    /// Whether a crawl pipe stops a body of radius `r` whose top is `top` metres above this facility's floor. The pipe's
    /// underside is the crouch clearance (`crawl_clearance`, the crouch capsule plus a margin): a crouched runner's top
    /// is under it and passes, a standing runner or a standing bot is over it and is stopped. Hunters use `hunter_fits`,
    /// which closes the footprint at every height.
    pub fn crawl_stops(&self, x: f32, y: f32, r: f32, top: f32) -> bool {
        top > crate::world::route::sewer::core::crawl_clearance() as f32
            && self.crawl_blocks(x, y, r)
    }

    /// Whether a hunter may be placed on `c`: the hunters' grid is open there, and the cell is not channel water.
    pub fn hunter_may_spawn(&self, c: Cell) -> bool {
        self.hunter_open(c) && !self.is_water(c)
    }

    /// Whether the hunters' grid is closed at `c` only by a crawl-under pipe: the cell is floor for a crouched runner
    /// and shut to a hunter.
    pub fn crawl_closed(&self, c: Cell) -> bool {
        let (x, y) = c.center();
        !self.crawl_props.is_empty()
            && !self.hunter_open(c)
            && self.crawl_blocks(x, y, DEFAULT_AGENT_RADIUS as f32)
            && !self.is_drop(c)
            && self.body_fits(x, y, DEFAULT_AGENT_RADIUS as f32)
    }

    /// Whether a hunter (`HUNTER_RADIUS`, tall) standing on the floor fits at `(x, y)`: `body_fits` for its radius, and no
    /// crawl-under pipe in the way. The one footprint rule of a hunter's step, route cell and spawn.
    pub fn hunter_fits(&self, x: f32, y: f32) -> bool {
        self.body_fits(x, y, crate::config::HUNTER_RADIUS) && !self.crawl_blocks(x, y, crate::config::HUNTER_RADIUS)
    }

    /// `body_fits` for a body of radius `r` that is as tall as it is wide to the crawl-under pipes: a body of a hunter's
    /// radius or more is a hunter (the runner and the bots are thinner), so it is kept out of them too. The placement
    /// searches (`nearest_open`, `clear_toward`) take their radius from the caller, which is `HUNTER_RADIUS` for every
    /// teleport and shove of a hunter (the Void hunters, the spawn spots, the admin tools); without this they could set a
    /// hunter down inside a crawl pipe.
    fn body_fits_tall(&self, x: f32, y: f32, r: f32) -> bool {
        self.body_fits(x, y, r) && (r < crate::config::HUNTER_RADIUS || !self.crawl_blocks(x, y, r))
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

    /// Whether a body of radius `r` at `(x, z)` overlaps the solid wall between the flights of any switchback (a wall
    /// `divider_width` metres thick with flat ends, so the body is kept `r` off its rectangle) or a spiral's post (a
    /// cylinder, so `r` off its circle).
    pub fn divider_blocks(&self, x: f32, z: f32, r: f32) -> bool {
        if self.stairs.is_empty() {
            return false;
        }
        let half = divider_width() * 0.5;
        // a flight's solids lie within 8 cells of its anchor: the cheap cull before building the flight
        const NEAR: f32 = 9.0;
        self.stairs.iter().any(|s| {
            let (cx, cz) = s.cell.center();
            if (x - cx).abs() > NEAR || (z - cz).abs() > NEAR {
                return false;
            }
            let wall = s.dividers(self.level).iter().any(|seg| {
                let (dx, dz) = (seg.b.0 - seg.a.0, seg.b.1 - seg.a.1);
                let len = (dx * dx + dz * dz).sqrt();
                if len < 1e-6 {
                    return false;
                }
                // the body's offset from the wall's start along it and across it
                let (ux, uz) = (dx / len, dz / len);
                let (px, pz) = (x - seg.a.0, z - seg.a.1);
                let (along, across) = (px * ux + pz * uz, -px * uz + pz * ux);
                // the rectangle [0, len] x [-half, half], grown by r with rounded corners
                let out_along = (-along).max(along - len).max(0.0);
                let out_across = (across.abs() - half).max(0.0);
                out_along * out_along + out_across * out_across < r * r
            });
            wall || s.post(self.level).is_some_and(|((px, pz), pr)| {
                let (ex, ez) = (x - px, z - pz);
                ex * ex + ez * ez < (pr + r) * (pr + r)
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

    /// `body_fits` for a body standing at walk height `height` (metres over the level's floor): off the floor it
    /// fits only on a flight at that height (see `fits`).
    pub fn body_fits_at(&self, x: f32, y: f32, r: f32, height: f32) -> bool {
        self.fits(x, y, r, height, false)
    }

    /// The floor-height and hole rules of `fits`, for a body of radius `r` at walk height `height`: off the floor line a
    /// body fits only on a flight at its height (an Inferno window's floor is one continuous noisy surface with the slope
    /// limited by construction (`TerrainRules`), so the height rule does not apply there), and no corner or the centre
    /// may be in a stair well or ladder hatch unless on a flight (`by_height`: at the body's height; otherwise any tread).
    fn stair_holes_fit(&self, x: f32, y: f32, r: f32, height: f32, by_height: bool) -> bool {
        if self.terrain.is_none() && height != 0.0 {
            for (sx, sy) in [(x - r, y - r), (x + r, y - r), (x - r, y + r), (x + r, y + r)] {
                let cell = Cell::new(sx.floor() as i32, sy.floor() as i32);
                // open air holds a body at any height: only the cells around it say whether it is below the floor
                if self.is_drop(cell) {
                    continue;
                }
                if height.abs() > 0.01 && !(by_height && self.on_flight_at(sx, sy, height)) {
                    return false;
                }
            }
        }
        for (sx, sy) in [(x - r, y - r), (x + r, y - r), (x - r, y + r), (x + r, y + r), (x, y)] {
            if self.is_stair_hole(sx.floor() as i32, sy.floor() as i32) {
                let on = if by_height { self.on_flight_at(sx, sy, height) } else { self.on_stair_flight(sx, sy) };
                if !on {
                    return false;
                }
            }
        }
        true
    }

    /// The shared body of `body_fits_at` and `body_fits_at_y`. `by_height`: a corner on a flight at the body's height is
    /// exempt from the floor-height and hole rules; otherwise any tread of a flight is exempt from the hole rule.
    fn fits(&self, x: f32, y: f32, r: f32, height: f32, by_height: bool) -> bool {
        // a drop has no wall, so a body may enter it (the hole rule below does not apply to a drop either)
        if !self.clear_of_solids(x as f64, y as f64, r as f64) {
            return false;
        }
        if self.divider_blocks(x, y, r) {
            return false;
        }
        if !self.stair_holes_fit(x, y, r, height, by_height) {
            return false;
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

    /// The prop index and centre of every room door that has a leaf (a room arch is an open way, not a door).
    pub fn doors(&self) -> impl Iterator<Item = (usize, (f32, f32))> + '_ {
        self.props
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.leaf_door().map(|d| (i, Self::door_slab(p, d).0)))
    }

    /// The point of door `i`'s slab nearest to `from` (what the runner faces when standing flush against the leaf).
    pub fn door_nearest(&self, i: usize, from: (f32, f32)) -> Option<(f32, f32)> {
        let p = self.props.get(i)?;
        let (c, h) = Self::door_slab(p, p.leaf_door()?);
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

    /// The height of the ground under the point (x, z), metres above the level's floor line: the noisy cavern floor of
    /// an Inferno window (`set_terrain`), else 0 (the flat floor). A flight tread is not included
    /// (`level_stack::local_ground` asks the flights first).
    pub fn ground_height(&self, x: f32, z: f32) -> f32 {
        match &self.terrain {
            Some(t) => t.floor_at(x, z),
            None => 0.0,
        }
    }

    /// The height of the ceiling over the point (x, z) on an Inferno window; `None` elsewhere (the region's wall height).
    pub fn ceiling_height(&self, x: f32, z: f32) -> Option<f32> {
        self.terrain.as_ref().map(|t| t.ceiling_at(x, z))
    }

    /// The Inferno terrain of this window, when it has one.
    pub fn terrain(&self) -> Option<&Terrain> {
        self.terrain.as_ref()
    }

    /// Gives this facility the Inferno terrain of world `world` for its grid (the cell (0, 0) is world cell
    /// `self.origin`, so set `origin` first). `chunks` is how many chunks the grid spans each way.
    pub fn set_terrain(&mut self, world: &str, chunks: i32) {
        self.terrain = Some(Terrain::new(world, self.origin, chunks));
    }

    /// Dresses a level 0 maze chunk with its region variant (`route::maze_regions`): thins
    /// the chunk's loose pickups by the variant's `loot_scale` and its props by `prop_density` (a scale of 1 or more
    /// changes nothing; relays, batteries and doors are never thinned). The floors, room roles and prop set of the
    /// variant are looked up from the data by the chunk key, not stored here. Returns the variant, `None` for the plain maze.
    pub fn apply_maze_variant(&mut self, seed: &str, key: ChunkKey) -> Option<&'static maze_regions::MazeVariant> {
        let v = maze_regions::variant_at(seed, key.cx, key.cy, key.level)?;
        let tag = format!("{},{}", key.cx, key.cy);
        let loot = v.spawn_rules.loot_scale;
        // the music box is not loot a region thins: it is kept, and it does not count toward the loot index
        let mut n = 0usize;
        self.pickups.retain(|(_, p)| {
            if *p == Pickup::MusicBox {
                return true;
            }
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

    /// The swing of every registered door, keyed by its threshold cell, to carry across a window rebuild.
    pub fn door_swing(&self) -> DoorSwing {
        self.door_state.iter().map(|(c, s)| (*c, s.open, s.want_open)).collect()
    }

    /// Re-registers the doors of a rebuilt window with the swing they had in the old one. `shift` is how far the
    /// cells moved (new cell = old cell + shift); a door the new window does not have is dropped.
    pub fn restore_door_swing(&mut self, swing: &DoorSwing, shift: (i32, i32)) {
        for p in &self.props {
            let Some(d) = p.leaf_door() else { continue };
            let old = Cell { x: p.cell.x - shift.0, y: p.cell.y - shift.1 };
            let Some(&(_, open, want_open)) = swing.iter().find(|(c, _, _)| *c == old) else { continue };
            let (centre, half) = Self::door_slab(p, d);
            self.door_state.insert(p.cell, DoorState { open, want_open, span: d.span as f32, centre, half });
        }
    }

    /// Swings every room door toward its wanted state. `openers` (the hunters, never the runner or a player bot) open a
    /// door they come within `DOOR_OPENER_REACH` of and leave it open. `bodies` is everyone with a body: a leaf
    /// that is not shut never swings closed with one of them in its sweep, it swings back open until they clear.
    /// The first call registers every door closed (open, when a body already stands in its sweep).
    pub fn update_doors(&mut self, openers: &[(f32, f32)], bodies: &[(f32, f32)], dt: f32) {
        for p in &self.props {
            let Some(d) = p.leaf_door() else { continue };
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
                    || self.rail_between(c, n)
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
        self.nearest_open_where(x, y, r, |_| true)
    }

    /// `nearest_open` for a place a bot or hunter is put down: the nearest spot that is dry (not channel water) and that the
    /// hunters' grid opens, so a body set down there can route out at once. Spawns use this, never `nearest_open`.
    pub fn nearest_dry_open(&self, x: f32, y: f32, r: f32) -> Option<(f32, f32)> {
        let grid_ready = self.hunter_open.len() == (self.cols * self.rows) as usize;
        self.nearest_open_where(x, y, r, |c| !self.is_water(c) && (!grid_ready || self.hunter_open(c)))
    }

    /// The BFS of `nearest_open`, taking only the cells `accept` allows as the spot.
    fn nearest_open_where(&self, x: f32, y: f32, r: f32, accept: impl Fn(Cell) -> bool) -> Option<(f32, f32)> {
        let s = Cell::new(x.floor() as i32, y.floor() as i32);
        let mut seen = HashSet::from([s]);
        let mut q = VecDeque::from([s]);
        while let Some(c) = q.pop_front() {
            let (wx, wy) = c.center();
            // a well cell the hunters' grid closes (under the ramp, over the drop) is no place to stand and walk from
            let closed_well = self.hunter_open.len() == (self.cols * self.rows) as usize
                && self.is_stair_hole(c.x, c.y)
                && !self.hunter_open[(c.y * self.cols + c.x) as usize];
            if !closed_well && accept(c) && self.body_fits_tall(wx, wy, r) {
                return Some((wx, wy));
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if seen.contains(&n) || self.rail_between(c, n) || !self.is_floor(n.x as f64 + 0.5, n.y as f64 + 0.5) {
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
        // the door rule, read before any prop stands: a prop that would close a doorway's approach or stand in a door's
        // clearance is skipped at that cell (world::door_clearance)
        let placement = super::door_clearance::Placement::of(self);
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
                            sewer: None,
                            crawl_under: false,
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
                        if !self.room_prop_fits(&room, cell, t, rotation, cat, &placement) {
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
                    sewer: None,
                    crawl_under: false,
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
        placement: &super::door_clearance::Placement,
    ) -> bool {
        if !self.valid_open_cell(cell) || cell == self.start || cell == self.exit {
            return false;
        }
        let fp = cat.footprint(t, rotation);
        if !placement.allows(t, cell, fp, cat.is_solid(t)) {
            return false;
        }
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
        // no locker in a door's way (world::door_clearance)
        let door_clear = super::door_clearance::Clearance::of(self);
        let mut in_room = vec![];
        let mut in_corridor = vec![];
        for y in 1..self.rows - 1 {
            for x in 1..self.cols - 1 {
                let c = Cell::new(x, y);
                if !self.can_occupy(x as f64 + 0.5, y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
                    || taken.contains(&c)
                    || door_clear.blocks(super::door_clearance::Kind::Locker, c)
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
                    kind: String::new(),
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
                    && !self.is_water(*c)
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
                    && self.body_fits(x as f32 + 0.5, y as f32 + 0.5, DEFAULT_AGENT_RADIUS as f32)
                    && !self.crawl_blocks(x as f32 + 0.5, y as f32 + 0.5, DEFAULT_AGENT_RADIUS as f32);
            }
        }
        // a railing closes the cell it faces (the water beyond it): a route never steps across the fence. Only an
        // unfenced shore (the deliberate gap, a chunk border) stays open to the hunters.
        for (c, bit) in &self.rails.edges {
            let (dx, dy) = rail_side_step(*bit);
            let (nx, ny) = (c.x + dx, c.y + dy);
            if nx >= 0 && ny >= 0 && nx < self.cols && ny < self.rows {
                open[(ny * self.cols + nx) as usize] = false;
            }
        }
        // A well cell is ground for a hunter only where the flight's surface is at the floor (the first step of the ramp, a
        // spiral's floor wedge): the rest is under the ramp or over the drop, and a hunter crosses by the flight's edge.
        // (The plain footprint test above exempts every tread, so a hunter would otherwise path across the wide wells.)
        for s in self.stairs.iter().filter(|s| !s.is_ladder()) {
            for c in s.well_cells() {
                if c.x < 0 || c.y < 0 || c.x >= self.cols || c.y >= self.rows {
                    continue;
                }
                let (x, y) = c.center();
                if !s.surface_rel(self.level, x, y).is_some_and(|rel| rel.abs() <= FLIGHT_FIT_TOL) {
                    open[(c.y * self.cols + c.x) as usize] = false;
                }
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
                    && !self.is_water(*c)
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
                if !seen.contains(&n) && !self.rail_between(c, n) && self.can_occupy(n.x as f64 + 0.5, n.y as f64 + 0.5, 0.25) {
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

    /// Whether a sight line may pass `(x, z)` on this level: open floor, or a well / hatch / drop / air cell (an opening
    /// in a slab is no wall). With no hole cell standing in a wall this equals `is_floor`.
    pub fn sight_open(&self, x: f32, z: f32) -> bool {
        if self.is_floor(x as f64, z as f64) {
            return true;
        }
        let c = Cell::new(x.floor() as i32, z.floor() as i32);
        c.x >= 0
            && c.y >= 0
            && c.x < self.cols
            && c.y < self.rows
            && (self.is_hole(c.x, c.y) || self.is_drop(c) || self.is_air_above(c))
    }

    /// Whether a hunter body of radius `r` stands at `(x, z)` with feet at absolute height `y_abs`. This is the hunter
    /// footprint rule for flight cells: `body_fits_at_y` lets a body stand on a flight surface within `FLIGHT_FIT_TOL`.
    pub fn hunter_fits_at_y(&self, x: f32, z: f32, y_abs: f32, r: f32) -> bool {
        self.body_fits_at_y(x, z, y_abs, r)
    }

    /// Two open levels (0 and -1) joined by one flight, for the tests of the hunters' stack navigation. `ladder` joins
    /// them with a ladder (the shape is then unused); otherwise a stair of `shape` runs along +y from the landing (8, 8).
    #[cfg(test)]
    pub(crate) fn test_levels(shape: FlightShape, ladder: bool) -> (Facility, Facility) {
        let anchor = Cell::new(8, 8);
        let (up_s, lo_s) = if ladder {
            (
                Stair::ladder_with_run(anchor, -1, Cover::Manhole, (0, -1)),
                Stair::ladder_with_run(anchor, 1, Cover::Manhole, (0, -1)),
            )
        } else {
            let upper = Stair::stairs(anchor, -1, (0, 1), shape, false);
            (upper, Stair::stairs(upper.foot(), 1, (0, 1), shape, false))
        };
        let mut upper = Facility::open_test_level(0);
        upper.stairs.push(up_s);
        upper.rebuild_grids();
        let mut lower = Facility::open_test_level(-1);
        lower.stairs.push(lo_s);
        lower.rebuild_grids();
        (upper, lower)
    }

    /// Two open levels (0 and -1) with a drop at cells (18..20, 18..20) above and open air under it, and no stairs.
    #[cfg(test)]
    pub(crate) fn test_levels_drop() -> (Facility, Facility) {
        let mut upper = Facility::open_test_level(0);
        let mut lower = Facility::open_test_level(-1);
        let cells: Vec<Cell> = (18..20).flat_map(|y| (18..20).map(move |x| Cell::new(x, y))).collect();
        upper.set_drop_cells(&cells);
        lower.set_air_above_cells(&cells);
        upper.rebuild_grids();
        lower.rebuild_grids();
        (upper, lower)
    }

    /// A generated facility of `level` whose cells 4..30 are open floor with no props.
    #[cfg(test)]
    pub(crate) fn open_test_level(level: i32) -> Facility {
        let cat = crate::world::catalog::Catalog::load();
        let mut f = Facility::generate("NAV-1", &Options::default(), &cat);
        f.level = level;
        for z in 4..30usize {
            for x in 4..30usize {
                f.wall[z][x] = false;
            }
        }
        let inside = |c: Cell| (3..31).contains(&c.x) && (3..31).contains(&c.y);
        f.props.retain(|p| !inside(p.cell));
        f.hiding.retain(|h| !inside(h.cell));
        f.rebuild_grids();
        f
    }

    /// 8-way A* with an octile heuristic and no corner cutting (AStarGrid2D,
    /// DIAGONAL_MODE_ONLY_IF_NO_OBSTACLES).
    pub fn find_path(&self, from: Cell, to: Cell) -> Vec<Cell> {
        self.find_path_inner(from, to).0
    }

    /// find_path plus its cost readout for the admin HUNTER VIEW: path length in cells, travel cost, nodes expanded,
    /// elapsed microseconds, and on failure the reason.
    pub fn find_path_stats(&self, from: Cell, to: Cell) -> PathStats {
        let t = std::time::Instant::now();
        let (path, nodes) = self.find_path_inner(from, to);
        let elapsed_us = t.elapsed().as_micros() as u64;
        let cost = path
            .windows(2)
            .map(|w| if w[0].x != w[1].x && w[0].y != w[1].y { std::f32::consts::SQRT_2 } else { 1.0 })
            .sum();
        let fail = if path.is_empty() { Some(self.path_fail_reason(from, to)) } else { None };
        PathStats { length: path.len(), cost, nodes_searched: nodes, elapsed_us, fail }
    }

    /// Why there is no route: a blocked end cell is a prop collider or a closed door when one stands there, else no route.
    pub fn path_fail_reason(&self, from: Cell, to: Cell) -> PathFail {
        for c in [to, from] {
            if !self.hunter_open(c) {
                if self.props.iter().any(|p| p.cell == c) {
                    return PathFail::PropCollider;
                }
                if self.door_state.get(&c).is_some_and(|d| d.open < 0.5) {
                    return PathFail::DoorClosed;
                }
                return PathFail::NoRoute;
            }
        }
        PathFail::NoRoute
    }

    fn find_path_inner(&self, from: Cell, to: Cell) -> (Vec<Cell>, usize) {
        if !self.hunter_open(from) || !self.hunter_open(to) {
            return (vec![], 0);
        }
        let mut expanded = 0usize;
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
            expanded += 1;
            if c == to {
                let mut path = vec![c];
                let mut i = ci;
                while came[i as usize] != u32::MAX {
                    i = came[i as usize];
                    path.push(Cell::new(i as i32 % w, i as i32 / w));
                }
                path.reverse();
                return (path, expanded);
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
        (vec![], expanded)
    }

    /// has_line_of_sight: samples the segment at ceil(distance*6) points; walls only.
    ///
    /// Deliberately 2D (Godot parity): it has no height term.
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

    #[test]
    fn a_finite_floor_has_no_mirror_and_the_pass_takes_only_the_listed_types() {
        let cat = Catalog::load();
        // the bathrooms still list the mirror (the generator places it, as Godot does); the floor does not keep it
        assert!(cat.types.iter().any(|t| t.props.iter().any(|p| cat.is_removed_prop(p))));
        for n in 0..6 {
            let mut f = Facility::generate(&format!("MIRROR-{n}"), &Options::default(), &cat);
            assert!(!f.props.iter().any(|p| cat.is_removed_prop(&p.ptype)), "MIRROR-{n}: a mirror is on the floor");
            let kept: Vec<String> = f.props.iter().map(|p| p.id.clone()).collect();
            let first = f.props[0].clone();
            f.props.insert(1, Prop { id: "m1".into(), ptype: "bathroomMirror".into(), ..first.clone() });
            f.props.push(Prop { id: "m2".into(), ptype: "standingMirror".into(), ..first });
            assert_eq!(f.remove_listed_props(&cat), 2);
            assert_eq!(f.props.iter().map(|p| p.id.clone()).collect::<Vec<_>>(), kept);
            f.rebuild_grids();
            assert_eq!(f.remove_listed_props(&cat), 0);
        }
    }

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
            sewer: None,
            crawl_under: false,
        });
        f
    }

    /// A rebuilt window keeps the swing of its doors, with the cells shifted by the window's move.
    #[test]
    fn door_swing_survives_a_rebuild_shifted_by_the_origin_change() {
        let mut old = door_floor();
        let cell = old.props.last().unwrap().cell;
        old.update_doors(&[], &[], 0.0);
        let runner = cell.center();
        assert_eq!(old.toggle_door(cell, &[runner]), Some(DoorToggle::Opened));
        old.update_doors(&[], &[], 1.0);
        assert!(old.door_is_open(cell));
        let swing = old.door_swing();
        // the rebuilt window has the same door 40 cells over; without the carry it would be unregistered
        let mut fresh = door_floor();
        fresh.props.last_mut().unwrap().cell = Cell { x: cell.x - 40, y: cell.y - 40 };
        let moved = fresh.props.last().unwrap().cell;
        fresh.restore_door_swing(&swing, (-40, -40));
        fresh.update_doors(&[], &[], 0.0);
        assert!(fresh.door_is_open(moved), "the opened door stays open");
        assert_eq!(fresh.door_open_amount(moved), 1.0);
        // a shift that matches no door restores nothing
        let mut other = door_floor();
        other.restore_door_swing(&swing, (7, 7));
        other.update_doors(&[], &[(1e6, 1e6)], 0.0);
        assert_eq!(other.door_open_amount(other.props.last().unwrap().cell), 0.0);
    }

    /// The invisible door: a room arch has a doorway but no leaf and no model, so it is never a door to use. It is
    /// not listed, never registered, never blocks and cannot be toggled; the same prop as a room door is all of those.
    #[test]
    fn a_room_arch_is_an_open_way_not_a_door() {
        let mut f = door_floor();
        let cell = f.props.last().unwrap().cell;
        let (cx, cy) = cell.center();
        let n = f.props.len() - 1;
        f.props[n].ptype = ROOM_ARCH.into();
        assert!(f.props[n].door.is_some() && f.props[n].leaf_door().is_none());
        assert!(f.doors().all(|(i, _)| i != n), "an arch takes no prompt");
        assert_eq!(f.door_nearest(n, (cx, cy)), None);
        f.update_doors(&[], &[], 0.0);
        f.update_doors(&[], &[], 1.0);
        assert_eq!(f.door_in_way(cx, cy, 0.3), None, "an arch never blocks");
        assert_eq!(f.door_centre(cell), None);
        assert_eq!(f.toggle_door(cell, &[]), None);
        // the same prop as a door: listed, registered shut and in the way
        f.props[n].ptype = "normalRoomDoor".into();
        assert!(f.doors().any(|(i, _)| i == n));
        f.update_doors(&[], &[], 0.0);
        assert_eq!(f.door_in_way(cx, cy, 0.3), Some(cell));
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
    fn a_maze_variant_thins_only_what_the_data_says() {
        let mut seen_variant = false;
        for (key, mut f) in maze_chunks() {
            let doors = f.props.iter().filter(|p| p.door.is_some()).count();
            let (pickups, relays, batteries) = (f.pickups.len(), f.relays.len(), f.batteries.len());
            let v = f.apply_maze_variant(WORLD, key);
            assert_eq!(v.map(|v| v.id.as_str()), maze_regions::variant_at(WORLD, key.cx, key.cy, 0).map(|v| v.id.as_str()));
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
        for z in az - 3..az + 14 {
            for x in ax - 6..ax + 10 {
                f.wall[z as usize][x as usize] = false;
            }
        }
        f.props.retain(|p| !((ax - 7..ax + 11).contains(&p.cell.x) && (az - 4..az + 15).contains(&p.cell.y)));
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
        // a body's radius and the wall's half width to either side is a lane, not the divider (it runs along z for run (0, 1))
        assert!(!f.divider_blocks(mid.0 - 0.75, mid.1, 0.2));
        assert!(!f.divider_blocks(mid.0 + 0.75, mid.1, 0.2));
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
                // only a spiral's post is solid: the point that is its centre (1.0 across, 2.0 along from A's centre)
                let post = shape == Spiral && (dx, dz) == (1.0, 2.0);
                assert_eq!(g.divider_blocks(c.0 + dx, c.1 + dz, 0.2), post, "{shape:?} {dx} {dz}");
            }
        }
    }

    #[test]
    fn a_spiral_post_blocks_a_body_by_its_radius_and_only_a_spiral_has_one() {
        use crate::world::flight::FlightShape::*;
        for (level, dir) in [(0, -1), (-1, 1)] {
            let (f, s) = with_stair(level, dir, Spiral);
            let ((px, pz), pr) = s.post(level).expect("a spiral has a post");
            assert!((pr - 0.4).abs() < 1e-6);
            let r = 0.25;
            assert!(f.divider_blocks(px, pz, r));
            assert!(f.divider_blocks(px + pr + r - 0.02, pz, r), "inside the post's radius plus the body's");
            assert!(!f.divider_blocks(px + pr + r + 0.02, pz, r), "just outside it");
            // blocked on both levels, from every side
            for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                assert!(f.divider_blocks(px + dx * (pr + r - 0.02), pz + dz * (pr + r - 0.02), r));
                assert!(!f.body_fits_at_y(px + dx * (pr + r - 0.02), pz + dz * (pr + r - 0.02), level as f32 * 5.0, r));
            }
        }
        for shape in [Switchback, Straight] {
            let (_, s) = with_stair(0, -1, shape);
            assert!(s.post(0).is_none(), "{shape:?}");
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
                    // and a body is kept off a spiral's end radial, where the walker changes owner
                    if s.near_cliff(level, x, z, 0.3) {
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

    // -- railings along the water ----------------------------------------------------------------------------

    const RAIL_INSET: f32 = 0.06;
    const RAIL_THICKNESS: f32 = 0.07;

    /// A walkway row y = 10 (x 8..=14) over a walled pocket of "water" y = 11, fenced on its south side except at the
    /// gap x = 11.
    fn fenced_pocket() -> Facility {
        let mut f = Facility::open_test_level(0);
        f.wall[11][7] = true;
        f.wall[11][15] = true;
        for x in 8..=14usize {
            f.wall[12][x] = true;
        }
        let edges: Vec<(Cell, u8)> = (8..=14).filter(|x| *x != 11).map(|x| (Cell::new(x, 10), RAIL_S)).collect();
        f.set_rails(&edges, RAIL_INSET, RAIL_THICKNESS);
        f.rebuild_grids();
        f
    }

    #[test]
    fn a_body_walking_into_the_railing_from_the_walkway_stops_short_of_it() {
        let f = fenced_pocket();
        let line = 11.0 - RAIL_INSET;
        for r in [0.24, 0.33] {
            assert!(f.body_fits(9.5, 10.5, r), "r {r}: the walkway is open");
            assert!(!f.body_fits(9.5, line, r), "r {r}: the rail line is a wall");
            assert!(f.body_fits(9.5, 11.5, r), "r {r}: the pocket behind the rail is open floor");
            let end = f.clear_toward((9.5, 10.5), (9.5, 11.5), r);
            assert!(end.1 + r <= line - RAIL_THICKNESS * 0.5 + 1e-4, "r {r}: stopped at {end:?}");
            assert!(f.body_fits(end.0, end.1, r));
        }
        // along the railing is free: the walkway is a corridor
        assert_eq!(f.clear_toward((9.5, 10.5), (13.5, 10.5), 0.33), (13.5, 10.5));
        // a hunter-sized body walks the row beside its rail
        assert!(f.body_fits(9.5, 10.5, crate::config::HUNTER_RADIUS));
    }

    #[test]
    fn the_gap_in_the_railing_lets_a_body_through() {
        let f = fenced_pocket();
        for r in [0.24, 0.33] {
            assert_eq!(f.clear_toward((11.5, 10.5), (11.5, 11.5), r), (11.5, 11.5), "r {r}");
        }
        assert!(!f.rail_between(Cell::new(11, 10), Cell::new(11, 11)));
        assert!(f.rail_between(Cell::new(10, 10), Cell::new(10, 11)));
        assert!(f.rail_between(Cell::new(10, 11), Cell::new(10, 10)), "the edge is the same from the water side");
        assert!(!f.rail_between(Cell::new(10, 10), Cell::new(11, 10)), "along the walkway there is no rail");
        assert!(!f.rail_between(Cell::new(10, 10), Cell::new(11, 11)), "only orthogonal neighbours share an edge");
    }

    #[test]
    fn the_hunters_grid_has_no_edge_across_the_railing() {
        let f = fenced_pocket();
        for x in 8..=14 {
            assert!(f.hunter_open(Cell::new(x, 10)), "the walkway cell {x} stays open");
        }
        for x in [8, 9, 10, 12, 13, 14] {
            assert!(!f.hunter_open(Cell::new(x, 11)), "the water behind the rail at {x} is closed");
            assert!(f.find_path(Cell::new(x, 10), Cell::new(x, 11)).is_empty(), "no route across the rail at {x}");
        }
        assert!(f.hunter_open(Cell::new(11, 11)), "the gap is the way in");
        let path = f.find_path(Cell::new(9, 10), Cell::new(11, 11));
        assert!(!path.is_empty());
        for w in path.windows(2) {
            let across = w[0].y == 10 && w[1].y == 11 || w[0].y == 11 && w[1].y == 10;
            assert!(!across || (w[0].x == 11 && w[1].x == 11), "a step across the rail line at {w:?}");
        }
        // the same cell without a railing is open to a route
        let mut bare = fenced_pocket();
        bare.set_rails(&[], RAIL_INSET, RAIL_THICKNESS);
        bare.rebuild_grids();
        assert!(bare.hunter_open(Cell::new(9, 11)));
        assert_eq!(bare.find_path(Cell::new(9, 10), Cell::new(9, 11)).len(), 2);
    }

    #[test]
    fn a_distance_walk_goes_round_through_the_gap() {
        let f = fenced_pocket();
        let d = f.distance_map(Cell::new(9, 10));
        assert_eq!(d.get(&Cell::new(11, 11)), Some(&3), "two along the walkway, one through the gap");
        assert_eq!(d.get(&Cell::new(9, 11)), Some(&5), "back along the pocket");
        let n = f.nearest_open(9.5, 11.5, 0.24).expect("a spot");
        assert_eq!(n, (9.5, 11.5));
    }

    #[test]
    fn railings_are_kept_when_chunks_are_merged_and_shifted() {
        let part = fenced_pocket();
        let (cols, rows) = (part.cols, part.rows);
        let merged = Facility::merge(vec![(part, 5, 7, 0)], cols + 5, rows + 7, vec!["maze".into()], "RAILS-MERGE");
        assert_eq!(merged.rail_sides(Cell::new(19, 17)), RAIL_S);
        assert_eq!(merged.rail_sides(Cell::new(16, 17)), 0, "the gap column shifted too");
        assert!(!merged.body_fits(19.5, 17.95, 0.24), "the shifted rail is a wall");
        assert_eq!(merged.rail_edges().len(), 6);
    }

    #[test]
    fn a_railing_never_stops_a_level_without_one() {
        let f = Facility::open_test_level(0);
        assert!(f.rail_edges().is_empty());
        assert_eq!(f.rail_sides(Cell::new(10, 10)), 0);
        assert_eq!(f.clear_toward((9.5, 10.5), (9.5, 14.5), 0.33), (9.5, 14.5));
    }

    #[test]
    fn find_path_stats_reports_length_nodes_and_cost() {
        let f = Facility::open_test_level(0);
        let s = f.find_path_stats(Cell::new(5, 5), Cell::new(12, 5));
        assert_eq!(s.length, f.find_path(Cell::new(5, 5), Cell::new(12, 5)).len());
        assert!(s.length >= 8 && s.nodes_searched >= s.length && s.fail.is_none());
        assert!((s.cost - 7.0).abs() < 1e-3, "a straight run costs one per step");
    }

    #[test]
    fn find_path_stats_gives_a_reason_when_there_is_no_route() {
        let mut f = Facility::open_test_level(0);
        f.wall[10][10] = true;
        f.rebuild_grids();
        let s = f.find_path_stats(Cell::new(5, 5), Cell::new(10, 10));
        assert_eq!(s.length, 0);
        assert_eq!(s.fail, Some(PathFail::NoRoute));
        assert_eq!(s.fail.unwrap().label(), "NO ROUTE");
    }
}
