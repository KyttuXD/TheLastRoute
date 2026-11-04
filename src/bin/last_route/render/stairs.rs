//! Stair flights and ladder wells as geometry: a flight of square tiles quantised from the flight's surface going down
//! into a lined pit (or up into a shaft through the ceiling), and a ladder in its shaft. The pieces come out as flat
//! quads and boxes that `world_mesh` files under the level's floor and wall pictures, so the pit carries the wall's
//! picture. The flight's footprint, surface and walk code live in `world::facility` / `world::flight`: this is only
//! what is drawn.

use super::shader::Lamp;
use crate::world::facility::{Cover, LinkKind, Stair};
use crate::world::flight::{flights, max_riser_of};
use crate::world::vertical::{STEPS_PER_CELL, STOREY};
use macroquad::prelude::*;

/// How far the shaft over a flight up rises past the ceiling before its cap.
pub const SHAFT_ABOVE: f32 = 1.6;

/// Which of the level's pictures a piece is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// The floor picture of the cell (a shaft's bottom).
    Floor,
    /// The wall picture of the cell (pit and shaft walls).
    Wall,
    /// Flat colour (treads, risers, the shaft cap).
    Plain,
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
    /// Boxes (centre, size, colour) for rails, rungs and grate bars, drawn in plain colour.
    pub boxes: Vec<(Vec3, Vec3, Color)>,
    /// Boxes drawn unlit (the nosing of each step), so every step's edge shows even where the light is thin.
    pub glow: Vec<(Vec3, Vec3, Color)>,
}

/// One square tile of a flight: its corner, its side (1/STEPS_PER_CELL m) and its quantised top, relative to the
/// floor of the level that holds the flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub x0: f32,
    pub z0: f32,
    pub size: f32,
    pub top: f32,
}

/// The tile side in metres.
fn tile_size() -> f32 {
    1.0 / STEPS_PER_CELL as f32
}

/// The riser grid: the shape's largest riser. A neighbouring tile's surface differs by at most that, so the rounded
/// tops differ by 0 or one grid step and a riser never exceeds it (a finer grid would round to two steps at times).
fn riser_grid(s: Stair) -> f32 {
    max_riser_of(flights(), s.shape)
}

/// `h` rounded to a multiple of `q` measured from the flight's lower floor `lo`, kept within `[lo, hi]`: the foot
/// tile sits exactly on the lower floor and no tread dips below it or rises past the upper floor.
fn quantise(h: f32, q: f32, lo: f32, hi: f32) -> f32 {
    // the nudge settles exact halves (a straight flight's tiles sit on them) the same way every time, so float noise never
    // rounds one tile down and its neighbour up by two grid steps
    (lo + (((h - lo) / q) + 1e-3).round() * q).clamp(lo, hi)
}

/// The flight's lower and upper floor relative to the level that holds it: a flight down (dir < 0) is held by its
/// upper level and runs from -STOREY to 0; a flight up is held by its lower level and runs from 0 to STOREY.
fn span_rel(s: Stair) -> (f32, f32) {
    let lo = if s.dir < 0 { -STOREY } else { 0.0 };
    (lo, lo + STOREY)
}

/// The raw surface height for the global tile (`ix`, `iz`): at its centre, or where the walkable band only clips the
/// tile, at the first of its four quarter points that has a surface (so no walkable point is left without a tile).
fn tile_surface(s: Stair, level: i32, ix: i32, iz: i32) -> Option<f32> {
    let size = tile_size();
    let (cx, cz) = ((ix as f32 + 0.5) * size, (iz as f32 + 0.5) * size);
    let q = size * 0.25;
    [(0.0, 0.0), (-q, -q), (q, -q), (-q, q), (q, q)].into_iter().find_map(|(dx, dz)| s.surface_rel(level, cx + dx, cz + dz))
}

/// The quantised top of the global tile (`ix`, `iz`) (tile coordinates, `STEPS_PER_CELL` a cell), or None where the
/// flight has no surface (open hole, or outside the flight).
fn tile_top(s: Stair, level: i32, ix: i32, iz: i32) -> Option<f32> {
    let (lo, hi) = span_rel(s);
    tile_surface(s, level, ix, iz).map(|h| quantise(h, riser_grid(s), lo, hi))
}

/// The tiles of well cell `c` of the flight `s` as held by `level`: one per 1/3 m square with a surface.
pub fn tiles(s: Stair, level: i32, c: (i32, i32)) -> Vec<Tile> {
    let per = STEPS_PER_CELL;
    let size = tile_size();
    let mut out = Vec::new();
    for i in 0..per {
        for j in 0..per {
            let (ix, iz) = (c.0 * per + i, c.1 * per + j);
            if let Some(top) = tile_top(s, level, ix, iz) {
                out.push(Tile { x0: ix as f32 * size, z0: iz as f32 * size, size, top });
            }
        }
    }
    out
}

