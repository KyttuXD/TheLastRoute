//! Generator-level validators for every stair (spirals above all): over many seeds and every level pair, the cells a
//! runner uses to get onto and off a flight are never taken by furniture, and a body can walk the whole way through
//! them. The walk down the flight itself and the descent into the Inferno are `game/spiral_stairs_tests.rs`.
//!
//! The zone of a flight on one level is its landing or foot, the clear margin and landing row behind it
//! (`Footprint::upper_clear` / `lower_clear`) and the RING: every open cell touching the well (the outer edge of the
//! helicoid, where a prop would pin a runner against the drop). A prop counts when its footprint (centre cell plus
//! offset, half extents) overlaps the interior of a zone cell, not only when its centre cell is inside.

use super::catalog::Catalog;
use super::facility::{Cell, Facility, LinkKind, Prop, Stair};
use super::flight::{well_footprint, FlightShape};
use crate::audio::Sounds;
use crate::config::{Difficulty, PLAYER_RADIUS};
use crate::content::Content;
use crate::game::level_stack::floor_y_of;
use crate::game::{Controls, Game};
use std::collections::HashSet;

/// Whether the solid footprint of `p` overlaps the interior of `c`.
fn prop_hits(p: &Prop, c: Cell) -> bool {
    let (px, py) = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
    (c.x as f32 + 0.5 - px).abs() < p.half.0 + 0.5 - 1e-3 && (c.y as f32 + 0.5 - py).abs() < p.half.1 + 0.5 - 1e-3
}

/// The cells a runner needs on one level's side of a flight, and the ring round its well.
pub(crate) struct Zones {
    pub zone: Vec<Cell>,
    pub ring: Vec<Cell>,
}

pub(crate) fn zones_of(f: &Facility, s: &Stair) -> Zones {
    let fp = well_footprint(&s.flight(f.level).expect("a stair has a flight"));
    let mut zone: Vec<Cell> = if s.dir < 0 { fp.upper_clear.clone() } else { fp.lower_clear.clone() };
    zone.push(if s.dir < 0 { fp.landing } else { fp.foot });
    zone.sort();
    zone.dedup();
    zone.retain(|c| !f.is_wall_cell(c.x, c.y) && !fp.upper_hole.contains(c));
    let mut ring: Vec<Cell> = vec![];
    for h in &fp.upper_hole {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let r = Cell::new(h.x + dx, h.y + dy);
                if !fp.upper_hole.contains(&r) && !f.is_wall_cell(r.x, r.y) && !ring.contains(&r) {
                    ring.push(r);
                }
            }
        }
    }
    ring.sort();
    Zones { zone, ring }
}

/// Every window of `seeds` seeds on `level` (centred by one step), calling `each` with it.
fn each_window(level: i32, seeds: usize, mut each: impl FnMut(&str, &Facility)) {
    let (c, cat) = (Content::load(), Catalog::load());
    for k in 0..seeds {
        let seed = format!("ROOM-SPIRAL-{k}");
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &seed, level, &c, &cat);
        g.step(1.0 / 60.0, &Controls::default(), &c, &cat, &Sounds::default());
        each(&seed, &g.fac);
    }
}

const LEVELS: [i32; 4] = [0, -1, -2, -3];

