//! Hunters respect exactly what blocks the runner (the KT report "hunters are able to noclip through pipes and obstacles").
//!
//! The runner's one collision is the facility's rule (`game::player::body_fits`: `Facility::body_fits_at_y` at the feet
//! height, plus the neighbour level when resident). A hunter walks the same facility with a wider body, so what stops
//! the runner must stop it. These tests hold that on real Infinite windows:
//!
//! * `hunters_never_stand_where_the_runner_cannot_*`: levels 0, -1 (sewers) and -3 (Inferno), several seeds, 12 hunters
//!   sent after a runner who keeps jumping across the window, one run at 60 fps and one at the window loop's longest
//!   step (`run_gate::MAX_FRAME_DT`). Every frame each hunter (the active list and every resident list) is held against
//!   the runner's predicate at the point it stands and along the segment it covered since the last frame.
//! * `every_solid_prop_footprint_is_closed_to_a_hunter`: every prop type on every level (pipes, pumps, outflow mains,
//!   gates, furniture), sub-cell and elongated footprints included.
//! * `a_hunter_driven_hard_at_a_pipe_never_enters_it`: a hunter pushed at a solid sewer prop from eight sides with steps
//!   far longer than any frame.
//! * the stair-well tests: the straight chase line across a well, and the crowd push beside one.
//!
//! A teleport on purpose (a gimmick, a window shift, an edge walk) is skipped by its length or its state, not hidden.

use super::brain::{cell_of, Brain};
use crate::audio::Sounds;
use crate::config::{Difficulty, HUNTER_RADIUS, PLAYER_RADIUS};
use crate::content::Content;
use crate::game::level_stack::floor_y_of;
use crate::game::{Controls, Game};
use crate::world::catalog::Catalog;
use crate::world::{Cell, Facility};

/// Seconds between the runner's jumps to a new cell.
const HOP_SECONDS: f32 = 4.0;

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// The runner's predicate for a body at `p` standing on the level's floor.
fn runner_fits(fac: &Facility, p: (f32, f32)) -> bool {
    crate::game::player::body_fits(fac, p.0, p.1, 0.0)
}

/// What one walk found.
#[derive(Default, Debug)]
pub struct Found {
    pub samples: usize,
    pub moved_m: f32,
    pub inside: Vec<String>,
    pub crossed: Vec<String>,
}

impl Found {
    fn add(&mut self, o: Found) {
        self.samples += o.samples;
        self.moved_m += o.moved_m;
        self.inside.extend(o.inside);
        self.crossed.extend(o.crossed);
    }
}

/// Holds one hunter's move since the last frame against the runner's predicate on `fac`.
fn hold(found: &mut Found, label: &str, fac: &Facility, prev: (f32, f32), b: &Brain) {
    found.samples += 1;
    let step = dist(prev, b.pos);
    found.moved_m += step;
    // an edge walk owns its own height and an off-floor hunter is a flight/ladder user; a long jump is a teleport
    if b.walk.is_some() || (b.y - floor_y_of(fac.level)).abs() > 0.05 || step > 1.0 {
        return;
    }
    let who = &b.def.id;
    let before = fac.body_fits(prev.0, prev.1, HUNTER_RADIUS);
    if before && !runner_fits(fac, b.pos) && found.inside.len() < 30 {
        found.inside.push(format!(
            "{label} {who} {:.2},{:.2} -> {:.2},{:.2} state {:?}: stands where the runner cannot (hunter-fit {}, open {})",
            prev.0,
            prev.1,
            b.pos.0,
            b.pos.1,
            b.state,
            fac.body_fits(b.pos.0, b.pos.1, HUNTER_RADIUS),
            fac.hunter_open(cell_of(b.pos)),
        ));
    }
    // the hunter's own rule (the runner's body rule plus the crawl-under pipes) holds at every step it takes on its own
    if fac.hunter_fits(prev.0, prev.1) && !fac.hunter_fits(b.pos.0, b.pos.1) && found.inside.len() < 30 {
        found.inside.push(format!(
            "{label} {who} {:.2},{:.2} -> {:.2},{:.2} state {:?}: left the hunter rule (a crawl-under pipe or a body fit)",
            prev.0, prev.1, b.pos.0, b.pos.1, b.state
        ));
    }
    // The hunter moves a short step in x and then in y, so a straight line is not its path: a graze of an obstacle's
    // corner by the line is no crossing. A crossing is a blocked run of the line as long as the thinnest thing that
    // stops the runner (a door slab, 0.1 m) plus his two radii would make it, taken as 0.2 m.
    if before && step > 0.0 {
        let k = (step / 0.04).ceil().max(1.0) as i32;
        let mut run = 0.0f32;
        for s in 1..k {
            let t = s as f32 / k as f32;
            let p = (prev.0 + (b.pos.0 - prev.0) * t, prev.1 + (b.pos.1 - prev.1) * t);
            if runner_fits(fac, p) {
                run = 0.0;
                continue;
            }
            run += step / k as f32;
            if run >= 0.2 {
                if found.crossed.len() < 30 {
                    found.crossed.push(format!(
                        "{label} {who} {:.2},{:.2} -> {:.2},{:.2}: crossed {:.2},{:.2} state {:?}",
                        prev.0, prev.1, b.pos.0, b.pos.1, p.0, p.1, b.state
                    ));
                }
                break;
            }
        }
    }
}

