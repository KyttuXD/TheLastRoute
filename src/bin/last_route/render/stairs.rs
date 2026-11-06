//! Stair flights and ladder wells as geometry: a flight is a smooth ramp (sloped tiles sampled from the flight's surface,
//! no tread blocks) going down through an open well, with its fit-out (handrails, the landing guards, the centre post,
//! the "LEVEL n" signs and the light), and a ladder in its shaft. The pieces come out as quads and boxes that
//! `world_mesh` files under the level's floor and wall pictures, so the shaft carries the wall's picture. The flight's
//! footprint, surface and walk code live in `world::facility` / `world::flight`: this is only what is drawn. The numbers
//! of the fit-out are data (`dressing` in data/flights.json); the geometry of each piece is `world::flight`'s
//! (`handrails_with`, `landing_guards_with`, `signs_with`, `lamp_place_with`, `spiral_post_with`).
//!
//! Which level draws what. The lower level (`dir > 0`) owns the ramp and everything standing on it: the tiles, the solid
//! divider and wall faces of a switchback or straight flight, the floor under a spiral's helicoid, the post, the
//! handrails (up to the upper floor), its "LEVEL n" sign and the well's light. The upper level (`dir < 0`) draws no floor
//! over the well, only its guard rails round the rim (posts, a top and a knee rail; a spiral's run the whole rim the
//! collision closes, `spiral_rim_guards`), the spiral's slab rail, the part of the handrails that stands above the upper
//! floor and its own sign. A spiral has no "LEVEL n" sign (`signs_with` gives it none).

use super::shader::{Flicker, Lamp};
use crate::world::facility::{Cell, Cover, LinkKind, Stair};
use crate::world::flight::{
    divider_width, flight_surface_y_with, flights, handrails_with, lamp_place_with, landing_guards_with, lane_dividers, signs_with,
    spiral_post_with, spiral_slab_rail_with, Dressing, Flight, FlightShape, Sign,
};
use crate::world::vertical::STOREY;
use macroquad::prelude::*;

/// Which of the level's pictures a piece is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// The floor picture of the cell (a shaft's bottom).
    Floor,
    /// The wall picture of the cell (pit and shaft walls).
    Wall,
    /// Flat colour (the ramp's tiles, the shaft cap).
    Plain,
    /// Flat colour of the iron fittings (rails, the post): the colour carries the iron, the pictures stay off it.
    Iron,
}

pub struct Quad {
    pub face: Face,
    pub c: [Vec3; 4],
    pub uv: [Vec2; 4],
    pub n: Vec3,
    pub col: Color,
}

/// Everything one cell of a flight, or one ladder, adds to the floor's meshes.
#[derive(Default)]
pub struct Parts {
    pub quads: Vec<Quad>,
    /// Boxes (centre, size, colour) for rails, rungs, guards and grate bars, drawn in plain colour.
    pub boxes: Vec<(Vec3, Vec3, Color)>,
    /// Boxes drawn unlit (the sign lettering, a grate's rim), so they read where the light is thin.
    pub glow: Vec<(Vec3, Vec3, Color)>,
}

/// One square tile of a flight's smooth ramp: its corner, its side (`1 / tiles_per_cell` of a cell) and the heights of
/// its four corners relative to the floor of the level that holds the flight, in the order (x0, z0), (x0, z1),
/// (x1, z1), (x1, z0) (x1 = x0 + size, z1 = z0 + size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub x0: f32,
    pub z0: f32,
    pub size: f32,
    pub y: [f32; 4],
}

impl Tile {
    /// The height at the tile's middle (the mean of its corners).
    pub fn mean(&self) -> f32 {
        self.y.iter().sum::<f32>() * 0.25
    }
    /// The corners as points.
    pub fn corners(&self) -> [Vec3; 4] {
        let (x1, z1) = (self.x0 + self.size, self.z0 + self.size);
        [
            vec3(self.x0, self.y[0], self.z0),
            vec3(self.x0, self.y[1], z1),
            vec3(x1, self.y[2], z1),
            vec3(x1, self.y[3], self.z0),
        ]
    }
    /// The unit normal of the tile's top face (pointing up).
    pub fn normal(&self) -> Vec3 {
        let p = self.corners();
        let n = (p[2] - p[0]).cross(p[3] - p[1]);
        let n = if n.length() > 1e-9 { n.normalize() } else { vec3(0.0, 1.0, 0.0) };
        if n.y < 0.0 {
            -n
        } else {
            n
        }
    }
}

/// What a cell's flight needs to know about its surroundings and ceiling.
pub struct Site<'a> {
    /// The storey the flight crosses (the level below's floor is `-h`).
    pub h: f32,
    /// This cell's wall height, capped under the slab (`seams::wall_top`): where a closed cell's walls end.
    pub wall_h: f32,
    /// How many times the wall picture repeats floor to ceiling.
    pub tiers: f32,
    /// True for a cell of a stairwell (a flight's own cells).
    pub well: &'a dyn Fn(i32, i32) -> bool,
    /// True for a wall cell.
    pub solid: &'a dyn Fn(i32, i32) -> bool,
}

impl Site<'_> {
    /// Where a shaft's lining ends: the underside of the floor above (the storey `h`). A flight's well and a ladder's
    /// shaft have no ceiling, so what lines them rises through the slab gap (`h - wall_h`) instead of stopping at the
    /// cell's wall height.
    pub fn shaft_top(&self) -> f32 {
        self.h
    }
}

/// Height of the rails a ladder hatch stands in (its two rails and rungs over the upper floor, `ladder_down`). A flight's
/// handrails and guards are `dressing.handrail.height` / `dressing.guard.height` in data/flights.json.
pub const RAIL_H: f32 = 1.05;

/// The ladder's rails and rungs: a light iron, so they read against the dark shaft from standing height.
const IRON: Color = Color::new(0.82, 0.78, 0.72, 1.0);
/// A grate's bars and a hatch's frame: a mid grey iron that catches the light (the old near-black barely showed).
const GRATE: Color = Color::new(0.5, 0.49, 0.47, 1.0);
/// The lit edge of a grate cover, a dim warm rim so the cover's outline reads down the shaft.
const RIM: Color = Color::new(0.95, 0.62, 0.3, 1.0);
/// A hatch's cover: a lit plate of pale grey-cream, unlit, so the cover never reads as a black hole in the ceiling
/// (the floor picture painted over the hatch came out black from below).
pub const HATCH: Color = Color::new(0.62, 0.6, 0.56, 1.0);

/// The ramp's stone: a flat pale grey-cream, drawn in plain colour rather than over the floor picture. The sewer's
/// floor is orange-brown, and a picture-painted ramp blended into it; a light flat stone reads apart from any floor.
pub const TREAD_STONE: Color = Color::new(0.74, 0.72, 0.68, 1.0);

/// The ramp fades with depth, but never to black: the lowest tile is still 75 % of the picture.
fn shade(y: f32, h: f32) -> Color {
    let f = if y < 0.0 { 1.0 - 0.25 * (-y / h).min(1.0) } else { 1.0 - 0.2 * (y / h).min(1.0) };
    Color::new(f, f, f, 1.0)
}

/// A tile's colour: its depth shade times the stone.
fn tread_tint(s: Color) -> Color {
    Color::new(s.r * TREAD_STONE.r, s.g * TREAD_STONE.g, s.b * TREAD_STONE.b, 1.0)
}

/// RefSurfaceMesh.add_wall's v at height `y` (0 at the ceiling row, `tiers` at the floor, repeating beyond both).
fn wall_v(y: f32, s: &Site) -> f32 {
    s.tiers * (1.0 - y / s.wall_h)
}

fn push(out: &mut Parts, face: Face, c: [Vec3; 4], uv: [Vec2; 4], n: Vec3, col: Color) {
    out.quads.push(Quad { face, c, uv, n, col });
}

/// The iron of the fittings (rails, post, guards): Godot's colour, lifted to read in this renderer's dark.
pub fn iron() -> Color {
    let d = &flights().dressing;
    let c = d.iron_colour;
    Color::new((c[0] * d.iron_lift).min(1.0), (c[1] * d.iron_lift).min(1.0), (c[2] * d.iron_lift).min(1.0), 1.0)
}

fn colour3(c: [f32; 3]) -> Color {
    Color::new(c[0], c[1], c[2], 1.0)
}

/// A face's colour dimmed to `f` of itself (the underside of a hollow ramp sits darker than its top).
fn dim(c: Color, f: f32) -> Color {
    Color::new(c.r * f, c.g * f, c.b * f, 1.0)
}

const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

// ---------------------------------------------------------------------------------------------------------------------
// The smooth ramp
// ---------------------------------------------------------------------------------------------------------------------

/// The tiles of well cell `c` of the flight `s` as held by `level`: `tiles_per_cell` squared of them, those with a
/// surface. A tile is there when the flight's surface is at its middle or, where the surface only clips the tile, at one
/// of its quarter points, so no walkable point is left bare. The four corners take the surface's own height (read a hair
/// inside the tile, so a corner on the edge of the surface still reads it); a corner off the surface takes the tile's
/// middle height. The whole drawn surface is used (`flight_surface_y_with`), so a spiral's slab is drawn too.
pub fn tiles(s: Stair, level: i32, c: (i32, i32)) -> Vec<Tile> {
    let Some(f) = s.flight(level) else { return vec![] };
    let rules = flights();
    let per = rules.tiles_for(f.shape).max(1);
    let size = 1.0 / per as f32;
    let base = level as f32 * STOREY;
    let at = |x: f32, z: f32| flight_surface_y_with(rules, &f, x, z).map(|y| y - base);
    let q = size * 0.25;
    let mut out = Vec::new();
    for i in 0..per {
        for j in 0..per {
            let (x0, z0) = (c.0 as f32 + i as f32 * size, c.1 as f32 + j as f32 * size);
            let (cx, cz) = (x0 + size * 0.5, z0 + size * 0.5);
            let Some(hc) = [(0.0, 0.0), (-q, -q), (q, -q), (-q, q), (q, q)].into_iter().find_map(|(dx, dz)| at(cx + dx, cz + dz)) else {
                continue;
            };
            let inset = size * 0.02;
            let corner = |px: f32, pz: f32| {
                let (ix, iz) = ((cx - px).signum() * inset, (cz - pz).signum() * inset);
                // a corner across a drop in the surface (the slab beside a spiral's lowest step) takes the tile's own height
                at(px + ix, pz + iz).filter(|h| (h - hc).abs() <= TILE_JUMP).unwrap_or(hc)
            };
            out.push(Tile {
                x0,
                z0,
                size,
                y: [corner(x0, z0), corner(x0, z0 + size), corner(x0 + size, z0 + size), corner(x0 + size, z0)],
            });
        }
    }
    out
}

