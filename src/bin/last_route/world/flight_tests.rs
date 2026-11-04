//! Flight tests: footprints, the ladder step cell, the shape choice and the data (declared in flight.rs).

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
                assert_eq!(fp.upper_clear.len() as i32, rules.margin_cells + 1);
                for c in fp.upper_clear.iter().chain(&fp.lower_clear) {
                    assert!(!fp.upper_hole.contains(c), "{shape:?} {run:?} cw {cw}: clear cell {c:?} is in the hole");
                }
                assert_eq!(fp.step_cell, None);
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
                            fp.upper_hole.contains(&c) || c == fp.landing || c == fp.foot,
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
    assert_eq!(n(FlightShape::Switchback), 8, "2x4");
    assert_eq!(n(FlightShape::Straight), 16, "8 long, 2 wide");
    assert_eq!(n(FlightShape::Spiral), 25, "a 5x5 square around the centre column");
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
        let (x, z) = to_world(&f, r.sb_length as f32 - r.sb_mid_len * 0.5, 0.5);
        flight_surface_y_with(r, &f, x, z).unwrap()
    };
    let higher = tweak(|v| v["switchback"]["mid_landing_rise"] = serde_json::json!(3.0)).unwrap();
    assert!(mid(&higher) > mid(base) + 0.4);

    let longer = tweak(|v| v["straight"]["length"] = serde_json::json!(10)).unwrap();
    let s = stairs(FlightShape::Straight, (0, 1), false);
    assert_eq!(well_footprint_with(base, &s).upper_hole.len(), 16);
    assert_eq!(well_footprint_with(&longer, &s).upper_hole.len(), 20);
    assert_eq!(well_footprint_with(&longer, &s).foot, Cell::new(20, 31));

    let wide = tweak(|v| v["spiral"]["radius"] = serde_json::json!(3)).unwrap();
    let sp = stairs(FlightShape::Spiral, (0, 1), false);
    assert_eq!(well_footprint_with(&wide, &sp).upper_hole.len(), 49);

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
    bad(tweak(|v| v["straight"]["length"] = serde_json::json!(40)), "straight.length");
    bad(tweak(|v| v["spiral"]["turns"] = serde_json::json!(0.6)), "spiral.turns");
    bad(tweak(|v| v["spiral"]["radius"] = serde_json::json!(2.5)), "spiral.radius");
    bad(tweak(|v| v["spiral"]["lane_width"] = serde_json::json!(2)), "spiral.lane_width");
    bad(tweak(|v| v["shape_weights"] = serde_json::json!({"switchback": 0, "straight": 0, "spiral": 0})), "weight");
    bad(tweak(|v| v["shape_weights"]["spiral"] = serde_json::json!(-1.0)), "negative");
    bad(tweak(|v| v["region_weights"] = serde_json::json!({"x": {"switchback": 1, "straight": 0, "spiral": 0}})), "level number");
    bad(tweak(|v| { v["switchback"].as_object_mut().unwrap().remove("length"); }), "switchback.length");
}