/// What a cell's flight needs to know about its surroundings and ceiling.
pub struct Site<'a> {
    /// The storey the flight crosses (the level below's floor is `-h`).
    pub h: f32,
    /// This cell's ceiling height.
    pub wall_h: f32,
    /// How many times the wall picture repeats floor to ceiling.
    pub tiers: f32,
    /// True for a cell of a stairwell (a flight's own cells).
    pub well: &'a dyn Fn(i32, i32) -> bool,
    /// True for a wall cell.
    pub solid: &'a dyn Fn(i32, i32) -> bool,
}

impl Site<'_> {
    /// The top of a shaft's lining: the storey above's floor, or this cell's ceiling when that is higher. A region whose
    /// ceiling is lower than the storey (the sewer's 4 m under a 5 m storey) still has its shafts lined up to the
    /// floor above, so no void shows between the ceiling and that floor.
    pub fn shaft_top(&self) -> f32 {
        self.h.max(self.wall_h)
    }
}

/// The ladder's rails and rungs: a light iron, so they read against the dark shaft from standing height.
const IRON: Color = Color::new(0.82, 0.78, 0.72, 1.0);
/// The risers: warm concrete, a shade under the treads' stone, so each step's face reads apart from its tread.
const RISER: Color = Color::new(0.56, 0.5, 0.45, 1.0);
/// A grate's bars and a hatch's frame: a mid grey iron that catches the light (the old near-black barely showed).
const GRATE: Color = Color::new(0.5, 0.49, 0.47, 1.0);
/// The lit edge of a grate cover, a dim warm rim so the cover's outline reads down the shaft.
const RIM: Color = Color::new(0.95, 0.62, 0.3, 1.0);
/// A hatch's cover: a lit plate of pale grey-cream, unlit, so the cover never reads as a black hole in the ceiling
/// (the floor picture painted over the hatch came out black from below).
pub const HATCH: Color = Color::new(0.62, 0.6, 0.56, 1.0);
const DAYLIGHT: Color = Color::new(0.75, 0.9, 1.0, 1.0);

/// The nosing at each step's lower edge: a pale cream strip, unlit, so the edge of every step shows.
const NOSING: Color = Color::new(1.0, 0.93, 0.7, 1.0);

/// The treads' stone: a flat pale grey-cream, drawn in plain colour rather than over the floor picture. The sewer's
/// floor is orange-brown, and a picture-painted tread blended into it; a light flat stone reads apart from any floor.
pub const TREAD_STONE: Color = Color::new(0.74, 0.72, 0.68, 1.0);

/// Steps and pit walls fade with depth, but never to black: the lowest tread is still 75 % of the picture.
fn shade(y: f32, h: f32) -> Color {
    let f = if y < 0.0 { 1.0 - 0.25 * (-y / h).min(1.0) } else { 1.0 - 0.2 * (y / h).min(1.0) };
    Color::new(f, f, f, 1.0)
}

/// A tread's colour: its depth shade times the stone.
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

/// A vertical face from height `lo` to `hi` over `a`..`b` across, painted with the wall picture.
fn wall_face(out: &mut Parts, s: &Site, a: (Vec3, f32), b: (Vec3, f32), lo: f32, hi: f32, n: Vec3, col: Color) {
    vface(out, Face::Wall, s, a, b, lo, hi, n, col);
}

/// A vertical face painted with `face`'s picture: the wall picture runs floor to ceiling, the floor picture is laid
/// by height (a metre a tile), so a pit lined with it reads as stone and not as a panelled wall.
fn vface(out: &mut Parts, face: Face, s: &Site, a: (Vec3, f32), b: (Vec3, f32), lo: f32, hi: f32, n: Vec3, col: Color) {
    let (p, u0) = a;
    let (q, u1) = b;
    let up = |v: Vec3, y: f32| vec3(v.x, y, v.z);
    let v = |y: f32| if face == Face::Wall { wall_v(y, s) } else { -y };
    push(
        out,
        face,
        [up(p, hi), up(q, hi), up(q, lo), up(p, lo)],
        [vec2(u0, v(hi)), vec2(u1, v(hi)), vec2(u1, v(lo)), vec2(u0, v(lo))],
        n,
        col,
    );
}

/// A face's colour dimmed to `f` of itself (risers and pit ends sit darker than the treads, so the two read apart).
fn dim(c: Color, f: f32) -> Color {
    Color::new(c.r * f, c.g * f, c.b * f, 1.0)
}