/// Wakes `n` hunters in a real window on `level` and sends them after a runner who jumps to a new open cell every
/// `HOP_SECONDS`, for `seconds` of play at `dt` per frame; holds every hunter's move against the runner's predicate.
pub fn walk(level: i32, seed: u32, n: usize, seconds: f32, dt: f32, content: &Content, catalog: &Catalog) -> Found {
    crate::world::route::layout::godot_regions_for_this_test();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, seed, &crate::world::route::room_text(seed), level, content, catalog);
    g.hunters.clear();
    g.admin.god = true;
    g.p.flashlight_on = true;
    let pool = content.draw_pool();
    let runner = g.p.pos;
    let open_cells = |g: &Game| -> Vec<Cell> {
        g.fac
            .floor_cells()
            .into_iter()
            .filter(|c| g.fac.hunter_open(*c) && g.fac.body_fits(c.center().0, c.center().1, HUNTER_RADIUS))
            .collect()
    };
    let cells: Vec<Cell> = open_cells(&g)
        .into_iter()
        .filter(|c| dist(c.center(), runner) >= 5.0 && dist(c.center(), runner) <= 40.0)
        .collect();
    let stride = (cells.len() / n.max(1)).max(1);
    for (i, c) in cells.iter().step_by(stride).take(n).enumerate() {
        let mut b = Brain::new(content.hunters[pool[i % pool.len()]].clone(), c.center(), &format!("COLL:{seed}:{i}"), Difficulty::Standard);
        b.set_level(level);
        b.activate_at(runner);
        g.hunters.push(b);
    }
    let controls = Controls { fwd: 1.0, sprint: true, ..Default::default() };
    let sounds = Sounds::default();
    let mut found = Found::default();
    let hop_frames = ((HOP_SECONDS / dt).round() as usize).max(1);
    let frames = (seconds / dt).round() as usize;
    let mut rng = seed;
    let key = |b: &Brain| (b.def.id.clone(), b.level);
    let mut last: Vec<((f32, f32), (String, i32))> = vec![];
    let mut resident_last: Vec<((f32, f32), (String, i32), i32)> = vec![];
    let mut last_origin = g.window_origin();
    for frame in 0..frames {
        if frame % hop_frames == 0 {
            let open = open_cells(&g);
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let c = open[(rng >> 8) as usize % open.len()];
            g.p.pos = c.center();
            for h in g.hunters.iter_mut() {
                h.chase(g.p.pos);
                h.active = true;
            }
        }
        g.p.flashlight_on = true;
        g.p.lamp = g.p.lamp.max(0.5);
        g.step(dt, &controls, content, catalog, &sounds);
        let origin = g.window_origin();
        let shifted = origin != last_origin;
        last_origin = origin;
        let now: Vec<((f32, f32), (String, i32))> = g.hunters.iter().map(|h| (h.pos, key(h))).collect();
        let now_resident: Vec<((f32, f32), (String, i32), i32)> =
            g.resident_hunters.iter_leveled().map(|(l, h)| (h.pos, key(h), l)).collect();
        if !shifted && last.len() == now.len() {
            for (i, h) in g.hunters.iter().enumerate() {
                if last[i].1 == now[i].1 {
                    hold(&mut found, &format!("L{level} seed {seed}"), &g.fac, last[i].0, h);
                }
            }
        }
        if !shifted {
            if let Some(inf) = g.inf.as_ref() {
                for (l, h) in g.resident_hunters.iter_leveled() {
                    let fac = if l == g.fac.level { Some(&g.fac) } else { inf.stack.get(l).map(|r| &r.fac) };
                    let Some(fac) = fac else { continue };
                    if let Some(p) = resident_last.iter().find(|p| p.1 == key(h) && p.2 == l) {
                        hold(&mut found, &format!("L{level} seed {seed} resident L{l}"), fac, p.0, h);
                    }
                }
            }
        }
        last = now;
        resident_last = now_resident;
    }
    found
}

