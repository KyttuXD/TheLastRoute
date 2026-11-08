//! The floor's static geometry: floors and walls in each room region's own pictures (halls in
//! the maze set), baseboard and brass rail trims, ceilings, and
//! the room props: each type's Blockbench model (assets/props/<set>/*.bbmodel, one of the variants the prop data
//! names). Lights: the switchable room lights and the lamp props' own lights.

use super::builder::Builder;
use super::door;
use super::prop_scale;
use super::lamp_flicker::{self, Tint};
use super::shader::{Flicker, Lamp};
use super::seams;
use super::stairs;
use super::surfaces;
use super::textures::{self, TexRef, TextureChoice};
use crate::shared::assets::Assets;
use crate::config::WALL_HEIGHT;
use crate::content::props::{stable_index, visual_y};
use crate::content::regions::{RegionDef, RegionSafeLight};
use crate::content::Content;
use crate::world::facility::{Cell, DoorDraw, Prop, Stair};
use crate::world::inferno::InfernoChunk;
use crate::world::inferno_terrain::Terrain;
use crate::world::infinite::{ChunkKey, CHUNK, INFERNO_LEVEL, WINDOW};
use crate::world::route::sewer::core::{Grid, LEVEL as SEWER_LEVEL, LOWER_LEVEL as SEWER_LOWER_LEVEL, WATER};
use crate::world::{region_texture, Facility, TEXTURE_SETS};
use macroquad::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use crate::shared::clock::Instant;

/// Test only: every frame behaves as a slow one (a web build on a slow machine), so a part build overruns the frame budget
/// and each rebuild makes just the one part it always may.
#[cfg(test)]
thread_local! {
    pub(crate) static SLOW_FRAMES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Default)]
pub struct WorldMeshes {
    pub meshes: Vec<Mesh>,
    /// Each of `meshes`' box over its vertices (min, max), kept in step with it, so the scene can skip a mesh that
    /// lies wholly in the fog.
    pub bounds: Vec<(Vec3, Vec3)>,
    pub lamps: Vec<Lamp>,
    /// The room doors of the floor, drawn each frame apart from `meshes` (which hold them shut) so their leaves swing.
    pub doors: Vec<DoorInstance>,
    /// The Infinite window's geometry by chunk part, kept while a part's chunk stays in (or just left) the window.
    parts: HashMap<PartKey, Part>,
    /// The part each of `meshes` was taken from (None for the safe pad, built fresh each time).
    drawn: Vec<Option<PartKey>>,
    /// The window origin the drawn vertices (`meshes`, `bounds`, `doors`) are placed for. A recentre moves the window
    /// origin before the new parts are built; `follow_origin` keeps the stale drawn set registered to it meanwhile.
    drawn_origin: (i32, i32),
    builds: u64,
    /// Window parts the last `rebuild` built, and parts it found already held (kept across a recentre, only moved).
    last_built: usize,
    last_kept: usize,
    /// Window parts the last rebuild left for a later frame (the frame budget ran out); see `has_pending`. While any
    /// are pending and something is drawn, the drawn set stays as it was, so the floor shows no holes.
    pending: usize,
    /// The window frame the last window rebuild was for: a jump from it (a level change or a teleport) builds in one step.
    last_frame: Option<ChunkFrame>,
    /// The level these meshes are of (set from the facility in every rebuild). An instance keeps its own level: the
    /// active/neighbour swap is `relabel`'s, so a rebuild never sees a level change.
    pub level: i32,
    /// How far above the active level's floor the vertices are placed (0 for the active level, +-STOREY for a neighbour).
    pub dy: f32,
}

/// A stairwell's opening seen from its upper level: the hole cells, their centre and the level that holds the hole.
#[derive(Clone, Debug)]
pub struct Opening {
    pub cells: Vec<Cell>,
    pub centre: (f32, f32),
    pub upper_level: i32,
}

/// The openings of the stairs of `f`, the UPPER level's facility (one per stair with dir < 0, ladder hatches among
/// them) and one per open drop (a connected patch of the falls void on level -1, `Facility::drop_cells`), so the level
/// below is built near the hole it is seen through. A neighbour below the active level is scoped with the openings of
/// the active facility; one above, with those of its own.
pub fn openings(f: &Facility) -> Vec<Opening> {
    let mut out: Vec<Opening> = f
        .stairs
        .iter()
        .filter(|s| s.dir < 0)
        .filter_map(|s| {
            let cells = s.well_cells();
            if cells.is_empty() {
                return None;
            }
            Some(opening_of(cells, f.level))
        })
        .collect();
    out.extend(drop_patches(f.drop_cells().to_vec()).into_iter().map(|cells| opening_of(cells, f.level)));
    out
}

/// An opening over `cells`, centred on their mean.
fn opening_of(cells: Vec<Cell>, upper_level: i32) -> Opening {
    let n = cells.len().max(1) as f32;
    let sum = cells.iter().map(|c| c.center()).fold((0.0, 0.0), |a, c| (a.0 + c.0, a.1 + c.1));
    Opening { centre: (sum.0 / n, sum.1 / n), cells, upper_level }
}

/// `cells` split into 4-connected patches, so two voids far apart are two openings and not one centred between them.
fn drop_patches(cells: Vec<Cell>) -> Vec<Vec<Cell>> {
    let mut left: HashSet<(i32, i32)> = cells.iter().map(|c| (c.x, c.y)).collect();
    let mut order: Vec<(i32, i32)> = left.iter().copied().collect();
    order.sort();
    let mut out = Vec::new();
    for start in order {
        if !left.remove(&start) {
            continue;
        }
        let mut patch = vec![Cell::new(start.0, start.1)];
        let mut i = 0;
        while i < patch.len() {
            let c = patch[i];
            i += 1;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if left.remove(&(c.x + dx, c.y + dy)) {
                    patch.push(Cell::new(c.x + dx, c.y + dy));
                }
            }
        }
        out.push(patch);
    }
    out
}

/// What a neighbour level builds: where it stands, the openings it is seen through and the data limits.
#[derive(Clone, Debug)]
pub struct NeighbourScope {
    pub dy: f32,
    pub openings: Vec<Opening>,
    /// Only chunk parts this close (metres) to an opening are built.
    pub radius: f32,
    pub max_parts: usize,
    pub chunks_per_frame: usize,
    /// The eye in the window's cell frame (x, z): the parts nearest it are kept first when `max_parts` cuts the set.
    pub eye: (f32, f32),
}

/// A room door to draw each frame: its threshold cell (the key of `Facility::door_open_amount`), the model it uses and
/// where it stands.
pub struct DoorInstance {
    pub cell: Cell,
    pub model: String,
    pub xf: Mat4,
    /// How the model is fitted to its doorway (the posts' slide, the leaves' stretch).
    pub fit: door::DoorFit,
}

/// Whether `c` lies in one of the half-open boxes (x0, y0, x1, y1): the same test as a part's `fixture` filter.
fn cell_in_rects(c: Cell, rects: &[(i32, i32, i32, i32)]) -> bool {
    rects.iter().any(|&(x0, y0, x1, y1)| c.x >= x0 && c.y >= y0 && c.x < x1 && c.y < y1)
}

/// The items whose cell lies in a built part's fixture box. A room door's frame is built with its chunk part's
/// props, so only those doors may have their leaves drawn.
fn keep_in_rects<T>(items: Vec<T>, cell: impl Fn(&T) -> Cell, rects: &[(i32, i32, i32, i32)]) -> Vec<T> {
    items.into_iter().filter(|i| cell_in_rects(cell(i), rects)).collect()
}

/// `doors` cut down to the ones whose frames are in the built parts `rects`. A floor built whole publishes all of
/// its doors, so this is for the streamed window only.
fn doors_in_rects(doors: Vec<DoorInstance>, rects: &[(i32, i32, i32, i32)]) -> Vec<DoorInstance> {
    keep_in_rects(doors, |d| d.cell, rects)
}

/// Where the Infinite route's 3x3 window stands: its level and its centre chunk.
#[derive(Clone, Copy, Debug)]
pub struct ChunkFrame {
    pub level: i32,
    pub center: (i32, i32),
}

/// One slice of a window chunk's geometry: the cells and rooms that read the same sides past the chunk's edge
/// (`needs`, a bit per side), and which of those sides had a loaded chunk (`open`). The slice is the same
/// wherever the window stands as long as `open` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PartKey {
    level: i32,
    cx: i32,
    cy: i32,
    needs: u8,
    open: u8,
}

struct Part {
    meshes: Vec<Mesh>,
    /// The window origin the vertices are placed for.
    origin: (i32, i32),
    /// The height offset the vertices are placed for (see `WorldMeshes::dy`).
    dy: f32,
    used: u64,
}

const WEST: u8 = 1;
const EAST: u8 = 2;
const NORTH: u8 = 4;
const SOUTH: u8 = 8;

/// How far along an axis a cell's geometry reads other cells: the hall runner's six-cell scan, plus one.
const REACH: i32 = 7;

/// Lifts the safe pad over the floor it covers.
const PAD_LIFT: f32 = 0.006;

/// The sides past its chunk a cell (chunk-local `lx`, `ly`) reads.
fn side_needs(lx: i32, ly: i32) -> u8 {
    let mut n = 0;
    if lx < REACH {
        n |= WEST;
    }
    if lx >= CHUNK - REACH {
        n |= EAST;
    }
    if ly < REACH {
        n |= NORTH;
    }
    if ly >= CHUNK - REACH {
        n |= SOUTH;
    }
    n
}

/// The sides of window slot (`sx`, `sy`) that have a loaded chunk.
fn slot_open(sx: i32, sy: i32) -> u8 {
    let mut n = 0;
    if sx > 0 {
        n |= WEST;
    }
    if sx < WINDOW - 1 {
        n |= EAST;
    }
    if sy > 0 {
        n |= NORTH;
    }
    if sy < WINDOW - 1 {
        n |= SOUTH;
    }
    n
}

fn slot_key(frame: ChunkFrame, sx: i32, sy: i32, needs: u8) -> PartKey {
    PartKey {
        level: frame.level,
        cx: frame.center.0 - 1 + sx,
        cy: frame.center.1 - 1 + sy,
        needs,
        open: slot_open(sx, sy) & needs,
    }
}

/// What one build draws: floor cells, the rooms it draws, the props and light fixtures inside
/// `fixtures` (x0, y0, x1, y1, half-open), and whether the safe pad replaces the floor under it.
struct Pick {
    cells: Vec<Cell>,
    rooms: Vec<usize>,
    fixtures: Option<(i32, i32, i32, i32)>,
    pad: bool,
}

/// The window's floor cut into chunk parts. Each chunk's fixtures go with its part that reads no other chunk.
fn window_parts(f: &Facility, frame: ChunkFrame) -> Vec<(PartKey, Pick)> {
    let mut parts: Vec<(PartKey, Pick)> = vec![];
    let mut index: HashMap<PartKey, usize> = HashMap::new();
    let slot = |v: i32| v.div_euclid(CHUNK).clamp(0, WINDOW - 1);
    let mut at = |sx: i32, sy: i32, needs: u8, parts: &mut Vec<(PartKey, Pick)>| {
        let key = slot_key(frame, sx, sy, needs);
        *index.entry(key).or_insert_with(|| {
            let (x0, y0) = (sx * CHUNK, sy * CHUNK);
            parts.push((
                key,
                Pick {
                    cells: vec![],
                    rooms: vec![],
                    fixtures: (needs == 0).then_some((x0, y0, x0 + CHUNK, y0 + CHUNK)),
                    pad: false,
                },
            ));
            parts.len() - 1
        })
    };
    for sy in 0..WINDOW {
        for sx in 0..WINDOW {
            at(sx, sy, 0, &mut parts);
        }
    }
    for c in drawn_cells(f) {
        let (sx, sy) = (slot(c.x), slot(c.y));
        let i = at(sx, sy, side_needs(c.x - sx * CHUNK, c.y - sy * CHUNK), &mut parts);
        parts[i].1.cells.push(c);
    }
    for (ri, r) in f.rooms.iter().enumerate() {
        let (sx, sy) = (slot(r.x), slot(r.y));
        let (x0, y0) = (sx * CHUNK, sy * CHUNK);
        // the room's trims and props read one cell past the room on every side
        let needs = side_needs(r.x - 1 - x0, r.y - 1 - y0) | side_needs(r.x + r.w - x0, r.y + r.h - y0);
        let i = at(sx, sy, needs, &mut parts);
        parts[i].1.rooms.push(ri);
    }
    parts
}

/// Moves a part's vertices from the window origin they were placed for to `to`.
fn reframe(meshes: &mut [Mesh], from: (i32, i32), to: (i32, i32)) {
    let (dx, dz) = ((from.0 - to.0) as f32, (from.1 - to.1) as f32);
    for m in meshes {
        for v in &mut m.vertices {
            v.position.x += dx;
            v.position.z += dz;
        }
    }
}

/// Moves every vertex of `meshes` up by `dy`.
fn lift(meshes: &mut [Mesh], dy: f32) {
    if dy == 0.0 {
        return;
    }
    for m in meshes {
        for v in &mut m.vertices {
            v.position.y += dy;
        }
    }
}

/// Whether a move of the window from `from` to `to` is a jump (a centre more than `jump_steps` chunks away) rather than
/// a step. streamed_facility_view_3d.gd 726-727. A level change is not a jump: a `WorldMeshes` keeps its own level and
/// the level swap is `relabel`'s, so the meshes never see one.
fn is_jump(from: ChunkFrame, to: ChunkFrame, jump_steps: i32) -> bool {
    (from.center.0 - to.center.0).abs() > jump_steps
        || (from.center.1 - to.center.1).abs() > jump_steps
}

/// Whether the window's parts are all built in this one rebuild: a jump from the last window, or coming back onto the
/// window after a floor that was not on it (`builds_before` > 0). The first load of a run streams.
fn builds_in_one_step(last: Option<ChunkFrame>, next: ChunkFrame, builds_before: u64, jump_steps: i32) -> bool {
    match last {
        Some(last) => is_jump(last, next, jump_steps),
        None => builds_before > 0,
    }
}

/// The wanted parts that are built when the window is drawn. The frame budget can leave some parts for a later
/// frame (the caller rebuilds again while `has_pending`), so a first load on a slow machine draws what is built
/// instead of asking for a part that was never built.
fn drawable_keys<T>(wanted: &[(PartKey, T)], built: impl Fn(&PartKey) -> bool) -> Vec<PartKey> {
    wanted.iter().map(|(k, _)| *k).filter(|k| built(k)).collect()
}

impl WorldMeshes {
    /// Rebuilds the meshes for `f`. On the Infinite window (`frame`) only the chunk parts that are new, or whose
    /// neighbour chunks changed, are built; the rest are moved to the new window origin. Any other floor is
    /// built whole.
    pub fn rebuild(
        &mut self,
        f: &Facility,
        lights_on: &[bool],
        frame: Option<ChunkFrame>,
        assets: &Assets,
        content: &Content,
    ) {
        // defensive: meshes of another level are not this level's (the swap is `relabel`'s)
        if self.level != f.level {
            if !self.parts.is_empty() || !self.meshes.is_empty() {
                self.clear_geometry();
            }
            self.level = f.level;
        }
        self.lamps = lamps(f, lights_on, content);
        for l in &mut self.lamps {
            l.pos.y += self.dy;
        }
        // The hinge leaves are drawn each frame apart from the meshes, whose frames are built part by part. The
        // leaves therefore follow the meshes: they are published only with the drawn set (below), and on a
        // streamed floor only for the parts that are built, so a leaf never stands without its frame.
        let doors = door_instances(f, assets, content);
        let size = CHUNK * WINDOW;
        let frame = frame.filter(|fr| {
            f.cols == size
                && f.rows == size
                && f.origin == ((fr.center.0 - 1) * CHUNK, (fr.center.1 - 1) * CHUNK)
        });
        let mut choices = HashMap::new();
        let Some(frame) = frame else {
            self.parts.clear();
            self.last_frame = None;
            let pick = Pick {
                cells: drawn_cells(f),
                rooms: (0..f.rooms.len()).collect(),
                fixtures: Some((0, 0, f.cols, f.rows)),
                pad: true,
            };
            // one texture's meshes side by side, so the batcher joins them into one draw call
            let built = build_meshes(f, &pick, &mut choices, assets, content).into_iter().map(|m| (None, m)).collect();
            let (drawn, meshes) = group_by_texture(built);
            self.meshes = meshes;
            self.drawn = drawn;
            lift(&mut self.meshes, self.dy);
            self.doors = doors;
            self.lift_doors();
            self.drawn_origin = f.origin;
            self.bounds = mesh_bounds(&self.meshes);
            return;
        };
        let table = stream_table();
        // the quality governor's share of the build budget (1 outside a notch)
        let stream_scale = f64::from(crate::app::quality::scales().stream);
        let started = Instant::now();
        // a level change or a teleport builds its whole window in this one step; a step builds what the budget allows
        let one_step = builds_in_one_step(self.last_frame, frame, self.builds, table.jump_steps);
        self.last_frame = Some(frame);
        let build_no = self.builds + 1;
        let wanted = window_parts(f, frame);
        let queued = wanted.iter().filter(|(k, _)| !self.parts.contains_key(k)).count();
        // Phase 1: build the parts still missing (the budget is data: data/render/stream.json). The drawn set is not
        // touched until every wanted part is here, so the old geometry stays on screen while the rest waits for later
        // frames, and no hole is drawn.
        let mut built = 0;
        let mut pending = 0;
        for (key, pick) in &wanted {
            if let Some(p) = self.parts.get_mut(key) {
                p.used = build_no;
                continue;
            }
            if !one_step && !table.allows_scaled(built, started.elapsed().as_secs_f64() * 1000.0, queued, stream_scale) {
                pending += 1;
                continue;
            }
            built += 1;
            let mut meshes = build_meshes(f, pick, &mut choices, assets, content);
            lift(&mut meshes, self.dy);
            self.parts.insert(*key, Part { meshes, origin: f.origin, dy: self.dy, used: build_no });
        }
        self.pending = pending;
        self.last_built = built;
        self.last_kept = wanted.len() - queued;
        // with something drawn, the drawn set waits for the window to be whole; with nothing drawn yet (the first
        // load) the built parts show as they come
        if pending > 0 && !self.drawn.is_empty() {
            // the window may have been recentred since the drawn set was placed: keep it where the camera now is
            self.follow_origin(f.origin);
            return;
        }
        self.builds = build_no;
        // the leaves of the doors whose frames are in a built part (every wanted part, unless this is the first load)
        let built: Vec<(i32, i32, i32, i32)> = wanted
            .iter()
            .filter(|(k, _)| self.parts.contains_key(k))
            .filter_map(|(_, pick)| pick.fixtures)
            .collect();
        self.doors = doors_in_rects(doors, &built);
        self.lift_doors();
        // Phase 2: the drawn set goes back into the parts it came from, then every wanted part is drawn in order.
        self.return_drawn_to_parts();
        let mut out: Vec<(Option<PartKey>, Mesh)> = vec![];
        for key in drawable_keys(&wanted, |k| self.parts.contains_key(k)) {
            let part = self.parts.get_mut(&key).expect("drawable_keys returns only built parts");
            if part.origin != f.origin {
                reframe(&mut part.meshes, part.origin, f.origin);
                part.origin = f.origin;
            }
            // a part built before a shift is reframed in height the same way
            if part.dy != self.dy {
                lift(&mut part.meshes, self.dy - part.dy);
                part.dy = self.dy;
            }
            out.extend(part.meshes.drain(..).map(|m| (Some(key), m)));
        }
        out.extend(safe_pad(f, assets).into_iter().map(|mut m| {
            lift(std::slice::from_mut(&mut m), self.dy);
            (None, m)
        }));
        // the window's last two builds stay, so a step back or a flight straight back up builds nothing
        let builds = self.builds;
        self.parts.retain(|_, p| p.used + 1 >= builds);
        // one texture's meshes side by side, so the batcher joins them into one draw call
        let (drawn, meshes) = group_by_texture(out);
        self.drawn = drawn;
        self.meshes = meshes;
        self.drawn_origin = f.origin;
        self.bounds = mesh_bounds(&self.meshes);
    }

    /// Puts the drawn meshes back into the parts they were taken from. A part whose meshes were drained into the drawn
    /// set takes the drawn set's origin, which `follow_origin` may have moved since, so the later reframe of the part
    /// starts from where its vertices really are.
    fn return_drawn_to_parts(&mut self) {
        for (m, key) in std::mem::take(&mut self.meshes).into_iter().zip(std::mem::take(&mut self.drawn)) {
            if let Some(p) = key.and_then(|k| self.parts.get_mut(&k)) {
                if p.meshes.is_empty() {
                    p.origin = self.drawn_origin;
                }
                p.meshes.push(m);
            }
        }
    }

    /// Moves the drawn set (vertices, bounds, door leaves) from the window origin it was placed for to `to`, when a
    /// recentre moved the window while new parts are still being built. Without it the stale set keeps the old
    /// origin's coordinates while the camera, the runner and the hunters are already in the new ones, so the walls
    /// stand a whole chunk off for the frames the build takes.
    fn follow_origin(&mut self, to: (i32, i32)) {
        let from = self.drawn_origin;
        if from == to {
            return;
        }
        reframe(&mut self.meshes, from, to);
        let (dx, dz) = ((from.0 - to.0) as f32, (from.1 - to.1) as f32);
        for (lo, hi) in &mut self.bounds {
            // an empty mesh keeps its inverted box
            if lo.x <= hi.x {
                lo.x += dx;
                hi.x += dx;
                lo.z += dz;
                hi.z += dz;
            }
        }
        let by = Mat4::from_translation(vec3(dx, 0.0, dz));
        for d in &mut self.doors {
            d.xf = by * d.xf;
            d.cell = Cell::new(d.cell.x + (from.0 - to.0), d.cell.y + (from.1 - to.1));
        }
        self.drawn_origin = to;
    }

    /// Whether the last rebuild left window parts unbuilt: the caller calls `rebuild` again (same floor) until
    /// this is false, so a window change is spread over frames instead of one long frame.
    pub fn has_pending(&self) -> bool {
        self.pending > 0
    }

