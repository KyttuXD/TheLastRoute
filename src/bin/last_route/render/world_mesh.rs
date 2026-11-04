//! The floor's static geometry: floors and walls in each room region's own pictures (halls in
//! the maze set), baseboard and brass rail trims, ceilings, and
//! the room props: each type's Blockbench model (assets/props/<set>/*.bbmodel, one of the variants the prop data
//! names). Lights: the switchable room lights and the lamp props' own lights.

use super::builder::Builder;
use super::door;
use super::prop_scale;
use super::shader::Lamp;
use super::stairs;
use super::surfaces;
use super::textures::{self, TexRef, TextureChoice};
use crate::shared::assets::Assets;
use crate::config::WALL_HEIGHT;
use crate::content::props::{stable_index, visual_y};
use crate::content::regions::{RegionDef, RegionSafeLight};
use crate::content::Content;
use crate::world::facility::{Cell, DeckKind, DoorDraw, Prop, Stair};
use crate::world::inferno::Inferno;
use crate::world::infinite::{ChunkKey, CHUNK, INFERNO_LEVEL, WINDOW};
use crate::world::route::sewer::core::{Grid, LEVEL as SEWER_LEVEL, LOWER_LEVEL as SEWER_LOWER_LEVEL, WATER};
use crate::world::{region_texture, Facility, TEXTURE_SETS};
use macroquad::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Instant;

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
    builds: u64,
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

/// The openings of the stairs of `f`, the UPPER level's facility (one per stair with dir < 0). A neighbour below the
/// active level is scoped with the openings of the active facility; one above, with those of its own.
pub fn openings(f: &Facility) -> Vec<Opening> {
    f.stairs
        .iter()
        .filter(|s| s.dir < 0)
        .filter_map(|s| {
            let cells = s.well_cells();
            if cells.is_empty() {
                return None;
            }
            let n = cells.len() as f32;
            let sum = cells.iter().map(|c| c.center()).fold((0.0, 0.0), |a, c| (a.0 + c.0, a.1 + c.1));
            Some(Opening { centre: (sum.0 / n, sum.1 / n), cells, upper_level: f.level })
        })
        .collect()
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
            self.meshes = build_meshes(f, &pick, &mut choices, assets, content);
            lift(&mut self.meshes, self.dy);
            self.doors = doors;
            self.lift_doors();
            self.drawn = vec![None; self.meshes.len()];
            self.bounds = mesh_bounds(&self.meshes);
            return;
        };
        let table = stream_table();
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
            if !one_step && !table.allows(built, started.elapsed().as_secs_f64() * 1000.0, queued) {
                pending += 1;
                continue;
            }
            built += 1;
            let mut meshes = build_meshes(f, pick, &mut choices, assets, content);
            lift(&mut meshes, self.dy);
            self.parts.insert(*key, Part { meshes, origin: f.origin, dy: self.dy, used: build_no });
        }
        self.pending = pending;
        // with something drawn, the drawn set waits for the window to be whole; with nothing drawn yet (the first
        // load) the built parts show as they come
        if pending > 0 && !self.drawn.is_empty() {
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
        for (m, key) in std::mem::take(&mut self.meshes).into_iter().zip(std::mem::take(&mut self.drawn)) {
            if let Some(p) = key.and_then(|k| self.parts.get_mut(&k)) {
                p.meshes.push(m);
            }
        }
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
        let mut textures: Vec<Option<Texture2D>> = vec![];
        let mut order: Vec<usize> = out
            .iter()
            .map(|(_, m)| match textures.iter().position(|t| *t == m.texture) {
                Some(i) => i,
                None => {
                    textures.push(m.texture.clone());
                    textures.len() - 1
                }
            })
            .collect();
        let mut sorted: Vec<(usize, (Option<PartKey>, Mesh))> = order.drain(..).zip(out).collect();
        sorted.sort_by_key(|(t, _)| *t);
        let (drawn, meshes) = sorted.into_iter().map(|(_, km)| km).unzip();
        self.drawn = drawn;
        self.meshes = meshes;
        self.bounds = mesh_bounds(&self.meshes);
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
        let wanted = neighbour_wanted(window_parts(f, frame), frame, &scope.openings, scope.radius, scope.max_parts);
        // the hard cap: nothing outside the wanted set is kept
        self.parts.retain(|k, _| wanted.iter().any(|(w, _)| w == k));
        let table = stream_table();
        let started = Instant::now();
        let build_no = self.builds + 1;
        let queued = wanted.iter().filter(|(k, _)| !self.parts.contains_key(k)).count();
        let mut choices = HashMap::new();
        let (mut built, mut pending) = (0, 0);
        for (key, pick) in &wanted {
            if let Some(p) = self.parts.get_mut(key) {
                p.used = build_no;
                continue;
            }
            if !neighbour_allows(table, scope.chunks_per_frame, built, started.elapsed().as_secs_f64() * 1000.0, queued) {
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
            return;
        }
        self.builds = build_no;
        for (m, key) in std::mem::take(&mut self.meshes).into_iter().zip(std::mem::take(&mut self.drawn)) {
            if let Some(p) = key.and_then(|k| self.parts.get_mut(&k)) {
                p.meshes.push(m);
            }
        }
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
        self.bounds = mesh_bounds(&self.meshes);
    }
}

