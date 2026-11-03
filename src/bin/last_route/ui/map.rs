//! The minimap and the fullscreen discovered map (facility_map.gd), in design units: walls are
//! the backdrop colour, discovered floor the panel colour, relays teal-blue, batteries yellow,
//! the lift and stairwells, hunters on the radar in red, and the runner's wedge. On the Infinite
//! route the minimap follows the runner over a 36-cell square (STREAMED_MINIMAP_RADIUS_CELLS).
//! The fullscreen map on level 0 draws every discovered chunk, fitted to the discovered area
//! (`draw_level0`, Godot `_draw_streamed`'s full-map branch).

use super::style::*;
use crate::app::assets::Assets;
use crate::game::endless::{FullMapFeed, MapChunk};
use crate::game::Game;
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
/// A stairwell's floor on the map (the window's hole colour).
const HOLE: Color = hex(0x2f5a52);
/// A solid prop's collision box on the full map (facility_map.gd PROP_BOX_TINT).
const PROP_BOX: Color = hex(0x7899a1);

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

/// Draws the map into `r` (units).
pub fn draw_map(g: &Game, r: Rect, full: bool) {
    let p = pal();
    let (vx, vy, vw, vh) = view_cells(g, full);
    let cell = (r.w / vw as f32).min(r.h / vh as f32);
    let ox = r.x + (r.w - cell * vw as f32) * 0.5 - vx as f32 * cell;
    let oy = r.y + (r.h - cell * vh as f32) * 0.5 - vy as f32 * cell;
    fill(
        Rect::new(
            ox + vx as f32 * cell,
            oy + vy as f32 * cell,
            vw as f32 * cell,
            vh as f32 * cell,
        ),
        p.backdrop,
    );
    for y in vy..vy + vh {
        for x in vx..vx + vw {
            if !g.discovered[y as usize][x as usize] || g.fac.is_wall_cell(x, y) {
                continue;
            }
            let col = if g.fac.is_hole(x, y) {
                HOLE
            } else {
                p.panel
            };
            fill(
                Rect::new(
                    ox + x as f32 * cell,
                    oy + y as f32 * cell,
                    cell + 0.02,
                    cell + 0.02,
                ),
                col,
            );
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
            circle(vec2(ox + c.0 * cell, oy + c.1 * cell), rad, col);
        }
    };
    if let Some(sz) = g.fac.safe_zone {
        if seen(sz) {
            dot(sz.center(), METER_SAFE, cell * 0.9);
        }
    }
    // a relay or battery is drawn once its marker is discovered (Game::map_relay_shown; level 0 only)
    for (i, rl) in g.fac.relays.iter().enumerate() {
        if seen(*rl) && !g.relays_taken[i] && g.map_relay_shown(i) {
            dot(rl.center(), RELAY_TEAL_BLUE, (cell * 0.34).max(0.5));
        }
    }
    for (i, b) in g.fac.batteries.iter().enumerate() {
        if seen(*b) && !g.batteries_taken[i] && g.map_battery_shown(i) {
            dot(b.center(), BATTERY, (cell * 0.26).max(0.4));
        }
    }
    if g.fac.has_lift && seen(g.fac.exit) {
        dot(g.fac.exit.center(), EXIT, cell * 0.4);
    }
    for s in &g.fac.stairs {
        if seen(s.cell) && inside(s.cell.center()) {
            let c = vec2(ox + s.cell.center().0 * cell, oy + s.cell.center().1 * cell);
            stair_icon(c, cell, full);
        }
    }
    // the runner's waypoint: the route to it (the full route, Game::waypoint_route) and its pin (Game::waypoint_marker)
    for w in g.waypoint_route().windows(2) {
        if inside(w[0]) && inside(w[1]) {
            line(
                vec2(ox + w[0].0 * cell, oy + w[0].1 * cell),
                vec2(ox + w[1].0 * cell, oy + w[1].1 * cell),
                (cell * 0.18).max(0.8),
                WAYPOINT_ROUTE,
            );
        }
    }
    if let Some(at) = g.waypoint_marker() {
        dot(at, WAYPOINT_PIN, (cell * 0.5).max(1.5));
    }
    for h in g
        .hunters
        .iter()
        .filter(|h| h.active && h.gimmick.on_radar())
    {
        dot(
            h.pos,
            METER_LOW,
            (cell * 0.55).max(if full { 3.0 } else { 1.2 }),
        );
    }
    let at = vec2(ox + g.p.pos.0 * cell, oy + g.p.pos.1 * cell);
    runner_wedge(g, at, cell, full);
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
    /// The plan position of an absolute cell point.
    pub fn at(&self, c: (f32, f32)) -> (f32, f32) {
        (self.ox + c.0 * self.cell, self.oy + c.1 * self.cell)
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
    tri(a, b, c, RUNNER);
}

/// The level-0 full map: every discovered chunk (`feed`), fitted to the discovered area in `r` (`fit_discovered`). The
/// floor is drawn per chunk in runs of cells, then the solid props, the relays and batteries, the stairs, the waypoint, the
/// hunters on the radar and the runner. Off the discovered area nothing is drawn.
fn draw_level0(g: &Game, feed: &FullMapFeed, r: Rect) {
    let p = pal();
    let keys: Vec<(i32, i32)> = feed.chunks.iter().map(|(k, _)| (k.cx, k.cy)).collect();
    let Some(fit) = fit_discovered(r, &keys) else { return };
    let cell = fit.cell;
    let (x1, y1) = (fit.x0 + fit.w, fit.y0 + fit.h);
    fill(
        Rect::new(
            fit.ox + fit.x0 as f32 * cell,
            fit.oy + fit.y0 as f32 * cell,
            fit.w as f32 * cell,
            fit.h as f32 * cell,
        ),
        p.backdrop,
    );
    // each chunk's open floor, as runs of cells along each row (one fill per run, not per cell)
    for (k, mc) in &feed.chunks {
        let (bx, by) = (k.cx * CHUNK, k.cy * CHUNK);
        for z in 0..CHUNK {
            let mut x = 0;
            while x < CHUNK {
                let Some(col) = floor_tint(mc, z, x, p.panel) else {
                    x += 1;
                    continue;
                };
                let start = x;
                while x < CHUNK && floor_tint(mc, z, x, p.panel) == Some(col) {
                    x += 1;
                }
                fill(
                    Rect::new(
                        fit.ox + (bx + start) as f32 * cell,
                        fit.oy + (by + z) as f32 * cell,
                        (x - start) as f32 * cell + 0.02,
                        cell + 0.02,
                    ),
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
            let q = fit.at(c);
            circle(vec2(q.0, q.1), rad, col);
        }
    };
    // the window's safe zone (the middle chunk's safe room while intact), in absolute cells
    if let Some(sz) = g.fac.safe_zone {
        let o = feed.origin;
        dot(
            crate::world::Cell::new(sz.x + o.0, sz.y + o.1).center(),
            METER_SAFE,
            cell * 0.9,
        );
    }
    for (_, mc) in &feed.chunks {
        for pr in &mc.props {
            if !g.full_map_prop_shown(&pr.id) {
                continue;
            }
            let c = pr.cell.center();
            let (hx, hy) = pr.half;
            let tl = fit.at((c.0 - hx, c.1 - hy));
            fill(Rect::new(tl.0, tl.1, hx * 2.0 * cell, hy * 2.0 * cell), PROP_BOX);
        }
    }
    for (_, mc) in &feed.chunks {
        for (id, c) in &mc.relays {
            if g.full_map_pickup_shown(id) {
                dot(c.center(), RELAY_TEAL_BLUE, (cell * 0.34).max(0.5));
            }
        }
        for (id, c) in &mc.batteries {
            if g.full_map_pickup_shown(id) {
                dot(c.center(), BATTERY, (cell * 0.26).max(0.4));
            }
        }
    }
    for (_, mc) in &feed.chunks {
        for s in &mc.stairs {
            let c = s.cell.center();
            if shown(c) {
                let q = fit.at(c);
                stair_icon(vec2(q.0, q.1), cell, true);
            }
        }
    }
    // the waypoint route and its pin, both absolute
    for w in feed.route.windows(2) {
        if shown(w[0]) && shown(w[1]) {
            let (a, b) = (fit.at(w[0]), fit.at(w[1]));
            line(
                vec2(a.0, a.1),
                vec2(b.0, b.1),
                (cell * 0.18).max(0.8),
                WAYPOINT_ROUTE,
            );
        }
    }
    if let Some(at) = feed.waypoint {
        dot(at, WAYPOINT_PIN, (cell * 0.5).max(1.5));
    }
    let o = (feed.origin.0 as f32, feed.origin.1 as f32);
    for h in g
        .hunters
        .iter()
        .filter(|h| h.active && h.gimmick.on_radar())
    {
        dot((h.pos.0 + o.0, h.pos.1 + o.1), METER_LOW, (cell * 0.55).max(3.0));
    }
    let q = fit.at((g.p.pos.0 + o.0, g.p.pos.1 + o.1));
    runner_wedge(g, vec2(q.0, q.1), cell, true);
}

/// The fullscreen map overlay (hud.tscn MapOverlay): title, plan and legend.
pub fn draw_full_map(g: &Game, a: &Assets) {
    let p = pal();
    let v = view();
    fill(
        Rect::new(0.0, 0.0, v.x, v.y),
        rgba(0.04, 0.024, 0.031, 0.94),
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
    let plan = Rect::new(12.0, 31.0, v.x - 24.0, v.y - 56.0);
    match g.full_map_feed() {
        Some(feed) => draw_level0(g, &feed, plan),
        None => draw_map(g, plan, true),
    }
    let legend = "TEAL-BLUE relay  •  YELLOW battery  •  GRAY prop  •  WHITE stairs  •  RED hunter";
    label_c(
        a,
        F::Body,
        legend,
        v.x * 0.5,
        v.y - 20.0 + (20.0 - lh(F::Body, 9.0)) * 0.5,
        9.0,
        rgba(0.49, 0.455, 0.431, 1.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn one_chunk_is_fitted_by_its_smaller_side_and_centred() {
        // 900 x 450: the width allows 20 a cell, the height 10, so the cell is 10 and the 450-wide box sits centred
        let plan = Rect::new(0.0, 0.0, 900.0, 450.0);
        let fit = fit_discovered(plan, &[(0, 0)]).expect("one chunk");
        assert_eq!((fit.x0, fit.y0, fit.w, fit.h), (0, 0, CHUNK, CHUNK));
        assert!(close(fit.cell, 10.0), "{fit:?}");
        assert!(close(fit.ox, 225.0) && close(fit.oy, 0.0), "{fit:?}");
        let (x, y) = fit.at((CHUNK as f32, CHUNK as f32));
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
        let (x, y) = fit.at((2.0 * CHUNK as f32, 2.0 * CHUNK as f32));
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
        let left = fit.at((fit.x0 as f32, fit.y0 as f32));
        assert!(close(left.0, plan.x + (plan.w - box_w) * 0.5), "left {left:?}");
        assert!(close(left.1, plan.y + (plan.h - box_h) * 0.5), "top {left:?}");
        // the far corner is the box's bottom right
        let far = fit.at(((fit.x0 + fit.w) as f32, (fit.y0 + fit.h) as f32));
        assert!(close(far.0, left.0 + box_w) && close(far.1, left.1 + box_h), "{far:?}");
    }

    #[test]
    fn nothing_discovered_fits_nothing() {
        assert_eq!(fit_discovered(Rect::new(0.0, 0.0, 100.0, 100.0), &[]), None);
    }
}
