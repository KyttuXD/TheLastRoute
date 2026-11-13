//! The prop facing pass: which way a level-0 chunk's props face, the 45 degree turns, and (through `hall_props`) the
//! hall dressing. It is a Rust-only layer applied to a chunk AFTER the Godot-ported generator (`route::layout`, whose
//! raw output the golden tests check) has been turned into the live chunk (`route::live::from_layout`), so the
//! generator and its goldens are untouched. `world::infinite::chunk` calls `dress` once when a chunk is built; the
//! result is cached with the chunk, so nothing here runs per frame. docs/PROP_FACING.md has the rules in words.
//!
//! North is the map's -z (cell y - 1, the top of the minimap and the admin "north"): rotation 0. A prop's facing is
//! its gameplay forward, the model's local -z turned by `Prop::yaw`: quarter turn 0 faces north, 1 west, 2 south,
//! 3 east (`FACING`).
//!
//! Every choice is a hash of the world seed text and the prop's absolute cell, so the host and every guest (each
//! builds its chunks from the seed) get the same props. The collider is the same axis-aligned box the drawn prop
//! stands in: a quarter turn swaps its half extents, a 45 degree turn of a non-square box takes the turned box's
//! bounding box, and a square or round one keeps its box.
//!
//! The pass never seals anything: every change must keep its footprint on open floor, clear of every other solid
//! prop, of pickups, lockers and stairs, and of the doors' approach cells, and at the end the chunk is flood-filled
//! with a hunter's body before and after. Should the fill ever differ, the chunk keeps its original props.

use std::collections::HashSet;
use std::sync::OnceLock;

use serde_json::Value;

use super::door_clearance::Placement;
use super::facility::{Cell, Facility, Prop, DIAGONAL};
use super::fixtures::Fixtures;
use super::hall_props;
use super::infinite::{ChunkKey, CHUNK};
use super::route::layout::ChunkLayout;

const TEXT: &str = include_str!("../data/world/prop_facing.json");
const PATH: &str = "data/world/prop_facing.json";

/// The cell step a prop faces for each quarter turn: north (-z), west, south, east.
pub const FACING: [(i32, i32); 4] = [(0, -1), (-1, 0), (0, 1), (1, 0)];

/// The quarter turns in the order ties are settled: north, east, south, west.
pub const ORDER: [usize; 4] = [0, 3, 2, 1];

/// Salts of the hashed choices, so each draw is its own.
const SALT_FREE: i32 = 0x51;
const SALT_SIDE: i32 = 0x52;
const SALT_CORNER: i32 = 0x53;

/// The rules of the facing pass (data/world/prop_facing.json).
#[derive(Clone, Debug)]
pub struct Rules {
    pub enabled: bool,
    pub run_cap: i32,
    pub wall_gap: f32,
    pub square_tolerance: f32,
    /// Lower-case fragments: a type containing one is never touched.
    pub keep: Vec<String>,
    /// Types with no front: never re-faced, but they may take a diagonal.
    pub symmetric: Vec<String>,
    pub diagonal: bool,
    pub free_chance: f32,
    pub corner_chance: f32,
    pub max_corner_shift: f32,
    pub free_types: Vec<String>,
    pub corner_types: Vec<String>,
    pub never_types: Vec<String>,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("{PATH}: {e}"))?;
        if v["schema"].as_str() != Some("lr.prop_facing/1") {
            return Err(format!("{PATH}: `schema` must be lr.prop_facing/1"));
        }
        let d = &v["diagonal"];
        Ok(Rules {
            enabled: flag(PATH, &v, "enabled")?,
            run_cap: number(PATH, &v, "run_cap", 1.0, 64.0)? as i32,
            wall_gap: number(PATH, &v, "wall_gap", 0.0, 0.4)? as f32,
            square_tolerance: number(PATH, &v, "square_tolerance", 0.0, 0.5)? as f32,
            keep: fragments(PATH, &v, "keep_types")?,
            symmetric: fragments(PATH, &v, "symmetric_types")?,
            diagonal: flag(PATH, d, "enabled")?,
            free_chance: number(PATH, d, "free_chance", 0.0, 1.0)? as f32,
            corner_chance: number(PATH, d, "corner_chance", 0.0, 1.0)? as f32,
            max_corner_shift: number(PATH, d, "max_corner_shift", 0.0, 0.5)? as f32,
            free_types: fragments(PATH, d, "free_types")?,
            corner_types: fragments(PATH, d, "corner_types")?,
            never_types: fragments(PATH, d, "never_types")?,
        })
    }
}

