//! Door clearance: the floor round a room door that no locker, vent or hide spot may take.
//!
//! One rule, one place. Every pass that puts a locker, a vent or a hide spot on a floor asks this module whether the
//! cell is in a door's clearance: the live level-0 adapter (`route::live`, through `route::fixtures`), the finite and
//! legacy floors (`Facility::place_hiding_spots`, `Facility::populate_rooms` through `drop_blocked`) and the vent ring
//! (`fixtures::place`). `validate` is the world check: it reads a built floor and lists every hide spot, vent or
//! locker prop that stands in a door's clearance and every doorway that has lost its approach.
//!
//! What is cleared round a door, per kind (`data/props/clearance.json`, `Rules`):
//! - the doorway itself (every cell the door opens, one per `door_span`): a double or wide door opens two or three
//!   cells, and the outer ones used to be wall-backed "hiding cells" in the room's rectangle;
//! - `along` cells in front of and behind the doorway row (the door's swing), at least `ceil(span * swing_per_span)`
//!   for a wide door, whose leaves are longer;
//! - `across` cells past each end of the doorway;
//! - the cells between two doors on the same axis that are at most `between_doors_gap` cells apart (a double-door
//!   pair, doors facing each other across a hall).
//!
//! A floor with no door props (the finite and legacy generators carve bare openings in a room's perimeter) is read
//! through those openings instead (`opening_sites`), so the same rule covers it. On those floors the placement pass
//! (`Placement`, read by `Facility::populate_rooms`) skips a prop that would stand in a clearance or close a doorway's
//! approach, so the prop takes its next best cell and the navigation floor is not changed afterwards. `drop_blocked`
//! keeps `clear_approaches` as a net for a solid prop that still closes an approach; on the sweeps it takes none.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::facility::{Cell, Facility, Prop, DEFAULT_AGENT_RADIUS};
use super::fixtures::Fixtures;
use crate::config::HUNTER_RADIUS;

/// The data file's name in messages.
pub const FILE: &str = "data/props/clearance.json";
const TEXT: &str = include_str!("../data/props/clearance.json");

/// What stands near a door: a locker (a wall hiding spot), a vent grate, or another hide spot (a bed or wardrobe).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Locker,
    Vent,
    Hide,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Locker, Kind::Vent, Kind::Hide];

    /// The key of the kind in the data file.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Locker => "locker",
            Kind::Vent => "vent",
            Kind::Hide => "hide",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The clearance of one kind, in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindRule {
    /// Cells in front of and behind the doorway row that stay clear.
    pub along: i32,
    /// Cells past each end of the doorway that stay clear.
    pub across: i32,
}

/// The clearance rules (`data/props/clearance.json`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    kinds: [KindRule; 3],
    /// A door of span N keeps at least `ceil(N * swing_per_span)` cells clear in front of and behind it.
    pub swing_per_span: f32,
    /// The widest gap, in cells, between two doors whose in-between cells are cleared too.
    pub between_doors_gap: i32,
    /// How many cells in front of and behind a doorway must stay free of lockers.
    pub approach_depth: i32,
    /// How far a fixture may be moved to leave a clearance before it is dropped.
    pub relocate_radius: i32,
    prop_types: [Vec<String>; 3],
}

impl Rules {
    /// Reads and checks a clearance file. Every error names the file and the key at fault.
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("{FILE}: {e}"))?;
        let int = |obj: &serde_json::Value, path: &str, key: &str, lo: i64, hi: i64| -> Result<i32, String> {
            let n = obj
                .get(key)
                .ok_or_else(|| format!("{FILE}: {path} is missing"))?
                .as_i64()
                .ok_or_else(|| format!("{FILE}: {path} must be a whole number"))?;
            if n < lo || n > hi {
                return Err(format!("{FILE}: {path} = {n} is outside {lo}..={hi}"));
            }
            Ok(n as i32)
        };
        let kinds_obj = v.get("kinds").ok_or_else(|| format!("{FILE}: kinds is missing"))?;
        let kinds_map = kinds_obj.as_object().ok_or_else(|| format!("{FILE}: kinds must be an object"))?;
        for key in kinds_map.keys() {
            if !Kind::ALL.iter().any(|k| k.key() == key) {
                return Err(format!("{FILE}: kinds.{key} is not a known kind (locker, vent, hide)"));
            }
        }
        let mut kinds = [KindRule { along: 0, across: 0 }; 3];
        for kind in Kind::ALL {
            let key = kind.key();
            let one = kinds_obj.get(key).ok_or_else(|| format!("{FILE}: kinds.{key} is missing"))?;
            if !one.is_object() {
                return Err(format!("{FILE}: kinds.{key} must be an object"));
            }
            kinds[kind.index()] = KindRule {
                along: int(one, &format!("kinds.{key}.along"), "along", 0, 6)?,
                across: int(one, &format!("kinds.{key}.across"), "across", 0, 4)?,
            };
        }
        let swing = v
            .get("swing_per_span")
            .ok_or_else(|| format!("{FILE}: swing_per_span is missing"))?
            .as_f64()
            .ok_or_else(|| format!("{FILE}: swing_per_span must be a number"))?;
        if !swing.is_finite() || !(0.0..=2.0).contains(&swing) {
            return Err(format!("{FILE}: swing_per_span = {swing} is outside 0..=2"));
        }
        let between_doors_gap = int(&v, "between_doors_gap", "between_doors_gap", 0, 6)?;
        let approach_depth = int(&v, "approach_depth", "approach_depth", 0, 4)?;
        let relocate_radius = int(&v, "relocate_radius", "relocate_radius", 0, 30)?;
        let types_obj = v.get("prop_types").ok_or_else(|| format!("{FILE}: prop_types is missing"))?;
        let types_map = types_obj.as_object().ok_or_else(|| format!("{FILE}: prop_types must be an object"))?;
        for key in types_map.keys() {
            if !Kind::ALL.iter().any(|k| k.key() == key) {
                return Err(format!("{FILE}: prop_types.{key} is not a known kind (locker, vent, hide)"));
            }
        }
        let mut prop_types: [Vec<String>; 3] = [vec![], vec![], vec![]];
        for kind in Kind::ALL {
            let key = kind.key();
            let list = types_obj
                .get(key)
                .ok_or_else(|| format!("{FILE}: prop_types.{key} is missing"))?
                .as_array()
                .ok_or_else(|| format!("{FILE}: prop_types.{key} must be a list of prop type names"))?;
            for (i, name) in list.iter().enumerate() {
                let name = name
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| format!("{FILE}: prop_types.{key}[{i}] must be a non-empty string"))?;
                if let Some(other) = Kind::ALL.iter().find(|k| prop_types[k.index()].iter().any(|n| n == name)) {
                    return Err(format!(
                        "{FILE}: prop_types.{key}[{i}] \"{name}\" is already listed under prop_types.{}",
                        other.key()
                    ));
                }
                prop_types[kind.index()].push(name.to_string());
            }
        }
        Ok(Rules {
            kinds,
            swing_per_span: swing as f32,
            between_doors_gap,
            approach_depth,
            relocate_radius,
            prop_types,
        })
    }

    pub fn kind(&self, kind: Kind) -> KindRule {
        self.kinds[kind.index()]
    }

    /// The prop type names that count as `kind`.
    pub fn prop_types(&self, kind: Kind) -> &[String] {
        &self.prop_types[kind.index()]
    }

    /// The kind of a prop type, when it is one the rule governs.
    pub fn kind_of_prop(&self, ptype: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| self.prop_types[k.index()].iter().any(|n| n == ptype))
    }

    /// Cells a door of `span` keeps clear in front of and behind its doorway row for `kind`.
    pub fn depth(&self, kind: Kind, span: i32) -> i32 {
        let swing = (span.max(1) as f32 * self.swing_per_span).ceil() as i32;
        self.kind(kind).along.max(swing)
    }
}

