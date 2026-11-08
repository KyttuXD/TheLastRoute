//! The minimap and the fullscreen discovered map (facility_map.gd), in design units: walls are
//! the backdrop colour, discovered floor the panel colour, relays teal-blue, batteries yellow,
//! the lift and stairwells, hunters on the radar in red, and the runner's wedge. On the Infinite
//! route the minimap follows the runner over a 36-cell square (STREAMED_MINIMAP_RADIUS_CELLS).
//! The fullscreen map on level 0 draws every discovered chunk, fitted to the discovered area
//! (`draw_level0`, Godot `_draw_streamed`'s full-map branch). The full map follows the runner
//! until the mouse drags it, zooms with the wheel (about the pointer) or F / G, recentres on
//! Space, and places a waypoint with a double click, a right click or E (Godot FacilityMap).
//!
//! Both maps draw only what the runner has seen. The sewer, the Inferno and level 0 all reveal the floor around the runner in line of
//! sight (`Game::reveal`, radius `reveal.radius_cells` in data/ui/map.json) and keep it by absolute cell; level 0's full map draws a
//! chunk's cells only where its seen bitmap (`game::discovery`) has them. On the sewer storeys the water cells the swim code reads
//! (`Facility::is_water`) are tinted blue once discovered, with a legend entry (data/ui/map.json).
//!
//! The hunters come from `Game::map_hunters`: Porcelle while awake, and every hunter on the floor
//! while the admin HUNTER REVEAL is on (the asleep ones dimmed). Each is marked by who it is
//! (`hunter_mark`): the capital letter of an A to Z hunter, the colour of a void hunter.

mod hunter_mark;
pub mod look;

use super::style::*;
use crate::app::assets::Assets;
use crate::game::endless::{FullMapFeed, MapChunk};
use crate::game::Game;
use crate::hunters::brain::Brain;
use super::markers::{self, Marker};
use crate::world::infinite::CHUNK;
use macroquad::prelude::*;

pub const RELAY_TEAL_BLUE: Color = hex(0x32c9d8);
pub const BATTERY: Color = rgba(0.95, 0.82, 0.25, 1.0);
pub const EXIT: Color = rgba(0.76, 0.55, 0.96, 1.0);
pub const STAIRS: Color = hex(0xf4f7f6);
pub const RUNNER: Color = hex(0x63d9f1);
/// The waypoint route (Godot WaypointGrid trail tint) and its pin (the amber own-pin colour).
pub const WAYPOINT_ROUTE: Color = rgba(0.18, 0.92, 0.84, 0.72);
pub const WAYPOINT_PIN: Color = rgba(1.0, 0.72, 0.2, 1.0);

/// The relay dot: web orange #ff7700 on Last Route, the City's teal-blue RELAY_TEAL_BLUE.
pub fn relay_colour() -> Color {
    lr(hex(0xff7700), RELAY_TEAL_BLUE)
}

/// The battery dot: web yellow #ffe600 on Last Route, BATTERY on the City.
fn battery_colour() -> Color {
    lr(hex(0xffe600), BATTERY)
}

/// The runner's wedge: web white #ffffff on Last Route, RUNNER on the City.
fn runner_colour() -> Color {
    lr(hex(0xffffff), RUNNER)
}

/// The waypoint pin: web gold #f2bb62 on Last Route, WAYPOINT_PIN on the City.
fn waypoint_pin_colour() -> Color {
    lr(hex(0xf2bb62), WAYPOINT_PIN)
}
/// A stairwell's floor on the map (the window's hole colour).
const HOLE: Color = hex(0x2f5a52);
/// A solid prop's collision box on the full map (facility_map.gd PROP_BOX_TINT).
const PROP_BOX: Color = hex(0x7899a1);
/// Godot FacilityMap's zoom: one wheel notch, F and G, and the zoom range (MIN_MAP_ZOOM, MAX_MAP_ZOOM).
const ZOOM_STEP: f32 = 1.22;
const MIN_FULL_ZOOM: f32 = 0.75;
const MAX_FULL_ZOOM: f32 = 8.0;
/// Godot's full-map keyboard pan, in design units a second (the stick and the move keys).
const PAN_SPEED: f32 = 120.0;
/// Two left presses this close in time (seconds) and in place (design units) are a double click.
const DOUBLE_CLICK_SECS: f64 = 0.5;
const DOUBLE_CLICK_UNITS: f32 = 8.0;

/// The cells a map view covers: the whole floor, or a 36-cell square around the runner on the
/// Infinite route's minimap.
pub fn view_cells(g: &Game, full: bool) -> (i32, i32, i32, i32) {
    if full || !g.is_infinite() {
        return (0, 0, g.fac.cols, g.fac.rows);
    }
    let n = 36.min(g.fac.cols).min(g.fac.rows);
    let x0 = (g.p.pos.0 as i32 - n / 2).clamp(0, g.fac.cols - n);
    let y0 = (g.p.pos.1 as i32 - n / 2).clamp(0, g.fac.rows - n);
    (x0, y0, n, n)
}

/// A fitted plan: the size of one map cell on the plan, and where map cell (0, 0) lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub cell: f32,
    pub ox: f32,
    pub oy: f32,
}

impl Xf {
    /// The plan position of a map cell point.
    pub fn at(&self, c: (f32, f32)) -> (f32, f32) {
        (self.ox + c.0 * self.cell, self.oy + c.1 * self.cell)
    }

    /// The map cell point under a plan position (the inverse of `at`).
    pub fn cell_at(&self, p: Vec2) -> (f32, f32) {
        let cell = self.cell.max(1e-6);
        ((p.x - self.ox) / cell, (p.y - self.oy) / cell)
    }
}

/// The whole of a `w` x `h` cell floor fitted into `plan`, centred: the smaller side sets the cell size.
fn fit_box(plan: Rect, w: i32, h: i32) -> Xf {
    let cell = (plan.w / w as f32).min(plan.h / h as f32);
    Xf {
        cell,
        ox: plan.x + (plan.w - cell * w as f32) * 0.5,
        oy: plan.y + (plan.h - cell * h as f32) * 0.5,
    }
}

/// Draws the minimap or a floor map into `r` (units), with the players and bots in `marks` as pins.
pub fn draw_map(g: &Game, a: &Assets, r: Rect, full: bool, marks: &[Marker]) {
    let (vx, vy, vw, vh) = view_cells(g, full);
    let b = fit_box(r, vw, vh);
    let xf = Xf {
        cell: b.cell,
        ox: b.ox - vx as f32 * b.cell,
        oy: b.oy - vy as f32 * b.cell,
    };
    draw_floor(g, a, xf, (vx, vy, vw, vh), full, marks);
    radar_state_overlay(g, r);
}

