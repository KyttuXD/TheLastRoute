//! Stair flights and ladder wells as geometry: a flight of treads and risers going down into a lined pit (or up
//! into a shaft through the ceiling), and a ladder in its shaft. The pieces come out as flat quads and boxes that
//! `world_mesh` files under the level's floor and wall pictures, so the steps carry the floor's texture and the pit
//! the wall's. The flight's cells and the walk and transition code are untouched: this is only what is drawn.

use super::shader::Lamp;
use crate::world::facility::{Cover, LinkKind, Stair, STAIR_FLIGHT};
use macroquad::prelude::*;

/// Treads per cell of the flight, so a cell's metre of run is three deep steps rather than one metre-high block.
pub const STEPS_PER_CELL: i32 = 3;

/// The flat treads at the foot of a flight down (its last cell): a landing to stand on, so the flight ends on a floor
/// and not against the pit's wall.
pub const LANDING: i32 = STEPS_PER_CELL;

/// How far a flight up rises per cell of run: about 33 degrees.
pub const RISE_UP: f32 = 0.65;

/// How far a flight down sinks per cell of run, over the cells before its landing: about 14 degrees. From a standing
/// eye the near lip of the pit hides a steep flight's treads (at 2 m back a 0.36 m cell hid the top steps), so the
/// flight is laid shallow enough that its treads still show past the lip; each riser is then 0.08 m, a visible step.
pub const RISE_DOWN: f32 = 0.24;

/// How far the shaft over a flight up rises past the ceiling before its cap.
pub const SHAFT_ABOVE: f32 = 1.6;

/// Which of the level's pictures a piece is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// The floor picture of the cell (treads, a shaft's bottom).
    Floor,
    /// The wall picture of the cell (risers, pit and shaft walls).
    Wall,
    /// Flat colour (the shaft cap).
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
    /// Boxes drawn unlit (the nosing of each tread), so every step's edge shows even where the light is thin.
    pub glow: Vec<(Vec3, Vec3, Color)>,
}

/// One tread: its top height and the run it covers, in metres from the landing cell's far edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tread {
    pub top: f32,
    pub from: f32,
    pub to: f32,
}