/// The two ground points (and their wall-picture u) of a vertical edge of length `len` centred on `mid`, whose face
/// looks along horizontal `n`. The order p -> q makes the quad's winding match `n` (see `vface`).
fn edge_ends(mid: (f32, f32), len: f32, n: (i32, i32)) -> ((Vec3, f32), (Vec3, f32)) {
    let e = (-(n.1 as f32) * len * 0.5, n.0 as f32 * len * 0.5);
    let p = vec3(mid.0 - e.0, 0.0, mid.1 - e.1);
    let q = vec3(mid.0 + e.0, 0.0, mid.1 + e.1);
    ((p, p.x + p.z), (q, q.x + q.z))
}

/// A vertical face over one edge (see `edge_ends`) from `lo` to `hi`, looking along `n`.
fn edge_face(out: &mut Parts, face: Face, site: &Site, mid: (f32, f32), len: f32, n: (i32, i32), lo: f32, hi: f32, col: Color) {
    let (a, b) = edge_ends(mid, len, n);
    vface(out, face, site, a, b, lo, hi, vec3(n.0 as f32, 0.0, n.1 as f32), col);
}

const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// Adds cell `c` (one of `s.well_cells()`) of the flight `s` as held by `level`: its tiles (flat tops, risers where a
/// neighbour tile is lower, a nosing on those edges) and the side walls at open block edges, for the flight's owner
/// (the lower level, dir > 0), with no shaft cap or ceiling (the well is open above) and the divider wall once. The
/// upper level (dir < 0) draws no tiles, lining or floor: only the rim railing.
pub fn flight_cell(out: &mut Parts, c: (i32, i32), s: Stair, level: i32, site: &Site) {
    if s.dir < 0 {
        // the upper level: the well is open (no floor, no tiles, no lining); only a railing round its rim
        rim_rails(out, c, s, site);
        return;
    }
    let per = STEPS_PER_CELL;
    let size = tile_size();
    let eps = 1e-5;
    for t in tiles(s, level, c) {
        let (ix, iz) = ((t.x0 / size).round() as i32, (t.z0 / size).round() as i32);
        let col = shade(t.top, site.h);
        let top = [vec3(t.x0, t.top, t.z0), vec3(t.x0, t.top, t.z0 + size), vec3(t.x0 + size, t.top, t.z0 + size), vec3(t.x0 + size, t.top, t.z0)];
        push(out, Face::Plain, top, [Vec2::ZERO; 4], vec3(0., 1., 0.), tread_tint(col));
        let centre = (t.x0 + size * 0.5, t.z0 + size * 0.5);
        for d in DIRS {
            let (nix, niz) = (ix + d.0, iz + d.1);
            let nc = (nix.div_euclid(per), niz.div_euclid(per));
            let mid = (centre.0 + d.0 as f32 * size * 0.5, centre.1 + d.1 as f32 * size * 0.5);
            if nc != c && !(site.well)(nc.0, nc.1) {
                // the edge of the whole block
                if !(site.solid)(nc.0, nc.1) && t.top > eps {
                    // the side of the stair block where nothing else closes it, facing out of the block (up to the tread, even through a ceiling lower than the storey: the shaft is lined to the floor above)
                    edge_face(out, Face::Wall, site, mid, size, d, 0.0, t.top.min(site.shaft_top()), col);
                }
                continue;
            }
            // a lower tile of the same flight: a riser down to it and a nosing on this tile's edge
            if let Some(nt) = tile_top(s, level, nix, niz) {
                if nt < t.top - eps {
                    edge_face(out, Face::Plain, site, mid, size, d, nt, t.top, dim(RISER, shade(nt, site.h).r));
                    let size3 = vec3(if d.0 != 0 { 0.08 } else { size * 0.96 }, 0.05, if d.1 != 0 { 0.08 } else { size * 0.96 });
                    let at = vec3(centre.0 + d.0 as f32 * (size * 0.5 - 0.05), t.top + 0.01, centre.1 + d.1 as f32 * (size * 0.5 - 0.05));
                    out.glow.push((at, size3, NOSING));
                }
            }
        }
    }
    // no shaft cap and no ceiling: the well is open to the level above. The divider wall goes in once per stair.
    if s.well_cells().first().is_some_and(|f| (f.x, f.y) == c) {
        divider_wall(out, s, level, site);
    }
}

/// Height of the railing round an open well rim.
pub const RAIL_H: f32 = 1.05;

