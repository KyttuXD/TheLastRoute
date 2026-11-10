//! The camera screen's minimap (port of `night_watch_cam_map.gd`): a 120x92 schematic with a CAM n button at the real
//! position of every camera room, routes (vents dashed), a YOU marker, hunter blips and a pulse ring. The geometry
//! (`compute`) is pure and tested; `draw` and `hit` use it.
#![allow(dead_code)]

use super::status::*;
use crate::core::{CamLayout, HudState};
use macroquad::prelude::*;

pub const SIZE: Vec2 = Vec2::new(120.0, 92.0);
pub const BTN: Vec2 = Vec2::new(30.0, 13.0);
pub const PAD: Vec2 = Vec2::new(3.0, 3.0);
/// Top left of the map on the 480x270 canvas (bottom right, 146 from the right edge, 128 from the bottom).
pub const MAP_POS: Vec2 = Vec2::new(334.0, 142.0);

const MAP_INK: Color = Color::new(0.8, 1.0, 0.85, 1.0);
const BLIP: Color = Color::new(1.0, 0.25, 0.2, 1.0);

#[derive(Clone, Debug, Default)]
pub struct MapGeometry {
    /// Every node, centre in map pixels.
    pub points: Vec<(String, Vec2)>,
    /// Camera buttons (centre in map pixels), in camera order.
    pub buttons: Vec<(String, Vec2)>,
    pub office: Vec2,
}

impl MapGeometry {
    pub fn point(&self, id: &str) -> Option<Vec2> {
        self.points.iter().find(|(r, _)| r == id).map(|(_, p)| *p)
    }
    pub fn button(&self, id: &str) -> Option<Vec2> {
        self.buttons.iter().find(|(r, _)| r == id).map(|(_, p)| *p)
    }
}

fn find(v: &[(String, Vec2)], id: &str) -> Option<Vec2> {
    v.iter().find(|(r, _)| r == id).map(|(_, p)| *p)
}

/// CamMap.setup: scale the rooms into the box, place the office, then push the buttons apart (relax) or grid-pack them.
pub fn compute(layout: &CamLayout) -> MapGeometry {
    if layout.rooms.is_empty() {
        return MapGeometry::default();
    }
    let mut min_p = Vec2::splat(f32::INFINITY);
    let mut max_p = Vec2::splat(f32::NEG_INFINITY);
    for (_, p) in &layout.rooms {
        min_p = min_p.min(*p);
        max_p = max_p.max(*p);
    }
    let span = (max_p - min_p).max(Vec2::ONE);
    let office = match layout.office {
        Some(o) => o,
        None => {
            let mut sx = 0.0;
            let mut n = 0;
            for e in &layout.entrances {
                if let Some(p) = find(&layout.rooms, e) {
                    sx += p.x;
                    n += 1;
                }
            }
            let x = if n > 0 { sx / (n as f32).max(1.0) } else { (min_p.x + max_p.x) * 0.5 };
            vec2(x, max_p.y + span.y * 0.3)
        }
    };
    min_p = min_p.min(office);
    max_p = max_p.max(office);
    let span = (max_p - min_p).max(Vec2::ONE);
    let inner = SIZE - BTN - PAD * 2.0 - vec2(0.0, 6.0);
    let origin = PAD + BTN * 0.5;
    let to_map = |p: Vec2| origin + (p - min_p) / span * inner;
    let mut points: Vec<(String, Vec2)> = layout.rooms.iter().map(|(id, p)| (id.clone(), to_map(*p))).collect();
    let office_point = to_map(office);
    let mut centres: Vec<(String, Vec2)> = Vec::new();
    for room in &layout.cams {
        if let Some(p) = find(&points, room) {
            if !centres.iter().any(|(r, _)| r == room) {
                centres.push((room.clone(), p));
            }
        }
    }
    relax(&mut centres);
    for (room, c) in &centres {
        if let Some(e) = points.iter_mut().find(|(r, _)| r == room) {
            e.1 = *c;
        }
    }
    // buttons in camera order
    let buttons: Vec<(String, Vec2)> = layout
        .cams
        .iter()
        .filter_map(|room| find(&centres, room).map(|c| (room.clone(), c)))
        .collect();
    MapGeometry { points, buttons, office: office_point }
}

