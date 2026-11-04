//! The generation contract of the continuous-levels project (docs/STAIRS_CONTINUOUS_DESIGN.md, section 7, task 2).
//!
//! Every connection between two levels must leave the two `Facility` objects in agreement: the same x/z footprint of
//! the well or shaft, an open landing on the upper level, an open foot on the lower level, and nothing solid (wall,
//! solid prop, water) in the footprint. A ladder also needs one clear step cell beside the hatch (decision D4).
//!
//! The tests walk the golden seeds plus a property sweep. Chunk-local cells of the same column (cx, cy) line up on
//! every level (all chunks are CHUNK x CHUNK, see `infinite::window`), so the two levels are compared chunk against
//! chunk. Nothing here is called by the running game.
//!
//! Each connection kind is its own test. A kind that fails today is `#[ignore]`d with the reason; the failure message
//! lists the failing seeds and chunks. Later phases un-ignore them one by one. This file was written in a stage where
//! cargo was not run, so every ignored kind says what is expected to fail and must be verified once: run
//! `cargo test --lib stack_contract -- --ignored` and un-ignore each test that passes.
#![cfg(test)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use super::catalog::Catalog;
use super::facility::{stair_well_cells, Cell, DeckKind, Facility, Stair, STAIR_FLIGHT};
use super::infinite::{chunk, ChunkKey, CHUNK};
use super::route::layout::Coord;
use super::route::sewer;
use super::vertical::stair_surface_y;
use crate::content::loot::LootTables;

/// The seeds every contract is checked on: the golden ones first, then a property sweep.
const GOLDEN: [&str; 3] = ["1426541270", "k7qxm", "demo"];
const SWEEP: [&str; 12] = [
    "4157433391", "2899547009", "40490470", "7", "12345", "alpha", "beta", "gamma", "delta", "123456789", "987654321",
    "zz-top",
];
/// Levels with a connection to the one below them (the Inferno, -3, is the bottom).
const UPPER_LEVELS: [i32; 3] = [0, -1, -2];

fn cat() -> &'static Catalog {
    static CAT: OnceLock<Catalog> = OnceLock::new();
    CAT.get_or_init(Catalog::load)
}

/// One built chunk plus the water cells of its sewer grid (empty off the sewer levels).
struct Built {
    f: Facility,
    water: HashSet<Cell>,
}

type Key = (String, i32, i32, i32);

fn built(seed: &str, cx: i32, cy: i32, level: i32) -> Arc<Built> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Arc<Built>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: Key = (seed.to_string(), cx, cy, level);
    if let Some(b) = cache.lock().unwrap().get(&key) {
        return b.clone();
    }
    // built outside the lock, so parallel tests do not wait on each other (a duplicate build is harmless)
    let (f, _) = chunk(seed, ChunkKey { cx, cy, level }, cat(), &LootTables::default());
    let mut water = HashSet::new();
    if level == sewer::core::LEVEL || level == sewer::core::LOWER_LEVEL {
        if let Some(sc) = sewer::chunk(Coord::new(cx, level, cy), &sewer::Inputs { world: seed }) {
            for (y, row) in sc.sewer_grid.iter().enumerate() {
                for (x, code) in row.iter().enumerate() {
                    if *code == sewer::core::WATER {
                        water.insert(Cell::new(x as i32, y as i32));
                    }
                }
            }
        }
    }
    let b = Arc::new(Built { f, water });
    cache.lock().unwrap().insert(key, b.clone());
    b
}

/// Every (seed, cx, cy) of the sweep: the 3x3 window around the golden seeds' origin and around a per-seed centre
/// for the property seeds.
fn columns() -> Vec<(String, i32, i32)> {
    let mut out = vec![];
    let seeds = GOLDEN.iter().map(|s| (*s, (0, 0))).chain(
        SWEEP.iter().enumerate().map(|(i, s)| (*s, ((i as i32 * 3) % 7 - 3, (i as i32 * 5) % 7 - 3))),
    );
    for (seed, c) in seeds {
        for dy in -1..=1 {
            for dx in -1..=1 {
                out.push((seed.to_string(), c.0 + dx, c.1 + dy));
            }
        }
    }
    out
}

/// Why `c` cannot be part of an open footprint of `b`, or `None` when it is clear: not a wall, no solid prop on it
/// (`can_occupy` tests the walls, props and lockers around the centre), no water. There is no separate pillar in the
/// facility: a pillar is a wall cell or a solid prop, which these two tests cover.
fn blocked(b: &Built, c: Cell) -> Option<&'static str> {
    if b.f.is_wall_cell(c.x, c.y) {
        return Some("wall");
    }
    if !b.f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.05) {
        return Some("solid prop");
    }
    if b.water.contains(&c) {
        return Some("water");
    }
    None
}

