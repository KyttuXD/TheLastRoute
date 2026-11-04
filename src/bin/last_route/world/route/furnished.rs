//! Stage 3b-iii part A of the Infinite route: the furnished rooms of a level-0 chunk, a port of
//! `StreamedRouteGenerator._furnished_room_props` (streamed_route_generator.gd:3058-3227) and of the
//! helpers it reaches. `props::region_props` calls `furnished_room_props` for every room that is not a
//! stairwell, suite parent or two-floor room (streamed_route_generator.gd:2884-2900); a room that gets
//! props is skipped by the DRESSING draw.
//!
//! Godot names and line numbers are in each doc comment. The RNG streams are the ones of the driver:
//! `"<world>:TEMPLATE:<chunk>:<room id>"` (3156: one mirror draw, always), `"<world>:VARIETY:<chunk>:<room id>"`
//! (3165, `_vary_specs`), and the draws inside `_room_template_specs` on the TEMPLATE stream. Every draw
//! is made in Godot order, and `and`/`or` short-circuit as in GDScript.
//!
//! Float arithmetic follows GDScript: a `Vector2`/`Vector3` is `f32`, a plain float expression is `f64`.
//!
//! Not ported here: `RoomRecipes` beyond the one shipped recipe (music_store, transcribed from
//! data/rooms/music_store.json); any other data recipe is absent, so its room gets no props. The post passes
//! (GeometryResolve.push_out, PlacementClearance.clear) are not applied, so a template prop that Godot removes
//! there is still made here. `interaction_for` checks the seat and bed words only for the template names used in
//! the goldens (its MusicProps and CarnivalBooths branches are not ported; for those names they never give sit
//! or lay).

use std::f64::consts::PI;
use std::sync::OnceLock;

use super::layout::{Coord, Grid, Rect, Room, CHUNK, V2};
use super::prop_tables::{
    lookup, BED_TEMPLATES, CEILING_FAMILY_ORDER, CEILING_FIXTURE_FAMILY, FALLBACK_BEDS, PROP_COLLIDER_EXTRA_BOXES,
    PROP_COLLIDER_OFFSETS, PROP_COLLIDER_SIZES, PROP_COLLIDER_VARIANT_BOXES, TEMPLATE_PROP_SIZES, WALL_MOUNTED_TYPES,
};
use super::props::{self, Mark, Prop, Reserved};
use crate::world::catalog::Catalog;
use crate::world::godot_sort;
use crate::world::rng::Rng;

// ----------------------------------------------------------------------------------------------------------
// tables and the catalog

/// The room catalog (RoomCatalog): surface parents, wall-backed types and surface heights (catalog.rs).
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(Catalog::load)
}

/// `RoomCatalog.is_surface_prop` (room_catalog.gd:641).
pub(super) fn is_surface_prop(t: &str) -> bool {
    catalog().is_surface_prop(t)
}

/// `RoomCatalog.is_wall_backed` (room_catalog.gd:637).
fn is_wall_backed(t: &str) -> bool {
    catalog().is_wall_backed(t)
}

/// `PropModelCatalog.is_wall_mounted` (prop_model_catalog.gd:568).
fn is_wall_mounted(t: &str) -> bool {
    WALL_MOUNTED_TYPES.contains(&t)
}

/// `PropModelCatalog.is_ceiling_mounted` (prop_model_catalog.gd:576): `CEILING_MODEL_TOP` keys.
fn is_ceiling_mounted(t: &str) -> bool {
    matches!(
        t,
        "diningChandelier" | "ceilingLight" | "ceilingFan" | "infernoChainLantern" | "surgicalLight" | "hangingVines"
    )
}

/// `PropModelCatalog.has_model` (prop_model_catalog.gd:508): a non-empty `MODEL_PATHS` entry.
fn has_model(t: &str) -> bool {
    MODEL_KEYS.contains(&t)
}

/// `PropModelCatalog.hunter_scale` (prop_model_catalog.gd:884): `HUNTER_SCALE.get(type, 1.0)`. The draw table
/// (data/render/prop_draw_scale.json) carries the same values.
fn hunter_scale(t: &str) -> f64 {
    crate::render::prop_scale::draw_scale(t) as f64
}

/// `PropModelCatalog.interaction_for` (prop_model_catalog.gd:923-948), in its branch order. Only "sit" and "lay"
/// are compared by the placement code; the other kinds are returned as their Godot strings where they are
/// known and "" otherwise.
pub(super) fn interaction_for(t: &str) -> &'static str {
    let lowered = t.to_lowercase();
    if t == "carouselCenterpiece" {
        return "ride";
    }
    if t.starts_with("retroComputer") {
        return "use";
    }
    if lowered.contains("mirror") && t != "standingMirror" {
        return "mirror";
    }
    if t == "dinerBooth" {
        return "sit";
    }
    if t == "labBench" || t == "pottingBench" {
        return "";
    }
    if t == "controlConsole" || t == "nursesStation" || t == "diveConsole" {
        return "use";
    }
    if lowered.ends_with("bed") || lowered.contains("bunk") {
        return "lay";
    }
    for word in ["chair", "couch", "bench", "stool", "sofa"] {
        if lowered.contains(word) {
            return "sit";
        }
    }
    ""
}

/// `RoomCatalog.surface_parent_types` (room_catalog.gd:645).
fn surface_parent_types(t: &str) -> &'static [String] {
    // the catalog owns the strings; a borrowed slice of a static catalog is enough here
    catalog().surface_parent_types(t)
}

/// `RoomCatalog.surface_parent_covers` (room_catalog.gd:653): `parent` is the placed parent, `child` the surface prop.
pub(super) fn surface_parent_covers(parent: &Prop, child: &Prop) -> bool {
    if parent.room_id != child.room_id {
        return false;
    }
    if !surface_parent_types(child.prop_type).iter().any(|p| p == parent.prop_type) {
        return false;
    }
    if parent.cell == child.cell {
        return true;
    }
    parent.occupied_cells.contains(&child.cell)
}

/// `_room_facing_yaw` (5290); also the snapping of `snappedf(atan2, PI * .25)` (`snap_quarter`).
fn snap_quarter(x: f64) -> f64 {
    let step = PI * 0.25;
    (x / step + 0.5).floor() * step
}

/// `wrapf(value, min, max)` (Godot math_funcs.h): `value - range * floor((value - min) / range)`.
fn wrapf(value: f64, min: f64, max: f64) -> f64 {
    let range = max - min;
    if range.abs() < 1e-12 {
        return min;
    }
    value - range * ((value - min) / range).floor()
}

/// The `ORIENT_PAIRS` of streamed_route_generator.gd:3331-3342 with Godot's float reaches (f64).
struct Pair {
    child: &'static [&'static str],
    anchor: &'static [&'static str],
    reach: f64,
}

const ORIENT_RULES: &[Pair] = &[
    Pair {
        child: &["Chair", "Stool"],
        anchor: &["Table", "Desk", "Counter", "Island", "Workstation", "Bar", "Station", "Console"],
        reach: 1.4,
    },
    Pair { child: &["Couch", "Sofa", "Chaise"], anchor: &["TvConsole", "CoffeeTable"], reach: 4.5 },
    Pair { child: &["Bench"], anchor: &["Piano", "Vanity"], reach: 1.5 },
    Pair {
        child: &["Computer", "Monitor", "Console", "Terminal"],
        anchor: &["Chair", "Stool", "Couch", "Bench"],
        reach: 2.6,
    },
    Pair { child: &["Monitor"], anchor: &["Bed"], reach: 2.0 },
];

/// `SCREEN_SEAT_REACH` (streamed_route_generator.gd:3319).
const SCREEN_SEAT_REACH: f64 = 2.6;

/// `ORIENT_EXCLUDE` (3343).
const ORIENT_EXCLUDE: &[&str] = &["TvConsole"];

/// `SEAT_SHAREABLE` (3501): the `reserved` value of a placed recipe prop's cell (only seats may share it).
const SEAT_SHAREABLE: Mark = Mark::SeatShareable;

/// `Mark::Plain` is the `reserved[cell] = true` of doors, thresholds and beds.
const PLAIN: Mark = Mark::Plain;

/// Godot `_CODE_MODEL_PATHS` keys, generated from prop_model_catalog.gd (see `MODEL_KEYS`'s doc).

// ----------------------------------------------------------------------------------------------------------
// vectors (Godot Vector2 is f32; `V2` is the i32 cell vector)

type F2 = [f32; 2];

fn f2(p: V2) -> F2 {
    [p.x as f32, p.y as f32]
}

fn f2add(a: F2, b: F2) -> F2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn f2sub(a: F2, b: F2) -> F2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn f2scale(a: F2, k: f32) -> F2 {
    [a[0] * k, a[1] * k]
}

fn f2dot(a: F2, b: F2) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn f2len(a: F2) -> f32 {
    f2dot(a, a).sqrt()
}

/// `Vector2.normalized()`: the zero vector stays zero.
fn f2norm(a: F2) -> F2 {
    let l = f2len(a);
    if l == 0.0 {
        [0.0, 0.0]
    } else {
        [a[0] / l, a[1] / l]
    }
}

fn f2distance(a: F2, b: F2) -> f32 {
    f2len(f2sub(b, a))
}
fn vsub(a: V2, b: V2) -> V2 {
    V2::new(a.x - b.x, a.y - b.y)
}

fn vneg(v: V2) -> V2 {
    V2::new(-v.x, -v.y)
}

/// `Rect2i.has_point` (end-exclusive).
fn rect_has(rect: Rect, p: V2) -> bool {
    p.x >= rect.x && p.y >= rect.y && p.x < rect.x + rect.w && p.y < rect.y + rect.h
}

/// `_grid_cell_open` (5285): inside the chunk and floor.
fn grid_open(grid: &Grid, cell: V2) -> bool {
    in_grid(cell) && grid[cell.y as usize][cell.x as usize] == 0
}

/// Inside the chunk (the grid is CHUNK cells square).
fn in_grid(cell: V2) -> bool {
    cell.x >= 0 && cell.y >= 0 && cell.x < CHUNK && cell.y < CHUNK
}

/// The grid value of an in-chunk cell (`grid[y][x]`); the outside reads as wall (1).
fn grid_value_at(grid: &Grid, cell: V2) -> u8 {
    if in_grid(cell) { grid[cell.y as usize][cell.x as usize] } else { 1 }
}

/// `Vector2(cell) + Vector2(.5, .5) + placement_offset`: the prop's point (`prop_point`, 3322).
fn prop_point(prop: &Prop) -> F2 {
    let po = prop.placement_offset.unwrap_or([0.0, 0.0]);
    [prop.cell.x as f32 + 0.5 + po[0], prop.cell.y as f32 + 0.5 + po[1]]
}

/// `collider_size` of a prop: `_collider_size` (5007) on its type, or the blocker size.
fn prop_collider_size(prop: &Prop) -> [f32; 3] {
    prop.collider_size.unwrap_or(prop.blocker_size)
}
// ----------------------------------------------------------------------------------------------------------
// collider boxes (PropColliderBoxes, prop_collider_boxes.gd) and the audit geometry (GeometryAudit)

/// `GeometryAudit.TOL_DEFAULT` (geometry_audit.gd:16); `tol()` is `TOL_DEFAULT` (`strict` is false).
const TOL_DEFAULT: f64 = 0.10;
/// `GeometryAudit.SEAT_TOL` (geometry_audit.gd:19).
const SEAT_TOL: f64 = 0.55;
/// `GeometryAudit.SEAT_WORDS` (geometry_audit.gd:32).
const SEAT_WORDS: &[&str] = &["Chair", "Stool", "Bench", "Seat", "Armchair", "Couch", "Sofa"];
/// `GeometryAudit.TABLE_WORDS` (geometry_audit.gd:33).
const TABLE_WORDS: &[&str] = &["Table", "Desk", "Console", "Counter", "Workstation", "Bar", "Bookcase", "Shelf", "Piano"];

fn word_in(t: &str, words: &[&str]) -> bool {
    words.iter().any(|w| t.contains(w))
}

/// `GeometryAudit.pair_allowance` (geometry_audit.gd:89) with `strict` false.
fn pair_allowance(a: &str, b: &str) -> f64 {
    let seat_a = word_in(a, SEAT_WORDS);
    let seat_b = word_in(b, SEAT_WORDS);
    if (seat_a && word_in(b, TABLE_WORDS)) || (seat_b && word_in(a, TABLE_WORDS)) {
        return SEAT_TOL;
    }
    if seat_a && seat_b {
        return 0.2;
    }
    0.0
}

/// `GeometryAudit._axis_x` / `_axis_y` (geometry_audit.gd:73-79).
fn axis_x(yaw: f64) -> [f64; 2] {
    [yaw.cos(), -yaw.sin()]
}

fn axis_y(yaw: f64) -> [f64; 2] {
    [yaw.sin(), yaw.cos()]
}

/// `GeometryAudit.obb_depth` (geometry_audit.gd:39): the smallest axis penetration of two oriented rectangles,
/// 0 when they do not overlap. The arithmetic is f64 (GDScript floats) on the f32 Vector2 components.
fn obb_depth(ca: F2, ha: F2, ya: f64, cb: F2, hb: F2, yb: f64) -> f64 {
    let axes = [axis_x(ya), axis_y(ya), axis_x(yb), axis_y(yb)];
    let d = [(cb[0] - ca[0]) as f64, (cb[1] - ca[1]) as f64];
    let (ha, hb) = ([ha[0] as f64, ha[1] as f64], [hb[0] as f64, hb[1] as f64]);
    let mut best = f64::INFINITY;
    for n in axes {
        let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
        let ra = ha[0] * dot(n, axis_x(ya)).abs() + ha[1] * dot(n, axis_y(ya)).abs();
        let rb = hb[0] * dot(n, axis_x(yb)).abs() + hb[1] * dot(n, axis_y(yb)).abs();
        let depth = ra + rb - dot(d, n).abs();
        if depth <= 0.0 {
            return 0.0;
        }
        best = best.min(depth);
    }
    best
}

/// `PropColliderBoxes.boxes` (prop_collider_boxes.gd) for a type whose box is `size`; the variant is the
/// index picked by `variant_of` (-1 when unknown). Returns (centre, half extents) pairs, f32 components.
pub(super) fn boxes(t: &str, size: [f32; 3], variant: i32) -> Vec<(F2, F2)> {
    let derived = lookup(PROP_COLLIDER_SIZES, t).unwrap_or([0.0, 0.0, 0.0]);
    if size != derived {
        return vec![([0.0, 0.0], [size[0] * 0.5, size[2] * 0.5])];
    }
    if let Some(extra) = lookup(PROP_COLLIDER_EXTRA_BOXES, t) {
        if !extra.is_empty() {
            return extra.iter().map(|v| ([v[0], v[1]], [v[2] * 0.5, v[3] * 0.5])).collect();
        }
    }
    if let Some(per_variant) = lookup(PROP_COLLIDER_VARIANT_BOXES, t) {
        if variant >= 0 && (variant as usize) < per_variant.len() {
            let v = per_variant[variant as usize];
            return vec![([v[0], v[1]], [v[2] * 0.5, v[3] * 0.5])];
        }
    }
    let offset = lookup(PROP_COLLIDER_OFFSETS, t).unwrap_or([0.0, 0.0]);
    vec![(offset, [size[0] * 0.5, size[2] * 0.5])]
}

/// `PropColliderBoxes.variant_of` (prop_collider_boxes.gd): the model variant the collider reads. Only the
/// types with `VARIANT_BOXES` (dining and office chairs) depend on it: `_variant_index` (prop_model_catalog.gd:796)
/// over the blocky model list, keyed by the room id (or the instance id for a dressing prop).
pub(super) fn variant_of(prop: &Prop) -> i32 {
    let t = prop.prop_type;
    let Some(len) = blocky_variant_count(t) else {
        return -1;
    };
    let key = if prop.room_id.is_empty() { prop.id.as_str() } else { prop.room_id.as_str() };
    stable_index(key, len) as i32
}

/// The number of blocky model variants of a type with `VARIANT_BOXES` (config/blocky_prop_models.gd).
fn blocky_variant_count(t: &str) -> Option<usize> {
    match t {
        "diningChair" | "diningChairCushion" => Some(13),
        "officeChair" => Some(5),
        _ => None,
    }
}

/// `PropModelCatalog._stable_index` (prop_model_catalog.gd:950).
fn stable_index(value: &str, size: usize) -> usize {
    let mut result: i64 = 0;
    for byte in value.as_bytes() {
        result = (result * 31 + *byte as i64) & 0x7fff_ffff;
    }
    (result as usize) % size.max(1)
}