/// The radar state over a map panel (void_player_effects.gd `hud.set_radar_state`): a blackout or an offline radar blanks
/// the panel; static flickers noise across it while the hunters still show.
fn radar_state_overlay(g: &Game, r: Rect) {
    if g.radar_dark() {
        draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.0, 0.0, 0.0, 1.0));
    } else if g.radar_static() {
        let t = macroquad::time::get_time() as f32;
        for i in 0..36 {
            let f = ((i as f32 * 12.9898 + (t * 24.0).floor() * 78.233).sin() * 43758.5453).fract().abs();
            draw_rectangle(r.x, r.y + f * r.h, r.w, 1.5, Color::new(0.85, 0.9, 0.9, 0.22));
        }
    }
}

/// Draws the floor cells `cells` (x, y, w, h) through `xf`: the floor, the safe zone, relays, batteries, the lift, the
/// stairs, the waypoint, the hunters on the map, the teammates and the runner.
fn draw_floor(g: &Game, a: &Assets, xf: Xf, cells: (i32, i32, i32, i32), full: bool, marks: &[Marker]) {
    let p = pal();
    let (vx, vy, vw, vh) = cells;
    let cell = xf.cell;
    let (bx, by) = xf.at((vx as f32, vy as f32));
    fill(Rect::new(bx, by, vw as f32 * cell, vh as f32 * cell), p.backdrop);
    // the sewer's water: a discovered water cell is the floor colour moved toward the blue (data/ui/map.json)
    let water_col = look::MapLook::load().water.over(p.panel);
    for y in vy..vy + vh {
        for x in vx..vx + vw {
            if !g.discovered[y as usize][x as usize] || g.fac.is_wall_cell(x, y) {
                continue;
            }
            let col = if g.fac.is_hole(x, y) {
                HOLE
            } else if g.fac.is_water(crate::world::Cell::new(x, y)) {
                water_col
            } else {
                p.panel
            };
            let (qx, qy) = xf.at((x as f32, y as f32));
            fill(Rect::new(qx, qy, cell + 0.02, cell + 0.02), col);
        }
    }
    let inside = |c: (f32, f32)| {
        c.0 >= vx as f32 && c.1 >= vy as f32 && c.0 < (vx + vw) as f32 && c.1 < (vy + vh) as f32
    };
    let seen = |c: crate::world::Cell| {
        c.x >= 0
            && c.y >= 0
            && c.x < g.fac.cols
            && c.y < g.fac.rows
            && g.discovered[c.y as usize][c.x as usize]
    };
    let dot = |c: (f32, f32), col: Color, rad: f32| {
        if inside(c) {
            let q = xf.at(c);
            circle(vec2(q.0, q.1), rad, col);
        }
    };
    if let Some(sz) = g.fac.safe_zone {
        if seen(sz) {
            dot(sz.center(), meter_safe(), cell * 0.9);
        }
    }
    // a relay or battery is drawn once its marker is discovered (Game::map_relay_shown; level 0 only)
    for (i, rl) in g.fac.relays.iter().enumerate() {
        if seen(*rl) && !g.relays_taken[i] && g.map_relay_shown(i) {
            dot(rl.center(), relay_colour(), (cell * 0.34).max(0.5));
        }
    }
    for (i, b) in g.fac.batteries.iter().enumerate() {
        if seen(*b) && !g.batteries_taken[i] && g.map_battery_shown(i) {
            dot(b.center(), battery_colour(), (cell * 0.26).max(0.4));
        }
    }
    if g.fac.has_lift && seen(g.fac.exit) {
        dot(g.fac.exit.center(), EXIT, cell * 0.4);
    }
    for s in &g.fac.stairs {
        if seen(s.cell) && inside(s.cell.center()) {
            let q = xf.at(s.cell.center());
            stair_icon(vec2(q.0, q.1), cell, full);
        }
    }
    // the runner's waypoint: the route to it (the full route, Game::waypoint_route) and its pin (Game::waypoint_marker)
    for w in g.waypoint_route().windows(2) {
        if inside(w[0]) && inside(w[1]) {
            let (a, b) = (xf.at(w[0]), xf.at(w[1]));
            line(
                vec2(a.0, a.1),
                vec2(b.0, b.1),
                (cell * 0.18).max(0.8),
                WAYPOINT_ROUTE,
            );
        }
    }
    if let Some(at) = g.waypoint_marker() {
        dot(at, waypoint_pin_colour(), (cell * 0.5).max(1.5));
    }
    let radius = (cell * 0.55).max(if full { 3.0 } else { 1.8 });
    let placed = hunter_marks(g, |h| inside(h.pos).then(|| xf.at(h.pos)), radius);
    hunter_mark::draw(a, &placed, full);
    porcelle_rings(g, |h| inside(h.pos).then(|| xf.at(h.pos)), radius, cell);
    draw_runner_pins(xf, cells, (0, 0), marks, g.fac.level, full);
    let q = xf.at(g.p.pos);
    runner_wedge(g, vec2(q.0, q.1), cell, full);
}

/// How visible a hunter's mark is on the maps (0..1): full while awake, dimmed while asleep (the admin reveal shows the
/// sleeping ones too). The dot, and the letter or colour that names it, share this one factor, so who a hunter is never shows
/// where its position does not.
fn hunter_alpha(h: &Brain) -> f32 {
    let base = if h.active { 1.0 } else { 0.4 };
    // Porcelle's radar ping (Godot RadarPing): strength gates alpha; reduced motion
    // keeps a steady dim via style::reduced_motion().
    if h.def.id == "porcelle" {
        let s = super::radar_ping::sample(macroquad::time::get_time() as f32, super::style::reduced_motion());
        if s.strength <= 0.001 {
            return 0.0;
        }
        return base * s.strength;
    }
    base
}

/// The hunters the maps show (`Game::map_hunters`), each as a mark that names it, placed by `place` (the plan point of a hunter
/// on this view, or `None` off it). `radius` is the dot's size.
fn hunter_marks(g: &Game, place: impl Fn(&Brain) -> Option<(f32, f32)>, radius: f32) -> Vec<hunter_mark::Placed> {
    g.map_hunters()
        .filter_map(|h| {
            let alpha = hunter_alpha(h);
            if alpha <= 0.001 {
                return None;
            }
            Some(hunter_mark::Placed { mark: hunter_mark::mark_of(&h.def), at: place(h)?, radius, alpha })
        })
        .collect()
}

/// Porcelle's radar ping ring around her dot (Godot `_draw_ghost_ping`), placed like `hunter_marks` places the dot.
fn porcelle_rings(g: &Game, place: impl Fn(&Brain) -> Option<(f32, f32)>, radius: f32, cell: f32) {
    let reduced = super::style::reduced_motion();
    let s = super::radar_ping::sample(macroquad::time::get_time() as f32, reduced);
    let Some(ring) = super::radar_ping::ring(&s, radius, cell, reduced) else { return };
    for h in g.map_hunters().filter(|h| h.def.id == "porcelle" && hunter_alpha(h) > 0.001) {
        if let Some(at) = place(h) {
            super::radar_ping::draw_ring(at, &ring, pal().amber);
        }
    }
}

