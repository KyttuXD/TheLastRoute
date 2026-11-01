//! The minimap and the fullscreen discovered map (facility_map.gd), in design units: walls are
//! the backdrop colour, discovered floor the panel colour, relays teal-blue, batteries yellow,
//! the lift and stairwells, hunters on the radar in red, and the runner's wedge. On the Infinite
//! route the minimap follows the runner over a 36-cell square (STREAMED_MINIMAP_RADIUS_CELLS).

use super::style::*;
use crate::app::assets::Assets;
use crate::game::Game;
use macroquad::prelude::*;

pub const RELAY_TEAL_BLUE: Color = hex(0x32c9d8);
pub const BATTERY: Color = rgba(0.95, 0.82, 0.25, 1.0);
pub const EXIT: Color = rgba(0.76, 0.55, 0.96, 1.0);
pub const STAIRS: Color = hex(0xf4f7f6);
pub const RUNNER: Color = hex(0x63d9f1);

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
                hex(0x2f5a52)
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
    for (i, rl) in g.fac.relays.iter().enumerate() {
        if seen(*rl) && !g.relays_taken[i] {
            dot(rl.center(), RELAY_TEAL_BLUE, (cell * 0.34).max(0.5));
        }
    }
    for (i, b) in g.fac.batteries.iter().enumerate() {
        if seen(*b) && !g.batteries_taken[i] {
            dot(b.center(), BATTERY, (cell * 0.26).max(0.4));
        }
    }
    if g.fac.has_lift && seen(g.fac.exit) {
        dot(g.fac.exit.center(), EXIT, cell * 0.4);
    }
    for s in &g.fac.stairs {
        if seen(s.cell) && inside(s.cell.center()) {
            let c = vec2(ox + s.cell.center().0 * cell, oy + s.cell.center().1 * cell);
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
    // the runner's wedge (an outline on the full map)
    let (fx, fy) = g.forward2();
    let at = vec2(ox + g.p.pos.0 * cell, oy + g.p.pos.1 * cell);
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
    draw_map(g, Rect::new(12.0, 31.0, v.x - 24.0, v.y - 56.0), true);
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
