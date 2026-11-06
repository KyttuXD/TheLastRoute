//! The sewer's channel railings as walls, on real generated chunks: the runner, the bots and the hunters cannot walk
//! through one, no route crosses one, the deliberate gap stays open, and a body that landed in a fenced pool can walk out.
//! The player's step is tested in `game/player.rs` and the bots' sweep in `game/npc.rs` (both private to them); the
//! synthetic walkway and the shape of the wall are tested in `world/facility.rs`.

use crate::config::{Difficulty, HUNTER_RADIUS, PLAYER_RADIUS};
use crate::content::loot::LootTables;
use crate::hunters::brain::{cell_of, Brain};
use crate::world::catalog::Catalog;
use crate::world::facility::{rail_side_bit, Cell, Facility, RAIL_E, RAIL_N, RAIL_S, RAIL_W};
use crate::world::infinite::{chunk, ChunkKey};
use crate::world::route::layout::Coord;
use crate::world::route::sewer::core::{Grid, BRIDGE, DRY, LEVEL, LOWER_LEVEL, WATER};
use crate::world::route::sewer::{self, Inputs};
use std::sync::OnceLock;

/// One real sewer chunk: its facility (railings set), its grid and the cell the railings leave open for the jump.
pub(crate) struct Sample {
    pub name: String,
    pub fac: Facility,
    pub grid: Grid,
    pub gap: Option<Cell>,
    /// The cells of the landing pool under the falls (the water a jump drops the runner into).
    pub pool: Vec<Cell>,
}

/// The sewer chunks the tests walk: a few seeds, the chunks round the origin, both sewer storeys.
pub(crate) fn samples() -> &'static [Sample] {
    static SAMPLES: OnceLock<Vec<Sample>> = OnceLock::new();
    SAMPLES.get_or_init(|| {
        let (cat, loot) = (Catalog::load(), LootTables::default());
        let mut out = vec![];
        for seed in ["ROOM-RAILS-1", "ROOM-RAILS-2", "ROOM-RAILS-3"] {
            for level in [LEVEL, LOWER_LEVEL] {
                for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (-1, -1), (2, 0), (0, 2)] {
                    let Some(sc) = sewer::chunk(Coord::new(cx, level, cy), &Inputs { world: seed }) else { continue };
                    let (fac, _) = chunk(seed, ChunkKey { cx, cy, level }, &cat, &loot);
                    let gap = sc.falls.as_ref().map(|f| Cell::new(f.jump.cell.x, f.jump.cell.y));
                    let pool = sc
                        .falls
                        .as_ref()
                        .map(|f| {
                            let p = f.pool;
                            (p.y..p.y + p.h).flat_map(|y| (p.x..p.x + p.w).map(move |x| Cell::new(x, y))).collect()
                        })
                        .unwrap_or_default();
                    out.push(Sample { name: format!("{seed} L{level} {cx},{cy}"), fac, grid: sc.sewer_grid, gap, pool });
                }
            }
        }
        out
    })
}

const SIDES: [u8; 4] = [RAIL_N, RAIL_E, RAIL_S, RAIL_W];

fn step_of(bit: u8) -> (i32, i32) {
    [(0, -1), (1, 0), (0, 1), (-1, 0)][SIDES.iter().position(|b| *b == bit).unwrap()]
}

/// Every railing as (walkway cell, side bit, the cell it faces).
fn edges(s: &Sample) -> Vec<(Cell, u8, Cell)> {
    s.fac
        .rail_edges()
        .iter()
        .map(|(c, bit)| {
            let st = step_of(*bit);
            (*c, *bit, Cell::new(c.x + st.0, c.y + st.1))
        })
        .collect()
}

fn aki() -> crate::content::hunters::HunterDef {
    let v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
    crate::content::hunters::parse(&v, None, "").unwrap()
}