/// Whether one more neighbour part may be built after `built` this frame (the first always may).
fn neighbour_allows(table: &StreamBudget, per_frame: usize, built: usize, elapsed_ms: f64, queued: usize) -> bool {
    built == 0 || (built < per_frame && elapsed_ms < table.budget_ms(queued))
}

/// The meshes with one texture's side by side (so the batcher joins them), and the part each came from.
fn group_by_texture(out: Vec<(Option<PartKey>, Mesh)>) -> (Vec<Option<PartKey>>, Vec<Mesh>) {
    let mut textures: Vec<Option<Texture2D>> = vec![];
    let mut sorted: Vec<(usize, (Option<PartKey>, Mesh))> = out
        .into_iter()
        .map(|km| {
            let i = match textures.iter().position(|t| *t == km.1.texture) {
                Some(i) => i,
                None => {
                    textures.push(km.1.texture.clone());
                    textures.len() - 1
                }
            };
            (i, km)
        })
        .collect();
    sorted.sort_by_key(|(t, _)| *t);
    sorted.into_iter().map(|(_, km)| km).unzip()
}

/// The distance from point `p` to the box (x0, z0, x1, z1); 0 inside it.
fn rect_distance(p: (f32, f32), r: (f32, f32, f32, f32)) -> f32 {
    let dx = (r.0 - p.0).max(0.0).max(p.0 - r.2);
    let dz = (r.1 - p.1).max(0.0).max(p.1 - r.3);
    (dx * dx + dz * dz).sqrt()
}