fn run_levels(dt: f32, seconds: f32, seeds: u32) {
    let (content, catalog) = (Content::load(), Catalog::load());
    let only: Option<i32> = std::env::var("LR_COLL_LEVEL").ok().and_then(|v| v.parse().ok());
    let mut all = Found::default();
    for level in [0, -1, -3].into_iter().filter(|l| only.map_or(true, |o| o == *l)) {
        for s in 0..seeds {
            let f = walk(level, 20261101 + s, 12, seconds, dt, &content, &catalog);
            println!("L{level} seed {s} dt {dt:.3}: {} samples, {:.0} m walked, {} inside, {} crossed", f.samples, f.moved_m, f.inside.len(), f.crossed.len());
            assert!(f.moved_m > 20.0, "L{level} seed {s}: the hunters walked ({:.1} m)", f.moved_m);
            all.add(f);
        }
    }
    for s in all.inside.iter().chain(all.crossed.iter()) {
        println!("{s}");
    }
    assert!(all.inside.is_empty(), "{} hunter moves ended where the runner cannot stand", all.inside.len());
    assert!(all.crossed.is_empty(), "{} hunter moves crossed what blocks the runner", all.crossed.len());
}

#[test]
fn hunters_never_stand_where_the_runner_cannot_at_60_fps() {
    run_levels(1.0 / 60.0, 14.0, 3);
}

#[test]
fn hunters_never_stand_where_the_runner_cannot_on_the_longest_frame() {
    run_levels(crate::app::run_gate::MAX_FRAME_DT, 14.0, 2);
}