/// A cell a body can stand on: clear and not a hole.
fn stand_ok(b: &Built, c: Cell) -> bool {
    blocked(b, c).is_none() && !b.f.is_hole(c.x, c.y) && b.f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.23)
}

/// Whether one of the four cells beside `c` is a clear standing cell (the ladder's step cell).
fn has_step_cell(b: &Built, c: Cell) -> bool {
    [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| stand_ok(b, Cell::new(c.x + dx, c.y + dy)))
}

fn footprint(s: Stair) -> BTreeSet<Cell> {
    (1..=STAIR_FLIGHT).flat_map(|k| stair_well_cells(s, k)).collect()
}

/// Fails the test with the failing seeds and chunks (at most 40 shown, all counted).
fn report(kind: &str, fails: &[String]) {
    if fails.is_empty() {
        return;
    }
    let shown: Vec<&String> = fails.iter().take(40).collect();
    panic!("{kind}: {} failures (seed cx,cy level: reason): {:#?}", fails.len(), shown);
}

/// A maze or sewer stairwell seen from both levels.
struct StairPair {
    tag: String,
    s: Stair,
    /// The lower level's flight that meets `s`: the up flight whose landing is nearest to the far end.
    t: Option<Stair>,
    up: Arc<Built>,
    lo: Arc<Built>,
}

fn stair_pairs() -> Vec<StairPair> {
    let mut out = vec![];
    for (seed, cx, cy) in columns() {
        for u in UPPER_LEVELS {
            let up = built(&seed, cx, cy, u);
            let lo = built(&seed, cx, cy, u - 1);
            for s in up.f.stairs.iter().filter(|s| s.dir < 0 && !s.is_ladder()) {
                let far = Cell::new(s.cell.x + s.run.0 * (STAIR_FLIGHT + 1), s.cell.y + s.run.1 * (STAIR_FLIGHT + 1));
                let t = lo
                    .f
                    .stairs
                    .iter()
                    .filter(|t| t.dir > 0 && !t.is_ladder())
                    .min_by_key(|t| (t.cell.x - far.x).abs() + (t.cell.y - far.y).abs())
                    .copied();
                out.push(StairPair { tag: format!("{seed} {cx},{cy} L{u}"), s: *s, t, up: up.clone(), lo: lo.clone() });
            }
        }
    }
    out
}

/// A sewer ladder seen from both levels (level 0 hatch, level -1 foot).
struct LadderPair {
    tag: String,
    s: Stair,
    t: Option<Stair>,
    up: Arc<Built>,
    lo: Arc<Built>,
}

