//! Hall pilasters and hall arches: the procedural wall dressing of the Godot original, ported line for line from
//! `game/look/ref_decor.gd` (LastRouteGodot). There is no model file: every piece is a box.
//!
//! What is ported, by Godot line:
//! - `is_pilaster_column`, `PILASTER_EVERY`, `PILASTER_PHASE`, `PILASTER_DEPTH` (175-180, 194) -> [`is_pilaster_column`]
//!   and the constants here.
//! - the pilaster branch of `_wall_cell` (698-716 for the face, 761-775 for the pilaster, its caps and the arch
//!   trigger) -> [`wall_face`].
//! - `_open` (624-625) -> [`ChunkView::is_open`]; `_open_run` (795-802) -> [`open_run`].
//! - `_hall_arch` (254-282) -> [`hall_arch`]; `_sewer_haunch` (290-311) -> [`sewer_haunch`].
//! - `plane_uv` (341-346, the mirror of `ref_plane_uv.gdshader`) -> [`plane_uv`].
//! - `_box_t` (392-393) -> [`ArchBox::flat`]; `_collide("pilaster", ..)` (212-213, 768) -> [`Decor::colliders`].
//! - `arch_log` (250, 280-282, 309-311) -> [`Decor::log`], which the tests here check as
//!   `tests/validate_wall_themes.gd` `_check_arch_log` does (165-200).
//! - `step`'s loop (99-104) with `DIRS` (687) -> [`chunk_decor`]. The `mezzanine_grid` skip (92-95) has nothing to
//!   port: the mezzanine is off in both engines.
//!
//! Everything is in the chunk's own frame, as in Godot: cell (x, z) of the chunk's 45 x 45 grid, metres from the
//! chunk's corner, y up from the storey's floor. A cell outside the chunk is never open (`_open`), so no run or arch
//! reads past the chunk's edge and a chunk's dressing never depends on its neighbours. Nothing here is random.
//!
//! This module is pure: no GL, no assets, no `Facility`. The caller hands it a [`ChunkView`] (closures over whatever
//! holds the grid) and turns the boxes into quads (`render::world_mesh`).

use macroquad::prelude::{vec2, vec3, Vec2, Vec3};

/// A pilaster stands on every sixth wall cell along a run (ref_decor.gd:175).
pub const PILASTER_EVERY: i32 = 6;
/// The phase of the pilaster cells along a run (ref_decor.gd:176).
pub const PILASTER_PHASE: i32 = 3;
/// How far a hall pilaster stands out of the wall, metres (ref_decor.gd:194).
pub const PILASTER_DEPTH: f32 = 0.32;
/// The wall directions in Godot's order (`DIRS`, ref_decor.gd:687): +z, -z, +x, -x.
pub const DIRS: [(i32, i32); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];

/// `is_pilaster_column` (ref_decor.gd:179-180): `posmod(along, 6) == 3`.
pub fn is_pilaster_column(along: i32) -> bool {
    along.rem_euclid(PILASTER_EVERY) == PILASTER_PHASE
}

/// What the placement reads of one chunk. Every closure takes a chunk-local cell (x, z) and is only called for cells
/// inside the chunk.
pub struct ChunkView<'a> {
    /// The chunk's side in cells (Godot `_size`, 45).
    pub size: i32,
    /// Whether the chunk's region is the sewer (`_region == "sewer"`): its halls get the haunch rib, not the full arch.
    pub sewer: bool,
    /// Godot `grid[z][x] == 0`: a walkable cell. In the sewers that is dry floor or a bridge deck only (the walk grid
    /// closes channels, drops and ramp tunnels).
    pub open: &'a dyn Fn(i32, i32) -> bool,
    /// Whether a closed cell is real wall. Always true outside the sewers; in the sewers only `SOLID` cells of the
    /// `sewer_grid` are (ref_decor.gd:702-704).
    pub brick: &'a dyn Fn(i32, i32) -> bool,
    /// Whether the cell is inside a room's rectangle (Godot `_room_id_at(cell) != ""`).
    pub room: &'a dyn Fn(i32, i32) -> bool,
    /// Whether the cell is a ramp cell (Godot `data["ramp_cells"].has(cell)`).
    pub ramp: &'a dyn Fn(i32, i32) -> bool,
    /// The ceiling height over an open cell, metres (Godot `_hh`).
    pub height: &'a dyn Fn(i32, i32) -> f32,
}