/// A number of `at[key]` within `lo..=hi`, else an error naming the file and the key.
pub(super) fn number(path: &str, at: &Value, key: &str, lo: f64, hi: f64) -> Result<f64, String> {
    at[key].as_f64().filter(|n| *n >= lo && *n <= hi).ok_or_else(|| format!("{path}: `{key}` must be a number from {lo} to {hi}"))
}

/// A true/false of `at[key]`, else an error naming the file and the key.
pub(super) fn flag(path: &str, at: &Value, key: &str) -> Result<bool, String> {
    at[key].as_bool().ok_or_else(|| format!("{path}: `{key}` must be true or false"))
}

/// A list of type fragments, lower-cased.
fn fragments(path: &str, at: &Value, key: &str) -> Result<Vec<String>, String> {
    let list = at[key].as_array().ok_or_else(|| format!("{path}: `{key}` must be a list"))?;
    list.iter()
        .map(|x| {
            x.as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase())
                .ok_or_else(|| format!("{path}: `{key}` must hold only non-empty text"))
        })
        .collect()
}

/// Whether the lower-case type contains one of the fragments.
fn has(list: &[String], lower: &str) -> bool {
    list.iter().any(|k| lower.contains(k.as_str()))
}

/// The embedded rules. A refused shipped file is a build error found by the first run, named by file and key.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules::parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

/// A hash of the world seed text and four numbers (FNV-1a, then a finishing mix): the same on every machine.
pub fn hash(seed: &str, parts: [i32; 4]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.bytes().chain(parts.iter().flat_map(|p| p.to_le_bytes())) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

/// A hash as a number in 0..1.
pub fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / 16_777_216.0
}

/// Which props the generator placed on purpose and the pass must respect.
#[derive(Clone, Debug, Default)]
pub struct Locks {
    /// Ids never touched: room doors, wall-mounted props, props standing on a surface, props a room template turned
    /// toward their pair (`oriented`: a seat at its table or screen) or stood against a wall (`backing_wall`), and
    /// the props a hiding fixture belongs to.
    pub fixed: HashSet<String>,
    /// Ids placed by a furnished-room template. They are only ever turned to put their back to a wall beside them:
    /// never to the north default, never diagonally.
    pub template: HashSet<String>,
}

impl Locks {
    /// The locks of a level-0 chunk, read from the generator's own marks on its props.
    pub fn of_layout(lay: &ChunkLayout) -> Locks {
        let mut locks = Locks::default();
        for p in lay.region_props.as_deref().unwrap_or(&[]) {
            let template = p.room_template.is_some();
            let wall_backed = template && p.backing_wall.is_some_and(|w| w.x != 0 || w.y != 0);
            if p.room_door || p.wall_mounted || p.oriented || p.mount_y != 0.0 || wall_backed {
                locks.fixed.insert(p.id.clone());
            }
            if template {
                locks.template.insert(p.id.clone());
            }
        }
        for x in &lay.fixtures {
            if let Some(hide) = &x.hide {
                locks.fixed.insert(hide.prop_id.clone());
            }
        }
        locks
    }
}

/// What a pass did to a chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Props turned to a new quarter turn.
    pub turned: usize,
    /// Props turned 45 degrees.
    pub diagonal: usize,
    /// Hall props added.
    pub hall: usize,
}

/// The centre of a prop on the plan, in cells.
pub fn centre(p: &Prop) -> (f32, f32) {
    (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1)
}

/// How many open floor cells lie in a straight line from `c` along `step`, counted to `cap`.
fn open_run(f: &Facility, c: Cell, step: (i32, i32), cap: i32) -> i32 {
    let mut n = 0;
    while n < cap {
        let (x, y) = (c.x + step.0 * (n + 1), c.y + step.1 * (n + 1));
        if f.is_wall_cell(x, y) || f.is_hole(x, y) {
            break;
        }
        n += 1;
    }
    n
}