/// No solid prop reaches a zone or ring cell and no locker stands in one, for every flight of every shape on every level.
#[test]
fn no_prop_or_locker_reaches_the_zone_or_ring_of_any_stair() {
    let mut checked = 0;
    let mut bad: Vec<String> = vec![];
    for level in LEVELS {
        each_window(level, 40, |seed, f| {
            for s in f.stairs.iter().filter(|s| matches!(s.kind, LinkKind::Stairs)) {
                checked += 1;
                let z = zones_of(f, s);
                for (what, cells) in [("zone", &z.zone), ("ring", &z.ring)] {
                    for c in cells {
                        for p in f.props.iter().filter(|p| p.solid && p.door.is_none()) {
                            if prop_hits(p, *c) {
                                bad.push(format!(
                                    "L{level} {seed} {:?} dir {}: {} reaches {what} cell {c:?} (centre {:?} half {:?})",
                                    s.shape, s.dir, p.ptype, p.cell, p.half
                                ));
                            }
                        }
                        if f.hiding.iter().any(|h| h.cell == *c) {
                            bad.push(format!("L{level} {seed} {:?}: a locker stands in {what} cell {c:?}", s.shape));
                        }
                    }
                }
            }
        });
    }
    assert!(checked > 100, "only {checked} flights were checked");
    assert!(bad.is_empty(), "{} blocked cells, first ones:\n{}", bad.len(), bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// A body of the player's radius can walk the lattice (0.25 m, eight directions) from the far end of the clear margin
/// to the landing on the upper level, and from the foot to the far end of the margin on the lower level, staying on
/// the zone and ring cells. This is the "can I get on and off the stairs" test, for every spiral.
#[test]
fn a_body_walks_from_the_margin_to_the_landing_and_from_the_foot_out_for_every_spiral() {
    let mut walked = 0;
    let mut bad: Vec<String> = vec![];
    for level in LEVELS {
        each_window(level, 40, |seed, f| {
            for s in f.stairs.iter().filter(|s| matches!(s.kind, LinkKind::Stairs) && s.shape == FlightShape::Spiral) {
                let z = zones_of(f, s);
                let fp = well_footprint(&s.flight(f.level).unwrap());
                let end = if s.dir < 0 { fp.landing } else { fp.foot };
                let far = z.zone.iter().copied().max_by_key(|c| (c.x - end.x).abs() + (c.y - end.y).abs()).unwrap();
                walked += 1;
                if !lattice_path(f, &z, end, far) {
                    bad.push(format!("L{level} {seed} spiral dir {} A {:?}: no body path between {end:?} and {far:?}", s.dir, fp.landing));
                }
            }
        });
    }
    assert!(walked >= 40, "only {walked} spirals were checked");
    assert!(bad.is_empty(), "{} spirals have no clear way on or off, first ones:\n{}", bad.len(), bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// Breadth-first over the 0.25 m lattice of the zone and ring cells: whether a body gets from the centre of `a` to the
/// centre of `b`.
fn lattice_path(f: &Facility, z: &Zones, a: Cell, b: Cell) -> bool {
    const STEP: f32 = 0.25;
    let allowed: HashSet<Cell> = z.zone.iter().chain(z.ring.iter()).copied().collect();
    let key = |x: f32, y: f32| ((x / STEP).round() as i32, (y / STEP).round() as i32);
    let fits = |x: f32, y: f32| {
        allowed.contains(&Cell::new(x.floor() as i32, y.floor() as i32))
            && f.body_fits_at_y(x, y, floor_y_of(f.level) + f.ground_height(x, y), PLAYER_RADIUS)
    };
    let (sa, sb) = (a.center(), b.center());
    if !fits(sa.0, sa.1) || !fits(sb.0, sb.1) {
        return false;
    }
    let goal = key(sb.0, sb.1);
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut stack = vec![key(sa.0, sa.1)];
    seen.insert(stack[0]);
    while let Some((i, j)) = stack.pop() {
        if (i, j) == goal {
            return true;
        }
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let n = (i + di, j + dj);
            if seen.contains(&n) {
                continue;
            }
            if fits(n.0 as f32 * STEP, n.1 as f32 * STEP) {
                seen.insert(n);
                stack.push(n);
            }
        }
    }
    false
}

/// The Inferno's caves know nothing of the flight that lands in them: 96 of 511 bays (84 spirals among them) had lava
/// cells inside the box the build opens for the runner, 20 under the foot, the landing or its margin. The box is now
/// walkable, lava-free and without cave decoration on every chunk of every seed, and the build opens all of it.
#[test]
fn the_inferno_stair_bay_has_no_lava_and_no_cave_decoration() {
    use super::flight_links::{bay_flight, bay_rect};
    use super::infinite::{chunk, ChunkKey, INFERNO_LEVEL};
    use super::inferno::InfernoChunk;
    let (cat, loot) = (Catalog::load(), crate::content::loot::LootTables::default());
    let (mut bays, mut spirals) = (0, 0);
    for n in 0..30 {
        let seed = format!("ROOM-PROBE-{n}");
        for cx in -2..=2 {
            for cy in -2..=2 {
                let k = ChunkKey { cx, cy, level: INFERNO_LEVEL };
                let Some(flight) = bay_flight(&seed, cx, cy, INFERNO_LEVEL + 1) else { continue };
                bays += 1;
                spirals += (flight.shape == FlightShape::Spiral) as usize;
                let ch = InfernoChunk::carve(&seed, k);
                let (x, y, w, h) = bay_rect(&flight);
                assert_eq!(ch.bay.map(|r| (r.x, r.y, r.w, r.h)), Some((x, y, w, h)), "{seed} {cx},{cy}: the chunk knows its bay");
                let open = ch.open_cells();
                for cy2 in y..y + h {
                    for cx2 in x..x + w {
                        let c = Cell::new(cx2, cy2);
                        assert!(!ch.is_lava(c), "{seed} {cx},{cy}: lava at {c:?} in the stair bay");
                        assert!(open.contains(&c), "{seed} {cx},{cy}: bay cell {c:?} is not walkable");
                    }
                }
                for d in crate::render::inferno_deco::inferno_deco().place(&seed, k, &ch) {
                    assert!(!ch.in_bay(d.cell), "{seed} {cx},{cy}: a {:?} stands at {:?} in the stair bay", d.kind, d.cell);
                }
                // the facility the game builds opens the same cells
                if cx == 0 && cy == 0 {
                    let (f, _) = chunk(&seed, k, &cat, &loot);
                    for cy2 in y..y + h {
                        for cx2 in x..x + w {
                            assert!(!f.is_wall_cell(cx2, cy2), "{seed}: bay cell {cx2},{cy2} is a wall");
                        }
                    }
                }
            }
        }
    }
    assert!(bays > 100 && spirals > 10, "{bays} bays, {spirals} spirals");
}

/// Sweeping a table by its footprint takes what stands on it too: a mounted prop whose centre is over a swept prop's
/// footprint must not hang in the air, while one on a prop that stays is kept.
#[test]
fn sweeping_a_wide_prop_by_reach_takes_its_tabletop_items_with_it() {
    let (c, cat) = (Content::load(), Catalog::load());
    let g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-SPIRAL-0", 0, &c, &cat);
    let mut f = g.fac.clone();
    let template = f.props.iter().find(|p| p.door.is_none() && p.sewer.is_none()).expect("a prop").clone();
    f.props.clear();
    let table = Prop { id: "t".into(), cell: Cell::new(10, 10), ptype: "table".into(), solid: true, half: (2.0, 0.5), offset: (0.0, 0.0), mount_y: None, ..template.clone() };
    // on the table's far end, outside the swept rectangle (x 8..10): centre cell 12, over the table's footprint
    let item = Prop { id: "i".into(), cell: Cell::new(12, 10), ptype: "lamp".into(), solid: false, half: (0.1, 0.1), offset: (0.0, 0.0), mount_y: Some(0.8), ..template.clone() };
    let stay = Prop { id: "s".into(), cell: Cell::new(20, 20), ptype: "lamp".into(), solid: false, half: (0.1, 0.1), offset: (0.0, 0.0), mount_y: Some(0.8), ..template };
    f.props.extend([table, item, stay]);
    f.clear_props_reaching(8, 9, 2, 3);
    let ids: Vec<&str> = f.props.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["s"], "the table reaches the rectangle and goes, its lamp with it; the far lamp stays");
}