impl ChunkView<'_> {
    fn inside(&self, x: i32, z: i32) -> bool {
        x >= 0 && z >= 0 && x < self.size && z < self.size
    }

    /// `_open` (ref_decor.gd:624-625): inside the chunk and walkable.
    pub fn is_open(&self, x: i32, z: i32) -> bool {
        self.inside(x, z) && (self.open)(x, z)
    }
}

/// What a box is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Material {
    /// The hall's own wall (Godot key `arch_wall:<wall kind>@<height>`): the wall kind of the face that looks into the
    /// open cell `cell`, mapped for a room of `height` metres ([`plane_uv`]).
    Wall { cell: (i32, i32), height: f32 },
    /// The district's accent (Godot key `arch_trim:<region>`), the region being the chunk's; `cell` is the open hall
    /// cell the piece belongs to, `height` its ceiling (the trim uses the same mapping as the wall).
    Trim { cell: (i32, i32), height: f32 },
}

/// One oriented box: Godot's `Transform3D(Basis(axes[0], axes[1], axes[2]), centre)` on a unit cube, so each axis is
/// the box's FULL extent along one of its edges (not a half extent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArchBox {
    pub centre: Vec3,
    pub axes: [Vec3; 3],
    pub material: Material,
}

impl ArchBox {
    /// `_box_t` (ref_decor.gd:392-393): an axis-aligned box of `size`.
    pub fn flat(centre: Vec3, size: Vec3, material: Material) -> ArchBox {
        ArchBox { centre, axes: [vec3(size.x, 0.0, 0.0), vec3(0.0, size.y, 0.0), vec3(0.0, 0.0, size.z)], material }
    }

    /// The eight corners of the box.
    pub fn corners(&self) -> [Vec3; 8] {
        let mut out = [self.centre; 8];
        for (i, slot) in out.iter_mut().enumerate() {
            for (k, axis) in self.axes.iter().enumerate() {
                let sign = if i & (1 << k) != 0 { 0.5 } else { -0.5 };
                *slot += *axis * sign;
            }
        }
        out
    }
}

/// A solid box for the player (Godot `collider_boxes`, kind "pilaster"): axis-aligned, `size` is the full extent.
/// Hunters ignore these in Godot (`HunterView3D.ignore_decor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collider {
    pub centre: Vec3,
    pub size: Vec3,
}

/// One `arch_log` entry (ref_decor.gd:250, 280-282, 309-311), for the checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArchEntry {
    /// A full arch across a hall.
    Hall { centre: Vec3, hall_centre: Vec3, span: f32, between: f32, spring: f32, top: f32, ceiling: f32 },
    /// A sewer rib: a quarter ellipse from the pilaster face up to the ceiling.
    Haunch { centre: Vec3, spring: f32, lowest: f32, top: f32, ceiling: f32, reach: f32, width: f32, cell: (i32, i32) },
}

/// What the placement produced: the drawn boxes, the player colliders (pilaster shafts only) and the arch log.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decor {
    pub boxes: Vec<ArchBox>,
    pub colliders: Vec<Collider>,
    pub log: Vec<ArchEntry>,
}

/// `_open_run` (ref_decor.gd:795-802): the wall run is solid and the hall open one and two cells either side of the
/// wall cell (x, z), so a pilaster here does not flank an opening.
pub fn open_run(view: &ChunkView, x: i32, z: i32, d: (i32, i32)) -> bool {
    let along = if d.1 != 0 { (1, 0) } else { (0, 1) };
    for k in [-2, -1, 1, 2] {
        let (cx, cz) = (x + along.0 * k, z + along.1 * k);
        if view.is_open(cx, cz) || !view.is_open(cx + d.0, cz + d.1) {
            return false;
        }
    }
    true
}