    /// Drops everything built (parts, the drawn set, doors); the level and height offset stay.
    fn clear_geometry(&mut self) {
        self.parts.clear();
        self.meshes.clear();
        self.drawn.clear();
        self.bounds.clear();
        self.doors.clear();
        self.last_frame = None;
        self.pending = 0;
    }

    /// Raises the door leaves (built at height 0) by this instance's `dy`.
    fn lift_doors(&mut self) {
        if self.dy != 0.0 {
            let up = Mat4::from_translation(vec3(0.0, self.dy, 0.0));
            for d in &mut self.doors {
                d.xf = up * d.xf;
            }
        }
    }

    /// Moves everything this instance holds up by `delta` metres: the drawn vertices, the cached parts (so a part
    /// reused after a level swap stands at the right height), the bounds, the lamps and the door leaves. `dy` follows.
    pub fn shift_y(&mut self, delta: f32) {
        if delta == 0.0 {
            return;
        }
        lift(&mut self.meshes, delta);
        for p in self.parts.values_mut() {
            lift(&mut p.meshes, delta);
            p.dy += delta;
        }
        for (lo, hi) in &mut self.bounds {
            // an empty mesh keeps its inverted box
            if lo.y <= hi.y {
                lo.y += delta;
                hi.y += delta;
            }
        }
        for l in &mut self.lamps {
            l.pos.y += delta;
        }
        let up = Mat4::from_translation(vec3(0.0, delta, 0.0));
        for d in &mut self.doors {
            d.xf = up * d.xf;
        }
        self.dy += delta;
    }

    /// The triangles currently drawn.
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(|m| m.indices.len() / 3).sum()
    }

    /// (built, kept) window parts of the last `rebuild`: after a recentre only the new chunks are built.
    pub fn last_rebuild_parts(&self) -> (usize, usize) {
        (self.last_built, self.last_kept)
    }

    /// How many window parts this instance holds (cached parts; the drawn ones stay in `parts` as empty shells).
    pub fn held_parts(&self) -> usize {
        self.parts.len()
    }

    /// Builds the neighbour level `f` (a window sharing the active window's centre and origin) for drawing at
    /// `scope.dy`: only the chunk parts within `scope.radius` of an opening, at most `scope.max_parts` of them, through
    /// the stream budget with `scope.chunks_per_frame`. Lamps are all on and offset by dy, doors are left empty, no
    /// safe pad. The drawn set swaps only once every wanted part is built (the same rule as `rebuild`); call again
    /// while `has_pending`.
    pub fn rebuild_neighbour(
        &mut self,
        f: &Facility,
        frame: ChunkFrame,
        scope: &NeighbourScope,
        assets: &Assets,
        content: &Content,
    ) {
        if self.level != f.level {
            if !self.parts.is_empty() || !self.meshes.is_empty() {
                self.clear_geometry();
            }
            self.level = f.level;
        }
        if self.dy != scope.dy {
            self.shift_y(scope.dy - self.dy);
        }
        self.lamps = lamps(f, &[], content);
        for l in &mut self.lamps {
            l.pos.y += self.dy;
        }
        self.doors.clear();
        let size = CHUNK * WINDOW;
        let ours = f.cols == size && f.rows == size && f.origin == ((frame.center.0 - 1) * CHUNK, (frame.center.1 - 1) * CHUNK);
        if !ours {
            self.clear_geometry();
            return;
        }
        self.last_frame = Some(frame);
        let wanted = neighbour_wanted_from(window_parts(f, frame), frame, &scope.openings, scope.radius, scope.max_parts, Some(scope.eye));
        // the hard cap: nothing outside the wanted set is kept
        self.parts.retain(|k, _| wanted.iter().any(|(w, _)| w == k));
        let table = stream_table();
        let started = Instant::now();
        let build_no = self.builds + 1;
        let queued = wanted.iter().filter(|(k, _)| !self.parts.contains_key(k)).count();
        let stream_scale = f64::from(crate::app::quality::scales().stream);
        let mut choices = HashMap::new();
        let (mut built, mut pending) = (0, 0);
        for (key, pick) in &wanted {
            if let Some(p) = self.parts.get_mut(key) {
                p.used = build_no;
                continue;
            }
            if !neighbour_allows(table, scope.chunks_per_frame, built, started.elapsed().as_secs_f64() * 1000.0, queued, stream_scale) {
                pending += 1;
                continue;
            }
            built += 1;
            let mut meshes = build_meshes(f, pick, &mut choices, assets, content);
            lift(&mut meshes, self.dy);
            self.parts.insert(*key, Part { meshes, origin: f.origin, dy: self.dy, used: build_no });
        }
        self.pending = pending;
        if pending > 0 && !self.drawn.is_empty() {
            self.follow_origin(f.origin);
            return;
        }
        self.builds = build_no;
        self.return_drawn_to_parts();
        let mut out: Vec<(Option<PartKey>, Mesh)> = vec![];
        for key in drawable_keys(&wanted, |k| self.parts.contains_key(k)) {
            let part = self.parts.get_mut(&key).expect("drawable_keys returns only built parts");
            if part.origin != f.origin {
                reframe(&mut part.meshes, part.origin, f.origin);
                part.origin = f.origin;
            }
            if part.dy != self.dy {
                lift(&mut part.meshes, self.dy - part.dy);
                part.dy = self.dy;
            }
            out.extend(part.meshes.drain(..).map(|m| (Some(key), m)));
        }
        let (drawn, meshes) = group_by_texture(out);
        self.drawn = drawn;
        self.meshes = meshes;
        self.drawn_origin = f.origin;
        self.bounds = mesh_bounds(&self.meshes);
    }
}

/// Whether one more neighbour part may be built after `built` this frame (the first always may). `scale` is the quality
/// governor's stream factor, cutting the time budget as it does for the streamed window.
fn neighbour_allows(table: &StreamBudget, per_frame: usize, built: usize, elapsed_ms: f64, queued: usize, scale: f64) -> bool {
    #[cfg(test)]
    if SLOW_FRAMES.with(|s| s.get()) {
        return built == 0;
    }
    built == 0 || (built < per_frame && elapsed_ms < table.budget_ms(queued) * scale.clamp(0.0, 1.0))
}

/// The meshes with one texture's side by side (so the batcher joins them), and the part each came from. Blended meshes
/// follow the opaque ones in the order they were built (render/mesh_batch.rs `draw_order`).
fn group_by_texture(out: Vec<(Option<PartKey>, Mesh)>) -> (Vec<Option<PartKey>>, Vec<Mesh>) {
    super::mesh_batch::group_for_batching(out).into_iter().unzip()
}

/// The distance from point `p` to the box (x0, z0, x1, z1); 0 inside it.
fn rect_distance(p: (f32, f32), r: (f32, f32, f32, f32)) -> f32 {
    let dx = (r.0 - p.0).max(0.0).max(p.0 - r.2);
    let dz = (r.1 - p.1).max(0.0).max(p.1 - r.3);
    (dx * dx + dz * dz).sqrt()
}

/// The window parts a neighbour draws with no eye to rank them by: those holding cells or rooms whose chunk lies within
/// `radius` metres of an opening centre, nearest an opening first, at most `max_parts`.
#[cfg(test)]
fn neighbour_wanted(
    parts: Vec<(PartKey, Pick)>,
    frame: ChunkFrame,
    openings: &[Opening],
    radius: f32,
    max_parts: usize,
) -> Vec<(PartKey, Pick)> {
    neighbour_wanted_from(parts, frame, openings, radius, max_parts, None)
}

/// `neighbour_wanted`, with the eye given: the parts that hold a cell nearest the eye come first, so when `max_parts`
/// cuts the set it is the far openings' parts that go and the shaft under the eye (its hatch's cells, the lower
/// floor round it) always stays. Without an eye the parts are ranked by their distance to an opening.
fn neighbour_wanted_from(
    parts: Vec<(PartKey, Pick)>,
    frame: ChunkFrame,
    openings: &[Opening],
    radius: f32,
    max_parts: usize,
    eye: Option<(f32, f32)>,
) -> Vec<(PartKey, Pick)> {
    let mut near: Vec<(f32, (PartKey, Pick))> = parts
        .into_iter()
        .filter(|(_, p)| !p.cells.is_empty() || !p.rooms.is_empty())
        .filter_map(|(k, p)| {
            let x0 = ((k.cx - (frame.center.0 - 1)) * CHUNK) as f32;
            let z0 = ((k.cy - (frame.center.1 - 1)) * CHUNK) as f32;
            let rect = (x0, z0, x0 + CHUNK as f32, z0 + CHUNK as f32);
            let d = openings.iter().map(|o| rect_distance(o.centre, rect)).fold(f32::MAX, f32::min);
            if d > radius {
                return None;
            }
            let rank = match eye {
                Some(e) => part_eye_distance(&p, rect, e),
                None => d,
            };
            Some((rank, (k, p)))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(max_parts);
    near.into_iter().map(|(_, kp)| kp).collect()
}

/// The distance from the eye to the nearest cell a part draws (its chunk's rectangle when it draws no cell, rooms only).
fn part_eye_distance(p: &Pick, rect: (f32, f32, f32, f32), eye: (f32, f32)) -> f32 {
    let cell = p
        .cells
        .iter()
        .map(|c| {
            let (x, z) = c.center();
            ((x - eye.0).powi(2) + (z - eye.1).powi(2)).sqrt()
        })
        .fold(f32::MAX, f32::min);
    if p.cells.is_empty() {
        rect_distance(eye, rect)
    } else {
        cell
    }
}

/// Path of the streaming budget under data/ (the embedded file's name).
pub const STREAM_TABLE_PATH: &str = "render/stream.json";

/// The streaming budget of the window's chunk builds (data/render/stream.json). Godot's values: STREAM_BUDGET_MS,
/// STREAM_BUDGET_PER_QUEUED_MS, STREAM_BUDGET_MAX_MS (streamed_facility_view_3d.gd 36-38) and
/// STARTUP_CHUNKS_PER_FRAME (line 1370).
pub struct StreamBudget {
    pub frame_budget_ms: f64,
    pub per_queued_ms: f64,
    pub max_budget_ms: f64,
    pub chunks_per_frame: usize,
    /// A window centre more than this many chunks from the last one in a frame is a jump (a teleport): built in one step.
    pub jump_steps: i32,
    /// A neighbour level builds at most this many chunk parts in a frame.
    pub neighbour_chunks_per_frame: usize,
    /// While the eye is within the prefetch distance of an opening of the active floor a neighbour builds this many
    /// chunk parts in a frame instead, so the well never shows an unbuilt pit for long.
    pub neighbour_near_chunks_per_frame: usize,
    /// A neighbour level holds at most this many chunk parts.
    pub neighbour_max_parts: usize,
    /// A neighbour level builds only chunk parts this many metres (or less) from a stairwell opening.
    pub neighbour_cull_radius: f32,
}

impl StreamBudget {
    /// Parses `{ "frame_budget_ms", "per_queued_ms", "max_budget_ms", "chunks_per_frame", "jump_steps", "neighbour_chunks_per_frame", "neighbour_near_chunks_per_frame", "neighbour_max_parts", "neighbour_cull_radius" }`; the times
    /// must be positive, chunks_per_frame a whole number of at least 1 and jump_steps one of at least 1.
    pub fn parse(text: &str) -> Result<StreamBudget, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let num = |key: &str| -> Result<f64, String> {
            value
                .get(key)
                .and_then(|v| v.as_f64())
                .filter(|v| *v > 0.0)
                .ok_or_else(|| format!("{key}: must be a positive number"))
        };
        let chunks = value
            .get("chunks_per_frame")
            .and_then(|v| v.as_u64())
            .filter(|v| *v >= 1)
            .ok_or_else(|| "chunks_per_frame: must be a whole number of at least 1".to_string())?;
        let jump_steps = value
            .get("jump_steps")
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok())
            .filter(|v| *v >= 1)
            .ok_or_else(|| "jump_steps: must be a whole number of at least 1".to_string())?;
        let whole = |key: &str| -> Result<usize, String> {
            value
                .get(key)
                .and_then(|v| v.as_u64())
                .filter(|v| *v >= 1)
                .map(|v| v as usize)
                .ok_or_else(|| format!("{key}: must be a whole number of at least 1"))
        };
        Ok(StreamBudget {
            frame_budget_ms: num("frame_budget_ms")?,
            per_queued_ms: num("per_queued_ms")?,
            max_budget_ms: num("max_budget_ms")?,
            chunks_per_frame: chunks as usize,
            jump_steps,
            neighbour_chunks_per_frame: whole("neighbour_chunks_per_frame")?,
            neighbour_near_chunks_per_frame: whole("neighbour_near_chunks_per_frame")?,
            neighbour_max_parts: whole("neighbour_max_parts")?,
            neighbour_cull_radius: num("neighbour_cull_radius")? as f32,
        })
    }

    /// The milliseconds a frame may spend on chunk builds with `queued` parts still to build, capped at the max.
    pub fn budget_ms(&self, queued: usize) -> f64 {
        (self.frame_budget_ms + self.per_queued_ms * queued as f64).min(self.max_budget_ms)
    }

    /// Whether one more part may be built after `built` this frame, `elapsed_ms` into the frame. The first part of
    /// a rebuild always builds, so the queue never stalls.
    pub fn allows(&self, built: usize, elapsed_ms: f64, queued: usize) -> bool {
        self.allows_scaled(built, elapsed_ms, queued, 1.0)
    }

    /// [`Self::allows`] with the frame's time budget cut to `scale` of itself (the quality governor's stream factor, 0 to 1;
    /// 1 is exactly `allows`). The first part of a rebuild still always builds, so a smaller budget only slows the fill.
    pub fn allows_scaled(&self, built: usize, elapsed_ms: f64, queued: usize, scale: f64) -> bool {
        #[cfg(test)]
        if SLOW_FRAMES.with(|s| s.get()) {
            return built == 0;
        }
        built == 0 || (built < self.chunks_per_frame && elapsed_ms < self.budget_ms(queued) * scale.clamp(0.0, 1.0))
    }
}

/// The table compiled into the executable. A table that fails to parse is reported on stderr and the Standard
/// values are used.
pub fn stream_table() -> &'static StreamBudget {
    static TABLE: OnceLock<StreamBudget> = OnceLock::new();
    TABLE.get_or_init(|| {
        let text = crate::content::embedded::data_under(STREAM_TABLE_PATH)
            .next()
            .map(|(_, text)| text)
            .unwrap_or("{}");
        StreamBudget::parse(text).unwrap_or_else(|e| {
            eprintln!("{STREAM_TABLE_PATH}: {e}");
            StreamBudget {
                frame_budget_ms: 4.0,
                per_queued_ms: 1.0,
                max_budget_ms: 9.0,
                chunks_per_frame: 4,
                jump_steps: 1,
                neighbour_chunks_per_frame: 2,
                neighbour_near_chunks_per_frame: 4,
                neighbour_max_parts: 12,
                neighbour_cull_radius: 14.0,
            }
        })
    })
}

/// Each mesh's box over its vertices (min, max). An empty mesh gets an inverted box, which the scene's cull treats as far.
fn mesh_bounds(meshes: &[Mesh]) -> Vec<(Vec3, Vec3)> {
    meshes
        .iter()
        .map(|m| {
            m.vertices.iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), v| (lo.min(v.position), hi.max(v.position)),
            )
        })
        .collect()
}

/// The safe-zone pad over the window's floor (the chunk parts draw the floor under it as it is without one).
fn safe_pad(f: &Facility, assets: &Assets) -> Vec<Mesh> {
    let Some(sz) = f.safe_zone else { return vec![] };
    let steps = stair_steps(f);
    let mut surf: HashMap<(&'static str, &'static str), Builder> = HashMap::new();
    for y in sz.y - 3..=sz.y + 3 {
        for x in sz.x - 3..=sz.x + 3 {
            if x < 1 || y < 1 || x >= f.cols - 1 || y >= f.rows - 1 || f.is_wall_cell(x, y) {
                continue;
            }
            if steps.contains_key(&(x, y)) {
                continue;
            }
            let Some(k) = textures::sets::safe_pad_kind(x - sz.x, y - sz.y) else {
                continue;
            };
            let (x, z) = (x as f32, y as f32);
            painted(&mut surf, TexRef::new("floor", k), assets.floor(room_set(f, x as i32, z as i32))).quad(
                [
                    vec3(x, PAD_LIFT, z),
                    vec3(x + 1., PAD_LIFT, z),
                    vec3(x + 1., PAD_LIFT, z + 1.),
                    vec3(x, PAD_LIFT, z + 1.),
                ],
                UV1,
                vec3(0., 1., 0.),
                WHITE,
                0.0,
            );
        }
    }
    surf.into_values().flat_map(Builder::finish).collect()
}

const UV1: [Vec2; 4] = [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)];

/// Whether a stair record pushes a lamp: only the lower level's (dir > 0) record owns the flight or shaft; the upper level's
/// (dir < 0) hatch adds none, matching `stairs::lamp`.
fn owns_lamp(s: &Stair) -> bool {
    s.dir > 0
}

/// The room lights plus each lamp prop's `light` from data/props (the original's fixture lights,
/// budget 10 per floor, offset turning with the prop).
///
/// Every room lamp takes its room's tint (`lamp_flicker` `room_tint`: its type, else its region). On a level 0
/// Infinite window about a quarter of the rooms start dark (`room_starts_dark`, Godot's `default_room_light`), and the
/// lamps of a dark room are left out of the list, as a switched-off room's lamps are zeroed.
pub fn lamps(f: &Facility, lights_on: &[bool], content: &Content) -> Vec<Lamp> {
    lamps_with(f, lights_on, content, &dark_rooms(f))
}

fn lamps_with(f: &Facility, lights_on: &[bool], content: &Content, dark: &[bool]) -> Vec<Lamp> {
    let mut out = vec![];
    let table = lamp_flicker::table();
    for (i, l) in f.room_lights.iter().enumerate() {
        if lights_on.get(i).copied().unwrap_or(true) {
            let (x, z) = l.center.center();
            let tint = f.rooms.get(l.room).map_or(Tint::NONE, |r| table.room_tint(&r.region, &r.rtype));
            let pos = vec3(x, fixture_ceiling(f, l.center) - 0.34, z);
            let (hz, depth) = f
                .rooms
                .get(l.room)
                .and_then(|r| lamp_flicker::table().room_flicker(&r.region))
                .unwrap_or((0.0, 0.0));
            out.push(Lamp {
                pos,
                range: l.range * 1.4,
                color: tinted(l.color, tint),
                energy: l.energy * tint.energy,
                flicker: lamp_flicker::make(hz, depth, world_place(f, pos)),
            });
        }
    }
    // the safe zone is a checkpoint: a warm light over its pad, so it reads as a lit area from the halls
    if let Some(s) = f.safe_zone {
        let (x, z) = s.center();
        let light = safe_light_for(f, s.x, s.y);
        out.push(Lamp {
            pos: vec3(x, fixture_ceiling(f, s) - 0.6, z),
            range: 7.0,
            color: Color::new(light.color[0], light.color[1], light.color[2], 1.0),
            energy: light.energy,
            flicker: Flicker::NONE,
        });
    }
    // each stairwell and ladder well carries a light, so its steps and rungs read in the dark. A flight has one light (Godot's
    // stairwell light): the lower level, which owns the flight, lights it; the upper level's record adds none
    out.extend(f.stairs.iter().filter(|s| owns_lamp(s)).map(|s| stairs::lamp(*s, WALL_HEIGHT)));
    let mut budget = lamp_flicker::table().prop_budget(f.level);
    for p in &f.props {
        if budget == 0 {
            break;
        }
        let Some(light) = content.props.get(&p.ptype).and_then(|d| d.light.clone()) else {
            continue;
        };
        let room = f.room_at(p.cell.x as f64 + 0.5, p.cell.y as f64 + 0.5).filter(|i| !f.rooms[*i].lobby());
        if room.is_some_and(|i| dark[i]) {
            continue;
        }
        budget -= 1;
        let tint = match room {
            Some(i) => table.room_tint(&f.rooms[i].region, &f.rooms[i].rtype),
            None => table.room_tint(f.region_at(p.cell.x, p.cell.y), ""),
        };
        let (cx, cz) = p.cell.center();
        let (x, z) = (cx + p.offset.0, cz + p.offset.1);
        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
        let off = Mat3::from_rotation_y(yaw) * vec3(light.offset.0, 0.0, light.offset.2);
        let y = visual_y(&p.ptype, fixture_ceiling(f, p.cell), p.mount_y.unwrap_or(0.0) as f32) + light.offset.1;
        let pos = vec3(x + off.x, y, z + off.z);
        out.push(Lamp {
            pos,
            range: light.range,
            color: tinted(light.color, tint),
            // Godot's omni energy reads brighter than this shader's linear falloff; doubled to match
            energy: light.energy * 2.0 * tint.energy,
            flicker: lamp_flicker::make(light.flicker_hz, light.flicker_depth, world_place(f, pos)),
        });
    }
    out
}

/// A lamp colour under a room tint.
fn tinted(c: (f32, f32, f32), t: Tint) -> Color {
    Color::new(c.0 * t.color[0], c.1 * t.color[1], c.2 * t.color[2], 1.0)
}

/// Per room: whether it starts dark. Only the rooms of a level 0 Infinite window (no service lift) can; the lobby never.
fn dark_rooms(f: &Facility) -> Vec<bool> {
    dark_rooms_with(f, lamp_flicker::table())
}

fn dark_rooms_with(f: &Facility, table: &lamp_flicker::Lamps) -> Vec<bool> {
    let streamed = f.level == 0 && !f.has_lift;
    f.rooms.iter().map(|r| streamed && !r.lobby() && table.room_starts_dark(&r.id)).collect()
}

/// A window-space point in world cells, so a lamp's flicker phase does not change when the window moves.
fn world_place(f: &Facility, p: Vec3) -> [f32; 3] {
    [p.x + f.origin.0 as f32, p.y, p.z + f.origin.1 as f32]
}

