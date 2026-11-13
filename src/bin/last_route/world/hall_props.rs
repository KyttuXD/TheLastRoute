//! The hall dressing layer: real, solid props along the walls of wide halls on level 0 (benches, plants, crates,
//! carts, vending machines, bins: types that already exist in data/props, by region). It is a Rust-only layer over
//! the Godot-ported generator, planned by `plan` and adopted by `world::prop_facing::dress` when
//! `world::infinite::chunk` builds a chunk, so it costs nothing per frame. The props are ordinary `Facility` props:
//! the solid-prop index, the hunters' grid, the mesh builder and the interact focus see them like any other.
//!
//! Where a prop may stand (data/world/hall_props.json, docs/PROP_FACING.md): on a hall cell (open floor outside
//! every room) with a wall behind it and behind the cells either side of it, back to the wall, in a hall at least
//! `min_width` cells wide, with the cells in front of it clear (two, so in a three-wide hall props hug one wall), and
//! away from doorways, stairs and ladders, lockers, vents and hiding props, pickups, other props, the start and the
//! chunk border. The pick is a hash of the world seed text and the absolute cell, so the host and the guests agree.
//! `prop_facing::dress` then flood-fills the chunk with a hunter's body before and after: a hall is never sealed.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::door_clearance::{self, Placement};
use super::facility::{Cell, Facility, Prop};
use super::fixtures::Fixtures;
use super::infinite::{ChunkKey, CHUNK};
use super::prop_facing::{centre, flag, hash, number, unit, FACING, ORDER};
use crate::hunters::navigation;
use crate::world::route::prop_tables::{lookup, PROP_COLLIDER_SIZES};

const TEXT: &str = include_str!("../data/world/hall_props.json");
const PATH: &str = "data/world/hall_props.json";

/// Salts of the hashed choices.
const SALT_CHANCE: i32 = 0x61;
const SALT_TYPE: i32 = 0x62;

/// How far a hall's width and a wall run are counted, in cells.
const WIDTH_CAP: i32 = 8;
const RUN_CAP: i32 = 16;

/// One prop type a region's halls draw from.
#[derive(Clone, Debug)]
pub struct Entry {
    pub ptype: String,
    /// (width, depth) in metres at quarter turn 0, used when the type has no collider row.
    pub size: (f32, f32),
}

