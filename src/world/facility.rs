//! The finite floor generator, ported draw-for-draw from
//! game/world/generation/facility_generator.gd: a 3x3 sector grid around an 11x11 lobby,
//! catalogue room types, a Kruskal graph of three-wide halls with up to four loops, seeded
//! room props, then the service lift, safe zone, relays, batteries and hiding spots. The same
//! seed text gives the same floor as the original (Godot's float32 Vector2 maths and its
//! unstable sort are reproduced where they decide placement).

use super::catalog::Catalog;
use super::godot_sort::sort_custom;
use super::rng::Rng;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

pub const HALL_WIDTH: i32 = 3;
pub const LOBBY_SIZE: i32 = 11;
pub const SECTOR_GAP: i32 = 2;
pub const SECTOR_MARGIN: i32 = 2;
pub const MIN_AUTHORED_ROOM_SIZE: i32 = 10;
pub const MAX_AUTHORED_ROOM_SIZE: i32 = 26;
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
}

/// Cells a flight runs down (or up) from its landing: 2 wide, this many long.
pub const STAIR_FLIGHT: i32 = 4;

/// The two cells of the well at step `k` (1..=STAIR_FLIGHT) from the landing.
pub fn stair_well_cells(s: Stair, k: i32) -> [Cell; 2] {
    // the second lane is always on the +x / +y side, so both ends of a flight agree on its well
    let side = (s.run.1.abs(), s.run.0.abs());
    let c = Cell::new(s.cell.x + s.run.0 * k, s.cell.y + s.run.1 * k);
    [c, Cell::new(c.x + side.0, c.y + side.1)]
}

#[derive(Clone, Copy, Debug)]
pub struct Stair {
    /// The landing cell where the runner steps on.
    pub cell: Cell,
    /// -1 leads down a level, +1 up.
    pub dir: i32,
    /// Unit step the flight runs along (from the landing into the well).
    pub run: (i32, i32),
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

    /// Recomputes the solid-prop index and the hunters' walkable grid after edits.
    pub fn rebuild_grids(&mut self) {
        self.hole = vec![false; (self.cols * self.rows) as usize];
        for s in self.stairs.clone() {
            for k in 1..=STAIR_FLIGHT {
                for c in stair_well_cells(s, k) {
                    if c.x >= 0 && c.y >= 0 && c.x < self.cols && c.y < self.rows {
                        self.hole[(c.y * self.cols + c.x) as usize] = true;
                    }
                }
            }
        }
        self.prop_grid = vec![vec![]; (self.cols * self.rows) as usize];
        for (i, p) in self.props.iter().enumerate() {
            if p.solid {
                self.prop_grid[(p.cell.y * self.cols + p.cell.x) as usize].push(i);
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
            f.stairs.extend(p.stairs.into_iter().map(|s| Stair {
                cell: sh(s.cell),
                ..s
            }));
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

    fn size_range(dim: i32, r: (f64, f64)) -> (i32, i32) {
        let clamp = |v: i32| {
            let max = MAX_AUTHORED_ROOM_SIZE.min(dim);
            MIN_AUTHORED_ROOM_SIZE.min(max).max(max.min(v))
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
            let (wl, wh) = Self::size_range(sw, class);
            let (hl, hh) = Self::size_range(sh, class);
            let dw = wl + self.random.next_index((wh - wl + 1) as usize) as i32;
            let dh = hl + self.random.next_index((hh - hl + 1) as usize) as i32;
            let wc = if t.landmark {
                sw
            } else {
                MIN_AUTHORED_ROOM_SIZE
                    .min(sw)
                    .max((sw as f64 * ROOM_SECTOR_FILL).floor() as i32)
            };
            let hc = if t.landmark {
                sh
            } else {
                MIN_AUTHORED_ROOM_SIZE
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
        let mut select = |a: usize,
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
                    let (px, py) = (p.cell.x as f64 + 0.5, p.cell.y as f64 + 0.5);
                    if (x - px).abs() < p.half.0 as f64 + r && (y - py).abs() < p.half.1 as f64 + r
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Movement test for bodies: the generator's rule plus the solid hiding lockers.
    pub fn is_hole(&self, x: i32, y: i32) -> bool {
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

    pub fn body_fits(&self, x: f32, y: f32, r: f32) -> bool {
        if !self.can_occupy(x as f64, y as f64, r as f64) {
            return false;
        }
        for (sx, sy) in [
            (x - r, y - r),
            (x + r, y - r),
            (x - r, y + r),
            (x + r, y + r),
            (x, y),
        ] {
            if self.is_hole(sx.floor() as i32, sy.floor() as i32) {
                return false;
            }
        }
        self.hiding.iter().all(|h| {
            let (lx, ly) = self.locker_center(h);
            (x - lx).abs() >= LOCKER_HALF + r || (y - ly).abs() >= LOCKER_HALF + r
        })
    }

    pub fn locker_center(&self, h: &Hiding) -> (f32, f32) {
        let (cx, cy) = h.cell.center();
        (cx - h.facing.0 as f32 * 0.18, cy - h.facing.1 as f32 * 0.18)
    }

    pub fn floor_cells(&self) -> Vec<Cell> {
        let mut v = vec![];
        for y in 1..self.rows - 1 {
            for x in 1..self.cols - 1 {
                if !self.wall[y as usize][x as usize] {
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

    fn add_prop(&mut self, p: Prop) {
        let idx = self.props.len();
        if p.solid {
            self.prop_grid[(p.cell.y * self.cols + p.cell.x) as usize].push(idx);
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
            let (ox, oy) = (o.cell.x as f64 + 0.5, o.cell.y as f64 + 0.5);
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
                open[(y * self.cols + x) as usize] =
                    self.body_fits(x as f32 + 0.5, y as f32 + 0.5, DEFAULT_AGENT_RADIUS as f32);
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
