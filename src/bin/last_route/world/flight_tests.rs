//! Flight tests: footprints, the surfaces, the Godot geometry, the dressing, the shape choice and the data (declared in
//! flight.rs).

use super::*;

const RUNS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];
const SHAPES: [FlightShape; 3] = [FlightShape::Switchback, FlightShape::Straight, FlightShape::Spiral];

fn stairs(shape: FlightShape, run: (i32, i32), cw: bool) -> Flight {
    Flight::new(FlightKind::Stairs, shape, 1, Cell::new(20, 20), run).with_clockwise(cw)
}

/// The shipped table with one edit, parsed again.
fn tweak(edit: impl FnOnce(&mut serde_json::Value)) -> Result<FlightRules, String> {
    let mut v: serde_json::Value = serde_json::from_str(FILE).unwrap();
    edit(&mut v);
    FlightRules::parse(&v.to_string())
}

/// The surface height a walker follows along a route, with the spiral's slab left out (a slab is upper floor beside the
/// helicoid's end, not part of the way down).
fn route_y(f: &Flight, x: f32, z: f32) -> Option<f32> {
    if f.shape == FlightShape::Spiral {
        let (u, v) = to_local(f, x, z);
        return spiral_surface(flights(), f, u, v).filter(|(_, l)| *l == SpiralLayer::Helix).map(|(y, _)| y);
    }
    flight_surface_y(f, x, z)
}

// ---------------------------------------------------------------------------------------------------------------------
// Footprints
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_shipped_table_is_valid_and_matches_the_game() {
    let r = FlightRules::parse(FILE).expect("data/flights.json is valid");
    assert!((r.storey - STOREY).abs() < 1e-6);
    assert_eq!(&r, flights());
}

#[test]
fn footprints_match_from_both_sides() {
    let rules = flights();
    for shape in SHAPES {
        for run in RUNS {
            for cw in [false, true] {
                let top = stairs(shape, run, cw);
                let fp = well_footprint_with(rules, &top);
                // the same flight rebuilt from the foot on the level below
                let bottom = Flight::from_foot_with(rules, FlightKind::Stairs, shape, 0, fp.foot, run, cw);
                assert_eq!(bottom, top, "{shape:?} {run:?} cw {cw}");
                assert_eq!(well_footprint_with(rules, &bottom), fp);
                assert_eq!(fp.upper_hole, fp.lower_open, "the hole and the open ceiling are the same cells");
                assert!(!fp.upper_hole.contains(&fp.landing), "{shape:?}: A is floor on the upper level");
                assert!(!fp.upper_hole.contains(&fp.foot), "{shape:?}: B is floor on the lower level");
                assert!(fp.upper_clear.contains(&fp.landing) && fp.lower_clear.contains(&fp.foot));
                assert!(fp.upper_clear.len() as i32 >= rules.margin_cells + 1);
                for c in fp.upper_clear.iter().chain(&fp.lower_clear) {
                    assert!(!fp.upper_hole.contains(c), "{shape:?} {run:?} cw {cw}: clear cell {c:?} is in the hole");
                }
                assert_eq!(fp.step_cell, None);
                assert_eq!(fp.slab, 0.1, "Godot's slab");
            }
        }
    }
    for kind in [FlightKind::Ladder, FlightKind::Drop] {
        for run in RUNS {
            let top = Flight::new(kind, FlightShape::Switchback, 2, Cell::new(9, 9), run);
            let fp = well_footprint_with(rules, &top);
            let bottom = Flight::from_foot_with(rules, kind, FlightShape::Switchback, 1, fp.foot, run, false);
            assert_eq!(well_footprint_with(rules, &bottom), fp, "{kind:?} {run:?}");
            assert_eq!(fp.upper_hole, vec![Cell::new(9, 9)]);
            assert_eq!(fp.upper_hole, fp.lower_open);
        }
    }
}

#[test]
fn the_surface_exists_only_over_the_footprint() {
    let rules = flights();
    for shape in SHAPES {
        for run in RUNS {
            let f = stairs(shape, run, run.0 > 0);
            let fp = well_footprint_with(rules, &f);
            let mut found = 0;
            for i in 0..160 {
                for j in 0..160 {
                    let (x, z) = (10.0 + i as f32 * 0.25 + 0.01, 10.0 + j as f32 * 0.25 + 0.01);
                    if flight_surface_y_with(rules, &f, x, z).is_some() {
                        found += 1;
                        let c = Cell::new(x.floor() as i32, z.floor() as i32);
                        assert!(
                            fp.upper_hole.contains(&c) || c == fp.landing || c == fp.foot || landing_row_with(rules, &f).contains(&c),
                            "{shape:?} {run:?}: surface at {x},{z} outside the footprint"
                        );
                    }
                }
            }
            assert!(found > 20, "{shape:?} {run:?}: only {found} surface samples");
        }
    }
}

#[test]
fn the_shapes_have_their_own_footprints() {
    let rules = flights();
    let n = |shape| well_footprint_with(rules, &stairs(shape, (0, 1), false)).upper_hole.len();
    assert_eq!(n(FlightShape::Switchback), 25, "Godot's 5 x 5 well beyond the 5 x 2 landing (the 5 x 7 interior)");
    assert_eq!(n(FlightShape::Straight), 16, "8 long, 2 wide");
    assert_eq!(n(FlightShape::Spiral), 22, "the 5 x 5 square less the doorway-wide landing row");
}

#[test]
fn a_switchback_is_godots_5_by_7_interior() {
    let rules = flights();
    for run in RUNS {
        let f = stairs(FlightShape::Switchback, run, false);
        let fp = well_footprint_with(rules, &f);
        let s = side(run);
        // the well is 5 across and 5 long, the landing 5 across and 2 deep: with the well that is the 5 x 7 interior
        let mut cells: Vec<Cell> = fp.upper_hole.clone();
        cells.extend(fp.upper_clear.iter().filter(|c| !fp.upper_hole.contains(c)));
        let along = |c: &Cell| c.x * run.0.abs() + c.y * run.1.abs();
        let across = |c: &Cell| c.x * s.0 + c.y * s.1;
        let landing_cells: Vec<&Cell> = fp.upper_clear.iter().collect();
        let lc = landing_cells.len();
        assert_eq!(lc, 10, "{run:?}: two landing rows of five (A's row and the row behind it)");
        let rows = |cs: &[Cell]| {
            let (a, b) = (cs.iter().map(&along).min().unwrap(), cs.iter().map(&along).max().unwrap());
            b - a + 1
        };
        let cols = |cs: &[Cell]| {
            let (a, b) = (cs.iter().map(&across).min().unwrap(), cs.iter().map(&across).max().unwrap());
            b - a + 1
        };
        assert_eq!((rows(&cells), cols(&cells)), (7, 5), "{run:?}: the 7 x 5 interior");
        assert_eq!((rows(&fp.upper_hole), cols(&fp.upper_hole)), (5, 5), "{run:?}");
        // A is the first cell of the arriving flight, B the cell beside the divider on the leaving one, level with A
        assert_eq!(along(&fp.landing), along(&fp.foot));
        assert_eq!(across(&fp.landing) - across(&fp.foot), 2, "{run:?}: A and B are two cells apart (the divider between)");
        assert!(fp.upper_hole.iter().all(|c| across(c) >= across(&fp.landing) - 4 && across(c) <= across(&fp.landing) + 1));
    }
}