fn relax(c: &mut Vec<(String, Vec2)>) {
    let lo = BTN * 0.5 + Vec2::ONE;
    let hi = SIZE - BTN * 0.5 - Vec2::ONE;
    let n = c.len();
    for _ in 0..200 {
        let mut moved = false;
        for i in 0..n {
            for j in (i + 1)..n {
                let a = c[i].1;
                let b = c[j].1;
                let d = b - a;
                let over_x = BTN.x + 1.0 - d.x.abs();
                let over_y = BTN.y + 1.0 - d.y.abs();
                if over_x > 0.0 && over_y > 0.0 {
                    moved = true;
                    if over_y < over_x {
                        let s = if d.y >= 0.0 { 1.0 } else { -1.0 };
                        c[i].1 = a - vec2(0.0, s * over_y * 0.5);
                        c[j].1 = b + vec2(0.0, s * over_y * 0.5);
                    } else {
                        let s = if d.x >= 0.0 { 1.0 } else { -1.0 };
                        c[i].1 = a - vec2(s * over_x * 0.5, 0.0);
                        c[j].1 = b + vec2(s * over_x * 0.5, 0.0);
                    }
                }
            }
        }
        for e in c.iter_mut() {
            e.1 = e.1.clamp(lo, hi);
        }
        if !moved {
            break;
        }
    }
    if overlaps(c) {
        pack(c);
    }
}

fn overlaps(c: &[(String, Vec2)]) -> bool {
    for i in 0..c.len() {
        for j in (i + 1)..c.len() {
            let d = c[i].1 - c[j].1;
            if d.x.abs() < BTN.x && d.y.abs() < BTN.y {
                return true;
            }
        }
    }
    false
}

/// Too crowded: lay the buttons on a grid in reading order (top to bottom, left to right).
fn pack(c: &mut Vec<(String, Vec2)>) {
    let n = c.len();
    // stable insertion sort with the Godot comparator (rows 6 px tall, then by x)
    let less = |pa: Vec2, pb: Vec2| pa.y < pb.y - 6.0 || ((pa.y - pb.y).abs() <= 6.0 && pa.x < pb.x);
    let mut order: Vec<usize> = (0..n).collect();
    for i in 1..n {
        let mut j = i;
        while j > 0 && less(c[order[j]].1, c[order[j - 1]].1) {
            order.swap(j, j - 1);
            j -= 1;
        }
    }
    let cols = (((SIZE.x - 2.0) / (BTN.x + 1.0)).floor() as usize).max(1);
    let rows = (n + cols - 1) / cols;
    let step_x = if cols > 1 { (SIZE.x - BTN.x - 2.0) / ((cols - 1) as f32).max(1.0) } else { 0.0 };
    let step_y = if rows > 1 {
        (BTN.y + 2.0).min((SIZE.y - BTN.y - 2.0) / ((rows - 1) as f32).max(1.0))
    } else {
        0.0
    };
    for (i, idx) in order.iter().enumerate() {
        c[*idx].1 = vec2(
            1.0 + BTN.x * 0.5 + (i % cols) as f32 * step_x,
            1.0 + BTN.y * 0.5 + (i / cols) as f32 * step_y,
        );
    }
}

/// Button rectangle in map pixels.
pub fn button_rect(centre: Vec2) -> Rect {
    Rect::new(centre.x - BTN.x * 0.5, centre.y - BTN.y * 0.5, BTN.x, BTN.y)
}

pub fn number_for(layout: &CamLayout, room: &str) -> usize {
    layout.cams.iter().position(|r| r == room).map(|i| i + 1).unwrap_or(0)
}

pub fn label_for<'a>(layout: &'a CamLayout, room: &str) -> &'a str {
    layout.labels.iter().find(|(r, _)| r == room).map(|(_, l)| l.as_str()).unwrap_or("")
}

/// "CAM n  LABEL" for a room of the layout.
pub fn title_for(layout: &CamLayout, room: &str) -> String {
    let n = number_for(layout, room);
    let l = label_for(layout, room).to_uppercase();
    if n == 0 {
        l
    } else {
        format!("CAM {}  {}", n, l)
    }
}