fn room_set(f: &Facility, x: i32, y: i32) -> &'static str {
    match f.room_at(x as f64 + 0.5, y as f64 + 0.5) {
        Some(i) if !f.rooms[i].lobby() => region_texture(&f.rooms[i].region),
        // halls take their district's set on the Infinite route (the maze on a finite floor)
        _ => {
            let r = f.region_at(x, y);
            TEXTURE_SETS
                .iter()
                .copied()
                .find(|s| *s == r)
                .unwrap_or("maze")
        }
    }
}

/// The cells the floor draws: its floor cells, and the shaft cells of a stairwell that are walls (a level 0 well is a
/// walled box), where the flight is drawn.
fn drawn_cells(f: &Facility) -> Vec<Cell> {
    let mut cells = f.floor_cells();
    let mut shaft: Vec<Cell> = stair_steps(f)
        .keys()
        .filter(|&&(x, y)| f.is_wall_cell(x, y) && x >= 1 && y >= 1 && x < f.cols - 1 && y < f.rows - 1)
        .map(|&(x, y)| Cell::new(x, y))
        .collect();
    shaft.sort_by_key(|c| (c.y, c.x));
    cells.extend(shaft);
    cells
}

/// Stairwell flights: each well cell and its stair.
fn stair_steps(f: &Facility) -> HashMap<(i32, i32), Stair> {
    let mut m = HashMap::new();
    for s in &f.stairs {
        for c in s.well_cells() {
            m.insert((c.x, c.y), *s);
        }
    }
    m
}

/// Cells a hall's floor kind spans each way: one pick per chunk of the infinite world.
const HALL_FLOOR_SPAN: i64 = CHUNK as i64;

/// The RefTextures floor kind the world mesh draws on cell `(x, y)` (a room's one kind, the hall's per chunk, the carpet
/// runner), without the safe-room pad; `cache` holds the texture picks per room or hall key between calls.
pub(crate) fn floor_kind_at(f: &Facility, x: i32, y: i32, cache: &mut HashMap<(String, String, bool), TextureChoice>) -> &'static str {
    cell_surfaces(f, x, y, false, cache).1.kind
}

/// The original's surfaces for the space a cell is in (a room by its region and type, else the
/// hall of its district) and the floor kind of that cell: the room's one kind, the district's
/// hall kind per chunk with the carpet runner where the district lays one, the safe-room pad.
fn cell_surfaces<'c>(
    f: &Facility,
    x: i32,
    y: i32,
    pad: bool,
    cache: &'c mut HashMap<(String, String, bool), TextureChoice>,
) -> (&'c TextureChoice, TexRef) {
    let room = f
        .room_at(x as f64 + 0.5, y as f64 + 0.5)
        .filter(|i| !f.rooms[*i].lobby());
    let key = match room {
        Some(i) => (f.rooms[i].region.clone(), f.rooms[i].rtype.clone(), false),
        None => (f.region_at(x, y).to_string(), String::new(), true),
    };
    let choice = cache
        .entry(key.clone())
        .or_insert_with(|| textures::region_set(&key.0, &key.1, key.2));
    let (gx, gy) = ((x + f.origin.0) as i64, (y + f.origin.1) as i64);
    let mut floor = match room {
        _ if choice.per_cell => choice.floor_at(gx, gy),
        Some(i) => choice.floor_at(
            (f.rooms[i].x + f.origin.0) as i64,
            (f.rooms[i].y + f.origin.1) as i64,
        ),
        None => choice.floor_at(gx.div_euclid(HALL_FLOOR_SPAN), gy.div_euclid(HALL_FLOOR_SPAN)),
    };
    if choice.runner {
        let open = |a: i32, b: i32| {
            !f.is_wall_cell(a, b) && f.room_at(a as f64 + 0.5, b as f64 + 0.5).is_none()
        };
        if let Some(k) = textures::sets::runner_kind(open, x, y, 3) {
            floor = TexRef::new("floor", k);
        }
    }
    if let Some(sz) = f.safe_zone.filter(|_| pad) {
        if let Some(k) = textures::sets::safe_pad_kind(x - sz.x, y - sz.y) {
            floor = TexRef::new("floor", k);
        }
    }
    (choice, floor)
}

/// A region's wall height in metres: its `geometry.wall_height`, or WALL_HEIGHT when the region has no block
/// (or no region), capped under the slab (`seams::wall_top`, Godot's `LEVEL_HEIGHT - SLAB`), so no cell's wall or
/// ceiling ever reaches the floor of the level above.
fn geometry_wall_height(def: Option<&RegionDef>) -> f32 {
    seams::wall_top(def.and_then(|d| d.geometry).map_or(WALL_HEIGHT, |g| g.wall_height))
}

/// Where a ceiling-hung fixture or prop is seated in the cell `c`: the underside of that cell's ceiling quad (a lamp's
/// top sits flush with the ceiling, never inside the slab or above a lower sewer ceiling).
fn fixture_ceiling(f: &Facility, c: Cell) -> f32 {
    seams::ceiling_y(cell_wall_height(f, c.x, c.y))
}

/// The wall height of the region named `region` (its installed `lr.region/1` definition), else WALL_HEIGHT.
fn region_wall_height(region: &str) -> f32 {
    geometry_wall_height(crate::content::regions::table().def(region))
}

/// The region a floor cell is in: its room's region, or the district of its hall when the cell is in no room
/// (or the lobby). The same choice `cell_surfaces` makes for the cell's textures.
fn cell_region(f: &Facility, x: i32, y: i32) -> &str {
    match f.room_at(x as f64 + 0.5, y as f64 + 0.5).filter(|i| !f.rooms[*i].lobby()) {
        Some(i) => &f.rooms[i].region,
        None => f.region_at(x, y),
    }
}

/// The wall height of the region a floor cell is in.
fn cell_wall_height(f: &Facility, x: i32, y: i32) -> f32 {
    region_wall_height(cell_region(f, x, y))
}

/// The safe-zone checkpoint light of the region that holds the cell (x, y): its `safe_light` block, or the
/// game's defaults when the region has none (or no region is installed for it).
fn safe_light_for(f: &Facility, x: i32, y: i32) -> RegionSafeLight {
    safe_light_of(crate::content::regions::table().def(cell_region(f, x, y)))
}

/// A region's safe-zone light: its `safe_light` block, or the defaults when the region has no block (or no region).
fn safe_light_of(def: Option<&RegionDef>) -> RegionSafeLight {
    def.and_then(|d| d.safe_light).unwrap_or_default()
}

/// A builder per painted kind, falling back to the region swatch PNG for an unknown kind.
fn painted<'b>(
    map: &'b mut HashMap<(&'static str, &'static str), Builder>,
    r: TexRef,
    fallback: &Texture2D,
) -> &'b mut Builder {
    map.entry((r.group, r.kind)).or_insert_with(|| {
        Builder::new(Some(
            surfaces::texture(r).unwrap_or_else(|| fallback.clone()),
        ))
    })
}

/// Files a stair cell's pieces under the cell's floor and wall pictures (the steps and the pit walls), or in plain colour.
fn add_stairs(
    bits: stairs::Parts,
    surf: &mut HashMap<(&'static str, &'static str), Builder>,
    plain: &mut Builder,
    glow: &mut Builder,
    floor: TexRef,
    wall: TexRef,
    assets: &Assets,
    set: &str,
) {
    for q in bits.quads {
        let b = match q.face {
            stairs::Face::Floor => painted(surf, floor, assets.floor(set)),
            stairs::Face::Wall => painted(surf, wall, assets.wall(set)),
            stairs::Face::Plain | stairs::Face::Iron => &mut *plain,
        };
        b.quad(q.c, q.uv, q.n, q.col, 0.0);
    }
    for (centre, size, col) in bits.boxes {
        plain.cube(centre, size, 0.0, col, 0.0);
    }
    for (centre, size, col) in bits.glow {
        glow.cube(centre, size, 0.0, col, 2.0);
    }
}

/// The whole floor's meshes, built in one go.
pub fn build_world(
    f: &Facility,
    lights_on: &[bool],
    assets: &Assets,
    content: &Content,
) -> WorldMeshes {
    let mut w = WorldMeshes::default();
    w.rebuild(f, lights_on, None, assets, content);
    w
}

/// The room a prop's model variant is picked by. A window room's id carries its chunk's place in the window
/// (`Facility::merge`), so it is put on the absolute cell instead and the variant holds as the window moves.
///
/// A prop whose room index names no room of the window (a chunk with props and no room of its own: the sewer props
/// find none and carry room 0, which the merge turns into the first room of a later chunk, or one past the last) is
/// picked by its own id. Indexing `f.rooms` there panicked in `build_meshes`, which in the browser build stops the
/// frame loop at the first sewer part that held such a prop: a black screen on reaching the sewers in those rooms.
fn variant_key(f: &Facility, room: usize, prop_id: &str) -> String {
    let Some(r) = f.rooms.get(room) else {
        return prop_id.to_string();
    };
    match r.id.rsplit_once('@') {
        Some((id, _)) => format!("{id}@{},{}", r.x + f.origin.0, r.y + f.origin.1),
        None => r.id.clone(),
    }
}

/// The model a prop is drawn with: the variant its room picks, else the first one that is loaded.
fn prop_model_key<'c>(f: &Facility, p: &Prop, assets: &Assets, content: &'c Content) -> Option<&'c String> {
    let models = content.props.get(&p.ptype).map(|d| d.models.as_slice()).filter(|m| !m.is_empty())?;
    let picked = &models[stable_index(&variant_key(f, p.room, &p.id), models.len())];
    if assets.models.contains_key(picked) {
        Some(picked)
    } else {
        models.iter().find(|m| assets.models.contains_key(*m))
    }
}

/// Where a room door stands: raised to the hunter's opening and widened to its doorway, on its doorway's centre.
fn door_xf(f: &Facility, p: &Prop, d: DoorDraw, fit: &door::DoorFit, h: f32) -> Mat4 {
    let (x, z) = p.cell.center();
    // on the Inferno's noisy floor the door stands on the ground under its doorway
    let lift = f.terrain().map_or(0.0, |t| t.floor_at(x + d.offset[0], z + d.offset[1]));
    let y = lift + visual_y(&p.ptype, h, p.mount_y.unwrap_or(0.0) as f32);
    let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
    door::door_transform(vec3(x + d.offset[0], y, z + d.offset[1]), yaw, fit.sx, fit.sy, prop_scale::draw_scale(&p.ptype))
}

/// Every room door of `f` that has a model, for the per-frame leaf draw.
fn door_instances(f: &Facility, assets: &Assets, content: &Content) -> Vec<DoorInstance> {
    f.props
        .iter()
        .filter_map(|p| {
            let d = p.door?;
            let key = prop_model_key(f, p, assets, content)?;
            let dims = assets.models.get(key)?.door?;
            let fit = door::DoorFit::new(dims, &p.ptype, d.span);
            Some(DoorInstance {
                cell: p.cell,
                model: key.clone(),
                xf: door_xf(f, p, d, &fit, fixture_ceiling(f, p.cell)),
                fit,
            })
        })
        .collect()
}

/// The level -3 window as the renderer sees it: the noisy floor and ceiling of the facility's terrain (corners are
/// facility grid corners) and each chunk's cavern carve, made once, which the decoration placement reads.
struct InfernoFloor {
    seed: String,
    origin: (i32, i32),
    chunks: HashMap<(i32, i32), InfernoChunk>,
    terrain: Terrain,
}

impl InfernoFloor {
    /// The window floor, or None when `f` is not a level -3 window (it has no terrain; its text is `<seed>:L<level>`).
    fn of(f: &Facility) -> Option<InfernoFloor> {
        let (seed, level) = f.seed_text.rsplit_once(":L")?;
        let terrain = f.terrain()?.clone();
        (level.parse::<i32>().ok()? == INFERNO_LEVEL).then(|| InfernoFloor {
            seed: seed.to_string(),
            origin: f.origin,
            terrain,
            chunks: HashMap::new(),
        })
    }

    /// The floor height of a cell and whether it is lava (a closed pit cell, which the floor and walls never follow down).
    fn cell(&mut self, x: i32, y: i32) -> (f32, bool) {
        let (gx, gy) = (x + self.origin.0, y + self.origin.1);
        let key = (gx.div_euclid(CHUNK), gy.div_euclid(CHUNK));
        let c = Cell::new(gx.rem_euclid(CHUNK), gy.rem_euclid(CHUNK));
        let chunk = self.carve(key);
        (chunk.height_at(c), chunk.is_lava(c))
    }

    /// The height of the floor at the grid corner (`i`, `j`).
    fn corner(&self, i: i32, j: i32) -> f32 {
        self.terrain.floor_corner(i, j)
    }

    /// The height of the ceiling at the grid corner (`i`, `j`).
    fn ceiling(&self, i: i32, j: i32) -> f32 {
        self.terrain.ceiling_corner(i, j)
    }

    /// The carve of the chunk `key`, made once.
    fn carve(&mut self, key: (i32, i32)) -> &InfernoChunk {
        let seed = &self.seed;
        self.chunks
            .entry(key)
            .or_insert_with(|| InfernoChunk::carve(seed, ChunkKey { cx: key.0, cy: key.1, level: INFERNO_LEVEL }))
    }
}

/// The cave decoration's drawing numbers (data/render/world_decor.json); the pieces' own look is in inferno.json.
#[derive(Clone, Debug, PartialEq)]
pub struct DecorCfg {
    /// Sides of a stalactite or stalagmite cone.
    pub cone_sides: usize,
    /// Base radius of a cone, as a fraction of its height.
    pub cone_radius_fraction: f32,
    /// Width of a charred prop's box, as a fraction of its size.
    pub charred_width_fraction: f32,
    /// A floor crack's width and length, as fractions of its size.
    pub crack_width_fraction: f32,
    pub crack_length_fraction: f32,
    /// How far a crack lies above the floor, in metres.
    pub crack_lift_m: f32,
    /// Shader mode of a crack (2 = unlit, so it glows).
    pub crack_unlit_mode: f32,
}

impl DecorCfg {
    pub fn parse(text: &str) -> Result<DecorCfg, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let num = |k: &str| -> Result<f32, String> {
            v.get(k)
                .and_then(|x| x.as_f64())
                .map(|x| x as f32)
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("world_decor.json: \"{k}\" must be a number of zero or more"))
        };
        let sides = num("cone_sides")?;
        if sides < 3.0 {
            return Err("world_decor.json: \"cone_sides\" must be at least 3".to_string());
        }
        Ok(DecorCfg {
            cone_sides: sides as usize,
            cone_radius_fraction: num("cone_radius_fraction")?,
            charred_width_fraction: num("charred_width_fraction")?,
            crack_width_fraction: num("crack_width_fraction")?,
            crack_length_fraction: num("crack_length_fraction")?,
            crack_lift_m: num("crack_lift_m")?,
            crack_unlit_mode: num("crack_unlit_mode")?,
        })
    }
}

pub const DECOR_PATH: &str = "render/world_decor.json";

/// The decoration numbers compiled into the executable; a bad file is reported on stderr and the Standard values used.
pub fn decor_cfg() -> &'static DecorCfg {
    static CFG: OnceLock<DecorCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = crate::content::embedded::data_under(DECOR_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        DecorCfg::parse(text).unwrap_or_else(|e| {
            eprintln!("{DECOR_PATH}: {e}");
            DecorCfg {
                cone_sides: 6,
                cone_radius_fraction: 0.2,
                charred_width_fraction: 0.7,
                crack_width_fraction: 0.12,
                crack_length_fraction: 1.0,
                crack_lift_m: 0.012,
                crack_unlit_mode: 2.0,
            }
        })
    })
}

/// A cone from a circular base (`base` centre, horizontal `radius`) to `apex`, as `sides` triangles, each wound
/// with its front outward (the floor's quads have `cross(b - a, c - a)` against the normal).
fn cone(b: &mut Builder, base: Vec3, apex: Vec3, radius: f32, sides: usize, col: Color) {
    for i in 0..sides {
        let (a0, a1) = (i as f32 / sides as f32 * std::f32::consts::TAU, (i + 1) as f32 / sides as f32 * std::f32::consts::TAU);
        let p0 = base + vec3(a0.cos() * radius, 0.0, a0.sin() * radius);
        let p1 = base + vec3(a1.cos() * radius, 0.0, a1.sin() * radius);
        let mid = (a0 + a1) * 0.5;
        let out = vec3(mid.cos(), 0.0, mid.sin());
        let (p0, p1) = if (p1 - p0).cross(apex - p0).dot(out) > 0.0 { (p1, p0) } else { (p0, p1) };
        let n = (p1 - p0).cross(apex - p0).normalize_or_zero() * -1.0;
        b.quad([p0, p1, apex, apex], [vec2(0., 0.), vec2(1., 0.), vec2(0.5, 1.), vec2(0.5, 1.)], n, col, 0.0);
    }
}

fn rgb(c: [f32; 3]) -> Color {
    Color::new(c[0], c[1], c[2], 1.0)
}

/// Draws the cave decoration `d` of a chunk whose cell (0, 0) lies at facility cell `at`: stalactites hang from the
/// ceiling `ceiling_y`, stalagmites rise from the floor, charred props are dark boxes, cracks are flat glowing quads.
/// Lava glow and embers are lights and particles (the scene's).
fn add_deco(
    plain: &mut Builder,
    glow: &mut Builder,
    d: &crate::render::inferno_deco::Deco,
    at: (i32, i32),
    ceiling_y: f32,
    cfg: &DecorCfg,
) {
    use crate::render::inferno_deco::DecoKind;
    let x = (at.0 + d.cell.x) as f32 + 0.5 + d.offset[0];
    let z = (at.1 + d.cell.y) as f32 + 0.5 + d.offset[1];
    let col = rgb(d.color);
    match d.kind {
        DecoKind::Stalactite => cone(
            plain,
            vec3(x, ceiling_y, z),
            vec3(x, (ceiling_y - d.size_m).max(d.floor_y), z),
            d.size_m * cfg.cone_radius_fraction,
            cfg.cone_sides,
            col,
        ),
        DecoKind::Stalagmite => cone(
            plain,
            vec3(x, d.floor_y, z),
            vec3(x, (d.floor_y + d.size_m).min(ceiling_y), z),
            d.size_m * cfg.cone_radius_fraction,
            cfg.cone_sides,
            col,
        ),
        DecoKind::CharredProp => {
            let w = d.size_m * cfg.charred_width_fraction;
            plain.cube(vec3(x, d.floor_y + d.size_m * 0.5, z), vec3(w, d.size_m, w), d.yaw, col, 0.0);
        }
        DecoKind::FloorCrack => {
            let (w, l) = (d.size_m * cfg.crack_width_fraction * 0.5, d.size_m * cfg.crack_length_fraction * 0.5);
            let (s, c) = d.yaw.sin_cos();
            let at = |u: f32, v: f32| vec3(x + c * u + s * v, d.floor_y + cfg.crack_lift_m, z - s * u + c * v);
            glow.quad(
                [at(-w, -l), at(w, -l), at(w, l), at(-w, l)],
                UV1,
                vec3(0., 1., 0.),
                col,
                cfg.crack_unlit_mode,
            );
        }
        DecoKind::LavaGlow | DecoKind::Ember => {}
    }
}

/// The sewer window's grids: each chunk's `sewer_grid`, made once, read per cell (level -1 and the lower sewer).
struct SewerFloor {
    seed: String,
    origin: (i32, i32),
    level: i32,
    /// Each chunk's grid and its falls' jump cell (the railing gap), None off the generated chunks.
    chunks: HashMap<(i32, i32), Option<(Grid, Option<(i32, i32)>)>>,
}

/// The chunk (cx, cy) and the cell inside it of facility cell (`x`, `y`) of a window that starts at `origin`.
fn chunk_cell(origin: (i32, i32), x: i32, y: i32) -> ((i32, i32), (usize, usize)) {
    let (gx, gy) = (x + origin.0, y + origin.1);
    ((gx.div_euclid(CHUNK), gy.div_euclid(CHUNK)), (gx.rem_euclid(CHUNK) as usize, gy.rem_euclid(CHUNK) as usize))
}

impl SewerFloor {
    /// The window's sewer grids, or None when `f` is not a sewer window (its text is `<seed>:L<level>`).
    fn of(f: &Facility) -> Option<SewerFloor> {
        let (seed, level) = f.seed_text.rsplit_once(":L")?;
        let level = level.parse::<i32>().ok().filter(|l| *l == SEWER_LEVEL || *l == SEWER_LOWER_LEVEL)?;
        Some(SewerFloor { seed: seed.to_string(), origin: f.origin, level, chunks: HashMap::new() })
    }

    fn grid(&mut self, key: (i32, i32)) -> Option<&Grid> {
        self.grid_and_gap(key).map(|(g, _)| g)
    }

    fn grid_and_gap(&mut self, key: (i32, i32)) -> Option<&(Grid, Option<(i32, i32)>)> {
        let (seed, level) = (&self.seed, self.level);
        self.chunks
            .entry(key)
            .or_insert_with(|| {
                crate::world::route::sewer::chunk(
                    crate::world::route::layout::Coord::new(key.0, level, key.1),
                    &crate::world::route::sewer::Inputs { world: seed },
                )
                .map(|c| (c.sewer_grid, c.falls.as_ref().map(|f| (f.jump.cell.x, f.jump.cell.y))))
            })
            .as_ref()
    }

    /// Whether facility cell (`x`, `y`) is channel water.
    fn is_water(&mut self, x: i32, y: i32) -> bool {
        let (key, (lx, ly)) = chunk_cell(self.origin, x, y);
        self.grid(key).is_some_and(|g| g[ly][lx] == WATER)
    }
}

/// What lies across one side of a lava cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PitSide {
    /// More lava: the pit goes on, no wall.
    Lava,
    /// A walkable cell: the pit wall rises from the bed to that cell's floor, open to it.
    Open,
    /// Rock: the wall rises from the bed to the ceiling.
    Solid,
}