/// The most a tile's corner may differ from its middle (m): a tile spans a quarter of a metre at a slope under 2, so a bigger
/// difference is a drop in the surface the tile straddles, and the corner is not read across it.
const TILE_JUMP: f32 = 0.6;

/// The tile's top face, and for a hollow ramp (a spiral's helicoid is a thin shell) its darker underside too.
fn tile_quads(out: &mut Parts, t: &Tile, hollow: bool, site: &Site) {
    let p = t.corners();
    let col = tread_tint(shade(t.mean(), site.h));
    let n = t.normal();
    push(out, Face::Plain, p, [Vec2::ZERO; 4], n, col);
    if hollow {
        push(out, Face::Plain, [p[3], p[2], p[1], p[0]], [Vec2::ZERO; 4], -n, dim(col, 0.8));
    }
}

/// The face of a vertical edge between `a` and `b` (ground points with the tile height at each), from the floor up to
/// those heights, painted with the wall picture, facing `d`. Heights are kept under the floor above.
fn edge_wall(out: &mut Parts, site: &Site, a: (f32, f32, f32), b: (f32, f32, f32), d: (i32, i32), col: Color) {
    let top = site.shaft_top();
    let (ha, hb) = (a.2.min(top), b.2.min(top));
    if ha <= 1e-5 && hb <= 1e-5 {
        return;
    }
    let (ua, ub) = (a.0 + a.1, b.0 + b.1);
    let v = |y: f32| wall_v(y, site);
    push(
        out,
        Face::Wall,
        [vec3(a.0, ha, a.1), vec3(b.0, hb, b.1), vec3(b.0, 0.0, b.1), vec3(a.0, 0.0, a.1)],
        [vec2(ua, v(ha)), vec2(ub, v(hb)), vec2(ub, v(0.0)), vec2(ua, v(0.0))],
        vec3(d.0 as f32, 0.0, d.1 as f32),
        col,
    );
}

/// The side faces of a solid ramp (a switchback or straight flight is a wedge down to the floor): where a tile's edge
/// borders a cell that is neither part of the well nor a wall, the wedge's side shows, from the floor up to the tile.
fn tile_walls(out: &mut Parts, t: &Tile, c: (i32, i32), site: &Site) {
    let (x1, z1) = (t.x0 + t.size, t.z0 + t.size);
    let col = shade(t.mean(), site.h);
    for d in DIRS {
        let probe = (t.x0 + t.size * 0.5 + d.0 as f32 * t.size, t.z0 + t.size * 0.5 + d.1 as f32 * t.size);
        let nc = (probe.0.floor() as i32, probe.1.floor() as i32);
        if nc == c || (site.well)(nc.0, nc.1) || (site.solid)(nc.0, nc.1) {
            continue;
        }
        let (a, b) = match d {
            (1, 0) => ((x1, t.z0, t.y[3]), (x1, z1, t.y[2])),
            (-1, 0) => ((t.x0, t.z0, t.y[0]), (t.x0, z1, t.y[1])),
            (0, 1) => ((t.x0, z1, t.y[1]), (x1, z1, t.y[2])),
            _ => ((t.x0, t.z0, t.y[0]), (x1, t.z0, t.y[3])),
        };
        edge_wall(out, site, a, b, d, col);
    }
}

/// The floor under a well cell, in the floor picture (a spiral's helicoid is a shell, the lower level's floor lies
/// under it).
fn floor_quad(out: &mut Parts, c: (i32, i32)) {
    let (x, z) = (c.0 as f32, c.1 as f32);
    push(
        out,
        Face::Floor,
        [vec3(x, 0.0, z), vec3(x + 1.0, 0.0, z), vec3(x + 1.0, 0.0, z + 1.0), vec3(x, 0.0, z + 1.0)],
        [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)],
        vec3(0.0, 1.0, 0.0),
        WHITE,
    );
}

/// Adds cell `c` (one of `s.well_cells()`) of the flight `s` as held by `level`: for the flight's owner (the lower level,
/// dir > 0) its smooth tiles, the side faces of a solid ramp, the floor under a spiral's shell, and once per stair the
/// divider, post, handrails, sign and (nothing else: the light is `lamp`) fittings; for the upper level (dir < 0) no tiles,
/// lining or floor, only once per stair the guards, the slab rail, the handrail tops and the upper sign.
pub fn flight_cell(out: &mut Parts, c: (i32, i32), s: Stair, level: i32, site: &Site) {
    let Some(f) = s.flight(level) else { return };
    let first = s.well_cells().first().is_some_and(|w| (w.x, w.y) == c);
    if s.dir < 0 {
        if first {
            upper_dressing(out, &f, site);
        }
        return;
    }
    let hollow = f.shape == FlightShape::Spiral;
    for t in tiles(s, level, c) {
        tile_quads(out, &t, hollow, site);
        if !hollow {
            tile_walls(out, &t, c, site);
        }
    }
    if hollow {
        floor_quad(out, c);
    }
    if first {
        lower_dressing(out, &f, site);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The fit-out
// ---------------------------------------------------------------------------------------------------------------------

/// The lower level's fittings of one stair, once: the divider, the post, the handrails up to the upper floor, the sign.
fn lower_dressing(out: &mut Parts, f: &Flight, site: &Site) {
    divider_wall(out, f, site);
    post(out, f, site);
    // a bar is cut half its thickness short of the upper floor, so none of it passes the floor above (the rest of the rail
    // is the upper level's, re-based to its floor)
    let m = flights().dressing.rail_thickness * 0.5;
    for r in handrails_with(flights(), f) {
        for (a, b) in clip_segments(&r.points, 0.0, site.shaft_top() - m) {
            rail_bar(out, a, b);
        }
    }
    for sign in signs_with(flights(), f).iter().filter(|s| !s.upper) {
        sign_lettering(out, sign);
    }
}

/// The upper level's fittings of one stair, once: the guards round the rim, the slab rail, the part of the handrails that
/// stands over the upper floor (re-based to it) and its sign.
fn upper_dressing(out: &mut Parts, f: &Flight, site: &Site) {
    let rules = flights();
    let d = &rules.dressing;
    for g in landing_guards_with(rules, f) {
        if (site.solid)(g.outside.x, g.outside.y) {
            continue;
        }
        guard_bar(out, g.seg.a, g.seg.b, g.outside, d);
    }
    if let Some(seg) = spiral_slab_rail_with(rules, f) {
        for y in [d.rail_height, d.rail_height * 0.5] {
            rail_bar(out, [seg.a.0, y, seg.a.1], [seg.b.0, y, seg.b.1]);
        }
        guard_posts(out, seg.a, seg.b, d);
    }
    let m = d.rail_thickness * 0.5;
    for r in handrails_with(rules, f) {
        for (a, b) in clip_segments(&r.points, site.shaft_top() + m, f32::MAX) {
            rail_bar(out, [a[0], a[1] - site.shaft_top(), a[2]], [b[0], b[1] - site.shaft_top(), b[2]]);
        }
    }
    for sign in signs_with(rules, f).iter().filter(|s| s.upper) {
        sign_lettering(out, sign);
    }
}

/// The parts of a polyline's segments that lie within `lo..=hi` in height: each segment cut to the band (a segment wholly
/// outside gives none).
pub fn clip_segments(points: &[[f32; 3]], lo: f32, hi: f32) -> Vec<([f32; 3], [f32; 3])> {
    let mut out = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dy = b[1] - a[1];
        // the parameter range of the segment inside the band
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        if dy.abs() < 1e-9 {
            if a[1] < lo || a[1] > hi {
                continue;
            }
        } else {
            let (ta, tb) = ((lo - a[1]) / dy, (hi.min(1e9) - a[1]) / dy);
            let (enter, leave) = if ta < tb { (ta, tb) } else { (tb, ta) };
            t0 = t0.max(enter);
            t1 = t1.min(leave);
        }
        if t1 - t0 < 1e-6 {
            continue;
        }
        let at = |t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + dy * t, a[2] + (b[2] - a[2]) * t];
        out.push((at(t0), at(t1)));
    }
    out
}

/// A square bar from `a` to `b` (grid x, height, grid z), `rail_thickness` across, built of four quads in the iron's
/// colour so both sides of every piece show.
fn rail_bar(out: &mut Parts, a: [f32; 3], b: [f32; 3]) {
    let t = flights().dressing.rail_thickness * 0.5;
    let (pa, pb) = (Vec3::from(a), Vec3::from(b));
    let dir = pb - pa;
    if dir.length() < 1e-6 {
        return;
    }
    let up = if dir.normalize().y.abs() > 0.95 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
    let s1 = dir.cross(up).normalize() * t;
    let s2 = dir.cross(s1).normalize() * t;
    let ring = [s1 + s2, -s1 + s2, -s1 - s2, s1 - s2];
    let col = iron();
    for i in 0..4 {
        let (r0, r1) = (ring[i], ring[(i + 1) % 4]);
        let n = (r0 + r1).normalize();
        push(out, Face::Iron, [pa + r0, pb + r0, pb + r1, pa + r1], [Vec2::ZERO; 4], n, col);
    }
}

/// The most metres between two posts of a guard.
pub const POST_SPACING: f32 = 1.0;

/// A guard along `seg` (grid coordinates) standing a hair out on the floor cell `outside` side of the rim: the top rail at
/// the handrail's height (Godot's thin bar), a knee rail at half that, and a post (`guard.height` tall) at each end and at
/// most `POST_SPACING` apart, all in the iron.
fn guard_bar(out: &mut Parts, a: (f32, f32), b: (f32, f32), outside: Cell, d: &Dressing) {
    let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let to_out = (outside.x as f32 + 0.5 - mid.0, outside.y as f32 + 0.5 - mid.1);
    // the rim is axis-aligned: step out along the axis the rim is not along
    let along_x = (b.0 - a.0).abs() > (b.1 - a.1).abs();
    let step = if along_x { (0.0, to_out.1.signum() * 0.02) } else { (to_out.0.signum() * 0.02, 0.0) };
    let len = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    let size = if along_x { vec3(len, d.rail_thickness, d.guard_thickness) } else { vec3(d.guard_thickness, d.rail_thickness, len) };
    for y in [d.rail_height, d.rail_height * 0.5] {
        out.boxes.push((vec3(mid.0 + step.0, y, mid.1 + step.1), size, iron()));
    }
    guard_posts(out, (a.0 + step.0, a.1 + step.1), (b.0 + step.0, b.1 + step.1), d);
}