/// The window parts a neighbour draws: those holding cells or rooms whose chunk lies within `radius` metres of an
/// opening centre, nearest first, at most `max_parts`.
fn neighbour_wanted(
    parts: Vec<(PartKey, Pick)>,
    frame: ChunkFrame,
    openings: &[Opening],
    radius: f32,
    max_parts: usize,
) -> Vec<(PartKey, Pick)> {
    let mut near: Vec<(f32, (PartKey, Pick))> = parts
        .into_iter()
        .filter(|(_, p)| !p.cells.is_empty() || !p.rooms.is_empty())
        .filter_map(|(k, p)| {
            let x0 = ((k.cx - (frame.center.0 - 1)) * CHUNK) as f32;
            let z0 = ((k.cy - (frame.center.1 - 1)) * CHUNK) as f32;
            let rect = (x0, z0, x0 + CHUNK as f32, z0 + CHUNK as f32);
            let d = openings.iter().map(|o| rect_distance(o.centre, rect)).fold(f32::MAX, f32::min);
            (d <= radius).then_some((d, (k, p)))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(max_parts);
    near.into_iter().map(|(_, kp)| kp).collect()
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
    /// A neighbour level holds at most this many chunk parts.
    pub neighbour_max_parts: usize,
    /// A neighbour level builds only chunk parts this many metres (or less) from a stairwell opening.
    pub neighbour_cull_radius: f32,
}

impl StreamBudget {
    /// Parses `{ "frame_budget_ms", "per_queued_ms", "max_budget_ms", "chunks_per_frame", "jump_steps", "neighbour_chunks_per_frame", "neighbour_max_parts", "neighbour_cull_radius" }`; the times
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
        built == 0 || (built < self.chunks_per_frame && elapsed_ms < self.budget_ms(queued))
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

/// The room lights plus each lamp prop's `light` from data/props (the original's fixture lights,
/// budget 10 per floor, offset turning with the prop).
pub fn lamps(f: &Facility, lights_on: &[bool], content: &Content) -> Vec<Lamp> {
    let mut out = vec![];
    for (i, l) in f.room_lights.iter().enumerate() {
        if lights_on.get(i).copied().unwrap_or(true) {
            let (x, z) = l.center.center();
            out.push(Lamp {
                pos: vec3(x, WALL_HEIGHT - 0.34, z),
                range: l.range * 1.4,
                color: Color::new(l.color.0, l.color.1, l.color.2, 1.0),
                energy: l.energy,
            });
        }
    }
    // the safe zone is a checkpoint: a warm light over its pad, so it reads as a lit area from the halls
    if let Some(s) = f.safe_zone {
        let (x, z) = s.center();
        let light = safe_light_for(f, s.x, s.y);
        out.push(Lamp {
            pos: vec3(x, WALL_HEIGHT - 0.6, z),
            range: 7.0,
            color: Color::new(light.color[0], light.color[1], light.color[2], 1.0),
            energy: light.energy,
        });
    }
    // each stairwell and ladder well carries a light, so its steps and rungs read in the dark
    out.extend(f.stairs.iter().map(|s| stairs::lamp(*s, WALL_HEIGHT)));
    let mut budget = 10;
    for p in &f.props {
        if budget == 0 {
            break;
        }
        let Some(light) = content.props.get(&p.ptype).and_then(|d| d.light.clone()) else {
            continue;
        };
        budget -= 1;
        let (cx, cz) = p.cell.center();
        let (x, z) = (cx + p.offset.0, cz + p.offset.1);
        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
        let off = Mat3::from_rotation_y(yaw) * vec3(light.offset.0, 0.0, light.offset.2);
        let y = visual_y(&p.ptype, WALL_HEIGHT, p.mount_y.unwrap_or(0.0) as f32) + light.offset.1;
        out.push(Lamp {
            pos: vec3(x + off.x, y, z + off.z),
            range: light.range,
            color: Color::new(light.color.0, light.color.1, light.color.2, 1.0),
            // Godot's omni energy reads brighter than this shader's linear falloff; doubled to match
            energy: light.energy * 2.0,
        });
    }
    out
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
        None => choice.floor_at(gx.div_euclid(45), gy.div_euclid(45)),
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
/// (or no region).
fn geometry_wall_height(def: Option<&RegionDef>) -> f32 {
    def.and_then(|d| d.geometry).map_or(WALL_HEIGHT, |g| g.wall_height)
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
            stairs::Face::Plain => &mut *plain,
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
fn variant_key(f: &Facility, room: usize) -> String {
    let r = &f.rooms[room];
    match r.id.rsplit_once('@') {
        Some((id, _)) => format!("{id}@{},{}", r.x + f.origin.0, r.y + f.origin.1),
        None => r.id.clone(),
    }
}

/// The model a prop is drawn with: the variant its room picks, else the first one that is loaded.
fn prop_model_key<'c>(f: &Facility, p: &Prop, assets: &Assets, content: &'c Content) -> Option<&'c String> {
    let models = content.props.get(&p.ptype).map(|d| d.models.as_slice()).filter(|m| !m.is_empty())?;
    let picked = &models[stable_index(&variant_key(f, p.room), models.len())];
    if assets.models.contains_key(picked) {
        Some(picked)
    } else {
        models.iter().find(|m| assets.models.contains_key(*m))
    }
}

/// Where a room door stands: raised to the hunter's opening and widened to its doorway, on its doorway's centre.
fn door_xf(p: &Prop, d: DoorDraw, fit: &door::DoorFit, h: f32) -> Mat4 {
    let (x, z) = p.cell.center();
    let y = visual_y(&p.ptype, h, p.mount_y.unwrap_or(0.0) as f32);
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
                xf: door_xf(p, d, &fit, WALL_HEIGHT),
                fit,
            })
        })
        .collect()
}

/// The level -3 floor heights of a window: each chunk's carve, made once, read per corner.
struct InfernoFloor {
    seed: String,
    origin: (i32, i32),
    chunks: HashMap<(i32, i32), Inferno>,
}