/// The faces of one lava cell: the glowing surface at the lava height, the bed, the ceiling over the pit and the wall on
/// every side that faces something other than lava. The sides are in the order of `PIT_SIDES`.
#[derive(Clone, Debug)]
struct PitCell {
    surface: [Vec3; 4],
    bed: [Vec3; 4],
    ceiling: [Vec3; 4],
    /// Quad corners (top a, top b, bottom b, bottom a) and the normal facing into the pit.
    walls: Vec<([Vec3; 4], Vec3)>,
}

/// The sides of a cell as (dx, dz): north, south, west, east.
const PIT_SIDES: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// The faces of the lava cell (`ci`, `cj`). `floor` and `ceil` give the terrain heights at a grid corner, so a pit wall
/// meets the floor of the cell beside it (and the rock meets the ceiling) exactly, with no slit.
fn pit_cell(ci: i32, cj: i32, sides: [PitSide; 4], surf: f32, bed: f32, floor: &dyn Fn(i32, i32) -> f32, ceil: &dyn Fn(i32, i32) -> f32) -> PitCell {
    let (x, z) = (ci as f32, cj as f32);
    let flat = |y: f32| [vec3(x, y, z), vec3(x + 1., y, z), vec3(x + 1., y, z + 1.), vec3(x, y, z + 1.)];
    let mut out = PitCell {
        surface: flat(surf),
        bed: flat(bed),
        // the order of the ceiling quads of the plain cells
        ceiling: [
            vec3(x, ceil(ci, cj + 1), z + 1.),
            vec3(x + 1., ceil(ci + 1, cj + 1), z + 1.),
            vec3(x + 1., ceil(ci + 1, cj), z),
            vec3(x, ceil(ci, cj), z),
        ],
        walls: vec![],
    };
    // the edge corners (integer) and the normal, as the plain cells' edge table lists them
    let edges: [((i32, i32), (i32, i32), Vec3); 4] = [
        ((ci + 1, cj), (ci, cj), vec3(0., 0., 1.)),
        ((ci, cj + 1), (ci + 1, cj + 1), vec3(0., 0., -1.)),
        ((ci, cj), (ci, cj + 1), vec3(1., 0., 0.)),
        ((ci + 1, cj + 1), (ci + 1, cj), vec3(-1., 0., 0.)),
    ];
    for (k, side) in sides.into_iter().enumerate() {
        if side == PitSide::Lava {
            continue;
        }
        let (a, b, n) = edges[k];
        let top = |p: (i32, i32)| if side == PitSide::Open { floor(p.0, p.1) } else { ceil(p.0, p.1) };
        let (ta, tb) = (vec3(a.0 as f32, top(a), a.1 as f32), vec3(b.0 as f32, top(b), b.1 as f32));
        let (la, lb) = (vec3(a.0 as f32, bed, a.1 as f32), vec3(b.0 as f32, bed, b.1 as f32));
        out.walls.push(([ta, tb, lb, la], n));
    }
    out
}

/// Whether a cell draws its ceiling quad: not over the well of a flight up (the lower level's well is open to the level
/// above, whose floor has the matching hole).
fn draws_ceiling(step: Option<Stair>) -> bool {
    !matches!(step, Some(s) if s.dir > 0)
}

/// What a cell of the window draws besides its fittings: the plain floor quad, the ceiling quad and the wall columns
/// on its sides. `step` is the stair whose well holds the cell (a ladder foot counts, dir > 0). The lower level's
/// ladder foot has no ceiling (the shaft is open to the hatch above), nor has a level -2 cell under the falls void
/// (`is_air_above`); a level -1 drop cell (the void) has no floor and no walls. A hatch (dir < 0) draws its floor
/// itself (`stairs::ladder_down`), so it has no plain quad either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellDraws {
    floor: bool,
    ceiling: bool,
    walls: bool,
}

fn cell_draws(f: &Facility, c: Cell, step: Option<Stair>) -> CellDraws {
    let drop = f.is_drop(c);
    let foot = matches!(step, Some(s) if s.is_ladder() && s.dir > 0);
    CellDraws {
        floor: !drop && (step.is_none() || foot),
        ceiling: draws_ceiling(step) && !f.is_air_above(c),
        walls: !drop,
    }
}

/// What fills one side of a cell between its floor and the storey above (`edge_fill`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum EdgeFill {
    None,
    /// A wall column up to `top`: the cell's wall height, or the storey for a shaft cell that has no ceiling.
    Wall { top: f32 },
    /// The side of the slab round an opening: from this cell's ceiling up to the underside of the floor above, facing
    /// the open neighbour. Without it the gap between the ceiling and the next floor shows as a slit round the opening.
    Rim { lo: f32, hi: f32 },
}

/// The fill of the side of cell `c` facing `(dx, dz)`. A stairwell's shaft is open to its landing and the other lane,
/// whatever its cells' wall flags say; the falls void has no wall columns, on either side of its edge. A cell with no
/// ceiling (a shaft, a ladder foot, the pool under the void) lines its walls to the storey instead of its wall height,
/// so the gap the slab cap leaves is closed where the shaft rises through it.
fn edge_fill(f: &Facility, steps: &HashMap<(i32, i32), Stair>, c: Cell, (dx, dz): (i32, i32), draw: CellDraws, wall_h: f32) -> EdgeFill {
    let n = Cell::new(c.x + dx, c.y + dz);
    if f.is_wall_cell(n.x, n.y) {
        if !draw.walls || steps.contains_key(&(n.x, n.y)) || f.is_drop(n) {
            return EdgeFill::None;
        }
        return EdgeFill::Wall { top: if draw.ceiling { wall_h } else { seams::shaft_top() } };
    }
    if draw.ceiling && f.open_above(n) {
        return EdgeFill::Rim { lo: seams::ceiling_y(wall_h), hi: seams::shaft_top() };
    }
    EdgeFill::None
}

fn build_meshes(
    f: &Facility,
    pick: &Pick,
    choices: &mut HashMap<(String, String, bool), TextureChoice>,
    assets: &Assets,
    content: &Content,
) -> Vec<Mesh> {
    let mut surf: HashMap<(&'static str, &'static str), Builder> = HashMap::new();
    let mut plain = Builder::new(Some(assets.white.clone()));
    // the stair treads' nosing, drawn unlit
    let mut glow = Builder::new(Some(assets.white.clone()));
    let h = WALL_HEIGHT;
    let ceiling = Color::from_rgba(46, 34, 44, 255);
    let up = vec3(0., 1., 0.);
    let steps = stair_steps(f);
    let mut inferno = InfernoFloor::of(f);
    // the sewer window's channel water: the bed lies below the floor, the surface (lit, tinted by depth) above it
    let mut sewer = SewerFloor::of(f);
    let water_params = crate::world::route::sewer::water::params();
    let bed = water_params.bed_offset();
    let mut water = Builder::new(Some(assets.white.clone()));
    // one painted storey stretched floor to ceiling, cornice at the top (RefSurfaceMesh.add_wall)
    let cap = seams::wall_top(h);
    let tiers = textures::wall_tiers(cap) as f32;
    for &c in &pick.cells {
        let (x, z) = (c.x as f32, c.y as f32);
        let set = room_set(f, c.x, c.y);
        let (choice, floor_kind) = cell_surfaces(f, c.x, c.y, pick.pad, choices);
        let wall_kind = choice.wall_at((c.x + f.origin.0) as i64, (c.y + f.origin.1) as i64);
        let ceil_kind = choice.ceiling;
        // this cell's walls and ceiling take its region's wall height (WALL_HEIGHT without a geometry block)
        let wall_h = cell_wall_height(f, c.x, c.y);
        let wall_tiers = textures::wall_tiers(wall_h) as f32;
        let step = steps.get(&(c.x, c.y)).copied();
        // what the cell draws (ceiling, floor, walls), then the stair split: the lower level's ladder foot is an
        // ordinary floor cell with the rails of its shaft, not a well drawn as a hatch
        let draw = cell_draws(f, c, step);
        let foot = step.filter(|s| s.is_ladder() && s.dir > 0);
        let step = step.filter(|s| !(s.is_ladder() && s.dir > 0));
        let site = stairs::Site {
            h,
            wall_h,
            tiers: wall_tiers,
            well: &|x, z| steps.contains_key(&(x, z)),
            solid: &|x, z| f.is_wall_cell(x, z),
        };
        let mut bits = stairs::Parts::default();
        match step {
            // a flight down opens the floor onto lit steps in a lined pit, a ladder onto its shaft; a flight up
            // climbs through an open shaft in the ceiling (render/stairs.rs)
            Some(s) if s.is_ladder() => stairs::ladder_down(&mut bits, (c.x, c.y), s, &site),
            Some(s) => stairs::flight_cell(&mut bits, (c.x, c.y), s, f.level, &site),
            // the falls void on level -1: open air, no floor quad (the lip around it keeps its own)
            None if !draw.floor => {}
            // a channel cell: the bed, flat, at the data depth below the floor
            None if sewer.as_mut().is_some_and(|s| s.is_water(c.x, c.y)) => painted(&mut surf, floor_kind, assets.floor(set)).quad(
                [vec3(x, bed, z), vec3(x + 1., bed, z), vec3(x + 1., bed, z + 1.), vec3(x, bed, z + 1.)],
                UV1,
                up,
                WHITE,
                0.0,
            ),
            None => {
                let (i, j) = (c.x, c.y);
                let drop = |inf: &Option<InfernoFloor>, i: i32, j: i32| inf.as_ref().map_or(0.0, |s| s.corner(i, j));
                painted(&mut surf, floor_kind, assets.floor(set)).quad(
                    [
                        vec3(x, drop(&inferno, i, j), z),
                        vec3(x + 1., drop(&inferno, i + 1, j), z),
                        vec3(x + 1., drop(&inferno, i + 1, j + 1), z + 1.),
                        vec3(x, drop(&inferno, i, j + 1), z + 1.),
                    ],
                    UV1,
                    up,
                    WHITE,
                    0.0,
                )
            }
        }
        // the lower level's ladder foot: rails and rungs up the shaft, no ceiling over it (stairs::ladder_up)
        if let Some(s) = foot {
            stairs::ladder_up(&mut bits, (c.x, c.y), s, &site);
        }
        add_stairs(bits, &mut surf, &mut plain, &mut glow, floor_kind, wall_kind, assets, set);
        if draw.ceiling {
            // inset under the slab's underside: no plane shared with the floor of the level above. The caverns hang
            // their ceiling at the noisy corner heights (which stay under the same line), everything else is flat
            let ceil_y = seams::ceiling_y(wall_h);
            let at_top = |i: i32, j: i32| inferno.as_ref().map_or(ceil_y, |s| s.ceiling(i, j));
            let corners = [
                vec3(x, at_top(c.x, c.y + 1), z + 1.),
                vec3(x + 1., at_top(c.x + 1, c.y + 1), z + 1.),
                vec3(x + 1., at_top(c.x + 1, c.y), z),
                vec3(x, at_top(c.x, c.y), z),
            ];
            match ceil_kind {
                Some(k) => painted(&mut surf, k, &assets.white).quad(
                    corners,
                    [vec2(0., 1.), vec2(1., 1.), vec2(1., 0.), vec2(0., 0.)],
                    -up,
                    WHITE,
                    0.0,
                ),
                None => plain.quad(corners, UV1, -up, ceiling, 0.0),
            }
        }
        let edges = [
            (0, -1, vec3(x + 1., 0., z), vec3(x, 0., z), vec3(0., 0., 1.)),
            (
                0,
                1,
                vec3(x, 0., z + 1.),
                vec3(x + 1., 0., z + 1.),
                vec3(0., 0., -1.),
            ),
            (-1, 0, vec3(x, 0., z), vec3(x, 0., z + 1.), vec3(1., 0., 0.)),
            (
                1,
                0,
                vec3(x + 1., 0., z + 1.),
                vec3(x + 1., 0., z),
                vec3(-1., 0., 0.),
            ),
        ];
        let here_water = step.is_none() && draw.walls && sewer.as_mut().is_some_and(|s| s.is_water(c.x, c.y));
        for (dx, dz, a, b, n) in edges {
            // the channel's bank: from the floor down to the bed, wherever the next cell is not channel water
            if here_water
                && !steps.contains_key(&(c.x + dx, c.y + dz))
                && !sewer.as_mut().is_some_and(|s| s.is_water(c.x + dx, c.y + dz))
            {
                painted(&mut surf, floor_kind, assets.floor(set)).quad([a, b, b + up * bed, a + up * bed], UV1, n, WHITE, 0.0);
            }
            // the side facing a lava pit is open: the pit's own wall (below) stands there instead of rock
            let into_pit = inferno.as_mut().is_some_and(|s| s.cell(c.x + dx, c.y + dz).1);
            let fill = if into_pit { EdgeFill::None } else { edge_fill(f, &steps, c, (dx, dz), draw, wall_h) };
            let wall_v = |y: f32| wall_tiers * (1.0 - y / wall_h);
            // in the caverns the wall's ends follow the noisy floor and ceiling corners
            let corner_at = |p: Vec3, y: f32| match inferno.as_ref() {
                Some(s) => vec3(p.x, y.max(s.corner(p.x as i32, p.z as i32)), p.z),
                None => vec3(p.x, y, p.z),
            };
            let floor_at = |p: Vec3| match inferno.as_ref() {
                Some(s) => vec3(p.x, s.corner(p.x as i32, p.z as i32), p.z),
                None => p,
            };
            let ceil_at = |p: Vec3, flat: f32| match inferno.as_ref() {
                Some(s) => vec3(p.x, s.ceiling(p.x as i32, p.z as i32), p.z),
                None => vec3(p.x, flat, p.z),
            };
            match fill {
                EdgeFill::None => {}
                EdgeFill::Wall { top } => {
                    // a cell with a ceiling hangs its wall to the (noisy) ceiling corner; a shaft cell rises to the storey
                    let (lo_a, lo_b) = (floor_at(a), floor_at(b));
                    let (top_a, top_b) = if draw.ceiling { (ceil_at(a, top), ceil_at(b, top)) } else { (corner_at(a, top), corner_at(b, top)) };
                    // the painted wall carries its own cornice, chair rail and skirting
                    painted(&mut surf, wall_kind, assets.wall(set)).quad(
                        [top_a, top_b, lo_b, lo_a],
                        [vec2(0., wall_v(top)), vec2(1., wall_v(top)), vec2(1., wall_tiers), vec2(0., wall_tiers)],
                        n,
                        WHITE,
                        0.0,
                    );
                }
                EdgeFill::Rim { lo, hi } => {
                    let (lo_a, lo_b) = (ceil_at(a, lo), ceil_at(b, lo));
                    painted(&mut surf, wall_kind, assets.wall(set)).quad(
                        [corner_at(a, hi), corner_at(b, hi), lo_b, lo_a],
                        [vec2(0., wall_v(hi)), vec2(1., wall_v(hi)), vec2(1., wall_v(lo)), vec2(0., wall_v(lo))],
                        -n,
                        WHITE,
                        0.0,
                    );
                }
            }
        }
    }
    // the picked cells by chunk, for the sewer water and the cave decoration, which are made per chunk
    let mut chunk_cells: HashMap<(i32, i32), HashSet<(i32, i32)>> = HashMap::new();
    if sewer.is_some() || inferno.is_some() {
        for c in &pick.cells {
            chunk_cells.entry(chunk_cell(f.origin, c.x, c.y).0).or_default().insert((c.x, c.y));
        }
    }
    // sewer water surface (render/water.rs), only for the cells this part owns so no cell is drawn twice
    if let Some(s) = sewer.as_mut() {
        for (&key, owned) in &chunk_cells {
            let Some(grid) = s.grid(key).copied() else { continue };
            let origin = (key.0 * CHUNK - f.origin.0, key.1 * CHUNK - f.origin.1);
            let keep = |lx: i32, lz: i32| owned.contains(&(origin.0 + lx, origin.1 + lz));
            super::water::add_water_cells(&mut water, &grid, vec2(origin.0 as f32, origin.1 as f32), 1.0, 0.0, &keep);
            // the iron railings along the banks (render/sewer_rails.rs), left open at the waterfall jump
            let gap = s.grid_and_gap(key).and_then(|(_, g)| *g);
            let rails = super::sewer_rails::config();
            let iron = rgb(rails.color);
            for seg in super::sewer_rails::channel_rail_segments(&grid, gap).into_iter().filter(|g| keep(g.x, g.z)) {
                for (c, size) in super::sewer_rails::rail_boxes(seg, rails) {
                    plain.cube(vec3(origin.0 as f32 + seg.x as f32 + c[0], c[1], origin.1 as f32 + seg.z as f32 + c[2]), Vec3::from(size), 0.0, iron, 0.0);
                }
            }
        }
    }
    // level -3 cave decoration (render/inferno_deco.rs): the pieces of the picked cells
    if let Some(inf) = inferno.as_mut() {
        let (seed, cfg, terrain) = (inf.seed.clone(), decor_cfg(), inf.terrain.clone());
        for (&key, owned) in &chunk_cells {
            let at = (key.0 * CHUNK - f.origin.0, key.1 * CHUNK - f.origin.1);
            let carved = inf.carve(key);
            for mut d in crate::render::inferno_deco::inferno_deco().place(&seed, ChunkKey { cx: key.0, cy: key.1, level: INFERNO_LEVEL }, carved) {
                let cell = (at.0 + d.cell.x, at.1 + d.cell.y);
                if owned.contains(&cell) {
                    // a piece stands on the noisy floor and hangs from the noisy ceiling under it
                    let (px, pz) = (cell.0 as f32 + 0.5 + d.offset[0], cell.1 as f32 + 0.5 + d.offset[1]);
                    d.floor_y = terrain.floor_at(px, pz);
                    add_deco(&mut plain, &mut glow, &d, at, terrain.ceiling_at(px, pz), cfg);
                }
            }
        }
    }
    // level -3 lava pits (look from render/inferno_deco.rs): the part that owns a chunk's fixtures draws its pits, so no pit
    // is drawn twice. Lava cells are wall cells to the build, so nothing else draws them.
    if let (Some(inf), Some((x0, y0, _, _))) = (inferno.as_mut(), pick.fixtures) {
        let key = chunk_cell(f.origin, x0, y0).0;
        let at = (key.0 * CHUNK - f.origin.0, key.1 * CHUNK - f.origin.1);
        let lava: Vec<Cell> = inf.carve(key).lava_cells();
        let rules = crate::world::inferno::InfernoRules::builtin();
        let look = crate::render::inferno_deco::lava_look();
        let mid = |k: usize| (look.hot[k] + look.cool[k]) * 0.5;
        let glow_col = Color::new(mid(0), mid(1), mid(2), 1.0);
        let pit_col = Color::new(look.pit_color[0], look.pit_color[1], look.pit_color[2], 1.0);
        let terrain = inf.terrain.clone();
        for lc in lava {
            let (wx, wz) = (at.0 + lc.x, at.1 + lc.y);
            let sides = PIT_SIDES.map(|(dx, dz)| {
                if inf.cell(wx + dx, wz + dz).1 {
                    PitSide::Lava
                } else if f.is_wall_cell(wx + dx, wz + dz) {
                    PitSide::Solid
                } else {
                    PitSide::Open
                }
            });
            let q = pit_cell(wx, wz, sides, rules.lava_y, rules.lava_basin_y, &|i, j| terrain.floor_corner(i, j), &|i, j| terrain.ceiling_corner(i, j));
            glow.quad(q.surface, UV1, up, glow_col, 2.0);
            plain.quad(q.bed, UV1, up, pit_col, 0.0);
            plain.quad(q.ceiling, UV1, -up, ceiling, 0.0);
            for (corners, n) in q.walls {
                plain.quad(corners, UV1, n, pit_col, 0.0);
            }
        }
    }
    let fixture = |c: Cell| pick.fixtures.is_some_and(|(x0, y0, x1, y1)| c.x >= x0 && c.y >= y0 && c.x < x1 && c.y < y1);

    // room props: each type's model, the variant picked by a hash of the room id like _variant_index
    let mut authored: HashMap<usize, Builder> = HashMap::new();
    for p in f.props.iter().filter(|p| fixture(p.cell)) {
        let (x, z) = p.cell.center();
        // a room door is centred on its doorway (a double door sits across two cells); any other prop is
        // drawn at its placement offset (a flush bed sits against its wall), as the footprint is
        let (x, z) = match p.door {
            Some(d) => (x + d.offset[0], z + d.offset[1]),
            None => (x + p.offset.0, z + p.offset.1),
        };
        // on the Inferno's noisy floor every prop stands on the ground under its centre
        let lift = f.terrain().map_or(0.0, |t| t.floor_at(x, z));
        let hung = fixture_ceiling(f, p.cell);
        let mut y = lift + visual_y(&p.ptype, hung, p.mount_y.unwrap_or(0.0) as f32);
        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
        // the model's drawn turn: a room door keeps the gameplay yaw, every other prop gets the half turn
        let visual_yaw = prop_visual_yaw(yaw, p.door.is_some());
        let model = prop_model_key(f, p, assets, content).and_then(|k| assets.models.get(k));
        // Godot's per-type hunter scale (PropModelCatalog.HUNTER_SCALE, data/render/prop_draw_scale.json).
        let scale = prop_scale::draw_scale(&p.ptype);
        if let Some(model) = model {
            // Surface children are authored with different local origins. Align the
            // model's actual lowest vertex to the authored tabletop height so computers,
            // printers and lamps sit on the parent instead of floating above it.
            if p.mount_y.is_some() {
                y -= model.min.y * scale;
            }
            // a room door is raised to the hunter's opening and widened to its doorway (door.rs, Godot's
            // RouteInteractable3D raise); every other prop is the uniform draw scale
            let xf = match p.door.zip(model.door) {
                Some((d, dims)) => {
                    let fit = door::DoorFit::new(dims, &p.ptype, d.span);
                    // the wall over the opening, from the raised frame's top up to the ceiling (Godot's door header)
                    let bottom = door::header_bottom(dims, fit.sy, cap);
                    if bottom < cap {
                        let (choice, _) = cell_surfaces(f, p.cell.x, p.cell.y, pick.pad, choices);
                        let wall = choice.wall_at((p.cell.x + f.origin.0) as i64, (p.cell.y + f.origin.1) as i64);
                        let fallback = assets.wall(room_set(f, p.cell.x, p.cell.y));
                        let filler = door::filler_from(dims, fit.sy);
                        door_header(&mut surf, wall, fallback, vec3(x, lift, z), yaw, d.span as f32, (bottom, filler), cap, tiers);
                    }
                    let xf = door_xf(f, p, d, &fit, hung);
                    model.add_door_body(&mut authored, xf, WHITE, &fit);
                    continue;
                }
                None => Mat4::from_translation(vec3(x, y, z))
                    * Mat4::from_rotation_y(visual_yaw)
                    * Mat4::from_scale(Vec3::splat(scale)),
            };
            model.add_to(&mut authored, xf, WHITE);
        }
    }
    // emissive lamp fixtures for the switchable room lights
    for l in f.room_lights.iter().filter(|l| fixture(l.center)) {
        let (x, z) = l.center.center();
        plain.cube(
            vec3(x, fixture_ceiling(f, l.center) - 0.04, z),
            vec3(0.7, 0.08, 0.7),
            0.0,
            Color::new(l.color.0, l.color.1, l.color.2, 1.0),
            2.0,
        );
    }

    let mut meshes = vec![];
    for (_, b) in surf {
        meshes.extend(b.finish());
    }
    for (_, b) in authored {
        meshes.extend(b.finish());
    }
    meshes.extend(plain.finish());
    meshes.extend(glow.finish());
    meshes.extend(water.finish());
    // cut into tiles, so each mesh's box is tight enough for the scene's fog and frustum culls to drop what is out of view
    meshes.into_iter().flat_map(|m| super::mesh_batch::split_into_tiles(m, super::mesh_batch::tile_size())).collect()
}