/// `_collider_rects` (streamed_route_generator.gd:3569): the world (x, z) boxes of a prop, with its yaw.
/// Each box is (centre, half extents, yaw).
fn collider_rects(prop: &Prop) -> Vec<(F2, F2, f64)> {
    let mount = prop.mount_offset.unwrap_or([0.0, 0.0]);
    let po = prop.placement_offset.unwrap_or([0.0, 0.0]);
    let at = f2add(f2add(f2(prop.cell), [0.5, 0.5]), f2add(mount, po));
    let yaw = prop.yaw;
    let size = prop_collider_size(prop);
    let variant = variant_of(prop);
    boxes(prop.prop_type, size, variant)
        .into_iter()
        .map(|(local, half)| {
            // world_centre (prop_collider_boxes.gd): at + (local.x * c + local.y * s, -local.x * s + local.y * c),
            // the sums in f64 and the result a Vector2 (f32)
            let (c, s) = (yaw.cos(), yaw.sin());
            let wx = local[0] as f64 * c + local[1] as f64 * s;
            let wz = -(local[0] as f64) * s + local[1] as f64 * c;
            let centre = [at[0] + wx as f32, at[1] + wz as f32];
            (centre, half, yaw)
        })
        .collect()
}

/// `_collider_clear` (streamed_route_generator.gd:3554): a solid prop's boxes may not penetrate a placed solid's
/// beyond `tol() + pair_allowance`.
fn collider_clear(entry: &Prop, placed: &[Prop]) -> bool {
    if !entry.solid || entry.wall_mounted {
        return true;
    }
    let mine = collider_rects(entry);
    for other in placed {
        if !other.solid || other.wall_mounted {
            continue;
        }
        let allowance = TOL_DEFAULT + pair_allowance(entry.prop_type, other.prop_type);
        for a in &mine {
            for b in collider_rects(other) {
                if obb_depth(a.0, a.1, a.2, b.0, b.1, b.2) > allowance {
                    return false;
                }
            }
        }
    }
    true
}

// ----------------------------------------------------------------------------------------------------------
// recipe specs (the `{type, side, depth, facing, ...}` dictionaries of the template arms)

/// One template spec. `side` runs across the room (0 is the door's column), `depth` from the back wall.
/// `solid` is `None` when the Godot dictionary has no `solid` key (the default depends on `wall_mount`).
#[derive(Clone, Debug)]
pub struct Spec {
    pub ty: &'static str,
    pub side: i32,
    pub depth: i32,
    pub facing: &'static str,
    pub solid: Option<bool>,
    pub wall_backed: bool,
    pub wall_mount: bool,
    pub mount_y: f64,
    pub claim_first: bool,
}

/// `{"type": ty, "side": side, "depth": depth, "facing": facing}` (the `_sp` shape, 4324).
fn sp(ty: &'static str, side: i32, depth: i32, facing: &'static str) -> Spec {
    Spec { ty, side, depth, facing, solid: None, wall_backed: false, wall_mount: false, mount_y: 0.0, claim_first: false }
}

impl Spec {
    /// `"wall_backed": true`
    fn wall(mut self) -> Spec {
        self.wall_backed = true;
        self
    }
    /// `"wall_mount": true`
    fn mount(mut self) -> Spec {
        self.wall_mount = true;
        self
    }
    /// `"solid": false`
    fn loose(mut self) -> Spec {
        self.solid = Some(false);
        self
    }
    /// `"mount_y": v`
    fn my(mut self, v: f64) -> Spec {
        self.mount_y = v;
        self
    }
    /// `"claim_first": true`
    fn claim(mut self) -> Spec {
        self.claim_first = true;
        self
    }
    /// `spec.get("solid", not wall_mount)` (the `_claim_order` default).
    fn solid_or_mount(&self) -> bool {
        self.solid.unwrap_or(!self.wall_mount)
    }
}

// ----------------------------------------------------------------------------------------------------------
// the room basis (`_room_template_basis`, streamed_route_generator.gd:4849-4875)

/// The frame of a room's recipe: the open direction (toward the doorway), its side, the back cell, and the
/// spans across and along the room.
#[derive(Clone, Copy, Debug)]
struct Basis {
    open: V2,
    side: V2,
    back_cell: V2,
    side_span: i32,
    depth_span: i32,
}

/// The room centre (`room.get("center", rect.position + rect.size / 2)`, 4850).
fn room_center(room: &Room) -> V2 {
    let r = room.rect;
    V2::new(r.x + r.w / 2, r.y + r.h / 2)
}

/// `_room_template_basis` (4849-4875).
fn room_template_basis(room: &Room) -> Option<Basis> {
    let rect = room.rect;
    let center = room_center(room);
    let doorway = room.doorway?;
    let delta = vsub(doorway, center);
    let open = if delta.x.abs() >= delta.y.abs() {
        V2::new(if delta.x >= 0 { 1 } else { -1 }, 0)
    } else {
        V2::new(0, if delta.y >= 0 { 1 } else { -1 })
    };
    let side = V2::new(-open.y, open.x);
    let mut back_cell = center;
    if open.x > 0 {
        back_cell.x = rect.x + 1;
    } else if open.x < 0 {
        back_cell.x = rect.x + rect.w - 2;
    } else if open.y > 0 {
        back_cell.y = rect.y + 1;
    } else {
        back_cell.y = rect.y + rect.h - 2;
    }
    let along_x = open.x != 0;
    Some(Basis {
        open,
        side,
        back_cell,
        side_span: if along_x { rect.h } else { rect.w },
        depth_span: if along_x { rect.w } else { rect.h },
    })
}

// ----------------------------------------------------------------------------------------------------------
// the template choice (`_furnished_room_props`, 3058-3154)

/// Whether `room_type` is a room role of one of the four data-backed districts (`HOSPITAL_ROOMS`, `LAB_ROOMS`,
/// `GREENHOUSE_ROOMS` and `MALL_ROOMS` of streamed_route_generator.gd:91-99): their `room_roles` in data/regions.
fn is_district_room_role(room_type: &str) -> bool {
    ["hospital", "lab", "greenhouse", "mall"]
        .iter()
        .any(|id| super::regions::region_def(id).is_some_and(|d| d.room_roles.iter().any(|r| r.as_str() == room_type)))
}

/// The template of a room type in a region (3070-3154). `None` when no template applies (or the type has only a
/// data recipe, which is not ported: see the module doc).
fn template_for(room_type: &str, region: &str) -> Option<String> {
    let t: String = match room_type {
        "bedroom" | "guest_room" => "bedroom".into(),
        "kitchen" => "kitchen".into(),
        "dining_room" => "mansion_dining".into(),
        "family_dining" => "family_dining".into(),
        "living_room" => "apartment_living".into(),
        "restaurant_dining" => "restaurant_dining".into(),
        "house_bathroom" => "house_bathroom".into(),
        "hotel_bathroom" => "hotel_bathroom".into(),
        "luxury_bathroom" => "luxury_bathroom".into(),
        "public_restroom" => "public_restroom".into(),
        "arcade_room" => "arcade".into(),
        _ => {
            if region == "carnival" && ["midway", "game_stall", "backstage"].contains(&room_type) {
                format!("carnival_{room_type}")
            } else if region == "beach" && ["cove", "tide_pool"].contains(&room_type) {
                "frozen_tideline".into()
            } else if region == "beach" && room_type == "beach_hut" {
                "beach_hut".into()
            } else if region == "gothic" && room_type == "mourning_room" {
                "gothic_bedroom".into()
            } else if region == "gothic" && ["archive", "chapel", "crypt"].contains(&room_type) {
                format!("gothic_{room_type}")
            } else if room_type == "prison_cell" {
                "prison_cell".into()
            } else if region == "maze" && room_type == "storage" {
                "maze_storage".into()
            } else if region == "carnival" && room_type == "big_top" {
                "carnival_big_top".into()
            } else if region == "beach" && room_type == "boardwalk_room" {
                "boardwalk".into()
            } else if room_type == "drawing_room" || room_type == "lounge" {
                "lounge".into()
            } else if room_type == "office_reception" {
                "office_reception".into()
            } else if room_type == "cubicle_room" {
                "office_cubicles".into()
            } else if room_type == "office_job" {
                "office_job".into()
            } else if room_type == "meeting_room" {
                "office_meeting".into()
            } else if room_type == "private_office" {
                "private_office".into()
            } else if room_type == "prison_cafeteria" {
                "prison_cafeteria".into()
            } else if room_type == "infirmary" {
                "infirmary".into()
            } else if ["thermal_chamber", "forge", "altar_room", "lava_gallery"].contains(&room_type) {
                room_type.into()
            } else if MUSIC_TEMPLATES.contains(&room_type) {
                room_type.into()
            } else if ["hospital", "lab", "greenhouse", "mall"].contains(&region)
                && (is_district_room_role(room_type)
                    || room_type == "dome_atrium"
                    || room_type == "pod_room")
            {
                room_type.into()
            } else if has_recipe(room_type) {
                room_type.into()
            } else {
                return None;
            }
        }
    };
    Some(t)
}

/// Templates whose recipe needs `side_span >= 7 and depth_span >= 7` (3149-3153).
const NEEDS_SEVEN: &[&str] = &[
    "mansion_dining",
    "family_dining",
    "restaurant_dining",
    "kitchen",
    "arcade",
    "frozen_tideline",
    "luxury_bathroom",
    "public_restroom",
    "apartment_living",
];

/// `_frame_type_for_region` (4825).
fn frame_type_for_region(region: &str, wide: bool) -> &'static str {
    match region {
        "hotel" | "mansion" => {
            if wide {
                "luxuryPaintingFrameEmpty"
            } else {
                "luxuryPictureFrameEmpty"
            }
        }
        "gothic" => {
            if wide {
                "gothicPaintingFrameEmpty"
            } else {
                "gothicPictureFrameEmpty"
            }
        }
        _ => {
            if wide {
                "normalPaintingFrameEmpty"
            } else {
                "normalPictureFrameEmpty"
            }
        }
    }
}

/// `_rug_type_for_region` (4835). An empty string is Godot's "" (no rug).
fn rug_type_for_region(region: &str) -> &'static str {
    match region {
        "mansion" => "mansionAreaRug",
        "hotel" => "hotelCarpet",
        "apartment" => "apartmentCarpet",
        "gothic" => "gothicBedroomRug",
        _ => "bedroomRug",
    }
}

/// `_furnished_room_props` (streamed_route_generator.gd:3058-3227): the furnished props of one room, in recipe
/// order. Empty when the room gets no template or its recipe does not fit. `reserved` is the chunk's reserved
/// cells: the props' occupied cells are added to it (`SEAT_SHAREABLE`), and a bed guarantee may remove some.
pub fn furnished_room_props(world: &str, c: Coord, region: &str, room: &Room, grid: &Grid, reserved: &mut Reserved) -> Vec<Prop> {
    let rect = room.rect;
    if rect.w < 5 || rect.h < 5 {
        return Vec::new();
    }
    let Some(basis) = room_template_basis(room) else {
        return Vec::new();
    };
    let side_span = basis.side_span;
    let depth_span = basis.depth_span;
    let Some(mut template) = template_for(&room.room_type, region) else {
        return Vec::new();
    };
    // 3138-3147: great halls (21x21) get the table-grid versions of their gathering-room recipes
    if side_span >= 15 && depth_span >= 15 {
        match template.as_str() {
            "restaurant_dining" => template = "restaurant_hall".into(),
            "prison_cafeteria" => template = "prison_mess".into(),
            "mansion_dining" => template = "dining_hall".into(),
            _ => {}
        }
    }
    // 3149-3153
    if NEEDS_SEVEN.contains(&template.as_str()) && (side_span < 7 || depth_span < 7) {
        return Vec::new();
    }
    let tag = props::chunk_tag(c);
    let room_id = room.id.as_str();
    // 3154-3156: the mirror draw, always
    let mut rng = Rng::new(&format!("{world}:TEMPLATE:{tag}:{room_id}"));
    let mirror: i32 = if rng.next_float() < 0.5 { -1 } else { 1 };
    let mut specs = room_template_specs(&template, region, side_span, depth_span, mirror, &mut rng);
    // 3159-3164: a room with no code recipe takes its data recipe, on its own stream
    if specs.is_empty() && has_recipe(&room.room_type) {
        let mut data = Rng::new(&format!("{world}:ROOMDATA:{room_id}"));
        specs = recipe_specs(&room.room_type, side_span, depth_span, &mut data);
    }
    // 3165: the VARIETY stream
    let mut variety = Rng::new(&format!("{world}:VARIETY:{tag}:{room_id}"));
    specs = vary_specs(&specs, side_span, depth_span, &mut variety);

    let mut result: Vec<Prop> = Vec::new();
    for spec_index in claim_order(&specs) {
        let spec = &specs[spec_index];
        let Some(mut entry) = template_prop_entry(c, room, &basis, &template, spec_index, spec, grid, reserved) else {
            continue;
        };
        // 3173-3176: a surface prop whose parent is placed is seated by it
        if is_surface_prop(entry.prop_type) && entry.mount_y > 0.0 && !result.iter().any(|other| surface_parent_covers(other, &entry)) {
            continue;
        }
        // 3177: _collider_clear, else _nudge_seat_clear (3532)
        if !collider_clear(&entry, &result) && !nudge_seat_clear(&mut entry, &result, grid, rect) {
            continue;
        }
        entry.spec_index = spec_index;
        for occupied in entry.occupied_cells.clone() {
            reserved.entry(occupied).or_insert(SEAT_SHAREABLE);
        }
        result.push(entry);
    }
    let mut result = even_ceiling_fixtures(one_ceiling_family(result), rect);
    orient_props(&mut result, grid);

    // 3203-3217: a bedroom never goes without a bed (KT: "a gothic bedroom is nearly empty, no bed")
    if BED_TEMPLATES.contains(&template.as_str()) && !result.iter().any(|e| interaction_for(e.prop_type) == "lay") {
        let mut bed = fallback_bed(c, room, &basis, &template, &specs, grid, reserved, &result);
        if bed.is_none() {
            // hunter-scale furniture can fill a small bedroom before the bed gets a wall: the bed wins, and the floor
            // furniture in its way goes (wall and ceiling props stay)
            let movable: Vec<usize> = (0..result.len())
                .filter(|&i| !is_wall_mounted(result[i].prop_type) && !is_ceiling_mounted(result[i].prop_type))
                .collect();
            let mut freed = reserved.clone();
            for &i in &movable {
                for cell in &result[i].occupied_cells {
                    freed.remove(cell);
                }
            }
            let keep: Vec<Prop> = result
                .iter()
                .enumerate()
                .filter(|(i, _)| !movable.contains(i))
                .map(|(_, p)| p.clone())
                .collect();
            bed = fallback_bed(c, room, &basis, &template, &specs, grid, &freed, &keep);
            if let Some(found) = &bed {
                let bed_cells: Vec<V2> = found.occupied_cells.clone();
                let mut drop_spec: Vec<usize> = Vec::new();
                for &i in &movable {
                    let e = &result[i];
                    let hit = !collider_clear(found, std::slice::from_ref(e))
                        || e.occupied_cells.iter().any(|c2| bed_cells.contains(c2));
                    if hit {
                        drop_spec.push(e.spec_index);
                        for cell in &e.occupied_cells {
                            reserved.remove(cell);
                        }
                    }
                }
                result.retain(|e| !drop_spec.contains(&e.spec_index));
                // a lamp whose nightstand made way for the bed would float: it goes too
                let snapshot = result.clone();
                result = snapshot
                    .iter()
                    .filter(|e| {
                        !is_surface_prop(e.prop_type)
                            || e.mount_y <= 0.0
                            || snapshot.iter().any(|other| surface_parent_covers(other, e))
                    })
                    .cloned()
                    .collect();
            }
        }
        if let Some(mut found) = bed {
            found.spec_index = specs.len();
            for cell in found.occupied_cells.clone() {
                reserved.insert(cell, PLAIN);
            }
            result.push(found);
        }
    }
    // 3224-3226: back to recipe order
    result.sort_by_key(|e| e.spec_index);
    result
}

/// `_fallback_bed` (streamed_route_generator.gd:3255-3284): a bed along the back wall, then the side walls' spans,
/// trying the recipe's beds first and then the fallback beds.
fn fallback_bed(
    c: Coord,
    room: &Room,
    basis: &Basis,
    template: &str,
    specs: &[Spec],
    grid: &Grid,
    reserved: &Reserved,
    placed: &[Prop],
) -> Option<Prop> {
    let mut types: Vec<&'static str> = Vec::new();
    for spec in specs {
        if interaction_for(spec.ty) == "lay" && !types.contains(&spec.ty) {
            types.push(spec.ty);
        }
    }
    for t in FALLBACK_BEDS.iter().copied() {
        if !types.contains(&t) {
            types.push(t);
        }
    }
    let span = basis.side_span / 2;
    for t in types {
        for step in 0..=span {
            let sides: Vec<i32> = if step == 0 { vec![0] } else { vec![-step, step] };
            for side in sides {
                let spec = Spec { wall_backed: true, ..sp(t, side, 0, "open") };
                if let Some(entry) = template_prop_entry(c, room, basis, template, specs.len(), &spec, grid, reserved) {
                    if collider_clear(&entry, placed) {
                        return Some(entry);
                    }
                }
            }
        }
    }
    None
}