/// The pilaster part of `_wall_cell` (ref_decor.gd:698-716, 761-775) for the face `d` of the wall cell (x, z): the
/// shaft with its collider, the two caps, and the arch (or sewer rib) every second pilaster springs.
///
/// Nothing is added unless (x, z) is a wall cell of the chunk (real brick in the sewers) and the cell it faces is
/// open, is in no room and is no ramp cell.
pub fn wall_face(view: &ChunkView, x: i32, z: i32, d: (i32, i32), out: &mut Decor) {
    // `_cell` (690-693) calls `_wall_cell` for a closed cell only; `_wall_cell` leaves a sewer cell that is not brick
    if !view.inside(x, z) || view.is_open(x, z) || !(view.brick)(x, z) {
        return;
    }
    let (ox, oz) = (x + d.0, z + d.1);
    if !view.is_open(ox, oz) {
        return;
    }
    let n = vec3(d.0 as f32, 0.0, d.1 as f32);
    let base = vec3(x as f32 + 0.5 + d.0 as f32 * 0.5, 0.0, z as f32 + 0.5 + d.1 as f32 * 0.5);
    // position along the wall run: chunk-local x for a wall facing z, else z
    let along = if d.1 != 0 { x } else { z };
    // Godot: `rid == "" and not beside_ramp and is_pilaster_column(along) and _open_run(..)`; the column test is
    // first here because it is the cheapest and the four are independent
    if !is_pilaster_column(along) || (view.room)(ox, oz) || (view.ramp)(ox, oz) || !open_run(view, x, z, d) {
        return;
    }
    let height = (view.height)(ox, oz);
    let hall_top = height - 0.1;
    let across_x = d.1 != 0;
    let wall = Material::Wall { cell: (ox, oz), height };
    let trim = Material::Trim { cell: (ox, oz), height };
    let column = base + n * (PILASTER_DEPTH * 0.5) + vec3(0.0, hall_top * 0.5, 0.0);
    let shaft = vec3(if across_x { 0.42 } else { PILASTER_DEPTH }, hall_top, if across_x { PILASTER_DEPTH } else { 0.42 });
    out.boxes.push(ArchBox::flat(column, shaft, wall));
    out.colliders.push(Collider { centre: column, size: shaft });
    for cap in [0.06, hall_top - 0.06] {
        out.boxes.push(ArchBox::flat(
            base + n * (PILASTER_DEPTH * 0.5) + vec3(0.0, cap, 0.0),
            vec3(if across_x { 0.54 } else { 0.42 }, 0.12, if across_x { 0.42 } else { 0.54 }),
            trim,
        ));
    }
    // every second pair springs one arch across the hall, from the +x / +z wall only (the facing wall has the
    // matching pilaster)
    if along % 12 == 3 && d.0 + d.1 > 0 {
        hall_arch(view, x, z, d, base, n, hall_top, (wall, trim), out);
    }
}

/// `_hall_arch` (ref_decor.gd:254-282): one arch across a hall, ten voussoirs on a half ellipse in the hall's wall
/// and a keystone in the accent. `base` is the middle of the wall face at the floor, `n` the face's normal.
#[allow(clippy::too_many_arguments)]
fn hall_arch(view: &ChunkView, x: i32, z: i32, d: (i32, i32), base: Vec3, n: Vec3, hall_top: f32, (wall, trim): (Material, Material), out: &mut Decor) {
    let mut width = 0;
    while width < 12 && view.is_open(x + d.0 * (width + 1), z + d.1 * (width + 1)) {
        width += 1;
    }
    // the far wall must be there (and carry its pilaster: an open run on its side too)
    let far = (x + d.0 * (width + 1), z + d.1 * (width + 1));
    if width < 2 || width >= 12 || view.is_open(far.0, far.1) || !open_run(view, far.0, far.1, (-d.0, -d.1)) {
        return;
    }
    if view.sewer {
        sewer_haunch(x, z, base, n, width as f32, hall_top, (wall, trim), out);
        return;
    }
    let half = width as f32 * 0.5 - PILASTER_DEPTH;
    let rise = (half * 0.55).clamp(0.45, 1.1);
    let spring = hall_top - rise - 0.18;
    let centre = base + n * (width as f32 * 0.5) + Vec3::Y * spring;
    let along = n.cross(Vec3::Y).normalize();
    let segments = 10;
    let mut previous = centre + n * half;
    for i in 1..=segments {
        let theta = std::f32::consts::PI * i as f32 / segments as f32;
        let point = centre + n * (half * theta.cos()) + Vec3::Y * (rise * theta.sin());
        let tangent = point - previous;
        let radial = along.cross(tangent.normalize());
        out.boxes.push(ArchBox { centre: (point + previous) * 0.5, axes: [tangent * 1.06, radial * 0.3, along * 0.5], material: wall });
        previous = point;
    }
    out.boxes.push(ArchBox::flat(centre + Vec3::Y * rise, vec3(0.3, 0.42, 0.3) + along.abs() * 0.26, trim));
    out.log.push(ArchEntry::Hall {
        centre,
        hall_centre: base + n * (width as f32 * 0.5),
        span: half * 2.0,
        between: width as f32 - PILASTER_DEPTH * 2.0,
        spring,
        top: spring + rise + 0.15,
        ceiling: hall_top + 0.1,
    });
}