/// The radians a room prop's model is turned by when it is drawn, from its gameplay yaw. Godot turns every
/// authored prop a half turn on top of its gameplay yaw (PropModelCatalog.authored_visual_yaw, applied in
/// streamed_facility_view_3d.gd `_spawn_authored_prop`): the model's front is local +Z and gameplay forward is -Z.
/// Room doors are not turned; RouteInteractable3D carries the raw yaw (door.rs `door_transform`).
pub fn prop_visual_yaw(yaw: f32, door: bool) -> f32 {
    if door {
        yaw
    } else {
        yaw + std::f32::consts::PI
    }
}

/// The wall over a room door's opening, from `bottom` (the frame's top) up to the ceiling `h` (Godot's door
/// header, streamed_facility_view_3d.gd add_door_header): a wall face on each side of the threshold cell and the
/// soffit under it. The door stands on its threshold's centre line, so the faces sit half a cell either side of the
/// origin, and the header spans the doorway's `span` cells across the wall (the door's local X).
#[allow(clippy::too_many_arguments)]
fn door_header(
    map: &mut HashMap<(&'static str, &'static str), Builder>,
    tex: TexRef,
    fallback: &Texture2D,
    origin: Vec3,
    yaw: f32,
    span: f32,
    (bottom, filler): (f32, Option<f32>),
    h: f32,
    tiers: f32,
) {
    let rot = Mat3::from_rotation_y(yaw);
    let (across, through) = (rot * Vec3::X, rot * Vec3::Z);
    let half = span / 2.0;
    let at = |s: f32, d: f32, y: f32| origin + across * s + through * d + Vec3::Y * y;
    // the wall texture's v runs from 0 at the top of the wall to `tiers` at its floor
    let v = tiers * (h - bottom) / h;
    let b = painted(map, tex, fallback);
    for (d, n) in [(0.5, through), (-0.5, -through)] {
        let (tl, tr, br, bl) = (at(-half, d, h), at(half, d, h), at(half, d, bottom), at(-half, d, bottom));
        let uv = [vec2(0., 0.), vec2(span, 0.), vec2(span, v), vec2(0., v)];
        if d > 0.0 {
            b.quad([tl, bl, br, tr], [uv[0], uv[3], uv[2], uv[1]], n, WHITE, 0.0);
        } else {
            b.quad([tl, tr, br, bl], uv, n, WHITE, 0.0);
        }
    }
    b.quad(
        [
            at(-half, -0.5, bottom),
            at(half, -0.5, bottom),
            at(half, 0.5, bottom),
            at(-half, 0.5, bottom),
        ],
        [vec2(0., 0.), vec2(span, 0.), vec2(span, 1.), vec2(0., 1.)],
        -Vec3::Y,
        WHITE,
        0.0,
    );
    // a crest or spire above the frame's bar stands in front of a wall-coloured band that closes the slit between the
    // bar and the soffit, a hair behind the frame's face so the crest hides it
    if let Some(from) = filler {
        let depth = door::FRAME_HALF_DEPTH - 0.005;
        let v = tiers * (h - from) / h;
        let v0 = tiers * (h - bottom) / h;
        for (d, n) in [(depth, through), (-depth, -through)] {
            let (tl, tr, br, bl) = (at(-half, d, bottom), at(half, d, bottom), at(half, d, from), at(-half, d, from));
            let uv = [vec2(0., v0), vec2(span, v0), vec2(span, v), vec2(0., v)];
            if d > 0.0 {
                b.quad([tl, bl, br, tr], [uv[0], uv[3], uv[2], uv[1]], n, WHITE, 0.0);
            } else {
                b.quad([tl, tr, br, bl], uv, n, WHITE, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};
    use crate::world::facility::DoorDraw;
    use std::collections::HashSet;

    /// A temporary region with a geometry block is parsed like a pack's file: its wall height is its own, and a
    /// region without the block (or no region) keeps WALL_HEIGHT.
    #[test]
    fn a_region_geometry_block_sets_its_wall_height() {
        use crate::content::regions::parse_file;
        let tall = r#"{"schema":"lr.region/1","id":"tmp_tall","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[],"geometry":{"wall_height":6.5}}"#;
        let plain = r#"{"schema":"lr.region/1","id":"tmp_plain","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[]}"#;
        let low = plain.replace("\"prop_set\":[]", "\"prop_set\":[],\"geometry\":{\"wall_height\":3.0}").replace("tmp_plain", "tmp_low");
        let tall = parse_file("tmp_tall.json", tall).unwrap();
        let plain = parse_file("tmp_plain.json", plain).unwrap();
        let low = parse_file("tmp_low.json", &low).unwrap();
        let cap = seams::wall_top(WALL_HEIGHT);
        assert_eq!(geometry_wall_height(Some(&tall)), cap, "a region taller than the storey is clamped under the slab");
        assert_eq!(geometry_wall_height(Some(&low)), 3.0, "a lower ceiling is its own");
        assert_eq!(geometry_wall_height(Some(&plain)), cap, "no block: the storey less the slab, not 5.0");
        assert_eq!(geometry_wall_height(None), cap);
        assert!(cap < WALL_HEIGHT);
    }

    /// A region's safe-zone light comes from its `safe_light` block. A region without the block, or no region,
    /// gets the checkpoint light the game shipped with (warm colour, energy 2.0).
    #[test]
    fn a_region_safe_light_block_sets_the_checkpoint_light_and_the_rest_default() {
        use crate::content::regions::parse_file;
        let lit = r#"{"schema":"lr.region/1","id":"tmp_lit","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[],"safe_light":{"color":[0.1,0.2,0.3],"energy":5.0}}"#;
        let plain = r#"{"schema":"lr.region/1","id":"tmp_dim","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[]}"#;
        let lit = parse_file("tmp_lit.json", lit).unwrap();
        let plain = parse_file("tmp_dim.json", plain).unwrap();
        assert_eq!(safe_light_of(Some(&lit)), RegionSafeLight { color: [0.1, 0.2, 0.3], energy: 5.0 });
        assert_eq!(safe_light_of(Some(&plain)), RegionSafeLight::default());
        assert_eq!(safe_light_of(None), RegionSafeLight::default());
        assert_eq!(safe_light_of(None).color, [0.95, 0.78, 0.55]);
        assert_eq!(safe_light_of(None).energy, 2.0);
    }

    /// Where a prop model's front (local +Z, the seat front of the chair GLBs) lands in (x, z) once drawn.
    fn drawn_front(gameplay_yaw: f32) -> (f32, f32) {
        let turn = Mat4::from_rotation_y(prop_visual_yaw(gameplay_yaw, false));
        let v = turn.transform_vector3(vec3(0.0, 0.0, 1.0));
        (v.x, v.z)
    }

    /// Gameplay forward in (x, z): (-sin yaw, -cos yaw). Golden check: yaw 0 gives open_direction (0, -1).
    fn forward(gameplay_yaw: f32) -> (f32, f32) {
        (-gameplay_yaw.sin(), -gameplay_yaw.cos())
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    #[test]
    fn wooden_chair_at_golden_yaw_zero_faces_its_open_direction() {
        // golden: woodenChairCushion at cell (8,6), rotation 0 (yaw 0), open_direction (0,-1)
        let rotation: i32 = 0;
        let yaw = rotation as f32 * FRAC_PI_2;
        assert!(close(drawn_front(yaw), (0.0, -1.0)));
        assert!(close(drawn_front(yaw), forward(yaw)));
    }

    #[test]
    fn office_chair_at_golden_yaw_minus_pi_faces_its_open_direction() {
        // golden: officeChair, yaw -PI, rotation round(-PI / (PI/2)) rem_euclid 4 = 2, open_direction (0,1)
        let rotation = ((-PI / FRAC_PI_2).round() as i32).rem_euclid(4);
        assert_eq!(rotation, 2);
        let yaw = rotation as f32 * FRAC_PI_2;
        assert!(close(drawn_front(yaw), (0.0, 1.0)));
    }

    #[test]
    fn every_quarter_turn_draws_the_front_on_gameplay_forward() {
        for rotation in 0..4 {
            let yaw = rotation as f32 * FRAC_PI_2;
            assert!(close(drawn_front(yaw), forward(yaw)), "rotation {rotation}");
        }
    }

    /// The door fix: leaves are kept only for doors whose cell lies in a built part's fixture box, so the first
    /// frame of a streamed chunk never has a leaf without its frame, and a built chunk never loses its doors.
    #[test]
    fn door_leaves_follow_the_built_parts() {
        let rects = [(0, 0, 45, 45)];
        let cells = vec![Cell::new(0, 0), Cell::new(44, 44), Cell::new(45, 10), Cell::new(10, 45), Cell::new(-1, 3)];
        let kept = keep_in_rects(cells.clone(), |c| *c, &rects);
        assert_eq!(kept, vec![Cell::new(0, 0), Cell::new(44, 44)], "the box is half-open");
        assert!(keep_in_rects(cells.clone(), |c| *c, &[]).is_empty(), "no built part, no leaf");
        let two = [(0, 0, 45, 45), (45, 0, 90, 45)];
        assert_eq!(keep_in_rects(cells, |c| *c, &two).len(), 3, "a second built part brings its doors");
    }

    #[test]
    fn a_lava_cell_builds_surface_at_the_pit_height_and_open_walls() {
        let (surf, bed) = (-0.9, -2.0);
        let floor = |_: i32, _: i32| 0.0;
        let ceil = |_: i32, _: i32| 4.0;
        let q = pit_cell(5, 7, [PitSide::Open, PitSide::Lava, PitSide::Solid, PitSide::Lava], surf, bed, &floor, &ceil);
        assert!(q.surface.iter().all(|p| p.y == surf), "the surface lies at the pit height");
        assert!(q.bed.iter().all(|p| p.y == bed));
        assert_eq!(q.walls.len(), 2, "one wall per side that is not lava");
        let span = |c: &[Vec3; 4]| (c.iter().map(|p| p.y).fold(f32::MAX, f32::min), c.iter().map(|p| p.y).fold(f32::MIN, f32::max));
        assert_eq!(span(&q.walls[0].0), (bed, 0.0), "the open side rises to the floor beside it");
        assert_eq!(span(&q.walls[1].0), (bed, 4.0), "the rock side rises to the ceiling");
        for quad in [q.surface, q.bed, q.ceiling] {
            assert!(quad.iter().all(|p| p.x >= 5.0 && p.x <= 6.0 && p.z >= 7.0 && p.z <= 8.0), "covers exactly the cell");
        }
        assert!(q.ceiling.iter().all(|p| p.y == 4.0), "the ceiling over the pit is kept");
    }

    #[test]
    fn a_pit_wall_winds_like_a_plain_wall_and_meets_the_neighbour_floor() {
        let floor = |i: i32, j: i32| 0.1 * (i + j) as f32;
        let ceil = |_: i32, _: i32| 4.0;
        let q = pit_cell(2, 2, [PitSide::Open; 4], -1.0, -2.0, &floor, &ceil);
        assert_eq!(q.walls.len(), 4);
        for (c, n) in &q.walls {
            let rh = (c[1] - c[0]).cross(c[2] - c[1]);
            assert!(rh.dot(*n) > 0.0, "{c:?} winds against its normal");
            for p in &c[..2] {
                assert_eq!(p.y, floor(p.x as i32, p.z as i32), "the rim sits on the terrain corners: no slit round the pit");
            }
        }
        let inner = pit_cell(2, 2, [PitSide::Lava; 4], -1.0, -2.0, &floor, &ceil);
        assert!(inner.walls.is_empty(), "a pit inside the pit has no wall");
    }

    #[test]
    fn lava_cells_are_not_walls_in_the_inferno_chunk() {
        for cx in 0..8 {
            for cy in 0..8 {
                let c = InfernoChunk::carve("pit-seed", ChunkKey { cx, cy, level: INFERNO_LEVEL });
                if let Some(l) = c.lava_cells().first().copied() {
                    assert!(!c.draws_wall(l, true), "a lava cell draws no wall mass");
                    assert!(c.draws_wall(c.open_cells()[0], true));
                    return;
                }
            }
        }
        panic!("no lava in the sample");
    }

    #[test]
    fn a_window_cell_maps_to_its_chunk_and_local_cell() {
        let origin = (-CHUNK, 2 * CHUNK);
        assert_eq!(chunk_cell(origin, 0, 0), ((-1, 2), (0, 0)));
        assert_eq!(chunk_cell(origin, CHUNK + 3, 5), ((0, 2), (3, 5)));
        assert_eq!(chunk_cell((-1, 0), 0, 0), ((-1, 0), ((CHUNK - 1) as usize, 0)));
    }

    #[test]
    fn decor_numbers_come_from_data() {
        let cfg = DecorCfg::parse(include_str!("../data/render/world_decor.json")).unwrap();
        assert_eq!(decor_cfg(), &cfg);
        assert!(DecorCfg::parse("{}").is_err());
        let few = include_str!("../data/render/world_decor.json").replace("\"cone_sides\": 6", "\"cone_sides\": 2");
        assert!(DecorCfg::parse(&few).is_err());
    }

    fn deco(kind: crate::render::inferno_deco::DecoKind) -> crate::render::inferno_deco::Deco {
        crate::render::inferno_deco::Deco {
            kind,
            cell: Cell::new(2, 3),
            offset: [0.1, -0.1],
            size_m: 1.5,
            yaw: 0.0,
            color: [0.5, 0.2, 0.1],
            floor_y: -0.4,
            phase: 0.0,
        }
    }

    #[test]
    fn stalactites_hang_from_the_ceiling_and_stalagmites_rise_from_the_floor() {
        use crate::render::inferno_deco::DecoKind;
        let cfg = decor_cfg();
        let (mut tite, mut mite, mut glow) = (Builder::new(None), Builder::new(None), Builder::new(None));
        add_deco(&mut tite, &mut glow, &deco(DecoKind::Stalactite), (10, 20), 5.0, cfg);
        add_deco(&mut mite, &mut glow, &deco(DecoKind::Stalagmite), (10, 20), 5.0, cfg);
        let ys = |b: &Builder| b.verts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.position.y), hi.max(v.position.y)));
        let (lo, hi) = ys(&tite);
        assert!((hi - 5.0).abs() < 1e-4 && (lo - 3.5).abs() < 1e-4, "hangs 1.5 m down from the ceiling: {lo}..{hi}");
        let (lo, hi) = ys(&mite);
        assert!((lo + 0.4).abs() < 1e-4 && (hi - 1.1).abs() < 1e-4, "rises 1.5 m from the floor: {lo}..{hi}");
        assert_eq!(tite.verts.len(), cfg.cone_sides * 4, "one quad per side");
        // placed at the chunk's cell plus the offset (cells from the centre)
        assert!(tite.verts.iter().any(|v| (v.position.x - 12.6).abs() < 1e-3 && (v.position.z - 23.4).abs() < 1e-3));
        assert!(glow.verts.is_empty(), "cones are not glowing");
    }

    #[test]
    fn cone_side_count_follows_the_data() {
        use crate::render::inferno_deco::DecoKind;
        let mut cfg = decor_cfg().clone();
        cfg.cone_sides = 9;
        let (mut b, mut g) = (Builder::new(None), Builder::new(None));
        add_deco(&mut b, &mut g, &deco(DecoKind::Stalactite), (0, 0), 5.0, &cfg);
        assert_eq!(b.verts.len(), 9 * 4);
    }

    #[test]
    fn floor_cracks_glow_and_charred_props_stand() {
        use crate::render::inferno_deco::DecoKind;
        let cfg = decor_cfg();
        let (mut plain, mut glow) = (Builder::new(None), Builder::new(None));
        add_deco(&mut plain, &mut glow, &deco(DecoKind::FloorCrack), (0, 0), 5.0, cfg);
        assert_eq!(glow.verts.len(), 4);
        assert!(glow.verts.iter().all(|v| (v.position.y - (-0.4 + cfg.crack_lift_m)).abs() < 1e-5 && v.normal.w == cfg.crack_unlit_mode));
        assert!(plain.verts.is_empty());
        add_deco(&mut plain, &mut glow, &deco(DecoKind::CharredProp), (0, 0), 5.0, cfg);
        assert!(!plain.verts.is_empty());
        let top = plain.verts.iter().map(|v| v.position.y).fold(f32::MIN, f32::max);
        assert!((top - 1.1).abs() < 1e-4, "stands on the floor, 1.5 m tall");
        // light and particle kinds draw nothing here
        let (mut p2, mut g2) = (Builder::new(None), Builder::new(None));
        add_deco(&mut p2, &mut g2, &deco(DecoKind::LavaGlow), (0, 0), 5.0, cfg);
        add_deco(&mut p2, &mut g2, &deco(DecoKind::Ember), (0, 0), 5.0, cfg);
        assert!(p2.verts.is_empty() && g2.verts.is_empty());
    }

    #[test]
    fn room_door_keeps_the_gameplay_yaw() {
        assert_eq!(prop_visual_yaw(FRAC_PI_2, true), FRAC_PI_2);
        assert_eq!(prop_visual_yaw(FRAC_PI_2, false), FRAC_PI_2 + PI);
    }

    /// The cells a room door's header covers, placed as `door_header` places it: the doorway's `span` cells
    /// across the wall (local X from the yaw), centred on the threshold cell's centre plus the drawn offset.
    fn door_header_cells(cell: Cell, rotation: i32, door: DoorDraw) -> Vec<(i32, i32)> {
        let yaw = rotation as f32 * FRAC_PI_2;
        let across = Mat3::from_rotation_y(yaw) * Vec3::X;
        let (cx, cz) = cell.center();
        let centre = vec3(cx + door.offset[0], 0.0, cz + door.offset[1]);
        (0..door.span)
            .map(|i| {
                let p = centre + across * (i as f32 - (door.span as f32 - 1.0) / 2.0);
                (p.x.floor() as i32, p.z.floor() as i32)
            })
            .collect()
    }

    /// A door's header covers exactly the cells its span crosses: a double door on the west wall at threshold
    /// (2, 4), across the wall along z (rotation 1), covers two cells.
    #[test]
    fn door_header_covers_the_cells_of_its_span() {
        let door = door_header_cells(Cell::new(2, 4), 1, DoorDraw { span: 2, offset: [0.0, 0.0] });
        assert_eq!(door.len(), 2, "a double door covers two cells: {door:?}");
        assert_eq!(door.iter().map(|c| c.0).collect::<HashSet<_>>(), HashSet::from([2]));
    }

    /// The keys a window's parts take: every slot with each side set its cells can read.
    fn keys(frame: ChunkFrame) -> HashSet<PartKey> {
        let mut out = HashSet::new();
        for sy in 0..WINDOW {
            for sx in 0..WINDOW {
                for (lx, ly) in [(0, 0), (20, 0), (44, 0), (0, 20), (20, 20), (44, 20), (0, 44), (20, 44), (44, 44)] {
                    out.insert(slot_key(frame, sx, sy, side_needs(lx, ly)));
                }
            }
        }
        out
    }

    /// A step east keeps the six chunks that stay, less the parts on the sides whose neighbour changed: the
    /// west parts of the chunks now on the window's west edge and the east parts of the ones that were on its east edge.
    #[test]
    fn a_step_east_rebuilds_only_the_new_chunks_and_the_changed_edges() {
        let a = keys(ChunkFrame { level: 0, center: (0, 0) });
        let b = keys(ChunkFrame { level: 0, center: (1, 0) });
        assert_eq!(a.len(), 81);
        let kept: Vec<_> = b.intersection(&a).collect();
        assert_eq!(kept.len(), 36, "six chunks keep six parts each");
        assert!(kept.iter().all(|k| k.cx == 0 || k.cx == 1));
        assert!(kept.iter().all(|k| k.needs & if k.cx == 0 { WEST } else { EAST } == 0));
        let down = keys(ChunkFrame { level: -1, center: (0, 0) });
        assert!(down.is_disjoint(&a), "another level shares no part");
    }

    /// What a cell's geometry reads in window `f`, at the absolute cell (ax, ay): the walls, districts and rooms
    /// along both axes out to REACH.
    fn reads(f: &Facility, ax: i32, ay: i32) -> Vec<(bool, String, Option<(i32, i32, i32, i32)>)> {
        let mut out = vec![];
        for d in -REACH..=REACH {
            for (x, y) in [(ax + d - f.origin.0, ay - f.origin.1), (ax - f.origin.0, ay + d - f.origin.1)] {
                let room = f
                    .room_at(x as f64 + 0.5, y as f64 + 0.5)
                    .map(|i| &f.rooms[i])
                    .map(|r| (r.x + f.origin.0, r.y + f.origin.1, r.w, r.h));
                out.push((f.is_wall_cell(x, y), f.region_at(x, y).to_string(), room));
            }
        }
        out
    }

    /// On two real windows a step apart, every part both share picks the same cells and rooms, and each of its
    /// cells reads the same walls, districts and rooms out to REACH: the part drawn for one window is right for the other.
    #[test]
    fn a_kept_part_reads_the_same_floor_in_both_windows() {
        crate::world::route::layout::godot_regions_for_this_test();
        use crate::content::loot::LootTables;
        use crate::world::catalog::Catalog;
        use crate::world::infinite::window;
        let cat = Catalog::load();
        let loot = LootTables::default();
        let mut cache = HashMap::new();
        let frames = [ChunkFrame { level: 0, center: (0, 0) }, ChunkFrame { level: 0, center: (1, 0) }];
        let wins: Vec<Facility> = frames
            .iter()
            .map(|fr| window("1426541270", fr.center, fr.level, &cat, &loot, &mut cache).0)
            .collect();
        let abs_parts = |f: &Facility, fr: ChunkFrame| -> HashMap<PartKey, (Vec<(i32, i32)>, Vec<(i32, i32, i32, i32)>)> {
            let parts = window_parts(f, fr);
            let cells: usize = parts.iter().map(|(_, p)| p.cells.len()).sum();
            assert_eq!(cells, drawn_cells(f).len(), "every floor cell in one part");
            let rooms: usize = parts.iter().map(|(_, p)| p.rooms.len()).sum();
            assert_eq!(rooms, f.rooms.len(), "every room in one part");
            parts
                .into_iter()
                .map(|(k, p)| {
                    let cells = p.cells.iter().map(|c| (c.x + f.origin.0, c.y + f.origin.1)).collect();
                    let rooms = p
                        .rooms
                        .iter()
                        .map(|&i| &f.rooms[i])
                        .map(|r| (r.x + f.origin.0, r.y + f.origin.1, r.w, r.h))
                        .collect();
                    (k, (cells, rooms))
                })
                .collect()
        };
        let a = abs_parts(&wins[0], frames[0]);
        let b = abs_parts(&wins[1], frames[1]);
        let mut shared = 0;
        for (k, pa) in &a {
            let Some(pb) = b.get(k) else { continue };
            shared += 1;
            assert_eq!(pa, pb, "part {k:?} picks other cells or rooms");
            for &(ax, ay) in &pa.0 {
                assert_eq!(reads(&wins[0], ax, ay), reads(&wins[1], ax, ay), "cell ({ax}, {ay}) of {k:?}");
            }
        }
        assert_eq!(shared, 36);
    }
}

#[cfg(test)]
mod stream_budget_tests {
    use super::*;

    const STANDARD: &str = r#"{
  "frame_budget_ms": 4.0,
  "per_queued_ms": 1.0,
  "max_budget_ms": 9.0,
  "chunks_per_frame": 4,
  "jump_steps": 1,
  "neighbour_chunks_per_frame": 2,
  "neighbour_near_chunks_per_frame": 4,
  "neighbour_max_parts": 12,
  "neighbour_cull_radius": 14.0
}"#;

    #[test]
    fn the_embedded_stream_table_holds_the_godot_standard_values() {
        let t = stream_table();
        assert_eq!(t.frame_budget_ms, 4.0, "streamed_facility_view_3d.gd STREAM_BUDGET_MS");
        assert_eq!(t.per_queued_ms, 1.0, "streamed_facility_view_3d.gd STREAM_BUDGET_PER_QUEUED_MS");
        assert_eq!(t.max_budget_ms, 9.0, "streamed_facility_view_3d.gd STREAM_BUDGET_MAX_MS");
        assert_eq!(t.chunks_per_frame, 4, "streamed_facility_view_3d.gd STARTUP_CHUNKS_PER_FRAME");
        assert_eq!(t.jump_steps, 1, "streamed_facility_view_3d.gd 726-727: more than one chunk away is a jump");
        assert_eq!(t.neighbour_chunks_per_frame, 2);
        assert_eq!(t.neighbour_near_chunks_per_frame, 4);
        assert!(t.neighbour_near_chunks_per_frame >= t.neighbour_chunks_per_frame, "near an opening the neighbour builds faster");
        assert_eq!(t.neighbour_max_parts, 12);
        assert_eq!(t.neighbour_cull_radius, 14.0);
    }

    #[test]
    fn the_budget_grows_with_the_queue_and_stops_at_the_max() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        assert_eq!(t.budget_ms(0), 4.0);
        assert_eq!(t.budget_ms(3), 7.0);
        assert_eq!(t.budget_ms(20), 9.0, "capped at STREAM_BUDGET_MAX_MS");
    }

    #[test]
    fn a_larger_frame_budget_in_the_data_builds_more_in_one_frame() {
        let standard = StreamBudget::parse(STANDARD).unwrap();
        let roomy = StreamBudget::parse(&STANDARD.replace("\"frame_budget_ms\": 4.0", "\"frame_budget_ms\": 8.0")).unwrap();
        // 5 ms into the frame with one part built: the Standard budget (4 + 1 per queued part) is spent, a roomier one is not
        assert!(!standard.allows(1, 5.0, 0));
        assert!(roomy.allows(1, 5.0, 0));
    }

    #[test]
    fn the_chunks_per_frame_cap_in_the_data_limits_the_builds() {
        let standard = StreamBudget::parse(STANDARD).unwrap();
        let capped = StreamBudget::parse(&STANDARD.replace("\"chunks_per_frame\": 4", "\"chunks_per_frame\": 2")).unwrap();
        assert!(standard.allows(3, 0.0, 9));
        assert!(!capped.allows(2, 0.0, 9));
        assert!(capped.allows(1, 0.0, 9));
    }

    #[test]
    fn the_governors_stream_factor_shrinks_the_time_budget_but_not_the_first_part() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        // 1 is exactly `allows`; the Standard budget with 3 queued is 7 ms
        for (built, elapsed, queued) in [(0usize, 999.0f64, 9usize), (1, 5.0, 0), (1, 6.9, 3), (1, 7.1, 3), (3, 0.0, 9), (4, 0.0, 9)] {
            assert_eq!(t.allows_scaled(built, elapsed, queued, 1.0), t.allows(built, elapsed, queued), "{built} {elapsed} {queued}");
        }
        assert!(t.allows_scaled(1, 6.0, 3, 1.0));
        assert!(!t.allows_scaled(1, 6.0, 3, 0.75), "5.25 ms is spent at 6 ms");
        assert!(t.allows_scaled(1, 5.0, 3, 0.75));
        assert!(!t.allows_scaled(1, 3.6, 3, 0.5));
        assert!(t.allows_scaled(0, 999.0, 9, 0.5), "the first part always builds, whatever the factor");
        assert!(t.allows_scaled(0, 999.0, 9, 0.0));
        assert!(!t.allows_scaled(1, 0.0, 9, 0.0), "a zero factor leaves only the first part");
        // out-of-range factors are clamped, not trusted
        assert_eq!(t.allows_scaled(1, 6.0, 3, 5.0), t.allows(1, 6.0, 3));
        assert!(!t.allows_scaled(1, 0.1, 3, -1.0));
    }

    #[test]
    fn the_first_part_of_a_rebuild_always_builds() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        assert!(t.allows(0, 999.0, 9), "no stall: at least one part per rebuild");
    }

    /// Regression: a first load that the frame budget cut short (macOS, 2026-11-04 crash at world_mesh.rs:296)
    /// has parts the budget did not reach. Drawing them must skip those parts, not ask for them.
    #[test]
    fn a_first_load_the_budget_cut_short_draws_only_the_parts_that_exist() {
        let key = |cx: i32| PartKey { level: 0, cx, cy: 0, needs: 0, open: 0 };
        let wanted: Vec<(PartKey, u8)> = (0..6).map(|cx| (key(cx), 0)).collect();
        let built: std::collections::HashSet<PartKey> = [key(0), key(2), key(3)].into_iter().collect();
        let drawn = drawable_keys(&wanted, |k| built.contains(k));
        assert_eq!(drawn, vec![key(0), key(2), key(3)], "the built parts, in the window's order");
        assert!(drawable_keys(&wanted, |_| false).is_empty(), "nothing built, nothing drawn, no panic");
    }

    #[test]
    fn a_jump_is_more_than_one_chunk_away_as_in_godot() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        let at = |level: i32, x: i32, z: i32| ChunkFrame { level, center: (x, z) };
        assert!(!is_jump(at(0, 5, 5), at(0, 6, 6), t.jump_steps), "a diagonal step to the next chunk is a step");
        assert!(is_jump(at(0, 5, 5), at(0, 7, 5), t.jump_steps), "two chunks in one frame is a teleport");
        assert!(is_jump(at(0, 5, 5), at(0, 5, 2), t.jump_steps), "a teleport along z is a jump too");
    }

    #[test]
    fn a_level_change_is_not_a_jump() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        let at = |level: i32, x: i32, z: i32| ChunkFrame { level, center: (x, z) };
        assert!(!is_jump(at(0, 5, 5), at(1, 5, 5), t.jump_steps), "the level swap is relabel's, not a rebuild");
        assert!(!is_jump(at(0, 5, 5), at(-1, 6, 5), t.jump_steps));
    }

    #[test]
    fn a_larger_jump_steps_in_the_data_changes_which_moves_are_jumps() {
        let standard = StreamBudget::parse(STANDARD).unwrap();
        let roomy = StreamBudget::parse(&STANDARD.replace("\"jump_steps\": 1", "\"jump_steps\": 3")).unwrap();
        let (from, to) = (ChunkFrame { level: 0, center: (5, 5) }, ChunkFrame { level: 0, center: (7, 5) });
        assert!(is_jump(from, to, standard.jump_steps));
        assert!(!is_jump(from, to, roomy.jump_steps));
    }

    #[test]
    fn a_teleport_builds_its_window_in_one_step_and_a_step_streams() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        let at = |level: i32, x: i32, z: i32| ChunkFrame { level, center: (x, z) };
        assert!(!builds_in_one_step(None, at(0, 5, 5), 0, t.jump_steps), "the first load of a run streams");
        assert!(builds_in_one_step(None, at(0, 5, 5), 3, t.jump_steps), "back onto the window after leaving the floor");
        assert!(!builds_in_one_step(Some(at(0, 5, 5)), at(0, 6, 5), 3, t.jump_steps), "a step streams");
        assert!(builds_in_one_step(Some(at(0, 5, 5)), at(0, 9, 5), 3, t.jump_steps), "a teleport builds in one step");
    }

    #[test]
    fn a_teleport_still_builds_in_one_step() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        let at = |level: i32, x: i32, z: i32| ChunkFrame { level, center: (x, z) };
        assert!(builds_in_one_step(Some(at(0, 5, 5)), at(0, 9, 5), 3, t.jump_steps));
        assert!(!builds_in_one_step(Some(at(0, 5, 5)), at(2, 5, 5), 3, t.jump_steps), "a level change streams");
    }

    #[test]
    fn a_bad_jump_steps_is_refused() {
        assert!(StreamBudget::parse(&STANDARD.replace("\"jump_steps\": 1", "\"jump_steps\": 0")).is_err());
        assert!(StreamBudget::parse(&STANDARD.replace("\"jump_steps\": 1", "\"jump_steps\": 1.5")).is_err());
    }

    #[test]
    fn a_table_with_a_bad_value_is_refused() {
        assert!(StreamBudget::parse(&STANDARD.replace("\"max_budget_ms\": 9.0", "\"max_budget_ms\": 0")).is_err());
        assert!(StreamBudget::parse(&STANDARD.replace("\"chunks_per_frame\": 4", "\"chunks_per_frame\": 0")).is_err());
        assert!(StreamBudget::parse(&STANDARD.replace("\"per_queued_ms\": 1.0,", "")).is_err());
    }
}