/// Every prop type on every level, how many are solid, and that a solid prop's footprint is closed to a hunter body: a
/// body at the prop's centre, and one just inside each corner of its box, never fits.
#[test]
fn every_solid_prop_footprint_is_closed_to_a_hunter() {
    use crate::content::loot::LootTables;
    use crate::world::infinite::{chunk, ChunkKey};
    use std::collections::BTreeMap;
    let (cat, loot) = (Catalog::load(), LootTables::default());
    // (level, type) -> (count, solid, open footprints)
    let mut table: BTreeMap<(i32, String), (u32, u32, u32)> = BTreeMap::new();
    let mut open_examples: Vec<String> = vec![];
    for level in [0, -1, -2, -3] {
        for seed in ["ROOM-CENSUS-1", "ROOM-CENSUS-2"] {
            for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (2, 0), (0, 2), (-1, -1)] {
                let (fac, _) = chunk(seed, ChunkKey { cx, cy, level }, &cat, &loot);
                for p in &fac.props {
                    let e = table.entry((level, p.ptype.clone())).or_default();
                    e.0 += 1;
                    if !p.solid && !p.crawl_under {
                        continue;
                    }
                    e.1 += 1;
                    let c = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
                    let pts = [
                        c,
                        (c.0 - p.half.0 + 0.05, c.1 - p.half.1 + 0.05),
                        (c.0 + p.half.0 - 0.05, c.1 - p.half.1 + 0.05),
                        (c.0 - p.half.0 + 0.05, c.1 + p.half.1 - 0.05),
                        (c.0 + p.half.0 - 0.05, c.1 + p.half.1 - 0.05),
                    ];
                    // a point of the box outside the floor is a wall already
                    if pts.iter().any(|q| fac.is_floor(q.0 as f64, q.1 as f64) && fac.body_fits(q.0, q.1, 0.05) && !(p.crawl_under && fac.crawl_blocks(q.0, q.1, 0.05))) {
                        e.2 += 1;
                        if open_examples.len() < 15 {
                            open_examples.push(format!("L{level} {seed} {cx},{cy} {} {} half {:?} at {c:?}", p.id, p.ptype, p.half));
                        }
                    }
                }
            }
        }
    }
    for ((level, t), (n, solid, open)) in &table {
        println!("L{level} {t:28} props {n:4} solid {solid:4} footprint open {open:4}");
    }
    assert!(open_examples.is_empty(), "a solid footprint a hunter body fits inside: {open_examples:#?}");
    let solid_pipes: u32 = table.iter().filter(|((_, t), _)| t.contains("ipe") || t.contains("ump")).map(|(_, v)| v.1).sum();
    assert!(solid_pipes > 0, "the census saw solid pipes and pumps");
}

/// A hunter driven hard at every solid sewer prop (the outflow main, a pump, a closed gate) from eight sides, with steps
/// far longer than a frame (0.05 to 1 m), never gets inside its footprint nor stands where the runner cannot.
#[test]
fn a_hunter_driven_hard_at_a_pipe_never_enters_it() {
    use crate::content::loot::LootTables;
    use crate::world::infinite::{chunk, ChunkKey};
    let (cat, loot) = (Catalog::load(), LootTables::default());
    let content = Content::load();
    let def = content.hunters[content.draw_pool()[0]].clone();
    let (mut driven, mut worst_gap) = (0, f32::MAX);
    for level in [-1, -2] {
        for seed in ["ROOM-PIPE-DRIVE-1", "ROOM-PIPE-DRIVE-2"] {
            for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (2, 0), (0, 2)] {
                let (fac, _) = chunk(seed, ChunkKey { cx, cy, level }, &cat, &loot);
                for p in fac.props.iter().filter(|p| (p.solid || p.crawl_under) && (p.ptype.contains("ipe") || p.ptype.contains("ump"))) {
                    let c = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
                    for k in 0..8 {
                        let a = k as f32 * std::f32::consts::FRAC_PI_4;
                        let dir = (a.cos(), a.sin());
                        let reach = p.half.0.max(p.half.1) + 1.6;
                        let from = (c.0 + dir.0 * reach, c.1 + dir.1 * reach);
                        let far = (c.0 - dir.0 * reach, c.1 - dir.1 * reach);
                        if !fac.hunter_fits(from.0, from.1) {
                            continue;
                        }
                        for travel in [0.05f32, 0.11, 0.3, 1.0] {
                            let mut b = Brain::new(def.clone(), from, "PIPE-DRIVE", Difficulty::Standard);
                            b.set_level(level);
                            for _ in 0..(40.0 / travel) as usize {
                                b.step_toward(far, travel, &fac, 0.0);
                                assert!(
                                    fac.hunter_fits(b.pos.0, b.pos.1) && runner_fits(&fac, b.pos),
                                    "L{level} {seed} {cx},{cy}: a hunter driven at {} {:?} from {from:?} by {travel} m stands at {:?}",
                                    p.ptype,
                                    c,
                                    b.pos
                                );
                                let inside = (b.pos.0 - c.0).abs() < p.half.0 && (b.pos.1 - c.1).abs() < p.half.1;
                                assert!(!inside, "L{level} {seed}: inside the footprint of {} at {c:?}: {:?}", p.ptype, b.pos);
                                worst_gap = worst_gap.min(((b.pos.0 - c.0).abs() - p.half.0).max((b.pos.1 - c.1).abs() - p.half.1));
                            }
                            driven += 1;
                        }
                    }
                }
            }
        }
    }
    println!("{driven} drives at sewer pipes and pumps; the nearest the body centre came to a footprint: {worst_gap:.3} m");
    assert!(driven > 40, "the chunks have solid pipes to drive at ({driven})");
}