/// The embedded rules. A refused shipped file is a build error found by the first run, named by file and key.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules::parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

/// One doorway: the cells a door opens, in a row across its wall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoorSite {
    /// The doorway cells (`door_span` of them), in order along the wall.
    pub cells: Vec<Cell>,
    /// Whether people pass along y (the doorway cells differ in x) or along x.
    pub vertical: bool,
}

impl DoorSite {
    pub fn span(&self) -> i32 {
        self.cells.len() as i32
    }

    /// The doorway as an inclusive cell rectangle `(x0, y0, x1, y1)`.
    fn rect(&self) -> (i32, i32, i32, i32) {
        let mut r = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for c in &self.cells {
            r = (r.0.min(c.x), r.1.min(c.y), r.2.max(c.x), r.3.max(c.y));
        }
        r
    }

    /// The cell `along` cells in front of (`+`) or behind (`-`) the doorway cell `i`.
    pub fn approach(&self, i: usize, along: i32) -> Cell {
        let c = self.cells[i];
        if self.vertical {
            Cell::new(c.x, c.y + along)
        } else {
            Cell::new(c.x + along, c.y)
        }
    }
}

/// The doorways of a floor: its door props, else (a floor with no door props) the openings in its rooms' perimeters.
pub fn sites_of(f: &Facility) -> Vec<DoorSite> {
    let mut sites: Vec<DoorSite> = f.props.iter().filter_map(door_site_of_prop).collect();
    if sites.is_empty() {
        sites = opening_sites(f);
    }
    sites
}

/// The doorway of a door prop: `door_span` cells across the wall, centred on `cell + .5 + offset`. An even rotation
/// lays the door's slab along x (`Facility::door_slab`), so people pass along y.
fn door_site_of_prop(p: &Prop) -> Option<DoorSite> {
    let d = p.door?;
    let span = d.span.max(1);
    let cx = p.cell.x as f32 + 0.5 + d.offset[0];
    let cy = p.cell.y as f32 + 0.5 + d.offset[1];
    let vertical = p.rotation.rem_euclid(2) == 0;
    let cells = (0..span)
        .map(|i| {
            let shift = -(span as f32) * 0.5 + 0.5 + i as f32;
            if vertical {
                Cell::new((cx + shift).floor() as i32, cy.floor() as i32)
            } else {
                Cell::new(cx.floor() as i32, (cy + shift).floor() as i32)
            }
        })
        .collect();
    Some(DoorSite { cells, vertical })
}

/// The widest opening in a room's perimeter that counts as a doorway.
const MAX_OPENING: usize = 4;

/// The openings of a floor with no door props: the runs of perimeter cells of each room (corners excluded) that open
/// onto floor on the far side of the wall, a hall's mouth. A room of the finite generator is carved solid, so its whole
/// side is open floor; the test is the cell outside the room, not the perimeter cell itself. A run longer than
/// `MAX_OPENING` is a hall running along the wall, or an arcade, not a door.
fn opening_sites(f: &Facility) -> Vec<DoorSite> {
    let mut out: Vec<DoorSite> = Vec::new();
    let open = |x: i32, y: i32| !f.is_wall_cell(x, y);
    for r in &f.rooms {
        if r.w < 3 || r.h < 3 {
            continue;
        }
        // (cells of one perimeter side, step to the inside, vertical passage)
        let sides: [(Vec<(i32, i32)>, (i32, i32), bool); 4] = [
            ((r.x + 1..r.x + r.w - 1).map(|x| (x, r.y)).collect(), (0, 1), true),
            ((r.x + 1..r.x + r.w - 1).map(|x| (x, r.y + r.h - 1)).collect(), (0, -1), true),
            ((r.y + 1..r.y + r.h - 1).map(|y| (r.x, y)).collect(), (1, 0), false),
            ((r.y + 1..r.y + r.h - 1).map(|y| (r.x + r.w - 1, y)).collect(), (-1, 0), false),
        ];
        for (cells, inward, vertical) in sides {
            let mut run: Vec<Cell> = Vec::new();
            let mut flush = |run: &mut Vec<Cell>| {
                if !run.is_empty() && run.len() <= MAX_OPENING {
                    let through = run.iter().any(|c| {
                        open(c.x + inward.0, c.y + inward.1) && open(c.x - inward.0, c.y - inward.1)
                    });
                    if through {
                        let site = DoorSite { cells: run.clone(), vertical };
                        if !out.contains(&site) {
                            out.push(site);
                        }
                    }
                }
                run.clear();
            };
            for (x, y) in cells {
                // a mouth cell: floor inside the room and floor on the far side of the wall
                if open(x, y) && open(x - inward.0, y - inward.1) {
                    run.push(Cell::new(x, y));
                } else {
                    flush(&mut run);
                }
            }
            flush(&mut run);
        }
    }
    out
}