/// The railing of a well cell of a flight down (held by the upper level): posts, a top rail and a mid rail along every
/// edge that borders a non-well, non-solid cell, except the edge facing the landing cell, which stays open. Built of
/// boxes, so both sides of every piece show.
fn rim_rails(out: &mut Parts, c: (i32, i32), s: Stair, site: &Site) {
    let landing = s.landing();
    let col = tread_tint(shade(0.0, site.h));
    let mid_c = (c.0 as f32 + 0.5, c.1 as f32 + 0.5);
    for d in DIRS {
        let nc = (c.0 + d.0, c.1 + d.1);
        if (site.well)(nc.0, nc.1) || (site.solid)(nc.0, nc.1) || (landing.x, landing.y) == nc {
            continue;
        }
        let inset = 0.06;
        let at = (mid_c.0 + d.0 as f32 * (0.5 - inset), mid_c.1 + d.1 as f32 * (0.5 - inset));
        // along the edge: x for a north/south edge, z for an east/west one
        let along = vec3(d.1.abs() as f32, 0.0, d.0.abs() as f32);
        let thin = vec3(d.0.abs() as f32, 0.0, d.1.abs() as f32);
        let bar = |len: f32, t: f32, h: f32| along * len + thin * t + vec3(0.0, h, 0.0);
        out.boxes.push((vec3(at.0, RAIL_H, at.1), bar(1.0, 0.06, 0.05), col));
        out.boxes.push((vec3(at.0, RAIL_H * 0.5, at.1), bar(1.0, 0.04, 0.04), col));
        for k in [-0.47f32, 0.0, 0.47] {
            let p = vec3(at.0 + along.x * k, RAIL_H * 0.5, at.1 + along.z * k);
            out.boxes.push((p, bar(0.06, 0.06, RAIL_H), col));
        }
    }
}