#[test]
fn the_facility_railings_are_the_drawn_segments() {
    let mut total = 0;
    for s in samples() {
        let drawn = crate::render::sewer_rails::channel_rail_segments(&s.grid, s.gap.map(|g| (g.x, g.y)));
        let mut want: Vec<(Cell, u8)> = drawn.iter().map(|g| (Cell::new(g.x, g.z), rail_side_bit(g.step))).collect();
        want.sort();
        want.dedup();
        assert_eq!(s.fac.rail_edges(), &want[..], "{}", s.name);
        total += want.len();
    }
    assert!(total > 100, "the sample chunks have water edges to fence ({total})");
}

#[test]
fn a_body_cannot_be_carried_through_a_railing_from_the_walkway() {
    let mut checked = 0;
    for s in samples() {
        for (c, bit, w) in edges(s) {
            // a runner or a hunter standing in the middle of the walkway cell, pushed hard at the water
            for r in [PLAYER_RADIUS, HUNTER_RADIUS] {
                let (cx, cz) = c.center();
                if !s.fac.body_fits(cx, cz, r) {
                    continue;
                }
                let st = step_of(bit);
                let to = (cx + st.0 as f32 * 2.0, cz + st.1 as f32 * 2.0);
                let end = s.fac.clear_toward((cx, cz), to, r);
                assert_eq!(cell_of(end), c, "{}: {c:?} -> {w:?} r {r}: stopped at {end:?}", s.name);
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "{checked}");
}

#[test]
fn no_route_of_the_hunters_grid_crosses_a_railing() {
    let mut checked = 0;
    for s in samples() {
        for (c, _, w) in edges(s) {
            assert!(!s.fac.hunter_open(w), "{}: the cell behind the rail at {c:?} is open to a route", s.name);
            assert!(s.fac.find_path(c, w).is_empty(), "{}: a route from {c:?} into {w:?}", s.name);
            checked += 1;
        }
        // the walkway itself is not closed by its own railings
        for (c, _, _) in edges(s) {
            if s.fac.body_fits(c.center().0, c.center().1, HUNTER_RADIUS) && !s.fac.is_drop(c) {
                assert!(s.fac.hunter_open(c), "{}: the walkway cell {c:?} is closed", s.name);
            }
        }
    }
    assert!(checked > 100);
}

#[test]
fn a_hunter_pushed_at_the_water_stays_on_the_walkway() {
    let mut checked = 0;
    for s in samples().iter().take(6) {
        for (c, bit, _) in edges(s).into_iter().step_by(7) {
            let (cx, cz) = c.center();
            if !s.fac.body_fits(cx, cz, HUNTER_RADIUS) {
                continue;
            }
            let st = step_of(bit);
            let mut b = Brain::new(aki(), (cx, cz), "RAIL-HUNTER", Difficulty::Standard);
            for _ in 0..40 {
                b.step_toward((cx + st.0 as f32 * 3.0, cz + st.1 as f32 * 3.0), 0.2, &s.fac, 0.0);
            }
            assert_eq!(cell_of(b.pos), c, "{}: the hunter went through the rail at {c:?} to {:?}", s.name, b.pos);
            assert!(s.fac.body_fits(b.pos.0, b.pos.1, HUNTER_RADIUS));
            checked += 1;
        }
    }
    assert!(checked > 10, "{checked}");
}

#[test]
fn the_deliberate_gap_stays_open_on_both_storeys() {
    let mut falls = 0;
    for s in samples() {
        let Some(g) = s.gap else { continue };
        falls += 1;
        assert!(edges(s).iter().all(|(_, _, w)| *w != g), "{}: a rail faces the jump cell {g:?}", s.name);
        // every walkway cell beside the jump cell can step into it
        let mut open = 0;
        for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
            let d = Cell::new(g.x + dx, g.y + dy);
            let code = s.grid[d.y as usize][d.x as usize];
            if code != DRY && code != BRIDGE {
                continue;
            }
            assert!(!s.fac.rail_between(d, g), "{}: a rail between {d:?} and the jump cell", s.name);
            let (dc, gc) = (d.center(), g.center());
            if s.fac.body_fits(dc.0, dc.1, PLAYER_RADIUS) {
                let end = s.fac.clear_toward(dc, gc, PLAYER_RADIUS);
                assert_eq!(end, gc, "{}: the way into the jump cell {g:?} from {d:?} is blocked", s.name);
                open += 1;
            }
        }
        assert!(open > 0, "{}: no walkway cell reaches the jump cell", s.name);
    }
    assert!(falls > 0, "no sample chunk has falls");
}

#[test]
fn a_body_in_a_fenced_pool_can_walk_out_of_it() {
    // the water of a falls chunk's lower storey is fenced all round but the gap: a body that lands in it (or wades in
    // through the gap above) must be able to walk back out to a dry cell
    let mut pools = 0;
    for s in samples() {
        let wet: Vec<Cell> = s.pool.iter().copied().filter(|c| s.grid[c.y as usize][c.x as usize] == WATER).collect();
        for start in wet.into_iter().step_by(5) {
            let (sc, oc) = start.center();
            if !s.fac.body_fits(sc, oc, PLAYER_RADIUS) {
                continue;
            }
            let mut seen = std::collections::HashSet::from([start]);
            let mut q = std::collections::VecDeque::from([start]);
            let mut out = false;
            while let Some(c) = q.pop_front() {
                let code = s.grid[c.y as usize][c.x as usize];
                if code == DRY || code == BRIDGE {
                    out = true;
                    break;
                }
                for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                    let n = Cell::new(c.x + dx, c.y + dy);
                    if n.x < 0 || n.y < 0 || n.x >= s.grid.len() as i32 || n.y >= s.grid.len() as i32 || seen.contains(&n) {
                        continue;
                    }
                    let (nx, ny) = n.center();
                    if s.fac.rail_between(c, n) || !s.fac.body_fits(nx, ny, PLAYER_RADIUS) {
                        continue;
                    }
                    seen.insert(n);
                    q.push_back(n);
                }
            }
            assert!(out, "{}: water at {start:?} is walled in by the railings", s.name);
            pools += 1;
        }
    }
    assert!(pools > 0);
}