impl InfernoFloor {
    /// The window floor's heights, or None when `f` is not a level -3 window (its text is `<seed>:L<level>`).
    fn of(f: &Facility) -> Option<InfernoFloor> {
        let (seed, level) = f.seed_text.rsplit_once(":L")?;
        (level.parse::<i32>().ok()? == INFERNO_LEVEL).then(|| InfernoFloor {
            seed: seed.to_string(),
            origin: f.origin,
            chunks: HashMap::new(),
        })
    }

    fn cell(&mut self, x: i32, y: i32) -> f32 {
        let (gx, gy) = (x + self.origin.0, y + self.origin.1);
        let key = (gx.div_euclid(CHUNK), gy.div_euclid(CHUNK));
        let seed = &self.seed;
        self.chunks
            .entry(key)
            .or_insert_with(|| Inferno::carve(seed, ChunkKey { cx: key.0, cy: key.1, level: INFERNO_LEVEL }))
            .height_at(Cell::new(gx.rem_euclid(CHUNK), gy.rem_euclid(CHUNK)))
    }

    /// The height of the grid corner (`i`, `j`): the mean of the four cells that share it.
    fn corner(&mut self, i: i32, j: i32) -> f32 {
        (self.cell(i - 1, j - 1) + self.cell(i, j - 1) + self.cell(i - 1, j) + self.cell(i, j)) / 4.0
    }
}

impl InfernoFloor {
    /// The carve of the chunk `key`, made once.
    fn carve(&mut self, key: (i32, i32)) -> &Inferno {
        let seed = &self.seed;
        self.chunks
            .entry(key)
            .or_insert_with(|| Inferno::carve(seed, ChunkKey { cx: key.0, cy: key.1, level: INFERNO_LEVEL }))
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
    chunks: HashMap<(i32, i32), Option<Grid>>,
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
        let (seed, level) = (&self.seed, self.level);
        self.chunks
            .entry(key)
            .or_insert_with(|| {
                crate::world::route::sewer::chunk(
                    crate::world::route::layout::Coord::new(key.0, level, key.1),
                    &crate::world::route::sewer::Inputs { world: seed },
                )
                .map(|c| c.sewer_grid)
            })
            .as_ref()
    }

    /// Whether facility cell (`x`, `y`) is channel water.
    fn is_water(&mut self, x: i32, y: i32) -> bool {
        let (key, (lx, ly)) = chunk_cell(self.origin, x, y);
        self.grid(key).is_some_and(|g| g[ly][lx] == WATER)
    }
}

/// The boxes of one catwalk cell as (centre, size, colour) in facility metres: `deck_height` above the floor,
/// `deck_at(x, y)` telling which neighbours are deck (a railing stands on each side that is not).
fn catwalk_boxes(c: Cell, deck_height: f32, deck_at: &dyn Fn(i32, i32) -> bool) -> Vec<(Vec3, Vec3, Color)> {
    use super::catwalk;
    let open = catwalk::OpenSides {
        north: !deck_at(c.x, c.y - 1),
        south: !deck_at(c.x, c.y + 1),
        west: !deck_at(c.x - 1, c.y),
        east: !deck_at(c.x + 1, c.y),
    };
    let mid = vec3(c.x as f32 + 0.5, deck_height, c.y as f32 + 0.5);
    catwalk::build(catwalk::config(), 1.0, deck_height, open, (c.x, c.y))
        .into_iter()
        .map(|p| (mid + Vec3::from(p.center), Vec3::from(p.size), rgb(p.color)))
        .collect()
}