/// The camera button under the pointer (virtual canvas coordinates), if any.
pub fn hit(layout: &CamLayout, mouse: Vec2) -> Option<String> {
    let geo = compute(layout);
    let p = mouse - MAP_POS;
    geo.buttons.iter().find(|(_, c)| button_rect(*c).contains(p)).map(|(r, _)| r.clone())
}

/// Draw the map at MAP_POS. `mouse` is the pointer in virtual coordinates, `time` seconds for the pulse.
pub fn draw(st: &HudState, mouse: Vec2, time: f64) {
    let Some(layout) = &st.cam_map else { return };
    let geo = compute(layout);
    let at = |p: Vec2| MAP_POS + p;
    let area = Rect::new(MAP_POS.x, MAP_POS.y, SIZE.x, SIZE.y);
    fill(area, Color::new(0.04, 0.08, 0.06, 0.8));
    stroke(area, Color::new(0.25, 0.3, 0.27, 1.0), 1.0);
    let is_vent = |id: &str| layout.vents.iter().any(|v| v == id);
    for (a, b) in &layout.edges {
        let (Some(pa), Some(pb)) = (geo.point(a), geo.point(b)) else { continue };
        if is_vent(a) || is_vent(b) {
            dashed(at(pa), at(pb), Color::new(0.55, 0.8, 0.9, 0.8), 1.0, 2.0);
        } else {
            line(at(pa), at(pb), Color::new(0.5, 0.6, 0.52, 0.9), 1.0);
        }
    }
    for e in &layout.entrances {
        if let Some(p) = geo.point(e) {
            dashed(at(p), at(geo.office), Color::new(0.96, 0.72, 0.28, 0.8), 1.0, 2.0);
        }
    }
    for v in &layout.vents {
        if let Some(p) = geo.point(v) {
            if geo.button(v).is_none() {
                dot(at(p), 1.6, Color::new(0.55, 0.8, 0.9, 1.0));
            }
        }
    }
    // camera buttons
    let local_mouse = mouse - MAP_POS;
    for (i, (room, c)) in geo.buttons.iter().enumerate() {
        let r = button_rect(*c);
        let on = *room == st.monitor_room;
        let hov = r.contains(local_mouse);
        let bg = if on {
            Color::new(0.22, 0.17, 0.06, 0.95)
        } else if hov {
            Color::new(0.12, 0.16, 0.14, 0.95)
        } else {
            Color::new(0.05, 0.08, 0.07, 0.95)
        };
        let vr = Rect::new(MAP_POS.x + r.x, MAP_POS.y + r.y, r.w, r.h);
        fill(vr, bg);
        stroke(vr, if on { AMBER } else { Color::new(0.3, 0.38, 0.33, 1.0) }, 1.0);
        let n = layout.cams.iter().position(|x| x == room).map(|p| p + 1).unwrap_or(i + 1);
        vtext_center(&format!("CAM {}", n), vr.x + vr.w * 0.5, vr.y + (vr.h - 6.0) * 0.5 - 0.5, 6.0, if on { AMBER } else { MAP_INK });
    }
    // pulse rings (a sound or a movement the player should look at)
    if !st.cam_pulse.is_empty() {
        let a = 0.35 + 0.35 * (time * 6.0).sin() as f32;
        for room in &st.cam_pulse {
            if let Some(c) = geo.button(room) {
                let r = button_rect(c);
                let vr = Rect::new(MAP_POS.x + r.x - 1.0, MAP_POS.y + r.y - 1.0, r.w + 2.0, r.h + 2.0);
                stroke(vr, Color::new(0.96, 0.72, 0.28, a), 1.0);
            } else if let Some(p) = geo.point(room) {
                dot(at(p), 3.2, Color::new(0.96, 0.72, 0.28, a));
            }
        }
    }
    // hunter blips (reveal)
    for room in &st.cam_blips {
        if let Some(p) = geo.point(room) {
            let pos = if geo.button(room).is_some() { p + vec2(BTN.x * 0.5 - 2.0, -BTN.y * 0.5 + 2.0) } else { p };
            dot(at(pos), 2.2, BLIP);
        }
    }
    vtext_center("YOU", MAP_POS.x + geo.office.x, MAP_POS.y + geo.office.y - 3.0, 6.0, AMBER);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(rooms: &[(&str, f32, f32)], cams: &[&str]) -> CamLayout {
        CamLayout {
            rooms: rooms.iter().map(|(n, x, z)| (n.to_string(), vec2(*x, *z))).collect(),
            edges: vec![],
            entrances: vec![],
            cams: cams.iter().map(|s| s.to_string()).collect(),
            labels: vec![],
            vents: vec![],
            office: None,
        }
    }

    fn no_overlap(g: &MapGeometry) -> bool {
        for i in 0..g.buttons.len() {
            for j in (i + 1)..g.buttons.len() {
                let d = g.buttons[i].1 - g.buttons[j].1;
                if d.x.abs() < BTN.x && d.y.abs() < BTN.y {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn an_empty_layout_gives_no_geometry() {
        assert!(compute(&layout(&[], &[])).buttons.is_empty());
    }

    #[test]
    fn spread_rooms_keep_their_relative_positions_inside_the_box() {
        let l = layout(&[("a", 0.0, 0.0), ("b", 16.0, 0.0), ("c", 0.0, 16.0)], &["a", "b", "c"]);
        let g = compute(&l);
        assert_eq!(g.buttons.len(), 3);
        let a = g.button("a").unwrap();
        let b = g.button("b").unwrap();
        let c = g.button("c").unwrap();
        assert!(b.x > a.x && c.y > a.y);
        for (_, p) in &g.buttons {
            assert!(p.x >= BTN.x * 0.5 && p.x <= SIZE.x - BTN.x * 0.5);
            assert!(p.y >= BTN.y * 0.5 && p.y <= SIZE.y - BTN.y * 0.5);
        }
        assert!(no_overlap(&g));
    }

    #[test]
    fn the_office_sits_below_the_entrances() {
        let mut l = layout(&[("a", 0.0, 0.0), ("b", 10.0, 10.0)], &["a", "b"]);
        l.entrances = vec!["b".into()];
        let g = compute(&l);
        assert!(g.office.y > g.point("b").unwrap().y - 0.001 || g.office.y > g.point("a").unwrap().y);
    }

    #[test]
    fn rooms_on_one_spot_are_pushed_apart_without_overlap() {
        let rooms: Vec<(String, f32, f32)> = (0..8).map(|i| (format!("r{i}"), 1.0, 1.0)).collect();
        let refs: Vec<(&str, f32, f32)> = rooms.iter().map(|(n, x, z)| (n.as_str(), *x, *z)).collect();
        let cams: Vec<&str> = refs.iter().map(|r| r.0).collect();
        let g = compute(&layout(&refs, &cams));
        assert_eq!(g.buttons.len(), 8);
        assert!(no_overlap(&g));
    }

    #[test]
    fn the_pack_fallback_gives_non_overlapping_rects_inside_the_box() {
        let mut c: Vec<(String, Vec2)> = (0..12).map(|i| (format!("r{i}"), vec2(60.0, 46.0))).collect();
        pack(&mut c);
        let g = MapGeometry { points: vec![], buttons: c.clone(), office: Vec2::ZERO };
        assert!(no_overlap(&g));
        for (_, p) in &c {
            let r = button_rect(*p);
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= SIZE.x + 0.01 && r.y + r.h <= SIZE.y + 0.01);
        }
    }

    #[test]
    fn hit_finds_the_button_under_the_pointer() {
        let l = layout(&[("a", 0.0, 0.0), ("b", 16.0, 16.0)], &["a", "b"]);
        let g = compute(&l);
        let b = g.button("b").unwrap();
        assert_eq!(hit(&l, MAP_POS + b), Some("b".to_string()));
        assert_eq!(hit(&l, vec2(0.0, 0.0)), None);
    }

    #[test]
    fn numbers_and_titles_follow_camera_order() {
        let mut l = layout(&[("stage", 0.0, 0.0), ("dining", 8.0, 0.0)], &["stage", "dining"]);
        l.labels = vec![("stage".into(), "Show Stage".into())];
        assert_eq!(number_for(&l, "dining"), 2);
        assert_eq!(number_for(&l, "nowhere"), 0);
        assert_eq!(title_for(&l, "stage"), "CAM 1  SHOW STAGE");
    }
}