#[test]
fn stairs_and_ladders_stay_open_past_the_railings() {
    // a railing stands only on the water's edge, never across a flight's own run: the landing and the foot of every
    // stair and ladder step on and off it along its axis
    let mut checked = 0;
    for s in samples() {
        for st in &s.fac.stairs {
            let run = if st.run == (0, 0) { (0, -1) } else { st.run };
            for c in [st.landing(), st.foot()] {
                for sign in [1, -1] {
                    let n = Cell::new(c.x + sign * run.0, c.y + sign * run.1);
                    let (cc, nc) = (c.center(), n.center());
                    if n.x < 0 || n.y < 0 || n.x >= s.fac.cols || n.y >= s.fac.rows {
                        continue;
                    }
                    if s.fac.body_fits(cc.0, cc.1, PLAYER_RADIUS) && s.fac.body_fits(nc.0, nc.1, PLAYER_RADIUS) {
                        assert!(!s.fac.rail_between(c, n), "{}: a railing blocks the {:?} at {c:?} toward {n:?}", s.name, st.kind);
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 0, "no sample chunk has a stair or ladder to walk");
}

#[test]
fn a_hunter_shoved_at_a_railing_lands_on_its_own_side() {
    // the void's shove (hunters/gimmicks/void/void_blue.rs) searches for an open spot from the point the clear line
    // stops at, so a push aimed through a railing never lands her on the water's side of it
    let mut checked = 0;
    for s in samples() {
        for (c, bit, _) in edges(s).into_iter().step_by(3) {
            let (cx, cz) = c.center();
            if !s.fac.body_fits(cx, cz, HUNTER_RADIUS) {
                continue;
            }
            let st = step_of(bit);
            let wanted = (cx + st.0 as f32 * 6.0, cz + st.1 as f32 * 6.0);
            let stop = s.fac.clear_toward((cx, cz), wanted, HUNTER_RADIUS);
            let landed = s.fac.nearest_open(stop.0, stop.1, HUNTER_RADIUS).expect("the walkway has an open spot");
            assert_eq!(cell_of(landed), c, "{}: a shove through the rail at {c:?} landed at {landed:?}", s.name);
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}