/// A hunter driven straight across a stair well from every side stops at its edge.
#[test]
fn a_hunter_driven_across_a_stair_well_stops_at_its_edge() {
    use crate::hunters::flight_edges::{self, TestShape};
    let def = Content::load().hunters[0].clone();
    for shape in [TestShape::Switchback, TestShape::Straight, TestShape::Spiral] {
        let s = flight_edges::test_stage(shape, &flight_edges::rules().clone());
        let fac = &s.upper;
        let well: Vec<Cell> = fac.well_cells_all();
        assert!(!well.is_empty());
        let mut driven = 0;
        for c in well.iter().step_by(2) {
            let centre = c.center();
            for k in 0..12 {
                let a = k as f32 * std::f32::consts::FRAC_PI_6;
                let dir = (a.cos(), a.sin());
                let from = (centre.0 + dir.0 * 4.5, centre.1 + dir.1 * 4.5);
                let far = (centre.0 - dir.0 * 4.5, centre.1 - dir.1 * 4.5);
                if !fac.body_fits(from.0, from.1, HUNTER_RADIUS) {
                    continue;
                }
                for travel in [0.05f32, 0.4] {
                    let mut b = Brain::new(def.clone(), from, "WELL-DRIVE", Difficulty::Standard);
                    b.set_level(fac.level);
                    for _ in 0..(40.0 / travel) as usize {
                        b.step_toward(far, travel, fac, 0.0);
                        assert!(runner_fits(fac, b.pos), "{shape:?}: a hunter driven across the well stands at {:?}, where the runner cannot", b.pos);
                    }
                    driven += 1;
                }
            }
        }
        assert!(driven > 20, "{shape:?}: {driven} drives");
    }
}

/// The crowd push (`director::separate`) never carries a hunter into a well, a prop or a door, however long it runs.
#[test]
fn the_crowd_push_keeps_hunters_out_of_a_stair_well() {
    use crate::hunters::flight_edges::{self, TestShape};
    let def = Content::load().hunters[0].clone();
    let s = flight_edges::test_stage(TestShape::Straight, &flight_edges::rules().clone());
    let fac = &s.upper;
    let landing = fac.stairs[0].landing().center();
    let mut moved_any = false;
    for k in 0..16 {
        let a = k as f32 * std::f32::consts::FRAC_PI_8;
        // two hunters on the landing, one a little way out along `a`, pushing each other toward the well edge
        let near = (landing.0 + a.cos() * 0.6, landing.1 + a.sin() * 0.6);
        if !fac.body_fits(landing.0, landing.1, HUNTER_RADIUS) || !fac.body_fits(near.0, near.1, HUNTER_RADIUS) {
            continue;
        }
        let mut hs: Vec<Brain> = [landing, near]
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut b = Brain::new(def.clone(), *p, &format!("PUSH:{i}"), Difficulty::Standard);
                b.set_level(fac.level);
                b.active = true;
                b
            })
            .collect();
        for _ in 0..600 {
            moved_any |= super::director::separate(1.0 / 20.0, &mut hs, fac) > 0;
            for b in &hs {
                assert!(runner_fits(fac, b.pos), "the crowd push put a hunter at {:?}, where the runner cannot stand", b.pos);
            }
        }
    }
    assert!(moved_any, "the push moved somebody");
}