#[cfg(test)]
mod stack_mesh_tests {
    use super::*;
    use crate::render::stairs::{Face, Parts, Site};
    use crate::world::facility::{Cell, Stair};
    use crate::world::flight::FlightShape;

    fn mesh(ys: &[f32]) -> Mesh {
        let vertices = ys
            .iter()
            .map(|&y| Vertex {
                position: vec3(1.0, y, 2.0),
                uv: vec2(0., 0.),
                color: [255; 4],
                normal: vec4(0., 1., 0., 0.),
            })
            .collect::<Vec<_>>();
        let n = vertices.len() as u16;
        Mesh { vertices, indices: (0..n).collect(), texture: None }
    }

    fn lamp(y: f32) -> Lamp {
        Lamp { pos: vec3(0., y, 0.), range: 5.0, color: WHITE, energy: 1.0, flicker: Flicker::NONE }
    }

    fn min_y(m: &WorldMeshes) -> f32 {
        m.meshes.iter().flat_map(|m| m.vertices.iter()).map(|v| v.position.y).fold(f32::MAX, f32::min)
    }

    #[test]
    fn shift_y_moves_bounds_lamps_and_doors() {
        let mut w = WorldMeshes::default();
        w.meshes = vec![mesh(&[0.0, 5.0, 0.0])];
        w.bounds = mesh_bounds(&w.meshes);
        w.lamps = vec![lamp(4.0)];
        w.doors = vec![DoorInstance {
            cell: Cell::new(1, 1),
            model: String::new(),
            xf: Mat4::from_translation(vec3(3.0, 1.0, 3.0)),
            fit: door::DoorFit { sx: 1.0, sy: 1.0, shift: 0.0, from: 0.0, jamb: 0.5 },
        }];
        w.shift_y(5.0);
        assert_eq!(w.dy, 5.0);
        assert_eq!(min_y(&w), 5.0, "neighbour_vertices_are_offset_by_storey");
        assert_eq!((w.bounds[0].0.y, w.bounds[0].1.y), (5.0, 10.0));
        assert_eq!(w.lamps[0].pos.y, 9.0);
        assert_eq!(w.doors[0].xf.w_axis.y, 6.0);
        assert_eq!(w.doors[0].xf.w_axis.x, 3.0, "only the height moves");
        w.shift_y(-5.0);
        assert_eq!(min_y(&w), 0.0);
        assert_eq!(w.dy, 0.0);
        assert_eq!(w.lamps[0].pos.y, 4.0);
    }

    #[test]
    fn shift_y_reframes_cached_parts_too() {
        let mut w = WorldMeshes::default();
        let key = PartKey { level: 0, cx: 0, cy: 0, needs: 0, open: 0 };
        w.parts.insert(key, Part { meshes: vec![mesh(&[0.0])], origin: (0, 0), dy: 0.0, used: 1 });
        w.shift_y(-5.0);
        let p = &w.parts[&key];
        assert_eq!(p.dy, -5.0);
        assert_eq!(p.meshes[0].vertices[0].position.y, -5.0);
        assert_eq!(w.held_parts(), 1);
    }

    /// A drawn set at window origin `at`: one mesh with a vertex at (1, y, 2), its box, and one door leaf.
    fn drawn_at(at: (i32, i32)) -> WorldMeshes {
        let mut w = WorldMeshes::default();
        w.meshes = vec![mesh(&[0.0, 3.0]), Mesh { vertices: vec![], indices: vec![], texture: None }];
        w.drawn = vec![None, None];
        w.drawn_origin = at;
        w.bounds = mesh_bounds(&w.meshes);
        w.doors = vec![DoorInstance {
            cell: Cell::new(10, 20),
            model: String::new(),
            xf: Mat4::from_translation(vec3(10.5, 1.0, 20.5)),
            fit: door::DoorFit { sx: 1.0, sy: 1.0, shift: 0.0, from: 0.0, jamb: 0.5 },
        }];
        w
    }

    #[test]
    fn the_drawn_set_follows_a_recentre_while_parts_are_pending() {
        // the runner crossed a chunk east: the window origin moved CHUNK cells, so old geometry sits CHUNK cells west
        // of where it is now, in the new window's coordinates
        let mut w = drawn_at((0, 0));
        w.follow_origin((CHUNK, 0));
        assert_eq!(w.drawn_origin, (CHUNK, 0));
        let c = CHUNK as f32;
        assert_eq!(w.meshes[0].vertices[0].position.x, 1.0 - c);
        assert_eq!(w.meshes[0].vertices[0].position.z, 2.0);
        assert_eq!((w.bounds[0].0.x, w.bounds[0].1.x), (1.0 - c, 1.0 - c));
        assert_eq!((w.bounds[0].0.y, w.bounds[0].1.y), (0.0, 3.0), "height is untouched");
        assert!(w.bounds[1].0.x > w.bounds[1].1.x, "an empty mesh keeps its inverted box");
        assert_eq!(w.doors[0].xf.w_axis.x, 10.5 - c);
        assert_eq!(w.doors[0].xf.w_axis.z, 20.5);
        assert_eq!(w.doors[0].cell, Cell::new(10 - CHUNK, 20));
        // the bounds still describe the vertices
        assert_eq!(w.bounds[0], mesh_bounds(&w.meshes)[0]);
    }

    #[test]
    fn following_the_same_origin_changes_nothing_and_a_return_trip_restores_it() {
        let mut w = drawn_at((CHUNK, -CHUNK));
        w.follow_origin((CHUNK, -CHUNK));
        assert_eq!(w.meshes[0].vertices[0].position, vec3(1.0, 0.0, 2.0));
        w.follow_origin((2 * CHUNK, CHUNK));
        w.follow_origin((CHUNK, -CHUNK));
        assert_eq!(w.meshes[0].vertices[0].position, vec3(1.0, 0.0, 2.0));
        assert_eq!(w.doors[0].cell, Cell::new(10, 20));
        assert_eq!(w.doors[0].xf.w_axis.x, 10.5);
    }

    #[test]
    fn parts_returned_after_a_follow_are_reframed_from_where_their_vertices_are() {
        let key = PartKey { level: 0, cx: 0, cy: 0, needs: 0, open: 0 };
        let mut w = WorldMeshes::default();
        // a part drained into the drawn set at origin (0, 0); its stale origin is that same (0, 0)
        w.parts.insert(key, Part { meshes: vec![], origin: (0, 0), dy: 0.0, used: 1 });
        w.meshes = vec![mesh(&[0.0])];
        w.drawn = vec![Some(key)];
        w.drawn_origin = (0, 0);
        w.bounds = mesh_bounds(&w.meshes);
        // two recentres while the build is pending
        w.follow_origin((CHUNK, 0));
        w.follow_origin((CHUNK, CHUNK));
        w.return_drawn_to_parts();
        let p = w.parts.get_mut(&key).unwrap();
        assert_eq!(p.origin, (CHUNK, CHUNK), "the part's origin is the one its vertices were moved to");
        // the finalize step reframes by (part.origin - target): the vertices are already at the target, so no move
        let origin = p.origin;
        reframe(&mut p.meshes, origin, (CHUNK, CHUNK));
        assert_eq!(p.meshes[0].vertices[0].position, vec3(1.0 - CHUNK as f32, 0.0, 2.0 - CHUNK as f32));
        // and a further recentre of the finished window moves them on by one chunk, once
        reframe(&mut p.meshes, origin, (2 * CHUNK, CHUNK));
        assert_eq!(p.meshes[0].vertices[0].position.x, 1.0 - 2.0 * CHUNK as f32);
    }

    #[test]
    fn an_empty_mesh_keeps_its_inverted_box_through_a_shift() {
        let mut w = WorldMeshes::default();
        w.meshes = vec![Mesh { vertices: vec![], indices: vec![], texture: None }];
        w.bounds = mesh_bounds(&w.meshes);
        w.shift_y(5.0);
        assert!(w.bounds[0].0.y > w.bounds[0].1.y);
    }

    #[test]
    fn triangle_count_counts_the_drawn_indices() {
        let mut w = WorldMeshes::default();
        w.meshes = vec![mesh(&[0.0; 6]), mesh(&[0.0; 3])];
        assert_eq!(w.triangle_count(), 3);
    }

    #[test]
    fn lift_places_vertices_at_the_neighbour_height() {
        let mut ms = vec![mesh(&[0.0, 0.0])];
        lift(&mut ms, 5.0);
        assert_eq!(ms[0].vertices[0].position.y, 5.0);
        lift(&mut ms, -5.0);
        assert_eq!(ms[0].vertices[0].position.y, 0.0);
    }

    fn flight(dir: i32) -> Stair {
        Stair::stairs(Cell::new(20, 20), dir, (0, 1), FlightShape::Switchback, false)
    }

    fn parts_of(s: Stair, level: i32) -> Parts {
        let well: HashSet<(i32, i32)> = s.well_cells().iter().map(|c| (c.x, c.y)).collect();
        let solid = |_: i32, _: i32| false;
        let in_well = move |x: i32, z: i32| well.contains(&(x, z));
        let site = Site { h: 5.0, wall_h: 5.0, tiers: 1.0, well: &in_well, solid: &solid };
        let mut out = Parts::default();
        for c in s.well_cells() {
            stairs::flight_cell(&mut out, (c.x, c.y), s, level, &site);
        }
        out
    }

    #[test]
    fn no_ceiling_over_a_well() {
        let s = flight(1);
        assert!(!draws_ceiling(Some(s)), "the lower level's well cell has no ceiling quad");
        assert!(draws_ceiling(None));
        assert!(draws_ceiling(Some(flight(-1))), "the upper level keeps its own ceiling");
        let p = parts_of(s, -1);
        assert!(!p.quads.iter().any(|q| q.n.y < -0.5 && q.face != Face::Iron), "the flight adds no cap under the level above (a rail's underside is iron)");
        assert!(!p.quads.is_empty(), "the lower level draws its treads");
    }

    #[test]
    fn no_floor_over_a_well_on_the_upper_level() {
        let p = parts_of(flight(-1), 0);
        assert!(
            !p.quads.iter().any(|q| q.n.y > 0.5 && q.c.iter().all(|v| v.y.abs() < 0.01)),
            "no floor quad at y = 0 over the hole"
        );
        assert!(p.quads.iter().all(|q| q.face != Face::Plain || q.n.y.abs() < 0.5), "no treads on the upper level");
    }