/// An inclusive cell rectangle `(x0, y0, x1, y1)`.
type Rect = (i32, i32, i32, i32);

/// The cells of the doors' clearance for every kind, ready to ask.
#[derive(Clone, Debug)]
pub struct Clearance {
    sites: Vec<DoorSite>,
    blocked: [HashSet<Cell>; 3],
}

impl Clearance {
    /// The clearance of a built floor, under the embedded rules.
    pub fn of(f: &Facility) -> Clearance {
        Clearance::new(rules(), sites_of(f))
    }

    /// The clearance of `sites` under `rules`.
    pub fn new(rules: &Rules, sites: Vec<DoorSite>) -> Clearance {
        let mut blocked: [HashSet<Cell>; 3] = [HashSet::new(), HashSet::new(), HashSet::new()];
        let rects: Vec<(Rect, bool, i32)> = sites.iter().map(|s| (s.rect(), s.vertical, s.span())).collect();
        for kind in Kind::ALL {
            let set = &mut blocked[kind.index()];
            for &(rect, vertical, span) in &rects {
                add_rect(set, grow(rules, kind, rect, vertical, span));
            }
            // the cells between two doors on the same axis: side by side in one wall, or facing each other across a hall
            for (i, &(a, va, sa)) in rects.iter().enumerate() {
                for &(b, vb, sb) in &rects[i + 1..] {
                    if va != vb {
                        continue;
                    }
                    let gap_x = a.0.max(b.0) - a.2.min(b.2) - 1;
                    let gap_y = a.1.max(b.1) - a.3.min(b.3) - 1;
                    let gap = rules.between_doors_gap;
                    let apart = (gap_y < 0 && (1..=gap).contains(&gap_x)) || (gap_x < 0 && (1..=gap).contains(&gap_y));
                    if apart {
                        let both = (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3));
                        add_rect(set, grow(rules, kind, both, va, sa.max(sb)));
                    }
                }
            }
        }
        Clearance { sites, blocked }
    }

    /// The doorways the clearance was made from.
    pub fn sites(&self) -> &[DoorSite] {
        &self.sites
    }

    /// Whether `cell` is in a door's clearance for `kind`.
    pub fn blocks(&self, kind: Kind, cell: Cell) -> bool {
        self.blocked[kind.index()].contains(&cell)
    }

    /// The nearest cell to `from` within `radius` (Chebyshev) that is outside the clearance of `kind` and passes `ok`,
    /// nearest first, ties by y then x. `None` when there is none.
    pub fn nearest_clear(&self, kind: Kind, from: Cell, radius: i32, ok: impl Fn(Cell) -> bool) -> Option<Cell> {
        let mut best: Option<(i64, Cell)> = None;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let c = Cell::new(from.x + dx, from.y + dy);
                if self.blocks(kind, c) || !ok(c) {
                    continue;
                }
                let d = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                if best.map_or(true, |(bd, bc)| d < bd || (d == bd && (c.y, c.x) < (bc.y, bc.x))) {
                    best = Some((d, c));
                }
            }
        }
        best.map(|(_, c)| c)
    }
}

/// A doorway rectangle grown by the kind's clearance: `across` past each end, the swing depth in front and behind.
fn grow(rules: &Rules, kind: Kind, r: Rect, vertical: bool, span: i32) -> Rect {
    let across = rules.kind(kind).across;
    let depth = rules.depth(kind, span);
    if vertical {
        (r.0 - across, r.1 - depth, r.2 + across, r.3 + depth)
    } else {
        (r.0 - depth, r.1 - across, r.2 + depth, r.3 + across)
    }
}

fn add_rect(set: &mut HashSet<Cell>, r: Rect) {
    for y in r.1..=r.3 {
        for x in r.0..=r.2 {
            set.insert(Cell::new(x, y));
        }
    }
}

/// The cell a prop stands on: the cell of its centre (`cell + .5 + offset`).
pub fn prop_cell(p: &Prop) -> Cell {
    Cell::new((p.cell.x as f32 + 0.5 + p.offset.0).floor() as i32, (p.cell.y as f32 + 0.5 + p.offset.1).floor() as i32)
}

/// What `drop_blocked` took off a floor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    pub lockers: usize,
    pub props: usize,
}

/// Whether a hunter's body (`HUNTER_RADIUS`, the body `hunters::brain::Standable` plans with) can stand at the centre of
/// `c`. The hunter grid is built for the thinner agent radius, so a cell it calls open can still be
/// too tight for the hunters: a doorway approach like that is closed to them, and the door rule judges it by this.
fn hunter_stands(f: &Facility, c: Cell) -> bool {
    f.body_fits(c.x as f32 + 0.5, c.y as f32 + 0.5, HUNTER_RADIUS)
}

/// The placement side of the rule, read by `Facility::populate_rooms` before it puts a room prop down. It is made from
/// a floor with no props yet: the governed kinds' clearance, and the cells a hunter must keep open (every doorway cell,
/// and each approach cell that is open now). A prop that would stand in the clearance, or a solid prop whose footprint
/// would close a kept cell, is skipped at that cell, so the prop takes the next best cell. A floor's navigation then
/// never changes after placement: `clear_approaches` finds nothing to take off.
#[derive(Clone, Debug)]
pub struct Placement {
    clear: Clearance,
    keep: Vec<Cell>,
}

impl Placement {
    /// The placement rule of a floor that has no props placed yet.
    pub fn of(f: &Facility) -> Placement {
        let depth = rules().approach_depth;
        let clear = Clearance::of(f);
        let mut keep: Vec<Cell> = Vec::new();
        for site in clear.sites() {
            for i in 0..site.cells.len() {
                keep.push(site.cells[i]);
                for side in [-1, 1] {
                    for step in 1..=depth {
                        keep.push(site.approach(i, side * step));
                    }
                }
            }
        }
        keep.retain(|c| !f.is_wall_cell(c.x, c.y) && !f.is_hole(c.x, c.y) && hunter_stands(f, *c));
        keep.sort();
        keep.dedup();
        Placement { clear, keep }
    }