/// `_sewer_haunch` (ref_decor.gd:290-311): the sewers' rib. The open span stops at the channel's edge, so a quarter
/// ellipse of six voussoirs springs from the pilaster face and curves up to the vault, its keystone flush against
/// the ceiling; nothing hangs below the spring line or stands in the walkway.
#[allow(clippy::too_many_arguments)]
fn sewer_haunch(x: i32, z: i32, base: Vec3, n: Vec3, width: f32, hall_top: f32, (wall, trim): (Material, Material), out: &mut Decor) {
    let reach = (width * 0.5 - PILASTER_DEPTH).clamp(0.55, 1.2);
    let rise = (reach * 0.8).clamp(0.45, 0.9);
    let ceiling = hall_top + 0.1;
    let spring = ceiling - rise;
    let centre = base + n * (PILASTER_DEPTH + reach) + Vec3::Y * spring;
    let along = n.cross(Vec3::Y).normalize();
    let segments = 6;
    let mut previous = centre - n * reach;
    let mut lowest = f32::INFINITY;
    for i in 1..=segments {
        let theta = std::f32::consts::PI - std::f32::consts::PI * 0.5 * i as f32 / segments as f32;
        let point = centre + n * (reach * theta.cos()) + Vec3::Y * (rise * theta.sin());
        let tangent = point - previous;
        let radial = along.cross(tangent.normalize());
        out.boxes.push(ArchBox { centre: (point + previous) * 0.5, axes: [tangent * 1.06, radial * 0.3, along * 0.5], material: wall });
        lowest = lowest.min(point.y.min(previous.y) - 0.16);
        previous = point;
    }
    out.boxes.push(ArchBox::flat(centre + Vec3::Y * (rise - 0.21), vec3(0.3, 0.42, 0.3) + along.abs() * 0.26, trim));
    out.log.push(ArchEntry::Haunch { centre, spring, lowest, top: ceiling, ceiling, reach: PILASTER_DEPTH + reach, width, cell: (x, z) });
}

/// The dressing of a whole chunk, in Godot's order (`step`, ref_decor.gd:99-104: rows of z, x across each row, the
/// four `DIRS` of each wall cell).
pub fn chunk_decor(view: &ChunkView) -> Decor {
    let mut out = Decor::default();
    for z in 0..view.size {
        for x in 0..view.size {
            for d in DIRS {
                wall_face(view, x, z, d, &mut out);
            }
        }
    }
    out
}