/// Whether a cell draws its ceiling quad: not over the well of a flight up (the lower level's well is open to the level
/// above, whose floor has the matching hole).
fn draws_ceiling(step: Option<Stair>) -> bool {
    !matches!(step, Some(s) if s.dir > 0)
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
    // a ladder's sewer end has no well: its ladder climbs to a hatch in the ceiling over an ordinary cell
    let hatches: Vec<Cell> = f.stairs.iter().filter(|s| s.is_ladder() && s.dir > 0).map(|s| s.cell).collect();
    let mut inferno = InfernoFloor::of(f);
    // the sewer window's channel water: the bed lies below the floor, the surface (unlit, tinted by depth) above it
    let mut sewer = SewerFloor::of(f);
    let water_params = crate::world::route::sewer::water::params();
    let bed = water_params.bed_offset();
    let mut water = Builder::new(Some(assets.white.clone()));
    // one painted storey stretched floor to ceiling, cornice at the top (RefSurfaceMesh.add_wall)
    let tiers = textures::wall_tiers(h) as f32;
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
                let drop = |inf: &mut Option<InfernoFloor>, i: i32, j: i32| inf.as_mut().map_or(0.0, |s| s.corner(i, j));
                painted(&mut surf, floor_kind, assets.floor(set)).quad(
                    [
                        vec3(x, drop(&mut inferno, i, j), z),
                        vec3(x + 1., drop(&mut inferno, i + 1, j), z),
                        vec3(x + 1., drop(&mut inferno, i + 1, j + 1), z + 1.),
                        vec3(x, drop(&mut inferno, i, j + 1), z + 1.),
                    ],
                    UV1,
                    up,
                    WHITE,
                    0.0,
                )
            }
        }
        if step.is_none() && hatches.contains(&c) {
            stairs::ladder_up(&mut bits, (c.x, c.y), &site);
        }
        add_stairs(bits, &mut surf, &mut plain, &mut glow, floor_kind, wall_kind, assets, set);
        if draws_ceiling(step) {
            let corners = [
                vec3(x, wall_h, z + 1.),
                vec3(x + 1., wall_h, z + 1.),
                vec3(x + 1., wall_h, z),
                vec3(x, wall_h, z),
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
        // a catwalk deck: grating, railings on its open sides and supports (render/catwalk.rs, data/render/catwalk.json)
        let surface = f.walk_surface_at(c);
        if surface.kind == DeckKind::Catwalk && step.is_none() {
            let deck_at = |x: i32, y: i32| f.walk_surface_at(Cell::new(x, y)).kind == DeckKind::Catwalk;
            for (center, size, col) in catwalk_boxes(c, surface.height, &deck_at) {
                plain.cube(center, size, 0.0, col, 0.0);
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
        let here_water = step.is_none() && sewer.as_mut().is_some_and(|s| s.is_water(c.x, c.y));
        for (dx, dz, a, b, n) in edges {
            // the channel's bank: from the floor down to the bed, wherever the next cell is not channel water
            if here_water
                && !steps.contains_key(&(c.x + dx, c.y + dz))
                && !sewer.as_mut().is_some_and(|s| s.is_water(c.x + dx, c.y + dz))
            {
                painted(&mut surf, floor_kind, assets.floor(set)).quad([a, b, b + up * bed, a + up * bed], UV1, n, WHITE, 0.0);
            }
            // a stairwell's shaft is open to its landing and the other lane, whatever its cells' wall flags say
            if !f.is_wall_cell(c.x + dx, c.y + dz) || steps.contains_key(&(c.x + dx, c.y + dz)) {
                continue;
            }
            let (a, b) = match inferno.as_mut() {
                Some(s) => (
                    vec3(a.x, s.corner(a.x as i32, a.z as i32), a.z),
                    vec3(b.x, s.corner(b.x as i32, b.z as i32), b.z),
                ),
                None => (a, b),
            };
            // the painted wall carries its own cornice, chair rail and skirting
            painted(&mut surf, wall_kind, assets.wall(set)).quad(
                [a + up * wall_h, b + up * wall_h, b, a],
                [vec2(0., 0.), vec2(1., 0.), vec2(1., wall_tiers), vec2(0., wall_tiers)],
                n,
                WHITE,
                0.0,
            );
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
        }
    }
    // level -3 cave decoration (render/inferno_deco.rs): the pieces of the picked cells
    if let Some(inf) = inferno.as_mut() {
        let (seed, cfg) = (inf.seed.clone(), decor_cfg());
        for (&key, owned) in &chunk_cells {
            let at = (key.0 * CHUNK - f.origin.0, key.1 * CHUNK - f.origin.1);
            let carved = inf.carve(key);
            for d in crate::render::inferno_deco::inferno_deco().place(&seed, ChunkKey { cx: key.0, cy: key.1, level: INFERNO_LEVEL }, carved) {
                let cell = (at.0 + d.cell.x, at.1 + d.cell.y);
                if owned.contains(&cell) {
                    add_deco(&mut plain, &mut glow, &d, at, cell_wall_height(f, cell.0, cell.1), cfg);
                }
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
        let mut y = visual_y(&p.ptype, h, p.mount_y.unwrap_or(0.0) as f32);
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
                    let bottom = door::header_bottom(dims, fit.sy, h);
                    if bottom < h {
                        let (choice, _) = cell_surfaces(f, p.cell.x, p.cell.y, pick.pad, choices);
                        let wall = choice.wall_at((p.cell.x + f.origin.0) as i64, (p.cell.y + f.origin.1) as i64);
                        let fallback = assets.wall(room_set(f, p.cell.x, p.cell.y));
                        let filler = door::filler_from(dims, fit.sy);
                        door_header(&mut surf, wall, fallback, vec3(x, 0.0, z), yaw, d.span as f32, (bottom, filler), h, tiers);
                    }
                    let xf = door_xf(p, d, &fit, h);
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
            vec3(x, h - 0.04, z),
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
    meshes
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
        let tall = parse_file("tmp_tall.json", tall).unwrap();
        let plain = parse_file("tmp_plain.json", plain).unwrap();
        assert_eq!(geometry_wall_height(Some(&tall)), 6.5);
        assert_eq!(geometry_wall_height(Some(&plain)), WALL_HEIGHT);
        assert_eq!(geometry_wall_height(None), WALL_HEIGHT);
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
    fn a_window_cell_maps_to_its_chunk_and_local_cell() {
        let origin = (-CHUNK, 2 * CHUNK);
        assert_eq!(chunk_cell(origin, 0, 0), ((-1, 2), (0, 0)));
        assert_eq!(chunk_cell(origin, CHUNK + 3, 5), ((0, 2), (3, 5)));
        assert_eq!(chunk_cell((-1, 0), 0, 0), ((-1, 0), ((CHUNK - 1) as usize, 0)));
    }

    #[test]
    fn a_catwalk_cell_gets_rails_only_on_its_open_sides() {
        let c = Cell::new(4, 4);
        let all = catwalk_boxes(c, 4.0, &|_, _| true);
        let alone = catwalk_boxes(c, 4.0, &|x, y| (x, y) == (4, 4));
        assert!(!all.is_empty());
        assert!(alone.len() > all.len(), "open sides add railing boxes");
        let side = catwalk_boxes(c, 4.0, &|x, y| !(x == 5 && y == 4));
        assert!(side.len() > all.len() && side.len() < alone.len(), "one open side adds fewer rails than four");
        // every box is placed around the cell centre, the grating at the deck height
        for (centre, _, _) in &alone {
            assert!((centre.x - 4.5).abs() < 1.0 && (centre.z - 4.5).abs() < 1.0);
        }
        assert!(all.iter().any(|(centre, _, _)| (centre.y - 4.0).abs() < 0.1), "grating sits at the deck height");
        assert!(all.iter().all(|(centre, _, _)| centre.y <= 4.0 + 1e-3), "all-deck cell has nothing above the deck");
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
        Lamp { pos: vec3(0., y, 0.), range: 5.0, color: WHITE, energy: 1.0 }
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
        assert!(!p.quads.iter().any(|q| q.n.y < -0.5), "the flight adds no cap under the level above");
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
    fn the_neighbour_budget_follows_the_data() {
        let t = StreamBudget::parse(
            r#"{"frame_budget_ms":4.0,"per_queued_ms":1.0,"max_budget_ms":9.0,"chunks_per_frame":4,"jump_steps":1,
            "neighbour_chunks_per_frame":2,"neighbour_max_parts":12,"neighbour_cull_radius":14.0}"#,
        )
        .unwrap();
        assert!(neighbour_allows(&t, 2, 0, 999.0, 5), "the first always builds");
        assert!(neighbour_allows(&t, 2, 1, 0.0, 5));
        assert!(!neighbour_allows(&t, 2, 2, 0.0, 5), "the per-frame cap");
        let old = r#"{"frame_budget_ms":4.0,"per_queued_ms":1.0,"max_budget_ms":9.0,"chunks_per_frame":4,"jump_steps":1}"#;
        assert!(StreamBudget::parse(old).is_err(), "the neighbour keys are required");
    }
}
