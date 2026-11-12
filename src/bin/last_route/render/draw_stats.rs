//! Per-frame render counters for the F7 performance panel (app/perf.rs): how many meshes and draw calls the 3D view
//! makes, how much vertex data goes to the GPU, and how many world meshes the fog and frustum culls dropped. The numbers
//! are there to compare a build before and after a change, so they are cheap and only counted while the full panel shows.
//!
//! The draw calls are macroquad's batched ones, found by running the meshes through a model of its batcher
//! (`mesh_batch::BatchModel`), since macroquad keeps its own count private. The WebGL call count is an estimate from
//! the draw calls (see `GL_CALLS_PER_DRAW`): hooking every GL function in the vendored miniquad would touch the wasm
//! glue for a diagnostic.

use super::mesh_batch::BatchModel;
use macroquad::prelude::*;
use std::cell::RefCell;
use crate::shared::atomic64::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};

/// macroquad's draw-call buffer sizes (main.rs sets them in the window conf): one draw call holds this many vertices
/// and indices, and a bigger batch starts another call.
pub const DRAW_CALL_VERTEX_CAPACITY: usize = 65536;
pub const DRAW_CALL_INDEX_CAPACITY: usize = 65536 * 3 / 2;

/// WebGL calls one macroquad draw call makes (framebuffer and viewport state, two buffer uploads, the pipeline, the
/// attribute bindings, the textures and the material's uniforms). Calibrated from a measured web frame: about 3,600
/// calls for 126 draw calls. An estimate, shown as one.
pub const GL_CALLS_PER_DRAW: u64 = 28;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static MESHES: AtomicU64 = AtomicU64::new(0);
static VERTICES: AtomicU64 = AtomicU64::new(0);
static INDICES: AtomicU64 = AtomicU64::new(0);
static WORLD_SEEN: AtomicU64 = AtomicU64::new(0);
static WORLD_FOGGED: AtomicU64 = AtomicU64::new(0);
static WORLD_OFF_VIEW: AtomicU64 = AtomicU64::new(0);
static ITEMS_OFF_VIEW: AtomicU64 = AtomicU64::new(0);

/// The frame sections draws are counted under in the F7 overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    World,
    Props,
    Actors,
    Effects,
    Other,
}

pub const SECTIONS: [(Section, &str); 5] = [
    (Section::World, "world"),
    (Section::Props, "props"),
    (Section::Actors, "actors"),
    (Section::Effects, "effects"),
    (Section::Other, "other"),
];

const SECTION_COUNT: usize = 5;
static SECTION_NOW: AtomicU64 = AtomicU64::new(4);
// a const item, not an inline `const { }` block: those need Rust 1.79 and the legacy macOS build uses 1.77
#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static SECTION_MESHES: [AtomicU64; SECTION_COUNT] = [ZERO; SECTION_COUNT];
static SECTION_CALLS: [AtomicU64; SECTION_COUNT] = [ZERO; SECTION_COUNT];

/// Names the section the following draws belong to (until the next call or the next frame start).
pub fn set_section(s: Section) {
    SECTION_NOW.store(s as u64, Ordering::Relaxed);
}

/// Per section: (meshes, draw calls) this frame.
pub fn section_counts() -> [(u64, u64); SECTION_COUNT] {
    let mut out = [(0, 0); SECTION_COUNT];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (SECTION_MESHES[i].load(Ordering::Relaxed), SECTION_CALLS[i].load(Ordering::Relaxed));
    }
    out
}

thread_local! {
    static BATCH: RefCell<BatchModel<Option<Texture2D>>> =
        RefCell::new(BatchModel::new(DRAW_CALL_VERTEX_CAPACITY, DRAW_CALL_INDEX_CAPACITY));
}

/// Turns the counting on or off (the panel is on screen). Off, every call here is a load and a return.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Starts a frame's count again.
pub fn begin_frame() {
    if !active() {
        return;
    }
    for c in [&MESHES, &VERTICES, &INDICES, &WORLD_SEEN, &WORLD_FOGGED, &WORLD_OFF_VIEW, &ITEMS_OFF_VIEW] {
        c.store(0, Ordering::Relaxed);
    }
    BATCH.with(|b| b.borrow_mut().clear());
    for i in 0..SECTION_COUNT {
        SECTION_MESHES[i].store(0, Ordering::Relaxed);
        SECTION_CALLS[i].store(0, Ordering::Relaxed);
    }
    SECTION_NOW.store(Section::Other as u64, Ordering::Relaxed);
}

/// Counts one mesh handed to `draw_mesh`.
pub fn note_mesh(m: &Mesh) {
    note(&m.texture, m.vertices.len(), m.indices.len());
}

/// Counts one mesh of `vertices` vertices and `indices` indices with `texture`, as it goes to the batcher.
pub fn note(texture: &Option<Texture2D>, vertices: usize, indices: usize) {
    if !active() {
        return;
    }
    MESHES.fetch_add(1, Ordering::Relaxed);
    VERTICES.fetch_add(vertices as u64, Ordering::Relaxed);
    INDICES.fetch_add(indices as u64, Ordering::Relaxed);
    let new_call = BATCH.with(|b| b.borrow_mut().push(texture, vertices, indices));
    let s = (SECTION_NOW.load(Ordering::Relaxed) as usize).min(SECTION_COUNT - 1);
    SECTION_MESHES[s].fetch_add(1, Ordering::Relaxed);
    if new_call {
        SECTION_CALLS[s].fetch_add(1, Ordering::Relaxed);
    }
}