/// The treads of a whole flight, nearest the landing first: RISE_DOWN or RISE_UP per cell of run, down for `dir` -1
/// and up for 1. A flight down ends in LANDING flat treads.
pub fn treads(dir: i32) -> Vec<Tread> {
    let per = STEPS_PER_CELL as f32;
    let rise = if dir < 0 { RISE_DOWN } else { RISE_UP } / per;
    let n = STAIR_FLIGHT * STEPS_PER_CELL;
    let sloped = if dir < 0 { n - LANDING } else { n };
    (1..=n)
        .map(|j| Tread {
            top: dir.signum() as f32 * j.min(sloped) as f32 * rise,
            from: (j - 1) as f32 / per,
            to: j as f32 / per,
        })
        .collect()
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

const CEILING: Color = Color::new(0.18, 0.13, 0.17, 1.0);
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

/// The nosing at each tread's front edge: a pale cream strip, unlit, so the edge of every step shows.
const NOSING: Color = Color::new(1.0, 0.93, 0.7, 1.0);

/// The treads' stone: a flat pale grey-cream, drawn in plain colour rather than over the floor picture. The sewer's
/// floor is orange-brown, and a picture-painted tread blended into it; a light flat stone reads apart from any floor.
pub const TREAD_STONE: Color = Color::new(0.74, 0.72, 0.68, 1.0);

/// Steps and pit walls fade with depth, but never to black: the lowest tread is still 70 % of the picture.
fn shade(y: f32, h: f32) -> Color {
    let f = if y < 0.0 { 1.0 - 0.3 * (-y / h).min(1.0) } else { 1.0 - 0.2 * (y / h).min(1.0) };
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

/// Adds cell `c` (step `k` from the landing, 1..=STAIR_FLIGHT, one lane of the two) of the flight `s`: its treads and
/// risers, the pit lining beside and at the end of a flight down, or the stringers and the shaft through the ceiling
/// of a flight up. Faces next to the flight's own other lane are left out.
pub fn flight_cell(out: &mut Parts, c: (i32, i32), k: i32, s: Stair, site: &Site) {
    let run = s.run;
    let down = s.dir < 0;
    let run3 = vec3(run.0 as f32, 0.0, run.1 as f32);
    // the second lane is on the +x / +z side of the first
    let side = (run.1.abs(), run.0.abs());
    let side3 = vec3(side.0 as f32, 0.0, side.1 as f32);
    let (cx, cz) = (c.0 as f32, c.1 as f32);
    // a point `p` along the run and `q` across it inside the cell, at height y
    let at = |p: f32, q: f32, y: f32| {
        let x = if run.0 > 0 { cx + p } else if run.0 < 0 { cx + 1.0 - p } else { cx + q };
        let z = if run.1 > 0 { cz + p } else if run.1 < 0 { cz + 1.0 - p } else { cz + q };
        vec3(x, y, z)
    };
    let all = treads(s.dir);
    let per = STEPS_PER_CELL as usize;
    let first = (k - 1) as usize * per;
    let nbr = [(c.0 - side.0, c.1 - side.1), (c.0 + side.0, c.1 + side.1)];
    let inward = [side3, -side3];
    for i in 0..per {
        let t = all[first + i];
        let prev = if first + i == 0 { 0.0 } else { all[first + i - 1].top };
        let (t0, t1) = (i as f32 / per as f32, (i + 1) as f32 / per as f32);
        let col = shade(t.top, site.h);
        // the tread: flat stone in plain colour (see TREAD_STONE), so it never blends into the floor's picture
        let top = [at(t0, 0.0, t.top), at(t0, 1.0, t.top), at(t1, 1.0, t.top), at(t1, 0.0, t.top)];
        push(out, Face::Plain, top, [Vec2::ZERO; 4], vec3(0., 1., 0.), tread_tint(col));
        // the riser at the tread's near edge: its front faces back up the flight, toward the viewer at the top (a
        // flight down is seen from its top, a flight up from its foot), so the lamp lights it as the face it is
        let (hi, lo) = (prev.max(t.top), prev.min(t.top));
        // (the landing's flat treads have neither a riser nor a nosing)
        if hi - lo > 1e-5 {
            let riser = [at(t0, 0.0, hi), at(t0, 1.0, hi), at(t0, 1.0, lo), at(t0, 0.0, lo)];
            push(out, Face::Plain, riser, [Vec2::ZERO; 4], -run3, dim(RISER, shade(lo, site.h).r));
            // the nosing: a bright strip along the tread's front edge, the width of the lane (unlit, so it always shows)
            let size = vec3(0.2 * run.0.abs() as f32 + 0.96 * side.0 as f32, 0.05, 0.2 * run.1.abs() as f32 + 0.96 * side.1 as f32);
            out.glow.push((at(t0 + 0.1, 0.5, t.top + 0.01), size, NOSING));
        }
        let along = |t: f32| (k - 1) as f32 + t;
        for (lane, q) in [0.0f32, 1.0].into_iter().enumerate() {
            let (nx, nz) = nbr[lane];
            if (site.well)(nx, nz) {
                continue;
            }
            if down {
                // the pit's lining, from the floor line down to this tread
                vface(out, Face::Wall, site, (at(t0, q, 0.0), along(t0)), (at(t1, q, 0.0), along(t1)), t.top, 0.0, inward[lane], dim(col, 0.85));
            } else if !(site.solid)(nx, nz) {
                // the side of the stair block where nothing else closes it, facing out of the block
                wall_face(out, site, (at(t0, q, 0.0), along(t0)), (at(t1, q, 0.0), along(t1)), 0.0, t.top, -inward[lane], col);
            }
        }
    }
    let last = all[first + per - 1].top;
    let end = k == STAIR_FLIGHT;
    if down {
        if end {
            // the pit's far wall, behind the lowest tread
            vface(out, Face::Wall, site, (at(1.0, 0.0, 0.0), 0.0), (at(1.0, 1.0, 0.0), 1.0), last, 0.0, -run3, dim(shade(last, site.h), 0.85));
        }
        return;
    }
    // a flight up: stringers at the open end, then the shaft through the ceiling with its cap
    let (floor, roof) = (site.wall_h, site.wall_h + SHAFT_ABOVE);
    let mut shaft = |p0: f32, q0: f32, p1: f32, q1: f32, n: Vec3, u: (f32, f32)| {
        wall_face(out, site, (at(p0, q0, 0.0), u.0), (at(p1, q1, 0.0), u.1), floor, roof, n, shade(floor, site.h));
    };
    for lane in 0..2 {
        if !(site.well)(nbr[lane].0, nbr[lane].1) {
            let q = lane as f32;
            shaft(0.0, q, 1.0, q, inward[lane], ((k - 1) as f32, k as f32));
        }
    }
    if k == 1 {
        shaft(0.0, 0.0, 0.0, 1.0, run3, (0.0, 1.0));
    }
    if end {
        shaft(1.0, 0.0, 1.0, 1.0, -run3, (0.0, 1.0));
        let fwd = (c.0 + run.0, c.1 + run.1);
        if !(site.solid)(fwd.0, fwd.1) {
            // the stair block's end face, open to the room behind it
            wall_face(out, site, (at(1.0, 0.0, 0.0), 0.0), (at(1.0, 1.0, 0.0), 1.0), 0.0, last, -run3, shade(last, site.h));
        }
    }
    let cap = [at(0.0, 0.0, roof), at(0.0, 1.0, roof), at(1.0, 1.0, roof), at(1.0, 0.0, roof)];
    push(out, Face::Plain, cap, [Vec2::ZERO; 4], vec3(0., -1., 0.), CEILING);
}

/// The side of a ladder cell the ladder is fixed to: the first wall next to it (north, east, south, west), else north.
fn ladder_side(c: (i32, i32), solid: &dyn Fn(i32, i32) -> bool) -> (i32, i32) {
    [(0, -1), (1, 0), (0, 1), (-1, 0)]
        .into_iter()
        .find(|m| solid(c.0 + m.0, c.1 + m.1))
        .unwrap_or((0, -1))
}

/// A ladder's two rails and rungs fixed against side `m` of cell `c`, from height `y0` to `y1`.
fn ladder(out: &mut Parts, c: (i32, i32), m: (i32, i32), y0: f32, y1: f32) {
    let centre = vec3(c.0 as f32 + 0.5, 0.0, c.1 as f32 + 0.5);
    let to_wall = vec3(m.0 as f32, 0.0, m.1 as f32);
    let across = vec3(m.1.abs() as f32, 0.0, m.0.abs() as f32);
    let base = centre + to_wall * 0.4;
    let (len, mid) = (y1 - y0, (y0 + y1) / 2.0);
    for s in [-0.2f32, 0.2] {
        let p = base + across * s;
        out.boxes.push((vec3(p.x, mid, p.z), vec3(0.08, len, 0.08), IRON));
    }
    let rungs = (len / 0.3) as i32;
    for r in 1..=rungs {
        let y = y0 + r as f32 * 0.3;
        // the rung runs across the ladder: wide along `across`, thin the other way
        let size = vec3(0.07 + 0.33 * across.x, 0.06, 0.07 + 0.33 * across.z);
        out.boxes.push((vec3(base.x, y, base.z), size, IRON));
    }
}

/// A shaft down under a ladder's maze end (`Stair::is_ladder`, `dir < 0`): the four walls of the cell down to the
/// storey below, its floor, the ladder on the first wall, and the bars of a grate cover.
pub fn ladder_down(out: &mut Parts, c: (i32, i32), s: Stair, site: &Site) {
    let (cx, cz) = (c.0 as f32, c.1 as f32);
    let corner = |x: i32, z: i32| vec3(cx + x as f32, 0.0, cz + z as f32);
    // the walls, each in bands so the depth darkens them
    let sides = [
        ((0, 0), (1, 0), vec3(0., 0., 1.)),
        ((1, 0), (1, 1), vec3(-1., 0., 0.)),
        ((1, 1), (0, 1), vec3(0., 0., -1.)),
        ((0, 1), (0, 0), vec3(1., 0., 0.)),
    ];
    let bands = 4;
    for (a, b, n) in sides {
        for i in 0..bands {
            let (hi, lo) = (-site.h * i as f32 / bands as f32, -site.h * (i + 1) as f32 / bands as f32);
            wall_face(out, site, (corner(a.0, a.1), 0.0), (corner(b.0, b.1), 1.0), lo, hi, n, shade((hi + lo) / 2.0, site.h));
        }
    }
    let bottom = [vec3(cx, 0., cz), vec3(cx + 1., 0., cz), vec3(cx + 1., 0., cz + 1.), vec3(cx, 0., cz + 1.)].map(|v| vec3(v.x, -site.h + 0.01, v.z));
    push(out, Face::Floor, bottom, [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)], vec3(0., 1., 0.), shade(-site.h, site.h));
    // under its cover (every ladder well has one) the rails stop just below it
    ladder(out, c, ladder_side(c, site.solid), -site.h + 0.1, -0.1);
    if s.kind == LinkKind::Ladder(Cover::Grate) {
        for i in 0..5 {
            out.boxes.push((vec3(cx + 0.1 + 0.2 * i as f32, 0.025, cz + 0.5), vec3(0.05, 0.05, 1.0), GRATE));
        }
        for z in [0.25f32, 0.75] {
            out.boxes.push((vec3(cx + 0.5, 0.02, cz + z), vec3(1.0, 0.04, 0.05), GRATE));
        }
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

/// A ladder's sewer end (`dir > 0`): the ladder up the wall to a hatch in the ceiling.
pub fn ladder_up(out: &mut Parts, c: (i32, i32), site: &Site) {
    let m = ladder_side(c, site.solid);
    ladder(out, c, m, 0.0, site.wall_h - 0.05);
    let (x, z) = (c.0 as f32 + 0.5 + m.0 as f32 * 0.4, c.1 as f32 + 0.5 + m.1 as f32 * 0.4);
    let y = site.wall_h - 0.02;
    // the cover: an unlit plate just under the hatch's line, so it is lit grey-cream whatever the shaft's light is
    out.glow.push((vec3(x, y - 0.02, z), vec3(0.9, 0.03, 0.9), HATCH));
    // a cool frame round its edge and two grate bars across it, so the hatch reads as a cover from the ladder's foot
    for (centre, size) in [
        (vec3(x, y, z - 0.45), vec3(0.94, 0.04, 0.04)),
        (vec3(x, y, z + 0.45), vec3(0.94, 0.04, 0.04)),
        (vec3(x - 0.45, y, z), vec3(0.04, 0.04, 0.94)),
        (vec3(x + 0.45, y, z), vec3(0.04, 0.04, 0.94)),
    ] {
        out.glow.push((centre, size, DAYLIGHT));
    }
    for along in [-0.15f32, 0.15] {
        out.boxes.push((vec3(x + along, y, z), vec3(0.04, 0.04, 0.9), GRATE));
    }
}

/// The warm light of a stairwell (a cool shaft of daylight at a ladder's sewer end), so its steps read in the dark.
pub fn lamp(s: Stair, h: f32) -> Lamp {
    let (cx, cz) = s.cell.center();
    if s.is_ladder() {
        return if s.dir < 0 {
            // low in the shaft, under the cover's rim, so the grate's bars and the rungs take the light from above
            Lamp { pos: vec3(cx, -0.9, cz), range: 9.0, color: Color::new(1.0, 0.72, 0.42, 1.0), energy: 5.0 }
        } else {
            Lamp { pos: vec3(cx, h * 0.45, cz), range: 7.0, color: Color::new(0.8, 0.88, 1.0, 1.0), energy: 1.7 }
        };
    }
    // over the middle of the flight, between its two lanes
    let along = 0.5 + STAIR_FLIGHT as f32 / 2.0;
    let side = (s.run.1.abs() as f32, s.run.0.abs() as f32);
    let (x, z) = (cx + s.run.0 as f32 * along + side.0 * 0.5, cz + s.run.1 as f32 * along + side.1 * 0.5);
    let y = if s.dir < 0 { 0.3 } else { h * 0.5 };
    Lamp { pos: vec3(x, y, z), range: 13.0, color: Color::new(1.0, 0.74, 0.45, 1.0), energy: 5.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::facility::{stair_well_cells, Cell, LinkKind};
    use std::collections::HashSet;

    const H: f32 = 5.0;

    fn stair(dir: i32, run: (i32, i32)) -> Stair {
        Stair { cell: Cell::new(20, 20), dir, run, kind: LinkKind::Stairs }
    }

    /// The flight's quads, every cell of both lanes, with nothing around it but open floor.
    fn build(s: Stair) -> Parts {
        let well: HashSet<(i32, i32)> = (1..=STAIR_FLIGHT)
            .flat_map(|k| stair_well_cells(s, k))
            .map(|c| (c.x, c.y))
            .collect();
        let (well_fn, solid_fn) = (|x, z| well.contains(&(x, z)), |_, _| false);
        let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well_fn, solid: &solid_fn };
        let mut out = Parts::default();
        for k in 1..=STAIR_FLIGHT {
            for c in stair_well_cells(s, k) {
                flight_cell(&mut out, (c.x, c.y), k, s, &site);
            }
        }
        out
    }

    fn tread_quads(p: &Parts) -> Vec<&Quad> {
        p.quads.iter().filter(|q| q.face == Face::Plain && q.n == vec3(0., 1., 0.)).collect()
    }

    const RUNS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];

    #[test]
    fn a_flight_has_every_step_with_monotonic_heights() {
        let n = (STAIR_FLIGHT * STEPS_PER_CELL) as usize;
        for dir in [-1, 1] {
            let t = treads(dir);
            assert_eq!(t.len(), n);
            // a flight down ends in LANDING flat treads; every other tread is one rise further down (or up)
            let landing = if dir < 0 { LANDING as usize } else { 0 };
            for (i, w) in t.windows(2).enumerate() {
                if i + 1 < n - landing {
                    assert!((w[1].top - w[0].top) * dir as f32 > 0.0, "dir {dir}: {w:?}");
                } else {
                    assert_eq!(w[1].top, w[0].top, "dir {dir}: the landing is flat");
                }
                // and each starts where the last one ended
                assert!((w[1].from - w[0].to).abs() < 1e-6);
            }
            assert!((t[0].top - dir as f32 * (if dir < 0 { RISE_DOWN } else { RISE_UP }) / STEPS_PER_CELL as f32).abs() < 1e-5, "the first step is one rise from the landing");
            assert!(t.iter().all(|x| x.top.abs() <= t.last().unwrap().top.abs() + 1e-6), "nothing is past the last tread");
        }
    }

    #[test]
    fn no_step_goes_below_the_floor_beneath_or_through_the_ceiling() {
        for t in treads(-1) {
            assert!(t.top < 0.0 && t.top > -H, "{t:?} sinks to or below the level beneath");
        }
        for t in treads(1) {
            assert!(t.top > 0.0 && t.top < H, "{t:?} climbs past the ceiling");
        }
        // the steps of the mesh itself, every run direction, both lanes
        for dir in [-1, 1] {
            for run in RUNS {
                let p = build(stair(dir, run));
                for q in &p.quads {
                    for v in q.c {
                        if dir < 0 {
                            assert!(v.y <= 1e-5 && v.y > -H, "{dir} {run:?}: vertex at {}", v.y);
                        } else {
                            assert!(v.y >= -1e-5 && v.y <= H + SHAFT_ABOVE + 1e-5, "{dir} {run:?}: vertex at {}", v.y);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_mesh_has_n_treads_in_each_of_two_lanes_in_both_directions() {
        let n = (STAIR_FLIGHT * STEPS_PER_CELL) as usize;
        for dir in [-1, 1] {
            for run in RUNS {
                let p = build(stair(dir, run));
                let treads_up = tread_quads(&p);
                assert_eq!(treads_up.len(), 2 * n, "{dir} {run:?}");
                // along the run the tread heights only ever go one way
                let along = |q: &Quad| {
                    let c = (q.c[0] + q.c[2]) * 0.5;
                    (c.x - 20.5) * run.0 as f32 + (c.z - 20.5) * run.1 as f32
                };
                // the first lane is the one on the -x / -z side of the flight
                let first_lane = |q: &Quad| {
                    let c = (q.c[0] + q.c[2]) * 0.5;
                    ((c.x - 20.5) * run.1.abs() as f32 + (c.z - 20.5) * run.0.abs() as f32) < 0.5
                };
                let mut lane: Vec<(f32, f32)> = treads_up.iter().filter(|q| first_lane(q)).map(|q| (along(q), q.c[0].y)).collect();
                lane.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                assert_eq!(lane.len(), n);
                for w in lane.windows(2) {
                    assert!((w[1].1 - w[0].1) * dir as f32 >= -1e-6, "{dir} {run:?}: {w:?}");
                }
                assert!((lane[n - 1].1 - lane[0].1) * dir as f32 > 0.5, "{dir} {run:?}: the flight goes somewhere");
                // a riser (plain, upright, across the run) for every sloped tread, none on the landing
                let sloped = if dir < 0 { n - LANDING as usize } else { n };
                let risers = p.quads.iter().filter(|q| q.face == Face::Plain && q.n.y == 0.0 && (q.c[0].y - q.c[3].y).abs() > 0.05).count();
                assert_eq!(risers, 2 * sloped, "{dir} {run:?}: {risers} risers");
            }
        }
    }

    #[test]
    fn steps_are_never_pure_black() {
        for dir in [-1, 1] {
            // the steps (their stone tops), the floor-and-wall pictures (the shaft's cap is a ceiling, and may be dark)
            for q in build(stair(dir, (0, -1))).quads.iter().filter(|q| q.face != Face::Plain || q.n.y > 0.5) {
                assert!(q.col.r >= 0.5 && q.col.g >= 0.5 && q.col.b >= 0.5, "{dir}: {:?}", q.col);
            }
            // the plain risers are concrete, lit to a third of white at the worst
            for q in build(stair(dir, (0, -1))).quads.iter().filter(|q| q.face == Face::Plain && q.n.y == 0.0) {
                assert!(q.col.r >= 0.28 && q.col.g >= 0.25 && q.col.b >= 0.22, "{dir}: {:?}", q.col);
            }
        }
    }

    #[test]
    fn a_flight_down_is_shallow_enough_to_show_its_treads_past_the_lip() {
        // the lip hides a tread when its drop per metre of run is steeper than the eye's line over the lip, so the
        // flight stays at or under 0.25 a metre (about 14 degrees), and every riser is still a visible step
        assert!(RISE_DOWN <= 0.25, "RISE_DOWN {RISE_DOWN}");
        let riser = RISE_DOWN / STEPS_PER_CELL as f32;
        assert!(riser >= 0.06, "riser {riser} m");
        let first = treads(-1)[0].top;
        assert!((first + riser).abs() < 1e-5, "the first step is one riser down: {first}");
    }

    #[test]
    fn treads_are_light_stone_not_the_floor_picture_and_their_nosings_are_cream() {
        // the stone is a light flat grey-cream, lit to at least two thirds of itself at the lowest tread
        assert!(TREAD_STONE.r >= 0.7 && TREAD_STONE.g >= 0.7 && TREAD_STONE.b >= 0.65, "{TREAD_STONE:?}");
        for dir in [-1, 1] {
            for run in RUNS {
                let p = build(stair(dir, run));
                let treads = tread_quads(&p);
                assert_eq!(treads.len(), 2 * (STAIR_FLIGHT * STEPS_PER_CELL) as usize, "{dir} {run:?}: every tread is stone");
                for q in treads {
                    // plain colour, never the floor picture: the stone is light on every shade
                    assert!(q.col.r >= 0.45 && q.col.g >= 0.45 && q.col.b >= 0.4, "{dir} {run:?}: tread {:?}", q.col);
                    assert!(q.col.r <= 1.0 && q.col.g <= 1.0 && q.col.b <= 1.0, "{dir} {run:?}: tread {:?}", q.col);
                }
            }
        }
        assert!(NOSING.r >= 0.95 && NOSING.g >= 0.85 && NOSING.b >= 0.6, "the nosing is cream: {NOSING:?}");
        let p = build(stair(-1, (0, -1)));
        assert_eq!(p.glow.len(), 2 * STAIR_FLIGHT as usize * STEPS_PER_CELL as usize - 2 * LANDING as usize, "one nosing per sloped tread and lane");
        // the nosings are wide enough to count from two metres: a fifth of a tread's run, and a bright edge
        assert!(p.glow.iter().all(|(_, s, _)| s.y >= 0.04 && (s.x.max(s.z) >= 0.2 && s.x.max(s.z) <= 1.0)));
    }

    #[test]
    fn risers_face_back_up_the_flight_toward_the_viewer() {
        // a riser's visible face points back up the flight (against the run), so the lamp lights it from the front
        for dir in [-1, 1] {
            for run in RUNS {
                let p = build(stair(dir, run));
                let r = vec3(run.0 as f32, 0.0, run.1 as f32);
                let risers: Vec<_> = p.quads.iter().filter(|q| q.face == Face::Plain && q.n.y == 0.0 && (q.c[0].y - q.c[3].y).abs() > 0.05).collect();
                assert!(!risers.is_empty());
                for q in risers {
                    assert!(q.n.dot(r) < 0.0, "{dir} {run:?}: riser faces {:?}", q.n);
                }
            }
        }
    }

    #[test]
    fn the_ladder_lamp_sits_in_the_shaft_below_the_cover_and_lights_the_rungs() {
        // the shaft's lamp hangs under the cover, above the ladder's foot, so the rungs take its light from the side
        let l = lamp(Stair { cell: Cell::new(5, 5), dir: -1, run: (0, 0), kind: LinkKind::Ladder(Cover::Grate) }, H);
        assert!(l.pos.y < 0.0 && l.pos.y > -H + 0.1, "lamp at {}", l.pos.y);
        assert!(l.energy >= 5.0 && l.range >= 9.0, "{} {}", l.energy, l.range);
        assert!(IRON.r >= 0.75 && IRON.g >= 0.7 && IRON.b >= 0.65, "the rungs are a light iron: {IRON:?}");
    }

    #[test]
    fn a_flight_down_is_lined_and_closed_at_the_far_end() {
        let p = build(stair(-1, (0, -1)));
        // the far wall at the end of the run sinks to the lowest tread
        let low = treads(-1).last().unwrap().top;
        assert!(p.quads.iter().any(|q| q.face == Face::Wall && q.c.iter().all(|v| (v.z - 16.0).abs() < 1e-5) && q.c.iter().any(|v| (v.y - low).abs() < 1e-5)));
        // the last cell of the flight is a flat landing of floor at that depth
        let landing: Vec<_> = tread_quads(&p).into_iter().filter(|q| q.c.iter().all(|v| v.z > 16.0 - 1e-5 && v.z < 17.0 + 1e-5)).collect();
        assert_eq!(landing.len(), 2 * LANDING as usize);
        assert!(landing.iter().all(|q| q.c.iter().all(|v| (v.y - low).abs() < 1e-5)));
        // and nothing is drawn across the two lanes' shared edge
        assert!(p.quads.iter().all(|q| !q.c.iter().all(|v| (v.x - 21.0).abs() < 1e-5)));
    }

    #[test]
    fn a_flight_up_opens_through_the_ceiling_into_a_capped_shaft() {
        let p = build(stair(1, (1, 0)));
        let caps = p.quads.iter().filter(|q| q.face == Face::Plain && q.n.y < 0.0).count();
        assert_eq!(caps, 2 * STAIR_FLIGHT as usize, "one cap per cell of the well");
        assert!(p.quads.iter().filter(|q| q.face == Face::Plain && q.n.y < 0.0).all(|q| q.c.iter().all(|v| (v.y - (H + SHAFT_ABOVE)).abs() < 1e-5)));
        assert!(p.quads.iter().any(|q| q.face == Face::Wall && q.c.iter().any(|v| v.y > H)), "shaft walls above the ceiling");
    }

    #[test]
    fn the_stairwell_lamp_hangs_over_its_flight_and_the_ladder_lamps_in_their_shafts() {
        for dir in [-1, 1] {
            for run in RUNS {
                let s = stair(dir, run);
                let l = lamp(s, H);
                let cells: Vec<Cell> = (1..=STAIR_FLIGHT).flat_map(|k| stair_well_cells(s, k)).collect();
                assert!(cells.iter().any(|c| (c.x as f32..c.x as f32 + 1.0).contains(&l.pos.x) && (c.y as f32..c.y as f32 + 1.0).contains(&l.pos.z)), "{dir} {run:?}: {:?}", l.pos);
                assert!(l.range > 0.0 && l.energy > 0.0);
            }
        }
        let ladder_stair = |dir| Stair { cell: Cell::new(5, 5), dir, run: (0, 0), kind: LinkKind::Ladder(Cover::Manhole) };
        assert!(lamp(ladder_stair(-1), H).pos.y < 0.0, "inside the shaft");
        assert!(lamp(ladder_stair(1), H).pos.y > 0.0);
    }

    #[test]
    fn a_ladder_well_is_walled_to_the_floor_below_and_has_a_ladder() {
        let solid = |x: i32, z: i32| (x, z) == (9, 10);
        let well = |_, _| false;
        let site = Site { h: H, wall_h: H, tiers: 1.0, well: &well, solid: &solid };
        let s = Stair { cell: Cell::new(10, 10), dir: -1, run: (0, 0), kind: LinkKind::Ladder(Cover::Grate) };
        let mut p = Parts::default();
        ladder_down(&mut p, (10, 10), s, &site);
        let walls: Vec<_> = p.quads.iter().filter(|q| q.face == Face::Wall).collect();
        assert_eq!(walls.len(), 16, "four walls in four bands");
        let deepest = walls.iter().flat_map(|q| q.c).map(|v| v.y).fold(0.0f32, f32::min);
        assert!((deepest + H).abs() < 1e-5);
        assert!(p.quads.iter().any(|q| q.face == Face::Floor && q.c.iter().all(|v| v.y < -H + 0.1)));
        // two rails and a rung every 0.3 m from the bottom to a hand above the floor, then the grate's bars
        assert!(p.boxes.len() >= 2 + 15 + 7);
        // the ladder hangs on the wall to the west (x = 9)
        assert!(p.boxes.iter().take(2).all(|(c, _, _)| c.x < 10.2));
        // the rails run from the bottom up to just under the cover, not above the floor
        assert!(p.boxes.iter().take(2).all(|(c, s, _)| c.y - s.y / 2.0 <= -H + 0.11 && c.y + s.y / 2.0 <= 0.0 && c.y + s.y / 2.0 >= -0.2));
        let mut up = Parts::default();
        ladder_up(&mut up, (10, 10), &site);
        assert!(up.boxes.iter().all(|(c, _, _)| c.y > 0.0 && c.y < H));
        assert_eq!(up.quads.len(), 0, "the hatch is an unlit plate, not a dark picture");
        // the hatch's plate is lit grey-cream (never black) and sits under the ceiling, with a lit frame and bars
        let plate = up.glow.iter().find(|(_, s, _)| s.x > 0.8 && s.z > 0.8).expect("the plate");
        assert!(plate.2.r >= 0.55 && plate.2.g >= 0.55 && plate.2.b >= 0.5, "{:?}", plate.2);
        assert!(plate.0.y < site.wall_h && plate.0.y > site.wall_h - 0.1, "under the ceiling: {}", plate.0.y);
        assert_eq!(up.glow.len(), 5, "the plate and the frame");
        assert!(up.boxes.iter().filter(|(_, _, col)| *col == GRATE).count() == 2, "two bars across the hatch");
        // the grate's bars are mid grey, not near-black, and its rim is lit
        assert!(GRATE.r >= 0.3 && GRATE.g >= 0.3 && GRATE.b >= 0.3, "{GRATE:?}");
        let mut grate = Parts::default();
        ladder_down(&mut grate, (10, 10), Stair { cell: Cell::new(10, 10), dir: -1, run: (0, 0), kind: LinkKind::Ladder(Cover::Grate) }, &site);
        assert_eq!(grate.glow.len(), 4, "the grate's lit rim");
        assert!(grate.glow.iter().all(|(_, _, col)| *col == RIM));
        // the shaft's lamp is lit well enough to read the rungs from a standing eye
        let l = lamp(Stair { cell: Cell::new(10, 10), dir: -1, run: (0, 0), kind: LinkKind::Ladder(Cover::Grate) }, H);
        assert!(l.energy >= 3.0 && l.range >= 8.0, "{} {}", l.energy, l.range);
    }
}