    /// Whether a prop of type `ptype`, with footprint half-size `half` and solidity `solid`, may stand on `cell`.
    pub fn allows(&self, ptype: &str, cell: Cell, half: (f32, f32), solid: bool) -> bool {
        if let Some(kind) = rules().kind_of_prop(ptype) {
            if self.clear.blocks(kind, cell) {
                return false;
            }
        }
        if !solid {
            return true;
        }
        // the hunters' body test: a body at a cell centre is shut out of a solid prop's footprint grown by the body radius
        let (ox, oy) = (cell.x as f64 + 0.5, cell.y as f64 + 0.5);
        let r = HUNTER_RADIUS as f64;
        !self.keep.iter().any(|k| {
            let (kx, ky) = (k.x as f64 + 0.5, k.y as f64 + 0.5);
            (kx - ox).abs() < half.0 as f64 + r && (ky - oy).abs() < half.1 as f64 + r
        })
    }
}

/// Takes the lockers and the locker, vent and hide-spot props that stand in a door's clearance off the floor, then
/// rebuilds the solid index and the hunters' grid so every dropped cell is open again. Room doors stay.
pub fn drop_blocked(f: &mut Facility) -> Dropped {
    let clear = Clearance::of(f);
    let mut dropped = drop_blocked_by(f, &clear);
    if dropped.props + dropped.lockers > 0 {
        f.rebuild_grids();
    }
    dropped.props += clear_approaches(f);
    dropped
}

/// How many rounds `clear_approaches` runs: a prop taken off can free the cell of the next one.
const APPROACH_ROUNDS: usize = 4;

/// Takes the solid props that close a doorway's approach off the floor. An approach cell (within `approach_depth` of a
/// doorway, on either side of it) that a hunter's body cannot stand on loses the solid, non-door prop that stands on it; when
/// none stands on it, the solid props on the cells next to it (a partition beside the mouth of a room). The rule keeps
/// the approach passable, not only free of lockers. Rebuilds the solid index and the hunters' grid between rounds.
/// Returns how many props went.
pub fn clear_approaches(f: &mut Facility) -> usize {
    let depth = rules().approach_depth;
    let mut total = 0;
    for _ in 0..APPROACH_ROUNDS {
        f.rebuild_grids();
        let clear = Clearance::of(f);
        let mut doomed = vec![false; f.props.len()];
        for site in clear.sites() {
            for i in 0..site.cells.len() {
                for side in [-1, 1] {
                    for step in 1..=depth {
                        let c = site.approach(i, side * step);
                        let floor = !f.is_wall_cell(c.x, c.y) && !f.is_hole(c.x, c.y);
                        if !floor || hunter_stands(f, c) {
                            continue;
                        }
                        let solid = |p: &Prop| p.solid && p.door.is_none();
                        let on: Vec<usize> = (0..f.props.len()).filter(|&j| solid(&f.props[j]) && prop_cell(&f.props[j]) == c).collect();
                        let near: Vec<usize> = (0..f.props.len())
                            .filter(|&j| {
                                let at = prop_cell(&f.props[j]);
                                solid(&f.props[j]) && (at.x - c.x).abs() <= 1 && (at.y - c.y).abs() <= 1
                            })
                            .collect();
                        for j in if on.is_empty() { near } else { on } {
                            doomed[j] = true;
                        }
                    }
                }
            }
        }
        let n = doomed.iter().filter(|d| **d).count();
        if n == 0 {
            break;
        }
        let mut index = 0;
        f.props.retain(|_| {
            let keep = !doomed[index];
            index += 1;
            keep
        });
        total += n;
    }
    if total > 0 {
        f.rebuild_grids();
    }
    total
}

/// `drop_blocked` with a clearance already made, for a caller that rebuilds the grids itself afterwards (the props'
/// solid index is stale until it does).
pub fn drop_blocked_by(f: &mut Facility, clear: &Clearance) -> Dropped {
    let rules = rules();
    let lockers = f.hiding.len();
    f.hiding.retain(|h| !clear.blocks(Kind::Locker, h.cell));
    let lockers = lockers - f.hiding.len();
    let props = f.props.len();
    f.props.retain(|p| {
        if p.door.is_some() {
            return true;
        }
        match rules.kind_of_prop(&p.ptype) {
            Some(kind) => !clear.blocks(kind, prop_cell(p)),
            None => true,
        }
    });
    let props = props - f.props.len();
    Dropped { lockers, props }
}

/// Takes the vents that stand in a door's clearance out of `fix`. Returns how many went.
pub fn drop_blocked_vents(clear: &Clearance, fix: &mut Fixtures) -> usize {
    let before = fix.vents.len();
    fix.vents.retain(|c| !clear.blocks(Kind::Vent, *c));
    before - fix.vents.len()
}

/// The world check: every way a built floor breaks the rule, as readable lines. Empty when the floor is clean.
/// - a hiding locker, a vent, or a locker, vent or hide-spot prop inside a door's clearance;
/// - a doorway that has lost its approach: an open cell within `approach_depth` of the doorway that a body cannot
///   stand on, or whose hunters' grid cell is closed, while the doorway itself is open.
pub fn validate(f: &Facility, fix: &Fixtures) -> Vec<String> {
    let rules = rules();
    let clear = Clearance::of(f);
    let mut out = Vec::new();
    for h in &f.hiding {
        if clear.blocks(Kind::Locker, h.cell) {
            out.push(format!("locker at {},{} is inside a door's clearance", h.cell.x, h.cell.y));
        }
    }
    for c in &fix.vents {
        if clear.blocks(Kind::Vent, *c) {
            out.push(format!("vent at {},{} is inside a door's clearance", c.x, c.y));
        }
    }
    for p in &f.props {
        if p.door.is_some() {
            continue;
        }
        if let Some(kind) = rules.kind_of_prop(&p.ptype) {
            let at = prop_cell(p);
            if clear.blocks(kind, at) {
                out.push(format!("{} prop {} at {},{} is inside a door's clearance", kind.key(), p.id, at.x, at.y));
            }
        }
    }
    out.extend(approach_problems(f, &clear));
    out
}