/// Godot PlayerColors' palette (Okabe-Ito plus Tol extensions), the colour of a teammate's pin. Godot ranks the peers of the
/// room for a slot; the Rust map has no roster in view, so the slot is the peer id.
const TEAMMATE_PALETTE: [Color; 12] = [
    hex(0xe69f00),
    hex(0x56b4e9),
    hex(0x009e73),
    hex(0xf0e442),
    hex(0x0072b2),
    hex(0xd55e00),
    hex(0xcc79a7),
    hex(0xb8b8b8),
    hex(0x882255),
    hex(0x88ccee),
    hex(0xddcc77),
    hex(0xaa4499),
];
/// The ink that rings every teammate arrow (Godot `_draw_remote_player`'s ink).
const TEAMMATE_INK: Color = rgba(0.01, 0.04, 0.05, 0.95);

/// The colour of a teammate's pin, by peer id.
pub fn teammate_colour(peer: u32) -> Color {
    TEAMMATE_PALETTE[peer as usize % TEAMMATE_PALETTE.len()]
}

/// The map cell of a teammate's runner: its `RemoteView` position moved by `origin` into the frame the map is drawn in (the window
/// origin on the full map's level 0, else (0, 0)). It is the same cell frame as the runner's own `Game::p.pos`.
pub fn runner_cell(origin: (i32, i32), at: (f32, f32)) -> (f32, f32) {
    (at.0 + origin.0 as f32, at.1 + origin.1 as f32)
}

/// The plan point of a teammate's runner through the frame `xf`.
pub fn runner_pin_at(xf: Xf, origin: (i32, i32), at: (f32, f32)) -> (f32, f32) {
    xf.at(runner_cell(origin, at))
}

/// Draws the players and bots as arrows on a map (Godot `_draw_remote_player`): each along its facing, ringed in ink, in its
/// own colour. Radius is the larger of 3.4 (the full map) or 3.0 (the minimap) and half a cell, as Godot takes it. `marks`
/// are the markers from [`markers::remote_markers`] and [`markers::bot_markers`], `origin` moves them into the frame of `xf`
/// and `cells` (x, y, w, h) is what is shown, in that same frame. A marker on `shown_level` is drawn whole; one on another
/// level is dimmed on the full map and left off the minimap ([`markers::pin_alpha`]). A marker outside `cells` is not drawn;
/// Godot's border arrows are not ported yet.
pub fn draw_runner_pins(
    xf: Xf,
    cells: (i32, i32, i32, i32),
    origin: (i32, i32),
    marks: &[Marker],
    shown_level: i32,
    full: bool,
) {
    let (vx, vy, vw, vh) = cells;
    let cell = xf.cell;
    let floor: f32 = if full { 3.4 } else { 3.0 };
    let radius = floor.max(cell * 0.5);
    let len = radius * 1.55;
    let half = radius * 1.05;
    for r in marks {
        let Some(alpha_k) = markers::pin_alpha(r.level, shown_level, full) else { continue };
        let c = runner_cell(origin, (r.x, r.y));
        let inside = c.0 >= vx as f32
            && c.1 >= vy as f32
            && c.0 < (vx + vw) as f32
            && c.1 < (vy + vh) as f32;
        if !inside {
            continue;
        }
        // Player::forward2, as the runner's wedge takes it; a zero heading points up
        let facing = super::markers::facing_dir(r.yaw).normalize_or_zero();
        let f = if facing == Vec2::ZERO { vec2(0.0, -1.0) } else { facing };
        let side = vec2(-f.y, f.x);
        let p = runner_pin_at(xf, origin, (r.x, r.y));
        let at = vec2(p.0, p.1);
        let arrow = |grow: f32, col: Color| {
            let (l, h) = (len + grow, half + grow);
            let tip = at + f * l;
            let left = at - f * l * 0.7 + side * h;
            let notch = at - f * len * 0.25;
            let right = at - f * l * 0.7 - side * h;
            tri(tip, left, notch, col);
            tri(tip, notch, right, col);
        };
        arrow(1.1, alpha(TEAMMATE_INK, TEAMMATE_INK.a * alpha_k));
        arrow(0.0, alpha(r.colour, r.colour.a * alpha_k));
    }
}

/// The fit of the discovered chunks on a plan: the discovered area in absolute cells, the size of one cell in the plan, and
/// where absolute cell (0, 0) lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// The discovered area's top-left, in absolute cells.
    pub x0: i32,
    pub y0: i32,
    /// The discovered area's size, in cells.
    pub w: i32,
    pub h: i32,
    /// The plan size of one cell.
    pub cell: f32,
    /// The plan position of absolute cell (0, 0).
    pub ox: f32,
    pub oy: f32,
}

impl Fit {
    /// The fit as a plan transform.
    pub fn xf(&self) -> Xf {
        Xf {
            cell: self.cell,
            ox: self.ox,
            oy: self.oy,
        }
    }
}

/// Godot `_draw_streamed`'s full-map fit (facility_map.gd ~774-830) for the discovered chunks `keys` (chunk coordinates
/// (cx, cy)) on the plan `plan`. The area is the bounding box of the chunks, in chunks, as Godot takes it. The cell is the
/// smaller of the plan's width over the area's width and its height over the area's height, and the area is centred in the
/// plan. `None` with no chunks.
pub fn fit_discovered(plan: Rect, keys: &[(i32, i32)]) -> Option<Fit> {
    let (&(x, y), rest) = keys.split_first()?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (x, x, y, y);
    for &(cx, cy) in rest {
        min_x = min_x.min(cx);
        max_x = max_x.max(cx);
        min_y = min_y.min(cy);
        max_y = max_y.max(cy);
    }
    let (x0, y0) = (min_x * CHUNK, min_y * CHUNK);
    let (w, h) = ((max_x - min_x + 1) * CHUNK, (max_y - min_y + 1) * CHUNK);
    let cell = (plan.w / w as f32).min(plan.h / h as f32);
    let ox = plan.x + (plan.w - cell * w as f32) * 0.5 - x0 as f32 * cell;
    let oy = plan.y + (plan.h - cell * h as f32) * 0.5 - y0 as f32 * cell;
    Some(Fit {
        x0,
        y0,
        w,
        h,
        cell,
        ox,
        oy,
    })
}

/// The floor colour of chunk-local cell (`x`, `z`) on the full map: `None` for a wall, the stairwell colour for a hole,
/// the panel colour otherwise.
fn floor_tint(mc: &MapChunk, z: i32, x: i32, panel: Color) -> Option<Color> {
    let i = (z * CHUNK + x) as usize;
    if mc.walls[i] {
        None
    } else if mc.holes[i] {
        Some(HOLE)
    } else {
        Some(panel)
    }
}

/// The floor colour of chunk-local cell (`x`, `z`) on the level-0 full map, or `None` for a wall and for a cell the runner has not
/// seen (`bits` is the chunk's seen bitmap): an unexplored cell stays the backdrop.
fn seen_floor_tint(mc: &MapChunk, bits: &crate::game::discovery::Bits, z: i32, x: i32, panel: Color) -> Option<Color> {
    if bits.get(x, z) {
        floor_tint(mc, z, x, panel)
    } else {
        None
    }
}