/// The rules of the hall dressing (data/world/hall_props.json).
#[derive(Clone, Debug)]
pub struct Rules {
    pub enabled: bool,
    pub min_width: i32,
    pub narrow_width: i32,
    pub min_lane: f32,
    pub min_lane_narrow: f32,
    pub long_run: i32,
    pub long_bonus: f32,
    pub spacing: i32,
    pub max_per_chunk: usize,
    pub door_gap: i32,
    pub stair_gap: i32,
    pub hide_gap: i32,
    pub pickup_gap: i32,
    pub prop_gap: i32,
    pub start_gap: i32,
    pub edge_margin: i32,
    pub max_depth: f32,
    pub max_width: f32,
    pub wall_gap: f32,
    pub chance: HashMap<String, f32>,
    pub props: HashMap<String, Vec<Entry>>,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("{PATH}: {e}"))?;
        if v["schema"].as_str() != Some("lr.hall_props/1") {
            return Err(format!("{PATH}: `schema` must be lr.hall_props/1"));
        }
        let cells = |key: &str, lo: f64, hi: f64| number(PATH, &v, key, lo, hi).map(|n| n as i32);
        let mut chance = HashMap::new();
        for (region, n) in v["chance"].as_object().ok_or_else(|| format!("{PATH}: `chance` must be a table"))? {
            let n = n.as_f64().filter(|n| (0.0..=1.0).contains(n)).ok_or_else(|| format!("{PATH}: `chance.{region}` must be 0 to 1"))?;
            chance.insert(region.clone(), n as f32);
        }
        let mut props = HashMap::new();
        for (region, list) in v["props"].as_object().ok_or_else(|| format!("{PATH}: `props` must be a table"))? {
            let list = list.as_array().ok_or_else(|| format!("{PATH}: `props.{region}` must be a list"))?;
            let mut entries = Vec::with_capacity(list.len());
            for e in list {
                let ptype = e["type"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| format!("{PATH}: `props.{region}` has an entry with no `type`"))?;
                let side = |i: usize| {
                    e["size"][i].as_f64().filter(|n| *n > 0.0 && *n <= 4.0).ok_or_else(|| format!("{PATH}: `props.{region}` {ptype}: `size` must be [width, depth] in metres"))
                };
                entries.push(Entry { ptype: ptype.to_string(), size: (side(0)? as f32, side(1)? as f32) });
            }
            props.insert(region.clone(), entries);
        }
        if !chance.contains_key("default") || !props.contains_key("default") {
            return Err(format!("{PATH}: `chance` and `props` each need a `default`"));
        }
        Ok(Rules {
            enabled: flag(PATH, &v, "enabled")?,
            min_width: cells("min_width", 3.0, 16.0)?,
            narrow_width: cells("narrow_width", 3.0, 16.0)?,
            min_lane: number(PATH, &v, "min_lane", 1.0, 8.0)? as f32,
            min_lane_narrow: number(PATH, &v, "min_lane_narrow", 1.0, 8.0)? as f32,
            long_run: cells("long_run", 2.0, 64.0)?,
            long_bonus: number(PATH, &v, "long_bonus", 0.0, 10.0)? as f32,
            spacing: cells("spacing", 1.0, 64.0)?,
            max_per_chunk: cells("max_per_chunk", 0.0, 200.0)? as usize,
            door_gap: cells("door_gap", 1.0, 8.0)?,
            stair_gap: cells("stair_gap", 2.0, 8.0)?,
            hide_gap: cells("hide_gap", 2.0, 8.0)?,
            pickup_gap: cells("pickup_gap", 0.0, 8.0)?,
            prop_gap: cells("prop_gap", 0.0, 8.0)?,
            start_gap: cells("start_gap", 0.0, 8.0)?,
            edge_margin: cells("edge_margin", 1.0, 8.0)?,
            max_depth: number(PATH, &v, "max_depth", 0.1, 2.0)? as f32,
            max_width: number(PATH, &v, "max_width", 0.1, 4.0)? as f32,
            wall_gap: number(PATH, &v, "wall_gap", 0.0, 0.4)? as f32,
            chance,
            props,
        })
    }

    /// The chance per wall-side cell in `region` (the default for a region not listed).
    pub fn chance_for(&self, region: &str) -> f32 {
        self.chance.get(region).or_else(|| self.chance.get("default")).copied().unwrap_or(0.0)
    }

    /// The types `region`'s halls draw from (the default list for a region not listed).
    pub fn props_for(&self, region: &str) -> &[Entry] {
        self.props.get(region).or_else(|| self.props.get("default")).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// The embedded rules. A refused shipped file is a build error found by the first run, named by file and key.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules::parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

/// The collider of a hall prop type as (width, depth) at quarter turn 0: the model's collider row
/// (`route::prop_tables::PROP_COLLIDER_SIZES`, what the generator's own props use), else the entry's size. `None` for
/// a type low enough to walk over (it would not be solid).
fn collider(e: &Entry) -> Option<(f32, f32)> {
    match lookup(PROP_COLLIDER_SIZES, &e.ptype) {
        Some(size) if navigation::walk_over(size) => None,
        Some(size) => Some((size[0], size[2])),
        None => Some(e.size),
    }
}

/// Marks every cell within `gap` (Chebyshev) of `c`.
fn mark(mask: &mut [bool], cols: i32, rows: i32, c: Cell, gap: i32) {
    for y in (c.y - gap).max(0)..=(c.y + gap).min(rows - 1) {
        for x in (c.x - gap).max(0)..=(c.x + gap).min(cols - 1) {
            mask[(y * cols + x) as usize] = true;
        }
    }
}

/// The cells no hall prop may stand on (`Rules` gaps): round doorways, stairs, hiding places, pickups, other props,
/// the start, and along the chunk border.
fn keep_out(f: &Facility, fix: &Fixtures, rules: &Rules) -> Vec<bool> {
    let (cols, rows) = (f.cols, f.rows);
    let mut mask = vec![false; (cols * rows) as usize];
    for site in door_clearance::sites_of(f) {
        for c in &site.cells {
            mark(&mut mask, cols, rows, *c, rules.door_gap);
        }
    }
    for s in &f.stairs {
        for c in s.well_cells().into_iter().chain(s.carve_cells()).chain([s.cell, s.step_cell()]) {
            mark(&mut mask, cols, rows, c, rules.stair_gap);
        }
    }
    for h in &f.hiding {
        mark(&mut mask, cols, rows, h.cell, rules.hide_gap);
        mark(&mut mask, cols, rows, Cell::new(h.cell.x + h.facing.0, h.cell.y + h.facing.1), rules.hide_gap);
    }
    for c in fix.vents.iter().chain(fix.extra_hiding.iter().map(|h| &h.cell)) {
        mark(&mut mask, cols, rows, *c, rules.hide_gap);
    }
    for c in f.relays.iter().chain(&f.batteries).chain(f.pickups.iter().map(|p| &p.0)).chain(&fix.coins) {
        mark(&mut mask, cols, rows, *c, rules.pickup_gap);
    }
    mark(&mut mask, cols, rows, f.start, rules.start_gap);
    let hide_rules = super::fixtures::hide_rules();
    for p in &f.props {
        if p.door.is_some() {
            continue;
        }
        let hides = door_clearance::rules().kind_of_prop(&p.ptype).is_some() || hide_rules.prop_style(&p.ptype).is_some();
        let gap = if hides { rules.hide_gap.max(rules.prop_gap) } else { rules.prop_gap };
        let (cx, cy) = centre(p);
        let (x0, x1) = ((cx - p.half.0 + 0.02).floor() as i32, (cx + p.half.0 - 0.02).floor() as i32);
        let (y0, y1) = ((cy - p.half.1 + 0.02).floor() as i32, (cy + p.half.1 - 0.02).floor() as i32);
        for y in y0.min(p.cell.y)..=y1.max(p.cell.y) {
            for x in x0.min(p.cell.x)..=x1.max(p.cell.x) {
                mark(&mut mask, cols, rows, Cell::new(x, y), gap);
            }
        }
    }
    let m = rules.edge_margin;
    for y in 0..rows {
        for x in 0..cols {
            if x < m || y < m || x >= cols - m || y >= rows - m {
                mask[(y * cols + x) as usize] = true;
            }
        }
    }
    mask
}

/// The hall props of a built chunk, not yet added to it: `f` is read with the grids it has (its own props), and the
/// props come back in placing order with ids `HALL:<cx>:<level>:<cy>:PROP:<n>`. `placement` is the doors' approach
/// rule of the floor. Empty when the region has no chance or no list.
pub fn plan(f: &Facility, fix: &Fixtures, seed: &str, k: ChunkKey, region: &str, placement: &Placement, rules: &Rules) -> Vec<Prop> {
    let chance = rules.chance_for(region);
    let list = rules.props_for(region);
    if !rules.enabled || chance <= 0.0 || list.is_empty() || rules.max_per_chunk == 0 {
        return vec![];
    }
    let (cols, rows) = (f.cols, f.rows);
    let inside = |c: Cell| c.x >= 0 && c.y >= 0 && c.x < cols && c.y < rows;
    let at = |c: Cell| (c.y * cols + c.x) as usize;
    // hall cells: open floor outside every room
    let mut hall = vec![false; (cols * rows) as usize];
    for y in 0..rows {
        for x in 0..cols {
            hall[(y * cols + x) as usize] =
                !f.is_wall_cell(x, y) && !f.is_hole(x, y) && f.room_at(x as f64 + 0.5, y as f64 + 0.5).is_none();
        }
    }
    let is_hall = |c: Cell| inside(c) && hall[at(c)];
    // a real wall inside the chunk: the border beyond it is the next chunk's, never a backing wall
    let is_wall = |c: Cell| inside(c) && f.wall[c.y as usize][c.x as usize];
    let out = keep_out(f, fix, rules);
    let step = |c: Cell, d: (i32, i32), n: i32| Cell::new(c.x + d.0 * n, c.y + d.1 * n);

    // every wall-side hall cell that wins its draw, in hash order (so the per-chunk cap favours no part of the chunk)
    let mut wins: Vec<(u64, Cell, usize, i32)> = Vec::new();
    let m = rules.edge_margin;
    for y in m..rows - m {
        for x in m..cols - m {
            let c = Cell::new(x, y);
            if !hall[at(c)] || out[at(c)] {
                continue;
            }
            for q in ORDER {
                let d = FACING[q];
                let back = step(c, d, -1);
                if !is_wall(back) {
                    continue;
                }
                let mut width = 0;
                while width < WIDTH_CAP && is_hall(step(c, d, width)) {
                    width += 1;
                }
                if width < rules.min_width {
                    continue;
                }
                // the straight wall run through this cell
                let t = (d.1.abs(), d.0.abs());
                let mut run = 1;
                for side in [-1, 1] {
                    let mut i = 1;
                    while i <= RUN_CAP && is_hall(step(c, t, side * i)) && is_wall(step(back, t, side * i)) {
                        i += 1;
                    }
                    run += i - 1;
                }
                let abs = (k.cx * CHUNK + c.x, k.cy * CHUNK + c.y);
                let h = hash(seed, [abs.0, abs.1, k.level, SALT_CHANCE + q as i32]);
                let odds = chance * if run >= rules.long_run { rules.long_bonus } else { 1.0 };
                if unit(h) < odds {
                    wins.push((h, c, q, width));
                }
                // one wall per cell: the first in north, east, south, west order
                break;
            }
        }
    }
    wins.sort_by_key(|w| (w.0, w.1.y, w.1.x));

    let mut props: Vec<Prop> = Vec::new();
    let mut taken = vec![false; (cols * rows) as usize];
    for (_, c, q, width) in wins {
        if props.len() >= rules.max_per_chunk {
            break;
        }
        if props.iter().any(|p| (p.cell.x - c.x).abs().max((p.cell.y - c.y).abs()) < rules.spacing) {
            continue;
        }
        let abs = (k.cx * CHUNK + c.x, k.cy * CHUNK + c.y);
        let entry = &list[(hash(seed, [abs.0, abs.1, k.level, SALT_TYPE]) % list.len() as u64) as usize];
        let Some((w, depth)) = collider(entry) else { continue };
        if depth > rules.max_depth || w > rules.max_width {
            continue;
        }
        let d = FACING[q];
        let t = (d.1.abs(), d.0.abs());
        let (hw, hd) = (w * 0.5, depth * 0.5);
        // the back stays `wall_gap` off the wall face; a deeper prop moves out into the hall
        let shift = (hd - (0.5 - rules.wall_gap)).max(0.0);
        let reach = 0.5 + shift + hd;
        let deep = ((reach - 0.001).ceil() as i32).max(1);
        let beside = if hw - 0.5 <= 0.001 { 0 } else { (hw - 0.5).ceil() as i32 };
        // the clear cells in front: two, or all that is left of a narrow hall
        let left = width - deep;
        let need = if width <= rules.narrow_width { left } else { left.min(2) };
        let lane = width as f32 - reach;
        let min_lane = if width <= rules.narrow_width { rules.min_lane_narrow } else { rules.min_lane };
        if need < 1 || lane < min_lane {
            continue;
        }
        let mut ok = true;
        'cells: for i in -(beside + 1)..=(beside + 1) {
            let side = step(c, t, i);
            // a straight wall behind the prop and one cell past each end: never beside an opening
            if !is_hall(side) || !is_wall(step(side, d, -1)) {
                ok = false;
                break;
            }
            for a in 0..deep + need {
                let cell = step(side, d, a);
                let footprint = a < deep && i.abs() <= beside;
                if !is_hall(cell) || taken[at(cell)] || (footprint && out[at(cell)]) {
                    ok = false;
                    break 'cells;
                }
                if !f.can_occupy(cell.x as f64 + 0.5, cell.y as f64 + 0.5, 0.3) {
                    ok = false;
                    break 'cells;
                }
            }
        }
        if !ok {
            continue;
        }
        let half = if q % 2 == 1 { (hd, hw) } else { (hw, hd) };
        let offset = (d.0 as f32 * shift, d.1 as f32 * shift);
        // the door rule tests a footprint round the cell centre: grow the box by the centre's shift
        if !placement.allows(&entry.ptype, c, (half.0 + offset.0.abs(), half.1 + offset.1.abs()), true) {
            continue;
        }
        for i in -beside..=beside {
            for a in 0..deep {
                taken[at(step(step(c, t, i), d, a))] = true;
            }
        }
        props.push(Prop {
            id: format!("HALL:{}:{}:{}:PROP:{}", k.cx, k.level, k.cy, props.len()),
            cell: c,
            ptype: entry.ptype.clone(),
            room: 0,
            rotation: q as i32,
            mount_y: None,
            solid: true,
            half,
            offset,
            door: None,
            sewer: None,
            crawl_under: false,
        });
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::prop_facing::{self, body_groups, still_joined, Locks};
    use std::collections::HashSet;

    const KEY: ChunkKey = ChunkKey { cx: 0, cy: 3, level: 0 };

    fn fix() -> Fixtures {
        Fixtures { vents: vec![], coins: vec![], extra_hiding: vec![] }
    }

    /// The shipped rules with every wall-side cell winning its draw, so a test hall fills as far as the rules allow.
    fn eager() -> Rules {
        let mut r = rules().clone();
        for v in r.chance.values_mut() {
            *v = 1.0;
        }
        r.max_per_chunk = 200;
        r
    }

    /// A floor with one hall `width` cells wide from x 3 to 31, and (with `side`) a 3-wide side hall opening south
    /// from it at x 15..18.
    fn hall(width: i32, side: bool) -> Facility {
        let mut f = Facility::blank("hall", 35, 35);
        f.open_cells(3, 10, 29, width);
        if side {
            f.open_cells(15, 10 + width, 3, 12);
        }
        f.start = Cell::new(4, 10);
        f.exit = f.start;
        f.rebuild_grids();
        f
    }

    /// The pass on `f` under `rules`, facing off; returns the hall props added.
    fn dressed(f: &mut Facility, seed: &str, rules: &Rules) -> Vec<Prop> {
        let n = f.props.len();
        let mut facing = prop_facing::rules().clone();
        facing.enabled = false;
        prop_facing::dress_with(f, &fix(), seed, KEY, "maze", &Locks::default(), false, &facing, rules);
        f.props[n..].to_vec()
    }

    #[test]
    fn the_shipped_rules_parse_and_name_only_solid_sized_types() {
        let r = Rules::parse(TEXT).expect("the shipped file parses");
        assert!(r.enabled);
        assert!(r.chance_for("no-such-region") > 0.0 && r.chance_for("no-such-region") < 0.2, "the default is low");
        assert_eq!(r.chance_for("sewer"), 0.0);
        for (region, list) in &r.props {
            assert!(!list.is_empty(), "{region}");
            for e in list {
                let (w, d) = collider(e).unwrap_or_else(|| panic!("{region} {}: too low to be solid", e.ptype));
                assert!(w <= r.max_width && d <= r.max_depth, "{region} {}: {w} x {d} m is larger than the rules place", e.ptype);
                assert!(crate::world::fixtures::hide_rules().prop_style(&e.ptype).is_none(), "{region} {}: a hiding prop", e.ptype);
                assert!(door_clearance::rules().kind_of_prop(&e.ptype).is_none(), "{region} {}: a locker, vent or hide prop", e.ptype);
            }
        }
        assert!(Rules::parse(&TEXT.replacen("\"min_width\": 3", "\"min_width\": 1", 1)).is_err());
    }

    #[test]
    fn hall_props_stand_back_to_a_wall_and_collide() {
        let mut placed = 0;
        for seed in ["hall-a", "hall-b", "hall-c"] {
            let mut f = hall(4, false);
            let props = dressed(&mut f, seed, &eager());
            placed += props.len();
            for p in &props {
                let d = FACING[p.quarter() as usize];
                assert!(f.is_wall_cell(p.cell.x - d.0, p.cell.y - d.1), "{} at {:?}: no wall behind", p.ptype, p.cell);
                assert!(!f.is_wall_cell(p.cell.x + d.0, p.cell.y + d.1), "{} at {:?}: faces a wall", p.ptype, p.cell);
                assert!(p.solid && !p.is_diagonal());
                let (cx, cy) = centre(p);
                assert!(!f.can_occupy(cx as f64, cy as f64, 0.2), "{} at {:?} does not block", p.ptype, p.cell);
                assert!(!f.hunter_open(p.cell) || p.half.0.min(p.half.1) < 0.3, "{} at {:?}: the hunters' grid does not see it", p.ptype, p.cell);
            }
        }
        assert!(placed > 0, "an eager pass places props in a four-wide hall");
    }

    #[test]
    fn hall_props_never_block_a_hall() {
        let r = crate::config::HUNTER_RADIUS;
        let mut placed = 0;
        for width in [3, 4, 5, 6] {
            for seed in ["hall-a", "hall-b", "hall-c", "hall-d"] {
                let mut f = hall(width, true);
                let before = body_groups(&f, r);
                let props = dressed(&mut f, seed, &eager());
                placed += props.len();
                let after = body_groups(&f, r);
                assert!(still_joined(&before, &after), "width {width} {seed}: a hall was cut");
                // every column of the main hall still has a cell a hunter's body crosses at
                for x in 3..32 {
                    assert!((0..width).any(|a| after[((10 + a) * f.cols + x) as usize] != 0), "width {width} {seed}: column {x} is shut");
                }
                // in a three-wide hall the props hug one wall: the cell at the far wall stays clear in front of each
                if width == 3 {
                    for p in &props {
                        let d = FACING[p.quarter() as usize];
                        let c = Cell::new(p.cell.x + d.0 * 2, p.cell.y + d.1 * 2);
                        if f.is_wall_cell(c.x + d.0, c.y + d.1) {
                            assert!(f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.3), "width 3 {seed}: {c:?} across from {:?} is not clear", p.cell);
                        }
                    }
                }
            }
        }
        assert!(placed > 0);
    }

    #[test]
    fn a_narrow_hall_gets_no_props() {
        for width in [1, 2] {
            let mut f = hall(width, false);
            assert!(dressed(&mut f, "hall-a", &eager()).is_empty(), "width {width}");
        }
    }

    #[test]
    fn no_hall_prop_stands_before_an_opening_a_door_a_stair_or_a_hiding_place() {
        for seed in ["hall-a", "hall-b", "hall-c", "hall-d"] {
            let mut f = hall(4, true);
            // a locker in the north wall's side of the hall and a pickup on the floor
            f.hiding.push(crate::world::facility::Hiding { cell: Cell::new(24, 10), facing: (0, 1), room: None, kind: String::new() });
            f.relays.push(Cell::new(8, 10));
            f.rebuild_grids();
            let rules = eager();
            let props = dressed(&mut f, seed, &rules);
            let cells: HashSet<Cell> = props.iter().map(|p| p.cell).collect();
            for p in &props {
                // the side hall opens through the south wall at x 15..18: no prop stands at its mouth or beside it
                let d = FACING[p.quarter() as usize];
                let t = (d.1.abs(), d.0.abs());
                for i in -1..=1 {
                    let behind = Cell::new(p.cell.x + t.0 * i - d.0, p.cell.y + t.1 * i - d.1);
                    assert!(f.is_wall_cell(behind.x, behind.y), "{seed}: {:?} stands at an opening", p.cell);
                }
                assert!((p.cell.x - 24).abs().max((p.cell.y - 10).abs()) > rules.hide_gap, "{seed}: {:?} is by the locker", p.cell);
                assert!((p.cell.x - 8).abs().max((p.cell.y - 10).abs()) > rules.pickup_gap, "{seed}: {:?} is on the pickup", p.cell);
                assert!((p.cell.x - f.start.x).abs().max((p.cell.y - f.start.y).abs()) > rules.start_gap);
                let m = rules.edge_margin;
                assert!(p.cell.x >= m && p.cell.y >= m && p.cell.x < f.cols - m && p.cell.y < f.rows - m);
            }
            // spacing between hall props
            for a in &cells {
                for b in &cells {
                    assert!(a == b || (a.x - b.x).abs().max((a.y - b.y).abs()) >= rules.spacing, "{seed}: {a:?} and {b:?} are too close");
                }
            }
        }
    }

    #[test]
    fn hall_props_are_the_same_for_a_seed_and_differ_between_seeds() {
        let place = |seed: &str| {
            let mut f = hall(5, true);
            dressed(&mut f, seed, rules()).iter().map(|p| (p.cell, p.ptype.clone(), p.rotation)).collect::<Vec<_>>()
        };
        assert_eq!(place("hall-a"), place("hall-a"));
        let eager_place = |seed: &str| {
            let mut f = hall(5, true);
            dressed(&mut f, seed, &eager()).iter().map(|p| (p.cell, p.ptype.clone(), p.rotation)).collect::<Vec<_>>()
        };
        assert_eq!(eager_place("hall-a"), eager_place("hall-a"));
        assert_ne!(eager_place("hall-a"), eager_place("hall-b"));
    }

    #[test]
    fn a_region_with_no_chance_and_a_safe_chunk_get_no_hall_props() {
        let mut f = hall(5, false);
        let placement = Placement::of(&f);
        assert!(plan(&f, &fix(), "hall-a", KEY, "sewer", &placement, &eager_keeping_zero()).is_empty());
        let mut facing = prop_facing::rules().clone();
        facing.enabled = false;
        let out = prop_facing::dress_with(&mut f, &fix(), "hall-a", KEY, "maze", &Locks::default(), true, &facing, &eager());
        assert_eq!(out.hall, 0);
        assert!(f.props.is_empty());
    }

    /// The shipped rules with the cap lifted (a zero chance stays zero).
    fn eager_keeping_zero() -> Rules {
        let mut r = rules().clone();
        r.max_per_chunk = 200;
        r
    }
}