    /// The first window near the origin of `level` whose facility passes `want`, and its centre chunk.
    fn find_window(level: i32, want: impl Fn(&Facility) -> bool) -> (Facility, (i32, i32)) {
        use crate::content::loot::LootTables;
        use crate::world::catalog::Catalog;
        use crate::world::infinite::window;
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let mut cache = HashMap::new();
        for r in 0..8i32 {
            for cx in -r..=r {
                for cy in -r..=r {
                    if cx.abs().max(cy.abs()) != r {
                        continue;
                    }
                    let (f, _) = window("1426541270", (cx, cy), level, &cat, &loot, &mut cache);
                    if want(&f) {
                        return (f, (cx, cy));
                    }
                }
            }
        }
        panic!("no level {level} window near the origin with the wanted feature");
    }

    #[test]
    fn no_ceiling_over_a_ladder_foot() {
        let (f, _) = find_window(SEWER_LEVEL, |f| f.stairs.iter().any(|s| s.is_ladder() && s.dir > 0));
        let mut seen = 0;
        for s in f.stairs.iter().filter(|s| s.is_ladder() && s.dir > 0) {
            let d = cell_draws(&f, s.cell, Some(*s));
            assert!(!d.ceiling, "the shaft is open above the foot at {:?}", s.cell);
            assert!(d.floor && d.walls, "the foot is an ordinary floor cell");
            seen += 1;
        }
        assert!(seen > 0);
        assert!(cell_draws(&f, f.start, None).ceiling, "an ordinary cell keeps its ceiling");
    }

    #[test]
    fn no_floor_over_a_ladder_hatch() {
        let (f, _) = find_window(0, |f| f.stairs.iter().any(|s| s.is_ladder() && s.dir < 0));
        for s in f.stairs.iter().filter(|s| s.is_ladder() && s.dir < 0) {
            let d = cell_draws(&f, s.cell, Some(*s));
            assert!(!d.floor, "the hatch at {:?} is a hole, drawn by ladder_down", s.cell);
            assert!(d.ceiling, "the upper level keeps its ceiling over the hatch");
        }
        assert!(cell_draws(&f, f.start, None).floor);
    }

    #[test]
    fn no_floor_or_wall_over_a_drop() {
        let (f, _) = find_window(SEWER_LEVEL, |f| !f.drop_cells().is_empty());
        for &c in f.drop_cells() {
            let d = cell_draws(&f, c, None);
            assert!(!d.floor && !d.walls, "the void at {c:?} is open: no floor quad, no wall columns");
        }
        assert!(cell_draws(&f, f.start, None).floor && cell_draws(&f, f.start, None).walls);
    }

    #[test]
    fn no_ceiling_over_the_pool_under_a_drop() {
        let (_, centre) = find_window(SEWER_LEVEL, |f| !f.drop_cells().is_empty());
        use crate::content::loot::LootTables;
        use crate::world::catalog::Catalog;
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let (low, _) = crate::world::infinite::window("1426541270", centre, SEWER_LOWER_LEVEL, &cat, &loot, &mut HashMap::new());
        let air: Vec<Cell> = (0..low.rows)
            .flat_map(|y| (0..low.cols).map(move |x| Cell::new(x, y)))
            .filter(|c| low.is_air_above(*c))
            .collect();
        assert!(!air.is_empty(), "the pool under the void is air above");
        assert!(air.iter().all(|c| !cell_draws(&low, *c, None).ceiling), "no ceiling quad over the pool");
        assert!(cell_draws(&low, low.start, None).ceiling, "the rest of level -2 keeps its ceiling");
    }