// ----------------------------------------------------------------------------------------------------------
// the recipe entry (`_template_prop_entry`, streamed_route_generator.gd:4878-4956)
/// `_template_prop_entry` (4878-4956): the prop of one spec, or `None` when it does not fit the room, the grid or
/// the reserved cells. The entry is a prop with `room_template` set.
fn template_prop_entry(
    c: Coord,
    room: &Room,
    basis: &Basis,
    template: &str,
    spec_index: usize,
    spec: &Spec,
    grid: &Grid,
    reserved: &Reserved,
) -> Option<Prop> {
    let prop_type = spec.ty;
    // 4879-4880: a model and a size are needed (the content-pack size is not ported)
    if !has_model(prop_type) || !(lookup(TEMPLATE_PROP_SIZES, prop_type).is_some() || lookup(PROP_COLLIDER_SIZES, prop_type).is_some()) {
        return None;
    }
    let open = basis.open;
    let side_dir = basis.side;
    let cell = V2::new(
        basis.back_cell.x + side_dir.x * spec.side + open.x * spec.depth,
        basis.back_cell.y + side_dir.y * spec.side + open.y * spec.depth,
    );
    let rect = room.rect;
    if !rect_has(rect, cell) || !grid_open(grid, cell) {
        return None;
    }
    let facing_direction = match spec.facing {
        "back" => vneg(open),
        "left" => vneg(side_dir),
        "right" => side_dir,
        "center" => cardinal_toward(cell, room_center(room), open),
        _ => open,
    };
    let blocker_size: [f32; 3] = lookup(TEMPLATE_PROP_SIZES, prop_type)
        .or_else(|| lookup(PROP_COLLIDER_SIZES, prop_type))
        .unwrap_or([1.0, 1.0, 1.0]);
    let mut placement_offset: F2 = [0.0, 0.0];
    let wall_mounted = spec.wall_mount;
    if spec.wall_backed && !wall_mounted {
        // flush: the back, 2 cm off the wall behind the cell (a shallow prop moves toward the wall)
        let extent = back_extent(prop_type, props::collider_size(prop_type, blocker_size));
        let k = (extent - 0.48) as f32;
        placement_offset = f2scale(f2(open), k);
    }
    let backing_wall = if spec.wall_backed || wall_mounted { vneg(open) } else { V2::new(0, 0) };
    if wall_mounted {
        let wall_cell = V2::new(cell.x + backing_wall.x, cell.y + backing_wall.y);
        let front_cell = V2::new(cell.x + open.x, cell.y + open.y);
        if !rect_has(rect, wall_cell) || !rect_has(rect, front_cell) || grid_open(grid, wall_cell) || !grid_open(grid, front_cell) {
            return None;
        }
    }
    let solid = spec.solid.unwrap_or(!wall_mounted);
    let occupied: Vec<V2> = if !solid {
        Vec::new()
    } else {
        let occ_size = if spec.wall_backed {
            let c_size = props::collider_size(prop_type, blocker_size);
            [blocker_size[0], blocker_size[1], blocker_size[2].min(c_size[2])]
        } else {
            blocker_size
        };
        // flush toward the wall stays in its cell: the offset counts only when it points into the room
        let po = if f2dot(placement_offset, f2(open)) >= 0.0 { placement_offset } else { [0.0, 0.0] };
        props::prop_occupied_cells(cell, facing_direction, occ_size, po)
    };
    for at in &occupied {
        // a seat may share a cell with its table (never a door or threshold cell)
        let shareable = reserved.get(at) == Some(&SEAT_SHAREABLE) && interaction_for(prop_type) == "sit";
        if !rect_has(rect, *at) || (reserved.contains_key(at) && !shareable) || !grid_open(grid, *at) {
            return None;
        }
    }
    let room_token = room.id.replace(':', "-");
    let mount_offset: F2 = if wall_mounted {
        [backing_wall.x as f32 * 0.44, backing_wall.y as f32 * 0.44]
    } else {
        [0.0, 0.0]
    };
    Some(Prop {
        id: format!("{}:TEMPLATE:{}:{}:{}", props::chunk_tag(c), room_token, template, spec_index),
        prop_type,
        cell,
        yaw: props::room_facing_yaw(cell, V2::new(cell.x + facing_direction.x, cell.y + facing_direction.y)),
        facing_target: V2::new(cell.x + facing_direction.x, cell.y + facing_direction.y),
        blocker_size,
        solid,
        occupied_cells: occupied,
        room_id: room.id.clone(),
        room_door: false,
        doorway_cells: Vec::new(),
        threshold_wall_cells: Vec::new(),
        placement_offset: Some(placement_offset),
        backing_wall: Some(backing_wall),
        open_direction: Some(facing_direction),
        collider_size: Some(props::collider_size(prop_type, blocker_size)),
        wall_mounted,
        mount_offset: Some(mount_offset),
        corridor_clear: true,
        room_template: Some(template.to_string()),
        mount_y: spec.mount_y,
        mount_scaled: false,
        oriented: false,
        spec_index,
    })
}

/// `_cardinal_toward` (streamed_route_generator.gd:4959): the cardinal step from `from` toward `to`.
fn cardinal_toward(from: V2, to: V2, fallback: V2) -> V2 {
    let delta = vsub(to, from);
    if delta.x == 0 && delta.y == 0 {
        return fallback;
    }
    if delta.x.abs() >= delta.y.abs() {
        V2::new(if delta.x > 0 { 1 } else { -1 }, 0)
    } else {
        V2::new(0, if delta.y > 0 { 1 } else { -1 })
    }
}

/// `back_extent` (streamed_route_generator.gd:3461): how far the prop's collision reaches behind its point.
fn back_extent(t: &str, size: [f32; 3]) -> f64 {
    let mut extent = 0.0f64;
    for (centre, half) in boxes(t, size, -1) {
        extent = extent.max((centre[1] + half[1]) as f64);
    }
    if extent > 0.0 {
        extent
    } else {
        size[2] as f64 * 0.5
    }
}

// ----------------------------------------------------------------------------------------------------------
// spec variety, claim order and the seat clear-up (`_vary_specs` 3607, `_claim_order` 3509, `_nudge_seat_clear` 3532)

/// `_vary_specs` (streamed_route_generator.gd:3607-3627): decoration moves a cell or two, and the room may gain
/// a ceiling light or two on the VARIETY stream.
fn vary_specs(specs: &[Spec], side_span: i32, depth_span: i32, rng: &mut Rng) -> Vec<Spec> {
    let mut out: Vec<Spec> = Vec::new();
    for spec in specs {
        let mut spec = spec.clone();
        let mounted = spec.wall_mount;
        let decor_on_wall = mounted && ["Frame", "Painting", "Sign", "Clock", "Poster"].iter().any(|w| spec.ty.contains(w));
        if (decor_on_wall || (!mounted && spec.solid == Some(false))) && rng.next_float() < 0.75 {
            let step = 1 + rng.next_index(2) as i32;
            let sign = if rng.next_float() < 0.5 { step } else { -step };
            spec.side += sign;
            if !mounted && spec.depth > 0 && rng.next_float() < 0.5 {
                let delta = if rng.next_float() < 0.5 { 1 } else { -1 };
                spec.depth = 1.max(spec.depth + delta);
            }
        }
        out.push(spec);
    }
    // a lamp or two of its own where the room has the floor for it (non-solid: clearance never changes)
    let half = (side_span - 3) / 2;
    if !specs.is_empty() && half >= 3 && depth_span >= 7 {
        let extra_count = rng.next_index(3);
        for _ in 0..extra_count {
            let side = rng.next_index((half * 2 + 1) as usize) as i32 - half;
            let depth = 1 + rng.next_index(1.max(depth_span - 4) as usize) as i32;
            out.push(Spec { solid: Some(false), ..sp("ceilingLight", side, depth, "open") });
        }
    }
    out
}

/// `_claim_order` (streamed_route_generator.gd:3509-3526): the spec indices, big solid footprints first (area
/// descending, then recipe order); `claim_first` specs take the infinite area.
fn claim_order(specs: &[Spec]) -> Vec<usize> {
    let mut order: Vec<(f64, usize)> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let t = spec.ty;
        let mut area = 0.0f64;
        if spec.solid_or_mount() && !is_surface_prop(t) {
            let size = lookup(PROP_COLLIDER_SIZES, t).unwrap_or_else(|| {
                let base = lookup(TEMPLATE_PROP_SIZES, t).unwrap_or([0.0, 0.0, 0.0]);
                let k = hunter_scale(t) as f32;
                [base[0] * k, base[1] * k, base[2] * k]
            });
            area = (size[0] * size[2]) as f64;
        }
        if spec.claim_first {
            area = f64::INFINITY;
        }
        order.push((area, i));
    }
    order.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
    order.into_iter().map(|(_, i)| i).collect()
}

/// `_nudge_seat_clear` (streamed_route_generator.gd:3532-3551): a seat slides straight back, 15 cm at a time up to
/// 1.2 m, until its collider clears what it is set against.
fn nudge_seat_clear(entry: &mut Prop, placed: &[Prop], grid: &Grid, rect: Rect) -> bool {
    if interaction_for(entry.prop_type) != "sit" {
        return false;
    }
    let yaw = entry.yaw;
    let back = [(yaw.sin()) as f32, (yaw.cos()) as f32];
    let start = entry.placement_offset.unwrap_or([0.0, 0.0]);
    let cell = entry.cell;
    for step in 1..=8 {
        let offset = f2add(start, f2scale(f2scale(back, 0.15), step as f32));
        let at = f2add(f2add(f2(cell), [0.5, 0.5]), offset);
        let at_cell = V2::new(at[0].floor() as i32, at[1].floor() as i32);
        if !rect_has(rect, at_cell) || !grid_open(grid, at_cell) {
            return false;
        }
        entry.placement_offset = Some(offset);
        if collider_clear(entry, placed) {
            return true;
        }
    }
    entry.placement_offset = Some(start);
    false
}

// ----------------------------------------------------------------------------------------------------------
// ceilings, orientation and the surface seats (`even_ceiling_fixtures` 3285, `one_ceiling_family` 3479,
// `orient_props` 3357, `seat_in_front_of_screens` 3386, `nearest_anchor` 3423, `back_to_wall` 3440,
// `wall_behind` 3469)

/// `one_ceiling_family` (streamed_route_generator.gd:3479-3494): a room keeps one ceiling fixture family.
fn one_ceiling_family(props: Vec<Prop>) -> Vec<Prop> {
    let family_of = |t: &str| -> &'static str {
        CEILING_FIXTURE_FAMILY.iter().find(|(k, _)| *k == t).map(|(_, f)| *f).unwrap_or("")
    };
    let present: Vec<&str> = props.iter().map(|p| family_of(p.prop_type)).filter(|f| !f.is_empty()).collect();
    let mut families: Vec<&str> = Vec::new();
    for f in &present {
        if !families.contains(f) {
            families.push(f);
        }
    }
    if families.len() <= 1 {
        return props;
    }
    let mut keep = "";
    for family in CEILING_FAMILY_ORDER {
        if families.contains(family) {
            keep = family;
            break;
        }
    }
    props
        .into_iter()
        .filter(|p| {
            let family = family_of(p.prop_type);
            family.is_empty() || family == keep
        })
        .collect()
}

/// `even_ceiling_fixtures` (streamed_route_generator.gd:3285-3314): the room's ceiling fixtures laid out evenly.
fn even_ceiling_fixtures(mut props: Vec<Prop>, rect: Rect) -> Vec<Prop> {
    let family_of = |t: &str| CEILING_FIXTURE_FAMILY.iter().any(|(k, _)| *k == t);
    let mut fixtures: Vec<usize> = (0..props.len()).filter(|&i| family_of(props[i].prop_type)).collect();
    if fixtures.is_empty() {
        return props;
    }
    let long_x = rect.w >= rect.h;
    let short_len = rect.w.min(rect.h) as f64;
    let rows: usize = if fixtures.len() >= 4 && short_len >= 12.0 { 2 } else { 1 };
    let cols = fixtures.len() / rows;
    let along = |i: &usize| -> f64 {
        let cell = props[*i].cell;
        (if long_x { cell.x } else { cell.y }) as f64
    };
    godot_sort::sort_custom(&mut fixtures, |a, b| along(a) < along(b));
    let long_start = (if long_x { rect.x } else { rect.y }) as f64;
    let long_len = (if long_x { rect.w } else { rect.h }) as f64;
    let short_start = (if long_x { rect.y } else { rect.x }) as f64;
    let mut dropped: Vec<usize> = Vec::new();
    for (index, &i) in fixtures.iter().enumerate() {
        if index >= rows * cols {
            dropped.push(i);
            continue;
        }
        let a = long_start + long_len * ((index % cols) as f64 + 0.5) / cols as f64;
        let b = short_start + short_len * ((index / cols) as f64 + 0.5) / rows as f64;
        let desired: F2 = if long_x { [a as f32, b as f32] } else { [b as f32, a as f32] };
        let cell = props[i].cell;
        props[i].placement_offset = Some(f2sub(desired, f2add(f2(cell), [0.5, 0.5])));
    }
    if dropped.is_empty() {
        return props;
    }
    props.into_iter().enumerate().filter(|(i, _)| !dropped.contains(i)).map(|(_, p)| p).collect()
}

/// `_type_has` (3346): a word of the list is in the type name.
fn type_has(t: &str, words: &[&str]) -> bool {
    words.iter().any(|w| t.contains(w))
}

/// `is_screen_type` (3353).
fn is_screen_type(t: &str) -> bool {
    type_has(t, &["Computer", "Monitor", "Console", "Terminal"]) && !type_has(t, ORIENT_EXCLUDE)
}

/// `_collider_size` of a prop as the orientation code reads it: `collider_size`, else `blocker_size`.
fn orient_size(p: &Prop) -> [f32; 3] {
    prop_collider_size(p)
}

/// `nearest_anchor` (3423-3437): the nearest other prop of a word, within `reach`, not on the child's own point.
fn nearest_anchor(child: &Prop, props: &[Prop], words: &[&str], reach: f64) -> Option<usize> {
    let at = prop_point(child);
    let mut best: Option<usize> = None;
    let mut best_distance = reach;
    for (i, other) in props.iter().enumerate() {
        if std::ptr::eq(other, child) || !type_has(other.prop_type, words) {
            continue;
        }
        let distance = f2distance(at, prop_point(other)) as f64;
        if distance > 0.05 && distance < best_distance {
            best = Some(i);
            best_distance = distance;
        }
    }
    best
}

/// `wall_behind` (3469-3478): the prop's back face is within 15 cm of a wall cell.
fn wall_behind(prop: &Prop, grid: &Grid) -> bool {
    let yaw = prop.yaw;
    let back = [yaw.sin() as f32, yaw.cos() as f32];
    let size = orient_size(prop);
    let probe = f2add(prop_point(prop), f2scale(back, (back_extent(prop.prop_type, size) + 0.15) as f32));
    let cell = V2::new(probe[0].floor() as i32, probe[1].floor() as i32);
    grid_value_at(grid, cell) != 0 && in_grid(cell)
}

/// `back_to_wall` (3440-3459): a solid prop that is not against a wall turns its back to the wall its cell touches.
fn back_to_wall(prop: &mut Prop, grid: &Grid) {
    if prop.wall_mounted || !prop.solid {
        return;
    }
    let cell = prop.cell;
    if wall_behind(prop, grid) {
        return;
    }
    let size = orient_size(prop);
    if size[2] > 0.95 || size[0] > 0.95 {
        return;
    }
    for dir in [V2::new(0, 1), V2::new(0, -1), V2::new(1, 0), V2::new(-1, 0)] {
        let wall = V2::new(cell.x + dir.x, cell.y + dir.y);
        if !in_grid(wall) || grid_value_at(grid, wall) == 0 {
            continue;
        }
        // back = gameplay +Z toward the wall, so the front (-Z) points away from it
        prop.yaw = (dir.x as f64).atan2(dir.y as f64);
        let k = (0.48 - back_extent(prop.prop_type, size)) as f32;
        prop.placement_offset = Some(f2scale(f2(dir), k));
        return;
    }
}

