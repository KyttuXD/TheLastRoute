//! The bots and the sewer's channel railings (`game/rail_tests.rs` has the real chunks): a bot's straight-line sweep and
//! its routes stop at a railing, and the jump's gap stays open. A child of `game::npc` for its private `sweep` and `route_for`.

use super::*;
use crate::game::rail_tests::samples;
use crate::world::facility::{RAIL_E, RAIL_N, RAIL_S};

fn step_of(bit: u8) -> (f32, f32) {
    match bit {
        RAIL_N => (0.0, -1.0),
        RAIL_E => (1.0, 0.0),
        RAIL_S => (0.0, 1.0),
        _ => (-1.0, 0.0),
    }
}

#[test]
fn a_bots_sweep_and_routes_stop_at_a_sewer_railing() {
    let mut checked = 0;
    for s in samples().iter().take(10) {
        for (c, bit) in s.fac.rail_edges().iter().step_by(4) {
            let (cx, cz) = c.center();
            if !s.fac.body_fits(cx, cz, NAV_RADIUS) {
                continue;
            }
            let st = step_of(*bit);
            let behind = Cell::new(c.x + st.0 as i32, c.y + st.1 as i32);
            let beyond = (cx + st.0 * 1.5, cz + st.1 * 1.5);
            assert!(matches!(sweep(&s.fac, (cx, cz), beyond), Sweep::Blocked), "{}: the sweep crossed the rail at {c:?}", s.name);
            assert!(route_for(&s.fac, *c, behind, &[], false).is_empty(), "{}: a route into the fenced cell {behind:?}", s.name);
            assert!(route_for(&s.fac, *c, behind, &[], true).is_empty(), "{}: a fleeing route into {behind:?}", s.name);
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}

#[test]
fn a_bots_sweep_goes_through_the_jumps_gap() {
    let mut checked = 0;
    for s in samples() {
        let Some(g) = s.gap else { continue };
        for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
            let d = Cell::new(g.x + dx, g.y + dy);
            let code = s.grid[d.y as usize][d.x as usize];
            let (dc, gc) = (d.center(), g.center());
            if (code == crate::world::route::sewer::core::DRY || code == crate::world::route::sewer::core::BRIDGE)
                && s.fac.body_fits(dc.0, dc.1, NAV_RADIUS)
            {
                assert!(matches!(sweep(&s.fac, dc, gc), Sweep::Clear), "{}: the gap {g:?} is closed to a bot at {d:?}", s.name);
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}