/// The crawl-under pipes stop a standing runner and a hunter, and let a crouched runner through. The floor footprint
/// stays open (`solid` is false); the height rule is `Facility::crawl_stops`.
#[test]
fn a_crawl_under_pipe_stops_a_standing_runner_and_a_hunter_and_lets_a_crouch_through() {
    use crate::content::loot::LootTables;
    use crate::world::infinite::{chunk, ChunkKey};
    let (cat, loot) = (Catalog::load(), LootTables::default());
    let (mut pipes, mut runner_passes) = (0, 0);
    for level in [-1, -2] {
        for seed in ["ROOM-CRAWL-1", "ROOM-CRAWL-2", "ROOM-CRAWL-3", "ROOM-CRAWL-4"] {
            for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (2, 0), (0, 2), (-1, -1), (2, 1), (1, 2)] {
                let (fac, _) = chunk(seed, ChunkKey { cx, cy, level }, &cat, &loot);
                for p in fac.props.iter().filter(|p| p.crawl_under) {
                    pipes += 1;
                    assert!(!p.solid, "the floor footprint stays open: {}", p.id);
                    let c = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
                    assert!(!fac.hunter_fits(c.0, c.1), "{}: a hunter fits at the pipe's centre {c:?}", p.id);
                    assert!(!fac.hunter_open(Cell::new(c.0.floor() as i32, c.1.floor() as i32)), "{}: the hunters' grid is open under the pipe", p.id);
                    let crouch = crate::world::route::sewer::core::CROUCH_COLLISION_HEIGHT as f32;
                    let stand = crate::config::PLAYER_HEIGHT;
                    if fac.body_fits(c.0, c.1, PLAYER_RADIUS) {
                        runner_passes += 1;
                        assert!(fac.crawl_stops(c.0, c.1, PLAYER_RADIUS, stand), "{}: a standing runner walks through", p.id);
                        assert!(!fac.crawl_stops(c.0, c.1, PLAYER_RADIUS, crouch), "{}: a crouched runner is blocked", p.id);
                    }
                }
            }
        }
    }
    println!("{pipes} crawl pipes; the runner's body fits under {runner_passes}");
    assert!(pipes >= 3, "the sample chunks have crawl pipes ({pipes})");
    assert!(runner_passes >= 1, "a pipe the runner can reach");
}