/// Draws a mesh and counts it.
pub fn draw_counted(m: &Mesh) {
    note_mesh(m);
    draw_mesh(m);
}

/// A flush (a camera change, an explicit `flush`) ends macroquad's running draw call.
pub fn note_flush() {
    if active() {
        BATCH.with(|b| b.borrow_mut().reset());
    }
}

/// Counts the world meshes one pass looked at, and how many of them it dropped for the fog and for the frustum.
pub fn note_world_cull(seen: usize, fogged: usize, off_view: usize) {
    if active() {
        WORLD_SEEN.fetch_add(seen as u64, Ordering::Relaxed);
        WORLD_FOGGED.fetch_add(fogged as u64, Ordering::Relaxed);
        WORLD_OFF_VIEW.fetch_add(off_view as u64, Ordering::Relaxed);
    }
}

/// Counts a scene item (a pickup, a fixture, a locker) the frustum dropped.
pub fn note_item_off_view() {
    if active() {
        ITEMS_OFF_VIEW.fetch_add(1, Ordering::Relaxed);
    }
}

/// The frame's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub meshes: u64,
    pub draw_calls: u64,
    pub vertices: u64,
    pub indices: u64,
    pub world_seen: u64,
    pub world_fogged: u64,
    pub world_off_view: u64,
    pub items_off_view: u64,
}

/// Bytes one vertex takes in macroquad's vertex buffer.
pub fn vertex_bytes() -> u64 {
    std::mem::size_of::<Vertex>() as u64
}

impl Counts {
    /// Megabytes of vertex and index data sent to the GPU this frame (macroquad uploads each batch whole).
    pub fn upload_mb(&self) -> f64 {
        (self.vertices * vertex_bytes() + self.indices * 2) as f64 / 1_000_000.0
    }

    pub fn gl_calls_estimate(&self) -> u64 {
        self.draw_calls * GL_CALLS_PER_DRAW
    }
}

pub fn counts() -> Counts {
    Counts {
        meshes: MESHES.load(Ordering::Relaxed),
        draw_calls: BATCH.with(|b| b.borrow().calls as u64),
        vertices: VERTICES.load(Ordering::Relaxed),
        indices: INDICES.load(Ordering::Relaxed),
        world_seen: WORLD_SEEN.load(Ordering::Relaxed),
        world_fogged: WORLD_FOGGED.load(Ordering::Relaxed),
        world_off_view: WORLD_OFF_VIEW.load(Ordering::Relaxed),
        items_off_view: ITEMS_OFF_VIEW.load(Ordering::Relaxed),
    }
}

/// The panel rows for this frame's counts; none while the counting is off.
pub fn rows() -> Vec<(String, String)> {
    if !active() {
        return vec![];
    }
    let c = counts();
    let mut rows = vec![
        ("3d draw calls".to_string(), format!("{}", c.draw_calls)),
        ("3d meshes".to_string(), format!("{}", c.meshes)),
        ("3d triangles".to_string(), format!("{}", c.indices / 3)),
        ("vertex upload".to_string(), format!("{:.2} MB", c.upload_mb())),
        ("webgl calls (est)".to_string(), format!("{}", c.gl_calls_estimate())),
        (
            "world culled".to_string(),
            format!("{} seen, {} fog, {} view", c.world_seen, c.world_fogged, c.world_off_view),
        ),
        ("items off view".to_string(), format!("{}", c.items_off_view)),
    ];
    let by = section_counts();
    for (i, (_, name)) in SECTIONS.iter().enumerate() {
        if by[i].0 > 0 {
            rows.push((format!("draws: {name}"), format!("{} calls, {} meshes", by[i].1, by[i].0)));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_convert_to_megabytes_and_a_gl_estimate() {
        let c = Counts { vertices: 250_000, indices: 300_000, draw_calls: 126, ..Counts::default() };
        let want = (250_000 * vertex_bytes() + 600_000) as f64 / 1_000_000.0;
        assert!((c.upload_mb() - want).abs() < 1e-9);
        assert_eq!(c.gl_calls_estimate(), 126 * GL_CALLS_PER_DRAW);
        assert!(vertex_bytes() >= 36 && vertex_bytes() <= 48, "macroquad's vertex is about 40 bytes");
    }

    /// The counters and the on/off flag are process-wide, so the tests that switch them take turns.
    static COUNTER_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn nothing_is_counted_or_shown_while_off() {
        let _serial = COUNTER_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        set_active(false);
        note_world_cull(5, 1, 2);
        note_item_off_view();
        assert_eq!(counts().world_seen, 0);
        assert!(rows().is_empty());
    }

    #[test]
    fn section_counts_add_up_to_the_frame_total() {
        let _serial = COUNTER_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        set_active(true);
        begin_frame();
        // meshes without a texture: the first starts a call, the rest join it
        let plain = Mesh { vertices: vec![], indices: vec![], texture: None };
        set_section(Section::World);
        note_mesh(&plain);
        note_mesh(&plain);
        set_section(Section::Props);
        note_mesh(&plain);
        let by = section_counts();
        let c = counts();
        assert_eq!(by.iter().map(|s| s.0).sum::<u64>(), c.meshes);
        assert_eq!(by.iter().map(|s| s.1).sum::<u64>(), c.draw_calls);
        set_active(false);
    }
}