/// A stairwell's icon at plan point `c` (the same drawing on the minimap and the full map).
fn stair_icon(c: Vec2, cell: f32, full: bool) {
    let rad = (cell * 0.62).max(if full { 3.5 } else { 1.5 });
    let th = (cell * 0.13).max(0.4);
    let pts = [
        vec2(-1.0, 0.55),
        vec2(-0.35, 0.55),
        vec2(-0.35, 0.0),
        vec2(0.25, 0.0),
        vec2(0.25, -0.55),
        vec2(1.0, -0.55),
    ];
    for w in pts.windows(2) {
        line(c + w[0] * rad, c + w[1] * rad, th, STAIRS);
    }
}

/// The runner's wedge at plan point `at`, pointing along the runner's heading (outlined on the full map).
fn runner_wedge(g: &Game, at: Vec2, cell: f32, full: bool) {
    let (fx, fy) = g.forward2();
    let f = vec2(fx, fy).normalize_or_zero();
    let side = vec2(-f.y, f.x);
    let reach = if full { 7.0 } else { (cell * 1.8).max(2.5) };
    let (a, b, c) = (
        at + f * reach,
        at - f * reach * 0.5 + side * reach * 0.6,
        at - f * reach * 0.5 - side * reach * 0.6,
    );
    if full {
        let w = (reach * 0.22).max(1.0);
        let o = rgba(0.01, 0.04, 0.05, 0.95);
        line(a, b, w, o);
        line(b, c, w, o);
        line(c, a, w, o);
    }
    tri(a, b, c, runner_colour());
}

/// The full map's view (Godot FacilityMap's `map_zoom`, `follow_player` and `_view_center`): the zoom multiplies the fitted cell
/// size, and `centre` is the map cell at the plan's middle once a pan has turned following off (`None` follows the runner).
/// The zoom outlives closing the map (Godot's remembered zoom); the rest is reset when it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FullView {
    pub zoom: f32,
    pub centre: Option<(f32, f32)>,
    /// The last left press inside the plan (its time and point), for the double click.
    last_press: Option<(f64, Vec2)>,
    /// The point the held left button last reached, while it drags the map.
    drag: Option<Vec2>,
}

impl Default for FullView {
    fn default() -> Self {
        FullView {
            zoom: 1.0,
            centre: None,
            last_press: None,
            drag: None,
        }
    }
}

impl FullView {
    /// Follows the runner again (Godot `recenter`): the zoom stays, the drag and the double click are forgotten.
    pub fn recentre(&mut self) {
        self.centre = None;
        self.drag = None;
        self.last_press = None;
    }
}

/// One frame of the full map's input: the pointer in design units, the buttons and wheel, the held pan keys and the keys
/// that zoom, recentre or place, the clock for the double click, and the frame's seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct MapPointer {
    pub at: Vec2,
    pub left_pressed: bool,
    pub left_down: bool,
    pub right_pressed: bool,
    /// Wheel notches: above zero scrolls up, which zooms in.
    pub wheel: f32,
    /// Held pan keys: x right, y down, each -1..1.
    pub pan: Vec2,
    pub zoom_in: bool,
    pub zoom_out: bool,
    pub recentre: bool,
    /// E: a waypoint at the middle of the map.
    pub place_centre: bool,
    /// The clock in seconds, for the double click.
    pub now: f64,
    pub dt: f32,
}

/// The full map's frame for one frame: the transform the picture is drawn with (the fit at the view's zoom and centre), the fit
/// at zoom 1, the window's origin in map cells (level 0's absolute cells, or (0, 0) on a floor) and the runner's map cell.
#[derive(Clone, Copy, Debug)]
pub struct FullPlan {
    pub xf: Xf,
    pub base_cell: f32,
    pub origin: (i32, i32),
    pub runner: (f32, f32),
}

impl FullPlan {
    /// The window position (the `Game::place_waypoint` frame) of a plan point.
    fn window_at(&self, p: Vec2) -> (f32, f32) {
        let c = self.xf.cell_at(p);
        (c.0 - self.origin.0 as f32, c.1 - self.origin.1 as f32)
    }
}

/// The fullscreen map's plan rectangle in design units (the picture's frame under the title and above the legend).
fn full_plan_rect() -> Rect {
    let v = view();
    Rect::new(12.0, 31.0, v.x - 24.0, v.y - 56.0)
}

/// The full map's frame for `plan` at the view's zoom and centre: the level-0 discovered fit when the feed has it, else the whole
/// floor (or the window) fitted and centred.
pub fn full_plan(g: &Game, plan: Rect, map_view: &FullView) -> FullPlan {
    let (base, origin) = match g.full_map_feed() {
        Some(feed) => {
            let keys: Vec<(i32, i32)> = feed.chunks.iter().map(|(k, _)| (k.cx, k.cy)).collect();
            match fit_discovered(plan, &keys) {
                Some(f) => (f.xf(), feed.origin),
                None => (fit_box(plan, g.fac.cols, g.fac.rows), (0, 0)),
            }
        }
        None => (fit_box(plan, g.fac.cols, g.fac.rows), (0, 0)),
    };
    let cell = base.cell * map_view.zoom;
    let runner = (
        g.p.pos.0 + origin.0 as f32,
        g.p.pos.1 + origin.1 as f32,
    );
    let c = map_view.centre.unwrap_or(runner);
    let xf = Xf {
        cell,
        ox: plan.x + plan.w * 0.5 - c.0 * cell,
        oy: plan.y + plan.h * 0.5 - c.1 * cell,
    };
    FullPlan {
        xf,
        base_cell: base.cell,
        origin,
        runner,
    }
}

/// Moves the view by a pointer-sized step on the plan (a drag moves the map with the pointer): the view leaves the runner.
fn pan_by(map_view: &mut FullView, geo: &FullPlan, screen_delta: Vec2) {
    let c = map_view.centre.unwrap_or(geo.runner);
    map_view.centre = Some((
        c.0 - screen_delta.x / geo.xf.cell,
        c.1 - screen_delta.y / geo.xf.cell,
    ));
}

/// Scales the zoom by `factor` and keeps the map cell under `focus` where it is (Godot `zoom_at`). Following the runner stays
/// on the runner.
fn zoom_about(map_view: &mut FullView, geo: &FullPlan, mid: Vec2, focus: Vec2, factor: f32) {
    let next = (map_view.zoom * factor).clamp(MIN_FULL_ZOOM, MAX_FULL_ZOOM);
    if (next - map_view.zoom).abs() < 1e-6 {
        return;
    }
    let under = geo.xf.cell_at(focus);
    map_view.zoom = next;
    if map_view.centre.is_some() {
        let cell = geo.base_cell * next;
        map_view.centre = Some((
            under.0 - (focus.x - mid.x) / cell,
            under.1 - (focus.y - mid.y) / cell,
        ));
    }
}