fn ladder_pairs() -> Vec<LadderPair> {
    let mut out = vec![];
    for (seed, cx, cy) in columns() {
        let up = built(&seed, cx, cy, 0);
        let lo = built(&seed, cx, cy, -1);
        for s in up.f.stairs.iter().filter(|s| s.dir < 0 && s.is_ladder()) {
            let t = lo.f.stairs.iter().find(|t| t.dir > 0 && t.is_ladder() && t.cell == s.cell).copied();
            out.push(LadderPair { tag: format!("{seed} {cx},{cy} L0"), s: *s, t, up: up.clone(), lo: lo.clone() });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// The sweep itself
// ---------------------------------------------------------------------------------------------------------------------

/// Not a contract: the sweep must actually find connections, or every test below passes by looking at nothing.
#[test]
fn the_sweep_finds_stairs_ladders_and_builds_every_level() {
    let pairs = stair_pairs();
    assert!(pairs.len() >= 20, "only {} stairwells in the sweep", pairs.len());
    for l in UPPER_LEVELS {
        assert!(pairs.iter().any(|p| p.tag.ends_with(&format!("L{l}"))), "no stairwell down from level {l}");
    }
    assert!(!ladder_pairs().is_empty(), "no sewer ladder in the sweep");
    for seed in GOLDEN {
        for level in [0, -1, -2, -3] {
            let b = built(seed, 0, 0, level);
            assert_eq!((b.f.cols, b.f.rows), (CHUNK, CHUNK), "{seed} level {level}");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Maze and sewer stairwells
// ---------------------------------------------------------------------------------------------------------------------

/// Both levels list a flight over the same x/z cells (the 2x4 well).
#[test]
fn stairwells_share_one_footprint() {
    let mut fails = vec![];
    for p in stair_pairs() {
        match p.t {
            None => fails.push(format!("{}: no up flight below", p.tag)),
            Some(t) if footprint(t) != footprint(p.s) => {
                fails.push(format!("{}: well {:?} above, {:?} below", p.tag, footprint(p.s), footprint(t)))
            }
            _ => {}
        }
    }
    report("stairwell footprint", &fails);
}

#[test]
#[ignore = "contract not met yet: the upper landing is the doorway cell and the lower foot is the cell past the well; margins and landing clearance are not generated from one footprint (phase 1/Task 4)"]
fn stairwells_have_an_open_landing_above_and_an_open_foot_below() {
    let mut fails = vec![];
    for p in stair_pairs() {
        if !stand_ok(&p.up, p.s.cell) {
            fails.push(format!("{}: upper landing {:?} not open ({:?})", p.tag, p.s.cell, blocked(&p.up, p.s.cell)));
        }
        match p.t {
            None => fails.push(format!("{}: no lower flight", p.tag)),
            Some(t) if !stand_ok(&p.lo, t.cell) => {
                fails.push(format!("{}: lower foot {:?} not open ({:?})", p.tag, t.cell, blocked(&p.lo, t.cell)))
            }
            _ => {}
        }
    }
    report("stairwell landing and foot", &fails);
}

#[test]
#[ignore = "contract not met yet: wells are carved per level (route::live::carve_stair_well, infinite bays, sewer wells) so a wall, solid prop or sewer water can sit in the shared footprint on one side (Task 4)"]
fn stairwell_footprints_hold_no_wall_prop_or_water_on_either_level() {
    let mut fails = vec![];
    for p in stair_pairs() {
        for (name, b, cells) in [("upper", &p.up, Some(footprint(p.s))), ("lower", &p.lo, p.t.map(footprint))] {
            for c in cells.unwrap_or_default() {
                if let Some(why) = blocked(b, c) {
                    fails.push(format!("{}: {name} well cell {:?}: {why}", p.tag, c));
                }
            }
        }
    }
    report("stairwell footprint clearance", &fails);
}

// ---------------------------------------------------------------------------------------------------------------------
// Sewer ladders
// ---------------------------------------------------------------------------------------------------------------------

/// The hatch and the foot are the same x/z cell on both levels (`level_links::ladders`).
#[test]
fn ladders_share_one_cell() {
    let mut fails = vec![];
    for p in ladder_pairs() {
        if p.t.is_none() {
            fails.push(format!("{}: no ladder foot on level -1 at {:?}", p.tag, p.s.cell));
        }
    }
    report("ladder footprint", &fails);
}

#[test]
#[ignore = "contract not met yet: the hatch cell is a layout hallway cell with no guaranteed clear cell beside it (decision D4), and the sewer foot opens only a 3x3 that can still hold water"]
fn ladders_have_an_open_hatch_a_step_cell_and_an_open_foot() {
    let mut fails = vec![];
    for p in ladder_pairs() {
        if blocked(&p.up, p.s.cell).is_some() {
            fails.push(format!("{}: hatch {:?} blocked ({:?})", p.tag, p.s.cell, blocked(&p.up, p.s.cell)));
        }
        if !has_step_cell(&p.up, p.s.cell) {
            fails.push(format!("{}: no clear step cell beside the hatch {:?} on level 0", p.tag, p.s.cell));
        }
        if !stand_ok(&p.lo, p.s.cell) {
            fails.push(format!("{}: ladder foot {:?} not open ({:?})", p.tag, p.s.cell, blocked(&p.lo, p.s.cell)));
        }
        if !has_step_cell(&p.lo, p.s.cell) {
            fails.push(format!("{}: no clear cell beside the ladder foot {:?} on level -1", p.tag, p.s.cell));
        }
        let _ = p.t;
    }
    report("ladder landing, step and foot", &fails);
}

#[test]
#[ignore = "contract not met yet: no generation step keeps the shaft cell clear of props and sewer water on both levels (Task 4)"]
fn ladder_shafts_hold_no_wall_prop_or_water_on_either_level() {
    let mut fails = vec![];
    for p in ladder_pairs() {
        for (name, b) in [("upper", &p.up), ("lower", &p.lo)] {
            if let Some(why) = blocked(b, p.s.cell) {
                fails.push(format!("{}: {name} shaft cell {:?}: {why}", p.tag, p.s.cell));
            }
        }
    }
    report("ladder shaft clearance", &fails);
}

// ---------------------------------------------------------------------------------------------------------------------
// Catwalk links (same world: the "upper level" is the deck)
// ---------------------------------------------------------------------------------------------------------------------

/// The ground end is open at height 0 and a deck cell at exactly the link's height is beside it.
#[test]
#[ignore = "contract not met yet: not verified when written; expected to need a clear step cell beside a ladder link and a deck landing that is not a hole or prop (Task 4)"]
fn catwalk_links_have_an_open_foot_and_a_deck_landing() {
    let mut fails = vec![];
    for (seed, cx, cy) in columns() {
        let b = built(&seed, cx, cy, 0);
        for l in &b.f.catwalk_links {
            let tag = format!("{seed} {cx},{cy} L0 link {:?}", l.cell);
            if !stand_ok(&b, l.cell) || b.f.height_at(l.cell) != 0.0 {
                fails.push(format!("{tag}: ground foot not open at height 0"));
            }
            let landing = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| {
                let c = Cell::new(l.cell.x + dx, l.cell.y + dy);
                let s = b.f.walk_surface_at(c);
                s.kind == DeckKind::Catwalk && (s.height - l.deck_height).abs() < 0.01 && !b.f.is_hole(c.x, c.y)
            });
            if !landing {
                fails.push(format!("{tag}: no deck cell at {} m beside the foot", l.deck_height));
            }
        }
    }
    report("catwalk foot and landing", &fails);
}

#[test]
#[ignore = "contract not met yet: not verified when written; a link's deck neighbours may hold a solid prop (the deck grid ignores props)"]
fn catwalk_link_footprints_hold_no_wall_prop_or_water() {
    let mut fails = vec![];
    for (seed, cx, cy) in columns() {
        let b = built(&seed, cx, cy, 0);
        for l in &b.f.catwalk_links {
            if let Some(why) = blocked(&b, l.cell) {
                fails.push(format!("{seed} {cx},{cy} L0 link {:?}: ground cell {why}", l.cell));
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let c = Cell::new(l.cell.x + dx, l.cell.y + dy);
                if b.f.walk_surface_at(c).kind == DeckKind::Catwalk {
                    if let Some(why) = blocked(&b, c) {
                        fails.push(format!("{seed} {cx},{cy} L0 link {:?}: deck cell {:?} {why}", l.cell, c));
                    }
                }
            }
        }
    }
    report("catwalk footprint clearance", &fails);
}

// ---------------------------------------------------------------------------------------------------------------------
// Waterfall drops (level -1 channel floor over a level -2 landing pool)
// ---------------------------------------------------------------------------------------------------------------------

/// Every drop cell: the upper fall cell, the same lower cell, and the falls chunk it belongs to.
fn drops() -> Vec<(String, Arc<Built>, Arc<Built>, Cell, sewer::Falls)> {
    let mut out = vec![];
    for (seed, cx, cy) in columns() {
        let inputs = sewer::Inputs { world: &seed };
        let l2 = sewer::level2::chunk(&inputs, cx, cy);
        if l2.drops.is_empty() {
            continue;
        }
        let Some(falls) = sewer::falls_between(&inputs, cx, cy) else { continue };
        let up = built(&seed, cx, cy, -1);
        let lo = built(&seed, cx, cy, -2);
        for d in &l2.drops {
            out.push((format!("{seed} {cx},{cy} L-1"), up.clone(), lo.clone(), Cell::new(d.upper_cell.x, d.upper_cell.y), falls.clone()));
        }
    }
    out
}

/// The upper cell is a fall void and the lower cell is open pool water: the shaft is the same x/z on both levels.
#[test]
fn drops_share_one_cell_over_a_landing_pool() {
    let mut fails = vec![];
    for (tag, up, lo, c, falls) in drops() {
        let void = falls.void;
        let inside = c.x >= void.x && c.x < void.x + void.w && c.y >= void.y && c.y < void.y + void.h;
        if !inside {
            fails.push(format!("{tag}: drop cell {c:?} is outside the falls void"));
        }
        if lo.f.is_wall_cell(c.x, c.y) {
            fails.push(format!("{tag}: pool cell {c:?} is a wall on level -2"));
        }
        if !lo.water.contains(&c) {
            fails.push(format!("{tag}: no pool water under the drop {c:?} on level -2"));
        }
        let _ = up;
    }
    report("drop footprint", &fails);
}

#[test]
#[ignore = "contract not met yet: the upper fall cells are WALL cells in the Facility (apply_sewer_floor treats FALL as solid), so the channel floor has no opening, and no landing is guaranteed beside the void"]
fn drops_have_an_open_upper_channel_and_a_clear_pool_below() {
    let mut fails = vec![];
    for (tag, up, lo, c, falls) in drops() {
        if up.f.is_wall_cell(c.x, c.y) {
            fails.push(format!("{tag}: upper drop cell {c:?} is a wall, not an opening"));
        }
        if let Some(why) = blocked(&up, c).filter(|w| *w != "wall") {
            fails.push(format!("{tag}: upper drop cell {c:?}: {why}"));
        }
        if !lo.f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.05) {
            fails.push(format!("{tag}: solid prop in the pool at {c:?}"));
        }
        // an open landing to jump from: a clear cell of the spur or the overlook on the upper level
        let open_edge = [falls.spur, falls.overlook].iter().any(|r| {
            (r.y..r.y + r.h).any(|y| (r.x..r.x + r.w).any(|x| stand_ok(&up, Cell::new(x, y))))
        });
        if !open_edge {
            fails.push(format!("{tag}: no open cell on the spur or overlook above {c:?}"));
        }
    }
    report("drop opening and landing", &fails);
}

// ---------------------------------------------------------------------------------------------------------------------
// The TODO seed
// ---------------------------------------------------------------------------------------------------------------------

/// TODO.md: "Stairwell with only 2 walk rows at seed 40490470, chunk (3,2): check that the flight is walkable end to
/// end". Every stair of that level 0 chunk: the stairwell's walk cells and the landing are open, both lanes of every
/// flight cell have a tread surface that never turns back, a body fits on each, and the landing is reachable.
#[test]
#[ignore = "contract not met yet: not verified when written; TODO.md reports this stairwell has only 2 walk rows, so a flight cell may be walled or have no tread surface"]
fn the_todo_stairwell_at_seed_40490470_chunk_3_2_is_walkable_end_to_end() {
    let (seed, cx, cy) = ("40490470", 3, 2);
    let b = built(seed, cx, cy, 0);
    let mut fails = vec![];
    for well in super::route::live::layout_stairwells(seed, ChunkKey { cx, cy, level: 0 }) {
        for w in &well.walk {
            if b.f.is_wall_cell(w.x, w.y) {
                fails.push(format!("walk cell ({}, {}) of the stairwell is walled", w.x, w.y));
            }
        }
    }
    let flights: Vec<&Stair> = b.f.stairs.iter().filter(|s| !s.is_ladder()).collect();
    if flights.is_empty() {
        fails.push("the chunk has no stair flight".to_string());
    }
    for s in flights {
        if !stand_ok(&b, s.cell) {
            fails.push(format!("landing {:?} is not open", s.cell));
        }
        if b.f.hunter_open(b.f.start) && b.f.hunter_open(s.cell) && b.f.find_path(b.f.start, s.cell).is_empty() {
            fails.push(format!("landing {:?} is not reachable from the chunk start {:?}", s.cell, b.f.start));
        }
        let mut last = 0.0f32;
        for k in 1..=STAIR_FLIGHT {
            for c in stair_well_cells(*s, k) {
                if b.f.is_wall_cell(c.x, c.y) {
                    fails.push(format!("flight cell {c:?} (step {k}) is a wall"));
                    continue;
                }
                let (x, y) = c.center();
                match stair_surface_y(*s, (x, y)) {
                    None => fails.push(format!("flight cell {c:?} (step {k}) has no tread surface")),
                    Some(h) => {
                        if h.abs() + 1e-3 < last.abs() {
                            fails.push(format!("tread height turns back at {c:?} (step {k}): {h} after {last}"));
                        }
                        last = last.abs().max(h.abs()) * h.signum();
                    }
                }
                if !b.f.body_fits(x, y, 0.2) {
                    fails.push(format!("a body does not fit on flight cell {c:?} (step {k})"));
                }
            }
        }
        // the far end of the run, where the other level takes over, must not be a wall either
        let far = Cell::new(s.cell.x + s.run.0 * (STAIR_FLIGHT + 1), s.cell.y + s.run.1 * (STAIR_FLIGHT + 1));
        if b.f.is_wall_cell(far.x, far.y) {
            fails.push(format!("the far end {far:?} of the flight is a wall"));
        }
    }
    report("seed 40490470 chunk (3,2)", &fails);
}