#[test]
fn a_ladder_has_one_clear_step_cell_beside_the_hatch() {
    let rules = flights();
    for run in RUNS {
        let f = Flight::new(FlightKind::Ladder, FlightShape::Switchback, 0, Cell::new(30, 30), run);
        let fp = well_footprint_with(rules, &f);
        let step = fp.step_cell.expect("a ladder has a step cell");
        assert_eq!((step.x - 30, step.y - 30), run, "beside the hatch, in the run direction");
        assert_eq!(step, fp.landing);
        assert!(!fp.upper_hole.contains(&step));
        assert_eq!(fp.foot, Cell::new(30, 30), "the climber starts under the hatch");
        // the step cell is the upper floor, the shaft bottom is the lower floor
        let (sx, sz) = step.center();
        assert_eq!(flight_surface_y(&f, sx, sz), Some(0.0));
        assert_eq!(flight_surface_y(&f, 30.5, 30.5), Some(-STOREY));
        assert_eq!(flight_surface_y(&f, 33.5, 30.5), None);
    }
}

#[test]
fn a_ladder_steps_off_beside_its_hatch_in_its_run_direction() {
    let rules = flights();
    for run in RUNS {
        let f = Flight::new(FlightKind::Ladder, FlightShape::Switchback, 0, Cell::new(20, 20), run);
        let fp = well_footprint_with(rules, &f);
        assert_eq!(fp.upper_hole, vec![Cell::new(20, 20)], "the hatch alone is the hole");
        assert_eq!(fp.landing, Cell::new(20 + run.0, 20 + run.1), "the step cell is one step along the run");
        assert_eq!(fp.step_cell, Some(fp.landing));
        assert_eq!(fp.foot, Cell::new(20, 20), "the foot is the hatch's own cell below");
        assert!(fp.upper_clear.contains(&fp.landing) && fp.lower_clear.contains(&fp.foot));
        // the same flight seen from its foot is the same record
        let low = Flight::from_foot_with(rules, FlightKind::Ladder, FlightShape::Switchback, -1, fp.foot, run, false);
        assert_eq!(low, f, "{run:?}: both levels build one flight");
    }
}