/// The thin wall between a switchback's two lanes: along every `s.dividers(level)` segment from the floor to the
/// higher of the two lane surfaces there plus the rail height, clipped to the ceiling. Straight and spiral flights
/// have no divider and add nothing. Drawn as boxes (so both faces show), a box per tile-length run.
pub fn divider_wall(out: &mut Parts, s: Stair, level: i32, site: &Site) {
    let t = flights().divider_thickness;
    for seg in s.dividers(level) {
        let (dx, dz) = (seg.b.0 - seg.a.0, seg.b.1 - seg.a.1);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1e-4 {
            continue;
        }
        let (ux, uz) = (dx / len, dz / len);
        let (nx, nz) = (-uz, ux);
        let step = tile_size();
        let n = (len / step).ceil().max(1.0) as i32;
        let run = len / n as f32;
        for i in 0..n {
            let m = (i as f32 + 0.5) * run;
            let (px, pz) = (seg.a.0 + ux * m, seg.a.1 + uz * m);
            let side = |k: f32| s.surface_rel(level, px + nx * k, pz + nz * k);
            let top = [side(0.0), side(0.2), side(-0.2)].into_iter().flatten().fold(0.0f32, f32::max);
            let hi = (top + RAIL_H).min(site.shaft_top());
            if hi <= 1e-4 {
                continue;
            }
            // thickness across the segment, a hair of overlap along it so the runs join
            let size = vec3((ux.abs() * (run + 0.002)).max(t * nx.abs()), hi, (uz.abs() * (run + 0.002)).max(t * nz.abs()));
            out.boxes.push((vec3(px, hi * 0.5, pz), size, tread_tint(shade(top, site.h))));
        }
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

/// The hatch of a ladder, seen from the upper level (`Stair::is_ladder`, `dir < 0`): a hole in the floor. There is no
/// pit, no shaft wall and no pit floor (the lower level is the floor you see through it). Round it: a low coaming
/// on the three sides that are not the step cell's side, two rails and two rungs standing `RAIL_H` above the floor on
/// the rails side, and the cover as a model propped open on that side's rim. A grate adds its lit rim strips.
pub fn ladder_down(out: &mut Parts, c: (i32, i32), s: Stair, site: &Site) {
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
    propped_cover(out, c, m, grate);
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

/// A ladder's foot, seen from the lower level (`dir > 0`): the rails and rungs on the rails side from the floor to the
/// ceiling (`site.wall_h`) and a thin back plate behind them, so the shaft reads as a fixture. No hatch plate, no grate
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
    // a ceiling lower than the storey leaves a slab between it and the floor above: line the shaft on all four sides
    // through that slab (Godot lines the ladder shaft from the maze floor down to the sewer ceiling)
    let (lo, hi) = (site.wall_h, top);
    if hi - lo > 1e-4 {
        for d in DIRS {
            let along = vec3(d.1.abs() as f32, 0.0, d.0.abs() as f32);
            let thin = vec3(d.0.abs() as f32, 0.0, d.1.abs() as f32);
            let at = vec3(
                c.0 as f32 + 0.5 + d.0 as f32 * (0.5 - BACK_PLATE_T * 0.5),
                (lo + hi) * 0.5,
                c.1 as f32 + 0.5 + d.1 as f32 * (0.5 - BACK_PLATE_T * 0.5),
            );
            out.boxes.push((at, along * 1.0 + thin * BACK_PLATE_T + vec3(0.0, hi - lo, 0.0), BACK_PLATE));
        }
    }
}

/// The warm light of a stairwell (a cool shaft of daylight at a ladder's sewer end), so its steps read in the dark.
/// Only the dir > 0 stair (the owner of the flight or shaft) should have its lamp pushed; callers (world_mesh,
/// world_stack) filter out dir < 0 stairs, whose lamp is not wanted: the upper level's hatch has no shaft to light, the
/// lower level's lamp does that. A dir < 0 ladder still returns a harmless lamp at the hatch so the type stays Lamp.
pub fn lamp(s: Stair, h: f32) -> Lamp {
    let (cx, cz) = s.cell.center();
    if s.is_ladder() {
        return if s.dir < 0 {
            // never pushed; a dim placeholder just over the hatch
            Lamp { pos: vec3(cx, 1.2, cz), range: 4.0, color: Color::new(1.0, 0.72, 0.42, 1.0), energy: 1.0 }
        } else {
            Lamp { pos: vec3(cx, h * 0.45, cz), range: 7.0, color: Color::new(0.8, 0.88, 1.0, 1.0), energy: 1.7 }
        };
    }
    // over the middle of the footprint: the middle of the well cell nearest the cells' centroid
    let cells = s.well_cells();
    let n = cells.len().max(1) as f32;
    let mean = cells.iter().fold((0.0f32, 0.0f32), |a, c| (a.0 + c.x as f32 + 0.5, a.1 + c.y as f32 + 0.5));
    let mean = (mean.0 / n, mean.1 / n);
    let (x, z) = cells
        .iter()
        .map(|c| (c.x as f32 + 0.5, c.y as f32 + 0.5))
        .min_by(|a, b| {
            let d = |p: &(f32, f32)| (p.0 - mean.0).powi(2) + (p.1 - mean.1).powi(2);
            d(a).partial_cmp(&d(b)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or((cx, cz));
    let y = if s.dir < 0 { 0.3 } else { h * 0.5 };
    Lamp { pos: vec3(x, y, z), range: 13.0, color: Color::new(1.0, 0.74, 0.45, 1.0), energy: 5.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::facility::{Cell, LinkKind};
    use crate::world::flight::FlightShape;
    use crate::world::vertical::STOREY;
    use std::collections::{HashMap, HashSet};

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
        let well = well_set(s);
        let (well_fn, solid_fn) = (|x, z| well.contains(&(x, z)), |_, _| false);
        let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well_fn, solid: &solid_fn };
        let mut out = Parts::default();
        for c in s.well_cells() {
            flight_cell(&mut out, (c.x, c.y), s, level_of(s), &site);
        }
        out
    }

    /// Every tile of the flight keyed by its global tile coordinates.
    fn tile_map(s: Stair) -> HashMap<(i32, i32), Tile> {
        let mut m = HashMap::new();
        for c in s.well_cells() {
            for t in tiles(s, level_of(s), (c.x, c.y)) {
                m.insert(((t.x0 / t.size).round() as i32, (t.z0 / t.size).round() as i32), t);
            }
        }
        m
    }

    fn tread_quads(p: &Parts) -> Vec<&Quad> {
        p.quads.iter().filter(|q| q.face == Face::Plain && q.n == vec3(0., 1., 0.)).collect()
    }

    fn risers(p: &Parts) -> Vec<&Quad> {
        p.quads.iter().filter(|q| q.face == Face::Plain && q.n.y == 0.0).collect()
    }

    #[test]
    fn flight_tiles_follow_the_surface() {
        for shape in SHAPES {
            let q = riser_grid(stair(-1, (0, -1), shape));
            for run in RUNS {
                for dir in [-1, 1] {
                    for level in [0, -1] {
                        let s = stair(dir, run, shape);
                        let mut count = 0;
                        for c in s.well_cells() {
                            for t in tiles(s, level, (c.x, c.y)) {
                                // a tile exists where the surface does: at its centre, or (where the band only clips it) a quarter point
                                let (cx, cz) = (t.x0 + t.size * 0.5, t.z0 + t.size * 0.5);
                                let surface = s.surface_rel(level, cx, cz).or_else(|| s.surface_rel(level, cx - t.size * 0.25, cz - t.size * 0.25)).or_else(|| s.surface_rel(level, cx + t.size * 0.25, cz - t.size * 0.25)).or_else(|| s.surface_rel(level, cx - t.size * 0.25, cz + t.size * 0.25)).or_else(|| s.surface_rel(level, cx + t.size * 0.25, cz + t.size * 0.25)).expect("a tile only exists where the surface does");
                                assert!((t.top - surface).abs() <= q / 2.0 + EPS, "{shape:?} {run:?} {dir} {level}: top {} surface {surface}", t.top);
                                assert!((t.size - 1.0 / STEPS_PER_CELL as f32).abs() < 1e-6);
                                count += 1;
                            }
                        }
                        // a level that does not hold this flight has no surface and draws nothing; the holding level has tiles
                        if level == level_of(s) {
                            assert!(count > 0, "{shape:?} {run:?} {dir}: the holding level has tiles");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn no_riser_exceeds_the_max_riser() {
        for shape in SHAPES {
            let max = max_riser_of(flights(), shape);
            for run in RUNS {
                for dir in [-1, 1] {
                    let s = stair(dir, run, shape);
                    let m = tile_map(s);
                    for (&(ix, iz), t) in &m {
                        for d in DIRS {
                            if let Some(n) = m.get(&(ix + d.0, iz + d.1)) {
                                // A switchback's two lanes lie side by side and climb opposite ways, so across the lane
                                // border the surface differs by up to a whole storey: that border is the divider wall
                                // (drawn as a riser face), not a step. A pair whose true surfaces already differ by more
                                // than a riser is such a divider; every other pair, on every shape, must keep the limit.
                                let raw = |a: i32, b: i32| tile_surface(s, level_of(s), a, b).unwrap();
                                if shape == FlightShape::Switchback && (raw(ix, iz) - raw(ix + d.0, iz + d.1)).abs() > max {
                                    continue;
                                }
                                assert!((t.top - n.top).abs() <= max + EPS, "{shape:?} {run:?} {dir}: step {} -> {} at tile ({ix}, {iz}) towards {d:?}, raw {:?} -> {:?}", t.top, n.top, tile_surface(s, level_of(s), ix, iz), tile_surface(s, level_of(s), ix + d.0, iz + d.1));
                            }
                        }
                    }
                    let hi = m.values().map(|t| t.top).fold(f32::MIN, f32::max);
                    let lo = m.values().map(|t| t.top).fold(f32::MAX, f32::min);
                    assert!(((hi - lo) - STOREY).abs() <= 0.5, "{shape:?} {run:?} {dir}: the flight spans {} of {STOREY}", hi - lo);
                }
            }
        }
    }

    #[test]
    fn every_surface_point_falls_on_a_tile() {
        for shape in SHAPES {
            for run in RUNS {
                for dir in [-1, 1] {
                    let s = stair(dir, run, shape);
                    let level = level_of(s);
                    for c in s.well_cells() {
                        let ts = tiles(s, level, (c.x, c.y));
                        for i in 0..6 {
                            for j in 0..6 {
                                let (x, z) = (c.x as f32 + (i as f32 + 0.5) / 6.0, c.y as f32 + (j as f32 + 0.5) / 6.0);
                                if s.surface_rel(level, x, z).is_some() {
                                    assert!(
                                        ts.iter().any(|t| x >= t.x0 && x < t.x0 + t.size && z >= t.z0 && z < t.z0 + t.size),
                                        "{shape:?} {run:?} {dir}: ({x}, {z}) has a surface but no tile"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_mesh_has_a_tread_per_tile_and_stays_inside_the_storey() {
        // (heights come from the flight surface, checked by flight_tiles_follow_the_surface and
        // no_riser_exceeds_the_max_riser)
        for shape in SHAPES {
            for dir in [-1, 1] {
                for run in RUNS {
                    let s = stair(dir, run, shape);
                    let p = build(s);
                    if dir < 0 {
                        // the upper level owns no tiles: the well is open
                        assert!(tread_quads(&p).is_empty() && p.quads.is_empty(), "{shape:?} {dir} {run:?}");
                        continue;
                    }
                    assert_eq!(tread_quads(&p).len(), tile_map(s).len(), "{shape:?} {dir} {run:?}");
                    for q in &p.quads {
                        for v in q.c {
                            assert!(v.y >= -EPS && v.y <= H + EPS, "{shape:?} {dir} {run:?}: vertex at {}", v.y);
                        }
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
                // the steps (their stone tops) and the wall pictures (the shaft's cap is a ceiling, and may be dark)
                for q in p.quads.iter().filter(|q| q.face != Face::Plain || q.n.y > 0.5) {
                    assert!(q.col.r >= 0.5 && q.col.g >= 0.5 && q.col.b >= 0.5, "{dir}: {:?}", q.col);
                }
                // the plain risers are concrete, lit to a third of white at the worst
                for q in risers(&p) {
                    assert!(q.col.r >= 0.28 && q.col.g >= 0.25 && q.col.b >= 0.22, "{dir}: {:?}", q.col);
                }
            }
        }
    }

    #[test]
    fn treads_are_light_stone_and_their_nosings_are_cream() {
        // the stone is a light flat grey-cream, lit to at least two thirds of itself at the lowest tread
        assert!(TREAD_STONE.r >= 0.7 && TREAD_STONE.g >= 0.7 && TREAD_STONE.b >= 0.65, "{TREAD_STONE:?}");
        assert!(NOSING.r >= 0.95 && NOSING.g >= 0.85 && NOSING.b >= 0.6, "the nosing is cream: {NOSING:?}");
        for shape in SHAPES {
            for dir in [1] {
                let p = build(stair(dir, (0, -1), shape));
                for q in tread_quads(&p) {
                    assert!(q.col.r >= 0.45 && q.col.g >= 0.45 && q.col.b >= 0.4 && q.col.r <= 1.0, "{shape:?} {dir}: tread {:?}", q.col);
                }
                // one nosing per riser: every step edge is marked, and only those
                assert_eq!(p.glow.len(), risers(&p).len(), "{shape:?} {dir}");
                assert!(!p.glow.is_empty());
                assert!(p.glow.iter().all(|(_, s, col)| s.y >= 0.04 && *col == NOSING));
            }
        }
    }

    #[test]
    fn risers_face_the_lower_tile_and_join_two_heights() {
        for shape in SHAPES {
            for dir in [-1, 1] {
                for run in RUNS {
                    let p = build(stair(dir, run, shape));
                    let tiles = tile_map(stair(dir, run, shape));
                    for q in risers(&p) {
                        let (hi, lo) = (q.c[0].y, q.c[3].y);
                        assert!(hi > lo + EPS, "{shape:?} {dir} {run:?}: a riser has height");
                        // a point a hair in front of the riser sits on a tile no higher than the riser's foot
                        let m = (q.c[0] + q.c[1]) * 0.5;
                        let (fx, fz) = (m.x + q.n.x * 0.01, m.z + q.n.z * 0.01);
                        let (ix, iz) = ((fx * STEPS_PER_CELL as f32).floor() as i32, (fz * STEPS_PER_CELL as f32).floor() as i32);
                        let front = tiles.get(&(ix, iz)).unwrap_or_else(|| panic!("{shape:?} {dir} {run:?}: nothing in front of a riser"));
                        assert!((front.top - lo).abs() < EPS, "{shape:?} {dir} {run:?}: front tile {} vs riser foot {lo}", front.top);
                    }
                }
            }
        }
    }

    #[test]
    fn a_flight_down_has_no_pit_lining_or_floor() {
        for shape in SHAPES {
            for run in RUNS {
                let p = build(stair(-1, run, shape));
                assert!(p.quads.is_empty(), "{shape:?} {run:?}: the upper cell emits no quad (no floor, lining or tiles)");
                assert!(p.glow.is_empty(), "{shape:?} {run:?}: no nosings");
                assert!(!p.boxes.is_empty(), "{shape:?} {run:?}: only rails");
            }
        }
    }

    #[test]
    fn a_flight_up_leaves_no_ceiling_or_cap_over_its_cells() {
        for shape in SHAPES {
            for run in RUNS {
                let p = build(stair(1, run, shape));
                // no quad faces down (a ceiling or cap), and nothing reaches past the ceiling height
                assert!(p.quads.iter().all(|q| q.n.y >= 0.0), "{shape:?} {run:?}: a quad faces -Y");
                assert!(p.quads.iter().all(|q| q.c.iter().all(|v| v.y <= H + EPS)), "{shape:?} {run:?}: wall above the ceiling");
                assert!(p.boxes.iter().all(|(c, s, _)| c.y + s.y * 0.5 <= H + EPS));
            }
        }
    }

    fn rail_boxes(p: &Parts) -> &[(Vec3, Vec3, Color)] {
        &p.boxes
    }

    #[test]
    fn rails_stand_on_the_upper_floor_at_rail_height() {
        for shape in SHAPES {
            for run in RUNS {
                let p = build(stair(-1, run, shape));
                let top = rail_boxes(&p).iter().map(|(c, s, _)| c.y + s.y * 0.5).fold(0.0f32, f32::max);
                assert!((top - (RAIL_H + 0.025)).abs() < 0.05, "{shape:?} {run:?}: top rail at {top}");
                assert!(rail_boxes(&p).iter().all(|(c, s, _)| c.y - s.y * 0.5 >= -EPS), "{shape:?} {run:?}: nothing below the floor");
            }
        }
    }

    #[test]
    fn upper_rails_leave_the_landing_open() {
        for shape in SHAPES {
            for run in RUNS {
                let s = stair(-1, run, shape);
                let l = s.landing();
                let p = build(s);
                // no rail piece sits on the shared edge between a well cell and the landing cell
                for c in s.well_cells() {
                    let d = (l.x - c.x, l.y - c.y);
                    if d.0.abs() + d.1.abs() != 1 {
                        continue;
                    }
                    let edge = (c.x as f32 + 0.5 + d.0 as f32 * 0.44, c.y as f32 + 0.5 + d.1 as f32 * 0.44);
                    assert!(
                        !p.boxes.iter().any(|(b, sz, _)| (b.x - edge.0).abs() <= sz.x * 0.5 + 0.02 && (b.z - edge.1).abs() <= sz.z * 0.5 + 0.02),
                        "{shape:?} {run:?}: the landing edge of {c:?} is railed"
                    );
                }
            }
        }
    }

    #[test]
    fn divider_wall_runs_the_lane_divider_and_only_for_switchback() {
        for shape in SHAPES {
            for run in RUNS {
                let s = stair(1, run, shape);
                let p = build(s);
                let has_wall = !p.boxes.is_empty();
                let has_dividers = !s.dividers(level_of(s)).is_empty();
                assert_eq!(has_wall, has_dividers, "{shape:?} {run:?}");
                assert_eq!(has_dividers, shape == FlightShape::Switchback, "{shape:?} {run:?}: only a switchback has a divider");
                // every divider box stays within the ceiling and starts at the floor
                for (c, sz, _) in &p.boxes {
                    assert!(c.y - sz.y * 0.5 >= -EPS && c.y + sz.y * 0.5 <= H + EPS);
                }
            }
        }
    }

    #[test]
    fn the_stairwell_lamp_hangs_over_its_flight_and_the_ladder_lamp_is_the_lower_levels() {
        for shape in SHAPES {
            for dir in [-1, 1] {
                for run in RUNS {
                    let s = stair(dir, run, shape);
                    let l = lamp(s, H);
                    assert!(
                        s.well_cells().iter().any(|c| (c.x as f32..c.x as f32 + 1.0).contains(&l.pos.x) && (c.y as f32..c.y as f32 + 1.0).contains(&l.pos.z)),
                        "{shape:?} {dir} {run:?}: {:?}",
                        l.pos
                    );
                    assert!(l.range > 0.0 && l.energy > 0.0);
                }
            }
        }
        // the shaft foot (dir > 0) has its cool lamp in the shaft; the hatch (dir < 0) has no shaft to light
        let l = lamp(ladder(1, Cover::Manhole), H);
        assert!(l.pos.y > 0.0 && l.pos.y < H && l.energy > 0.0);
        assert!(lamp(ladder(-1, Cover::Manhole), H).pos.y >= 0.0, "nothing is lit below the upper floor any more");
    }

    const LADDER_CELL: (i32, i32) = (10, 10);

    fn ladder(dir: i32, cover: Cover) -> Stair {
        Stair { cell: Cell::new(LADDER_CELL.0, LADDER_CELL.1), dir, run: (1, 0), kind: LinkKind::Ladder(cover), shape: FlightShape::Switchback, clockwise: false }
    }

    fn ladder_parts(dir: i32, cover: Cover) -> Parts {
        let (well, solid) = (|_: i32, _: i32| false, |_: i32, _: i32| false);
        let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well, solid: &solid };
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

    #[test]
    fn ladder_rails_span_floor_to_wall_h_on_the_lower_level() {
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
    fn a_low_ceiling_still_lines_the_ladder_shaft_to_the_floor_above() {
        // the sewer: a 4 m ceiling under a 5 m storey
        let (well, solid) = (|_: i32, _: i32| false, |_: i32, _: i32| false);
        let site = Site { h: H, wall_h: 4.0, tiers: 1.0, well: &well, solid: &solid };
        assert_eq!(site.shaft_top(), H);
        let mut p = Parts::default();
        ladder_up(&mut p, LADDER_CELL, ladder(1, Cover::Manhole), &site);
        for (c, s, _) in p.boxes.iter().filter(|(_, s, col)| *col == IRON && s.y > 1.0) {
            assert!((c.y - s.y * 0.5).abs() < EPS && (c.y + s.y * 0.5 - H).abs() < EPS, "rails climb to the floor above");
        }
        // four lining boxes through the 1 m slab, none above the floor above, none under the ceiling
        let lining: Vec<_> = p.boxes.iter().filter(|(_, s, col)| *col == BACK_PLATE && (s.y - 1.0).abs() < EPS).collect();
        assert_eq!(lining.len(), 4);
        for (c, s, _) in lining {
            assert!((c.y - s.y * 0.5 - 4.0).abs() < EPS && (c.y + s.y * 0.5 - H).abs() < EPS);
        }
        // a ceiling as high as the storey needs no lining
        let tall = Site { h: H, wall_h: H, tiers: 1.0, well: &well, solid: &solid };
        let mut q = Parts::default();
        ladder_up(&mut q, LADDER_CELL, ladder(1, Cover::Manhole), &tall);
        assert_eq!(q.boxes.iter().filter(|(_, _, col)| *col == BACK_PLATE).count(), 1);
    }

    #[test]
    fn ladder_up_emits_no_plate_or_grate() {
        let p = ladder_parts(1, Cover::Grate);
        assert!(p.quads.is_empty() && p.glow.is_empty(), "no hatch plate, no frame");
        assert!(p.boxes.iter().all(|(_, _, col)| *col != GRATE), "no grate bars");
        assert!(p.boxes.iter().all(|(c, s, _)| c.y + s.y * 0.5 <= H + EPS), "nothing above the ceiling line, no cap");
    }
}