/// `seat_in_front_of_screens` (3386-3420): a screen too wide to turn has its seat moved to the open floor in
/// front of it, then turned to face it.
fn seat_in_front_of_screens(props: &mut [Prop], grid: &Grid) {
    for si in 0..props.len() {
        if !is_screen_type(props[si].prop_type) {
            continue;
        }
        let size = orient_size(&props[si]);
        if size[0] <= 0.95 && size[2] <= 0.95 {
            continue;
        }
        let Some(seat) = nearest_anchor(&props[si], props, &["Chair", "Stool"], SCREEN_SEAT_REACH) else {
            continue;
        };
        let yaw = props[si].yaw;
        let front = [-(yaw.sin()) as f32, -(yaw.cos()) as f32];
        let at = prop_point(&props[si]);
        let to_seat = f2norm(f2sub(prop_point(&props[seat]), at));
        if f2dot(front, to_seat) > 0.7 {
            continue;
        }
        let spot = f2add(at, f2scale(front, (size[2] as f64 * 0.5 + 0.55) as f32));
        let cell = V2::new(spot[0].floor() as i32, spot[1].floor() as i32);
        if !in_grid(cell) || grid_value_at(grid, cell) != 0 {
            continue;
        }
        let mut taken = false;
        for (oi, other) in props.iter().enumerate() {
            if oi != seat && oi != si && other.solid && (f2distance(prop_point(other), spot) < 0.6 || other.occupied_cells.contains(&cell)) {
                taken = true;
                break;
            }
        }
        if taken {
            continue;
        }
        let s = &mut props[seat];
        s.cell = cell;
        s.occupied_cells = vec![cell];
        s.placement_offset = Some(f2sub(spot, f2add(f2(cell), [0.5, 0.5])));
        s.yaw = snap_quarter((front[0] as f64).atan2(front[1] as f64));
        s.oriented = true;
    }
}

/// `orient_props` (streamed_route_generator.gd:3357-3380): every furnished room's props turn to face their pairs.
fn orient_props(props: &mut [Prop], grid: &Grid) {
    seat_in_front_of_screens(props, grid);
    for i in 0..props.len() {
        if is_wall_backed(props[i].prop_type) {
            back_to_wall(&mut props[i], grid);
        }
    }
    for rule in ORIENT_RULES {
        for ci in 0..props.len() {
            let child_type = props[ci].prop_type;
            if !type_has(child_type, rule.child) || type_has(child_type, ORIENT_EXCLUDE) || is_wall_backed(child_type) || props[ci].oriented {
                continue;
            }
            let Some(anchor) = nearest_anchor(&props[ci], props, rule.anchor, rule.reach) else {
                continue;
            };
            let toward = f2sub(prop_point(&props[anchor]), prop_point(&props[ci]));
            let yaw = snap_quarter((-(toward[0]) as f64).atan2(-(toward[1]) as f64));
            let size = orient_size(&props[ci]);
            let turn = wrapf(yaw - props[ci].yaw, -PI, PI).abs();
            if (size[0] > 0.95 || size[2] > 0.95) && turn > 0.01 && (turn - PI).abs() > 0.01 {
                continue;
            }
            props[ci].yaw = yaw;
            props[ci].oriented = true;
        }
    }
}