/// Posts along the line `a`..`b` (grid coordinates): one at each end and enough between to keep them `POST_SPACING` apart.
fn guard_posts(out: &mut Parts, a: (f32, f32), b: (f32, f32), d: &Dressing) {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (len / POST_SPACING).ceil().max(1.0) as usize;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, z) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        out.boxes.push((vec3(x, d.guard_height * 0.5, z), vec3(d.guard_thickness, d.guard_height, d.guard_thickness), iron()));
    }
}

/// The solid wall between a switchback's flights, floor to the upper floor, `divider_width` metres thick with flat ends
/// (Godot's divider box). Straight and spiral flights have none and add nothing. Drawn as one box, so both faces show.
fn divider_wall(out: &mut Parts, f: &Flight, site: &Site) {
    let w = divider_width();
    for seg in lane_dividers(f) {
        let (dx, dz) = (seg.b.0 - seg.a.0, seg.b.1 - seg.a.1);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1e-4 {
            continue;
        }
        let (ux, uz) = (dx / len, dz / len);
        let (nx, nz) = (-uz, ux);
        let top = site.shaft_top();
        let size = vec3(ux.abs() * len + nx.abs() * w, top, uz.abs() * len + nz.abs() * w);
        let mid = ((seg.a.0 + seg.b.0) * 0.5, (seg.a.1 + seg.b.1) * 0.5);
        out.boxes.push((vec3(mid.0, top * 0.5, mid.1), size, tread_tint(shade(top * 0.5, site.h))));
    }
}

/// A spiral's centre post: an upright prism of `POST_SIDES` faces, its radius the data's, floor to the upper floor.
fn post(out: &mut Parts, f: &Flight, site: &Site) {
    let Some(((cx, cz), r)) = spiral_post_with(flights(), f) else { return };
    let top = site.shaft_top();
    let col = iron();
    let at = |i: usize| {
        let a = i as f32 / POST_SIDES as f32 * std::f32::consts::TAU;
        (cx + r * a.cos(), cz + r * a.sin(), a)
    };
    for i in 0..POST_SIDES {
        let (p, q) = (at(i), at((i + 1) % POST_SIDES));
        let mid = (p.2 + q.2 + if i + 1 == POST_SIDES { std::f32::consts::TAU } else { 0.0 }) * 0.5;
        push(
            out,
            Face::Iron,
            [vec3(p.0, top, p.1), vec3(q.0, top, q.1), vec3(q.0, 0.0, q.1), vec3(p.0, 0.0, p.1)],
            [Vec2::ZERO; 4],
            vec3(mid.cos(), 0.0, mid.sin()),
            col,
        );
    }
}

/// The post is drawn as a prism this many sided.
const POST_SIDES: usize = 12;

// ---------------------------------------------------------------------------------------------------------------------
// The "LEVEL n" sign
// ---------------------------------------------------------------------------------------------------------------------

/// The rows of a glyph of the sign's 5 x 7 face, top to bottom, bit 4 the left column. Only the letters and signs the
/// sign says ("LEVEL +n", "LEVEL 0", "LEVEL -n") and the digits.
fn glyph(c: char) -> [u8; 7] {
    match c {
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        _ => [0; 7],
    }
}

/// The lit runs of a sign's lettering as (offset along the reading direction from the sign's middle, run length, height
/// over the sign's middle, in metres): each row of each glyph, runs of lit dots joined.
pub fn sign_runs(text: &str, text_height: f32) -> Vec<(f32, f32, f32)> {
    let dot = text_height / 7.0;
    let n = text.chars().count() as f32;
    let width = (n * 6.0 - 1.0) * dot;
    let mut out = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        let origin = -width * 0.5 + i as f32 * 6.0 * dot;
        for (row, bits) in glyph(ch).into_iter().enumerate() {
            let y = text_height * 0.5 - (row as f32 + 0.5) * dot;
            let mut col = 0;
            while col < 5 {
                if bits >> (4 - col) & 1 == 0 {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < 5 && bits >> (4 - col) & 1 == 1 {
                    col += 1;
                }
                let len = (col - start) as f32 * dot;
                out.push((origin + start as f32 * dot + len * 0.5, len, y));
            }
        }
    }
    out
}

/// A sign's lettering as unlit boxes in the sign's colour, standing on the wall plane facing into the well.
fn sign_lettering(out: &mut Parts, s: &Sign) {
    let d = &flights().dressing;
    let (th, depth) = (d.sign_text_height(), 0.02);
    let dot = th / 7.0;
    let col = colour3(d.sign_colour);
    for (along, len, y) in sign_runs(&s.text, th) {
        let c = vec3(
            s.at.0 + s.right.0 * along + s.normal.0 * depth * 0.5,
            s.y + y,
            s.at.1 + s.right.1 * along + s.normal.1 * depth * 0.5,
        );
        let size = vec3(
            s.right.0.abs() * len * 0.98 + s.normal.0.abs() * depth,
            dot * 0.92,
            s.right.1.abs() * len * 0.98 + s.normal.1.abs() * depth,
        );
        out.glow.push((c, size, col));
    }
}

/// A ladder's two rails and its rungs fixed against side `m` of cell `c`, from height `y0` to `y1`, rungs at `rungs`.
fn ladder_with(out: &mut Parts, c: (i32, i32), m: (i32, i32), y0: f32, y1: f32, rungs: &[f32]) {
    let centre = vec3(c.0 as f32 + 0.5, 0.0, c.1 as f32 + 0.5);
    let to_wall = vec3(m.0 as f32, 0.0, m.1 as f32);
    let across = vec3(m.1.abs() as f32, 0.0, m.0.abs() as f32);
    let base = centre + to_wall * 0.4;
    let (len, mid) = (y1 - y0, (y0 + y1) / 2.0);
    for s in [-0.2f32, 0.2] {
        let p = base + across * s;
        out.boxes.push((vec3(p.x, mid, p.z), vec3(0.08, len, 0.08), IRON));
    }
    for &y in rungs {
        // the rung runs across the ladder: wide along `across`, thin the other way
        let size = vec3(0.07 + 0.33 * across.x, 0.06, 0.07 + 0.33 * across.z);
        out.boxes.push((vec3(base.x, y, base.z), size, IRON));
    }
}

/// A full-height ladder: a rung every 0.3 m from `y0` up.
fn ladder(out: &mut Parts, c: (i32, i32), m: (i32, i32), y0: f32, y1: f32) {
    let rungs: Vec<f32> = (1..=((y1 - y0) / 0.3) as i32).map(|r| y0 + r as f32 * 0.3).collect();
    ladder_with(out, c, m, y0, y1, &rungs);
}

/// Height and width of the low coaming round a ladder hatch.
const COAMING_H: f32 = 0.1;
const COAMING_W: f32 = 0.08;
/// The coaming's iron, a dark steel.
const COAMING: Color = Color::new(0.38, 0.37, 0.36, 1.0);
/// A manhole cover's iron.
const COVER: Color = Color::new(0.46, 0.45, 0.43, 1.0);
/// The back plate behind a shaft's ladder.
const BACK_PLATE: Color = Color::new(0.34, 0.33, 0.32, 1.0);
/// Thickness of the back plate behind the shaft ladder.
const BACK_PLATE_T: f32 = 0.04;
/// How far a propped cover stands open from the floor, in degrees.
const COVER_OPEN_DEG: f32 = 75.0;
/// Diameter of the cover (the hatch cell is 1 m).
const COVER_DIA: f32 = 0.8;

/// A cover propped open about 75 degrees on the rim of side `m` of cell `c`: a disc (octagon) built of axis boxes, one
/// slice per step away from the hinge, each slice as wide as the octagon's chord there. It leans out over the rim, away
/// from the hole, so the hole stays open and the lower floor shows through. A grate is the same outline as a frame and
/// bars.
fn propped_cover(out: &mut Parts, c: (i32, i32), m: (i32, i32), grate: bool) {
    let a = COVER_OPEN_DEG.to_radians();
    let (rise, lean) = (a.sin(), a.cos());
    let hinge = vec3(c.0 as f32 + 0.5 + m.0 as f32 * 0.5, 0.0, c.1 as f32 + 0.5 + m.1 as f32 * 0.5);
    let out_dir = vec3(m.0 as f32, 0.0, m.1 as f32);
    let across = vec3(m.1.abs() as f32, 0.0, m.0.abs() as f32);
    let r = COVER_DIA * 0.5;
    let n = 6;
    let step = COVER_DIA / n as f32;
    for i in 0..n {
        let u = (i as f32 + 0.5) * step;
        // octagon chord at distance u along the disc's diameter (flat-topped octagon, apothem r)
        let d = (u - r).abs();
        let k = r * 0.4142; // half the side of a regular octagon of apothem r
        let chord = if d <= k { r * 2.0 } else { (r - (d - k)).max(0.05) * 2.0 };
        let p = hinge + out_dir * (u * lean) + vec3(0.0, u * rise, 0.0);
        // thin along the lean, as wide as the chord across, as tall as the slice rises (plus a hair so the slices join)
        let thick = step * lean + 0.03;
        let tall = step * rise + 0.03;
        // the hair of overlap goes up, so the lowest slice rests on the floor and nothing dips below it
        let p = p + vec3(0.0, 0.015, 0.0);
        let size = across * chord + vec3(out_dir.x.abs() * thick, tall, out_dir.z.abs() * thick);
        if grate && i % 2 == 1 {
            // between the bars: only the frame's two side rails of the slice
            for s in [-1.0f32, 1.0] {
                let q = p + across * (s * (chord * 0.5 - 0.025));
                let w = across * 0.05 + vec3(out_dir.x.abs() * thick, tall, out_dir.z.abs() * thick);
                out.boxes.push((q, w, GRATE));
            }
        } else {
            out.boxes.push((p, size, if grate { GRATE } else { COVER }));
        }
    }
}

/// Thickness of a closed cover's plate.
const LID_T: f32 = 0.04;