/// The doorways whose approach a locker or a closed navigation cell takes (see `validate`): a doorway cell the hunters'
/// grid has closed, a locker standing in an approach cell, or a side of the doorway (in front or behind) that is floor
/// but has no approach cell a hunter can stand on.
pub fn approach_problems(f: &Facility, clear: &Clearance) -> Vec<String> {
    let depth = rules().approach_depth;
    let mut out = Vec::new();
    let floor = |c: Cell| !f.is_wall_cell(c.x, c.y) && !f.is_hole(c.x, c.y);
    for site in clear.sites() {
        for (i, door_cell) in site.cells.iter().enumerate() {
            if !floor(*door_cell) {
                continue;
            }
            if !f.hunter_open(*door_cell) || !hunter_stands(f, *door_cell) {
                out.push(format!("the doorway cell {},{} is closed to the hunters", door_cell.x, door_cell.y));
            }
            for side in [-1, 1] {
                for step in 1..=depth {
                    let c = site.approach(i, side * step);
                    if !floor(c) {
                        break;
                    }
                    if f.hiding.iter().any(|h| h.cell == c) {
                        out.push(format!(
                            "the doorway at {},{} has a locker in its approach cell {},{}",
                            door_cell.x, door_cell.y, c.x, c.y
                        ));
                    }
                }
            }
        }
        for side in [-1, 1] {
            let cells: Vec<Cell> = (0..site.cells.len())
                .flat_map(|i| (1..=depth).map(move |step| (i, step)))
                .map(|(i, step)| site.approach(i, side * step))
                .filter(|c| floor(*c))
                .collect();
            if !cells.is_empty() && !cells.iter().any(|c| f.hunter_open(*c) && hunter_stands(f, *c)) {
                let first = site.cells[0];
                out.push(format!(
                    "the doorway at {},{} has no free approach {} it",
                    first.x,
                    first.y,
                    if side < 0 { "behind" } else { "in front of" }
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn site(cells: &[(i32, i32)], vertical: bool) -> DoorSite {
        DoorSite { cells: cells.iter().map(|&(x, y)| Cell::new(x, y)).collect(), vertical }
    }

    fn shipped() -> Rules {
        Rules::parse(TEXT).expect("the shipped file parses")
    }

    #[test]
    fn the_shipped_file_parses_and_the_embedded_rules_are_it() {
        let r = shipped();
        assert_eq!(*rules(), r);
        for kind in Kind::ALL {
            assert!(r.kind(kind).along >= 1, "{} keeps the swing clear", kind.key());
            assert!(!r.prop_types(kind).is_empty(), "{} has prop types", kind.key());
        }
        assert_eq!(r.kind_of_prop("locker"), Some(Kind::Locker));
        assert_eq!(r.kind_of_prop("lavaVent"), Some(Kind::Vent));
        assert_eq!(r.kind_of_prop("bedroomWardrobe"), Some(Kind::Hide));
        assert_eq!(r.kind_of_prop("table"), None);
    }

    #[test]
    fn a_door_clears_its_doorway_and_the_swing_in_front_and_behind() {
        let rules = shipped();
        // a single door in a wall along y = 10, people pass along y
        let c = Clearance::new(&rules, vec![site(&[(10, 10)], true)]);
        for kind in Kind::ALL {
            assert!(c.blocks(kind, Cell::new(10, 10)), "{}: the door cell", kind.key());
            assert!(c.blocks(kind, Cell::new(10, 9)), "{}: behind the door", kind.key());
            assert!(c.blocks(kind, Cell::new(10, 11)), "{}: in front of the door", kind.key());
            assert!(!c.blocks(kind, Cell::new(10, 13)), "{}: well away", kind.key());
            assert!(!c.blocks(kind, Cell::new(12, 11)), "{}: beside, two cells over", kind.key());
        }
    }

    #[test]
    fn a_wide_door_clears_every_cell_it_opens_and_a_deeper_swing() {
        let rules = shipped();
        // a three-cell door: the outer cells are the ones the old fixtures pool picked
        let c = Clearance::new(&rules, vec![site(&[(10, 10), (11, 10), (12, 10)], true)]);
        for x in 10..=12 {
            assert!(c.blocks(Kind::Locker, Cell::new(x, 10)), "doorway cell {x}");
            assert!(c.blocks(Kind::Vent, Cell::new(x, 11)), "front of cell {x}");
            assert!(c.blocks(Kind::Vent, Cell::new(x, 8)), "back of cell {x}: a wide door swings two deep");
        }
        assert!(!c.blocks(Kind::Locker, Cell::new(13, 11)), "past the end, across 0");
        // a horizontal door (people pass along x) turns the rule
        let h = Clearance::new(&rules, vec![site(&[(10, 10), (10, 11), (10, 12)], false)]);
        assert!(h.blocks(Kind::Locker, Cell::new(11, 12)));
        assert!(h.blocks(Kind::Locker, Cell::new(8, 10)));
        assert!(!h.blocks(Kind::Locker, Cell::new(11, 13)));
    }

    #[test]
    fn the_cell_between_two_doors_is_clear() {
        let rules = shipped();
        // two single doors side by side with a one-cell pier between them
        let pair = Clearance::new(&rules, vec![site(&[(10, 10)], true), site(&[(12, 10)], true)]);
        assert!(pair.blocks(Kind::Locker, Cell::new(11, 10)), "the pier between the doors");
        assert!(pair.blocks(Kind::Vent, Cell::new(11, 11)), "in front of the pier");
        // doors facing each other across a two-cell hall (the widest gap the shipped rules link)
        let facing = Clearance::new(&rules, vec![site(&[(10, 10)], true), site(&[(10, 13)], true)]);
        assert!(facing.blocks(Kind::Hide, Cell::new(10, 11)), "the first cell of the hall between them");
        assert!(facing.blocks(Kind::Hide, Cell::new(10, 12)), "the second cell of the hall between them");
        // a three-cell hall is wider than `between_doors_gap`: the cells in its middle stay open
        let wide_hall = Clearance::new(&rules, vec![site(&[(10, 10)], true), site(&[(10, 14)], true)]);
        assert!(!wide_hall.blocks(Kind::Hide, Cell::new(10, 12)), "a three-cell hall does not link the doors");
        // doors too far apart do not link up
        let far = Clearance::new(&rules, vec![site(&[(10, 10)], true), site(&[(18, 10)], true)]);
        assert!(!far.blocks(Kind::Locker, Cell::new(14, 11)));
        // perpendicular doors never link
        let turned = Clearance::new(&rules, vec![site(&[(10, 10)], true), site(&[(12, 10)], false)]);
        assert!(!turned.blocks(Kind::Locker, Cell::new(11, 12)));
    }

    #[test]
    fn the_data_changes_the_clearance() {
        let mut text: serde_json::Value = serde_json::from_str(TEXT).unwrap();
        text["kinds"]["vent"]["along"] = serde_json::json!(3);
        text["kinds"]["vent"]["across"] = serde_json::json!(1);
        let wide = Rules::parse(&text.to_string()).unwrap();
        let c = Clearance::new(&wide, vec![site(&[(10, 10)], true)]);
        assert!(c.blocks(Kind::Vent, Cell::new(11, 13)), "three deep, one across");
        assert!(!c.blocks(Kind::Locker, Cell::new(11, 13)), "the locker rule is untouched");
    }

    #[test]
    fn the_nearest_clear_cell_skips_the_clearance() {
        let rules = shipped();
        let c = Clearance::new(&rules, vec![site(&[(10, 10)], true)]);
        let got = c.nearest_clear(Kind::Locker, Cell::new(10, 10), 3, |_| true).unwrap();
        assert!(!c.blocks(Kind::Locker, got));
        assert_eq!(c.nearest_clear(Kind::Locker, Cell::new(10, 10), 0, |_| true), None, "radius 0 finds only the blocked cell");
        assert_eq!(c.nearest_clear(Kind::Locker, Cell::new(10, 10), 5, |_| false), None, "nothing passes");
    }

    fn with(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Rules, String> {
        let mut v: serde_json::Value = serde_json::from_str(TEXT).unwrap();
        edit(&mut v);
        Rules::parse(&v.to_string())
    }

    #[test]
    fn a_bad_data_file_names_the_key_at_fault() {
        let err = |r: Result<Rules, String>| r.expect_err("the file is refused");
        assert!(err(Rules::parse("not json")).starts_with(FILE));
        assert!(err(with(|v| {
            v.as_object_mut().unwrap().remove("kinds");
        }))
        .contains("kinds is missing"));
        assert!(err(with(|v| {
            v["kinds"].as_object_mut().unwrap().remove("vent");
        }))
        .contains("kinds.vent is missing"));
        assert!(err(with(|v| v["kinds"]["locker"]["along"] = serde_json::json!(-1))).contains("kinds.locker.along = -1 is outside"));
        assert!(err(with(|v| v["kinds"]["hide"]["across"] = serde_json::json!("two"))).contains("kinds.hide.across must be a whole number"));
        assert!(err(with(|v| v["kinds"]["hide"]["across"] = serde_json::json!(1.5))).contains("kinds.hide.across must be a whole number"));
        assert!(err(with(|v| {
            v["kinds"].as_object_mut().unwrap().remove("locker");
        }))
        .contains("kinds.locker is missing"));
        assert!(err(with(|v| v["kinds"]["door"] = serde_json::json!({"along": 1, "across": 0}))).contains("kinds.door is not a known kind"));
        assert!(err(with(|v| v["swing_per_span"] = serde_json::json!(9))).contains("swing_per_span = 9 is outside"));
        assert!(err(with(|v| v["swing_per_span"] = serde_json::json!("x"))).contains("swing_per_span must be a number"));
        assert!(err(with(|v| v["between_doors_gap"] = serde_json::json!(99))).contains("between_doors_gap = 99 is outside"));
        assert!(err(with(|v| v["approach_depth"] = serde_json::json!(null))).contains("approach_depth must be a whole number"));
        assert!(err(with(|v| v["relocate_radius"] = serde_json::json!(-4))).contains("relocate_radius = -4 is outside"));
        assert!(err(with(|v| v["prop_types"]["vent"] = serde_json::json!("lavaVent"))).contains("prop_types.vent must be a list"));
        assert!(err(with(|v| v["prop_types"]["vent"] = serde_json::json!([""]))).contains("prop_types.vent[0] must be a non-empty string"));
        assert!(err(with(|v| v["prop_types"]["vent"] = serde_json::json!(["lavaVent", 7]))).contains("prop_types.vent[1] must be a non-empty string"));
        assert!(err(with(|v| {
            v["prop_types"].as_object_mut().unwrap().remove("hide");
        }))
        .contains("prop_types.hide is missing"));
        assert!(err(with(|v| v["prop_types"]["gate"] = serde_json::json!(["x"]))).contains("prop_types.gate is not a known kind"));
    }

    #[test]
    fn a_prop_type_cannot_be_two_kinds() {
        let e = with(|v| v["prop_types"]["hide"] = serde_json::json!(["locker"])).expect_err("a duplicate type is refused");
        assert!(e.contains("prop_types.hide[0] \"locker\" is already listed under prop_types.locker"), "{e}");
    }

    // ---- the rule on built floors ------------------------------------------------------------------------------

    use crate::world::catalog::Catalog;
    use crate::world::facility::{Hiding, Options};
    use crate::world::infinite::ChunkKey;
    use crate::world::route::layout::{self, Coord};
    use crate::world::route::{live, regions, world_seed};
    use std::collections::BTreeSet;

    fn cat() -> &'static Catalog {
        static CAT: OnceLock<Catalog> = OnceLock::new();
        CAT.get_or_init(Catalog::load)
    }

    /// The level-0 chunk of `seed` at `(cx, cz)` as the live floor, with its fixtures.
    fn live_chunk(seed: &str, cx: i32, cz: i32) -> Option<(Facility, Fixtures)> {
        live::chunk(seed, ChunkKey { cx, cy: cz, level: 0 }, cat())
    }

    /// Chunks of many worlds: the sweep every placement test reads.
    fn sweep() -> impl Iterator<Item = (String, i32, i32)> {
        (0..14).flat_map(|n| {
            let seed = world_seed(&format!("ROOM-DOORCLEAR-{n:020}"));
            (-2..=2).flat_map(move |cz| {
                let seed = seed.clone();
                (-2..=2).map(move |cx| (seed.clone(), cx, cz))
            })
        })
    }

    #[test]
    fn no_live_chunk_has_a_locker_or_vent_in_a_doors_clearance() {
        let mut regions_seen: BTreeSet<&'static str> = BTreeSet::new();
        let (mut chunks, mut doors, mut wide, mut lockers, mut vents) = (0, 0, 0, 0, 0);
        for (seed, cx, cz) in sweep() {
            let Some((f, fix)) = live_chunk(&seed, cx, cz) else { continue };
            regions_seen.insert(regions::region_for(&seed, cx, cz, 0));
            chunks += 1;
            let at = format!("world {seed} chunk {cx},{cz}");
            let bad = validate(&f, &fix);
            assert!(bad.is_empty(), "{at}: {bad:?}");
            let sites = sites_of(&f);
            doors += sites.len();
            wide += sites.iter().filter(|s| s.span() >= 3).count();
            lockers += f.hiding.len();
            vents += fix.vents.len();
            // the doorway read from a door prop is the doorway the grid opened: every cell is floor, and so is a cell
            // on at least one side of it (the orientation of the door is right)
            for s in &sites {
                for c in &s.cells {
                    assert!(!f.is_wall_cell(c.x, c.y), "{at}: doorway cell {},{} is a wall (axis wrong?)", c.x, c.y);
                }
                let open_side = |side: i32| {
                    (0..s.cells.len()).any(|i| {
                        let a = s.approach(i, side);
                        !f.is_wall_cell(a.x, a.y)
                    })
                };
                assert!(open_side(1) || open_side(-1), "{at}: doorway at {:?} leads nowhere", s.cells[0]);
            }
        }
        assert!(chunks > 300, "the sweep built {chunks} chunks");
        // level 0 is the concept map: the 5 x 5 chunks round the spawn show the apartment, greenhouse, hospital, mall,
        // maze and prison areas (Godot's draw used to give nine or more types there)
        assert!(regions_seen.len() >= 6, "facility types in the sweep: {regions_seen:?}");
        assert!(doors > 500 && wide > 20, "doors {doors}, wide doors {wide}");
        assert!(lockers > 300 && vents > 100, "lockers {lockers}, vents {vents}: the sweep must still place fixtures");
    }

    #[test]
    fn the_layout_does_put_wall_fixtures_in_door_clearance_and_the_adapter_clears_them() {
        // the Godot pool is door-blind: without the rule the sweep would fail. Count what the layout alone gives.
        let (mut raw_bad, mut layout_total, mut live_total) = (0, 0, 0);
        for (seed, cx, cz) in sweep() {
            let Some(lay) = layout::chunk_layout(&seed, Coord::new(cx, 0, cz)) else { continue };
            let Some((f, fix)) = live_chunk(&seed, cx, cz) else { continue };
            let clear = Clearance::of(&f);
            let wall: Vec<_> = lay.fixtures.iter().filter(|x| x.hide.is_none()).collect();
            layout_total += wall.len();
            live_total += f.hiding.len() + fix.vents.len();
            for x in &wall {
                let kind = if x.kind == "vent" { Kind::Vent } else { Kind::Locker };
                let cell = Cell::new(x.cell.x, x.cell.y);
                if clear.blocks(kind, cell) {
                    raw_bad += 1;
                    let kept_here = f.hiding.iter().any(|h| h.cell == cell) || fix.vents.contains(&cell);
                    assert!(!kept_here, "world {seed} chunk {cx},{cz}: {} at {cell:?} stayed in a door's clearance", x.kind);
                }
            }
        }
        assert!(raw_bad > 0, "the layout never put a fixture in a door's clearance: the sweep proves nothing");
        // a fixture is moved when a legal cell is near, so most survive
        assert!(live_total * 10 >= layout_total * 9, "{live_total} of {layout_total} wall fixtures survive the rule");
    }

    #[test]
    fn every_doorway_of_the_sweep_keeps_a_free_approach() {
        let depth = rules().approach_depth;
        assert!(depth >= 1);
        for (seed, cx, cz) in sweep().step_by(3) {
            let Some((f, _)) = live_chunk(&seed, cx, cz) else { continue };
            let at = format!("world {seed} chunk {cx},{cz}");
            for s in sites_of(&f) {
                for side in [-1, 1] {
                    let cells: Vec<Cell> = (0..s.cells.len())
                        .map(|i| s.approach(i, side))
                        .filter(|c| !f.is_wall_cell(c.x, c.y) && !f.is_hole(c.x, c.y))
                        .collect();
                    if cells.is_empty() {
                        continue;
                    }
                    assert!(
                        cells.iter().any(|c| f.hunter_open(*c) && hunter_stands(&f, *c)),
                        "{at}: the doorway at {:?} has no free approach on side {side}",
                        s.cells[0]
                    );
                    assert!(
                        cells.iter().all(|c| !f.hiding.iter().any(|h| h.cell == *c)),
                        "{at}: a locker stands in the approach of the doorway at {:?}",
                        s.cells[0]
                    );
                }
            }
        }
    }

    #[test]
    fn finite_and_legacy_floors_of_every_size_keep_lockers_and_vents_out_of_door_clearance() {
        let mut floors = 0;
        let mut spots = 0;
        for n in 0..40 {
            let opt = Options {
                cols: 35 + (n % 5) * 14,
                rows: 35 + (n % 4) * 18,
                room_count: 5 + (n as usize % 8),
                safe_zone_count: n % 3,
                ..Default::default()
            };
            let f = Facility::generate(&format!("DOORCLEAR-FLOOR-{n}"), &opt, cat());
            let fix = crate::world::fixtures::place(&f);
            floors += 1;
            spots += f.hiding.len() + fix.vents.len();
            let bad = validate(&f, &fix);
            assert!(bad.is_empty(), "floor {n}: {bad:?}");
            assert!(!sites_of(&f).is_empty(), "floor {n}: no doorway found");
        }
        assert_eq!(floors, 40);
        assert!(spots > 150, "the floors must still hold lockers and vents, got {spots}");
        // the streamed-layout re-dress the legacy chunks use
        for n in 0..12 {
            for region in ["maze", "mansion", "hotel", "prison", "gothic", "hospital", "lab", "mall"] {
                let mut f = Facility::generate(&format!("DOORCLEAR-LEGACY-{n}"), &Options::default(), cat());
                f.apply_godot_streamed_layout(&format!("DOORCLEAR-LEGACY-{n}:{region}"), region, cat());
                let bad = validate(&f, &Fixtures::default());
                assert!(bad.is_empty(), "legacy {region} {n}: {bad:?}");
            }
        }
    }

    /// The first wide doorway of the sweep, with its floor.
    fn a_wide_door() -> (Facility, DoorSite) {
        for (seed, cx, cz) in sweep() {
            let Some((f, _)) = live_chunk(&seed, cx, cz) else { continue };
            if let Some(s) = sites_of(&f).into_iter().find(|s| s.span() >= 3) {
                return (f, s);
            }
        }
        panic!("no wide door in the sweep");
    }

    #[test]
    fn a_locker_in_a_doorway_is_reported_and_dropping_it_reopens_the_cell_for_the_hunters() {
        let (mut f, site) = a_wide_door();
        let outer = site.cells[2];
        assert!(f.hunter_open(outer), "the doorway is open to the hunters to begin with");
        f.hiding.push(Hiding { cell: outer, facing: (0, 1), room: None, kind: String::new() });
        f.rebuild_grids();
        assert!(!f.hunter_open(outer), "a locker closes the cell to the hunters");
        let bad = validate(&f, &Fixtures::default());
        assert!(bad.iter().any(|m| m.contains("locker at") && m.contains(&format!("{},{}", outer.x, outer.y))), "{bad:?}");
        assert!(bad.iter().any(|m| m.contains("closed to the hunters")), "{bad:?}");
        let dropped = drop_blocked(&mut f);
        assert_eq!(dropped.lockers, 1);
        assert!(f.hunter_open(outer), "the navigation grid has the doorway open again");
        assert!(f.can_occupy(outer.x as f64 + 0.5, outer.y as f64 + 0.5, 0.23));
        assert!(validate(&f, &Fixtures::default()).is_empty());
    }

    #[test]
    fn a_vent_between_two_doors_is_reported() {
        let (f, site) = a_wide_door();
        let mut fix = Fixtures::default();
        let in_front = site.approach(1, 1);
        fix.vents.push(in_front);
        let bad = validate(&f, &fix);
        assert!(bad.iter().any(|m| m.starts_with("vent at")), "{bad:?}");
        let clear = Clearance::of(&f);
        assert_eq!(drop_blocked_vents(&clear, &mut fix), 1);
        assert!(fix.vents.is_empty());
    }

    #[test]
    fn props_of_the_governed_kinds_in_a_door_clearance_are_dropped() {
        let (mut f, site) = a_wide_door();
        let front = site.approach(0, 1);
        let mut prop = f.props.iter().find(|p| p.door.is_none()).cloned().expect("a plain prop");
        prop.id = "TEST-LOCKER".into();
        prop.ptype = "locker".into();
        prop.cell = front;
        prop.offset = (0.0, 0.0);
        prop.solid = true;
        f.props.push(prop);
        let bad = validate(&f, &Fixtures::default());
        assert!(bad.iter().any(|m| m.contains("TEST-LOCKER")), "{bad:?}");
        let dropped = drop_blocked(&mut f);
        assert_eq!(dropped.props, 1);
        assert!(f.props.iter().all(|p| p.id != "TEST-LOCKER"));
        assert!(f.props.iter().any(|p| p.door.is_some()), "room doors stay");
    }

    #[test]
    fn a_floor_with_no_door_props_reads_its_room_openings() {
        let f = Facility::generate("DOORCLEAR-OPENINGS", &Options::default(), cat());
        assert!(f.props.iter().all(|p| p.door.is_none()), "the finite generator has no door props");
        let sites = sites_of(&f);
        assert!(!sites.is_empty());
        for s in &sites {
            assert!(s.span() >= 1 && s.span() <= MAX_OPENING as i32);
            assert!(s.cells.iter().all(|c| !f.is_wall_cell(c.x, c.y)));
        }
    }

    #[test]
    fn a_solid_prop_on_a_doorway_approach_is_taken_off_and_the_approach_reopens() {
        let mut f = Facility::generate("DOORCLEAR-APPROACH", &Options::default(), cat());
        let open_approach = |f: &Facility| {
            sites_of(f).iter().flat_map(|s| (0..s.cells.len()).flat_map(move |i| [-1, 1].map(|side| s.approach(i, side)))).find(|c| {
                !f.is_wall_cell(c.x, c.y) && !f.is_hole(c.x, c.y) && f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, DEFAULT_AGENT_RADIUS)
            })
        };
        let at = open_approach(&f).expect("a doorway with a free approach cell");
        let mut desk = f.props.iter().find(|p| p.door.is_none()).cloned().expect("a plain prop");
        desk.id = "TEST-DESK".into();
        desk.ptype = "table".into();
        desk.cell = at;
        desk.offset = (0.0, 0.0);
        desk.solid = true;
        f.props.push(desk);
        f.rebuild_grids();
        assert!(!f.can_occupy(at.x as f64 + 0.5, at.y as f64 + 0.5, DEFAULT_AGENT_RADIUS), "the desk closes the approach");
        let doors_before = f.props.iter().filter(|p| p.door.is_some()).count();
        assert!(clear_approaches(&mut f) >= 1);
        assert!(f.props.iter().all(|p| p.id != "TEST-DESK"), "the desk is taken off");
        assert_eq!(f.props.iter().filter(|p| p.door.is_some()).count(), doors_before, "room doors stay");
        assert!(f.can_occupy(at.x as f64 + 0.5, at.y as f64 + 0.5, DEFAULT_AGENT_RADIUS), "the approach is passable again");
        assert_eq!(clear_approaches(&mut f), 0, "a second pass finds nothing to take off");
    }

    #[test]
    fn placement_never_closes_an_approach_so_the_floor_is_not_changed_after() {
        // the placement pass skips a prop that would close a doorway's approach: a floor it built has nothing for
        // `clear_approaches` to take off, so the navigation floor is the one placement made (NAV-2 included)
        for n in 0..24 {
            let mut f = Facility::generate(&format!("DOORCLEAR-PLACE-{n}"), &Options::default(), cat());
            assert_eq!(clear_approaches(&mut f), 0, "floor {n}: a prop closed a doorway's approach at placement");
        }
        let mut nav2 = Facility::generate("NAV-2", &Options::default(), cat());
        assert_eq!(clear_approaches(&mut nav2), 0, "NAV-2: nothing is taken off the default floor after placement");
    }
}
