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
use crate::world::facility::{stair_well_cells, Cell, DoorDraw, Prop, Stair, STAIR_FLIGHT};
use crate::world::inferno::Inferno;
use crate::world::infinite::{ChunkKey, CHUNK, INFERNO_LEVEL, WINDOW};
use crate::world::{region_texture, Facility, TEXTURE_SETS};
use macroquad::prelude::*;
use std::collections::HashMap;
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

/// Whether a move of the window from `from` to `to` is a jump (a level change, or a centre more than `jump_steps`
/// chunks away) rather than a step. streamed_facility_view_3d.gd 726-727.
fn is_jump(from: ChunkFrame, to: ChunkFrame, jump_steps: i32) -> bool {
    from.level != to.level
        || (from.center.0 - to.center.0).abs() > jump_steps
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
        self.lamps = lamps(f, lights_on, content);
        self.doors = door_instances(f, assets, content);
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
            let meshes = build_meshes(f, pick, &mut choices, assets, content);
            self.parts.insert(*key, Part { meshes, origin: f.origin, used: build_no });
        }
        self.pending = pending;
        // with something drawn, the drawn set waits for the window to be whole; with nothing drawn yet (the first
        // load) the built parts show as they come
        if pending > 0 && !self.drawn.is_empty() {
            return;
        }
        self.builds = build_no;
        // Phase 2: the drawn set goes back into the parts it came from, then every wanted part is drawn in order.
        for (m, key) in std::mem::take(&mut self.meshes).into_iter().zip(std::mem::take(&mut self.drawn)) {
            if let Some(p) = key.and_then(|k| self.parts.get_mut(&k)) {
                p.meshes.push(m);
            }
        }
        let mut out: Vec<(Option<PartKey>, Mesh)> = vec![];
        for (key, _) in &wanted {
            let part = self.parts.get_mut(key).expect("phase 1 built or kept every wanted part");
            if part.origin != f.origin {
                reframe(&mut part.meshes, part.origin, f.origin);
                part.origin = f.origin;
            }
            out.extend(part.meshes.drain(..).map(|m| (Some(*key), m)));
        }
        out.extend(safe_pad(f, assets).into_iter().map(|m| (None, m)));
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
}

impl StreamBudget {
    /// Parses `{ "frame_budget_ms", "per_queued_ms", "max_budget_ms", "chunks_per_frame", "jump_steps" }`; the times
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
        Ok(StreamBudget {
            frame_budget_ms: num("frame_budget_ms")?,
            per_queued_ms: num("per_queued_ms")?,
            max_budget_ms: num("max_budget_ms")?,
            chunks_per_frame: chunks as usize,
            jump_steps,
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

/// Stairwell flights: each hole cell's step number (1 = next to the landing) and its stair.
fn stair_steps(f: &Facility) -> HashMap<(i32, i32), (i32, Stair)> {
    let mut out = HashMap::new();
    for s in &f.stairs {
        for k in 1..=STAIR_FLIGHT {
            for c in stair_well_cells(*s, k) {
                out.insert((c.x, c.y), (k, *s));
            }
        }
    }
    out
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
            Some((_, s)) if s.is_ladder() => stairs::ladder_down(&mut bits, (c.x, c.y), s, &site),
            Some((k, s)) => stairs::flight_cell(&mut bits, (c.x, c.y), k, s, &site),
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
        if !matches!(step, Some((_, s)) if s.dir > 0) {
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
        for (dx, dz, a, b, n) in edges {
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
  "jump_steps": 1
}"#;

    #[test]
    fn the_embedded_stream_table_holds_the_godot_standard_values() {
        let t = stream_table();
        assert_eq!(t.frame_budget_ms, 4.0, "streamed_facility_view_3d.gd STREAM_BUDGET_MS");
        assert_eq!(t.per_queued_ms, 1.0, "streamed_facility_view_3d.gd STREAM_BUDGET_PER_QUEUED_MS");
        assert_eq!(t.max_budget_ms, 9.0, "streamed_facility_view_3d.gd STREAM_BUDGET_MAX_MS");
        assert_eq!(t.chunks_per_frame, 4, "streamed_facility_view_3d.gd STARTUP_CHUNKS_PER_FRAME");
        assert_eq!(t.jump_steps, 1, "streamed_facility_view_3d.gd 726-727: more than one chunk away is a jump");
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

    #[test]
    fn a_jump_is_more_than_one_chunk_away_or_a_level_change_as_in_godot() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        let at = |level: i32, x: i32, z: i32| ChunkFrame { level, center: (x, z) };
        assert!(!is_jump(at(0, 5, 5), at(0, 6, 6), t.jump_steps), "a diagonal step to the next chunk is a step");
        assert!(is_jump(at(0, 5, 5), at(0, 7, 5), t.jump_steps), "two chunks in one frame is a teleport");
        assert!(is_jump(at(0, 5, 5), at(0, 5, 2), t.jump_steps), "a teleport along z is a jump too");
        assert!(is_jump(at(0, 5, 5), at(1, 5, 5), t.jump_steps), "a level change is a jump");
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
    fn a_teleport_or_a_level_change_builds_its_window_in_one_step_and_a_step_streams() {
        let t = StreamBudget::parse(STANDARD).unwrap();
        let at = |level: i32, x: i32, z: i32| ChunkFrame { level, center: (x, z) };
        assert!(!builds_in_one_step(None, at(0, 5, 5), 0, t.jump_steps), "the first load of a run streams");
        assert!(builds_in_one_step(None, at(0, 5, 5), 3, t.jump_steps), "back onto the window after leaving the floor");
        assert!(!builds_in_one_step(Some(at(0, 5, 5)), at(0, 6, 5), 3, t.jump_steps), "a step streams");
        assert!(builds_in_one_step(Some(at(0, 5, 5)), at(0, 9, 5), 3, t.jump_steps), "a teleport builds in one step");
        assert!(builds_in_one_step(Some(at(0, 5, 5)), at(2, 5, 5), 3, t.jump_steps), "a level change builds in one step");
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