/// `scale_surface_mounts` (streamed_route_generator.gd:3233-3244): a surface prop's mount rises with its parent's
/// hunter scale. Only props with a `mount_y` are touched.
pub fn scale_surface_mounts(props: &mut [Prop]) {
    for pi in 0..props.len() {
        let mount = props[pi].mount_y;
        if mount <= 0.0 || !is_surface_prop(props[pi].prop_type) || props[pi].mount_scaled {
            continue;
        }
        for parent in 0..props.len() {
            if parent != pi && surface_parent_covers(&props[parent], &props[pi]) {
                let scale = hunter_scale(props[parent].prop_type);
                props[pi].mount_y = mount * scale;
                props[pi].mount_scaled = true;
                break;
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------------------
// the template arms (`_room_template_specs`, streamed_route_generator.gd:3630-4291)

/// `_table_grid_specs` (3582-3600): long tables on a grid, each with `chair_span` chairs along both edges.
fn table_grid_specs(
    table_type: &'static str,
    chair_type: &'static str,
    columns: &[i32],
    rows: &[i32],
    chair_span: i32,
    heads: bool,
    rng: &mut Rng,
) -> Vec<Spec> {
    let mut specs = Vec::new();
    let plain: &'static str = chair_type.strip_suffix("Cushion").unwrap_or(chair_type);
    for &column in columns {
        for &row in rows {
            specs.push(sp(table_type, column, row, "open"));
            if chair_type.is_empty() {
                continue;
            }
            let half = chair_span / 2;
            for offset in -half..=half {
                let pick = if rng.next_float() < 0.6 { chair_type } else { plain };
                specs.push(sp(pick, column + offset, row - 1, "open"));
                let pick = if rng.next_float() < 0.6 { chair_type } else { plain };
                specs.push(sp(pick, column + offset, row + 1, "back"));
            }
            if heads {
                specs.push(sp(chair_type, column - half - 1, row, "right"));
                specs.push(sp(chair_type, column + half + 1, row, "left"));
            }
        }
    }
    specs
}

/// `_room_template_specs` (streamed_route_generator.gd:3630-4291) for the arms the goldens need, plus the
/// region-specific recipes of `_region_room_specs` (see `region_room_specs`). `mirror` is -1 or 1.
pub fn room_template_specs(template: &str, region: &str, side_span: i32, depth_span: i32, mirror: i32, rng: &mut Rng) -> Vec<Spec> {
    let mid = depth_span / 2;
    let m = mirror;
    let frame = frame_type_for_region(region, false);
    let painting = frame_type_for_region(region, true);
    let rug = rug_type_for_region(region);
    let mut specs: Vec<Spec> = Vec::new();
    match template {
        "bedroom" => {
            let mut bed = if side_span >= 9 && rng.next_float() < 0.45 { "doubleBed" } else { "singleBed" };
            if side_span >= 11 && depth_span >= 9 && rng.next_float() < 0.25 {
                bed = "bunkBed";
            }
            specs.push(sp(bed, -1 * m, 0, "open").wall());
            specs.push(sp("bedroomNightstand", 1 * m, 0, "open").wall());
            specs.push(sp("tableLamp", 1 * m, 0, "open").loose().my(0.71));
            specs.push(sp("ceilingFan", 0, mid, "open").loose());
            if !rug.is_empty() {
                specs.push(sp(rug, 0, mid, "open").loose());
            }
            if side_span >= 9 {
                specs.push(sp("bedroomWardrobe", 3 * m, 0, "open").wall());
                specs.push(sp("bedroomDresser", -3 * m, 0, "open").wall());
                specs.push(sp("bedroomVanity", 5 * m, 0, "open").wall());
                specs.push(sp("vanityStool", 5 * m, 1, "back"));
                specs.push(sp("normalBookcase", -5 * m, 2, "center"));
            }
            specs.push(sp("standingMirror", 2 * m, 0, "open").wall());
            if !frame.is_empty() {
                specs.push(sp(frame, -3 * m, 0, "open").mount().loose());
            }
            if depth_span >= 9 {
                specs.push(sp("woodenTable", -3 * m, mid, "center"));
                specs.push(sp("woodenChairCushion", -3 * m, mid + 1, "back"));
                specs.push(sp("normalTvConsole", 2 * m, mid + 1, "center"));
            }
        }
        "gothic_bedroom" => {
            specs.push(sp("gothicDoubleBed", -1 * m, 0, "open").wall());
            specs.push(sp("gothicNightstand", 1 * m, 0, "open").wall());
            specs.push(sp("tableLamp", 1 * m, 0, "open").loose().my(0.76));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
            if side_span >= 9 {
                specs.push(sp("gothicWardrobe", 3 * m, 0, "open").wall());
                specs.push(sp("gothicHighbackChair", -3 * m, mid, "center"));
            }
        }
        "kitchen" => {
            for (ty, side) in [
                ("normalKitchenCounter", -3),
                ("normalKitchenSink", -1),
                ("normalKitchenStove", 1),
                ("normalKitchenFridge", 3),
            ] {
                specs.push(sp(ty, side * m, 0, "open").wall());
            }
            if side_span >= 9 && depth_span >= 9 {
                specs.push(sp("normalKitchenIsland", 0, mid, "open"));
            }
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "mansion_dining" => {
            specs.push(sp("diningSideboard", 0, 0, "open").wall());
            specs.push(sp("diningTable", 0, mid, "open"));
            specs.push(sp("diningChandelier", 0, mid, "open").loose());
            // six side chairs plus two head chairs (the GDScript order of the seat list)
            let mut seats: Vec<(i32, i32, &'static str)> = vec![(-2, mid, "right"), (2, mid, "left")];
            for seat_side in [-1, 0, 1] {
                seats.push((seat_side, mid - 1, "open"));
                seats.push((seat_side, mid + 1, "back"));
            }
            for (side, depth, facing) in seats {
                let ty = if rng.next_float() < 0.5 { "diningChairCushion" } else { "diningChair" };
                specs.push(sp(ty, side, depth, facing));
            }
        }
        "family_dining" => {
            specs.push(sp("woodenDresserSideboard", 0, 0, "open").wall());
            specs.push(sp("woodenTable", 0, mid, "open"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
            for (side, depth, facing) in [(-1, mid, "right"), (1, mid, "left"), (0, mid - 1, "open"), (0, mid + 1, "back")] {
                let ty = if rng.next_float() < 0.45 { "woodenChairCushion" } else { "woodenChair" };
                specs.push(sp(ty, side, depth, facing));
            }
        }
        "restaurant_dining" => {
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
            let mut table_depths: Vec<i32> = vec![1, 4];
            if depth_span >= 12 {
                table_depths.push(7);
            }
            for table_depth in table_depths {
                specs.push(sp("restaurantPartyTable", 0, table_depth, "open"));
                for chair_side in [-1, 0, 1] {
                    specs.push(sp("woodenChairCushion", chair_side, table_depth - 1, "open"));
                    specs.push(sp("woodenChairCushion", chair_side, table_depth + 1, "back"));
                }
            }
        }
        "restaurant_hall" => {
            specs.extend(table_grid_specs("restaurantPartyTable", "woodenChairCushion", &[-6, 0, 6], &[3, 8, 13], 3, false, rng));
            for light_side in [-6, 0, 6] {
                for light_depth in [3, 8, 13] {
                    specs.push(sp("ceilingLight", light_side, light_depth, "open").loose());
                }
            }
            for counter_side in [-6, 0, 6] {
                specs.push(sp("diningSideboard", counter_side, 0, "open").wall());
            }
        }
        "prison_mess" => {
            specs.push(sp("prisonServingCounter", -3 * m, 0, "open").wall());
            specs.push(sp("prisonKitchenRack", 4 * m, 0, "open").wall());
            specs.extend(table_grid_specs("prisonCafeteriaTable", "", &[-6, 0, 6], &[4, 9, 14], 0, false, rng));
            for light_side in [-6, 0, 6] {
                for light_depth in [4, 9, 14] {
                    specs.push(sp("ceilingLight", light_side, light_depth, "open").loose());
                }
            }
            specs.push(sp("prisonTrayReturn", 7 * m, 17, "open"));
        }
        "dining_hall" => {
            specs.extend(table_grid_specs("diningTable", "diningChairCushion", &[-5, 5], &[4, 9, 14], 3, true, rng));
            for light_side in [-5, 5] {
                for light_depth in [4, 9, 14] {
                    specs.push(sp("diningChandelier", light_side, light_depth, "open").loose());
                }
            }
            specs.push(sp("diningSideboard", 0, 0, "open").wall());
        }
        "prison_cafeteria" => {
            // the serving line is the room's point: it claims before the hunter-scale tables
            specs.push(sp("prisonServingCounter", 0, 0, "open").wall().claim());
            // side kit beside the table line, at the walls where the room is wide enough
            let kit_side = if side_span >= 7 { 3 } else { 1 };
            specs.push(sp("prisonKitchenRack", -kit_side * m, 2.max(mid), "open"));
            // linear wings are five cells deep: a middle-row table keeps the serving line clear
            let mut table_depths: Vec<i32> = vec![1];
            if depth_span >= 9 {
                table_depths = vec![mid];
            }
            if depth_span >= 12 {
                table_depths = vec![mid - 1, mid + 2];
            }
            if depth_span >= 15 {
                table_depths.push(mid + 6);
            }
            for table_depth in table_depths {
                if table_depth <= 0 {
                    continue;
                }
                specs.push(sp("prisonCafeteriaTable", 0, table_depth, "open"));
            }
            // the tray return sits at the side of the queue
            specs.push(sp("prisonTrayReturn", kit_side * m, 2.max(mid), "open").claim());
            specs.push(sp("ceilingLight", -2, mid, "open").loose());
            specs.push(sp("ceilingLight", 2, mid, "open").loose());
            specs.push(sp("supplyCrates", -1 * m, depth_span - 1, "back"));
        }
        "thermal_chamber" => {
            specs.push(sp("lavaVent", -2 * m, mid, "center"));
            specs.push(sp("lavaVent", 2 * m, mid, "center"));
            specs.push(sp("magmaCrystalCluster", 0, mid + 1, "center"));
            specs.push(sp("infernoChainLantern", 0, mid, "open").loose());
        }
        "forge" => {
            specs.push(sp("lavaPipeManifold", -2 * m, 0, "open").wall());
            specs.push(sp("emberBrazier", 2 * m, mid, "center"));
            specs.push(sp("charredSupplyCrate", 0, mid + 1, "center"));
            specs.push(sp("infernoChainLantern", 0, mid, "open").loose());
        }
        "altar_room" => {
            specs.push(sp("infernoAltar", 0, mid, "center"));
            specs.push(sp("emberBrazier", -2 * m, mid + 1, "center"));
            specs.push(sp("emberBrazier", 2 * m, mid + 1, "center"));
            specs.push(sp("infernoChainLantern", 0, mid - 1, "open").loose());
        }
        "lava_gallery" => {
            specs.push(sp("basaltPillar", -2 * m, mid, "center"));
            specs.push(sp("basaltPillar", 2 * m, mid, "center"));
            specs.push(sp("basaltRubble", -1 * m, mid + 2, "center"));
            specs.push(sp("magmaCrystalCluster", 1 * m, mid + 2, "center"));
            specs.push(sp("infernoChainLantern", 0, mid, "open").loose());
        }
        "house_bathroom" | "hotel_bathroom" => {
            let compact = template == "hotel_bathroom" || side_span < 7;
            let wet_side = if compact { -1 } else { -2 };
            let toilet_side = if compact { 1 } else { 2 };
            let wet_type = if compact {
                "houseShowerEnclosure"
            } else if rng.next_float() < 0.55 {
                "houseBathtub"
            } else {
                "houseShowerEnclosure"
            };
            specs.push(sp(wet_type, wet_side, 0, "open").wall());
            specs.push(sp("houseToilet", toilet_side, 0, "open").wall());
            specs.push(sp("bathroomVanity", 0, 0, "open").wall());
            specs.push(sp("bathroomMirror", 0, 0, "open").mount());
            specs.push(sp("houseBathMat", wet_side, 2, "open").loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
            if !compact {
                specs.push(sp("houseTowelRack", 3, 0, "open").mount());
            }
        }
        "luxury_bathroom" => {
            specs.push(sp("houseBathtub", -2 * m, 0, "open").wall());
            specs.push(sp("houseShowerEnclosure", 2 * m, 0, "open").wall());
            specs.push(sp("bathroomVanity", 0, 0, "open").wall());
            specs.push(sp("houseToilet", 3 * m, mid, "center"));
            specs.push(sp("bathroomMirror", 0, 0, "open").mount());
            specs.push(sp("houseTowelRack", -3 * m, 0, "open").mount());
            specs.push(sp("houseBathMat", 0, mid + 1, "open").loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "public_restroom" => {
            for stall_side in [-2, 0] {
                specs.push(sp("bathroomStall", stall_side, 0, "open").wall());
            }
            specs.push(sp("publicRestroomToilet", 2, 0, "open").wall());
            specs.push(sp("publicSinkBank", 0, mid, "back"));
            specs.push(sp("bathroomMirror", 0, 0, "open").mount());
            specs.push(sp("publicHandDryer", 3, 0, "open").mount());
            specs.push(sp("publicSoapDispenser", -3, 0, "open").mount());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "arcade" => {
            // two opposing cabinet rows share their backs while both fronts keep a playable aisle
            for side in [-2, 0, 2] {
                let cabinet = if rng.next_float() < 0.34 { "arcadeCabinetCyanMagenta" } else { "arcadeCabinet" };
                specs.push(sp(cabinet, side, mid, "back"));
                specs.push(sp("arcadeCabinet", side, mid + 1, "open"));
            }
            if side_span >= 9 {
                specs.push(sp("vendingMachine", 3 * m, 0, "open").wall());
            }
        }
        "carnival_midway" => {
            if side_span >= 17 {
                let stall_types = ["ringTossStall", "shootingGallery", "whackAMole"];
                let stall_sides = [-7, 0, 7];
                for i in 0..3 {
                    specs.push(sp(stall_types[i], stall_sides[i], 0, "open").wall());
                }
                specs.push(sp("carnivalTicketBooth", -4, 0, "open").wall());
                specs.push(sp("prizeClawMachine", 4, 0, "open").wall());
                specs.push(sp("strengthTester", -9, 1, "center"));
                specs.push(sp("strengthTester", 9, 1, "center"));
                for cart_side in [-6, 6] {
                    specs.push(sp("popcornCart", cart_side, mid + 1, "center"));
                }
                specs.push(sp("circusPedestal", 0, mid + 1, "center"));
                for light_side in [-6, 0, 6] {
                    specs.push(sp("ceilingLight", light_side, mid, "open").loose());
                }
            } else {
                specs.push(sp("carnivalTicketBooth", -2 * m, 0, "open").wall());
                specs.push(sp("popcornCart", 2 * m, mid, "center"));
                specs.push(sp("circusPedestal", -2 * m, mid + 2, "center"));
            }
        }
        "carnival_big_top" => {
            specs.push(sp("arcadeCarpetCircle", 0, mid, "open").loose());
            if side_span >= 17 && depth_span >= 17 {
                specs.push(sp("carouselCenterpiece", 0, mid, "center"));
            }
            let flank = if side_span >= 17 { 8 } else { 6 };
            specs.push(sp("duckPond", flank, mid, "center"));
            specs.push(sp("circusPedestal", -flank, mid, "center"));
            for booth_side in [-8, 8] {
                specs.push(sp("carnivalTicketBooth", booth_side, 0, "open").wall());
            }
            for cart_side in [-4, 4] {
                specs.push(sp("popcornCart", cart_side, 1, "open"));
            }
            for light_side in [-6, 0, 6] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "boardwalk" => {
            for umbrella_side in [-7, -1, 5] {
                specs.push(sp("beachUmbrella", umbrella_side, mid, "center"));
                specs.push(sp("beachChair", umbrella_side - 2, mid + 1, "center"));
                specs.push(sp("beachChair", umbrella_side + 2, mid + 1, "center"));
                specs.push(sp("beachMat", umbrella_side, mid + 2, "open").loose());
            }
            specs.push(sp("tikiBar", -8, 0, "open").wall());
            specs.push(sp("surfboardRack", 8, 0, "open").wall());
        }
        "prison_cell" => {
            specs.push(sp("prisonBunk", -3, 0, "open").wall());
            specs.push(sp("prisonBunk", 3, 0, "open").wall());
            specs.push(sp("houseToilet", -1, 0, "open").wall());
            specs.push(sp("bathroomSink", 1, 0, "open").wall());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "gothic_archive" => {
            for shelf_side in (-8..=8).step_by(2) {
                specs.push(sp("gothicBookcase", shelf_side, 0, "open").wall());
            }
            for desk_side in [-5, 5] {
                specs.push(sp("gothicPedestalTable", desk_side, mid, "open"));
                specs.push(sp("gothicHighbackChair", desk_side, mid - 1, "open"));
                specs.push(sp("gothicHighbackChair", desk_side, mid + 1, "back"));
            }
            specs.push(sp("libraryGlobe", 3, 1, "center"));
            specs.push(sp("readingTable", 0, mid - 2, "open"));
            specs.push(sp("gothicHighbackChair", 0, mid - 3, "open"));
            specs.push(sp("gothicBedroomRug", 0, mid, "open").loose());
            for light_side in [-6, 0, 6] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "gothic_chapel" => {
            specs.push(sp("gothicPedestalTable", 0, 0, "open").wall());
            for candle_side in [-3, 3] {
                specs.push(sp("gothicFloorCandelabra", candle_side, 0, "open").wall());
            }
            for pew_row in [3, 5, 7] {
                for pew_side in [-6, -3, 3, 6] {
                    specs.push(sp("gothicBedroomBench", pew_side, pew_row, "back"));
                }
            }
            specs.push(sp("gothicBedroomRug", 0, 4, "open").loose());
            for light_side in [-6, 0, 6] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "gothic_crypt" => {
            for coffin_side in [-8, -4, 0, 4, 8] {
                specs.push(sp("gothicCoffinChest", coffin_side, 2, "open"));
            }
            for coffin_side in [-6, -2, 2, 6] {
                specs.push(sp("gothicCoffinChest", coffin_side, 6, "open"));
            }
            for candle_side in [-9, 9] {
                specs.push(sp("gothicFloorCandelabra", candle_side, 0, "open").wall());
            }
        }
        "infirmary" => {
            for bed_side in [-4, 0, 4] {
                specs.push(sp("hospitalBed", bed_side, 0, "open").wall());
                specs.push(sp("ivStand", bed_side + 2, 0, "open"));
            }
            for curtain_side in [-2, 2] {
                specs.push(sp("privacyCurtain", curtain_side, 2, "right"));
            }
            for cart_side in [-3, 3] {
                specs.push(sp("medicalCart", cart_side, mid + 1, "back"));
            }
            for light_side in [-3, 3] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "maze_storage" => {
            for shelf_side in (-8..=8).step_by(2) {
                let ty = if shelf_side % 4 == 0 { "storageShelving" } else { "normalBookcase" };
                specs.push(sp(ty, shelf_side, 0, "open").wall());
            }
            for barrel_side in [-8, -7, 7, 8] {
                specs.push(sp("storageBarrel", barrel_side, mid + 2, "open"));
            }
            for crate_side in [-6, -2, 2, 6] {
                specs.push(sp("supplyCrates", crate_side, mid, "open"));
            }
            for box_side in [-4, 4] {
                specs.push(sp("officeFileBoxes", box_side, mid + 2, "open"));
            }
            for light_side in [-6, 0, 6] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "carnival_game_stall" => {
            specs.push(sp("ringTossStall", -2 * m, 0, "open").wall());
            specs.push(sp("strengthTester", 2 * m, 0, "open").wall());
            if depth_span >= 9 {
                specs.push(sp("popcornCart", 0, mid + 2, "center"));
            }
        }
        "carnival_backstage" => {
            for side in [-2, 0, 2] {
                specs.push(sp("arcadeCabinet", side, 0, "open").wall());
            }
        }
        "frozen_tideline" => {
            specs.push(sp("frozenTidelinePool", 0, mid, "open").loose());
            specs.push(sp("beachChair", -3 * m, mid, "center"));
            specs.push(sp("beachUmbrella", 3 * m, mid + 2, "center"));
            if side_span >= 11 {
                specs.push(sp("tikiBar", -4 * m, 0, "open").wall());
                specs.push(sp("surfboardRack", 4 * m, 0, "open").wall());
            }
        }
        "beach_hut" => {
            specs.push(sp("frozenBeachHut", -2 * m, 0, "open").wall());
            specs.push(sp("beachChair", 2 * m, mid, "center"));
        }
        "apartment_living" => {
            specs.push(sp("normalTvConsole", 0, 0, "open").wall());
            specs.push(sp("normalBookcase", -3 * m, 0, "open").wall());
            specs.push(sp("floorLamp", -2 * m, 0, "open").wall());
            specs.push(sp("normalCoffeeTable", 0, 3, "back"));
            specs.push(sp("loungeCouchWide", 0, 5, "back"));
            specs.push(sp("loungeCouch", 3 * m, 3, "center"));
            specs.push(sp("diningTable", -2 * m, 7, "open"));
            specs.push(sp("woodenChairCushion", -2 * m, 6, "center"));
            specs.push(sp("woodenChairCushion", -2 * m, 8, "center"));
            specs.push(sp("ceilingLight", 0, 4, "open").loose());
            if !rug.is_empty() {
                specs.push(sp(rug, 0, 3, "open").loose());
            }
            if !painting.is_empty() {
                specs.push(sp(painting, 1 * m, 0, "open").mount().loose());
            }
        }
        "lounge" => {
            specs.push(sp("loungeCouchWide", -1 * m, 0, "open").wall());
            for plant_side in [-4, 4] {
                specs.push(sp("pottedPlant", plant_side, 0, "open").wall());
            }
            specs.push(sp("libraryGlobe", -3 * m, mid, "center"));
            specs.push(sp("normalCoffeeTable", -1 * m, mid, "back"));
            specs.push(sp("normalTvConsole", 2 * m, mid, "center"));
            specs.push(sp("floorLamp", 2 * m, 0, "open").wall());
            specs.push(sp("ceilingFan", 0, mid, "open").loose());
            if !rug.is_empty() {
                specs.push(sp(rug, -1 * m, mid, "open").loose());
            }
            if !painting.is_empty() {
                specs.push(sp(painting, 2 * m, 0, "open").mount().loose());
            }
        }
        "office_reception" => {
            specs.push(sp("receptionDesk", -1 * m, 1, "open"));
            specs.push(sp("officeChair", -1 * m, 0, "open"));
            for plant_side in [-2, 1] {
                specs.push(sp("pottedPlant", plant_side * m, 0, "open").wall());
            }
            specs.push(sp("filingCabinet", 2 * m, 0, "open").wall());
            specs.push(sp("officeWaterCooler", 2 * m, mid + 1, "center"));
            specs.push(sp("normalPictureFrameEmpty", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid - 1, "open").loose());
        }
        "office_cubicles" => {
            for (side, depth, computer) in [(-1, 1, "retroComputerTerminal"), (1, 3, "retroComputerTower")] {
                specs.push(sp("officeDesk", side, depth, "open"));
                specs.push(sp("officeChair", side, depth + 1, "back"));
                specs.push(sp(computer, side, depth, "open").loose().my(0.78));
            }
            specs.push(sp("filingCabinet", -1 * m, 0, "open").wall());
            specs.push(sp("officeDesk", 0, 2, "open"));
            specs.push(sp("officeCubiclePartition", 0, 0, "open"));
            specs.push(sp("officePrinter", 0, 2, "open").loose().my(0.78));
            specs.push(sp("officePhotocopier", -1 * m, 4, "center"));
            specs.push(sp("officeFileBoxes", 1 * m, 0, "open").wall());
            specs.push(sp("normalPictureFrameEmpty", 1 * m, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", -1, mid, "open").loose());
            specs.push(sp("ceilingLight", 1, mid, "open").loose());
        }
        "office_meeting" => {
            specs.push(sp("woodenTable", 0, mid, "open"));
            specs.push(sp("woodenTable", 0, mid + 2, "open"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
            for (side, depth, facing) in [(-1, mid, "right"), (1, mid, "left"), (0, mid - 1, "open"), (0, mid + 1, "back")] {
                specs.push(sp("woodenChairCushion", side, depth, facing));
            }
            specs.push(sp("filingCabinet", -2 * m, 0, "open").wall());
            specs.push(sp("retroComputerTower", 0, mid, "open").loose().my(0.78));
            specs.push(sp("normalTvConsole", 0, mid + 3, "center"));
            specs.push(sp("normalPaintingFrameEmpty", 2 * m, 0, "open").mount().loose());
        }
        "office_job" => {
            specs.push(sp("officeDesk", -1, mid, "open"));
            specs.push(sp("officeDesk", 1, mid + 1, "open"));
            specs.push(sp("officeDesk", 0, 1, "open"));
            for (side, depth, facing) in [
                (-1, mid - 1, "right"),
                (1, mid - 1, "left"),
                (-1, mid + 1, "left"),
                (1, mid + 1, "right"),
                (0, 2, "back"),
            ] {
                specs.push(sp("officeChair", side, depth, facing));
            }
            specs.push(sp("officePhotocopier", -1 * m, 0, "open").wall());
            specs.push(sp("officeFileBoxes", 1 * m, 0, "open").wall());
            specs.push(sp("filingCabinet", 0, 0, "open").wall());
            specs.push(sp("retroComputerTerminal", -1, mid, "open").loose().my(0.78));
            specs.push(sp("retroComputerTower", 1, mid + 1, "open").loose().my(0.78));
            specs.push(sp("retroComputerDesktop", 0, 1, "open").loose().my(0.78));
            specs.push(sp("normalTvConsole", 0, mid + 3, "center"));
            specs.push(sp("officeWaterCooler", 1 * m, 4, "center"));
            specs.push(sp("officeShredder", -1 * m, 4, "center"));
            specs.push(sp("normalPaintingFrameEmpty", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid - 1, "open").loose());
        }
        "private_office" => {
            specs.push(sp("officeCornerWorkstation", 0, 1, "open"));
            specs.push(sp("retroComputerDesktop", 0, 1, "open").loose().my(0.78));
            specs.push(sp("officePhotocopier", -1 * m, 4, "center"));
            specs.push(sp("filingCabinet", 1 * m, 4, "center"));
            specs.push(sp("officeChair", 0, 3, "back"));
            specs.push(sp("woodenChair", -1 * m, 3, "right"));
            specs.push(sp("woodenChair", 1 * m, 3, "left"));
            specs.push(sp("normalTvConsole", 0, 4, "center"));
            specs.push(sp("normalPaintingFrameEmpty", 0, 0, "open").mount().loose());
            specs.push(sp("officeWaterCooler", 1 * m, 0, "open").wall());
            specs.push(sp("ceilingLight", 0, mid - 1, "open").loose());
        }
        _ => {}
    }
    if ["hospital", "lab", "greenhouse", "mall"].contains(&region) {
        specs.extend(region_room_specs(template, (side_span - 3) / 2, depth_span, mirror));
    }
    if MUSIC_TEMPLATES.contains(&template) {
        specs.extend(music_wing_specs(template, (side_span - 3) / 2, depth_span, mirror));
    }
    if ["thermal_chamber", "forge", "altar_room", "lava_gallery"].contains(&template) && side_span >= 17 && depth_span <= 11 {
        let originals = specs.clone();
        for shift in [-7, 7] {
            for original in &originals {
                let mut copy = original.clone();
                copy.side += shift;
                specs.push(copy);
            }
        }
    }
    if region == "mansion" {
        for spec in specs.iter_mut() {
            if spec.ty == "ceilingFan" || spec.ty == "ceilingLight" {
                spec.ty = "diningChandelier";
            }
        }
    }
    if region == "mansion" && ["bedroom", "lounge", "mansion_dining", "dining_hall"].contains(&template) {
        let corner = 2.max(side_span / 2 - 1);
        for column_side in [-corner, corner] {
            specs.push(sp("mansionMarbleColumn", column_side, 0, "open").wall());
        }
        for vase_side in [-corner + 1, corner - 1] {
            specs.push(sp("mansionPorcelainVase", vase_side, 1, "open"));
        }
    }
    keep_entrance_clear(specs, depth_span)
}

/// `_keep_entrance_clear` (streamed_route_generator.gd:4297-4320): a solid spec that covers the door aisle (the
/// middle column in front of the door) is dropped.
fn keep_entrance_clear(specs: Vec<Spec>, depth_span: i32) -> Vec<Spec> {
    let mut kept = Vec::new();
    for spec in specs {
        if spec.wall_backed || spec.solid == Some(false) {
            kept.push(spec);
            continue;
        }
        let size = lookup(PROP_COLLIDER_SIZES, spec.ty).unwrap_or([1.0, 1.0, 1.0]);
        let (sx, sz) = (size[0] as f64, size[2] as f64);
        let mut across_half = sx.max(sz) * 0.5;
        let mut toward_half = across_half;
        match spec.facing {
            "open" | "back" => {
                across_half = sx * 0.5;
                toward_half = sz * 0.5;
            }
            "left" | "right" => {
                across_half = sz * 0.5;
                toward_half = sx * 0.5;
            }
            _ => {}
        }
        let covers_door = (spec.side as f64).abs() - across_half < 0.5 && spec.depth as f64 + toward_half > depth_span as f64 - 3.5;
        if !covers_door {
            kept.push(spec);
        }
    }
    kept
}

// ----------------------------------------------------------------------------------------------------------
// region recipes (`_region_room_specs`, streamed_route_generator.gd:4339-4824) are in `furnished_regions`
// ----------------------------------------------------------------------------------------------------------

// ----------------------------------------------------------------------------------------------------------
// region recipes (`_region_room_specs`, streamed_route_generator.gd:4339-4824): the hospital, research lab,
// greenhouse and mall rooms. Their recipes are written for the 11-wide bank room; `front` is the entrance row.

/// `REGION_REPEAT_TEMPLATES` (streamed_route_generator.gd:103-105): laid twice, at sides -5 and +5, in a 21-wide hall.
const REGION_REPEAT_TEMPLATES: &[&str] = &[
    "operating_theatre",
    "icu",
    "morgue",
    "pharmacy",
    "imaging",
    "supply_closet",
    "nurses_station",
    "waiting_room",
    "clean_room",
    "control_room",
    "decon",
    "chem_lab",
    "bio_lab",
    "cold_storage",
    "lab_office",
    "palm_house",
    "cactus_room",
    "potting_shed",
    "irrigation_room",
    "overgrown_room",
    "glasshouse",
];

/// `_region_room_specs(template, half, depth_span, mirror)` (streamed_route_generator.gd:4339-4824). Ported arms
/// for every template of the hospital, lab, greenhouse and mall regions; the repeat (4815-4822) follows the arms.
pub fn region_room_specs(template: &str, half: i32, depth_span: i32, mirror: i32) -> Vec<Spec> {
    let front = depth_span - 3;
    let mid = front / 2;
    let m = mirror;
    let mut specs: Vec<Spec> = Vec::new();
    match template {
        // ---- hospital ----
        "ward" => {
            let bed_sides: Vec<i32> = (-half..half + 1).step_by(4).collect();
            for (bed_index, &bed_side) in bed_sides.iter().enumerate() {
                specs.push(sp("hospitalBed", bed_side, 0, "open").wall());
                if bed_side + 2 <= half {
                    specs.push(sp("hospitalCurtain", bed_side + 2, 1, "right"));
                }
                let kit_side = if bed_side + 1 <= half { bed_side + 1 } else { bed_side - 1 };
                let kit = if bed_index % 2 == 0 { "ivDripStand" } else { "vitalsMonitor" };
                specs.push(sp(kit, kit_side, 4, "back"));
            }
            if depth_span >= 15 {
                for bed_side in (-half..half + 1).step_by(4) {
                    if bed_side.abs() <= 2 {
                        continue;
                    }
                    specs.push(sp("hospitalBed", bed_side, front - 1, "back"));
                    if bed_side + 2 <= half && (bed_side + 2).abs() > 2 {
                        specs.push(sp("hospitalCurtain", bed_side + 2, front - 1, "right"));
                    }
                }
                specs.push(sp("nursesStation", 0, mid, "open"));
                specs.push(sp("officeChair", -1, mid - 1, "open"));
                specs.push(sp("officeChair", 1, mid - 1, "open"));
                specs.push(sp("medicalSupplyRack", -4, mid, "center"));
                specs.push(sp("wheelchair", 4, mid, "center"));
            }
            for light_side in (-half + 2..half).step_by(4) {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
            specs.push(sp("hospitalWardSign", 1, 0, "open").mount().loose());
        }
        "nurses_station" => {
            specs.push(sp("nursesStation", 0, 2, "open"));
            specs.push(sp("officeChair", -1, 1, "open"));
            specs.push(sp("officeChair", 1, 1, "open"));
            specs.push(sp("medicalSupplyRack", -3 * m, 0, "open").wall());
            specs.push(sp("filingCabinet", 3 * m, 0, "open").wall());
            specs.push(sp("filingCabinet", 4 * m, 0, "open").wall());
            specs.push(sp("vitalsMonitor", -4 * m, 3, "center"));
            specs.push(sp("ivDripStand", -3 * m, 4, "center"));
            specs.push(sp("wheelchair", 3 * m, 4, "center"));
            specs.push(sp("hospitalWardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "waiting_room" => {
            for row_side in [-3, 0, 3] {
                specs.push(sp("waitingChairs", row_side, 0, "open").wall());
            }
            specs.push(sp("waitingChairs", -3, 3, "open"));
            specs.push(sp("waitingChairs", 3, 3, "open"));
            specs.push(sp("vendingMachine", -4 * m, front - 1, "center"));
            specs.push(sp("officeWaterCooler", 4 * m, front - 1, "center"));
            specs.push(sp("bananaPlant", 4 * m, 2, "center"));
            specs.push(sp("magazineRack", -4 * m, 2, "center"));
            specs.push(sp("hospitalWardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", -2, mid, "open").loose());
            specs.push(sp("ceilingLight", 2, mid, "open").loose());
        }
        "operating_theatre" => {
            specs.push(sp("operatingTable", 0, 2, "open"));
            specs.push(sp("surgicalLight", 0, 2, "open").loose());
            specs.push(sp("instrumentTrolley", 2 * m, 2, "center"));
            specs.push(sp("vitalsMonitor", -2 * m, 1, "center"));
            specs.push(sp("ivDripStand", -2 * m, 3, "center"));
            specs.push(sp("medicalSupplyRack", 3 * m, 0, "open").wall());
            specs.push(sp("biohazardBin", -4 * m, 0, "open").wall());
            specs.push(sp("xrayLightbox", 0, 0, "open").mount().loose());
            specs.push(sp("hazardSign", -2 * m, 0, "open").mount().loose());
        }
        "icu" => {
            for bay in [-2, 2] {
                specs.push(sp("hospitalBed", bay, 0, "open").wall());
                specs.push(sp("vitalsMonitor", bay * 2, 0, "open").wall());
                specs.push(sp("ivDripStand", bay.signum(), 0, "center"));
            }
            specs.push(sp("instrumentTrolley", -4 * m, 5, "center"));
            specs.push(sp("medicalSupplyRack", 4 * m, 5, "center"));
            specs.push(sp("hospitalWardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", -2, mid, "open").loose());
            specs.push(sp("ceilingLight", 2, mid, "open").loose());
        }
        "morgue" => {
            specs.push(sp("morgueCabinet", -2, 0, "open").wall());
            specs.push(sp("morgueCabinet", 2, 0, "open").wall());
            specs.push(sp("autopsyTable", 0, 3, "open"));
            specs.push(sp("instrumentTrolley", 2 * m, 3, "center"));
            specs.push(sp("biohazardBin", -4 * m, 3, "center"));
            specs.push(sp("hazardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "pharmacy" => {
            for shelf_side in [-3, 0, 3] {
                specs.push(sp("pharmacyShelf", shelf_side, 0, "open").wall());
            }
            specs.push(sp("nursesStation", 0, 3, "open"));
            specs.push(sp("officeChair", 0, 2, "open"));
            specs.push(sp("medicalSupplyRack", -3 * m, 3, "center"));
            specs.push(sp("filingCabinet", 4 * m, 3, "center"));
            specs.push(sp("hospitalWardSign", 2 * m, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, 1, "open").loose());
        }
        "imaging" => {
            specs.push(sp("xrayMachine", 0, 1, "open"));
            specs.push(sp("xrayLightbox", -3 * m, 0, "open").mount().loose());
            specs.push(sp("hazardSign", 3 * m, 0, "open").mount().loose());
            specs.push(sp("officeDesk", 3 * m, 3, "center"));
            specs.push(sp("retroComputerDesktop", 3 * m, 3, "center").loose().my(0.78));
            specs.push(sp("officeChair", 4 * m, 3, "center"));
            specs.push(sp("wheelchair", -3 * m, 3, "center"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "supply_closet" => {
            for rack_side in [-3, 0, 3] {
                specs.push(sp("medicalSupplyRack", rack_side, 0, "open").wall());
            }
            specs.push(sp("supplyCrates", -3 * m, 3, "center"));
            specs.push(sp("utilityCart", 3 * m, 3, "center"));
            specs.push(sp("wheelchair", -4 * m, front - 1, "center"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        // ---- research lab ----
        "clean_room" => {
            // glass airlock: two partitions across the front leave a one-door gap behind the entrance
            specs.push(sp("labGlassPartition", -3, front - 2, "open"));
            specs.push(sp("labGlassPartition", 3, front - 2, "open"));
            specs.push(sp("labBench", -2, 0, "open").wall());
            specs.push(sp("labBench", 2, 0, "open").wall());
            specs.push(sp("labMicroscope", -2, 0, "open").loose().my(0.92));
            specs.push(sp("labStool", -2, 1, "back"));
            specs.push(sp("labStool", 2, 1, "back"));
            specs.push(sp("specimenTank", 4 * m, 2, "center"));
            specs.push(sp("hazardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", -2, mid - 1, "open").loose());
            specs.push(sp("ceilingLight", 2, mid - 1, "open").loose());
        }
        "containment" => {
            let cell_sides: Vec<i32> = if half < 9 { vec![-3, 3] } else { vec![-7, -3, 3, 7] };
            let tank_sides: Vec<i32> = if half < 9 { vec![0] } else { vec![-5, 0, 5] };
            for &cell_side in &cell_sides {
                specs.push(sp("containmentCell", cell_side, 0, "open").wall());
                specs.push(sp("hazardSign", cell_side, 0, "open").mount().loose());
            }
            for &tank_side in &tank_sides {
                specs.push(sp("specimenTank", tank_side, 0, "open").wall());
            }
            if half >= 9 || depth_span >= 11 {
                specs.push(sp("controlConsole", 0, mid + 1, "back"));
                specs.push(sp("officeChair", -1, mid + 2, "back"));
                specs.push(sp("officeChair", 1, mid + 2, "back"));
            }
            if depth_span >= 15 {
                for &cell_side in &cell_sides {
                    specs.push(sp("containmentCell", cell_side, front - 1, "back"));
                }
                for tank_side in [-5, 5] {
                    specs.push(sp("specimenTank", tank_side, mid, "center"));
                }
            }
            specs.push(sp("biohazardBin", -half * m, mid, "center"));
            for light_side in (-half + 2..half).step_by(4) {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "server_hall" => {
            for rack_side in -half..half + 1 {
                specs.push(sp("serverRack", rack_side, 0, "open").wall());
            }
            let mut island = 2;
            while island + 1 <= front - 3 {
                for rack_side in (-half + 3)..(half - 2) {
                    specs.push(sp("serverRack", rack_side, island, "back"));
                    specs.push(sp("serverRack", rack_side, island + 1, "open"));
                }
                specs.push(sp("labCoolingUnit", -half, island, "center"));
                specs.push(sp("labCoolingUnit", half, island + 1, "center"));
                island += 6;
            }
            specs.push(sp("hazardSign", 0, 0, "open").mount().loose());
            for light_side in (-half + 1..half).step_by(3) {
                specs.push(sp("ceilingLight", light_side, 1, "open").loose());
            }
        }
        "pod_room" => {
            let pod_sides = [-5, -3, -1, 1, 3, 5];
            for (pod_index, &base_side) in pod_sides.iter().enumerate() {
                let pod_side = base_side * m;
                let pod_type = if pod_index == 1 || pod_index == 4 {
                    "divePodFilm"
                } else if pod_index == 2 {
                    "divePodLit"
                } else {
                    "divePod"
                };
                let outer = pod_index == 0 || pod_index == pod_sides.len() - 1;
                let pod = sp(pod_type, pod_side, if outer { 1 } else { 0 }, "open");
                specs.push(if outer { pod } else { pod.wall() });
                specs.push(sp("diveHeadsetStand", pod_side + m, 0, "open").wall());
            }
            specs.push(sp("diveConsole", (half - 1) * m, 0, "open").wall());
            specs.push(sp("diveHeadsetShelf", (half - 1) * m, 0, "open").mount().loose().my(1.45));
        }
        "control_room" => {
            specs.push(sp("monitorWall", 0, 0, "open").mount().loose());
            specs.push(sp("controlConsole", 0, 2, "back"));
            specs.push(sp("officeChair", -1, 3, "back"));
            specs.push(sp("officeChair", 1, 3, "back"));
            specs.push(sp("serverRack", -4, 0, "open").wall());
            specs.push(sp("serverRack", 4, 0, "open").wall());
            specs.push(sp("filingCabinet", 4 * m, front - 1, "center"));
            specs.push(sp("officeWaterCooler", -4 * m, front - 1, "center"));
            specs.push(sp("labWhiteboard", -3 * m, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "decon" => {
            specs.push(sp("deconShower", -2, 0, "open").wall());
            specs.push(sp("deconShower", 2, 0, "open").wall());
            specs.push(sp("locker", -4, 0, "open").wall());
            specs.push(sp("locker", 4, 0, "open").wall());
            specs.push(sp("labGlassPartition", -3, front - 2, "open"));
            specs.push(sp("labGlassPartition", 3, front - 2, "open"));
            specs.push(sp("publicShowerBench", -3, 1, "open"));
            specs.push(sp("biohazardBin", -4 * m, 3, "center"));
            specs.push(sp("hazardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, 2, "open").loose());
        }
        "chem_lab" => {
            specs.push(sp("fumeHood", -3 * m, 0, "open").wall());
            specs.push(sp("labBench", 1 * m, 0, "open").wall());
            specs.push(sp("gasCylinderRack", 4 * m, 0, "open").wall());
            specs.push(sp("labBench", 0, 3, "open"));
            specs.push(sp("labMicroscope", 0, 3, "open").loose().my(0.92));
            specs.push(sp("labStool", -1, 4, "back"));
            specs.push(sp("labStool", 1, 4, "back"));
            specs.push(sp("biohazardBin", -4 * m, 3, "center"));
            specs.push(sp("hazardSign", -1 * m, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid - 1, "open").loose());
        }
        "bio_lab" => {
            specs.push(sp("specimenTank", -4 * m, 0, "open").wall());
            specs.push(sp("specimenTank", -2 * m, 0, "open").wall());
            specs.push(sp("labBench", 2 * m, 0, "open").wall());
            specs.push(sp("labMicroscope", 2 * m, 0, "open").loose().my(0.92));
            specs.push(sp("labBench", 0, 3, "open"));
            specs.push(sp("labStool", 0, 4, "back"));
            specs.push(sp("coldStorageUnit", 3 * m, 3, "center"));
            specs.push(sp("biohazardBin", -4 * m, 3, "center"));
            specs.push(sp("hazardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid - 1, "open").loose());
        }
        "cold_storage" => {
            for unit_side in [-3, 0, 3] {
                specs.push(sp("coldStorageUnit", unit_side, 0, "open").wall());
            }
            specs.push(sp("gasCylinderRack", -4 * m, 3, "center"));
            specs.push(sp("supplyCrates", 3 * m, 3, "center"));
            specs.push(sp("hazardSign", 2 * m, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "lab_office" => {
            for desk_side in [-2, 2] {
                specs.push(sp("officeDesk", desk_side, 1, "open"));
                let screen = if desk_side < 0 { "retroComputerDesktop" } else { "retroComputerTerminal" };
                specs.push(sp(screen, desk_side, 1, "open").loose().my(0.78));
                specs.push(sp("officeChair", desk_side, 0, "open"));
            }
            specs.push(sp("labWhiteboard", 0, 0, "open").mount().loose());
            specs.push(sp("filingCabinet", -4, 0, "open").wall());
            specs.push(sp("filingCabinet", 4, 0, "open").wall());
            specs.push(sp("officeWaterCooler", 4 * m, front - 1, "center"));
            specs.push(sp("bananaPlant", -4 * m, front - 1, "center"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        // ---- greenhouse ----
        "glasshouse" => {
            for bed_side in [-3, 0, 3] {
                specs.push(sp("raisedPlanter", bed_side, 0, "open").wall());
            }
            specs.push(sp("raisedPlanter", -2, 3, "open"));
            specs.push(sp("raisedPlanter", 2, 3, "open"));
            specs.push(sp("mistingPost", 0, 3, "open"));
            specs.push(sp("greenhouseBench", -4 * m, front - 1, "center"));
            specs.push(sp("overgrowthPatch", 3 * m, front - 1, "open").loose());
            specs.push(sp("hangingVines", -2, mid, "open").loose());
            specs.push(sp("hangingVines", 2, mid, "open").loose());
        }
        "palm_house" => {
            specs.push(sp("pottedPalm", -3, 1, "open"));
            specs.push(sp("pottedPalm", 3, 1, "open"));
            specs.push(sp("bananaPlant", 0, 0, "open").wall());
            specs.push(sp("overgrowthPatch", 0, 2, "open").loose());
            specs.push(sp("mistingPost", -1, 3, "open"));
            specs.push(sp("mistingPost", 1, 3, "open"));
            specs.push(sp("greenhouseBench", -3, 4, "center"));
            specs.push(sp("greenhouseBench", 3, 4, "center"));
            specs.push(sp("hangingVines", 0, mid, "open").loose());
        }
        "cactus_room" => {
            specs.push(sp("saguaroCactus", -3 * m, 0, "open"));
            specs.push(sp("saguaroCactus", 1 * m, 1, "open"));
            specs.push(sp("saguaroCactus", 4 * m, 0, "open"));
            specs.push(sp("cactusCluster", -1 * m, 0, "open"));
            specs.push(sp("cactusCluster", 3 * m, 2, "open"));
            specs.push(sp("cactusCluster", -4 * m, 3, "open"));
            specs.push(sp("greenhouseBench", 2 * m, 4, "back"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "potting_shed" => {
            specs.push(sp("pottingBench", -2, 0, "open").wall());
            specs.push(sp("pottingBench", 2, 0, "open").wall());
            specs.push(sp("seedlingRack", -3 * m, 3, "center"));
            specs.push(sp("compostBins", 3 * m, 3, "center"));
            specs.push(sp("wateringCart", -4 * m, front - 1, "center"));
            specs.push(sp("supplyCrates", 4 * m, front - 1, "center"));
            specs.push(sp("hangingVines", 0, mid, "open").loose());
            specs.push(sp("ceilingLight", 0, 1, "open").loose());
        }
        "irrigation_room" => {
            specs.push(sp("irrigationManifold", -2 * m, 0, "open").wall());
            specs.push(sp("irrigationManifold", 2 * m, 0, "open").wall());
            specs.push(sp("mistingPost", 0, 1, "open"));
            specs.push(sp("seedlingRack", -3 * m, 3, "center"));
            specs.push(sp("wateringCart", 3 * m, 3, "center"));
            specs.push(sp("overgrowthPatch", 0, 3, "open").loose());
            specs.push(sp("hazardSign", 0, 0, "open").mount().loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "overgrown_room" => {
            specs.push(sp("pottedPalm", -3 * m, 1, "open"));
            specs.push(sp("bananaPlant", 3 * m, 0, "open").wall());
            specs.push(sp("raisedPlanter", 1 * m, 0, "open").wall());
            specs.push(sp("compostBins", 3 * m, 3, "center"));
            for (px, py) in [(-2, 3), (1, 2), (3, front - 1), (-3, front - 1)] {
                specs.push(sp("overgrowthPatch", px * m, py, "open").loose());
            }
            specs.push(sp("hangingVines", -1, mid, "open").loose());
            specs.push(sp("hangingVines", 2, 1, "open").loose());
        }
        "dome_atrium" => {
            let centre = mid;
            specs.push(sp("domeFountain", 0, centre, "open"));
            for (bx, _) in [(-3, 0), (3, 0)] {
                specs.push(sp("greenhouseBench", bx, centre, "center"));
            }
            specs.push(sp("greenhouseBench", 0, centre - 3, "open"));
            for (px, py) in [(-6, 3), (6, 3), (-6, front - 4), (6, front - 4)] {
                specs.push(sp("pottedPalm", px, py, "open"));
            }
            for bed_side in [-6, -2, 2, 6] {
                specs.push(sp("raisedPlanter", bed_side, 0, "open").wall());
            }
            for side_index in 0..2 {
                let edge = if side_index == 0 { -half } else { half };
                for plant_depth in (3..front - 2).step_by(4) {
                    let ty = if plant_depth % 8 == 3 { "bananaPlant" } else { "cactusCluster" };
                    specs.push(sp(ty, edge, plant_depth, "center"));
                }
            }
            for (px, py) in [(-3, centre - 3), (3, centre + 3), (-3, centre + 3), (3, centre - 3)] {
                specs.push(sp("mistingPost", px, py, "open"));
            }
            for (px, py) in [(-5, centre), (5, centre), (0, 3), (0, front - 3)] {
                specs.push(sp("hangingVines", px, py, "open").loose());
            }
            for (px, py) in [(-7, centre + 1), (7, centre - 1), (-2, front - 2), (4, 2)] {
                specs.push(sp("overgrowthPatch", px, py, "open").loose());
            }
        }
        // ---- mall ----
        "donut_shop" => {
            specs.push(sp("donutFryer", -6, 0, "open").wall());
            specs.push(sp("bakeryRack", -3, 0, "open").wall());
            specs.push(sp("normalKitchenFridge", 3, 0, "open").wall());
            specs.push(sp("shopShelf", 6, 0, "open").wall());
            specs.push(sp("donutSign", 0, 0, "open").mount().loose().my(1.7));
            specs.push(sp("menuBoard", -3, 0, "open").mount().loose().my(2.05));
            specs.push(sp("displayCase", -5, 2, "open"));
            specs.push(sp("displayCase", -2, 2, "open"));
            specs.push(sp("checkoutCounter", 3, 2, "open"));
            specs.push(sp("coffeeStation", 6, 2, "open"));
            specs.push(sp("dinerStool", -5, 3, "back"));
            specs.push(sp("dinerStool", -3, 3, "back"));
            specs.push(sp("dinerBooth", -7, 5, "open"));
            specs.push(sp("dinerBooth", 7, 5, "open"));
            for light_side in [-5, 0, 5] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        "cinema" => {
            specs.push(sp("cinemaScreen", 0, 0, "open").mount().loose().my(0.9));
            for row_depth in [1, 2] {
                for seat_side in [-5, 0, 5] {
                    specs.push(sp("cinemaSeats", seat_side, row_depth, "back"));
                }
            }
            for curtain_side in [-7, -4, 4, 7] {
                specs.push(sp("privacyCurtain", curtain_side, 3, "open"));
            }
            specs.push(sp("ticketBooth", -6, 5, "center"));
            specs.push(sp("concessionCounter", 6, 5, "center"));
            specs.push(sp("ceilingLight", -3, front - 1, "open").loose());
            specs.push(sp("ceilingLight", 3, front - 1, "open").loose());
        }
        "bakery" => {
            specs.push(sp("bakeryRack", -7, 0, "open").wall());
            specs.push(sp("bakeryRack", -5, 0, "open").wall());
            specs.push(sp("normalKitchenStove", -2, 0, "open").wall());
            specs.push(sp("shopShelf", 4, 0, "open").wall());
            specs.push(sp("shopShelf", 7, 0, "open").wall());
            specs.push(sp("menuBoard", 0, 0, "open").mount().loose().my(1.9));
            specs.push(sp("displayCase", -5, 2, "open"));
            specs.push(sp("displayCase", -2, 2, "open"));
            specs.push(sp("checkoutCounter", 4, 2, "open"));
            specs.push(sp("mallBench", -6, front - 1, "open"));
            specs.push(sp("ceilingLight", -3, mid, "open").loose());
            specs.push(sp("ceilingLight", 3, mid, "open").loose());
        }
        "laundromat" => {
            for machine_side in -8i32..9 {
                if machine_side.abs() <= 1 {
                    continue;
                }
                let ty = if machine_side % 3 == 0 { "dryerStack" } else { "washingMachine" };
                specs.push(sp(ty, machine_side, 0, "open").wall());
            }
            for machine_side in -5i32..6 {
                if machine_side.abs() <= 1 {
                    continue;
                }
                specs.push(sp("washingMachine", machine_side, 3, "open"));
            }
            specs.push(sp("mallBench", -6, front - 1, "open"));
            specs.push(sp("vendingMachine", 8, front - 1, "center"));
            specs.push(sp("dinerStool", 6, front - 1, "open"));
            for light_side in [-5, 0, 5] {
                specs.push(sp("ceilingLight", light_side, 2, "open").loose());
            }
        }
        "drugstore" => {
            for shelf_side in [-6, -3, 0, 3, 6] {
                specs.push(sp("pharmacyShelf", shelf_side, 0, "open").wall());
            }
            for aisle_side in [-6, -3, 3] {
                specs.push(sp("shopShelf", aisle_side, 3, "open"));
            }
            specs.push(sp("checkoutCounter", 6, front - 1, "center"));
            specs.push(sp("shopFridge", 8, 2, "center"));
            specs.push(sp("ceilingLight", -3, mid, "open").loose());
            specs.push(sp("ceilingLight", 3, mid, "open").loose());
        }
        "corner_store" => {
            for fridge_side in [-7, -6, -5, -4] {
                specs.push(sp("shopFridge", fridge_side, 0, "open").wall());
            }
            for shelf_side in [1, 4, 7] {
                specs.push(sp("shopShelf", shelf_side, 0, "open").wall());
            }
            for aisle_side in [-5, -2, 2, 5] {
                specs.push(sp("shopShelf", aisle_side, 3, "open"));
            }
            specs.push(sp("checkoutCounter", 6, front - 1, "center"));
            specs.push(sp("vendingMachine", -8, front - 1, "center"));
            specs.push(sp("ceilingLight", -3, mid, "open").loose());
            specs.push(sp("ceilingLight", 3, mid, "open").loose());
        }
        "electronics" => {
            for tv_side in [-6, -3, 3, 6] {
                specs.push(sp("tvWall", tv_side, 0, "open").wall());
            }
            specs.push(sp("arcadeCabinet", -1, 0, "open").wall());
            specs.push(sp("arcadeCabinetCyanMagenta", 1, 0, "open").wall());
            specs.push(sp("displayCase", -5, 3, "open"));
            specs.push(sp("displayCase", 5, 3, "open"));
            specs.push(sp("checkoutCounter", 6, front - 1, "center"));
            specs.push(sp("ceilingLight", -3, mid, "open").loose());
            specs.push(sp("ceilingLight", 3, mid, "open").loose());
        }
        "night_diner" => {
            for booth_side in [-6, -3, 3, 6] {
                specs.push(sp("dinerBooth", booth_side, 0, "open").wall());
            }
            specs.push(sp("coffeeStation", 0, 0, "open").wall());
            specs.push(sp("menuBoard", 0, 0, "open").mount().loose().my(1.9));
            specs.push(sp("checkoutCounter", -4, 4, "open"));
            specs.push(sp("checkoutCounter", 4, 4, "open"));
            for stool_side in [-5, -3, 3, 5] {
                specs.push(sp("dinerStool", stool_side, 5, "back"));
            }
            specs.push(sp("ceilingLight", -3, mid, "open").loose());
            specs.push(sp("ceilingLight", 3, mid, "open").loose());
        }
        "florist" => {
            for stand_side in [-6, -3, 3, 6] {
                specs.push(sp("flowerStand", stand_side, 0, "open").wall());
            }
            specs.push(sp("bananaPlant", 0, 0, "open").wall());
            specs.push(sp("flowerStand", -4, 3, "open"));
            specs.push(sp("flowerStand", 4, 3, "open"));
            specs.push(sp("checkoutCounter", 6, front - 1, "center"));
            specs.push(sp("mallPlanter", -7, front - 1, "open"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "barber" => {
            for station_side in [-6, -3, 3, 6] {
                specs.push(sp("barberStation", station_side, 0, "open").wall());
                specs.push(sp("barberChair", station_side, 2, "back"));
            }
            specs.push(sp("mallBench", -5, front - 1, "open"));
            specs.push(sp("checkoutCounter", 6, front - 1, "center"));
            specs.push(sp("ceilingLight", -3, mid, "open").loose());
            specs.push(sp("ceilingLight", 3, mid, "open").loose());
        }
        "pawn_shop" => {
            for shelf_side in [-6, -3, 3, 6] {
                specs.push(sp("shopShelf", shelf_side, 0, "open").wall());
            }
            specs.push(sp("tvWall", 0, 0, "open").wall());
            specs.push(sp("displayCase", -4, 3, "open"));
            specs.push(sp("displayCase", 4, 3, "open"));
            specs.push(sp("checkoutCounter", -7, front - 1, "center"));
            specs.push(sp("clothesRack", 7, front - 1, "center"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "stationery" => {
            for shelf_side in [-6, -3, 3, 6] {
                specs.push(sp("shopShelf", shelf_side, 0, "open").wall());
            }
            specs.push(sp("normalBookcase", 0, 0, "open").wall());
            specs.push(sp("shopShelf", -5, 3, "open"));
            specs.push(sp("shopShelf", 5, 3, "open"));
            specs.push(sp("magazineRack", -1, 3, "open"));
            specs.push(sp("checkoutCounter", 6, front - 1, "center"));
            specs.push(sp("mallBench", -6, front - 1, "open"));
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "food_court" => {
            specs.push(sp("displayCase", -6, 0, "open").wall());
            specs.push(sp("coffeeStation", -3, 0, "open").wall());
            specs.push(sp("concessionCounter", 3, 0, "open").wall());
            specs.push(sp("donutFryer", 6, 0, "open").wall());
            specs.push(sp("menuBoard", -3, 0, "open").mount().loose().my(1.9));
            specs.push(sp("menuBoard", 3, 0, "open").mount().loose().my(1.9));
            for (tx, ty) in [(-6, 3), (-2, 3), (2, 3), (6, 3)] {
                specs.push(sp("foodCourtTable", tx, ty, "open"));
            }
            specs.push(sp("mallPlanter", -7, front - 1, "open"));
            specs.push(sp("mallPlanter", 7, front - 1, "open"));
            for light_side in [-6, -2, 2, 6] {
                specs.push(sp("ceilingLight", light_side, mid, "open").loose());
            }
        }
        _ => {}
    }
    // 4815-4822: the repeated templates are laid twice, at -5 and +5 (the originals are replaced)
    if half >= 9 && REGION_REPEAT_TEMPLATES.contains(&template) {
        let mut repeated: Vec<Spec> = Vec::new();
        for shift in [-5, 5] {
            for original in &specs {
                let mut copy = original.clone();
                copy.side += shift;
                repeated.push(copy);
            }
        }
        specs = repeated;
    }
    specs
}

/// Godot `_CODE_MODEL_PATHS` keys (game/config/prop_model_catalog.gd lines 27-387): `has_model` is true for these
/// (every entry has model paths). The model paths themselves are not needed for placement.
pub const MODEL_KEYS: &[&str] = &[
    "mansionMarbleColumn",
    "mansionPorcelainVase",
    "ivStand",
    "medicalCart",
    "readingTable",
    "libraryGlobe",
    "storageBarrel",
    "storageShelving",
    "receptionDesk",
    "pottedPlant",
    "standingMirror",
    "arcadeCabinet",
    "arcadeCabinetCyanMagenta",
    "filingCabinet",
    "officeDesk",
    "officeChair",
    "officeCrtSecurityConsole",
    "officeCubiclePartition",
    "officePrinter",
    "officeWaterCooler",
    "officePhotocopier",
    "officeShredder",
    "officeFileBoxes",
    "officeCornerWorkstation",
    "arcadeBasketballHoop",
    "arcadeCarpetCircle",
    "arcadeCarpetRunner",
    "arcadeTokenChanger",
    "backstageRepairStand",
    "breakerCabinet",
    "brokenAquarium",
    "carnivalCarousel",
    "flowerBlue",
    "flowerRose",
    "flowerSun",
    "flowerViolet",
    "gardenPlanterBench",
    "giantTree",
    "grassFern",
    "locker",
    "maintenanceCleaningCart",
    "maintenanceToolCabinet",
    "mascotStatue",
    "prizeClawMachine",
    "prizeCounter",
    "snackKiosk",
    "stagePlatform",
    "stageSpeaker",
    "utilityCart",
    "floorLamp",
    "ceilingLight",
    "ceilingFan",
    "tableLamp",
    "loungeCouch",
    "loungeCouchWide",
    "privacyCurtain",
    "serverRack",
    "supplyCrates",
    "trashBin",
    "vendingMachine",
    "woodenChair",
    "woodenChairCushion",
    "woodenTable",
    "restaurantPartyTable",
    "woodenDresserSideboard",
    "retroComputerCompact",
    "retroComputerTower",
    "retroComputerDesktop",
    "retroComputerTerminal",
    "diningTable",
    "diningChair",
    "diningChairCushion",
    "diningChandelier",
    "diningSideboard",
    "flowerCenterpiece",
    "framedWallPortrait",
    "normalPictureFrameEmpty",
    "normalPaintingFrameEmpty",
    "luxuryPictureFrameEmpty",
    "luxuryPaintingFrameEmpty",
    "gothicPictureFrameEmpty",
    "gothicPaintingFrameEmpty",
    "normalRoomDoor",
    "luxuryRoomDoor",
    "gothicRoomDoor",
    "normalDoubleRoomDoor",
    "luxuryDoubleRoomDoor",
    "gothicDoubleRoomDoor",
    "heavyMansionDrapes",
    "mansionAreaRug",
    "corridorCarpet",
    "hotelCarpet",
    "apartmentCarpet",
    "beachMat",
    "mansionBookshelf",
    "mansionFireplace",
    "magazineRack",
    "tableCandelabra",
    "wallSconce",
    "bathroomStall",
    "bathroomSink",
    "bathroomMirror",
    "bathroomVanity",
    "publicRestroomToilet",
    "publicMensUrinal",
    "publicSinkBank",
    "publicHandDryer",
    "publicSoapDispenser",
    "publicPaperTowelDispenser",
    "publicBabyChangeStation",
    "publicSanitaryBin",
    "publicMensRoomSign",
    "publicWomensRoomSign",
    "publicMensShowerStall",
    "publicWomensShowerStall",
    "publicShowerBench",
    "houseBathtub",
    "houseToilet",
    "houseShowerEnclosure",
    "houseMedicineCabinet",
    "houseTowelRack",
    "houseBathMat",
    "houseBathroomShelf",
    "grandStaircase",
    "grandFoyerStaircase",
    "singleBed",
    "doubleBed",
    "bunkBed",
    "bedroomNightstand",
    "bedroomWardrobe",
    "bedroomDresser",
    "bedroomVanity",
    "vanityStool",
    "bedsideLamp",
    "toyChest",
    "laundryHamper",
    "bedroomRug",
    "bedroomWallClock",
    "bedroomBench",
    "gothicDoubleBed",
    "gothicNightstand",
    "gothicWardrobe",
    "gothicDresser",
    "gothicVanity",
    "gothicVanityStool",
    "gothicBedroomBench",
    "gothicBedroomRug",
    "gothicBedsideCandelabra",
    "gothicHighbackChair",
    "mansionConsoleTable",
    "mansionGrandfatherClock",
    "mansionCoatRack",
    "mansionTeaCart",
    "mansionRockingChair",
    "mansionRoomDivider",
    "gothicWritingDesk",
    "gothicBookcase",
    "gothicPedestalTable",
    "gothicFloorCandelabra",
    "gothicCoffinChest",
    "normalKitchenCounter",
    "normalKitchenSink",
    "normalKitchenStove",
    "normalKitchenFridge",
    "normalKitchenWallCabinet",
    "normalKitchenIsland",
    "luxuryKitchenCounter",
    "luxuryKitchenSink",
    "luxuryKitchenStove",
    "luxuryKitchenFridge",
    "luxuryKitchenWallCabinet",
    "luxuryKitchenIsland",
    "gothicKitchenCounter",
    "gothicKitchenSink",
    "gothicKitchenStove",
    "gothicKitchenFridge",
    "gothicKitchenWallCabinet",
    "gothicKitchenIsland",
    "normalBookcase",
    "normalCoffeeTable",
    "normalTvConsole",
    "normalEntryBench",
    "mansionDisplayCabinet",
    "mansionChaiseLounge",
    "mansionUprightPiano",
    "mansionBustPedestal",
    "sandCastle",
    "beachUmbrella",
    "tikiBar",
    "beachChair",
    "surfboardRack",
    "palmTree",
    "frozenBeachHut",
    "frozenTidelinePool",
    "carnivalTicketBooth",
    "popcornCart",
    "ringTossStall",
    "strengthTester",
    "circusPedestal",
    "shootingGallery",
    "whackAMole",
    "duckPond",
    "carouselCenterpiece",
    "lavaVent",
    "emberBrazier",
    "basaltPillar",
    "basaltRubble",
    "magmaCrystalCluster",
    "infernoAltar",
    "infernoChainLantern",
    "lavaPipeManifold",
    "charredSupplyCrate",
    "prisonCellGate",
    "sewerLadder",
    "sewerLadderTop",
    "sewerManhole",
    "sewerFloorGrate",
    "sewerValveWheel",
    "sewerPump",
    "sewerPipeRun",
    "sewerOutflowPipe",
    "sewerSluiceGate",
    "sewerOverflowGrate",
    "sewerLamp",
    "sewerBridge",
    "sewerGrateDeck",
    "prisonBunk",
    "prisonCafeteriaTable",
    "prisonServingCounter",
    "prisonTrayReturn",
    "prisonKitchenRack",
    "dungeonCage",
    "restraintChair",
    "hospitalBed",
    "hospitalCurtain",
    "ivDripStand",
    "vitalsMonitor",
    "nursesStation",
    "waitingChairs",
    "operatingTable",
    "surgicalLight",
    "instrumentTrolley",
    "morgueCabinet",
    "autopsyTable",
    "pharmacyShelf",
    "xrayMachine",
    "xrayLightbox",
    "medicalSupplyRack",
    "wheelchair",
    "hospitalWardSign",
    "labBench",
    "fumeHood",
    "specimenTank",
    "containmentCell",
    "labCoolingUnit",
    "controlConsole",
    "monitorWall",
    "divePod",
    "divePodFilm",
    "divePodLit",
    "diveHeadsetStand",
    "diveHeadsetShelf",
    "diveConsole",
    "deconShower",
    "coldStorageUnit",
    "labWhiteboard",
    "hazardSign",
    "labGlassPartition",
    "labStool",
    "labMicroscope",
    "gasCylinderRack",
    "biohazardBin",
    "raisedPlanter",
    "pottingBench",
    "pottedPalm",
    "bananaPlant",
    "saguaroCactus",
    "cactusCluster",
    "seedlingRack",
    "irrigationManifold",
    "mistingPost",
    "overgrowthPatch",
    "hangingVines",
    "domeFountain",
    "wateringCart",
    "greenhouseBench",
    "compostBins",
    "shopShelf",
    "shopFridge",
    "checkoutCounter",
    "displayCase",
    "coffeeStation",
    "donutFryer",
    "bakeryRack",
    "dinerStool",
    "dinerBooth",
    "menuBoard",
    "donutSign",
    "washingMachine",
    "dryerStack",
    "flowerStand",
    "barberChair",
    "barberStation",
    "clothesRack",
    "tvWall",
    "cinemaSeats",
    "cinemaScreen",
    "ticketBooth",
    "concessionCounter",
    "foodCourtTable",
    "mallPlanter",
    "mallBench",
    "micStand",
    "podiumMic",
    "deskMic",
    "drumKit",
    "electricGuitarStand",
    "acousticGuitarStand",
    "bassGuitarStand",
    "keyboardStand",
    "uprightPianoMusic",
    "ampStack",
    "comboAmp",
    "stageMonitor",
    "paSpeakerPole",
    "mixingDesk",
    "studioMonitors",
    "rackGear",
    "musicStand",
    "cableCoil",
    "cableRun",
    "stageLightRig",
    "acousticPanel",
    "guitarWallHanger",
    "instrumentCases",
    "onAirSign",
    "bedroomWardrobeMansion",
];

// ----------------------------------------------------------------------------------------------------------
// music rooms (`MusicWing.room_specs`, music_wing.gd:96-172) and the data recipe of the music store
// (`RoomRecipes`, data/rooms/music_store.json, room_recipes.gd `specs`)

/// `MusicWing.TEMPLATES` (music_wing.gd:16): room types that are furnished by the music wing's recipes.
const MUSIC_TEMPLATES: &[&str] = &["music_stage", "practice_room", "instrument_storage", "recording_booth", "carnival_stage"];

/// `MusicWing.MIXING_DESK_TOP` (music_wing.gd:34).
const MIXING_DESK_TOP: f64 = 1.23;

/// `MusicWing.room_specs(template, half, depth_span, mirror)` (music_wing.gd:96-172). No RNG.
fn music_wing_specs(template: &str, half: i32, depth_span: i32, mirror: i32) -> Vec<Spec> {
    let front = depth_span - 3;
    let mid = front / 2;
    let m = mirror;
    let mut specs: Vec<Spec> = Vec::new();
    match template {
        "practice_room" => {
            for panel_side in (-half..half + 1).step_by(2) {
                specs.push(sp("acousticPanel", panel_side, 0, "open").mount().loose());
            }
            specs.push(sp("drumKit", -2 * m, 1, "open"));
            specs.push(sp("ampStack", 2 * m, 0, "open").wall());
            specs.push(sp("electricGuitarStand", half.min(4) * m, 1, "center"));
            specs.push(sp("bassGuitarStand", -half.min(4) * m, front - 2, "center"));
            specs.push(sp("micStand", 1 * m, front - 3, "back"));
            specs.push(sp("musicStand", 3 * m, front - 3, "back"));
            specs.push(sp("comboAmp", half.min(4) * m, front - 2, "center"));
            specs.push(sp("cableCoil", 1 * m, 2, "open").loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "instrument_storage" => {
            for hanger_side in [-2, 2] {
                specs.push(sp("guitarWallHanger", hanger_side, 0, "open").mount().loose());
            }
            specs.push(sp("instrumentCases", -half.min(4) * m, 0, "open").wall());
            specs.push(sp("storageShelving", half.min(4) * m, 0, "open").wall());
            specs.push(sp("acousticGuitarStand", -2 * m, 2, "open"));
            specs.push(sp("bassGuitarStand", 2 * m, 2, "open"));
            specs.push(sp("keyboardStand", -2 * m, front - 2, "center"));
            specs.push(sp("instrumentCases", half.min(4) * m, front - 2, "center"));
            specs.push(sp("comboAmp", 3 * m, front - 3, "center"));
            specs.push(sp("cableCoil", 0, 2, "open").loose());
            specs.push(sp("cableCoil", -3 * m, front - 4, "open").loose());
            specs.push(sp("ceilingLight", 0, mid, "open").loose());
        }
        "recording_booth" => {
            // a glass wall across the room with a gap in the middle: the vocal booth behind it, the control room in front
            let glass = 2.max(mid);
            for glass_side in (-half..half + 1).step_by(2) {
                if glass_side.abs() >= 2 {
                    specs.push(sp("labGlassPartition", glass_side, glass, "open"));
                }
            }
            specs.push(sp("micStand", -1 * m, 1, "open"));
            specs.push(sp("musicStand", 1 * m, 1, "open"));
            specs.push(sp("onAirSign", 2 * m, 0, "open").mount().loose());
            for panel_side in [-half.min(4), -2] {
                specs.push(sp("acousticPanel", panel_side * m, 0, "open").mount().loose());
            }
            specs.push(sp("mixingDesk", -2 * m, glass + 2, "back"));
            specs.push(sp("studioMonitors", -2 * m, glass + 2, "back").loose().my(MIXING_DESK_TOP));
            specs.push(sp("officeChair", -2 * m, glass + 3, "back"));
            specs.push(sp("rackGear", half.min(4) * m, glass + 2, "center"));
            specs.push(sp("ceilingLight", 0, glass + 2, "open").loose());
        }
        "music_stage" | "carnival_stage" => {
            // the band at the back under the light truss, monitors facing them at the stage edge
            let edge = 6.min(front - 2);
            specs.push(sp("stageLightRig", 0, 0, "open").wall());
            specs.push(sp("drumKit", 0, 2.min(edge - 2), "open"));
            for amp_side in [-4, 4] {
                specs.push(sp("ampStack", amp_side, 0, "open").wall());
            }
            for mic_side in [-2, 2] {
                specs.push(sp("micStand", mic_side, edge - 2, "open"));
            }
            specs.push(sp("electricGuitarStand", -4, edge - 1, "open"));
            specs.push(sp("bassGuitarStand", 4, edge - 1, "open"));
            if half >= 6 {
                specs.push(sp("keyboardStand", -6, 2, "center"));
                for pa_side in [-half, half] {
                    specs.push(sp("paSpeakerPole", pa_side, 1, "open"));
                }
            }
            for monitor_side in [-2, 0, 2] {
                specs.push(sp("stageMonitor", monitor_side, edge, "back"));
            }
            specs.push(sp("cableRun", 0, edge + 1, "open").loose());
            specs.push(sp("cableCoil", 3, edge - 1, "open").loose());
            if template == "music_stage" && front >= edge + 6 {
                for row_depth in (edge + 3..front - 2).step_by(3) {
                    for chair_side in (-half + 2..half - 1).step_by(5) {
                        specs.push(sp("waitingChairs", chair_side, row_depth, "back"));
                    }
                }
                specs.push(sp("mixingDesk", 0, front - 2, "back"));
                specs.push(sp("studioMonitors", 0, front - 2, "back").loose().my(MIXING_DESK_TOP));
                specs.push(sp("rackGear", 3, front - 2, "back"));
            }
            for light_depth in (mid..front).step_by(5) {
                specs.push(sp("ceilingLight", 0, light_depth, "open").loose());
            }
        }
        _ => {}
    }
    specs
}

/// Whether `RoomRecipes.has` holds for a room type. Only the shipped recipe (data/rooms/music_store.json) is
/// ported: the built-in data of the game, with no content packs.
fn has_recipe(room_type: &str) -> bool {
    room_type == "music_store"
}

/// `RoomRecipes.specs(room_type, side_span, depth_span, rng, region_types)` (room_recipes.gd) for the shipped
/// recipe. One draw for the mirror, then the items in order (music_store.json has no `chance` or `choose`).
fn recipe_specs(room_type: &str, side_span: i32, depth_span: i32, rng: &mut Rng) -> Vec<Spec> {
    if room_type != "music_store" {
        return Vec::new();
    }
    // the sizes of the recipe: back 0, front depth - 3, mid depth / 2, half side / 2
    let front = depth_span - 3;
    let mid = depth_span / 2;
    let half = side_span / 2;
    let mirror: i32 = if rng.next_float() < 0.5 { -1 } else { 1 };
    let mut out: Vec<Spec> = Vec::new();
    // guitarWallHanger: side ["-half+3", "half-2", 3], depth "back", wall_mount, solid false, mirror false
    let sides: Vec<i32> = ((-half + 3)..(half - 2)).step_by(3).collect();
    for &side in &sides {
        out.push(sp("guitarWallHanger", side, 0, "open").mount().loose());
    }
    // a list of stands over the same sides, depth 2, mirror false
    let stands = ["electricGuitarStand", "acousticGuitarStand", "bassGuitarStand"];
    for (index, &side) in sides.iter().enumerate() {
        out.push(sp(stands[index % stands.len()], side, 2, "open"));
    }
    out.push(sp("keyboardStand", -4 * mirror, front - 2, "center"));
    out.push(sp("ampStack", 5 * mirror, front - 2, "center"));
    out.push(sp("comboAmp", 3 * mirror, front - 2, "center"));
    out.push(sp("checkoutCounter", (-half + 3) * mirror, front - 1, "center"));
    out.push(sp("ceilingLight", 0, mid - 1, "open").loose());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_and_wrapping_follow_godot() {
        assert_eq!(snap_quarter(0.0), 0.0);
        assert!((snap_quarter(-PI) + PI).abs() < 1e-12);
        assert!((wrapf(3.0 * PI / 2.0, -PI, PI) + PI / 2.0).abs() < 1e-9);
    }

    #[test]
    fn interaction_kinds_match_the_seat_and_bed_words() {
        assert_eq!(interaction_for("woodenChair"), "sit");
        assert_eq!(interaction_for("singleBed"), "lay");
        assert_eq!(interaction_for("bedroomNightstand"), "");
        assert_eq!(interaction_for("standingMirror"), "");
        assert_eq!(interaction_for("bathroomMirror"), "mirror");
    }

    /// A bed's generator collider stays inside its room. A wall-backed bed is pushed flush to its wall by
    /// `placement_offset`, and `collider_rects` (Godot's `_collider_rects`) includes that offset. The live
    /// footprint is checked by `live_beds_stay_inside_their_room`.
    #[test]
    fn beds_stay_inside_their_room() {
        const BEDS: [&str; 4] = ["singleBed", "doubleBed", "bunkBed", "gothicDoubleBed"];
        const EPS: f32 = 1e-3;
        let mut beds = 0usize;
        for world in ["1426541270", "4157433391", "2899547009"] {
            for cx in -3..=3 {
                for cz in -3..=3 {
                    let Some(layout) = crate::world::route::layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                        continue;
                    };
                    let Some(props) = layout.region_props.as_ref() else {
                        continue;
                    };
                    for p in props.iter().filter(|p| BEDS.contains(&p.prop_type)) {
                        beds += 1;
                        let room = layout.rooms.iter().find(|r| r.id == p.room_id).expect("a bed has its room");
                        let (x0, y0) = (room.rect.x as f32, room.rect.y as f32);
                        let (x1, y1) = ((room.rect.x + room.rect.w) as f32, (room.rect.y + room.rect.h) as f32);
                        let inside = |x: f32, y: f32| x >= x0 - EPS && x <= x1 + EPS && y >= y0 - EPS && y <= y1 + EPS;
                        let label = format!("{world} {cx},{cz} {} {}", p.id, p.prop_type);
                        // the generator's collider boxes, with the placement offset
                        for (centre, half, yaw) in collider_rects(p) {
                            let (ax, ay) = (axis_x(yaw), axis_y(yaw));
                            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                                let x = centre[0] as f64 + sx * half[0] as f64 * ax[0] + sy * half[1] as f64 * ay[0];
                                let y = centre[1] as f64 + sx * half[0] as f64 * ax[1] + sy * half[1] as f64 * ay[1];
                                assert!(inside(x as f32, y as f32), "{label}: collider corner ({x:.3}, {y:.3}) leaves the room");
                            }
                        }
                    }
                }
            }
        }
        assert!(beds > 0, "the sampled chunks place no beds");
    }

    /// The same bed rule on the live footprints the player and the drawn model use (`live::chunk` ->
    /// `facility::Prop`): centre `cell + .5 + offset`, half extents of the rotated collider, inside the bed's room.
    #[test]
    fn live_beds_stay_inside_their_room() {
        const BEDS: [&str; 4] = ["singleBed", "doubleBed", "bunkBed", "gothicDoubleBed"];
        const EPS: f32 = 1e-3;
        let mut beds = 0usize;
        for world in ["1426541270", "4157433391", "2899547009"] {
            for cx in -3..=3 {
                for cz in -3..=3 {
                    let key = crate::world::infinite::ChunkKey { cx, cy: cz, level: 0 };
                    let Some((fac, _)) = crate::world::route::live::chunk(world, key, catalog()) else {
                        continue;
                    };
                    for p in fac.props.iter().filter(|p| BEDS.contains(&p.ptype.as_str()) && p.door.is_none()) {
                        beds += 1;
                        let room = &fac.rooms[p.room];
                        let (x0, y0) = (room.x as f32, room.y as f32);
                        let (x1, y1) = ((room.x + room.w) as f32, (room.y + room.h) as f32);
                        let cxf = p.cell.x as f32 + 0.5 + p.offset.0;
                        let cyf = p.cell.y as f32 + 0.5 + p.offset.1;
                        let (hx, hy) = p.half;
                        for (x, y) in [(cxf - hx, cyf - hy), (cxf + hx, cyf + hy)] {
                            assert!(
                                x >= x0 - EPS && x <= x1 + EPS && y >= y0 - EPS && y <= y1 + EPS,
                                "{world} {cx},{cz} {}: footprint corner ({x:.3}, {y:.3}) leaves the room",
                                p.id
                            );
                        }
                    }
                }
            }
        }
        assert!(beds > 0, "the sampled live chunks place no beds");
    }
}