/// The cells no turned footprint may newly cover: the start, pickups, lockers and the cell in front of each, vents,
/// coins and every stair cell.
fn reserved_cells(f: &Facility, fix: &Fixtures) -> HashSet<Cell> {
    let mut set: HashSet<Cell> = HashSet::new();
    set.insert(f.start);
    set.extend(f.relays.iter().copied());
    set.extend(f.batteries.iter().copied());
    set.extend(f.pickups.iter().map(|p| p.0));
    for h in &f.hiding {
        set.insert(h.cell);
        set.insert(Cell::new(h.cell.x + h.facing.0, h.cell.y + h.facing.1));
    }
    set.extend(fix.vents.iter().copied());
    set.extend(fix.coins.iter().copied());
    set.extend(fix.extra_hiding.iter().map(|h| h.cell));
    for s in &f.stairs {
        set.insert(s.cell);
        set.insert(s.step_cell());
        set.extend(s.well_cells());
        set.extend(s.carve_cells());
    }
    set
}

/// What the footprint test reads besides the props.
struct Site<'a> {
    f: &'a Facility,
    reserved: &'a HashSet<Cell>,
    placement: &'a Placement,
}

/// Whether prop `i` of `work` may take the footprint `half` at `offset`: on open floor, off every reserved cell it
/// does not already stand on, clear of every other solid prop, and leaving the doors' approach cells to the hunters.
fn fits(site: &Site, work: &[Prop], i: usize, half: (f32, f32), offset: (f32, f32)) -> bool {
    const EDGE: f32 = 0.02;
    let p = &work[i];
    let (cx, cy) = (p.cell.x as f32 + 0.5 + offset.0, p.cell.y as f32 + 0.5 + offset.1);
    let (x0, x1) = ((cx - half.0 + EDGE).floor() as i32, (cx + half.0 - EDGE).floor() as i32);
    let (y0, y1) = ((cy - half.1 + EDGE).floor() as i32, (cy + half.1 - EDGE).floor() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            if site.f.is_wall_cell(x, y) || site.f.is_hole(x, y) {
                return false;
            }
            let c = Cell::new(x, y);
            if c != p.cell && site.reserved.contains(&c) {
                return false;
            }
        }
    }
    for (j, q) in work.iter().enumerate() {
        if j == i || !q.solid || q.door.is_some() {
            continue;
        }
        let (qx, qy) = centre(q);
        if (cx - qx).abs() < half.0 + q.half.0 - EDGE && (cy - qy).abs() < half.1 + q.half.1 - EDGE {
            return false;
        }
    }
    // the door rule tests a footprint round a cell centre: grow the box by the centre's distance from it
    let at = Cell::new(cx.floor() as i32, cy.floor() as i32);
    let grown = (half.0 + (cx - at.x as f32 - 0.5).abs(), half.1 + (cy - at.y as f32 - 0.5).abs());
    site.placement.allows(&p.ptype, at, grown, true)
}

/// The half extents of the bounding box of a `base` box turned 45 degrees.
pub fn diagonal_half(base: (f32, f32)) -> (f32, f32) {
    let e = (base.0 + base.1) * std::f32::consts::FRAC_1_SQRT_2;
    (e, e)
}

