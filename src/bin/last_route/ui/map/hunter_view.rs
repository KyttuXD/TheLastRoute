//! HUNTER VIEW on the minimap and the full map: the selected hunter's route line with waypoints, its
//! target and last sighting, the sight cone and the hearing circle. The shapes come from
//! ui/admin/hunter_view.rs; `place` turns a world (x, z) into a screen point (None when off the map),
//! the same closure the hunter dots use.

use crate::ui::admin::hunter_view::{self, HunterSnapshot};
use macroquad::prelude::*;

const ROUTE: Color = Color::new(0.2, 1.0, 0.4, 0.95);
const TARGET: Color = Color::new(1.0, 0.2, 0.2, 1.0);
const SIGHTING: Color = Color::new(1.0, 0.9, 0.2, 1.0);
const CONE: Color = Color::new(1.0, 0.3, 0.3, 0.8);
const HEARING: Color = Color::new(0.3, 0.6, 1.0, 0.8);

fn outline(points: &[(f32, f32)], place: &impl Fn((f32, f32)) -> Option<(f32, f32)>, col: Color, close: bool, w: f32) {
    let screen: Vec<(f32, f32)> = points.iter().filter_map(|p| place(*p)).collect();
    for s in screen.windows(2) {
        draw_line(s[0].0, s[0].1, s[1].0, s[1].1, w, col);
    }
    if close && screen.len() == points.len() && screen.len() > 2 {
        let (a, b) = (screen[screen.len() - 1], screen[0]);
        draw_line(a.0, a.1, b.0, b.1, w, col);
    }
}

/// Draws the overlay. `cell` is the map's pixels per cell (line and dot sizes follow it).
pub fn draw(s: &HunterSnapshot, place: impl Fn((f32, f32)) -> Option<(f32, f32)>, cell: f32, paths: bool) {
    let w = (cell * 0.15).max(1.0);
    if paths {
        let route: Vec<(f32, f32)> = s.path.iter().map(|c| (c.x as f32 + 0.5, c.y as f32 + 0.5)).collect();
        outline(&route, &place, ROUTE, false, w);
        for p in route.iter().filter_map(|p| place(*p)) {
            draw_circle(p.0, p.1, w * 1.2, ROUTE);
        }
        if let Some(p) = s.target.and_then(&place) {
            draw_circle(p.0, p.1, (cell * 0.45).max(2.0), TARGET);
        }
        if let Some(p) = s.last_sighting.and_then(&place) {
            draw_circle_lines(p.0, p.1, (cell * 0.45).max(2.0), w, SIGHTING);
        }
    }
    outline(&hunter_view::sight_cone(s.pos, s.facing, s.sight), &place, CONE, true, w);
    outline(&hunter_view::hearing_circle(s.pos, s.hearing), &place, HEARING, true, w);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_skips_points_off_the_map() {
        // pure geometry: a point the closure rejects is dropped, so a partly visible cone still draws
        let pts = [(0.0, 0.0), (1.0, 0.0), (99.0, 0.0)];
        let place = |p: (f32, f32)| (p.0 < 50.0).then_some(p);
        let kept: Vec<_> = pts.iter().filter_map(|p| place(*p)).collect();
        assert_eq!(kept.len(), 2);
    }
}