/// A standing runner walks into a crawl pipe and stops; crouched, the same approach goes under it, and releasing
/// crouch there does not stand the body back up.
#[test]
fn a_standing_runner_is_stopped_by_a_crawl_pipe_and_a_crouched_one_passes() {
    use crate::content::loot::LootTables;
    use crate::game::level_stack::LevelStack;
    use crate::game::meters::Meters;
    use crate::game::player::Player;
    use crate::world::infinite::{chunk, ChunkKey};
    let (cat, loot) = (Catalog::load(), LootTables::default());
    let (fac, _) = chunk("ROOM-CRAWL-1", ChunkKey { cx: 0, cy: 0, level: -1 }, &cat, &loot);
    let pipe = fac.props.iter().find(|p| p.crawl_under && {
        let c = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
        fac.body_fits(c.0, c.1, PLAYER_RADIUS)
    }).expect("a reachable crawl pipe");
    let centre = (pipe.cell.x as f32 + 0.5 + pipe.offset.0, pipe.cell.y as f32 + 0.5 + pipe.offset.1);
    let approaches = [
        ((0.0_f32, 1.0_f32), 0.0_f32, pipe.half.1),
        ((0.0, -1.0), std::f32::consts::PI, pipe.half.1),
        ((1.0, 0.0), std::f32::consts::FRAC_PI_2, pipe.half.0),
        ((-1.0, 0.0), -std::f32::consts::FRAC_PI_2, pipe.half.0),
    ];
    let (dir, yaw, half) = approaches
        .into_iter()
        .find(|(dir, _, half)| {
            let start = (centre.0 + dir.0 * (half + PLAYER_RADIUS + 0.4), centre.1 + dir.1 * (half + PLAYER_RADIUS + 0.4));
            fac.body_fits(start.0, start.1, PLAYER_RADIUS) && !fac.crawl_blocks(start.0, start.1, PLAYER_RADIUS)
        })
        .expect("an open approach to the pipe");
    let start = (centre.0 + dir.0 * (half + PLAYER_RADIUS + 0.4), centre.1 + dir.1 * (half + PLAYER_RADIUS + 0.4));
    let prof = Difficulty::Standard.profile();
    let stack = LevelStack::EMPTY;
    let mut meters = Meters::default();
    let sounds = Sounds::default();
    let under = |body: &Player| fac.crawl_blocks(body.pos.0, body.pos.1, PLAYER_RADIUS);
    let mut walk = |crouch: bool, frames: i32| {
        let mut body = Player::new(start);
        body.floor_y = floor_y_of(fac.level);
        body.yaw = yaw;
        body.crouch_t = if crouch { 1.0 } else { 0.0 };
        let keys = Controls { fwd: 1.0, crouch, ..Controls::default() };
        let mut entered = false;
        for _ in 0..frames {
            body.update_movement(1.0 / 60.0, &keys, &prof, &fac, &stack, &mut meters, &sounds);
            entered |= under(&body);
        }
        (body, entered)
    };
    let (stood, stood_in) = walk(false, 180);
    assert!(!stood_in, "a standing runner went under the pipe at {:?} (centre {centre:?})", stood.pos);
    assert!(dist(stood.pos, start) > 0.05, "the standing runner did move");
    assert!(dist(stood.pos, centre) + 0.05 < dist(start, centre), "the standing runner never closed on the pipe");
    let (ducked, ducked_in) = walk(true, 180);
    assert!(ducked_in, "crouched runner never went under the pipe: {:?} centre {centre:?}", ducked.pos);
    assert!(ducked.crouch_t > 0.9, "still crouched under the pipe");
    // letting the key go must not stand the body up inside the pipe: walk back in and hold there
    let mut held = Player::new(start);
    held.floor_y = floor_y_of(fac.level);
    held.yaw = yaw;
    held.crouch_t = 1.0;
    let down = Controls { fwd: 1.0, crouch: true, ..Controls::default() };
    for _ in 0..180 {
        held.update_movement(1.0 / 60.0, &down, &prof, &fac, &stack, &mut meters, &sounds);
        if under(&held) {
            break;
        }
    }
    assert!(under(&held), "could not stop under the pipe to test the uncrouch lock");
    let up = Controls { fwd: 0.0, crouch: false, ..Controls::default() };
    for _ in 0..90 {
        held.update_movement(1.0 / 60.0, &up, &prof, &fac, &stack, &mut meters, &sounds);
    }
    assert!(held.crouching && held.crouch_t > 0.85, "released crouch stood up inside the pipe ({})", held.crouch_t);
}

/// Cost of the hunter footprint test on real windows (`cargo test collision_cost -- --ignored --nocapture`): the plain
/// test the hunters used before (`body_fits_at` at height 0), `body_fits` now, and `hunter_fits`. Per call, in ns.
#[test]
#[ignore]
fn collision_cost() {
    use crate::shared::clock::Instant;
    let (content, catalog) = (Content::load(), Catalog::load());
    for level in [0, -1, -3] {
        let g = Game::new_infinite_sim(Difficulty::Standard, 20261101, &crate::world::route::room_text(20261101), level, &content, &catalog);
        let pts: Vec<(f32, f32)> = g
            .fac
            .floor_cells()
            .into_iter()
            .flat_map(|c| [(c.center().0, c.center().1), (c.center().0 + 0.3, c.center().1 - 0.2)])
            .collect();
        let time = |name: &str, f: &dyn Fn(f32, f32) -> bool| {
            let t = Instant::now();
            let mut hits = 0u64;
            let rounds = 40;
            for _ in 0..rounds {
                for p in &pts {
                    hits += u64::from(f(p.0, p.1));
                }
            }
            let ns = t.elapsed().as_secs_f64() * 1e9 / (rounds as f64 * pts.len() as f64);
            println!("L{level} {name:14} {ns:7.1} ns/call ({hits} fit, {} stairs, {} cells)", g.fac.stairs.len(), pts.len());
        };
        time("old plain", &|x, y| g.fac.body_fits_at(x, y, HUNTER_RADIUS, 0.0));
        time("body_fits", &|x, y| g.fac.body_fits(x, y, HUNTER_RADIUS));
        time("hunter_fits", &|x, y| g.fac.hunter_fits(x, y));
    }
}