/// The full map's input for one frame (Godot FacilityMap `_gui_input` and `_process`): a held left button drags the map, which
/// stops following the runner; a double click or a right click inside the plan places the waypoint under the pointer, E places
/// it at the middle; the wheel zooms about the pointer and F / G about the middle; Space recentres; the move keys pan.
/// Returns the window position a waypoint is placed at this frame, if one was asked for.
pub fn step(map_view: &mut FullView, geo: &FullPlan, plan: Rect, p: &MapPointer) -> Option<(f32, f32)> {
    let mid = vec2(plan.x + plan.w * 0.5, plan.y + plan.h * 0.5);
    let inside = plan.contains(p.at);
    let mut placed = None;

    if p.left_pressed && inside {
        let double = map_view.last_press.is_some_and(|(t, q)| {
            p.now - t <= DOUBLE_CLICK_SECS && q.distance(p.at) <= DOUBLE_CLICK_UNITS
        });
        if double {
            map_view.last_press = None;
            map_view.drag = None;
            placed = Some(geo.window_at(p.at));
        } else {
            map_view.last_press = Some((p.now, p.at));
            map_view.drag = Some(p.at);
        }
    }
    if p.right_pressed && inside {
        placed = placed.or(Some(geo.window_at(p.at)));
    }
    if p.place_centre {
        placed = placed.or(Some(geo.window_at(mid)));
    }

    if !p.left_down {
        map_view.drag = None;
    } else if let Some(prev) = map_view.drag {
        let d = p.at - prev;
        if d != Vec2::ZERO {
            pan_by(map_view, geo, d);
        }
        map_view.drag = Some(p.at);
    }
    if p.pan != Vec2::ZERO {
        pan_by(map_view, geo, -p.pan * PAN_SPEED * p.dt);
    }
    if p.recentre {
        map_view.recentre();
    }

    let mut factor = 1.0_f32;
    if p.wheel > 0.0 {
        factor *= ZOOM_STEP;
    } else if p.wheel < 0.0 {
        factor /= ZOOM_STEP;
    }
    if p.zoom_in {
        factor *= ZOOM_STEP;
    }
    if p.zoom_out {
        factor /= ZOOM_STEP;
    }
    if factor != 1.0 {
        let focus = if p.wheel != 0.0 && inside { p.at } else { mid };
        zoom_about(map_view, geo, mid, focus, factor);
    }
    placed
}

/// The full map's input applied to the game: reads the pointer, wheel and keys this frame, steps the view, and places the
/// waypoint it asks for (`Game::place_waypoint`). Call it only while the full map is open and the run is not paused.
pub fn full_map_input(g: &mut Game, map_view: &mut FullView) {
    let plan = full_plan_rect();
    let geo = full_plan(g, plan, map_view);
    let axis = |neg: bool, pos: bool| (pos as i32 - neg as i32) as f32;
    let mut pan = vec2(
        axis(
            is_key_down(KeyCode::A) || is_key_down(KeyCode::Left),
            is_key_down(KeyCode::D) || is_key_down(KeyCode::Right),
        ),
        axis(
            is_key_down(KeyCode::W) || is_key_down(KeyCode::Up),
            is_key_down(KeyCode::S) || is_key_down(KeyCode::Down),
        ),
    );
    if pan.length() > 1.0 {
        pan = pan.normalize();
    }
    let p = MapPointer {
        at: mouse_u(),
        left_pressed: is_mouse_button_pressed(MouseButton::Left),
        left_down: is_mouse_button_down(MouseButton::Left),
        right_pressed: is_mouse_button_pressed(MouseButton::Right),
        wheel: mouse_wheel().1,
        pan,
        zoom_in: is_key_pressed(KeyCode::F),
        zoom_out: is_key_pressed(KeyCode::G),
        recentre: is_key_pressed(KeyCode::Space),
        place_centre: is_key_pressed(KeyCode::E),
        now: get_time(),
        dt: get_frame_time(),
    };
    if let Some(w) = step(map_view, &geo, plan, &p) {
        g.place_waypoint(w);
    }
}

/// The level-0 full map: every discovered chunk (`feed`), drawn through the frame `geo`. The floor is drawn per chunk in runs
/// of cells, then the solid props, the relays and batteries, the stairs, the waypoint, the hunters on the map and the runner.
/// Off the discovered area nothing is drawn.
fn draw_level0(g: &Game, a: &Assets, feed: &FullMapFeed, plan: Rect, geo: &FullPlan, marks: &[Marker]) {
    let p = pal();
    let keys: Vec<(i32, i32)> = feed.chunks.iter().map(|(k, _)| (k.cx, k.cy)).collect();
    let Some(fit) = fit_discovered(plan, &keys) else { return };
    let xf = geo.xf;
    let cell = xf.cell;
    let (x1, y1) = (fit.x0 + fit.w, fit.y0 + fit.h);
    let (bx0, by0) = xf.at((fit.x0 as f32, fit.y0 as f32));
    fill(
        Rect::new(bx0, by0, fit.w as f32 * cell, fit.h as f32 * cell),
        p.backdrop,
    );
    // each chunk's open floor, as runs of cells along each row (one fill per run, not per cell)
    // only the cells the runner has seen (the chunk's bitmap), so an unexplored cell stays the backdrop
    for (k, mc) in &feed.chunks {
        let (bx, by) = (k.cx * CHUNK, k.cy * CHUNK);
        let Some(bits) = feed.seen.chunk((k.cx, k.cy)) else { continue };
        let tint = |z: i32, x: i32| seen_floor_tint(mc, bits, z, x, p.panel);
        for z in 0..CHUNK {
            let mut x = 0;
            while x < CHUNK {
                let Some(col) = tint(z, x) else {
                    x += 1;
                    continue;
                };
                let start = x;
                while x < CHUNK && tint(z, x) == Some(col) {
                    x += 1;
                }
                let (qx, qy) = xf.at(((bx + start) as f32, (by + z) as f32));
                fill(
                    Rect::new(qx, qy, (x - start) as f32 * cell + 0.02, cell + 0.02),
                    col,
                );
            }
        }
    }
    let shown = |c: (f32, f32)| {
        c.0 >= fit.x0 as f32 && c.1 >= fit.y0 as f32 && c.0 < x1 as f32 && c.1 < y1 as f32
    };
    let dot = |c: (f32, f32), col: Color, rad: f32| {
        if shown(c) {
            let q = xf.at(c);
            circle(vec2(q.0, q.1), rad, col);
        }
    };
    // a mark on the map is drawn only where the runner has seen the cell under it
    let seen = |c: crate::world::Cell| feed.seen.is_seen(c.x, c.y);
    // the window's safe zone (the middle chunk's safe room while intact), in absolute cells
    if let Some(sz) = g.fac.safe_zone {
        let o = feed.origin;
        let at = crate::world::Cell::new(sz.x + o.0, sz.y + o.1);
        if seen(at) {
            dot(at.center(), meter_safe(), cell * 0.9);
        }
    }
    for (_, mc) in &feed.chunks {
        for pr in &mc.props {
            if !seen(pr.cell) || !g.full_map_prop_shown(&pr.id) {
                continue;
            }
            let c = pr.cell.center();
            let (hx, hy) = pr.half;
            let tl = xf.at((c.0 - hx, c.1 - hy));
            fill(Rect::new(tl.0, tl.1, hx * 2.0 * cell, hy * 2.0 * cell), PROP_BOX);
        }
    }
    for (_, mc) in &feed.chunks {
        for (id, c) in &mc.relays {
            if seen(*c) && g.full_map_pickup_shown(id) {
                dot(c.center(), relay_colour(), (cell * 0.34).max(0.5));
            }
        }
        for (id, c) in &mc.batteries {
            if seen(*c) && g.full_map_pickup_shown(id) {
                dot(c.center(), battery_colour(), (cell * 0.26).max(0.4));
            }
        }
    }
    for (_, mc) in &feed.chunks {
        for s in &mc.stairs {
            let c = s.cell.center();
            if shown(c) && seen(s.cell) {
                let q = xf.at(c);
                stair_icon(vec2(q.0, q.1), cell, true);
            }
        }
    }
    // the waypoint route and its pin, both absolute
    for w in feed.route.windows(2) {
        if shown(w[0]) && shown(w[1]) {
            let (a, b) = (xf.at(w[0]), xf.at(w[1]));
            line(
                vec2(a.0, a.1),
                vec2(b.0, b.1),
                (cell * 0.18).max(0.8),
                WAYPOINT_ROUTE,
            );
        }
    }
    if let Some(at) = feed.waypoint {
        dot(at, waypoint_pin_colour(), (cell * 0.5).max(1.5));
    }
    let o = (feed.origin.0 as f32, feed.origin.1 as f32);
    let placed = hunter_marks(
        g,
        |h| {
            let c = (h.pos.0 + o.0, h.pos.1 + o.1);
            shown(c).then(|| xf.at(c))
        },
        (cell * 0.55).max(3.0),
    );
    hunter_mark::draw(a, &placed, true);
    porcelle_rings(
        g,
        |h| {
            let c = (h.pos.0 + o.0, h.pos.1 + o.1);
            shown(c).then(|| xf.at(c))
        },
        (cell * 0.55).max(3.0),
        cell,
    );
    draw_runner_pins(xf, (fit.x0, fit.y0, fit.w, fit.h), feed.origin, marks, g.fac.level, true);
    let q = xf.at(geo.runner);
    runner_wedge(g, vec2(q.0, q.1), cell, true);
    radar_state_overlay(g, plan);
}