#[test]
fn the_ladder_dir_order_is_a_seeded_rotation_of_the_four_steps() {
    let mut firsts = std::collections::BTreeSet::new();
    for seed in [1u64, 99, 123_456_789] {
        for cx in -2..=2 {
            for cell in [Cell::new(10, 12), Cell::new(30, 7), Cell::new(22, 22)] {
                let order = ladder_dir_order(seed, (cx, 1), cell);
                assert_eq!(order, ladder_dir_order(seed, (cx, 1), cell), "deterministic");
                let mut sorted = order.to_vec();
                sorted.sort();
                sorted.dedup();
                assert_eq!(sorted.len(), 4, "all four directions, once each");
                for d in order {
                    assert_eq!(d.0.abs() + d.1.abs(), 1, "a unit step");
                }
                // a rotation: consecutive entries are quarter turns of one another, all the same way
                let turn = |a: (i32, i32), b: (i32, i32)| a.0 * b.1 - a.1 * b.0;
                let t0 = turn(order[0], order[1]);
                assert!(t0 != 0 && (1..3).all(|i| turn(order[i], order[i + 1]) == t0), "{order:?} is not a rotation");
                firsts.insert(order[0]);
            }
        }
    }
    assert!(firsts.len() >= 3, "the rotation should start at different steps, got {firsts:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// Surfaces
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_surface_is_the_lower_floor_at_b_and_the_upper_floor_at_a() {
    let rules = flights();
    for level in [1, 0, -2, 3] {
        for shape in SHAPES {
            for run in RUNS {
                for cw in [false, true] {
                    let f = Flight { upper_level: level, ..stairs(shape, run, cw) };
                    let fp = well_footprint_with(rules, &f);
                    let (a, b) = (fp.landing.center(), fp.foot.center());
                    for lower in [false, true] {
                        let top = flight_walk_y_with(rules, &f, a.0, a.1, lower).expect("A has a surface");
                        let bottom = flight_walk_y_with(rules, &f, b.0, b.1, lower).expect("B has a surface");
                        assert!((top - level as f32 * STOREY).abs() < 1e-4, "{shape:?} {run:?}: A {top}");
                        assert!((bottom - (level - 1) as f32 * STOREY).abs() < 1e-4, "{shape:?} {run:?}: B {bottom}");
                    }
                }
            }
        }
    }
    for kind in [FlightKind::Ladder, FlightKind::Drop] {
        for run in RUNS {
            let f = Flight::new(kind, FlightShape::Switchback, 0, Cell::new(5, 5), run);
            let fp = well_footprint(&f);
            let (a, b) = (fp.landing.center(), fp.foot.center());
            assert_eq!(flight_surface_y(&f, a.0, a.1), Some(0.0), "{kind:?} A");
            assert_eq!(flight_surface_y(&f, b.0, b.1), Some(-STOREY), "{kind:?} B");
        }
    }
}

#[test]
fn the_walked_route_is_all_surface_and_only_goes_down() {
    let rules = flights();
    for shape in SHAPES {
        for run in RUNS {
            for cw in [false, true] {
                let f = stairs(shape, run, cw);
                let fp = well_footprint_with(rules, &f);
                let path = walk_path_with(rules, &f, 0.05);
                let mut prev = f32::MAX;
                for (x, z) in &path {
                    let c = Cell::new(x.floor() as i32, z.floor() as i32);
                    if flight_surface_y(&f, *x, *z).is_none() {
                        // only the landing floor lies off the surface
                        assert!(
                            fp.upper_clear.contains(&c) || fp.lower_clear.contains(&c),
                            "{shape:?} {run:?}: no surface and no landing floor at {x},{z}"
                        );
                    }
                    if let Some(y) = route_y(&f, *x, *z) {
                        assert!(y <= prev + 1e-4, "{shape:?} {run:?} cw {cw}: rises from {prev} to {y} at {x},{z}");
                        prev = y;
                    }
                }
                assert!((flight_surface_y(&f, path[0].0, path[0].1).unwrap() - STOREY).abs() < 1e-4);
                // the last sample of a spiral's arc may fall a hair short of the end radial (one 5 cm step at most)
                assert!(prev.abs() < 0.06, "{shape:?}: ends at {prev}");
            }
        }
    }
}

#[test]
fn no_riser_is_above_the_data_max() {
    let rules = flights();
    let tread = 1.0 / rules.steps_per_cell;
    for shape in SHAPES {
        assert!(max_riser_of(rules, shape) <= rules.max_riser + 1e-4, "{shape:?}");
        let f = stairs(shape, (0, 1), false);
        let path = walk_path_with(rules, &f, tread * 0.25);
        // heights one tread apart along the route (a smooth ramp: the slope on the walking line)
        let ys: Vec<Option<f32>> = path.iter().map(|(x, z)| route_y(&f, *x, *z)).collect();
        for i in 0..ys.len().saturating_sub(4) {
            if let (Some(a), Some(b)) = (ys[i], ys[i + 4]) {
                assert!((a - b).abs() <= rules.max_riser + 1e-3, "{shape:?} at {i}: {a} -> {b}");
            }
        }
    }
}

#[test]
fn the_switchback_has_its_mid_landing_at_the_data_rise() {
    let rules = flights();
    let f = stairs(FlightShape::Switchback, (0, 1), false);
    let (lw, len) = (rules.sb_lane_width as f32, rules.sb_length as f32);
    for v in [-2.0, 0.5, 1.0, 2.0] {
        let (x, z) = to_world(&f, len - rules.sb_mid_len as f32 * 0.5, v);
        assert!((flight_surface_y(&f, x, z).unwrap() - 2.5).abs() < 1e-4, "v {v}: the mid landing spans the whole width");
    }
    // the ramps meet it flush at their far end and the upper floor / lower floor at their near end
    let ramp = rules.sb_ramp();
    let (top, foot) = (lw * 0.5, -(rules.sb_divider_width as f32) - lw * 0.5);
    let y = |u: f32, v: f32| {
        let (x, z) = to_world(&f, u, v);
        flight_surface_y(&f, x, z).unwrap()
    };
    assert!((y(0.0, top) - STOREY).abs() < 1e-4 && (y(0.0, foot) - 0.0).abs() < 1e-4);
    assert!((y(ramp, top) - 2.5).abs() < 1e-4 && (y(ramp, foot) - 2.5).abs() < 1e-4);
    // each flight is a plane: halfway down is halfway between
    assert!((y(ramp * 0.5, top) - (STOREY + 2.5) * 0.5).abs() < 1e-4 && (y(ramp * 0.5, foot) - 1.25).abs() < 1e-4);
    assert_eq!(well_footprint(&f).upper_hole.len(), 25, "the 5 x 5 well");
}

#[test]
fn lane_dividers_are_empty_for_straight_and_spiral() {
    for run in RUNS {
        for cw in [false, true] {
            assert!(lane_dividers(&stairs(FlightShape::Straight, run, cw)).is_empty());
            assert!(lane_dividers(&stairs(FlightShape::Spiral, run, cw)).is_empty());
        }
    }
    let ladder = Flight::new(FlightKind::Ladder, FlightShape::Switchback, 1, Cell::new(20, 20), (0, 1));
    assert!(lane_dividers(&ladder).is_empty());
}

#[test]
fn the_switchback_divider_is_a_one_metre_solid_between_the_flights() {
    let rules = flights();
    assert_eq!(divider_width(), 1.0, "Godot's one metre divider");
    for run in RUNS {
        let f = stairs(FlightShape::Switchback, run, false);
        let d = lane_dividers(&f);
        assert_eq!(d.len(), 1);
        let (ua, va) = to_local(&f, d[0].a.0, d[0].a.1);
        let (ub, vb) = to_local(&f, d[0].b.0, d[0].b.1);
        // the centre line of a one metre wall between the two flights, the whole length of the ramp
        assert!(ua.abs() < 1e-4 && (ub - rules.sb_ramp()).abs() < 1e-4, "{run:?}: {ua}..{ub}");
        assert!((va + 0.5).abs() < 1e-4 && (vb + 0.5).abs() < 1e-4, "{run:?}: v {va} {vb}");
        // there is no surface inside it, and the flights either side are a storey apart near the landing
        let at = |u: f32, v: f32| {
            let (x, z) = to_world(&f, u, v);
            flight_surface_y(&f, x, z)
        };
        assert_eq!(at(1.5, -0.5), None, "{run:?}: no floor in the divider");
        assert!(at(0.05, 1.0).unwrap() - at(0.05, -2.0).unwrap() > 4.0);
        assert!(at(rules.sb_ramp() + 0.5, -0.5).is_some(), "the mid landing runs on past the divider's end");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The spiral
// ---------------------------------------------------------------------------------------------------------------------

/// Godot's spiral (stairwells.gd:116-134): landing and foot are the two end cells of the doorway-wide landing row, 270
/// degrees of arc start and end on the two doorway-side corners, and the well is the 5 x 5 square around the post.
#[test]
fn a_spiral_starts_and_ends_on_the_landing_row() {
    let rules = flights();
    for run in RUNS {
        for cw in [false, true] {
            let f = stairs(FlightShape::Spiral, run, cw);
            let fp = well_footprint_with(rules, &f);
            let dir = if cw { -1 } else { 1 };
            let s = side(run);
            // B = A + side * 2 * dir, level with A along the run
            assert_eq!(fp.foot, Cell::new(f.anchor_cell.x + s.0 * 2 * dir, f.anchor_cell.y + s.1 * 2 * dir), "{run:?} cw {cw}");
            // the landing row is A, the cell between and B; it is floor on both levels, not well
            let middle = Cell::new(f.anchor_cell.x + s.0 * dir, f.anchor_cell.y + s.1 * dir);
            for c in [fp.landing, middle, fp.foot] {
                assert!(!fp.upper_hole.contains(&c), "{run:?} cw {cw}: {c:?} is landing floor");
                assert!(fp.upper_clear.contains(&c) && fp.lower_clear.contains(&c), "{run:?} cw {cw}: {c:?} stays open on both levels");
            }
            // the well: 22 cells of the 5 x 5 square, whose centre is the post two rows in from the doorway row
            let centre = Cell::new(middle.x + run.0 * 2, middle.y + run.1 * 2);
            assert_eq!(fp.upper_hole.len(), 22);
            for c in &fp.upper_hole {
                assert!((c.x - centre.x).abs() <= 2 && (c.y - centre.y).abs() <= 2, "{run:?} cw {cw}: {c:?} is outside the square");
            }
            let post = spiral_post_with(rules, &f).expect("a spiral has a post");
            assert_eq!(Cell::new(post.0 .0.floor() as i32, post.0 .1.floor() as i32), centre, "{run:?} cw {cw}: the post is the centre");
            assert!((post.1 - 0.4).abs() < 1e-6, "Godot's post radius");
            // both margins run outwards from the landing row
            for c in fp.upper_clear.iter().chain(&fp.lower_clear) {
                assert!(!fp.upper_hole.contains(c));
            }
            // the arc starts and ends on the doorway-side corners, mirrored about the doorway axis
            let (start, end) = (spiral_point(rules, &f, rules.sp_offset()), spiral_point(rules, &f, rules.sp_offset() + rules.sp_span()));
            let (uc, vc) = spiral_centre(rules, &f);
            for (name, p) in [("start", start), ("end", end)] {
                assert!(p.0 < uc - 1.0 && p.0 > 0.0, "{name} {p:?} is on the doorway side");
                assert!((p.1 - vc).abs() > 1.0, "{name} {p:?} is off the middle column");
            }
            assert!(((start.1 - vc) + (end.1 - vc)).abs() < 1e-3 && (start.0 - end.0).abs() < 1e-3, "the two ends mirror each other");
            let path = walk_path_with(rules, &f, 0.1);
            let (a, b) = (fp.landing.center(), fp.foot.center());
            assert!((path[0].0 - a.0).abs() + (path[0].1 - a.1).abs() < 1e-3);
            let last = path.last().unwrap();
            assert!((last.0 - b.0).abs() + (last.1 - b.1).abs() < 1e-3, "{run:?} cw {cw}: the route ends at the centre of B");
        }
    }
}

#[test]
fn the_spiral_is_a_helicoid_from_the_post_to_the_wall() {
    let rules = flights();
    for cw in [false, true] {
        let f = stairs(FlightShape::Spiral, (0, 1), cw);
        let (span, off) = (rules.sp_span(), rules.sp_offset());
        let at = |swept: f32, r: f32| {
            let (u, v) = spiral_point_at(rules, &f, swept, r);
            spiral_surface(rules, &f, u, v)
        };
        // the same angle is the same height from the post to the wall (a function of the angle only)
        for t in [0.01f32, 0.25, 0.5, 0.75, 0.99] {
            let want = STOREY * (1.0 - t) + f.lower_y();
            for r in [rules.sp_inner + 0.01, 1.0, rules.sp_walk, 2.0, 2.4] {
                let (y, layer) = at(off + span * t, r).unwrap_or_else(|| panic!("cw {cw} t {t} r {r}: no surface"));
                assert_eq!(layer, SpiralLayer::Helix);
                assert!((y - want).abs() < 1e-3, "cw {cw} t {t} r {r}: {y} vs {want}");
            }
        }
        // nothing inside the inner radius, nothing past the wall
        assert!(at(off + span * 0.5, rules.sp_inner - 0.05).is_none());
        assert!(at(off + span * 0.5, rules.sp_reach() + 0.3).is_none());
        // the slope on the walking line is the pitch Godot walks (a 5 m rise over 270 degrees)
        let slope = rules.rise_total / (span * rules.sp_walk);
        assert!((slope - 0.663).abs() < 1e-3, "{slope}");
    }
}

#[test]
fn the_spiral_has_the_slab_in_the_wedge_and_a_drop_in_the_bottom_corner() {
    let rules = flights();
    for run in RUNS {
        for cw in [false, true] {
            let f = stairs(FlightShape::Spiral, run, cw);
            let (uc, vc) = spiral_centre(rules, &f);
            let dir = spiral_dir(rules, &f);
            let hi = f.lower_y() + STOREY;
            // in front of the post, past the landing row: the slab, upper floor to the upper level, plain floor to the lower
            let (u, v) = (0.4, vc);
            assert_eq!(spiral_surface(rules, &f, u, v), Some((hi, SpiralLayer::Slab)));
            let (x, z) = to_world(&f, u, v);
            assert_eq!(flight_walk_y_with(rules, &f, x, z, false), Some(hi), "{run:?} cw {cw}: upper floor");
            assert_eq!(flight_walk_y_with(rules, &f, x, z, true), Some(f.lower_y()), "{run:?} cw {cw}: the lower floor under it");
            // the landing row's corner cell on the arc's start side (the top end, side -dir) is floored by the slab; the one on
            // the bottom end's side (+dir) is the open drop
            let corner = |side: f32| (-0.5, vc + side * (uc + 0.5 - 0.2));
            let (cu, cv) = corner(-dir);
            assert_eq!(spiral_surface(rules, &f, cu, cv), Some((hi, SpiralLayer::Slab)), "{run:?} cw {cw}: top-side corner slab");
            let (cu, cv) = corner(dir);
            assert_eq!(spiral_surface(rules, &f, cu, cv), None, "{run:?} cw {cw}: bottom-side corner is open");
            let (x, z) = to_world(&f, cu, cv);
            assert_eq!(flight_walk_y_with(rules, &f, x, z, false), None, "{run:?} cw {cw}: a drop for the upper level");
            assert_eq!(flight_walk_y_with(rules, &f, x, z, true), Some(f.lower_y()), "{run:?} cw {cw}: floor for the lower level");
        }
    }
}

/// The spiral's surface has no cliff: two surface points a few centimetres apart differ by no more than the steepest
/// slope allows (the inner edge of the helicoid), except across the end radial that borders the slab, where the upper
/// floor sits a storey above the lowest step.
#[test]
fn the_spiral_surface_has_no_cliffs_but_the_end_radial() {
    let rules = flights();
    let steepest = rules.rise_total / (rules.sp_span() * rules.sp_inner);
    let h = 0.04f32;
    for cw in [false, true] {
        let f = stairs(FlightShape::Spiral, (0, 1), cw);
        let (_, vc) = spiral_centre(rules, &f);
        let lo = f.lower_y();
        let at = |i: i32, j: i32| spiral_surface(rules, &f, -1.5 + i as f32 * h + 0.003, vc - 3.5 + j as f32 * h + 0.003);
        let (mut across_radial, mut samples) = (0, 0);
        for i in 0..160 {
            for j in 0..180 {
                let Some((y, layer)) = at(i, j) else { continue };
                samples += 1;
                for (di, dj) in [(1, 0), (0, 1)] {
                    if let Some((n, nl)) = at(i + di, j + dj) {
                        if (y - n).abs() <= steepest * h * 1.5 + 1e-3 {
                            continue;
                        }
                        let (helix_y, slab_y) = match (layer, nl) {
                            (SpiralLayer::Helix, SpiralLayer::Slab) => (y, n),
                            (SpiralLayer::Slab, SpiralLayer::Helix) => (n, y),
                            _ => panic!("cw {cw}: a cliff of {} inside one layer at ({i}, {j})", (y - n).abs()),
                        };
                        assert!(helix_y < lo + 0.6 && (slab_y - (lo + STOREY)).abs() < 1e-4, "cw {cw}: a cliff off the end radial: {helix_y} / {slab_y}");
                        across_radial += 1;
                    }
                }
            }
        }
        assert!(samples > 2000, "cw {cw}: only {samples} samples");
        assert!(across_radial > 10, "cw {cw}: the end radial is the one drop ({across_radial})");
    }
}

#[test]
fn the_spiral_rail_and_guard_stand_where_godot_puts_them() {
    let rules = flights();
    for run in RUNS {
        for cw in [false, true] {
            let f = stairs(FlightShape::Spiral, run, cw);
            let (uc, vc) = spiral_centre(rules, &f);
            let dir = spiral_dir(rules, &f);
            // the rail along the end radial runs from just outside the post to the landing row's inner edge
            let rail = spiral_slab_rail_with(rules, &f).expect("a spiral has the slab rail");
            let (ua, va) = to_local(&f, rail.a.0, rail.a.1);
            let (ub, vb) = to_local(&f, rail.b.0, rail.b.1);
            assert!(((ua - uc).hypot(va - vc) - rules.sp_post).abs() < 1e-3, "{run:?} cw {cw}: starts at the post");
            assert!(ub.abs() < 1e-3 && (vb - (vc + dir * 1.5)).abs() < 1e-3, "{run:?} cw {cw}: ends on the landing row's edge at {ub},{vb}");
            // the guard includes the landing row's end beside B, on the open side (the whole edge, a metre)
            let g = landing_guards_with(rules, &f);
            let foot = well_footprint_with(rules, &f).foot;
            let at_b: Vec<_> = g.iter().filter(|g| g.outside == foot).collect();
            assert_eq!(at_b.len(), 1, "{run:?} cw {cw}: one guard beside B");
            let ((ua, gv), (ub, gw)) = (to_local(&f, at_b[0].seg.a.0, at_b[0].seg.a.1), to_local(&f, at_b[0].seg.b.0, at_b[0].seg.b.1));
            assert!((gv - (vc + dir * 1.5)).abs() < 1e-3 && (gw - gv).abs() < 1e-3, "{run:?} cw {cw}: along the landing row's end");
            assert!((ua - ub).abs() > 0.99 && ua.min(ub) > -1.001 && ua.max(ub) < 0.001, "{run:?} cw {cw}: from u -1 to 0");
            // the helix rail winds round the post at inner_radius + 0.12, 0.95 over the tread
            let rails = handrails_with(rules, &f);
            assert_eq!(rails.len(), 1);
            let pts = &rails[0].points;
            assert_eq!(pts.len() as i32, rules.sp_steps + 1, "27 steps make 28 points");
            let (cx, cz) = to_world(&f, uc, vc);
            for (i, p) in pts.iter().enumerate() {
                let r = (p[0] - cx).hypot(p[2] - cz);
                assert!((r - 0.67).abs() < 1e-3, "point {i} at radius {r}");
                let t = i as f32 / rules.sp_steps as f32;
                assert!((p[1] - (STOREY * (1.0 - t) + 0.95)).abs() < 1e-3, "point {i} at height {}", p[1]);
            }
        }
    }
}

#[test]
fn the_spiral_start_offset_is_data_and_validated() {
    let rules = flights();
    assert_eq!(rules.sp_start_offset_deg, 45.0, "Godot's 45 degrees");
    assert!((rules.sp_start_offset_deg * 2.0 + rules.sp_turns * 360.0 - 360.0).abs() < 1e-3, "45 + 270 + 45 closes the circle");
    let mut bad = rules.clone();
    bad.sp_start_offset_deg = 120.0;
    assert!(bad.validate().unwrap_err().contains("start_offset_deg"));
    bad.sp_start_offset_deg = -5.0;
    assert!(bad.validate().unwrap_err().contains("start_offset_deg"));
    // an offset the circle cannot hold: 80 + 270 + 80 overruns it
    bad.sp_start_offset_deg = 80.0;
    assert!(bad.validate().unwrap_err().contains("overruns"));
}

// ---------------------------------------------------------------------------------------------------------------------
// Dressing
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_switchback_is_railed_on_both_edges_of_both_flights_and_guarded_but_for_the_head_of_the_stairs() {
    let rules = flights();
    for run in RUNS {
        let f = stairs(FlightShape::Switchback, run, false);
        // four handrails, 0.95 over the tread, one on each edge of each flight
        let rails = handrails_with(rules, &f);
        assert_eq!(rails.len(), 4);
        let ramp = rules.sb_ramp();
        for r in &rails {
            assert_eq!(r.points.len(), 2);
            let (a, b) = (r.points[0], r.points[1]);
            let (ua, va) = to_local(&f, a[0], a[2]);
            let (ub, vb) = to_local(&f, b[0], b[2]);
            assert!(ua.abs() < 1e-3 && (ub - ramp).abs() < 1e-3 && (va - vb).abs() < 1e-3);
            // the tread a hair inside the ramp (its edge cell at the landing row is the level's flat floor)
            let tread = |u: f32| {
                let (x, z) = to_world(&f, u.clamp(0.002, ramp - 0.002), va);
                flight_surface_y(&f, x, z).unwrap() - f.lower_y()
            };
            assert!((a[1] - (tread(0.0) + 0.95)).abs() < 5e-3 && (b[1] - (tread(ramp) + 0.95)).abs() < 5e-3);
        }
        let vs: Vec<f32> = rails.iter().map(|r| to_local(&f, r.points[0][0], r.points[0][2]).1).collect();
        for want in [0.06, 1.94, -2.94, -1.06] {
            assert!(vs.iter().any(|v| (v - want).abs() < 1e-3), "{run:?}: a rail at v {want} in {vs:?}");
        }
        // the guard: the landing row's inner edge over the leaving flight and the divider (3 cells), none over the arriving flight
        let guards = landing_guards_with(rules, &f);
        assert_eq!(guards.len(), 3);
        for g in &guards {
            let (ua, va) = to_local(&f, g.seg.a.0, g.seg.a.1);
            let (ub, vb) = to_local(&f, g.seg.b.0, g.seg.b.1);
            assert!(ua.abs() < 1e-3 && ub.abs() < 1e-3, "{run:?}: along the landing row's inner edge");
            assert!(va.max(vb) <= 0.0 + 1e-3 && (vb - va).abs() > 0.5, "{run:?}: nothing over the arriving flight (v >= 0)");
            let fp = well_footprint_with(rules, &f);
            assert!(fp.upper_clear.contains(&g.outside) && !fp.upper_hole.contains(&g.outside));
        }
    }
}

/// Whether the grid point `p` lies on the guard's segment, its length widened by `slack` metres (negative: shrunk); guards
/// run along cell edges, so they are axis-aligned.
fn on_guard(g: &Guard, p: (f32, f32), slack: f32) -> bool {
    let along_x = (g.seg.b.0 - g.seg.a.0).abs() > (g.seg.b.1 - g.seg.a.1).abs();
    let ((lo_x, hi_x), (lo_z, hi_z)) = (
        (g.seg.a.0.min(g.seg.b.0), g.seg.a.0.max(g.seg.b.0)),
        (g.seg.a.1.min(g.seg.b.1), g.seg.a.1.max(g.seg.b.1)),
    );
    let (sx, sz) = if along_x { (slack, 1e-3) } else { (1e-3, slack) };
    p.0 >= lo_x - sx && p.0 <= hi_x + sx && p.1 >= lo_z - sz && p.1 <= hi_z + sz
}

/// Every point of a spiral well's rim, as (the point on the cell edge, the point just inside the well, the outside cell).
fn rim_points(rules: &FlightRules, f: &Flight) -> Vec<((f32, f32), (f32, f32), Cell)> {
    let hole = well_footprint_with(rules, f).upper_hole;
    let mut out = Vec::new();
    for c in &hole {
        for d in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n = Cell::new(c.x + d.0, c.y + d.1);
            if hole.contains(&n) {
                continue;
            }
            for i in 0..40 {
                let t = (i as f32 + 0.5) / 40.0;
                let (x0, z0) = (c.x as f32, c.y as f32);
                let p = match d {
                    (1, 0) => (x0 + 1.0, z0 + t),
                    (-1, 0) => (x0, z0 + t),
                    (0, 1) => (x0 + t, z0 + 1.0),
                    _ => (x0 + t, z0),
                };
                out.push((p, (p.0 - d.0 as f32 * 0.005, p.1 - d.1 as f32 * 0.005), n));
            }
        }
    }
    out
}

/// The spiral's rim is guarded wherever the collision stops a body and nowhere else: the whole outer edge of the 5 x 5
/// well, the open corner on the arc's end side, and not the doorway edge or the corner floored by the slab. The rim is
/// read through the very rule the hole rule of `Facility::body_fits_at_y` applies (the surface within the fit tolerance of
/// the upper floor), so the guard and the invisible wall cannot drift apart.
#[test]
fn a_spiral_is_guarded_wherever_the_collision_closes_its_rim() {
    let rules = flights();
    let tol = crate::world::facility::FLIGHT_FIT_TOL;
    for run in RUNS {
        for cw in [false, true] {
            let f = stairs(FlightShape::Spiral, run, cw);
            let fp = well_footprint_with(rules, &f);
            let guards = landing_guards_with(rules, &f);
            let (uc, vc) = spiral_centre(rules, &f);
            let (mut closed, mut open, mut open_guarded, mut outer) = (0, 0, 0, 0);
            for (p, q, n) in rim_points(rules, &f) {
                let walks = |x: f32, z: f32| flight_walk_y_with(rules, &f, x, z, false).is_some_and(|y| (y - f.upper_y()).abs() <= tol);
                let walkable = walks(q.0, q.1);
                // a closed sliver a few cm wide beside an open stretch (the well's 2 cm outer strip crossing a doorway edge) is no wall
                let sliver = !walkable && [(0.03, 0.0), (-0.03, 0.0), (0.0, 0.03), (0.0, -0.03)].iter().any(|d| walks(q.0 + d.0, q.1 + d.1));
                // the guards are read on a 5 cm grid: a point within that of a guard end counts as on it
                let guarded = guards.iter().any(|g| g.outside == n && on_guard(g, p, 0.06));
                let well_inside = guards.iter().any(|g| g.outside == n && on_guard(g, p, -0.06));
                if sliver {
                    continue;
                } else if !walkable {
                    closed += 1;
                    assert!(guarded, "{run:?} cw {cw}: the collision closes the rim at {p:?} but no guard stands there");
                } else {
                    open += 1;
                    open_guarded += usize::from(well_inside);
                }
                // the outer edge of the well is closed to a body everywhere: its last 2 cm has no surface
                let (u, v) = to_local(&f, q.0, q.1);
                if (u - uc).abs() > rules.sp_reach() || (v - vc).abs() > rules.sp_reach() {
                    outer += 1;
                    assert!(guarded, "{run:?} cw {cw}: the outer edge at {p:?} is unguarded");
                }
            }
            assert_eq!(open_guarded, 0, "{run:?} cw {cw}: no rail where a body can walk over the rim");
            assert!(closed > 300 && outer > 300, "{run:?} cw {cw}: the rim is guarded all round ({closed} closed, {outer} outer samples)");
            // the arrival gap: the doorway edge is open, so the landing row's cells have no guard on their well side
            assert!(open >= 40, "{run:?} cw {cw}: an arrival gap ({open} open samples)");
            let doorway: Vec<Cell> = landing_row_with(rules, &f);
            for g in &guards {
                let (ua, _) = to_local(&f, g.seg.a.0, g.seg.a.1);
                let (ub, vb) = to_local(&f, g.seg.b.0, g.seg.b.1);
                assert!(!(ua.abs() < 1e-3 && ub.abs() < 1e-3 && (vb - vc).abs() < 1.5 - 1e-3), "{run:?} cw {cw}: a guard across the doorway");
                assert!(!fp.upper_hole.contains(&g.outside), "{run:?} cw {cw}: a guard stands on floor");
            }
            assert!(guards.iter().all(|g| g.outside != fp.landing), "{run:?} cw {cw}: the landing A is the arrival");
            assert!(doorway.contains(&fp.landing) && doorway.contains(&fp.foot));
            // a guard beside B closes the open corner on the arc's end side
            assert!(guards.iter().any(|g| g.outside == fp.foot), "{run:?} cw {cw}: the drop beside B is guarded");
        }
    }
}

#[test]
fn a_straight_flight_is_railed_on_both_sides_and_guarded_but_at_the_landing() {
    let rules = flights();
    let f = stairs(FlightShape::Straight, (0, 1), false);
    assert_eq!(handrails_with(rules, &f).len(), 2);
    let guards = landing_guards_with(rules, &f);
    assert!(!guards.is_empty());
    assert!(guards.iter().all(|g| g.outside != well_footprint_with(rules, &f).landing));
}

#[test]
fn ladders_and_drops_have_no_dressing() {
    for kind in [FlightKind::Ladder, FlightKind::Drop] {
        let f = Flight::new(kind, FlightShape::Switchback, 0, Cell::new(9, 9), (0, 1));
        assert!(handrails(&f).is_empty() && landing_guards(&f).is_empty() && signs(&f).is_empty());
        assert!(spiral_post(&f).is_none() && spiral_slab_rail(&f).is_none());
    }
}

#[test]
fn the_signs_name_both_levels_in_godots_gold_at_two_and_a_half_metres_but_a_spiral_has_none() {
    let rules = flights();
    assert_eq!(sign_text(1), "LEVEL +1");
    assert_eq!(sign_text(0), "LEVEL 0");
    assert_eq!(sign_text(-1), "LEVEL -1");
    let d = &rules.dressing;
    let gold = d.sign_colour;
    assert_eq!(gold.map(|c| (c * 255.0).round() as u8), [0xe8, 0xc2, 0x5a]);
    assert!((d.sign_y() - 2.1 * 1.25).abs() < 1e-5, "2.1 m in a 4 m storey is 2.625 m in a 5 m one");
    for shape in SHAPES {
        if shape == FlightShape::Spiral {
            for run in RUNS {
                assert!(signs_with(rules, &stairs(shape, run, false)).is_empty(), "no LEVEL sign on a spiral staircase");
            }
            continue;
        }
        for run in RUNS {
            for level in [1, 0, -2] {
                let f = Flight { upper_level: level, ..stairs(shape, run, false) };
                let signs = signs_with(rules, &f);
                assert_eq!(signs.len(), 2, "{shape:?}: one sign per level");
                assert!(signs[0].upper && !signs[1].upper);
                assert_eq!((signs[0].text.as_str(), signs[1].text.as_str()), (sign_text(level).as_str(), sign_text(level - 1).as_str()));
                for s in &signs {
                    assert!((s.y - d.sign_y()).abs() < 1e-5);
                    // the sign faces into the well (+v), a reader's right is perpendicular to it
                    let side = (run.1.abs() as f32, run.0.abs() as f32);
                    assert_eq!(s.normal, side);
                    assert!((s.normal.0 * s.right.0 + s.normal.1 * s.right.1).abs() < 1e-6);
                    assert!((s.right.0.hypot(s.right.1) - 1.0).abs() < 1e-6);
                    // standing on the wall plane, next to the landing, inside the chunk's cells
                    let (_, v) = to_local(&f, s.at.0, s.at.1);
                    assert!(v < 0.0 + 1.0, "{shape:?}: on the low-v side");
                }
            }
        }
    }
}

#[test]
fn the_stairwell_lamp_is_godots_warm_light_scaled_to_the_storey() {
    let rules = flights();
    let d = &rules.dressing;
    assert_eq!(d.lamp_colour.map(|c| (c * 255.0).round() as u8), [0xff, 0xd2, 0x9a]);
    assert!((d.lamp_energy - 0.7).abs() < 1e-6, "Godot's energy");
    assert!((d.lamp_energy() - 1.4).abs() < 1e-6, "the same light as a prop's: doubled for this renderer's falloff");
    assert!((d.lamp_range() - 7.0 * 1.25).abs() < 1e-5, "Godot's 7 m range in a 5 m storey");
    for shape in SHAPES {
        let f = stairs(shape, (0, 1), false);
        let (x, y, z) = lamp_place_with(rules, &f);
        assert!(y > STOREY * 0.5 && y < STOREY, "{shape:?}: hangs in the upper half of the shaft ({y})");
        let c = Cell::new(x.floor() as i32, z.floor() as i32);
        let fp = well_footprint_with(rules, &f);
        assert!(fp.upper_hole.contains(&c) || fp.upper_clear.contains(&c), "{shape:?}: over the well ({x}, {z})");
    }
}

#[test]
fn the_dressing_is_data_and_validated() {
    assert!(tweak(|v| v["dressing"]["sign"]["colour"] = serde_json::json!("nothex")).unwrap_err().contains("colour"));
    assert!(tweak(|v| v["dressing"]["sign"]["colour"] = serde_json::json!("e8c25")).unwrap_err().contains("colour"));
    assert!(tweak(|v| v["dressing"]["lamp"]["energy"] = serde_json::json!(0.0)).unwrap_err().contains("lamp.energy"));
    assert!(tweak(|v| v["dressing"]["godot_storey"] = serde_json::json!(0.0)).unwrap_err().contains("godot_storey"));
    // heights grow: handrail < guard < storey
    assert!(tweak(|v| v["dressing"]["guard"]["height"] = serde_json::json!(0.5)).unwrap_err().contains("dressing heights"));
    assert!(tweak(|v| v["dressing"]["handrail"]["height"] = serde_json::json!(0.0)).unwrap_err().contains("handrail.height"));
    // a sign cannot stand through the ceiling
    assert!(tweak(|v| v["dressing"]["sign"]["godot_height"] = serde_json::json!(3.9)).unwrap_err().contains("sign"));
    // data decides: a taller guard changes the number the renderer reads
    let taller = tweak(|v| v["dressing"]["guard"]["height"] = serde_json::json!(1.2)).unwrap();
    assert_eq!(taller.dressing.guard_height, 1.2);
    assert_eq!(flights().dressing.guard_height, 1.1);
    assert_eq!(flights().dressing.rail_height, 0.95);
}

// ---------------------------------------------------------------------------------------------------------------------
// Shape choice and data
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn shape_for_is_deterministic() {
    for seed in [1u64, 40490470, u64::MAX] {
        for level in [0, -1, -3] {
            for c in [Cell::new(0, 0), Cell::new(-7, 12), Cell::new(300, -40)] {
                assert_eq!(shape_for(seed, level, c), shape_for(seed, level, c));
                assert_eq!(spiral_clockwise_for(seed, level, c), spiral_clockwise_for(seed, level, c));
            }
        }
    }
}

#[test]
fn both_sides_of_a_stairwell_get_the_same_shape_and_spin() {
    let rules = flights();
    for seed in 0..60u64 {
        for upper in [1, 0, -1, -2, -3] {
            for c in [Cell::new(0, 0), Cell::new(-7, 12), Cell::new(33, 41), Cell::new(300, -40)] {
                let shape = shape_for_with(rules, seed, upper, c);
                let cw = spiral_clockwise_for(seed, upper, c);
                let top = Flight::new(FlightKind::Stairs, shape, upper, c, (0, 1)).with_clockwise(cw);
                let fp = well_footprint_with(rules, &top);
                // the lower side rebuilds the flight from its foot, then asks with the upper level and anchor it finds
                let low = Flight::from_foot_with(rules, FlightKind::Stairs, shape, upper - 1, fp.foot, (0, 1), cw);
                assert_eq!(low.upper_level, upper);
                assert_eq!(shape_for_with(rules, seed, low.upper_level, low.anchor_cell), shape, "seed {seed} upper {upper} {c:?}");
                assert_eq!(spiral_clockwise_for(seed, low.upper_level, low.anchor_cell), cw, "seed {seed} upper {upper} {c:?}");
            }
        }
    }
}

#[test]
fn steps_per_cell_has_one_source() {
    assert_eq!(flights().steps_per_cell, STEPS_PER_CELL as f32);
    let e = tweak(|v| v["steps_per_cell"] = serde_json::json!(4)).expect_err("a drifted table is refused");
    assert!(e.contains("STEPS_PER_CELL"), "{e}");
}

#[test]
fn the_limits_are_data_and_each_one_bites() {
    let r = flights();
    assert_eq!((r.limits.st_length.min, r.limits.st_length.max), (4.0, 16.0));
    assert_eq!((r.limits.margin_cells.min, r.limits.margin_cells.max), (0.0, 4.0));
    assert_eq!((r.limits.sp_half.min, r.limits.sp_half.max), (1.0, 4.0));
    assert_eq!((r.limits.max_riser.min, r.limits.max_riser.max), (0.05, 0.5));
    // (field path, a value past max, a value below min, label in the message)
    let cases: [(&[&str], f64, f64, &str); 14] = [
        (&["margin_cells"], 5.0, -1.0, "margin_cells"),
        (&["switchback", "length"], 13.0, 2.0, "switchback.length"),
        (&["switchback", "lane_width"], 4.0, 0.0, "switchback.lane_width"),
        (&["switchback", "divider_width"], 3.0, -1.0, "switchback.divider_width"),
        (&["switchback", "landing_depth"], 4.0, 0.0, "switchback.landing_depth"),
        (&["straight", "length"], 17.0, 3.0, "straight.length"),
        (&["straight", "width"], 5.0, 0.0, "straight.width"),
        (&["spiral", "half"], 5.0, 0.0, "spiral.half"),
        (&["spiral", "landing_cells"], 6.0, 0.0, "spiral.landing_cells"),
        (&["spiral", "inner_radius"], 1.6, 0.1, "spiral.inner_radius"),
        (&["spiral", "post_radius"], 1.1, 0.05, "spiral.post_radius"),
        (&["spiral", "walk_radius"], 7.0, 0.4, "spiral.walk_radius"),
        (&["spiral", "steps"], 73.0, 7.0, "spiral.steps"),
        (&["render", "tiles_per_cell", "spiral"], 9.0, 0.0, "render.tiles_per_cell"),
    ];
    let set = |v: &mut serde_json::Value, path: &[&str], n: f64| {
        let mut cur = v;
        for k in &path[..path.len() - 1] {
            cur = &mut cur[*k];
        }
        cur[path[path.len() - 1]] = serde_json::json!(n);
    };
    for (path, hi, lo, label) in cases {
        for n in [hi, lo] {
            let e = tweak(|v| set(v, path, n)).expect_err(label);
            assert!(e.contains(label), "{n}: {e:?} should mention {label:?}");
        }
    }
    for n in [2.6, -0.1] {
        let e = tweak(|v| v["slab"] = serde_json::json!(n)).expect_err("slab outside its limit");
        assert!(e.contains("slab"), "{e}");
    }
    for n in [0.6, 0.01] {
        let e = tweak(|v| v["max_riser"] = serde_json::json!(n)).expect_err("max_riser");
        assert!(e.contains("max_riser"), "{e}");
    }
    // data decides: a widened limit lets a longer straight flight through
    let longer = tweak(|v| {
        v["limits"]["straight_length"]["max"] = serde_json::json!(20);
        v["straight"]["length"] = serde_json::json!(18);
    });
    assert!(longer.is_ok(), "{longer:?}");
}

#[test]
fn a_missing_or_inverted_limit_is_refused() {
    let e = tweak(|v| {
        v.as_object_mut().unwrap().remove("limits");
    })
    .expect_err("no limits block");
    assert!(e.contains("limits"), "{e}");
    for name in [
        "slab",
        "max_riser",
        "margin_cells",
        "switchback_length",
        "switchback_lane_width",
        "switchback_divider_width",
        "switchback_landing_depth",
        "straight_length",
        "straight_width",
        "spiral_half",
        "spiral_landing_cells",
        "spiral_inner_radius",
        "spiral_post_radius",
        "spiral_walk_radius",
        "spiral_steps",
        "tiles_per_cell",
    ] {
        let e = tweak(|v| {
            v["limits"].as_object_mut().unwrap().remove(name);
        })
        .expect_err(name);
        assert!(e.contains(name), "{e}");
        let e = tweak(|v| {
            v["limits"][name].as_object_mut().unwrap().remove("max");
        })
        .expect_err(name);
        assert!(e.contains(name), "{e}");
        let e = tweak(|v| v["limits"][name] = serde_json::json!({"min": 9, "max": 1})).expect_err(name);
        assert!(e.contains("inverted") && e.contains(name), "{e}");
    }
}

fn share(rules: &FlightRules, level: i32) -> [f32; 3] {
    let mut count = [0u32; 3];
    let total = 6000;
    for i in 0..total {
        let shape = shape_for_with(rules, 777 + i as u64 / 40, level, Cell::new(i % 40, i / 40));
        count[SHAPES.iter().position(|s| *s == shape).unwrap()] += 1;
    }
    count.map(|c| c as f32 / total as f32)
}

#[test]
fn shape_for_follows_the_weights() {
    let rules = flights();
    let w = rules.weights_for(0);
    let t = w.total();
    let got = share(rules, 0);
    for (g, want) in got.iter().zip([w.switchback, w.straight, w.spiral]) {
        assert!((g - want / t).abs() < 0.04, "share {g} against weight {}", want / t);
    }
    // a level with its own weights uses them: the sewer has no spirals
    assert_eq!(share(rules, -1)[2], 0.0);
    // data decides: only spirals
    let only = tweak(|v| v["shape_weights"] = serde_json::json!({"switchback": 0.0, "straight": 0.0, "spiral": 1.0})).unwrap();
    assert_eq!(share(&only, 0), [0.0, 0.0, 1.0]);
}

#[test]
fn changing_the_data_changes_the_geometry() {
    let base = flights();
    let f = stairs(FlightShape::Switchback, (0, 1), false);
    let mid = |r: &FlightRules| {
        let (x, z) = to_world(&f, r.sb_length as f32 - r.sb_mid_len as f32 * 0.5, r.sb_lane_width as f32 * 0.5);
        flight_surface_y_with(r, &f, x, z).unwrap()
    };
    let higher = tweak(|v| v["switchback"]["mid_landing_rise"] = serde_json::json!(2.7)).unwrap();
    assert!(mid(&higher) > mid(base) + 0.15);

    // a narrower switchback: lane width 1 is a 3-wide well
    let narrow = tweak(|v| v["switchback"]["lane_width"] = serde_json::json!(1)).unwrap();
    assert_eq!(well_footprint_with(&narrow, &f).upper_hole.len(), 15);
    // no divider: the lanes meet and nothing is left to wall
    let open = tweak(|v| v["switchback"]["divider_width"] = serde_json::json!(0)).unwrap();
    assert!(lane_dividers_with(&open, &f).is_empty());
    assert_eq!(lane_dividers_with(base, &f).len(), 1);
    // a deeper mid landing makes a shorter ramp (a steeper one, still within the limit)
    let deeper = tweak(|v| {
        v["switchback"]["length"] = serde_json::json!(6);
    })
    .unwrap();
    assert_eq!(well_footprint_with(&deeper, &f).upper_hole.len(), 30);
    assert!((deeper.sb_ramp() - 4.0).abs() < 1e-6);

    let longer = tweak(|v| v["straight"]["length"] = serde_json::json!(10)).unwrap();
    let s = stairs(FlightShape::Straight, (0, 1), false);
    assert_eq!(well_footprint_with(base, &s).upper_hole.len(), 16);
    assert_eq!(well_footprint_with(&longer, &s).upper_hole.len(), 20);
    assert_eq!(well_footprint_with(&longer, &s).foot, Cell::new(20, 31));

    let wide = tweak(|v| v["spiral"]["half"] = serde_json::json!(3)).unwrap();
    let sp = stairs(FlightShape::Spiral, (0, 1), false);
    assert_eq!(well_footprint_with(&wide, &sp).upper_hole.len(), 7 * 7 - 3);

    let margin = tweak(|v| v["margin_cells"] = serde_json::json!(2)).unwrap();
    assert_eq!(well_footprint_with(&margin, &s).lower_clear.len(), 3);
    assert_eq!(well_footprint_with(base, &s).lower_clear.len(), 2);
}

#[test]
fn a_bad_table_is_refused_with_a_reason() {
    let bad = |r: Result<FlightRules, String>, want: &str| {
        let e = r.expect_err(want);
        assert!(e.contains(want), "{e:?} should mention {want:?}");
    };
    bad(FlightRules::parse("{ not json"), "not valid JSON");
    bad(tweak(|v| v["schema"] = serde_json::json!("other")), "schema");
    bad(tweak(|v| v["storey"] = serde_json::json!(4.0)), "STOREY");
    bad(tweak(|v| v["rise_total"] = serde_json::json!(4.0)), "rise_total");
    bad(tweak(|v| v["max_riser"] = serde_json::json!(0.1)), "riser");
    bad(tweak(|v| v["margin_cells"] = serde_json::json!(9)), "margin_cells");
    bad(tweak(|v| v["switchback"]["mid_landing_rise"] = serde_json::json!(5.0)), "mid_landing_rise");
    bad(tweak(|v| v["switchback"]["length"] = serde_json::json!(2)), "switchback.length");
    bad(tweak(|v| v["switchback"]["mid_landing_len"] = serde_json::json!(5)), "mid_landing_len");
    bad(tweak(|v| v["switchback"]["mid_landing_len"] = serde_json::json!(0)), "mid_landing_len");
    bad(tweak(|v| v["straight"]["length"] = serde_json::json!(40)), "straight.length");
    bad(tweak(|v| v["spiral"]["turns"] = serde_json::json!(0.6)), "spiral.turns");
    bad(tweak(|v| v["spiral"]["landing_cells"] = serde_json::json!(2)), "spiral.landing_cells");
    bad(tweak(|v| v["spiral"]["wall_clearance"] = serde_json::json!(0.6)), "wall_clearance");
    // the radii must grow from the post out to the wall
    bad(
        tweak(|v| {
            v["spiral"]["inner_radius"] = serde_json::json!(0.3);
            v["spiral"]["post_radius"] = serde_json::json!(0.4);
        }),
        "radii must grow",
    );
    bad(tweak(|v| v["spiral"]["walk_radius"] = serde_json::json!(2.6)), "radii must grow");
    bad(tweak(|v| v["shape_weights"] = serde_json::json!({"switchback": 0, "straight": 0, "spiral": 0})), "weight");
    bad(tweak(|v| v["shape_weights"]["spiral"] = serde_json::json!(-1.0)), "negative");
    bad(tweak(|v| v["region_weights"] = serde_json::json!({"x": {"switchback": 1, "straight": 0, "spiral": 0}})), "level number");
    bad(
        tweak(|v| {
            v["switchback"].as_object_mut().unwrap().remove("length");
        }),
        "switchback.length",
    );
    bad(
        tweak(|v| {
            v["switchback"].as_object_mut().unwrap().remove("divider_width");
        }),
        "switchback.divider_width",
    );
}

#[test]
fn the_slope_limit_is_checked_on_each_shapes_walking_line() {
    let rules = flights();
    // 5 m over a 3 m ramp is 0.278 m a tread, over 0.3 only by a hair of margin: a shorter ramp is refused
    assert!(max_riser_of(rules, FlightShape::Switchback) < rules.max_riser);
    let steep = tweak(|v| {
        v["switchback"]["length"] = serde_json::json!(4);
    });
    assert!(steep.unwrap_err().contains("riser"), "a 2-row ramp is too steep");
    // the spiral's pitch is measured on the walking line, not at the post
    let inner = rules.rise_total / (rules.sp_span() * rules.sp_inner) / rules.steps_per_cell;
    assert!(inner > rules.max_riser, "the post end is steeper than the limit ({inner}), by design");
    assert!(max_riser_of(rules, FlightShape::Spiral) < rules.max_riser);
}
