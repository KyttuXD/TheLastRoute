//! HUNTER VIEW in the 3D view: the selected hunter's route line with waypoints and target, its sight
//! cone and hearing circle, and (with DEBUG VIEW NAV on) the blocked cells around it tinted by the
//! path fail reason. Lines only, drawn with the depth test off so a route behind a wall still shows.
//! What to draw comes from ui/admin/hunter_view.rs; the caller (hook in render/scene.rs) passes the
//! snapshot, the stats and the three DEBUG VIEW flags.

use crate::ui::admin::hunter_view::{self, HunterSnapshot};
use crate::world::facility::{Cell, Facility, PathFail, PathStats};
use macroquad::prelude::*;

/// Floor-plane lift so the lines do not z-fight the floor.
const LIFT: f32 = 0.08;

/// The tint of a blocked cell by the fail reason (None: no failure, plain walls).
pub fn blocked_tint(fail: Option<PathFail>) -> Color {
    match fail {
        Some(PathFail::DoorClosed) => Color::new(1.0, 0.6, 0.1, 0.5),
        Some(PathFail::PropCollider) => Color::new(0.9, 0.2, 0.9, 0.5),
        Some(PathFail::NoRoute) => Color::new(1.0, 0.1, 0.1, 0.5),
        None => Color::new(0.5, 0.5, 0.5, 0.25),
    }
}

/// The 3D points of a route: each cell centre at floor height.
pub fn route_points(path: &[Cell], y: f32) -> Vec<Vec3> {
    path.iter().map(|c| vec3(c.x as f32 + 0.5, y + LIFT, c.y as f32 + 0.5)).collect()
}

fn line_loop(points: &[(f32, f32)], y: f32, col: Color, close: bool) {
    for w in points.windows(2) {
        draw_line_3d(vec3(w[0].0, y, w[0].1), vec3(w[1].0, y, w[1].1), col);
    }
    if close && points.len() > 2 {
        let (a, b) = (points[points.len() - 1], points[0]);
        draw_line_3d(vec3(a.0, y, a.1), vec3(b.0, y, b.1), col);
    }
}

/// Draws the overlay for one hunter. `floor_y` is the floor height under it.
pub fn draw(fac: &Facility, s: &HunterSnapshot, stats: Option<&PathStats>, paths: bool, nav: bool, floor_y: f32) {
    let gl = unsafe { get_internal_gl() };
    gl.quad_gl.depth_test(false);
    let y = floor_y + LIFT;
    if paths {
        let pts = route_points(&s.path, floor_y);
        for w in pts.windows(2) {
            draw_line_3d(w[0], w[1], Color::new(0.2, 1.0, 0.4, 0.95));
        }
        for p in &pts {
            draw_sphere(*p, 0.08, None, Color::new(0.2, 1.0, 0.4, 1.0));
        }
        if let Some((tx, tz)) = s.target {
            draw_sphere(vec3(tx, y + 0.3, tz), 0.2, None, Color::new(1.0, 0.2, 0.2, 1.0));
        }
        if let Some((lx, lz)) = s.last_sighting {
            draw_sphere(vec3(lx, y + 0.3, lz), 0.15, None, Color::new(1.0, 0.9, 0.2, 1.0));
        }
    }
    line_loop(&hunter_view::sight_cone(s.pos, s.facing, s.sight), y, Color::new(1.0, 0.3, 0.3, 0.8), true);
    line_loop(&hunter_view::hearing_circle(s.pos, s.hearing), y, Color::new(0.3, 0.6, 1.0, 0.8), true);
    if nav {
        let tint = blocked_tint(stats.and_then(|p| p.fail));
        let centre = Cell::new(s.pos.0.floor() as i32, s.pos.1.floor() as i32);
        for c in hunter_view::blocked_cells(fac, centre) {
            draw_cube(vec3(c.x as f32 + 0.5, y, c.y as f32 + 0.5), vec3(0.9, 0.02, 0.9), None, tint);
        }
    }
    gl.quad_gl.depth_test(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_fail_reason_has_its_own_tint() {
        let tints = [
            blocked_tint(Some(PathFail::NoRoute)),
            blocked_tint(Some(PathFail::DoorClosed)),
            blocked_tint(Some(PathFail::PropCollider)),
            blocked_tint(None),
        ];
        for i in 0..tints.len() {
            for j in i + 1..tints.len() {
                assert_ne!(tints[i], tints[j]);
            }
        }
    }

    #[test]
    fn a_route_is_drawn_through_cell_centres() {
        let pts = route_points(&[Cell::new(2, 3), Cell::new(3, 3)], 0.0);
        assert_eq!(pts.len(), 2);
        assert_eq!((pts[0].x, pts[0].z), (2.5, 3.5));
        assert!(pts[0].y > 0.0);
    }
}