/// The legend's font size: 9, or smaller so a line that is `width_at_9` wide at size 9 fits `screen_w` with a margin each side.
fn legend_size(width_at_9: f32, screen_w: f32) -> f32 {
    let room = (screen_w - 16.0).max(1.0);
    if width_at_9 <= room {
        9.0
    } else {
        9.0 * room / width_at_9
    }
}

/// The full map's legend line: the marks it can show, and the water on a floor that has any (the sewers). Built once per colour
/// scheme and floor kind, so the full map's frame does not format a string.
fn legend_text(g: &Game) -> &'static str {
    const MARKS: [&str; 2] = [
        "ORANGE relay  •  YELLOW battery  •  GRAY prop  •  WHITE stairs  •  LETTER or COLOUR hunter",
        "TEAL-BLUE relay  •  YELLOW battery  •  GRAY prop  •  WHITE stairs  •  LETTER or COLOUR hunter",
    ];
    static WET: std::sync::OnceLock<[String; 2]> = std::sync::OnceLock::new();
    let scheme = if is_last_route() { 0 } else { 1 };
    if g.fac.water_cells().is_empty() {
        MARKS[scheme]
    } else {
        let wet = WET.get_or_init(|| MARKS.map(|m| format!("{m}  •  {}", look::MapLook::load().water.legend)));
        &wet[scheme]
    }
}