    #[test]
    fn a_drop_is_an_opening_for_the_neighbour() {
        let (f, _) = find_window(SEWER_LEVEL, |f| !f.drop_cells().is_empty());
        let ops = openings(&f);
        let drops = f.drop_cells();
        for c in drops {
            assert!(ops.iter().any(|o| o.cells.contains(c)), "the void cell {c:?} is in an opening");
        }
        for o in ops.iter().filter(|o| o.cells.iter().any(|c| drops.contains(c))) {
            assert_eq!(o.upper_level, f.level);
            let (lo, hi) = o.cells.iter().fold(((f32::MAX, f32::MAX), (f32::MIN, f32::MIN)), |(lo, hi), c| {
                let p = c.center();
                ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1)))
            });
            assert!(o.centre.0 >= lo.0 && o.centre.0 <= hi.0 && o.centre.1 >= lo.1 && o.centre.1 <= hi.1);
        }
        // a patch is one connected void: two openings never share a cell
        let total: usize = ops.iter().filter(|o| o.cells.iter().all(|c| drops.contains(c))).map(|o| o.cells.len()).sum();
        assert_eq!(total, drops.len(), "every drop cell belongs to exactly one opening");
    }

    fn sample_window() -> (Facility, ChunkFrame) {
        use crate::content::loot::LootTables;
        use crate::world::catalog::Catalog;
        use crate::world::infinite::window;
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let mut cache = HashMap::new();
        for cx in -4..4 {
            for cy in -4..4 {
                let (f, _) = window("1426541270", (cx, cy), 0, &cat, &loot, &mut cache);
                if f.stairs.iter().any(|s| s.dir < 0 && !s.is_ladder()) {
                    return (f, ChunkFrame { level: 0, center: (cx, cy) });
                }
            }
        }
        panic!("no level 0 window with a flight down near the origin");
    }

    fn cells_of(parts: &[(PartKey, Pick)]) -> usize {
        parts.iter().map(|(_, p)| p.cells.len()).sum()
    }

    fn dist(k: &PartKey, frame: ChunkFrame, ops: &[Opening]) -> f32 {
        let x0 = ((k.cx - (frame.center.0 - 1)) * CHUNK) as f32;
        let z0 = ((k.cy - (frame.center.1 - 1)) * CHUNK) as f32;
        ops.iter().map(|o| rect_distance(o.centre, (x0, z0, x0 + CHUNK as f32, z0 + CHUNK as f32))).fold(f32::MAX, f32::min)
    }

    #[test]
    fn neighbour_builds_only_parts_near_an_opening() {
        let (f, frame) = sample_window();
        let ops = openings(&f);
        assert!(!ops.is_empty());
        assert!(ops.iter().all(|o| o.upper_level == 0 && !o.cells.is_empty()));
        let all = window_parts(&f, frame);
        let total = cells_of(&all);
        let near = neighbour_wanted(all, frame, &ops, 14.0, 99);
        assert!(!near.is_empty() && cells_of(&near) < total, "a radius cuts the window down");
        assert!(near.iter().all(|(k, _)| dist(k, frame, &ops) <= 14.0));
        let tight = neighbour_wanted(window_parts(&f, frame), frame, &ops, 0.0, 99);
        assert!(!tight.is_empty(), "the part holding the opening is always wanted");
    }

    /// The part holding the well is built first (nearest first, distance 0 inside the opening's own part), so the pit is
    /// filled before the rest of the neighbour.
    #[test]
    fn the_first_wanted_part_holds_the_well() {
        let (f, frame) = sample_window();
        let ops = openings(&f);
        let wanted = neighbour_wanted(window_parts(&f, frame), frame, &ops, 1000.0, 99);
        assert!(wanted.len() > 1);
        assert_eq!(dist(&wanted[0].0, frame, &ops), 0.0, "the first part contains an opening centre");
        for w in wanted.windows(2) {
            assert!(dist(&w[0].0, frame, &ops) <= dist(&w[1].0, frame, &ops), "nearest first");
        }
    }

    #[test]
    fn neighbour_max_parts_is_a_hard_cap() {
        let (f, frame) = sample_window();
        let ops = openings(&f);
        let many = neighbour_wanted(window_parts(&f, frame), frame, &ops, 1000.0, 99);
        assert!(many.len() > 3);
        assert_eq!(neighbour_wanted(window_parts(&f, frame), frame, &ops, 1000.0, 3).len(), 3);
        assert!(neighbour_wanted(window_parts(&f, frame), frame, &ops, 1000.0, 0).is_empty());
    }

    /// Cells stand in for triangles here (they need no GL textures to build); `triangle_count` is the real count.
    #[test]
    fn neighbour_triangles_at_most_35_percent_of_the_active_level() {
        let (f, frame) = sample_window();
        let ops = openings(&f);
        let all = cells_of(&window_parts(&f, frame));
        let t = stream_table();
        let near = cells_of(&neighbour_wanted(window_parts(&f, frame), frame, &ops, t.neighbour_cull_radius, t.neighbour_max_parts));
        assert!((near as f32) <= all as f32 * 0.35, "{near} of {all} cells");
    }

    #[test]
    fn an_upper_floor_hatch_pushes_no_lamp() {
        use crate::world::facility::LinkKind;
        use crate::world::flight::FlightShape;
        let s = |dir, kind| Stair { cell: Cell::new(3, 3), dir, run: (0, 1), kind, shape: FlightShape::Straight, clockwise: false };
        assert!(!owns_lamp(&s(-1, LinkKind::Ladder(crate::world::facility::Cover::Manhole))), "the hatch lights nothing");
        assert!(owns_lamp(&s(1, LinkKind::Ladder(crate::world::facility::Cover::Manhole))), "a ladder foot does");
        assert!(owns_lamp(&s(1, LinkKind::Stairs)));
        assert!(!owns_lamp(&s(-1, LinkKind::Stairs)));
    }

    #[test]
    fn the_neighbour_budget_follows_the_data() {
        let t = StreamBudget::parse(
            r#"{"frame_budget_ms":4.0,"per_queued_ms":1.0,"max_budget_ms":9.0,"chunks_per_frame":4,"jump_steps":1,
            "neighbour_chunks_per_frame":2,"neighbour_near_chunks_per_frame":4,"neighbour_max_parts":12,"neighbour_cull_radius":14.0}"#,
        )
        .unwrap();
        assert!(neighbour_allows(&t, 2, 0, 999.0, 5, 0.0), "the first always builds");
        assert!(neighbour_allows(&t, 2, 1, 0.0, 5, 1.0));
        assert!(!neighbour_allows(&t, 2, 1, 5.0, 5, 0.5), "a governor scale below 1 cuts the neighbour budget");
        assert!(neighbour_allows(&t, 2, 1, 5.0, 5, 1.0));
        assert!(!neighbour_allows(&t, 2, 2, 0.0, 5, 1.0), "the per-frame cap");
        let old = r#"{"frame_budget_ms":4.0,"per_queued_ms":1.0,"max_budget_ms":9.0,"chunks_per_frame":4,"jump_steps":1}"#;
        assert!(StreamBudget::parse(old).is_err(), "the neighbour keys are required");
    }

    // -----------------------------------------------------------------------------------------------------------
    // The slab: no two surfaces of different levels share a plane (render/seams.rs)
    // -----------------------------------------------------------------------------------------------------------

    /// One axis-aligned rectangle of a level's drawn shell, in absolute y. `axis` is the normal's axis (0 x, 1 y,
    /// 2 z); `lo`/`hi` span the other two axes in x, y, z order.
    #[derive(Clone, Copy, Debug)]
    struct Rect {
        level: i32,
        what: &'static str,
        axis: usize,
        at: f32,
        lo: (f32, f32),
        hi: (f32, f32),
    }

    /// Godot's `coplanar_scan.gd` tolerance (metres) and the least overlap (per side) that counts as a shared surface.
    const COPLANAR_TOL: f32 = 0.002;
    const MIN_OVERLAP: f32 = 0.02;

    fn rect_of_points(level: i32, what: &'static str, pts: &[Vec3], lift: f32) -> Option<Rect> {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in pts {
            let p = vec3(p.x, p.y + lift, p.z);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let ext = hi - lo;
        let axis = (0..3).find(|&a| ext[a] < 1e-4)?;
        let (u, v) = match axis {
            0 => ((lo.y, lo.z), (hi.y, hi.z)),
            1 => ((lo.x, lo.z), (hi.x, hi.z)),
            _ => ((lo.x, lo.y), (hi.x, hi.y)),
        };
        Some(Rect { level, what, axis, at: lo[axis], lo: u, hi: v })
    }

    /// The shell of the facility's drawn cells as rectangles: floors, ceilings, wall columns and slab rims through
    /// the same `cell_draws` / `edge_fill` / `seams` the mesh build uses, plus the quads of its flights and ladders.
    fn shell(f: &Facility) -> Vec<Rect> {
        let lift = f.level as f32 * crate::world::vertical::STOREY;
        let steps = stair_steps(f);
        let mut out = vec![];
        for c in drawn_cells(f) {
            let (x, z) = (c.x as f32, c.y as f32);
            let wall_h = cell_wall_height(f, c.x, c.y);
            let step = steps.get(&(c.x, c.y)).copied();
            let draw = cell_draws(f, c, step);
            let foot = step.filter(|s| s.is_ladder() && s.dir > 0);
            let step = step.filter(|s| !(s.is_ladder() && s.dir > 0));
            let flat = |y: f32| [vec3(x, y, z), vec3(x + 1., y, z), vec3(x + 1., y, z + 1.), vec3(x, y, z + 1.)];
            if draw.floor && step.is_none() {
                out.extend(rect_of_points(f.level, "floor", &flat(0.0), lift));
            }
            if draw.ceiling {
                out.extend(rect_of_points(f.level, "ceiling", &flat(seams::ceiling_y(wall_h)), lift));
            }
            for (dx, dz) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                // the edge's two ground points
                let (a, b) = match (dx, dz) {
                    (0, -1) => (vec3(x + 1., 0., z), vec3(x, 0., z)),
                    (0, 1) => (vec3(x, 0., z + 1.), vec3(x + 1., 0., z + 1.)),
                    (-1, 0) => (vec3(x, 0., z), vec3(x, 0., z + 1.)),
                    _ => (vec3(x + 1., 0., z + 1.), vec3(x + 1., 0., z)),
                };
                let (lo, hi, what) = match edge_fill(f, &steps, c, (dx, dz), draw, wall_h) {
                    EdgeFill::None => continue,
                    EdgeFill::Wall { top } => (0.0, top, "wall"),
                    EdgeFill::Rim { lo, hi } => (lo, hi, "rim"),
                };
                out.extend(rect_of_points(
                    f.level,
                    what,
                    &[vec3(a.x, lo, a.z), vec3(b.x, lo, b.z), vec3(a.x, hi, a.z), vec3(b.x, hi, b.z)],
                    lift,
                ));
            }
            let site = Site {
                h: crate::world::vertical::STOREY,
                wall_h,
                tiers: 1.0,
                well: &|x, z| steps.contains_key(&(x, z)),
                solid: &|x, z| f.is_wall_cell(x, z),
            };
            let mut bits = Parts::default();
            match step {
                Some(s) if s.is_ladder() => stairs::ladder_down(&mut bits, (c.x, c.y), s, &site),
                Some(s) => stairs::flight_cell(&mut bits, (c.x, c.y), s, f.level, &site),
                None => {}
            }
            if let Some(s) = foot {
                stairs::ladder_up(&mut bits, (c.x, c.y), s, &site);
            }
            for q in &bits.quads {
                out.extend(rect_of_points(f.level, "stairs", &q.c, lift));
            }
        }
        out
    }

    /// Every pair of rectangles from different levels that lie in one plane (within Godot's tolerance) and overlap.
    fn coplanar_pairs(a: &[Rect], b: &[Rect]) -> Vec<(Rect, Rect)> {
        let mut hits = vec![];
        for p in a {
            for q in b {
                if p.axis != q.axis || (p.at - q.at).abs() > COPLANAR_TOL {
                    continue;
                }
                let du = p.hi.0.min(q.hi.0) - p.lo.0.max(q.lo.0);
                let dv = p.hi.1.min(q.hi.1) - p.lo.1.max(q.lo.1);
                if du > MIN_OVERLAP && dv > MIN_OVERLAP {
                    hits.push((*p, *q));
                }
            }
        }
        hits
    }

    fn window_at(level: i32, centre: (i32, i32)) -> Facility {
        use crate::content::loot::LootTables;
        use crate::world::catalog::Catalog;
        let (cat, loot) = (Catalog::load(), LootTables::default());
        crate::world::infinite::window("1426541270", centre, level, &cat, &loot, &mut HashMap::new()).0
    }

    /// The stacked pairs the seam is checked on: (the upper window, the window right under it).
    fn stacked_samples() -> Vec<(&'static str, Facility, Facility)> {
        let mut out = vec![];
        let (up, centre) = find_window(0, |f| f.stairs.iter().any(|s| s.dir < 0 && !s.is_ladder()));
        out.push(("flight", up, window_at(SEWER_LEVEL, centre)));
        let (up, centre) = find_window(0, |f| f.stairs.iter().any(|s| s.dir < 0 && s.is_ladder()));
        out.push(("ladder", up, window_at(SEWER_LEVEL, centre)));
        let (up, centre) = find_window(SEWER_LEVEL, |f| !f.drop_cells().is_empty());
        out.push(("drop", up, window_at(SEWER_LOWER_LEVEL, centre)));
        out
    }

    /// The headline: with both levels of a stack built, no surface of the upper level shares a plane with a surface of
    /// the lower one. On HEAD the lower ceiling (y = 5.0) and the upper floor (y = 5.0) were the same rectangle.
    #[test]
    fn no_two_surfaces_of_different_levels_are_coplanar() {
        for (what, up, low) in stacked_samples() {
            let (a, b) = (shell(&up), shell(&low));
            assert!(a.iter().any(|r| r.what == "floor") && b.iter().any(|r| r.what == "ceiling"), "{what}: both shells have surfaces");
            let hits = coplanar_pairs(&a, &b);
            assert!(hits.is_empty(), "{what}: {} coplanar pairs, first {:?}", hits.len(), hits.first());
        }
    }

    /// The seam in numbers: every ceiling of the lower level is at least slab + inset under every floor of the upper
    /// one that lies over it.
    #[test]
    fn ceilings_sit_the_slab_and_inset_under_the_floor_above() {
        for (what, up, low) in stacked_samples() {
            let (a, b) = (shell(&up), shell(&low));
            let mut over = 0;
            for floor in a.iter().filter(|r| r.what == "floor") {
                for ceil in b.iter().filter(|r| r.what == "ceiling") {
                    let du = floor.hi.0.min(ceil.hi.0) - floor.lo.0.max(ceil.lo.0);
                    let dv = floor.hi.1.min(ceil.hi.1) - floor.lo.1.max(ceil.lo.1);
                    if du > MIN_OVERLAP && dv > MIN_OVERLAP {
                        over += 1;
                        assert!(
                            floor.at - ceil.at >= seams::seam_gap() - 1e-4,
                            "{what}: ceiling {} under floor {} leaves under the {} gap",
                            ceil.at,
                            floor.at,
                            seams::seam_gap()
                        );
                    }
                }
            }
            assert!(over > 0, "{what}: some floor lies over some ceiling");
        }
    }

    /// An opening through the slab is lined from the floor of its level to the floor above on every side: a cell with
    /// no ceiling (a shaft, a ladder foot, the pool under the void) takes its walls to the storey, and a ceilinged
    /// neighbour of one has a slab rim from its ceiling to the storey. No opening edge leaves the gap bare.
    #[test]
    fn every_opening_edge_is_lined_through_the_slab() {
        use crate::world::vertical::STOREY;
        let (flight_up, _) = find_window(0, |f| f.stairs.iter().any(|s| s.dir < 0 && !s.is_ladder()));
        let lows: Vec<Facility> = stacked_samples().into_iter().map(|(_, _, low)| low).chain([flight_up]).collect();
        let (mut walls, mut rims) = (0, 0);
        for f in &lows {
            let steps = stair_steps(f);
            for c in drawn_cells(f) {
                let step = steps.get(&(c.x, c.y)).copied();
                let draw = cell_draws(f, c, step);
                let wall_h = cell_wall_height(f, c.x, c.y);
                for d in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                    let n = Cell::new(c.x + d.0, c.y + d.1);
                    match edge_fill(f, &steps, c, d, draw, wall_h) {
                        EdgeFill::Wall { top } if !draw.ceiling => {
                            assert_eq!(top, STOREY, "{c:?} {d:?}: a shaft wall rises to the floor above");
                            walls += 1;
                        }
                        EdgeFill::Wall { top } => assert_eq!(top, wall_h),
                        EdgeFill::Rim { lo, hi } => {
                            assert!(draw.ceiling && f.open_above(n), "{c:?} {d:?}: a rim only over an open neighbour");
                            assert_eq!((lo, hi), (seams::ceiling_y(wall_h), STOREY));
                            rims += 1;
                        }
                        EdgeFill::None => {
                            // bare only where there is nothing to see: no opening beside it
                            let beside = draw.ceiling && f.open_above(n) && !f.is_wall_cell(n.x, n.y);
                            assert!(!beside, "{c:?} {d:?}: a ceilinged cell beside an opening has its rim");
                        }
                    }
                }
            }
        }
        assert!(walls > 0 && rims > 0, "the samples have shaft walls ({walls}) and slab rims ({rims})");
    }

    /// The falls void on level -1: the pool cells under it (level -2, air above) line their walls to the storey, so
    /// the opening between the ceiling of -2 and the lip of -1 is closed all round (the sewer ceiling may be lower
    /// than the storey, and the gap then is wide).
    #[test]
    fn the_pool_under_the_falls_void_is_lined_to_the_floor_above() {
        use crate::world::vertical::STOREY;
        let (_, centre) = find_window(SEWER_LEVEL, |f| !f.drop_cells().is_empty());
        let low = window_at(SEWER_LOWER_LEVEL, centre);
        let steps = stair_steps(&low);
        let air: Vec<Cell> = drawn_cells(&low).into_iter().filter(|c| low.is_air_above(*c)).collect();
        assert!(!air.is_empty());
        let (mut lined, mut rimmed) = (0, 0);
        for c in air {
            let draw = cell_draws(&low, c, None);
            assert!(!draw.ceiling);
            for d in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                let n = Cell::new(c.x + d.0, c.y + d.1);
                if low.is_wall_cell(n.x, n.y) {
                    assert_eq!(edge_fill(&low, &steps, c, d, draw, STOREY), EdgeFill::Wall { top: STOREY }, "{c:?} {d:?}");
                    lined += 1;
                } else if !low.is_air_above(n) {
                    // an open cell with its own ceiling beside the pool: its rim closes the gap
                    let wall_h = cell_wall_height(&low, n.x, n.y);
                    let back = (-d.0, -d.1);
                    let fill = edge_fill(&low, &steps, n, back, cell_draws(&low, n, steps.get(&(n.x, n.y)).copied()), wall_h);
                    assert!(matches!(fill, EdgeFill::Rim { hi, .. } if hi == STOREY), "{n:?} -> {c:?}: {fill:?}");
                    rimmed += 1;
                }
            }
        }
        assert!(lined + rimmed > 0, "the pool has a perimeter");
    }

    /// Flights, ladders and their rails never rise above the floor above, nor sink below their own floor. (The glow
    /// nosing of the top tread sits 1 to 3.5 cm on the tread, a box well clear of any plane, and is not held to it.)
    #[test]
    fn stair_parts_stay_between_their_floor_and_the_floor_above() {
        use crate::world::vertical::STOREY;
        for (what, up, low) in stacked_samples() {
            for f in [&up, &low] {
                let steps = stair_steps(f);
                for c in drawn_cells(f).into_iter().filter(|c| steps.contains_key(&(c.x, c.y))) {
                    let s = steps[&(c.x, c.y)];
                    let wall_h = cell_wall_height(f, c.x, c.y);
                    let site = Site {
                        h: STOREY,
                        wall_h,
                        tiers: 1.0,
                        well: &|x, z| steps.contains_key(&(x, z)),
                        solid: &|x, z| f.is_wall_cell(x, z),
                    };
                    let mut bits = Parts::default();
                    match s {
                        s if s.is_ladder() && s.dir > 0 => stairs::ladder_up(&mut bits, (c.x, c.y), s, &site),
                        s if s.is_ladder() => stairs::ladder_down(&mut bits, (c.x, c.y), s, &site),
                        s => stairs::flight_cell(&mut bits, (c.x, c.y), s, f.level, &site),
                    }
                    let ys = bits
                        .quads
                        .iter()
                        .flat_map(|q| q.c.iter().map(|v| v.y))
                        .chain(bits.boxes.iter().flat_map(|(c, s, _)| [c.y - s.y * 0.5, c.y + s.y * 0.5]));
                    for y in ys {
                        assert!((-1e-4..=STOREY + 1e-4).contains(&y), "{what} {:?}: a part at y {y}", s.cell);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod sewer_lamp_tests {
    use super::*;
    use crate::content::loot::LootTables;
    use crate::render::{lamp_flicker, shader, world_stack};
    use crate::world::catalog::Catalog;
    use crate::world::infinite;

    fn sewer_lamps(level: i32) -> (Facility, Vec<Lamp>) {
        let content = Content::load();
        let cat = Catalog::load();
        let mut cache = HashMap::new();
        let (f, _) = infinite::window("wired-sewer", (0, 0), level, &cat, &LootTables::default(), &mut cache);
        let lamps = lamps(&f, &[], &content);
        (f, lamps)
    }

    fn level_0(seed: &str) -> Facility {
        crate::world::route::layout::godot_regions_for_this_test();
        let cat = Catalog::load();
        let mut cache = HashMap::new();
        infinite::window(seed, (0, 0), 0, &cat, &LootTables::default(), &mut cache).0
    }

    /// The shipped lamp table with the dark-room share a quarter (it ships at 0 until streamed windows have a light switch).
    fn quarter_dark() -> lamp_flicker::Lamps {
        let text = include_str!("../data/render/lamps.json").replacen("\"dark_rooms\": { \"fraction\": 0 }", "\"dark_rooms\": { \"fraction\": 0.25 }", 1);
        lamp_flicker::Lamps::parse(&text).unwrap()
    }

    #[test]
    fn the_shipped_table_starts_no_room_dark() {
        let f = level_0("dark-a");
        assert!(dark_rooms(&f).iter().all(|d| !d));
    }

    #[test]
    fn about_a_quarter_of_level_0_rooms_start_dark_the_same_way_every_build() {
        let q = quarter_dark();
        let (mut rooms, mut dark_n) = (0, 0);
        for seed in ["dark-a", "dark-b", "dark-c"] {
            let f = level_0(seed);
            let dark = dark_rooms_with(&f, &q);
            assert_eq!(dark, dark_rooms_with(&level_0(seed), &q), "{seed}: same rooms dark on a rebuild");
            assert!(f.rooms.iter().zip(&dark).all(|(r, d)| !(r.lobby() && *d)), "the lobby is always lit");
            rooms += f.rooms.iter().filter(|r| !r.lobby()).count();
            dark_n += dark.iter().filter(|d| **d).count();
        }
        let share = dark_n as f32 / rooms as f32;
        assert!(rooms >= 30 && (0.12..=0.40).contains(&share), "{dark_n} of {rooms} rooms dark");
    }

    #[test]
    fn a_dark_level_0_room_has_no_lamps_and_a_lit_one_keeps_them() {
        let content = Content::load();
        let q = quarter_dark();
        let (mut hidden, mut lit_kept) = (0, 0);
        for seed in ["dark-a", "dark-b", "dark-c"] {
            let f = level_0(seed);
            let dark = dark_rooms_with(&f, &q);
            let all = lamps_with(&f, &[], &content, &dark);
            for (i, r) in f.rooms.iter().enumerate() {
                let inside = |l: &Lamp| {
                    l.pos.x > r.x as f32 + 1.0 && l.pos.x < (r.x + r.w) as f32 - 1.0 && l.pos.z > r.y as f32 + 1.0 && l.pos.z < (r.y + r.h) as f32 - 1.0
                };
                let n = all.iter().filter(|l| inside(l)).count();
                if dark[i] {
                    assert_eq!(n, 0, "{seed}: a dark room's lamps are zeroed");
                    hidden += f.props.iter().filter(|p| content.props.get(&p.ptype).is_some_and(|d| d.light.is_some()) && f.room_at(p.cell.x as f64 + 0.5, p.cell.y as f64 + 0.5) == Some(i)).count();
                } else {
                    lit_kept += n;
                }
            }
        }
        assert!(hidden > 0, "some dark room held lit props that were left out");
        assert!(lit_kept > 0, "lit rooms keep their lamps");
    }

    #[test]
    fn the_sewers_and_finite_floors_have_no_dark_rooms() {
        let q = quarter_dark();
        let f = level_0("dark-a");
        assert!(dark_rooms_with(&f, &q).iter().any(|d| *d));
        let mut finite = f.clone();
        finite.has_lift = true;
        assert!(dark_rooms_with(&finite, &q).iter().all(|d| !d));
        let mut below = f;
        below.level = -1;
        assert!(dark_rooms_with(&below, &q).iter().all(|d| !d));
    }

    fn near(a: Color, rgb: (f32, f32, f32)) -> bool {
        (a.r - rgb.0).abs() < 0.01 && (a.g - rgb.1).abs() < 0.01 && (a.b - rgb.2).abs() < 0.01
    }

    #[test]
    fn a_sewer_window_holds_dim_flickering_lamps_and_pulsing_emergency_lights() {
        for level in [-1, -2] {
            let (f, lamps) = sewer_lamps(level);
            let steady: Vec<&Lamp> = lamps.iter().filter(|l| near(l.color, (1.0, 0.635, 0.29))).collect();
            let red: Vec<&Lamp> = lamps.iter().filter(|l| near(l.color, (1.0, 0.165, 0.102))).collect();
            let amber: Vec<&Lamp> = lamps.iter().filter(|l| near(l.color, (1.0, 0.69, 0.125))).collect();
            assert!(steady.len() > 10, "level {level}: the sewer lamps are all kept, not cut at 10 ({})", steady.len());
            assert!(!red.is_empty() && !amber.is_empty(), "level {level}: emergency lights of both colours");
            assert!(lamps.len() <= lamp_flicker::table().prop_budget(f.level) + f.stairs.len() + f.room_lights.len() + 1);
            for l in &steady {
                assert!(l.energy > 0.5 && l.energy <= 1.5 && l.range >= 6.0, "a dim sewer lamp: {} at {}", l.energy, l.range);
                assert!(l.flicker.hz > 0.0 && l.flicker.depth > 0.0 && l.flicker.depth <= 0.25, "{:?}", l.flicker);
            }
            for l in red.iter().chain(&amber) {
                assert!(l.flicker.depth >= 0.5 && l.flicker.depth <= 1.0 && l.flicker.hz > 0.0, "{:?}", l.flicker);
            }
            for l in &lamps {
                for i in 0..400 {
                    let k = lamp_flicker::factor(i as f32 * 0.05, l.flicker);
                    assert!(k >= 1.0 - l.flicker.depth - 1e-5 && k <= 1.0 + l.flicker.depth + 1e-5);
                }
            }
            let phases: HashSet<u32> = steady.iter().map(|l| l.flicker.phase.to_bits()).collect();
            assert!(phases.len() > steady.len() / 2, "lamps start at different phases");
        }
    }

    #[test]
    fn the_lamp_list_is_the_same_every_build_and_merges_within_the_cap() {
        let (_, a) = sewer_lamps(-1);
        let (_, b) = sewer_lamps(-1);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| x.pos == y.pos && x.flicker == y.flicker && x.energy == y.energy));
        let eye = a[a.len() / 2].pos;
        for cap in [4, 8, 16, 100] {
            assert!(world_stack::merge_lamps(&a, &[], eye, cap).len() <= cap.min(shader::MAX_LAMPS));
        }
        let applied = lamp_flicker::applied(&a, 3.7, false);
        assert_eq!(applied.len(), a.len());
        assert!(shader::nearest_lamps(eye, &applied) == shader::nearest_lamps(eye, &applied));
    }
}

/// A manhole (or floor grate) is a hole in the level-0 floor onto the streamed sewer below it: the neighbour build must
/// hold the cells of every shaft in reach, or the hatch opens onto nothing (a black void).
#[cfg(test)]
mod manhole_tests {
    use super::*;
    use crate::content::loot::LootTables;
    use crate::render::world_stack;
    use crate::world::catalog::Catalog;
    use crate::world::infinite::window;

    const SEEDS: [&str; 4] = ["3976365387", "1426541270", "4157433391", "2899547009"];

    /// Stood at each ladder hatch of the level-0 window of `seed` at `centre` (the eye on its step cell), every hatch in
    /// reach of that eye must have its shaft drawn in the sewer below: the 3x3 of floor cells round its foot. Returns
    /// the (eye hatch, hatch in reach, cell) triples whose cell the neighbour build leaves out, and how many were checked.
    fn missing_cells(seed: &str, centre: (i32, i32)) -> (usize, Vec<(Cell, Cell, Cell)>) {
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let mut cache = HashMap::new();
        let (up, _) = window(seed, centre, 0, &cat, &loot, &mut cache);
        let (low, _) = window(seed, centre, SEWER_LEVEL, &cat, &loot, &mut cache);
        let frame = ChunkFrame { level: SEWER_LEVEL, center: centre };
        let floor: HashSet<Cell> = low.floor_cells().into_iter().collect();
        let hatches: Vec<Cell> = up.stairs.iter().filter(|s| s.dir < 0 && s.is_ladder()).map(|s| s.cell).collect();
        let reach = crate::game::transition::links().prefetch_distance;
        let mut missing = vec![];
        let mut checked = 0;
        for s in up.stairs.iter().filter(|s| s.dir < 0 && s.is_ladder()) {
            // the eye on the step cell, as the game sees it standing at the hatch
            let step = s.step_cell();
            let eye = (step.x as f32 + 0.5, 1.6, step.y as f32 + 0.5);
            let scope = world_stack::neighbour_scope(-crate::world::vertical::STOREY, openings(&low), &openings(&up), eye, reach, false, stream_table());
            let wanted = neighbour_wanted_from(window_parts(&low, frame), frame, &scope.openings, scope.radius, scope.max_parts, Some(scope.eye));
            let drawn: HashSet<(i32, i32)> = wanted.iter().flat_map(|(_, p)| p.cells.iter().map(|c| (c.x, c.y))).collect();
            for &h in &hatches {
                let (hx, hz) = h.center();
                let d = ((hx - eye.0).powi(2) + (hz - eye.2).powi(2)).sqrt();
                if d > reach {
                    continue;
                }
                checked += 1;
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        let c = Cell::new(h.x + dx, h.y + dz);
                        if floor.contains(&c) && !drawn.contains(&(c.x, c.y)) {
                            missing.push((s.cell, h, c));
                        }
                    }
                }
            }
        }
        (checked, missing)
    }

    #[test]
    fn every_hatch_in_reach_has_its_shaft_built_in_the_sewer_below() {
        let mut checked = 0;
        let mut failures = vec![];
        for seed in SEEDS {
            for cx in -1..=1 {
                for cy in -1..=1 {
                    let (n, missing) = missing_cells(seed, (cx, cy));
                    checked += n;
                    if let Some(first) = missing.first() {
                        failures.push(format!("{seed} ({cx},{cy}): eye at {:?} leaves hatch {:?} cell {:?} unbuilt", first.0, first.1, first.2));
                    }
                }
            }
        }
        assert!(checked > 40, "only {checked} hatch-in-reach checks in the sweep");
        assert!(failures.is_empty(), "{} windows leave a shaft unbuilt: {:#?}", failures.len(), failures);
    }
}

/// The sewer under a manhole, seen from its ladder foot (the web build showed a black screen on reaching it in some
/// rooms): over many seeds and window centres, the foot and its step cell are drawn cells of the sewer window, the
/// neighbour build near the eye holds the parts that carry them, a lamp lights the foot, and every lamp of the three
/// storeys round it is a finite, positive-range light (a NaN lamp in the shader's lamp loop blacks out every surface).
#[cfg(test)]
mod ladder_arrival_tests {
    use super::*;
    use crate::content::loot::LootTables;
    use crate::render::world_stack;
    use crate::world::catalog::Catalog;
    use crate::world::infinite::window;

    const NUMERIC: u32 = 6;
    const NAMED: [&str; 4] = ["ROOM-TEST-7", "SEWER", "KYTTU", "ABCDE"];
    const CENTRES: [(i32, i32); 3] = [(0, 0), (1, 0), (-1, 1)];

    fn world_seeds() -> Vec<(String, String)> {
        (1..=NUMERIC)
            .map(|n| n.to_string())
            .chain(NAMED.iter().map(|s| s.to_string()))
            .map(|text| (crate::world::route::world_seed(&text), text))
            .collect()
    }

    fn lamp_problem(l: &Lamp) -> Option<String> {
        let finite3 = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        if !finite3(l.pos) {
            return Some(format!("position {:?}", l.pos));
        }
        if !(l.range.is_finite() && l.range > 0.0) {
            return Some(format!("range {}", l.range));
        }
        if !(l.energy.is_finite() && l.energy >= 0.0) {
            return Some(format!("energy {}", l.energy));
        }
        let c = l.color;
        if ![c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite()) {
            return Some(format!("colour {c:?}"));
        }
        let fl = l.flicker;
        if ![fl.hz, fl.depth, fl.phase].iter().all(|v| v.is_finite()) {
            return Some(format!("flicker {fl:?}"));
        }
        for t in [0.0f32, 3.7, 1234.5, 86_400.0] {
            let k = lamp_flicker::factor(t, fl);
            if !(k.is_finite() && k >= 0.0) {
                return Some(format!("flicker factor {k} at {t}"));
            }
        }
        None
    }

    #[test]
    fn the_sewer_under_every_manhole_is_drawn_lit_and_made_of_finite_lamps() {
        let content = Content::load();
        let assets = Assets::headless();
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let reach = crate::game::transition::links().prefetch_distance;
        let (mut hatches, mut problems) = (0, vec![]);
        for (seed, text) in world_seeds() {
            for centre in CENTRES {
                let mut cache = HashMap::new();
                let (up, _) = window(&seed, centre, 0, &cat, &loot, &mut cache);
                let (low, _) = window(&seed, centre, SEWER_LEVEL, &cat, &loot, &mut cache);
                let (low2, _) = window(&seed, centre, SEWER_LOWER_LEVEL, &cat, &loot, &mut cache);
                for (name, f) in [("maze", &up), ("sewer", &low), ("lower sewer", &low2)] {
                    // every prop names a room of its window, and picking its model never indexes past the rooms
                    // (`variant_key` panicked on a sewer prop of a chunk with no room, in `build_meshes`)
                    for p in &f.props {
                        if !f.rooms.is_empty() && p.room >= f.rooms.len() {
                            problems.push(format!("{text} {centre:?} {name}: prop {} names room {} of {}", p.id, p.room, f.rooms.len()));
                        }
                        let _ = prop_model_key(f, p, &assets, &content);
                    }
                    for (i, l) in lamps(f, &[], &content).iter().enumerate() {
                        if let Some(p) = lamp_problem(l) {
                            problems.push(format!("{text} {centre:?} {name}: lamp {i} has a bad {p}"));
                        }
                    }
                }
                let sewer_lamps = lamps(&low, &[], &content);
                let frame = ChunkFrame { level: SEWER_LEVEL, center: centre };
                let drawn_cells: HashSet<Cell> = drawn_cells(&low).into_iter().collect();
                for s in up.stairs.iter().filter(|s| s.dir < 0 && s.is_ladder()) {
                    hatches += 1;
                    let (foot, step) = (s.cell, s.step_cell());
                    for (what, c) in [("foot", foot), ("step", step)] {
                        if !drawn_cells.contains(&c) {
                            problems.push(format!("{text} {centre:?}: the sewer's {what} cell {c:?} is not a drawn cell"));
                        }
                    }
                    // the neighbour build for an eye on the ladder (a link: on_flight) and on the step cell
                    for (at, on_link) in [(step, false), (foot, true)] {
                        let (cx, cz) = at.center();
                        let eye = (cx, 1.6, cz);
                        let scope = world_stack::neighbour_scope(-crate::world::vertical::STOREY, openings(&low), &openings(&up), eye, reach, on_link, stream_table());
                        let wanted = neighbour_wanted_from(window_parts(&low, frame), frame, &scope.openings, scope.radius, scope.max_parts, Some(scope.eye));
                        let held: HashSet<Cell> = wanted.iter().flat_map(|(_, p)| p.cells.iter().copied()).collect();
                        for (what, c) in [("foot", foot), ("step", step)] {
                            if !held.contains(&c) {
                                problems.push(format!("{text} {centre:?}: with the eye at {at:?} the neighbour build leaves out the {what} cell {c:?}"));
                            }
                        }
                    }
                    let (fx, fz) = foot.center();
                    let lit = sewer_lamps.iter().any(|l| (l.pos.x - fx).hypot(l.pos.z - fz) < l.range * 0.8);
                    if !lit {
                        problems.push(format!("{text} {centre:?}: no lamp of the sewer reaches the ladder foot {foot:?}"));
                    }
                }
            }
        }
        assert!(hatches > 100, "only {hatches} hatches in the sweep");
        assert!(problems.is_empty(), "{} problems in {hatches} hatches:\n{}", problems.len(), problems.join("\n"));
    }
}

/// A recentre of the Infinite window keeps the chunk parts still in it (keyed by absolute chunk) and builds only the new ones.
#[cfg(test)]
mod recentre_tests {
    use super::*;
    use crate::content::loot::LootTables;
    use crate::world::catalog::Catalog;
    use crate::world::infinite::window;

    fn settle(w: &mut WorldMeshes, f: &Facility, frame: ChunkFrame, assets: &Assets, content: &Content) -> (usize, usize) {
        let (mut built, mut kept) = (0, 0);
        for _ in 0..200 {
            w.rebuild(f, &[], Some(frame), assets, content);
            built += w.last_built;
            kept = w.last_kept;
            if !w.has_pending() {
                break;
            }
        }
        (built, kept)
    }

    /// The drawn vertices of each part, by key, in the window's coordinates.
    fn by_key(w: &WorldMeshes) -> HashMap<PartKey, Vec<Vec3>> {
        let mut out: HashMap<PartKey, Vec<Vec3>> = HashMap::new();
        for (m, k) in w.meshes.iter().zip(&w.drawn) {
            if let Some(k) = k {
                out.entry(*k).or_default().extend(m.vertices.iter().map(|v| v.position));
            }
        }
        out
    }

    #[test]
    fn a_recentre_builds_only_the_new_chunks_and_a_kept_chunk_equals_a_fresh_build() {
        let content = Content::load();
        let assets = Assets::headless();
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let mut cache = HashMap::new();
        let (a, _) = window("ROOM-TEST-7", (0, 0), 0, &cat, &loot, &mut cache);
        let (b, _) = window("ROOM-TEST-7", (1, 0), 0, &cat, &loot, &mut cache);
        let (fa, fb) = (ChunkFrame { level: 0, center: (0, 0) }, ChunkFrame { level: 0, center: (1, 0) });
        let mut w = WorldMeshes::default();
        let (first, _) = settle(&mut w, &a, fa, &assets, &content);
        assert!(first >= (WINDOW * WINDOW) as usize, "the first load builds the window");
        let (built, kept) = settle(&mut w, &b, fb, &assets, &content);
        assert!(kept > 0, "chunks still in the window are kept");
        assert!(built < first, "a recentre builds fewer parts than a full window: {built} vs {first}");
        let mut fresh = WorldMeshes::default();
        settle(&mut fresh, &b, fb, &assets, &content);
        // the meshes of a part come in the order their materials were met, so match each vertex to one of the other
        // build's within a millimetre instead of pairing them by index
        let (got, want) = (by_key(&w), by_key(&fresh));
        let mut compared = 0;
        for (k, v) in &got {
            let Some(r) = want.get(k) else { continue };
            assert_eq!(v.len(), r.len(), "part {k:?}");
            let missing: Vec<&Vec3> = v.iter().filter(|p| !r.iter().any(|q| (**p - *q).length() < 1e-3)).collect();
            assert!(missing.is_empty(), "part {k:?}: {} of {} vertices differ, first {:?}", missing.len(), v.len(), missing.first());
            compared += 1;
        }
        assert!(compared >= (WINDOW * (WINDOW - 1)) as usize / 2, "only {compared} parts compared");
    }
}