/// `plane_uv` (ref_decor.gd:341-346), what `ref_plane_uv.gdshader` computes for a point of a face: `world` is the
/// point, `normal` the face's normal, `floor_y` the floor's y, `room_height` the ceiling height the wall texture is
/// stretched over and `tiers` the texture repeats up that height (`RefLook.wall_tiers(room_height)`). A flat face
/// (|n.y| > 0.95) takes the ground plane; any other takes u in metres along the face and v counted from the ceiling.
pub fn plane_uv(world: Vec3, normal: Vec3, floor_y: f32, room_height: f32, tiers: f32) -> Vec2 {
    let n = normal.normalize();
    if n.y.abs() > 0.95 {
        return vec2(world.x, world.z);
    }
    let along = Vec3::Y.cross(n).normalize();
    vec2(world.dot(along), tiers * (1.0 - (world.y - floor_y) / room_height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: i32 = 45;
    const HEIGHT: f32 = 3.3;

    /// A chunk of solid wall with the given rectangles (x, z, w, h) carved open.
    fn carved(rects: &[(i32, i32, i32, i32)]) -> Vec<Vec<bool>> {
        let mut open = vec![vec![false; SIZE as usize]; SIZE as usize];
        for &(x0, z0, w, h) in rects {
            for z in z0..z0 + h {
                for x in x0..x0 + w {
                    open[z as usize][x as usize] = true;
                }
            }
        }
        open
    }

    /// The dressing of a carved chunk; `room` and `ramp` mark cells, `sewer` picks the rib.
    fn decor_of(open: &[Vec<bool>], sewer: bool, room: &dyn Fn(i32, i32) -> bool, ramp: &dyn Fn(i32, i32) -> bool) -> Decor {
        let is_open = |x: i32, z: i32| open[z as usize][x as usize];
        let view = ChunkView { size: SIZE, sewer, open: &is_open, brick: &|_, _| true, room, ramp, height: &|_, _| HEIGHT };
        chunk_decor(&view)
    }

    fn plain(open: &[Vec<bool>]) -> Decor {
        decor_of(open, false, &|_, _| false, &|_, _| false)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.0e-4
    }

    /// A hall `width` cells wide running the whole chunk along x, its near wall on row 9.
    fn hall_along_x(width: i32) -> Vec<Vec<bool>> {
        carved(&[(0, 10, SIZE, width)])
    }

    #[test]
    fn pilaster_columns_are_every_sixth_cell_at_phase_three() {
        let columns: Vec<i32> = (-12..24).filter(|a| is_pilaster_column(*a)).collect();
        assert_eq!(columns, vec![-9, -3, 3, 9, 15, 21]);
    }

    #[test]
    fn a_three_wide_hall_gets_pilasters_every_six_cells_on_both_walls() {
        let decor = plain(&hall_along_x(3));
        // x = 3, 9, .. 39: the run must be whole two cells either side, so nothing nearer the chunk's edge
        let mut near: Vec<i32> = vec![];
        let mut far: Vec<i32> = vec![];
        for c in &decor.colliders {
            assert!(close(c.size.x, 0.42) && close(c.size.z, PILASTER_DEPTH) && close(c.size.y, HEIGHT - 0.1), "{c:?}");
            assert!(close(c.centre.y, (HEIGHT - 0.1) * 0.5), "{c:?}");
            let x = (c.centre.x - 0.5).round() as i32;
            assert!(close(c.centre.x, x as f32 + 0.5), "{c:?}");
            if close(c.centre.z, 10.0 + PILASTER_DEPTH * 0.5) {
                near.push(x);
            } else {
                assert!(close(c.centre.z, 13.0 - PILASTER_DEPTH * 0.5), "{c:?}");
                far.push(x);
            }
        }
        near.sort_unstable();
        far.sort_unstable();
        let want = vec![3, 9, 15, 21, 27, 33, 39];
        assert_eq!(near, want);
        assert_eq!(far, want);
        // each pilaster is a shaft in the hall's wall and two caps in the trim; each arch ten voussoirs and a keystone
        assert_eq!(decor.log.len(), 4);
        assert_eq!(decor.boxes.len(), 14 * 3 + 4 * 11);
        let trims = decor.boxes.iter().filter(|b| matches!(b.material, Material::Trim { .. })).count();
        assert_eq!(trims, 14 * 2 + 4);
    }

    #[test]
    fn arches_spring_at_every_twelfth_cell_from_the_positive_wall_only() {
        let decor = plain(&hall_along_x(3));
        let mut at: Vec<i32> = vec![];
        for entry in &decor.log {
            let ArchEntry::Hall { centre, hall_centre, .. } = *entry else { panic!("a rib outside the sewers: {entry:?}") };
            // the +z-facing wall is row 9: the arch starts on its face (z = 10) and is centred on the hall (z = 11.5)
            assert!(close(hall_centre.z, 11.5) && close(centre.z, 11.5), "{entry:?}");
            at.push((centre.x - 0.5).round() as i32);
        }
        at.sort_unstable();
        assert_eq!(at, vec![3, 15, 27, 39]);
        // the same hall along z: the arch springs from the +x-facing wall
        let decor = plain(&carved(&[(10, 0, 3, SIZE)]));
        assert_eq!(decor.colliders.len(), 14);
        assert_eq!(decor.log.len(), 4);
        for entry in &decor.log {
            let ArchEntry::Hall { centre, .. } = *entry else { panic!("{entry:?}") };
            assert!(close(centre.x, 11.5), "{entry:?}");
            assert_eq!(((centre.z - 0.5).round() as i32) % 12, 3, "{entry:?}");
        }
        for c in &decor.colliders {
            assert!(close(c.size.x, PILASTER_DEPTH) && close(c.size.z, 0.42), "{c:?}");
        }
    }

    #[test]
    fn a_hall_one_or_twelve_cells_wide_gets_no_arch() {
        for (width, arches) in [(1, 0), (2, 4), (11, 4), (12, 0), (13, 0)] {
            let decor = plain(&hall_along_x(width));
            assert_eq!(decor.log.len(), arches, "width {width}");
            assert_eq!(decor.colliders.len(), 14, "width {width}: the pilasters stand either way");
        }
    }

    #[test]
    fn nothing_stands_beside_a_room_cell_or_a_ramp_cell() {
        let open = hall_along_x(3);
        let all = |_: i32, _: i32| true;
        let none = |_: i32, _: i32| false;
        assert_eq!(decor_of(&open, false, &all, &none), Decor::default());
        assert_eq!(decor_of(&open, false, &none, &all), Decor::default());
        // a room along the near wall only: the far wall keeps its pilasters, and no arch springs (the +z wall has none)
        let near_row = |_: i32, z: i32| z == 10;
        let decor = decor_of(&open, false, &near_row, &none);
        assert_eq!(decor.colliders.len(), 7);
        assert!(decor.log.is_empty());
    }

    #[test]
    fn a_pilaster_never_flanks_an_opening() {
        // a three-cell stub opens through the near wall at x = 14: the pilaster at x = 15 (and its arch) goes, and the far
        // wall's pilaster at x = 15 stays
        let mut open = hall_along_x(3);
        for row in open.iter_mut().take(10).skip(7) {
            row[14] = true;
        }
        let decor = plain(&open);
        assert_eq!(decor.colliders.len(), 13);
        assert_eq!(decor.log.len(), 3);
        // a gap in the far wall beside x = 27 takes the far pilaster and with it the arch from the near wall
        let mut open = hall_along_x(3);
        open[13][28] = true;
        let decor = plain(&open);
        assert_eq!(decor.colliders.len(), 13);
        assert_eq!(decor.log.len(), 3);
    }

    /// tests/validate_wall_themes.gd `_check_arch_log` (165-200), the hall arch part: centred on the hall under 2 cm,
    /// the span exactly pilaster face to pilaster face, the top under the ceiling.
    #[test]
    fn hall_arches_are_centred_span_the_pilaster_faces_and_stay_under_the_ceiling() {
        for width in 2..=11 {
            let decor = plain(&hall_along_x(width));
            assert_eq!(decor.log.len(), 4, "width {width}");
            for entry in &decor.log {
                let ArchEntry::Hall { centre, hall_centre, span, between, spring, top, ceiling } = *entry else { panic!("{entry:?}") };
                let offset = vec2(centre.x - hall_centre.x, centre.z - hall_centre.z).length();
                assert!(offset < 0.02, "width {width}: {offset} m off the hall centreline");
                assert!((span - between).abs() < 0.02, "width {width}: span {span}, opening {between}");
                assert!(top <= ceiling + 0.001, "width {width}: top {top} over the ceiling {ceiling}");
                assert!(close(ceiling, HEIGHT) && spring < top);
            }
            // the same from the boxes: the voussoirs run from one pilaster's face to the other's and no corner of any box
            // of the chunk is over the ceiling
            let voussoirs: Vec<&ArchBox> = decor.boxes.iter().filter(|b| b.axes[0].y != 0.0 || b.axes[0].z != 0.0).collect();
            assert_eq!(voussoirs.len(), 40, "width {width}");
            for arch in voussoirs.chunks(10) {
                let start = arch[0].centre - arch[0].axes[0] / 1.06 * 0.5;
                let end = arch[9].centre + arch[9].axes[0] / 1.06 * 0.5;
                assert!((start.z - (10.0 + width as f32 - PILASTER_DEPTH)).abs() < 1.0e-3, "width {width}: {start}");
                assert!((end.z - (10.0 + PILASTER_DEPTH)).abs() < 1.0e-3, "width {width}: {end}");
                assert!((start.y - end.y).abs() < 1.0e-3, "width {width}: the arch springs level");
            }
            for b in &decor.boxes {
                for corner in b.corners() {
                    assert!(corner.y <= HEIGHT + 0.001 && corner.y >= -0.001, "width {width}: {b:?}");
                }
            }
        }
    }

    /// `_check_arch_log`, the sewer part: half a C from the wall to the ceiling, never down over the walkway or past
    /// its far edge.
    #[test]
    fn sewer_ribs_meet_the_ceiling_and_stay_over_their_half_of_the_walkway() {
        for width in 2..=11 {
            let decor = decor_of(&hall_along_x(width), true, &|_, _| false, &|_, _| false);
            assert_eq!(decor.log.len(), 4, "width {width}");
            assert_eq!(decor.boxes.len(), 14 * 3 + 4 * 7, "width {width}");
            for entry in &decor.log {
                let ArchEntry::Haunch { spring, lowest, top, ceiling, reach, width: w, cell, .. } = *entry else { panic!("{entry:?}") };
                assert!((top - ceiling).abs() < 0.001, "the rib does not meet the ceiling");
                assert!(lowest >= spring - 0.2, "the rib hangs {} m below its spring line", spring - lowest);
                assert!(spring >= 1.9, "the rib springs at {spring} m, in the walkway's headroom");
                assert!(reach <= w * 0.5 + 0.001, "the rib reaches {reach} m over a {w}-cell walkway");
                assert_eq!(cell.1, 9);
            }
        }
    }

    #[test]
    fn sewer_brick_is_the_only_wall_that_is_dressed() {
        let open = hall_along_x(3);
        let is_open = |x: i32, z: i32| open[z as usize][x as usize];
        // the far side (row 13) is a channel, closed to the walk grid but not brick: no pilaster stands on it, and the
        // rib from the brick wall still springs (the channel's edge is the far side of the walkway)
        let brick = |_: i32, z: i32| z != 13;
        let view = ChunkView { size: SIZE, sewer: true, open: &is_open, brick: &brick, room: &|_, _| false, ramp: &|_, _| false, height: &|_, _| 4.0 };
        let decor = chunk_decor(&view);
        assert_eq!(decor.colliders.len(), 7);
        assert_eq!(decor.log.len(), 4);
    }

    #[test]
    fn wall_face_agrees_with_the_chunk_pass_and_leaves_open_cells_alone() {
        let open = hall_along_x(3);
        let is_open = |x: i32, z: i32| open[z as usize][x as usize];
        let view = ChunkView { size: SIZE, sewer: false, open: &is_open, brick: &|_, _| true, room: &|_, _| false, ramp: &|_, _| false, height: &|_, _| HEIGHT };
        let mut out = Decor::default();
        // an open cell, a cell outside the chunk and a wall cell that is not a pilaster column add nothing
        wall_face(&view, 3, 10, (0, 1), &mut out);
        wall_face(&view, 3, -1, (0, 1), &mut out);
        wall_face(&view, 4, 9, (0, 1), &mut out);
        assert_eq!(out, Decor::default());
        wall_face(&view, 3, 9, (0, 1), &mut out);
        assert_eq!((out.boxes.len(), out.colliders.len(), out.log.len()), (14, 1, 1));
        assert_eq!(out.boxes[0].material, Material::Wall { cell: (3, 10), height: HEIGHT });
        assert_eq!(out.boxes[1].material, Material::Trim { cell: (3, 10), height: HEIGHT });
    }

    /// ref_plane_uv.gdshader through `plane_uv`: u runs along the face in metres, v counts storeys from the ceiling, a
    /// flat face takes the ground plane.
    #[test]
    fn plane_uv_runs_along_the_face_and_counts_storeys_from_the_ceiling() {
        let p = vec3(2.25, 1.0, 7.5);
        // a face looking +z: along = UP x n = +x
        assert_eq!(plane_uv(p, Vec3::Z, 0.0, 4.0, 1.0), vec2(2.25, 0.75));
        // a face looking +x: along = -z
        assert_eq!(plane_uv(p, Vec3::X, 0.0, 4.0, 2.0), vec2(-7.5, 1.5));
        assert_eq!(plane_uv(p, Vec3::Y, 0.0, 4.0, 1.0), vec2(2.25, 7.5));
        assert_eq!(plane_uv(p, -Vec3::Y, 0.0, 4.0, 1.0), vec2(2.25, 7.5));
        // a sloped voussoir face keeps the wall's level courses: v depends on the height only
        let sloped = vec3(0.0, 0.6, 0.8);
        let uv = plane_uv(p, sloped, 0.0, 4.0, 1.0);
        assert!(close(uv.x, 2.25) && close(uv.y, 0.75), "{uv}");
        // the floor's y is taken off, and the ceiling row is v = 0
        assert!(close(plane_uv(vec3(0.0, 5.0, 0.0), Vec3::Z, 1.0, 4.0, 1.0).y, 0.0));
    }
}