/// A cover lying closed over the hole of cell `c`: a plate flat on the floor across the cell (a manhole's iron, one box),
/// or a grate's frame of four rails with two bars across it. Flat, it covers the hole, so nothing shows through the
/// hatch from above or below.
fn closed_cover(out: &mut Parts, c: (i32, i32), grate: bool) {
    let (cx, cz) = (c.0 as f32 + 0.5, c.1 as f32 + 0.5);
    if !grate {
        out.boxes.push((vec3(cx, LID_T * 0.5, cz), vec3(COVER_DIA + 0.1, LID_T, COVER_DIA + 0.1), COVER));
        return;
    }
    let half = 0.42;
    let rail = 0.06;
    for s in [-1.0f32, 1.0] {
        let edge = half - rail * 0.5;
        out.boxes.push((vec3(cx, LID_T * 0.5, cz + s * edge), vec3(2.0 * half, LID_T, rail), GRATE));
        out.boxes.push((vec3(cx + s * edge, LID_T * 0.5, cz), vec3(rail, LID_T, 2.0 * half), GRATE));
    }
    for k in [-0.2f32, 0.2] {
        out.boxes.push((vec3(cx + k, LID_T * 0.5, cz), vec3(rail, LID_T, 2.0 * half - rail * 2.0), GRATE));
    }
}

/// The hatch of a ladder, seen from the upper level (`Stair::is_ladder`, `dir < 0`): a hole in the floor. There is no
/// pit, no shaft wall and no pit floor (the lower level is the floor you see through it). Round it: a low coaming
/// on the three sides that are not the step cell's side, two rails and two rungs standing `RAIL_H` above the floor on
/// the rails side, and the cover as a model propped open on that side's rim. A grate adds its lit rim strips.
pub fn ladder_down(out: &mut Parts, c: (i32, i32), s: Stair, site: &Site) {
    ladder_down_with_lid(out, c, s, site, Lid::Propped);
}

/// Where a hatch's cover stands: propped open on the rim (the hole is open and the lower floor shows through it), or
/// lying closed over the hole (a plate flat on the floor, so the hatch reads as a cover from above and below).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lid {
    Propped,
    // no caller in the game yet: the game keeps no per-hatch lid state, so every hatch is drawn propped
    #[cfg_attr(not(test), allow(dead_code))]
    Closed,
}

/// `ladder_down` with the cover in `lid`'s pose. The coaming, rails and rungs are the same either way.
pub fn ladder_down_with_lid(out: &mut Parts, c: (i32, i32), s: Stair, site: &Site, lid: Lid) {
    let _ = site;
    let (cx, cz) = (c.0 as f32, c.1 as f32);
    let step = s.step_cell();
    let step_dir = (step.x - c.0, step.y - c.1);
    let m = s.rails_side();
    for d in DIRS {
        if d == step_dir {
            continue;
        }
        let along = vec3(d.1.abs() as f32, 0.0, d.0.abs() as f32);
        let thin = vec3(d.0.abs() as f32, 0.0, d.1.abs() as f32);
        let at = vec3(cx + 0.5 + d.0 as f32 * (0.5 - COAMING_W * 0.5), COAMING_H * 0.5, cz + 0.5 + d.1 as f32 * (0.5 - COAMING_W * 0.5));
        out.boxes.push((at, along * 1.0 + thin * COAMING_W + vec3(0.0, COAMING_H, 0.0), COAMING));
    }
    ladder_with(out, c, m, 0.0, RAIL_H, &[RAIL_H * 0.5, RAIL_H * 0.9]);
    let grate = s.kind == LinkKind::Ladder(Cover::Grate);
    match lid {
        Lid::Propped => propped_cover(out, c, m, grate),
        Lid::Closed => closed_cover(out, c, grate),
    }
    if grate {
        // the grate's lit rim: four thin warm strips round the cell's edge, so the cover's outline reads from above
        for (centre, size) in [
            (vec3(cx + 0.5, 0.03, cz + 0.02), vec3(1.0, 0.03, 0.04)),
            (vec3(cx + 0.5, 0.03, cz + 0.98), vec3(1.0, 0.03, 0.04)),
            (vec3(cx + 0.02, 0.03, cz + 0.5), vec3(0.04, 0.03, 1.0)),
            (vec3(cx + 0.98, 0.03, cz + 0.5), vec3(0.04, 0.03, 1.0)),
        ] {
            out.glow.push((centre, size, RIM));
        }
    }
}

/// A ladder's foot, seen from the lower level (`dir > 0`): the rails and rungs on the rails side from the floor up
/// through the slab gap to the floor above (`site.shaft_top()`) and a thin back plate behind them, so the shaft reads
/// as a fixture. No hatch plate, no grate
/// bars and no ceiling: the shaft is open above, and the cell above is the real hole.
pub fn ladder_up(out: &mut Parts, c: (i32, i32), s: Stair, site: &Site) {
    let m = s.rails_side();
    // up to the floor above (`shaft_top`), which is the ceiling when the region's ceiling is as high as the storey
    let top = site.shaft_top();
    ladder(out, c, m, 0.0, top);
    let centre = vec3(c.0 as f32 + 0.5, top * 0.5, c.1 as f32 + 0.5);
    let to_wall = vec3(m.0 as f32, 0.0, m.1 as f32);
    let across = vec3(m.1.abs() as f32, 0.0, m.0.abs() as f32);
    let thin = vec3(m.0.abs() as f32, 0.0, m.1.abs() as f32);
    let at = centre + to_wall * (0.5 - BACK_PLATE_T * 0.5);
    out.boxes.push((at, across * 0.6 + thin * BACK_PLATE_T + vec3(0.0, top, 0.0), BACK_PLATE));
    // the shaft's other sides are lined by the world mesh (world_mesh::edge_fill): the ladder cell's own wall columns
    // rise to the storey and a ceilinged neighbour's slab rim closes the gap above its ceiling, so nothing is drawn
    // twice on one plane here
}