/// The facing pass over the first `n` props of `work` (the generator's; the hall props after them are already
/// faced). `f` is read for its walls and holes only: its prop grids still describe the props before the pass.
fn face_props(site: &Site, work: &mut [Prop], n: usize, locks: &Locks, rules: &Rules, seed: &str, k: ChunkKey) -> Outcome {
    let f = site.f;
    let mut out = Outcome::default();
    // where the surface props stand: what carries one is never turned from under it
    let mounted: Vec<(f32, f32)> = work.iter().filter(|p| p.mount_y.is_some()).map(centre).collect();
    for i in 0..n.min(work.len()) {
        let p = &work[i];
        if p.door.is_some() || p.sewer.is_some() || p.crawl_under || p.mount_y.is_some() || !p.solid || p.is_diagonal() {
            continue;
        }
        if locks.fixed.contains(&p.id) {
            continue;
        }
        let lower = p.ptype.to_ascii_lowercase();
        if has(&rules.keep, &lower) {
            continue;
        }
        let (cx, cy) = centre(p);
        if mounted.iter().any(|m| (m.0 - cx).abs() <= p.half.0 && (m.1 - cy).abs() <= p.half.1) {
            continue;
        }
        let template = locks.template.contains(&p.id);
        let symmetric = has(&rules.symmetric, &lower);
        let may_turn_45 = rules.diagonal && !template && !has(&rules.never_types, &lower);
        let c = p.cell;
        let old = p.quarter() as usize;
        let (old_half, old_offset) = (p.half, p.offset);
        // the half extents at quarter turn 0
        let base = if old % 2 == 1 { (old_half.1, old_half.0) } else { old_half };
        let square = (base.0 - base.1).abs() <= rules.square_tolerance;
        let half_for = |q: usize| if q % 2 == 1 { (base.1, base.0) } else { base };
        let mut walls = [false; 4];
        let mut runs = [0; 4];
        for q in 0..4 {
            walls[q] = f.is_wall_cell(c.x + FACING[q].0, c.y + FACING[q].1);
            runs[q] = open_run(f, c, FACING[q], rules.run_cap);
        }
        let wall_count = walls.iter().filter(|w| **w).count();
        let abs = (k.cx * CHUNK + c.x, k.cy * CHUNK + c.y);
        // the back stays `wall_gap` off the wall face; a deeper prop moves out into the room (Godot's depth shift)
        let flush = |q: usize| {
            let h = half_for(q);
            let depth = if FACING[q].0 != 0 { h.0 } else { h.1 };
            let s = (depth - (0.5 - rules.wall_gap)).max(0.0);
            (FACING[q].0 as f32 * s, FACING[q].1 as f32 * s)
        };
        // (rotation, half, offset) to try, best first
        let mut tries: Vec<(i32, (f32, f32), (f32, f32))> = Vec::new();
        if wall_count == 0 {
            if template {
                continue;
            }
            if may_turn_45 && has(&rules.free_types, &lower) && unit(hash(seed, [abs.0, abs.1, k.level, SALT_FREE])) < rules.free_chance {
                // north-west or north-east, by a second draw
                let q = if hash(seed, [abs.0, abs.1, k.level, SALT_SIDE]) & 1 == 0 { 0 } else { 3 };
                let half = if square { old_half } else { diagonal_half(base) };
                tries.push((q + DIAGONAL, half, old_offset));
            }
            if !symmetric && old != 0 {
                tries.push((0, half_for(0), old_offset));
            }
        } else if wall_count < 4 {
            // faces with a wall behind and open floor ahead; with none (a prop in a one-cell passage), any open side
            let mut cands: Vec<usize> = ORDER.iter().copied().filter(|q| walls[(*q + 2) % 4] && !walls[*q]).collect();
            let backed = !cands.is_empty();
            if !backed {
                if template {
                    continue;
                }
                cands = ORDER.iter().copied().filter(|q| !walls[*q]).collect();
            }
            // the longest open run first; a tie keeps the present facing, then north, east, south, west (the sort is stable)
            cands.sort_by_key(|q| (-runs[*q], if *q == old { 0 } else { 1 }));
            let corner = backed && cands.len() == 2 && (cands[0] + cands[1]) % 2 == 1;
            if corner && may_turn_45 && has(&rules.corner_types, &lower) && unit(hash(seed, [abs.0, abs.1, k.level, SALT_CORNER])) < rules.corner_chance {
                // the diagonal lies between quarter turn q and q + 1
                let (a, b) = (cands[0], cands[1]);
                let q = if (a + 1) % 4 == b { a } else { b };
                let next = (q + 1) % 4;
                let reach = (base.0 + base.1) * std::f32::consts::FRAC_1_SQRT_2;
                let s = (reach - (0.5 - rules.wall_gap)).max(0.0);
                if s <= rules.max_corner_shift {
                    let offset = ((FACING[q].0 + FACING[next].0) as f32 * s, (FACING[q].1 + FACING[next].1) as f32 * s);
                    let half = if square { old_half } else { diagonal_half(base) };
                    tries.push((q as i32 + DIAGONAL, half, offset));
                }
            }
            if !symmetric {
                for q in cands {
                    if q == old {
                        break;
                    }
                    tries.push((q as i32, half_for(q), if backed { flush(q) } else { old_offset }));
                }
            }
        }
        for (rotation, half, offset) in tries {
            let same_box = half == old_half && offset == old_offset;
            if same_box || fits(site, work, i, half, offset) {
                let p = &mut work[i];
                p.rotation = rotation;
                p.half = half;
                p.offset = offset;
                if rotation >= DIAGONAL {
                    out.diagonal += 1;
                } else {
                    out.turned += 1;
                }
                break;
            }
        }
    }
    out
}