/// The crowd push and the placement searches (`nearest_open`, `clear_toward`: the Void teleports, the spawn spots) keep a
/// hunter out of a crawl-under pipe too, from every side.
#[test]
fn the_push_and_the_placement_searches_keep_a_hunter_out_of_a_crawl_pipe() {
    use crate::content::loot::LootTables;
    use crate::world::infinite::{chunk, ChunkKey};
    let (cat, loot) = (Catalog::load(), LootTables::default());
    let def = Content::load().hunters[0].clone();
    let (mut pushes, mut placed) = (0, 0);
    for level in [-1, -2] {
        for seed in ["ROOM-CRAWL-1", "ROOM-CRAWL-2", "ROOM-CRAWL-3", "ROOM-CRAWL-4"] {
            for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (2, 0), (0, 2), (-1, -1), (2, 1), (1, 2)] {
                let (fac, _) = chunk(seed, ChunkKey { cx, cy, level }, &cat, &loot);
                for p in fac.props.iter().filter(|p| p.crawl_under) {
                    let c = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
                    // the placement searches asked from the pipe's centre
                    if let Some(q) = fac.nearest_open(c.0, c.1, HUNTER_RADIUS) {
                        assert!(fac.hunter_fits(q.0, q.1), "{}: nearest_open set a hunter down at {q:?}", p.id);
                        placed += 1;
                    }
                    for k in 0..8 {
                        let a = k as f32 * std::f32::consts::FRAC_PI_4;
                        let out = (a.cos(), a.sin());
                        // the first spot of the line in from outside where a hunter no longer fits
                        let reach = p.half.0.max(p.half.1) + 1.5;
                        let (mut edge, mut hit) = (None, false);
                        for i in 0..=((reach / 0.02) as i32) {
                            let d = reach - i as f32 * 0.02;
                            let q = (c.0 + out.0 * d, c.1 + out.1 * d);
                            if fac.hunter_fits(q.0, q.1) {
                                edge = Some(q);
                            } else if edge.is_some() && fac.crawl_blocks(q.0, q.1, HUNTER_RADIUS) {
                                hit = true;
                                break;
                            } else if edge.is_some() {
                                break;
                            }
                        }
                        let (Some(edge), true) = (edge, hit) else { continue };
                        // the clear-toward shove stops at the pipe
                        let end = fac.clear_toward(edge, c, HUNTER_RADIUS);
                        assert!(fac.hunter_fits(end.0, end.1), "{}: clear_toward carried a hunter to {end:?}", p.id);
                        // a second hunter just outside pushes this one in toward the pipe, however long it runs
                        let behind = (edge.0 + out.0 * 0.4, edge.1 + out.1 * 0.4);
                        if !fac.hunter_fits(behind.0, behind.1) {
                            continue;
                        }
                        let mut hs: Vec<Brain> = [edge, behind]
                            .iter()
                            .enumerate()
                            .map(|(i, q)| {
                                let mut b = Brain::new(def.clone(), *q, &format!("CRAWL-PUSH:{i}"), Difficulty::Standard);
                                b.set_level(level);
                                b.active = true;
                                b
                            })
                            .collect();
                        for _ in 0..400 {
                            pushes += super::director::separate(1.0 / 20.0, &mut hs, &fac);
                            for b in &hs {
                                assert!(fac.hunter_fits(b.pos.0, b.pos.1), "{}: the push put a hunter at {:?}, inside the crawl pipe", p.id, b.pos);
                            }
                        }
                    }
                }
            }
        }
    }
    println!("{pushes} pushes, {placed} placements");
    assert!(placed > 3, "crawl pipes were sampled ({placed})");
}