/// The light of a stairwell (a cool shaft of daylight at a ladder's sewer end), so its flight reads in the dark. A flight's
/// light is Godot's (warm `ffd29a`, energy 0.7, range 7 m in its 4 m storey; here scaled to the storey and with the
/// renderer's falloff factor, all data: `dressing.lamp`), hanging over the middle of the well. Only the dir > 0 stair (the
/// owner of the flight or shaft) should have its lamp pushed; callers (world_mesh, world_stack) filter out dir < 0
/// stairs, whose lamp is not wanted: the upper level's hatch has no shaft to light, the lower level's lamp does that.
/// A dir < 0 ladder still returns a harmless lamp at the hatch so the type stays Lamp, and a dir < 0 stair the same
/// lamp held a storey `h` lower.
pub fn lamp(s: Stair, h: f32) -> Lamp {
    let (cx, cz) = s.cell.center();
    if s.is_ladder() {
        return if s.dir < 0 {
            // never pushed; a dim placeholder just over the hatch
            Lamp { pos: vec3(cx, 1.2, cz), range: 4.0, color: Color::new(1.0, 0.72, 0.42, 1.0), energy: 1.0, flicker: Flicker::NONE }
        } else {
            Lamp { pos: vec3(cx, h * 0.45, cz), range: 7.0, color: Color::new(0.8, 0.88, 1.0, 1.0), energy: 1.7, flicker: Flicker::NONE }
        };
    }
    let rules = flights();
    let d = &rules.dressing;
    let (x, y, z) = s.flight(0).map_or((cx, d.lamp_y(), cz), |f| lamp_place_with(rules, &f));
    let y = if s.dir < 0 { y - h } else { y };
    Lamp { pos: vec3(x, y, z), range: d.lamp_range(), color: colour3(d.lamp_colour), energy: d.lamp_energy(), flicker: Flicker::NONE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::facility::{Cell, LinkKind};
    use crate::world::flight::{well_footprint, FlightShape};
    use std::collections::HashSet;

    const H: f32 = 5.0;
    const RUNS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];
    const SHAPES: [FlightShape; 3] = [FlightShape::Switchback, FlightShape::Straight, FlightShape::Spiral];
    const EPS: f32 = 1e-4;

    fn stair(dir: i32, run: (i32, i32), shape: FlightShape) -> Stair {
        Stair { cell: Cell::new(20, 20), dir, run, kind: LinkKind::Stairs, shape, clockwise: false }
    }

    /// The level that holds the stair: the upper one (0) for a flight down, the lower (-1) for a flight up.
    fn level_of(s: Stair) -> i32 {
        if s.dir < 0 {
            0
        } else {
            -1
        }
    }

    fn well_set(s: Stair) -> HashSet<(i32, i32)> {
        s.well_cells().iter().map(|c| (c.x, c.y)).collect()
    }

    /// The flight's quads, every well cell, with nothing around it but open floor.
    fn build(s: Stair) -> Parts {
        build_at(s, H)
    }

    /// The same under a ceiling of height `wall_h` (the storey is still `H`).
    fn build_at(s: Stair, wall_h: f32) -> Parts {
        let well = well_set(s);
        let (well_fn, solid_fn) = (|x, z| well.contains(&(x, z)), |_, _| false);
        let site = Site { h: H, wall_h, tiers: 1.0, well: &well_fn, solid: &solid_fn };
        let mut out = Parts::default();
        for c in s.well_cells() {
            flight_cell(&mut out, (c.x, c.y), s, level_of(s), &site);
        }
        out
    }

    /// Every tile of the flight.
    fn all_tiles(s: Stair) -> Vec<Tile> {
        s.well_cells().iter().flat_map(|c| tiles(s, level_of(s), (c.x, c.y))).collect()
    }

    fn ramp_quads(p: &Parts) -> Vec<&Quad> {
        p.quads.iter().filter(|q| q.face == Face::Plain).collect()
    }

    fn iron_quads(p: &Parts) -> Vec<&Quad> {
        p.quads.iter().filter(|q| q.face == Face::Iron).collect()
    }

    fn top_faces(p: &Parts) -> Vec<&Quad> {
        p.quads.iter().filter(|q| q.face == Face::Plain && q.n.y > 0.0).collect()
    }

    fn base_of(s: Stair) -> f32 {
        level_of(s) as f32 * H
    }

    #[test]
    fn flight_tiles_follow_the_surface() {
        for shape in SHAPES {
            for run in RUNS {
                let s = stair(1, run, shape);
                let f = s.flight(level_of(s)).unwrap();
                let ts = all_tiles(s);
                assert!(!ts.is_empty(), "{shape:?} {run:?}: the holding level has tiles");
                for t in &ts {
                    // a tile exists where the surface does: at its middle, or (where the band only clips it) a quarter point
                    let (cx, cz) = (t.x0 + t.size * 0.5, t.z0 + t.size * 0.5);
                    let q = t.size * 0.25;
                    let surface = [(0.0, 0.0), (-q, -q), (q, -q), (-q, q), (q, q)]
                        .into_iter()
                        .find_map(|(dx, dz)| flight_surface_y_with(flights(), &f, cx + dx, cz + dz))
                        .expect("a tile only exists where the surface does")
                        - base_of(s);
                    // the tile is planar-ish: its mean is the surface within the surface's own slope across the tile
                    assert!((t.mean() - surface).abs() <= 0.35, "{shape:?} {run:?}: tile mean {} surface {surface}", t.mean());
                    assert!(t.y.iter().all(|y| *y >= -EPS && *y <= H + EPS), "{shape:?} {run:?}: a corner at {:?}", t.y);
                    assert!((t.size - 1.0 / flights().tiles_for(shape) as f32).abs() < 1e-6);
                }
                // the other level holds nothing of this flight
                assert!(tiles(stair(-1, run, shape), 0, (20, 21)).iter().all(|t| t.y.iter().all(|y| *y <= EPS)), "an upper cell's tiles are below its floor");
            }
        }
    }

    /// The ramp has no step treads: neighbouring tiles of a solid flight differ by no more than the slope across a tile,
    /// and nothing draws a riser.
    #[test]
    fn the_ramp_is_smooth_with_no_treads_or_risers() {
        for shape in [FlightShape::Switchback, FlightShape::Straight] {
            let slope = crate::world::flight::max_riser_of(flights(), shape) * flights().steps_per_cell;
            for run in RUNS {
                let s = stair(1, run, shape);
                let per = flights().tiles_for(shape);
                let map: std::collections::HashMap<(i32, i32), Tile> = all_tiles(s)
                    .into_iter()
                    .map(|t| (((t.x0 * per as f32).round() as i32, (t.z0 * per as f32).round() as i32), t))
                    .collect();
                for (&(ix, iz), t) in &map {
                    for d in [(1, 0), (0, 1)] {
                        if let Some(n) = map.get(&(ix + d.0, iz + d.1)) {
                            assert!(
                                (t.mean() - n.mean()).abs() <= slope * t.size + 1e-3,
                                "{shape:?} {run:?}: a step of {} between tiles ({ix}, {iz}) and ({}, {})",
                                (t.mean() - n.mean()).abs(),
                                ix + d.0,
                                iz + d.1
                            );
                        }
                    }
                }
                // sloped, never flat treads and vertical risers: every ramp quad is a top face
                let p = build(s);
                assert!(ramp_quads(&p).iter().all(|q| q.n.y > 0.5), "{shape:?} {run:?}: a riser or an underside");
                assert!(p.glow.iter().all(|(_, s, _)| s.y < 0.2), "{shape:?} {run:?}: no nosing strips, only the sign's dots");
                // a sloped tile is not flat: some tile of each flight slopes
                assert!(map.values().any(|t| (t.y[0] - t.y[2]).abs() > 0.05), "{shape:?} {run:?}: the ramp slopes");
            }
        }
    }

    #[test]
    fn every_surface_point_falls_on_a_tile() {
        for shape in SHAPES {
            for run in RUNS {
                let s = stair(1, run, shape);
                let f = s.flight(level_of(s)).unwrap();
                for c in s.well_cells() {
                    let ts = tiles(s, level_of(s), (c.x, c.y));
                    for i in 0..6 {
                        for j in 0..6 {
                            let (x, z) = (c.x as f32 + (i as f32 + 0.5) / 6.0, c.y as f32 + (j as f32 + 0.5) / 6.0);
                            if flight_surface_y_with(flights(), &f, x, z).is_some() {
                                assert!(
                                    ts.iter().any(|t| x >= t.x0 && x < t.x0 + t.size && z >= t.z0 && z < t.z0 + t.size),
                                    "{shape:?} {run:?}: ({x}, {z}) has a surface but no tile"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_mesh_has_a_top_face_per_tile_and_stays_inside_the_storey() {
        for shape in SHAPES {
            for dir in [-1, 1] {
                for run in RUNS {
                    let s = stair(dir, run, shape);
                    let p = build(s);
                    if dir < 0 {
                        // the upper level owns no tiles: the well is open
                        assert!(ramp_quads(&p).is_empty(), "{shape:?} {dir} {run:?}");
                    } else {
                        assert_eq!(top_faces(&p).len(), all_tiles(s).len(), "{shape:?} {dir} {run:?}");
                    }
                    for q in &p.quads {
                        for v in q.c {
                            assert!(v.y >= -EPS && v.y <= H + EPS, "{shape:?} {dir} {run:?}: {:?} vertex at {}", q.face, v.y);
                        }
                    }
                    for (c, sz, _) in p.boxes.iter().chain(&p.glow) {
                        assert!(c.y - sz.y * 0.5 >= -EPS && c.y + sz.y * 0.5 <= H + EPS, "{shape:?} {dir} {run:?}: a box {c:?} {sz:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn steps_are_never_pure_black() {
        for shape in SHAPES {
            for dir in [-1, 1] {
                let p = build(stair(dir, (0, -1), shape));
                for q in p.quads.iter().filter(|q| q.face != Face::Plain || q.n.y > 0.5) {
                    assert!(q.col.r >= 0.28 && q.col.g >= 0.28 && q.col.b >= 0.28, "{dir}: {:?} {:?}", q.face, q.col);
                }
                for q in ramp_quads(&p) {
                    // the underside of a hollow ramp sits at 80 % of its top
                    assert!(q.col.r >= 0.4 && q.col.g >= 0.4 && q.col.b >= 0.36, "{shape:?} {dir}: {:?}", q.col);
                }
            }
        }
        assert!(TREAD_STONE.r >= 0.7 && TREAD_STONE.g >= 0.7 && TREAD_STONE.b >= 0.65, "{TREAD_STONE:?}");
    }

    #[test]
    fn a_flight_down_draws_no_floor_lining_or_tiles_over_its_well() {
        for shape in SHAPES {
            for run in RUNS {
                let p = build(stair(-1, run, shape));
                assert!(
                    p.quads.iter().all(|q| q.face == Face::Iron),
                    "{shape:?} {run:?}: the upper cell emits no floor, lining or tile quad, only rails"
                );
                assert!(!p.boxes.is_empty(), "{shape:?} {run:?}: guards");
            }
        }
    }

    #[test]
    fn a_flight_up_leaves_no_ceiling_or_cap_over_its_cells() {
        for shape in SHAPES {
            for run in RUNS {
                let p = build(stair(1, run, shape));
                // no wall or floor quad faces -Y (a ceiling or cap); only a hollow ramp's underside does
                assert!(p.quads.iter().filter(|q| q.n.y < -0.01).all(|q| matches!(q.face, Face::Plain | Face::Iron)), "{shape:?} {run:?}: a cap");
                assert!(p.quads.iter().all(|q| q.c.iter().all(|v| v.y <= H + EPS)), "{shape:?} {run:?}: wall above the ceiling");
                assert!(p.boxes.iter().all(|(c, s, _)| c.y + s.y * 0.5 <= H + EPS));
            }
        }
    }

    /// A spiral's shell has a floor under it; a solid ramp does not need one, the wedge stands on the floor.
    #[test]
    fn a_spiral_has_a_floor_under_its_shell_and_a_hollow_underside() {
        for run in RUNS {
            let s = stair(1, run, FlightShape::Spiral);
            let p = build(s);
            let floors = p.quads.iter().filter(|q| q.face == Face::Floor).count();
            assert_eq!(floors, s.well_cells().len(), "{run:?}: a floor quad per well cell, no bottomless void");
            assert!(p.quads.iter().filter(|q| q.face == Face::Floor).all(|q| q.c.iter().all(|v| v.y.abs() < EPS) && q.n.y > 0.5));
            let tops = top_faces(&p).len();
            let unders = ramp_quads(&p).iter().filter(|q| q.n.y < 0.0).count();
            assert_eq!(tops, unders, "{run:?}: every tile has its underside");
            let w = build(stair(1, run, FlightShape::Switchback));
            assert!(w.quads.iter().all(|q| q.face != Face::Floor), "{run:?}: a solid wedge needs no floor quad");
        }
    }

    #[test]
    fn a_switchback_has_a_one_metre_solid_divider_floor_to_the_upper_floor() {
        for shape in SHAPES {
            for run in RUNS {
                let s = stair(1, run, shape);
                let p = build(s);
                let dividers: Vec<_> = p.boxes.iter().filter(|(_, sz, _)| (sz.y - H).abs() < EPS).collect();
                if shape == FlightShape::Switchback {
                    assert_eq!(dividers.len(), 1, "{run:?}");
                    let (c, sz, _) = dividers[0];
                    assert!((c.y - H * 0.5).abs() < EPS, "{run:?}: stands on the floor");
                    let (long, thick) = (sz.x.max(sz.z), sz.x.min(sz.z));
                    assert!((thick - 1.0).abs() < EPS && (long - 3.0).abs() < EPS, "{run:?}: {sz:?} is 3 m of one metre wall");
                } else {
                    assert!(dividers.is_empty(), "{shape:?} {run:?}: only a switchback has a divider");
                }
            }
        }
    }

    #[test]
    fn a_spiral_has_its_post_floor_to_the_upper_floor() {
        for run in RUNS {
            let s = stair(1, run, FlightShape::Spiral);
            let p = build(s);
            let post: Vec<&Quad> = iron_quads(&p).into_iter().filter(|q| (q.c[0].y - H).abs() < EPS && q.c[2].y.abs() < EPS).collect();
            assert_eq!(post.len(), POST_SIDES, "{run:?}");
            let f = s.flight(-1).unwrap();
            let ((cx, cz), r) = spiral_post_with(flights(), &f).unwrap();
            for q in &post {
                for v in [q.c[0], q.c[1]] {
                    assert!(((v.x - cx).hypot(v.z - cz) - r).abs() < 1e-3, "{run:?}: a post corner off the circle");
                }
            }
            for shape in [FlightShape::Switchback, FlightShape::Straight] {
                let p = build(stair(1, run, shape));
                assert!(iron_quads(&p).iter().all(|q| !((q.c[0].y - H).abs() < EPS && q.c[2].y.abs() < EPS)), "{shape:?}: no post");
            }
        }
    }

    /// The handrails: a bar along every rail the flight's data gives, 0.95 over the tread, the lower level's up to the
    /// upper floor and the rest standing on the upper floor.
    #[test]
    fn the_handrails_run_the_flight_and_pass_through_the_upper_floor() {
        let rules = flights();
        let t = rules.dressing.rail_thickness;
        for shape in SHAPES {
            for run in RUNS {
                let lower = stair(1, run, shape);
                let upper = stair(-1, run, shape);
                let f = lower.flight(-1).unwrap();
                let rails = handrails_with(rules, &f);
                assert_eq!(rails.len(), match shape { FlightShape::Switchback => 4, FlightShape::Straight => 2, FlightShape::Spiral => 1 }, "{shape:?}");
                let (mut low_bars, mut high_bars) = (0, 0);
                for r in &rails {
                    low_bars += clip_segments(&r.points, 0.0, H - t * 0.5).len();
                    high_bars += clip_segments(&r.points, H + t * 0.5, f32::MAX).len();
                }
                let post = if shape == FlightShape::Spiral { POST_SIDES } else { 0 };
                let (lp, up) = (build(lower), build(upper));
                assert_eq!(iron_quads(&lp).len(), low_bars * 4 + post, "{shape:?} {run:?}: four faces to every bar below the floor above");
                let slab_rail = usize::from(shape == FlightShape::Spiral) * 8;
                assert_eq!(iron_quads(&up).len(), high_bars * 4 + slab_rail, "{shape:?} {run:?}: and to every bar over the upper floor");
                assert!(low_bars > 0 && high_bars > 0, "{shape:?}: rails climb past the floor");
                // 0.95 over the tread at the top of the flight: the highest rail point stands 0.95 over the upper floor
                let top = iron_quads(&up).iter().flat_map(|q| q.c.iter().map(|v| v.y)).fold(0.0f32, f32::max);
                assert!((top - (rules.dressing.rail_height + t * 0.5)).abs() < 2.0 * t, "{shape:?} {run:?}: the rails top out at {top}");
                let low_top = iron_quads(&lp).iter().flat_map(|q| q.c.iter().map(|v| v.y)).fold(0.0f32, f32::max);
                assert!(low_top <= H + EPS && low_top > H - 0.1, "{shape:?} {run:?}: the lower level's rails are cut at the upper floor ({low_top})");
            }
        }
    }

    #[test]
    fn clip_segments_cuts_a_polyline_to_a_band() {
        let line = [[0.0, 1.0, 0.0], [10.0, 6.0, 0.0], [20.0, 6.0, 0.0]];
        let below = clip_segments(&line, 0.0, 5.0);
        assert_eq!(below.len(), 1);
        assert!((below[0].1[1] - 5.0).abs() < 1e-5 && (below[0].1[0] - 8.0).abs() < 1e-5);
        let above = clip_segments(&line, 5.0, f32::MAX);
        assert_eq!(above.len(), 2);
        assert!((above[0].0[1] - 5.0).abs() < 1e-5 && (above[0].0[0] - 8.0).abs() < 1e-5);
        assert_eq!(above[1], ([10.0, 6.0, 0.0], [20.0, 6.0, 0.0]));
        assert!(clip_segments(&line, 7.0, 9.0).is_empty());
    }

    /// The guards: Godot's thin bar at 0.95 m along the rim, a gap where the stairs arrive.
    #[test]
    fn the_landing_is_guarded_at_the_rim_but_for_the_head_of_the_stairs() {
        let rules = flights();
        for shape in SHAPES {
            for run in RUNS {
                let s = stair(-1, run, shape);
                let f = s.flight(0).unwrap();
                let guards = landing_guards_with(rules, &f);
                assert!(!guards.is_empty(), "{shape:?} {run:?}");
                let p = build(s);
                let bars: Vec<_> = p.boxes.iter().filter(|(c, sz, _)| (c.y - rules.dressing.rail_height).abs() < EPS && (sz.y - rules.dressing.rail_thickness).abs() < EPS).collect();
                assert_eq!(bars.len(), guards.len(), "{shape:?} {run:?}: a bar per guard edge");
                // no guard over the landing cell A, nor the arriving flight's first cells
                let fp = well_footprint(&f);
                assert!(guards.iter().all(|g| g.outside != fp.landing), "{shape:?} {run:?}: the landing edge is the gap");
                for g in &guards {
                    assert!(!fp.upper_hole.contains(&g.outside), "{shape:?} {run:?}: a guard stands on floor");
                }
                if shape == FlightShape::Switchback {
                    assert_eq!(bars.len(), 3);
                    for (_, sz, _) in bars {
                        assert!((sz.x.max(sz.z) - 1.0).abs() < EPS, "a metre of rail per cell edge");
                    }
                }
            }
        }
    }

    /// A spiral's rim is railed along the whole guard path the collision closes (posts, a top rail at 0.95 and a knee
    /// rail, in the iron of the other rails) and open at the arrival.
    #[test]
    fn a_spiral_rim_has_posts_and_rails_along_its_guard_path_and_a_gap_at_the_arrival() {
        let rules = flights();
        let d = &rules.dressing;
        for run in RUNS {
            for cw in [false, true] {
                let s = Stair { clockwise: cw, ..stair(-1, run, FlightShape::Spiral) };
                let f = s.flight(0).unwrap();
                let guards = landing_guards_with(rules, &f);
                assert!(guards.len() >= 15, "{run:?} cw {cw}: a guard on every closed edge of the rim ({})", guards.len());
                let p = build(s);
                let is = |c: &Vec3, sz: &Vec3, y: f32, h: f32| (c.y - y).abs() < EPS && (sz.y - h).abs() < EPS;
                let tops: Vec<_> = p.boxes.iter().filter(|(c, sz, _)| is(c, sz, d.rail_height, d.rail_thickness)).collect();
                let knees: Vec<_> = p.boxes.iter().filter(|(c, sz, _)| is(c, sz, d.rail_height * 0.5, d.rail_thickness)).collect();
                let posts: Vec<_> = p.boxes.iter().filter(|(c, sz, _)| is(c, sz, d.guard_height * 0.5, d.guard_height)).collect();
                assert_eq!(tops.len(), guards.len(), "{run:?} cw {cw}: a top rail per guard");
                assert_eq!(knees.len(), guards.len(), "{run:?} cw {cw}: a knee rail per guard");
                assert!(posts.len() > guards.len(), "{run:?} cw {cw}: posts along the rail");
                assert!(tops.iter().chain(&knees).chain(&posts).all(|(_, _, col)| *col == iron()), "{run:?} cw {cw}: the iron of the other rails");
                // the top rail covers every guard's segment end to end, and a post stands at each end of it
                for g in &guards {
                    for i in 0..=10 {
                        let t = i as f32 / 10.0;
                        let (x, z) = (g.seg.a.0 + (g.seg.b.0 - g.seg.a.0) * t, g.seg.a.1 + (g.seg.b.1 - g.seg.a.1) * t);
                        assert!(
                            tops.iter().any(|(c, sz, _)| (x - c.x).abs() <= sz.x * 0.5 + 0.03 && (z - c.z).abs() <= sz.z * 0.5 + 0.03),
                            "{run:?} cw {cw}: no rail at ({x}, {z}) of the guard {g:?}"
                        );
                    }
                    for e in [g.seg.a, g.seg.b] {
                        assert!(posts.iter().any(|(c, _, _)| (c.x - e.0).hypot(c.z - e.1) < 0.05), "{run:?} cw {cw}: no post at the end {e:?} of {g:?}");
                    }
                }
                // the gap: nothing stands across the doorway edge, where the stairs arrive
                for c in crate::world::flight::landing_row_with(rules, &f) {
                    let (x, z) = (c.x as f32 + 0.5 + f.run.0 as f32 * 0.5, c.y as f32 + 0.5 + f.run.1 as f32 * 0.5);
                    for (c, sz, _) in tops.iter().chain(&knees).chain(&posts) {
                        assert!(
                            (x - c.x).abs() > sz.x * 0.5 + 0.3 || (z - c.z).abs() > sz.z * 0.5 + 0.3,
                            "{run:?} cw {cw}: a rail piece at {c:?} in the arrival gap ({x}, {z})"
                        );
                    }
                }
            }
        }
    }

    /// A spiral guard standing beside a wall is left out like any other.
    #[test]
    fn a_spiral_guard_beside_a_wall_is_left_out() {
        let s = stair(-1, (0, 1), FlightShape::Spiral);
        let well = well_set(s);
        let well_fn = |x: i32, z: i32| well.contains(&(x, z));
        let all_open = |_: i32, _: i32| false;
        let all_walls = |x: i32, z: i32| !well.contains(&(x, z));
        let count = |solid: &dyn Fn(i32, i32) -> bool| {
            let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well_fn, solid };
            let mut p = Parts::default();
            for c in s.well_cells() {
                flight_cell(&mut p, (c.x, c.y), s, 0, &site);
            }
            p.boxes.len()
        };
        assert!(count(&all_open) > count(&all_walls) + 30, "the walled rim has no guards; only the slab rail's posts stand");
    }

    #[test]
    fn a_guard_beside_a_wall_is_left_out() {
        let s = stair(-1, (0, 1), FlightShape::Straight);
        let well = well_set(s);
        let landing = s.landing();
        let wall = |x: i32, z: i32| (x, z) != (landing.x, landing.y) && !well.contains(&(x, z));
        let well_fn = |x: i32, z: i32| well.contains(&(x, z));
        let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well_fn, solid: &wall };
        let mut p = Parts::default();
        for c in s.well_cells() {
            flight_cell(&mut p, (c.x, c.y), s, 0, &site);
        }
        assert!(p.boxes.is_empty(), "every neighbour is a wall, so there is no floor edge to guard");
    }

    #[test]
    fn each_level_has_its_sign_in_godots_gold_over_its_own_floor() {
        let rules = flights();
        let gold = colour3(rules.dressing.sign_colour);
        for shape in SHAPES {
            if shape == FlightShape::Spiral {
                for run in RUNS {
                    for dir in [-1, 1] {
                        assert!(build(stair(dir, run, shape)).glow.is_empty(), "no LEVEL lettering on a spiral staircase ({run:?} {dir})");
                    }
                }
                continue;
            }
            for run in RUNS {
                for dir in [-1, 1] {
                    let s = stair(dir, run, shape);
                    let f = s.flight(level_of(s)).unwrap();
                    let p = build(s);
                    let sign = signs_with(rules, &f).into_iter().find(|g| g.upper == (dir < 0)).unwrap();
                    let lit: Vec<_> = p.glow.iter().filter(|(_, _, c)| *c == gold).collect();
                    let runs = sign_runs(&sign.text, rules.dressing.sign_text_height());
                    assert_eq!(lit.len(), runs.len(), "{shape:?} {run:?} dir {dir}: a box per lit run of {}", sign.text);
                    assert!(lit.len() > 20);
                    // the lettering is centred 2.625 m over the floor, on the wall plane, 2 cm deep
                    let ys: Vec<f32> = lit.iter().map(|(c, _, _)| c.y).collect();
                    let mid = (ys.iter().cloned().fold(f32::MAX, f32::min) + ys.iter().cloned().fold(f32::MIN, f32::max)) * 0.5;
                    assert!((mid - rules.dressing.sign_y()).abs() < 0.1, "{shape:?} {run:?} dir {dir}: lettering at {mid}");
                    for (c, sz, _) in &lit {
                        let off = (c.x - sign.at.0) * sign.normal.0 + (c.z - sign.at.1) * sign.normal.1;
                        assert!(off > 0.0 && off < 0.03, "stands off the wall by {off}");
                        assert!((sz.x * sign.normal.0.abs() + sz.z * sign.normal.1.abs() - 0.02).abs() < EPS);
                    }
                    // and reads along `right`: the extent along it is the text's width, wider than tall
                    let along: Vec<f32> = lit.iter().map(|(c, _, _)| (c.x - sign.at.0) * sign.right.0 + (c.z - sign.at.1) * sign.right.1).collect();
                    let span = along.iter().cloned().fold(f32::MIN, f32::max) - along.iter().cloned().fold(f32::MAX, f32::min);
                    assert!(span > 4.0 * rules.dressing.sign_text_height(), "{shape:?} {run:?}: {} is {span} m wide", sign.text);
                }
            }
        }
    }

    #[test]
    fn the_sign_font_has_the_letters_and_digits_of_a_level_sign() {
        for text in ["LEVEL +1", "LEVEL 0", "LEVEL -1", "LEVEL -3", "LEVEL +2"] {
            for ch in text.chars().filter(|c| *c != ' ') {
                assert!(glyph(ch).iter().any(|r| *r != 0), "{ch:?} has a face");
            }
        }
        assert!(glyph(' ').iter().all(|r| *r == 0), "a space is blank");
        // an L is six dots down the left edge and a run of five at the bottom
        let runs = sign_runs("L", 0.7);
        assert_eq!(runs.len(), 7);
        assert!(runs.iter().filter(|(_, len, _)| (*len - 0.5).abs() < 1e-4).count() == 1, "one full-width run");
        // text is centred
        let (lo, hi) = sign_runs("LEVEL 0", 0.7)
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), (c, len, _)| (lo.min(c - len * 0.5), hi.max(c + len * 0.5)));
        assert!((lo + hi).abs() < 1e-3, "centred on the sign: {lo} {hi}");
    }

    #[test]
    fn the_stairwell_lamp_hangs_over_its_flight_and_the_ladder_lamp_is_the_lower_levels() {
        let d = &flights().dressing;
        for shape in SHAPES {
            for dir in [-1, 1] {
                for run in RUNS {
                    let s = stair(dir, run, shape);
                    let l = lamp(s, H);
                    let fp = well_footprint(&s.flight(level_of(s)).unwrap());
                    assert!(
                        fp.upper_hole.iter().chain(&fp.upper_clear).any(|c| (c.x as f32..c.x as f32 + 1.0).contains(&l.pos.x) && (c.y as f32..c.y as f32 + 1.0).contains(&l.pos.z)),
                        "{shape:?} {dir} {run:?}: {:?}",
                        l.pos
                    );
                    // Godot's warm light: ffd29a, energy 0.7 (doubled for this renderer), 7 m in a 4 m storey is 8.75 m in 5
                    assert_eq!(l.color, colour3(d.lamp_colour));
                    assert!((l.color.r - 1.0).abs() < 1e-3 && (l.color.g - 0xd2 as f32 / 255.0).abs() < 1e-3 && (l.color.b - 0x9a as f32 / 255.0).abs() < 1e-3);
                    assert!((l.energy - 0.7 * 2.0).abs() < 1e-5 && (l.range - 8.75).abs() < 1e-4, "{shape:?}: energy {} range {}", l.energy, l.range);
                    if dir > 0 {
                        assert!((l.pos.y - (H * 0.5 + 1.4 * 1.25)).abs() < 1e-4, "{shape:?}: over mid storey: {}", l.pos.y);
                        assert!(l.pos.y < H);
                    } else {
                        assert!(l.pos.y < 0.0, "the upper record's copy is held a storey lower");
                    }
                }
            }
        }
        // the shaft foot (dir > 0) has its cool lamp in the shaft; the hatch (dir < 0) has no shaft to light
        let l = lamp(ladder(1, Cover::Manhole), H);
        assert!(l.pos.y > 0.0 && l.pos.y < H && l.energy > 0.0);
        assert!(lamp(ladder(-1, Cover::Manhole), H).pos.y >= 0.0, "nothing is lit below the upper floor any more");
    }

    #[test]
    fn the_iron_is_godots_colour_lifted() {
        let c = iron();
        let d = &flights().dressing;
        assert!((c.r - 0x3b as f32 / 255.0 * d.iron_lift).abs() < 1e-3);
        assert!(c.r > 0.3 && c.g > 0.3 && c.b > 0.3, "reads in the dark: {c:?}");
    }

    #[test]
    fn a_low_ceiling_still_lines_the_flight_side_walls_to_the_tread() {
        let mut checked = 0;
        for shape in [FlightShape::Switchback, FlightShape::Straight] {
            for run in RUNS {
                let s = stair(1, run, shape);
                let (low, full) = (build_at(s, 4.0), build_at(s, H));
                let wall_tops = |p: &Parts| {
                    let mut v: Vec<f32> = p.quads.iter().filter(|q| q.face == Face::Wall).map(|q| q.c.iter().map(|v| v.y).fold(f32::MIN, f32::max)).collect();
                    v.sort_by(f32::total_cmp);
                    v
                };
                assert_eq!(wall_tops(&low), wall_tops(&full), "{shape:?} {run:?}: the lining does not depend on the cell's ceiling");
                let highest = wall_tops(&low).last().copied().unwrap_or(0.0);
                assert!(highest > 4.0 + EPS, "{shape:?} {run:?}: a side wall reaches the ramp above the 4 m ceiling ({highest})");
                assert!(highest <= H + EPS, "{shape:?} {run:?}: never past the floor above ({highest})");
                checked += 1;
            }
        }
        assert_eq!(checked, 2 * RUNS.len());
    }

    /// The divider rises to the floor above, not to a lower ceiling.
    #[test]
    fn a_low_ceiling_still_runs_the_divider_wall_to_the_floor_above() {
        for run in RUNS {
            let s = stair(1, run, FlightShape::Switchback);
            let tops = |p: &Parts| {
                let mut v: Vec<f32> = p.boxes.iter().map(|(c, sz, _)| c.y + sz.y * 0.5).collect();
                v.sort_by(f32::total_cmp);
                v
            };
            let (low, full) = (build_at(s, 4.0), build_at(s, H));
            assert_eq!(tops(&low), tops(&full), "{run:?}: the divider does not depend on the cell's ceiling");
            assert!((tops(&low).last().copied().unwrap() - H).abs() < EPS, "{run:?}: divider top");
        }
    }

    /// The well's holes are lit and lined: a smooth ramp is sunk into walls that run to the upper floor.
    #[test]
    fn a_switchback_wedge_shows_its_side_below_the_arriving_flight() {
        for run in RUNS {
            let s = stair(1, run, FlightShape::Switchback);
            let p = build(s);
            // the wall under the arriving flight's top end rises to the upper floor at the landing row's edge
            let tall = p.quads.iter().filter(|q| q.face == Face::Wall && q.c.iter().any(|v| v.y > H - 0.3)).count();
            assert!(tall >= 2 * flights().tiles_for(FlightShape::Switchback) as usize, "{run:?}: {tall} tall wall faces");
        }
    }

    const LADDER_CELL: (i32, i32) = (10, 10);

    fn ladder(dir: i32, cover: Cover) -> Stair {
        Stair { cell: Cell::new(LADDER_CELL.0, LADDER_CELL.1), dir, run: (1, 0), kind: LinkKind::Ladder(cover), shape: FlightShape::Switchback, clockwise: false }
    }

    fn ladder_parts(dir: i32, cover: Cover) -> Parts {
        ladder_parts_at(dir, cover, H)
    }

    fn ladder_parts_at(dir: i32, cover: Cover, wall_h: f32) -> Parts {
        let (well, solid) = (|_: i32, _: i32| false, |_: i32, _: i32| false);
        let site = Site { h: H, wall_h, tiers: 1.0, well: &well, solid: &solid };
        let mut p = Parts::default();
        let s = ladder(dir, cover);
        if dir < 0 {
            ladder_down(&mut p, LADDER_CELL, s, &site);
        } else {
            ladder_up(&mut p, LADDER_CELL, s, &site);
        }
        p
    }

    #[test]
    fn a_ladder_down_has_no_pit_floor_or_shaft_walls() {
        for cover in [Cover::Manhole, Cover::Grate] {
            let p = ladder_parts(-1, cover);
            assert!(p.quads.is_empty(), "{cover:?}: no pit walls, no pit floor");
            let lowest = p.boxes.iter().chain(&p.glow).map(|(c, s, _)| c.y - s.y * 0.5).fold(f32::MAX, f32::min);
            assert!(lowest >= -EPS, "{cover:?}: nothing below the floor ({lowest})");
        }
    }

    #[test]
    fn upper_rails_top_out_at_rail_h() {
        for cover in [Cover::Manhole, Cover::Grate] {
            let p = ladder_parts(-1, cover);
            let rails: Vec<_> = p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y > 0.5).collect();
            assert_eq!(rails.len(), 2, "two rails");
            for (c, s, _) in rails {
                assert!((c.y + s.y * 0.5 - RAIL_H).abs() < EPS && (c.y - s.y * 0.5).abs() < EPS, "rail runs 0..RAIL_H");
            }
            assert_eq!(p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y < 0.1).count(), 2, "two rungs");
        }
    }

    #[test]
    fn ladder_rails_side_is_opposite_the_step_cell() {
        for dir in [-1, 1] {
            let s = ladder(dir, Cover::Manhole);
            let (m, step) = (s.rails_side(), s.step_cell());
            assert_eq!((m.0, m.1), (-(step.x - LADDER_CELL.0), -(step.y - LADDER_CELL.1)), "dir {dir}");
        }
        // the geometry follows it: the rails sit on the rails side of the cell centre
        let m = ladder(-1, Cover::Manhole).rails_side();
        let p = ladder_parts(-1, Cover::Manhole);
        let rail = p.boxes.iter().find(|(_, s, col)| *col == IRON && s.y > 0.5).unwrap();
        let off = ((rail.0.x - (LADDER_CELL.0 as f32 + 0.5)) * m.0 as f32) + ((rail.0.z - (LADDER_CELL.1 as f32 + 0.5)) * m.1 as f32);
        assert!(off > 0.3, "rails are against the rails side: {off}");
        // and the coaming leaves the step cell side open
        let step = ladder(-1, Cover::Manhole).step_cell();
        let sd = ((step.x - LADDER_CELL.0) as f32, (step.y - LADDER_CELL.1) as f32);
        let coaming: Vec<_> = p.boxes.iter().filter(|(_, _, col)| *col == COAMING).collect();
        assert_eq!(coaming.len(), 3);
        assert!(coaming.iter().all(|(c, s, _)| {
            assert!((s.y - COAMING_H).abs() < EPS);
            (c.x - (LADDER_CELL.0 as f32 + 0.5)) * sd.0 + (c.z - (LADDER_CELL.1 as f32 + 0.5)) * sd.1 <= 0.01
        }));
    }

    /// Share of cell `LADDER_CELL` covered, seen from above, by the cover's boxes and lit strips.
    fn cover_share(p: &Parts, cover: Cover) -> f32 {
        let iron = if cover == Cover::Grate { GRATE } else { COVER };
        let pieces: Vec<_> = p.boxes.iter().filter(|(_, _, col)| *col == iron).chain(p.glow.iter()).collect();
        let n = 100;
        let mut hit = 0;
        for i in 0..n {
            for j in 0..n {
                let (x, z) = (LADDER_CELL.0 as f32 + (i as f32 + 0.5) / n as f32, LADDER_CELL.1 as f32 + (j as f32 + 0.5) / n as f32);
                if pieces.iter().any(|(c, s, _)| (x - c.x).abs() <= s.x * 0.5 && (z - c.z).abs() <= s.z * 0.5) {
                    hit += 1;
                }
            }
        }
        hit as f32 / (n * n) as f32
    }

    #[test]
    fn ladder_cover_sits_on_the_rim_and_leaves_the_hole_open() {
        for cover in [Cover::Manhole, Cover::Grate] {
            let p = ladder_parts(-1, cover);
            let iron = if cover == Cover::Grate { GRATE } else { COVER };
            let lid: Vec<_> = p.boxes.iter().filter(|(_, _, col)| *col == iron).collect();
            assert!(lid.len() >= 4, "{cover:?}: the cover is a model of several boxes");
            // it stands up from the floor, propped (about 75 degrees), not lying flat over the hole
            let top = lid.iter().map(|(c, s, _)| c.y + s.y * 0.5).fold(0.0f32, f32::max);
            assert!(top > 0.5, "{cover:?}: propped open ({top})");
            // its foot is on the rails-side rim
            let m = ladder(-1, cover).rails_side();
            let rim = lid.iter().map(|(c, _, _)| (c.x - (LADDER_CELL.0 as f32 + 0.5)) * m.0 as f32 + (c.z - (LADDER_CELL.1 as f32 + 0.5)) * m.1 as f32).fold(f32::MAX, f32::min);
            assert!(rim >= 0.45, "{cover:?}: starts on the rim ({rim})");
            let share = cover_share(&p, cover);
            assert!(share <= 0.4, "{cover:?}: covers {share} of the cell");
        }
        assert_eq!(ladder_parts(-1, Cover::Grate).glow.len(), 4, "the grate keeps its lit rim");
        assert!(ladder_parts(-1, Cover::Manhole).glow.is_empty());
    }

    /// The hatch drawn with its cover in `lid`'s pose.
    fn hatch_parts(cover: Cover, lid: Lid) -> Parts {
        let (well, solid) = (|_: i32, _: i32| false, |_: i32, _: i32| false);
        let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well, solid: &solid };
        let mut p = Parts::default();
        ladder_down_with_lid(&mut p, LADDER_CELL, ladder(-1, cover), &site, lid);
        p
    }

    #[test]
    fn a_closed_cover_lies_flat_over_the_hole() {
        for cover in [Cover::Manhole, Cover::Grate] {
            let p = hatch_parts(cover, Lid::Closed);
            let iron = if cover == Cover::Grate { GRATE } else { COVER };
            let lid: Vec<_> = p.boxes.iter().filter(|(_, _, col)| *col == iron).collect();
            assert!(!lid.is_empty(), "{cover:?}: the closed cover is drawn");
            let top = lid.iter().map(|(c, s, _)| c.y + s.y * 0.5).fold(0.0f32, f32::max);
            assert!((top - LID_T).abs() < EPS, "{cover:?}: lies flat on the floor ({top})");
            let lowest = lid.iter().map(|(c, s, _)| c.y - s.y * 0.5).fold(f32::MAX, f32::min);
            assert!(lowest >= -EPS, "{cover:?}: nothing below the floor ({lowest})");
        }
        // the manhole's plate covers the cell, so the hole is closed from above and below
        assert!(cover_share(&hatch_parts(Cover::Manhole, Lid::Closed), Cover::Manhole) > 0.6);
        // and the grate is still a grate: bars, not a solid plate
        assert!(cover_share(&hatch_parts(Cover::Grate, Lid::Closed), Cover::Grate) < 0.6);
    }

    #[test]
    fn the_default_hatch_is_propped_open_and_the_closed_pose_is_another_pose() {
        for cover in [Cover::Manhole, Cover::Grate] {
            let default = ladder_parts(-1, cover);
            let propped = hatch_parts(cover, Lid::Propped);
            assert_eq!(default.boxes.len(), propped.boxes.len(), "{cover:?}: ladder_down draws the propped cover");
            let closed = hatch_parts(cover, Lid::Closed);
            let rim_top = |p: &Parts| {
                let iron = if cover == Cover::Grate { GRATE } else { COVER };
                p.boxes.iter().filter(|(_, _, col)| *col == iron).map(|(c, s, _)| c.y + s.y * 0.5).fold(0.0f32, f32::max)
            };
            assert!(rim_top(&propped) > rim_top(&closed) + 0.3, "{cover:?}: propped stands up, closed lies down");
            // the shaft and rails stand the same either way
            let iron_rails = |p: &Parts| p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y > 0.5).count();
            assert_eq!(iron_rails(&propped), iron_rails(&closed), "{cover:?}: the rails do not change with the lid");
        }
    }

    #[test]
    fn ladder_rails_span_floor_to_the_floor_above_on_the_lower_level() {
        // under a capped ceiling (the real one, storey less slab) and under a 4 m one: the rails and the plate still
        // run the whole shaft, through the slab gap, so no stretch of the shaft is bare
        for wall_h in [H, crate::render::seams::wall_top(H), 4.0] {
            let p = ladder_parts_at(1, Cover::Manhole, wall_h);
            let rails: Vec<_> = p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y > 1.0).collect();
            assert_eq!(rails.len(), 2, "wall_h {wall_h}");
            for (c, s, _) in rails {
                assert!((c.y - s.y * 0.5).abs() < EPS && (c.y + s.y * 0.5 - H).abs() < EPS, "wall_h {wall_h}: rail top");
            }
            let plate = p.boxes.iter().find(|(_, _, col)| *col == BACK_PLATE).expect("a back plate");
            assert!((plate.1.y - H).abs() < EPS, "wall_h {wall_h}: plate height");
        }
        let p = ladder_parts(1, Cover::Manhole);
        let rails: Vec<_> = p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y > 1.0).collect();
        assert_eq!(rails.len(), 2);
        for (c, s, _) in rails {
            assert!((c.y - s.y * 0.5).abs() < EPS && (c.y + s.y * 0.5 - H).abs() < EPS);
        }
        let plate = p.boxes.iter().find(|(_, _, col)| *col == BACK_PLATE).expect("a back plate");
        assert!((plate.1.y - H).abs() < EPS && (plate.1.x.min(plate.1.z) - BACK_PLATE_T).abs() < EPS);
    }

    #[test]
    fn a_low_ceiling_still_runs_the_ladder_to_the_floor_above_with_one_back_plate() {
        // the sewer: a 4 m ceiling under a 5 m storey. The rails and the plate climb the whole shaft; the shaft's other
        // sides are the world mesh's wall columns and slab rims (world_mesh::edge_fill, tested there), so no second
        // box is drawn on the plane those occupy
        let (well, solid) = (|_: i32, _: i32| false, |_: i32, _: i32| false);
        for wall_h in [4.0, H] {
            let site = Site { h: H, wall_h, tiers: 1.0, well: &well, solid: &solid };
            assert_eq!(site.shaft_top(), H);
            let mut p = Parts::default();
            ladder_up(&mut p, LADDER_CELL, ladder(1, Cover::Manhole), &site);
            for (c, s, _) in p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y > 1.0) {
                assert!((c.y - s.y * 0.5).abs() < EPS && (c.y + s.y * 0.5 - H).abs() < EPS, "rails climb to the floor above");
            }
            assert_eq!(p.boxes.iter().filter(|(_, _, col)| *col == BACK_PLATE).count(), 1, "wall_h {wall_h}");
        }
    }

    #[test]
    fn ladder_up_emits_no_plate_or_grate() {
        let p = ladder_parts(1, Cover::Grate);
        assert!(p.quads.is_empty() && p.glow.is_empty(), "no hatch plate, no frame");
        assert!(p.boxes.iter().all(|(_, _, col)| *col != GRATE), "no grate bars");
        assert!(p.boxes.iter().all(|(c, s, _)| c.y + s.y * 0.5 <= H + EPS), "nothing above the ceiling line, no cap");
    }
}