/// The fullscreen map overlay (hud.tscn MapOverlay): title, plan and legend, with the players and bots in `marks` drawn as
/// pins. `map_view` is the view the input steps.
pub fn draw_full_map_with_runners(g: &Game, a: &Assets, map_view: &FullView, marks: &[Marker]) {
    let p = pal();
    let v = view();
    fill(
        Rect::new(0.0, 0.0, v.x, v.y),
        lr(hexa(0x040306f9), rgba(0.04, 0.024, 0.031, 0.94)),
    );
    let floor = if g.is_infinite() {
        -(g.depth - 1)
    } else {
        g.depth - 1
    };
    let title = format!("DISCOVERED ROUTE // FLOOR {floor:+}");
    label_c(
        a,
        F::Title,
        &title,
        v.x * 0.5,
        8.0 + (19.0 - lh(F::Title, 13.0)) * 0.5,
        13.0,
        p.ink,
    );
    let plan = full_plan_rect();
    let geo = full_plan(g, plan, map_view);
    clip_begin(plan);
    match g.full_map_feed() {
        Some(feed) => draw_level0(g, a, &feed, plan, &geo, marks),
        None => draw_floor(g, a, geo.xf, (0, 0, g.fac.cols, g.fac.rows), true, marks),
    }
    clip_end();
    let legend = legend_text(g);
    // a longer legend (the sewer's water entry) shrinks to the screen's width rather than running off both edges
    let size = legend_size(tw(a, F::Body, legend, 9.0), v.x);
    label_c(
        a,
        F::Body,
        legend,
        v.x * 0.5,
        v.y - 20.0 + (20.0 - lh(F::Body, size)) * 0.5,
        size,
        lr(hex(0x8b7f85), rgba(0.49, 0.455, 0.431, 1.0)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// A plan the size of the full map on a 480 x 270 canvas (the tests cannot ask the window for it).
    fn plan() -> Rect {
        Rect::new(12.0, 31.0, 456.0, 214.0)
    }

    fn game() -> Game {
        let content = Content::load();
        let catalog = crate::world::catalog::Catalog::load();
        Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog)
    }

    #[test]
    fn one_chunk_is_fitted_by_its_smaller_side_and_centred() {
        // 900 x 450: the width allows 20 a cell, the height 10, so the cell is 10 and the 450-wide box sits centred
        let plan = Rect::new(0.0, 0.0, 900.0, 450.0);
        let fit = fit_discovered(plan, &[(0, 0)]).expect("one chunk");
        assert_eq!((fit.x0, fit.y0, fit.w, fit.h), (0, 0, CHUNK, CHUNK));
        assert!(close(fit.cell, 10.0), "{fit:?}");
        assert!(close(fit.ox, 225.0) && close(fit.oy, 0.0), "{fit:?}");
        let (x, y) = fit.xf().at((CHUNK as f32, CHUNK as f32));
        assert!(close(x, 675.0) && close(y, 450.0), "bottom right at {x}, {y}");
    }

    #[test]
    fn an_l_shape_fits_its_bounding_box() {
        // (0,0), (1,0), (0,1): the bounding box is 2 x 2 chunks, so the empty corner (1,1) counts inside it
        let plan = Rect::new(0.0, 0.0, 900.0, 450.0);
        let fit = fit_discovered(plan, &[(0, 0), (1, 0), (0, 1)]).expect("an L");
        assert_eq!((fit.x0, fit.y0, fit.w, fit.h), (0, 0, 2 * CHUNK, 2 * CHUNK));
        // 90 cells by 90: the height allows 5 a cell, the width 10
        assert!(close(fit.cell, 5.0), "{fit:?}");
        assert!(close(fit.ox, 225.0) && close(fit.oy, 0.0), "{fit:?}");
        let (x, y) = fit.xf().at((2.0 * CHUNK as f32, 2.0 * CHUNK as f32));
        assert!(close(x, 675.0) && close(y, 450.0), "bottom right at {x}, {y}");
    }

    #[test]
    fn a_discovery_into_negative_coordinates_is_fitted_and_centred() {
        // chunks (-2, -1) and (0, 0): 135 x 90 cells, so the cell is the height's 664 / 90 (the smaller side)
        let plan = Rect::new(12.0, 31.0, 1256.0, 664.0);
        let fit = fit_discovered(plan, &[(-2, -1), (0, 0)]).expect("negative chunks");
        assert_eq!((fit.x0, fit.y0, fit.w, fit.h), (-2 * CHUNK, -CHUNK, 3 * CHUNK, 2 * CHUNK));
        assert!(close(fit.cell, 664.0 / 90.0), "{fit:?}");
        // the area's top-left lands where the centred box starts in the plan
        let box_w = fit.w as f32 * fit.cell;
        let box_h = fit.h as f32 * fit.cell;
        let left = fit.xf().at((fit.x0 as f32, fit.y0 as f32));
        assert!(close(left.0, plan.x + (plan.w - box_w) * 0.5), "left {left:?}");
        assert!(close(left.1, plan.y + (plan.h - box_h) * 0.5), "top {left:?}");
        // the far corner is the box's bottom right
        let far = fit.xf().at(((fit.x0 + fit.w) as f32, (fit.y0 + fit.h) as f32));
        assert!(close(far.0, left.0 + box_w) && close(far.1, left.1 + box_h), "{far:?}");
    }

    #[test]
    fn a_teammate_pin_lands_on_its_cell_through_the_frame_and_origin() {
        // cells 4 units across, cell (0, 0) at (10, 20); a window origin of (3, -2) moves the runner cell (1.5, 2.5) to absolute
        // (4.5, 0.5), so the pin is at (10 + 4.5 * 4, 20 + 0.5 * 4)
        let xf = Xf { cell: 4.0, ox: 10.0, oy: 20.0 };
        let pin = runner_pin_at(xf, (3, -2), (1.5, 2.5));
        assert!(close(pin.0, 28.0) && close(pin.1, 22.0), "{pin:?}");
        // with no origin (the minimap's frame) it is the runner's own cell through the same transform
        assert_eq!(runner_pin_at(xf, (0, 0), (1.5, 2.5)), xf.at((1.5, 2.5)));
    }

    #[test]
    fn nothing_discovered_fits_nothing() {
        assert_eq!(fit_discovered(Rect::new(0.0, 0.0, 100.0, 100.0), &[]), None);
    }

    #[test]
    fn the_admin_reveal_puts_every_hunter_on_both_maps() {
        // the minimap and the full map draw the same feed (Game::map_hunters): asleep hunters are in it only under the reveal
        let mut g = game();
        let content = Content::load();
        let def = content
            .hunters
            .iter()
            .find(|h| h.id != "porcelle")
            .expect("a hunter")
            .clone();
        let id = def.id.clone();
        let mut b = crate::hunters::brain::Brain::new(def, (5.0, 5.0), "reveal-test", Difficulty::Standard);
        b.active = false;
        g.hunters.push(b);
        let at = g.hunters.len() - 1;
        let shown = |g: &Game| g.map_hunters().any(|h| std::ptr::eq(h, &g.hunters[at]));
        assert_eq!(g.hunters[at].def.id, id);
        assert!(!shown(&g), "a sleeping hunter is not on the maps without the reveal");
        g.admin.hunter_reveal = true;
        assert!(shown(&g), "the reveal puts the sleeping hunter on the maps");
    }

    #[test]
    fn a_hunter_is_named_on_the_map_only_where_the_map_already_shows_it() {
        let mut g = game();
        let content = Content::load();
        let def = content.hunters.iter().find(|h| h.id == "kathy").expect("kathy").clone();
        let mut b = crate::hunters::brain::Brain::new(def, (5.0, 5.0), "label-test", Difficulty::Standard);
        b.active = false;
        g.hunters.clear();
        g.hunters.push(b);
        let all = |_: &Brain| Some((1.0, 2.0));
        assert!(hunter_marks(&g, all, 2.0).is_empty(), "no reveal: no position, so no letter either");
        g.admin.hunter_reveal = true;
        let marks = hunter_marks(&g, all, 2.0);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].mark.label, Some('K'));
        assert!((marks[0].alpha - 0.4).abs() < 1e-6, "a sleeping hunter's letter is dimmed like its dot");
        // a hunter off this view is not placed, so its letter is not drawn
        assert!(hunter_marks(&g, |_: &Brain| None, 2.0).is_empty());
        // a blanked radar shows nobody, so names nobody
        g.admin.hunter_reveal = false;
        assert!(hunter_marks(&g, all, 2.0).is_empty());
    }

    #[test]
    fn a_double_click_places_a_waypoint_at_the_clicked_point() {
        let mut g = game();
        let plan = plan();
        let map_view = FullView::default();
        let geo = full_plan(&g, plan, &map_view);
        // a point a little away from the runner, in map cells, and where it lands on the screen
        let target = (geo.runner.0 + 3.25, geo.runner.1 + 1.75);
        let at = geo.xf.at(target);
        let at = vec2(at.0, at.1);
        let mut map_view = map_view;
        let press = |now: f64| MapPointer {
            at,
            left_pressed: true,
            left_down: true,
            now,
            ..Default::default()
        };
        assert_eq!(step(&mut map_view, &geo, plan, &press(10.0)), None, "one press only starts a click");
        let placed = step(&mut map_view, &geo, plan, &press(10.2)).expect("a second press at once is a double click");
        // the window point of the click, and the cell it snaps to
        let window = (target.0 - geo.origin.0 as f32, target.1 - geo.origin.1 as f32);
        assert!(close(placed.0, window.0) && close(placed.1, window.1), "{placed:?} vs {window:?}");
        assert!(g.place_waypoint(placed));
        let marker = g.waypoint_marker().expect("the waypoint is placed");
        let expected = (window.0.floor() + 0.5, window.1.floor() + 0.5);
        assert!(close(marker.0, expected.0) && close(marker.1, expected.1), "{marker:?} vs {expected:?}");
    }

    #[test]
    fn a_slow_second_press_is_not_a_double_click_and_a_right_click_places_at_the_pointer() {
        let g = game();
        let plan = plan();
        let mut map_view = FullView::default();
        let geo = full_plan(&g, plan, &map_view);
        let at = vec2(100.0, 100.0);
        let first = MapPointer { at, left_pressed: true, left_down: true, now: 1.0, ..Default::default() };
        assert_eq!(step(&mut map_view, &geo, plan, &first), None);
        let late = MapPointer { at, left_pressed: true, left_down: true, now: 2.0, ..Default::default() };
        assert_eq!(step(&mut map_view, &geo, plan, &late), None, "half a second later is a new click");
        let right = MapPointer { at, right_pressed: true, now: 3.0, ..Default::default() };
        assert_eq!(step(&mut map_view, &geo, plan, &right), Some(geo.window_at(at)));
    }

    #[test]
    fn the_mouse_pans_the_map_and_turns_following_off() {
        let g = game();
        let plan = plan();
        let mut map_view = FullView::default();
        let geo = full_plan(&g, plan, &map_view);
        let start = vec2(100.0, 100.0);
        let press = MapPointer { at: start, left_pressed: true, left_down: true, now: 0.0, ..Default::default() };
        step(&mut map_view, &geo, plan, &press);
        assert_eq!(map_view.centre, None, "a press alone does not pan");
        // the pointer moves 20 units right: the map moves with it, so the centre moves left by 20 cells' worth
        let geo = full_plan(&g, plan, &map_view);
        let drag = MapPointer { at: start + vec2(20.0, 0.0), left_down: true, now: 0.1, ..Default::default() };
        assert_eq!(step(&mut map_view, &geo, plan, &drag), None);
        let c = map_view.centre.expect("a drag turns following off");
        assert!(close(c.0, geo.runner.0 - 20.0 / geo.xf.cell), "{c:?} vs runner {:?}", geo.runner);
        assert!(close(c.1, geo.runner.1), "{c:?}");
        // released, the next move does nothing
        let up = MapPointer { at: start + vec2(40.0, 0.0), now: 0.2, ..Default::default() };
        step(&mut map_view, &geo, plan, &up);
        assert_eq!(map_view.centre, Some(c));
    }

    #[test]
    fn the_wheel_zooms_about_the_pointer_and_space_recentres() {
        let g = game();
        let plan = plan();
        let mut map_view = FullView::default();
        let geo = full_plan(&g, plan, &map_view);
        // turn following off first with a small drag
        let start = vec2(120.0, 120.0);
        step(&mut map_view, &geo, plan, &MapPointer { at: start, left_pressed: true, left_down: true, now: 0.0, ..Default::default() });
        let geo = full_plan(&g, plan, &map_view);
        step(&mut map_view, &geo, plan, &MapPointer { at: start + vec2(5.0, 0.0), left_down: true, now: 0.1, ..Default::default() });
        let geo = full_plan(&g, plan, &map_view);
        let focus = vec2(200.0, 150.0);
        let under = geo.xf.cell_at(focus);
        let zoom_before = map_view.zoom;
        step(&mut map_view, &geo, plan, &MapPointer { at: focus, wheel: 1.0, now: 0.2, ..Default::default() });
        assert!(close(map_view.zoom, zoom_before * ZOOM_STEP), "{}", map_view.zoom);
        let geo = full_plan(&g, plan, &map_view);
        let after = geo.xf.cell_at(focus);
        assert!(close(after.0, under.0) && close(after.1, under.1), "the cell under the pointer stays: {under:?} -> {after:?}");
        // Space follows the runner again, and the zoom stays
        step(&mut map_view, &geo, plan, &MapPointer { recentre: true, now: 0.3, ..Default::default() });
        assert_eq!(map_view.centre, None);
        assert!(close(map_view.zoom, zoom_before * ZOOM_STEP));
    }

    #[test]
    fn the_zoom_stays_within_its_range() {
        let g = game();
        let plan = plan();
        let mut map_view = FullView::default();
        for _ in 0..40 {
            let geo = full_plan(&g, plan, &map_view);
            step(&mut map_view, &geo, plan, &MapPointer { at: vec2(100.0, 100.0), wheel: 1.0, now: 0.0, ..Default::default() });
        }
        assert!(close(map_view.zoom, MAX_FULL_ZOOM), "{}", map_view.zoom);
        for _ in 0..80 {
            let geo = full_plan(&g, plan, &map_view);
            step(&mut map_view, &geo, plan, &MapPointer { at: vec2(100.0, 100.0), wheel: -1.0, now: 0.0, ..Default::default() });
        }
        assert!(close(map_view.zoom, MIN_FULL_ZOOM), "{}", map_view.zoom);
    }

    #[test]
    fn the_level_zero_full_map_tints_only_the_floor_the_runner_has_seen() {
        let g = game();
        let feed = g.full_map_feed().expect("level 0 has a feed");
        let panel = hex(0x123456);
        let (k, mc) = feed.chunks[0];
        let bits = feed.seen.chunk((k.cx, k.cy)).expect("the chunk has seen cells");
        let (mut drawn, mut open_unseen) = (0, 0);
        for z in 0..CHUNK {
            for x in 0..CHUNK {
                let t = seen_floor_tint(mc, bits, z, x, panel);
                if bits.get(x, z) {
                    assert_eq!(t, floor_tint(mc, z, x, panel));
                    drawn += t.is_some() as i32;
                } else {
                    assert_eq!(t, None, "unseen cell {x},{z} stays the backdrop");
                    open_unseen += floor_tint(mc, z, x, panel).is_some() as i32;
                }
            }
        }
        assert!(drawn > 0, "the lobby is drawn");
        assert!(open_unseen > drawn, "most of the chunk's open floor is still hidden ({open_unseen} vs {drawn})");
    }

    #[test]
    fn the_legend_names_the_water_only_on_a_floor_that_has_it() {
        let content = Content::load();
        let catalog = crate::world::catalog::Catalog::load();
        let sewer = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", -1, &content, &catalog);
        let word = &look::MapLook::load().water.legend;
        assert!(legend_text(&sewer).ends_with(word.as_str()), "{}", legend_text(&sewer));
        assert!(!legend_text(&game()).contains(word.as_str()));
    }

    #[test]
    fn a_long_legend_shrinks_to_fit_the_screen_and_a_short_one_stays_at_nine() {
        assert_eq!(legend_size(300.0, 480.0), 9.0);
        let s = legend_size(560.0, 480.0);
        assert!(s < 9.0 && (560.0 * s / 9.0 - 464.0).abs() < 1e-3, "{s}");
    }

    #[test]
    fn a_water_cell_is_tinted_blue_over_the_floor_colour() {
        let panel = hex(0x2a2530);
        let water = look::MapLook::load().water.over(panel);
        assert_ne!(water, panel);
        assert!(water.b > water.r && water.b > panel.b, "bluer than the floor: {water:?}");
        assert_eq!(water.a, panel.a);
    }
}