/// The cells a body of radius `r` can stand on (at the cell centre), labelled by connected group (0 is closed).
pub fn body_groups(f: &Facility, r: f32) -> Vec<u32> {
    let (cols, rows) = (f.cols, f.rows);
    let mut label = vec![0u32; (cols * rows) as usize];
    let mut open = vec![false; (cols * rows) as usize];
    for y in 0..rows {
        for x in 0..cols {
            open[(y * cols + x) as usize] = !f.is_wall_cell(x, y) && f.body_fits(x as f32 + 0.5, y as f32 + 0.5, r);
        }
    }
    let mut next = 0u32;
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for y in 0..rows {
        for x in 0..cols {
            let i = (y * cols + x) as usize;
            if !open[i] || label[i] != 0 {
                continue;
            }
            next += 1;
            label[i] = next;
            stack.push((x, y));
            while let Some((sx, sy)) = stack.pop() {
                for (dx, dy) in FACING {
                    let (nx, ny) = (sx + dx, sy + dy);
                    if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                        continue;
                    }
                    let ni = (ny * cols + nx) as usize;
                    if open[ni] && label[ni] == 0 {
                        label[ni] = next;
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }
    label
}

/// Whether every group of `before` is still one group in `after`: the cells of it that are still open all carry one
/// label. A cell a prop now stands on has simply left its group.
pub fn still_joined(before: &[u32], after: &[u32]) -> bool {
    let groups = before.iter().copied().max().unwrap_or(0) as usize + 1;
    let mut now = vec![0u32; groups];
    for (b, a) in before.iter().zip(after) {
        if *b == 0 || *a == 0 {
            continue;
        }
        let slot = &mut now[*b as usize];
        if *slot == 0 {
            *slot = *a;
        } else if *slot != *a {
            return false;
        }
    }
    true
}

/// The whole pass on a built chunk, under the shipped rules: the hall props, then the facing and the 45 degree turns
/// of the generator's props, then one rebuild of the prop and hunter grids and the flood-fill check. `locks` are the
/// generator's deliberate placements, `region` the chunk's region, `safe` whether the chunk holds a safe room (it
/// gets no hall props). A chunk nothing changes in is left exactly as it was, grids included.
pub fn dress(f: &mut Facility, fix: &Fixtures, seed: &str, k: ChunkKey, region: &str, locks: &Locks, safe: bool) -> Outcome {
    dress_with(f, fix, seed, k, region, locks, safe, rules(), hall_props::rules())
}

/// `dress` under the given rules.
#[allow(clippy::too_many_arguments)]
pub fn dress_with(
    f: &mut Facility,
    fix: &Fixtures,
    seed: &str,
    k: ChunkKey,
    region: &str,
    locks: &Locks,
    safe: bool,
    rules: &Rules,
    hall: &hall_props::Rules,
) -> Outcome {
    if !rules.enabled && !hall.enabled {
        return Outcome::default();
    }
    let placement = Placement::of(f);
    let n = f.props.len();
    let added = if safe { vec![] } else { hall_props::plan(f, fix, seed, k, region, &placement, hall) };
    let mut out = Outcome { hall: added.len(), ..Outcome::default() };
    let mut work = f.props.clone();
    work.extend(added);
    if rules.enabled {
        let reserved = reserved_cells(f, fix);
        let site = Site { f: &*f, reserved: &reserved, placement: &placement };
        let faced = face_props(&site, &mut work, n, locks, rules, seed, k);
        out.turned = faced.turned;
        out.diagonal = faced.diagonal;
    }
    if out == Outcome::default() {
        return out;
    }
    let r = crate::config::HUNTER_RADIUS;
    let before = body_groups(f, r);
    let old = std::mem::replace(&mut f.props, work);
    f.rebuild_grids();
    if !still_joined(&before, &body_groups(f, r)) {
        f.props = old;
        f.rebuild_grids();
        return Outcome::default();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "facing-test";
    const KEY: ChunkKey = ChunkKey { cx: 2, cy: -1, level: 0 };

    /// A walled floor with one open room from (5,5) to (24,24).
    fn room() -> Facility {
        let mut f = Facility::blank("facing", 35, 35);
        f.open_cells(5, 5, 20, 20);
        f.start = Cell::new(15, 15);
        f.exit = f.start;
        f.rebuild_grids();
        f
    }

    fn fix() -> Fixtures {
        Fixtures { vents: vec![], coins: vec![], extra_hiding: vec![] }
    }

    /// A solid floor prop of `size` (width, depth at quarter turn 0) at `cell`, turned `rotation` quarter turns.
    fn prop(id: &str, ptype: &str, cell: (i32, i32), rotation: i32, size: (f32, f32)) -> Prop {
        let half = if rotation % 2 == 1 { (size.1 / 2.0, size.0 / 2.0) } else { (size.0 / 2.0, size.1 / 2.0) };
        Prop {
            id: id.into(),
            cell: Cell::new(cell.0, cell.1),
            ptype: ptype.into(),
            room: 0,
            rotation,
            mount_y: None,
            solid: true,
            half,
            offset: (0.0, 0.0),
            door: None,
            sewer: None,
            crawl_under: false,
        }
    }

    /// The pass with no hall props, over `props` in the test room.
    fn faced(props: Vec<Prop>, locks: &Locks) -> (Facility, Outcome) {
        let mut f = room();
        f.props = props;
        f.rebuild_grids();
        let mut hall = hall_props::rules().clone();
        hall.enabled = false;
        let out = dress_with(&mut f, &fix(), SEED, KEY, "maze", locks, false, rules(), &hall);
        (f, out)
    }

    #[test]
    fn the_shipped_rules_parse_and_a_bad_file_is_refused() {
        let r = Rules::parse(TEXT).expect("the shipped file parses");
        assert!(r.enabled && r.diagonal);
        assert!(Rules::parse(&TEXT.replacen("\"free_chance\": 0.3", "\"free_chance\": 3", 1)).is_err());
        assert!(Rules::parse(&TEXT.replacen("lr.prop_facing/1", "lr.other/1", 1)).is_err());
    }

    #[test]
    fn a_prop_beside_a_wall_turns_its_back_to_the_wall() {
        // against the north, west, south and east walls of the room, each first facing into its wall
        for (cell, into_wall, want) in [((15, 5), 0, 2), ((5, 15), 1, 3), ((15, 24), 2, 0), ((24, 15), 3, 1)] {
            let (f, out) = faced(vec![prop("a", "filingCabinet", cell, into_wall, (0.8, 0.5))], &Locks::default());
            assert_eq!(f.props[0].quarter(), want, "at {cell:?}");
            assert!(!f.props[0].is_diagonal());
            assert_eq!(out.turned, 1);
            // the facing points at open floor and the cell behind is wall
            let d = FACING[want as usize];
            assert!(!f.is_wall_cell(cell.0 + d.0, cell.1 + d.1));
            assert!(f.is_wall_cell(cell.0 - d.0, cell.1 - d.1));
        }
    }

    #[test]
    fn a_sideways_prop_beside_a_wall_turns_and_its_collider_turns_with_it() {
        // a wide shelf against the north wall, standing sideways (facing east)
        let (f, _) = faced(vec![prop("a", "shopShelf", (15, 5), 3, (1.6, 0.5))], &Locks::default());
        let p = &f.props[0];
        assert_eq!(p.quarter(), 2);
        assert_eq!(p.half, (0.8, 0.25), "the wide side now runs along the wall");
        // the collider is what the grid blocks: a body beside the shelf along the wall is stopped, one in front is not
        assert!(!f.can_occupy(16.1, 5.5, 0.2));
        assert!(f.can_occupy(15.5, 6.6, 0.2));
    }

    #[test]
    fn in_a_corner_a_prop_faces_the_longer_open_run() {
        // a room 20 wide and 6 deep: from its north-west corner the east run is the longer one
        let mut f = Facility::blank("facing", 35, 35);
        f.open_cells(5, 5, 20, 6);
        f.start = Cell::new(15, 8);
        f.props = vec![prop("a", "filingCabinet", (5, 5), 0, (0.6, 0.6))];
        f.rebuild_grids();
        let mut hall = hall_props::rules().clone();
        hall.enabled = false;
        dress_with(&mut f, &fix(), SEED, KEY, "maze", &Locks::default(), false, rules(), &hall);
        assert_eq!(f.props[0].quarter(), 3, "east");
        assert!(!f.props[0].is_diagonal(), "a cabinet is not a corner diagonal type");
    }

    #[test]
    fn a_prop_with_no_wall_near_faces_north() {
        for rotation in 0..4 {
            let (f, out) = faced(vec![prop("a", "officeDesk", (15, 15), rotation, (0.7, 0.7))], &Locks::default());
            assert_eq!(f.props[0].rotation, 0, "from quarter turn {rotation}");
            assert_eq!(out.turned, usize::from(rotation != 0));
        }
        // north is the map's -z
        assert_eq!(FACING[0], (0, -1));
    }

    #[test]
    fn exempt_and_deliberately_placed_props_are_unchanged() {
        let listed = |ptype: &str, cell: (i32, i32)| prop(ptype, ptype, cell, 1, (0.9, 0.5));
        let mut props = vec![
            // kept types, beside a wall and in the open
            listed("singleBed", (5, 10)),
            listed("locker", (15, 5)),
            listed("bedroomWardrobe", (12, 12)),
            // a symmetric type is never re-faced
            listed("floorLamp", (18, 18)),
            // locked by the generator
            prop("locked", "filingCabinet", (24, 10), 3, (0.8, 0.5)),
            // a template prop with no wall beside it keeps the template's facing
            prop("templ", "officeDesk", (10, 18), 2, (0.7, 0.7)),
            // a table something stands on, and the thing on it
            prop("table", "woodenTable", (20, 12), 1, (0.9, 0.9)),
            prop("on", "tableLamp", (20, 12), 2, (0.2, 0.2)),
        ];
        props[7].mount_y = Some(0.8);
        props[7].solid = false;
        let mut locks = Locks::default();
        locks.fixed.insert("locked".into());
        locks.template.insert("templ".into());
        let before = props.clone();
        let (f, out) = faced(props, &locks);
        assert_eq!(out, Outcome::default());
        for (a, b) in before.iter().zip(&f.props) {
            assert_eq!((a.rotation, a.half, a.offset), (b.rotation, b.half, b.offset), "{}", a.id);
        }
    }

    #[test]
    fn a_template_prop_beside_a_wall_still_turns_its_back_to_it() {
        let mut locks = Locks::default();
        locks.template.insert("a".into());
        let (f, _) = faced(vec![prop("a", "filingCabinet", (15, 5), 0, (0.8, 0.5))], &locks);
        assert_eq!(f.props[0].rotation, 2);
    }

    #[test]
    fn a_turn_that_would_not_fit_is_not_made() {
        // a long sideways shelf against the north wall with a crate beside it: turned, it would overlap the crate
        let shelf = prop("a", "shopShelf", (15, 5), 3, (2.4, 0.5));
        let crate_ = prop("b", "storageBarrel", (16, 5), 0, (0.7, 0.7));
        let (f, _) = faced(vec![shelf.clone(), crate_], &Locks::default());
        assert_eq!((f.props[0].rotation, f.props[0].half), (shelf.rotation, shelf.half));
    }

    #[test]
    fn only_listed_types_turn_45_degrees_and_the_turn_is_the_same_every_time() {
        // a grid of free-standing props in the open, half crates (listed) and half desks (not listed)
        let build = || {
            let mut props = vec![];
            for i in 0..8 {
                for j in 0..8 {
                    let ptype = if (i + j) % 2 == 0 { "supplyCrates" } else { "officeDesk" };
                    props.push(prop(&format!("p{i}-{j}"), ptype, (7 + i * 2, 7 + j * 2), 0, (0.6, 0.6)));
                }
            }
            props
        };
        let (a, out) = faced(build(), &Locks::default());
        let (b, _) = faced(build(), &Locks::default());
        let turned: Vec<bool> = a.props.iter().map(|p| p.is_diagonal()).collect();
        assert_eq!(turned, b.props.iter().map(|p| p.is_diagonal()).collect::<Vec<_>>(), "the same seed gives the same turns");
        assert_eq!(a.props.iter().map(|p| p.rotation).collect::<Vec<_>>(), b.props.iter().map(|p| p.rotation).collect::<Vec<_>>());
        let crates = a.props.iter().filter(|p| p.ptype == "supplyCrates").count();
        let diagonal_crates = a.props.iter().filter(|p| p.ptype == "supplyCrates" && p.is_diagonal()).count();
        assert!(a.props.iter().all(|p| p.ptype == "supplyCrates" || !p.is_diagonal()), "a desk never turns 45 degrees");
        assert!(diagonal_crates > 0 && diagonal_crates < crates, "{diagonal_crates} of {crates} crates: a share, not all");
        assert_eq!(out.diagonal, diagonal_crates);
        // a seat type is never diagonal, in the open or in a corner
        let (c, _) = faced(vec![prop("s", "woodenChair", (15, 15), 0, (0.5, 0.5)), prop("t", "loungeCouch", (5, 5), 0, (0.6, 0.6))], &Locks::default());
        assert!(c.props.iter().all(|p| !p.is_diagonal()));
        // another seed gives another pattern
        let mut f = room();
        f.props = build();
        f.rebuild_grids();
        let mut hall = hall_props::rules().clone();
        hall.enabled = false;
        dress_with(&mut f, &fix(), "another-seed", KEY, "maze", &Locks::default(), false, rules(), &hall);
        assert_ne!(turned, f.props.iter().map(|p| p.is_diagonal()).collect::<Vec<_>>());
    }

    #[test]
    fn a_listed_prop_in_a_corner_faces_diagonally_out_of_it() {
        // the south-east corner of the room: open floor lies north and west
        let (f, out) = faced(vec![prop("a", "storageBarrel", (24, 24), 2, (0.6, 0.6))], &Locks::default());
        let p = &f.props[0];
        assert!(p.is_diagonal());
        assert_eq!(out.diagonal, 1);
        // between north (0) and west (1): yaw 45 degrees, facing (-x, -z)
        assert_eq!(p.quarter(), 0);
        assert!((p.yaw() - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
        let facing = (-p.yaw().sin(), -p.yaw().cos());
        assert!(facing.0 < -0.5 && facing.1 < -0.5, "{facing:?}");
        // a square collider is kept, and the prop moved out of the corner, not into the walls
        assert_eq!(p.half, (0.3, 0.3));
        assert!(p.offset.0 <= 0.0 && p.offset.1 <= 0.0);
    }

    #[test]
    fn a_diagonal_prop_with_a_long_box_collides_as_the_turned_boxs_bounding_box() {
        let base = (0.6, 0.2);
        let d = diagonal_half(base);
        assert!((d.0 - 0.8 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && d.0 == d.1);
        // readers of quarter turns still see the quarter turn of a diagonal prop
        let mut p = prop("a", "utilityCart", (15, 15), 3 + DIAGONAL, (1.0, 0.5));
        assert_eq!(p.rotation.rem_euclid(2), 1);
        assert_eq!(p.quarter(), 3);
        p.rotation = 2;
        assert!(!p.is_diagonal());
        assert!((p.yaw() - std::f32::consts::PI).abs() < 1e-6);
    }

    #[test]
    fn the_flood_fill_tells_a_sealed_hall_from_an_open_one() {
        // a hall three wide: a prop on one wall leaves it joined, a wall of props across it does not
        let mut f = Facility::blank("facing", 35, 35);
        f.open_cells(5, 10, 25, 3);
        f.rebuild_grids();
        let r = crate::config::HUNTER_RADIUS;
        let before = body_groups(&f, r);
        f.props = vec![prop("a", "supplyCrates", (15, 10), 2, (0.8, 0.8))];
        f.rebuild_grids();
        assert!(still_joined(&before, &body_groups(&f, r)));
        f.props = (0..3).map(|i| prop(&format!("w{i}"), "supplyCrates", (15, 10 + i), 2, (0.9, 0.9))).collect();
        f.rebuild_grids();
        assert!(!still_joined(&before, &body_groups(&f, r)));
    }

    #[test]
    fn the_hash_is_fixed_for_a_seed_and_cell() {
        assert_eq!(hash("a", [1, 2, 3, 4]), hash("a", [1, 2, 3, 4]));
        assert_ne!(hash("a", [1, 2, 3, 4]), hash("b", [1, 2, 3, 4]));
        assert_ne!(hash("a", [1, 2, 3, 4]), hash("a", [2, 1, 3, 4]));
        let u = unit(hash("a", [1, 2, 3, 4]));
        assert!((0.0..1.0).contains(&u));
    }
}
