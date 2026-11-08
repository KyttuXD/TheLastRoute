//! The camera screen's minimap (port of `night_watch_cam_map.gd`): a 120x92 schematic with a CAM n button at the real
//! position of every camera room, routes (vents dashed), a YOU marker, hunter blips and a pulse ring. The geometry
//! (`compute`) is pure and tested; `draw` and `hit` use it.
#![allow(dead_code)]

use super::status::*;
use crate::core::{CamLayout, HudState};
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

pub const SIZE: Vec2 = Vec2::new(120.0, 92.0);
pub const BTN: Vec2 = Vec2::new(30.0, 13.0);
pub const PAD: Vec2 = Vec2::new(3.0, 3.0);
/// Top left of the map on the 480x270 canvas (bottom right, 146 from the right edge, 128 from the bottom).
pub const MAP_POS: Vec2 = Vec2::new(334.0, 142.0);

/// Clear space kept between two buttons, and between the lowest button and the YOU slot.
pub const GAP: f32 = 3.0;
/// The YOU slot: bottom centre of the panel, below every button.
pub const YOU_POS: Vec2 = Vec2::new(60.0, 86.0);
pub const YOU_HALF: Vec2 = Vec2::new(8.0, 3.5);

/// Where button centres may sit: inside the panel and above the YOU slot (with clearance).
fn btn_lo() -> Vec2 {
    BTN * 0.5 + Vec2::ONE
}
fn btn_hi() -> Vec2 {
    vec2(SIZE.x - BTN.x * 0.5 - 1.0, YOU_POS.y - YOU_HALF.y - GAP - BTN.y * 0.5)
}

/// The YOU slot rectangle in map pixels.
pub fn you_rect() -> Rect {
    Rect::new(YOU_POS.x - YOU_HALF.x, YOU_POS.y - YOU_HALF.y, YOU_HALF.x * 2.0, YOU_HALF.y * 2.0)
}

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

thread_local! {
    static CACHE: RefCell<Option<(String, MapGeometry)>> = const { RefCell::new(None) };
}

/// The geometry of a layout. Placing the buttons can search a little, so the last result is kept (draw and hit ask every frame).
pub fn compute(layout: &CamLayout) -> MapGeometry {
    let key = format!("{:?}", layout);
    CACHE.with(|c| {
        if let Some((k, g)) = &*c.borrow() {
            if *k == key {
                return g.clone();
            }
        }
        let g = compute_fresh(layout);
        *c.borrow_mut() = Some((key, g.clone()));
        g
    })
}

/// CamMap.setup: scale the rooms into the area above the YOU slot, push the buttons apart (relax) or grid-pack them, then
/// nudge them off every line that would run through a button it does not end at. The office is always the YOU slot at the
/// bottom centre.
fn compute_fresh(layout: &CamLayout) -> MapGeometry {
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
    let (lo, hi) = (btn_lo(), btn_hi());
    let to_map = |p: Vec2| lo + (p - min_p) / span * (hi - lo);
    let mut points: Vec<(String, Vec2)> = layout.rooms.iter().map(|(id, p)| (id.clone(), to_map(*p))).collect();
    let mut centres: Vec<(String, Vec2)> = Vec::new();
    for room in &layout.cams {
        if let Some(p) = find(&points, room) {
            if !centres.iter().any(|(r, _)| r == room) {
                centres.push((room.clone(), p));
            }
        }
    }
    let orig: Vec<Vec2> = centres.iter().map(|c| c.1).collect();
    relax(&mut centres);
    // the lines the map will draw: room links and the office links from the doorway rooms
    let mut segs: Vec<Seg> = Vec::new();
    {
        let button_index = |room: &str| centres.iter().position(|(r, _)| r == room);
        let end_of = |room: &str| -> Option<End> {
            match button_index(room) {
                Some(i) => Some(End::Btn(i)),
                None => find(&points, room).map(End::Pt),
            }
        };
        for (a, b) in &layout.edges {
            if let (Some(ea), Some(eb)) = (end_of(a), end_of(b)) {
                if matches!(ea, End::Btn(_)) || matches!(eb, End::Btn(_)) {
                    segs.push((ea, eb));
                }
            }
        }
        for e in &layout.entrances {
            if let Some(i) = button_index(e) {
                segs.push((End::Btn(i), End::Pt(YOU_POS)));
            }
        }
    }
    let mut pos: Vec<Vec2> = centres.iter().map(|c| c.1).collect();
    clear_lines(&mut pos, &orig, &segs);
    for (c, p) in centres.iter_mut().zip(&pos) {
        c.1 = *p;
    }
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
    MapGeometry { points, buttons, office: YOU_POS }
}

// ---- keeping lines clear of buttons ----------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum End {
    /// A camera button (index into the button list; it can move).
    Btn(usize),
    /// A fixed point (a room without a button, or the YOU slot).
    Pt(Vec2),
}
type Seg = (End, End);

fn end_at(e: End, pos: &[Vec2]) -> Vec2 {
    match e {
        End::Btn(i) => pos[i],
        End::Pt(p) => p,
    }
}

fn is_end(e: End, k: usize) -> bool {
    matches!(e, End::Btn(i) if i == k)
}

/// Does the segment a-b cross the button centred at c (the button rectangle plus one pixel)?
fn seg_hits_rect(a: Vec2, b: Vec2, c: Vec2) -> bool {
    let (x0, x1) = (c.x - BTN.x * 0.5 - 1.0, c.x + BTN.x * 0.5 + 1.0);
    let (y0, y1) = (c.y - BTN.y * 0.5 - 1.0, c.y + BTN.y * 0.5 + 1.0);
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x - x0), (d.x, x1 - a.x), (-d.y, a.y - y0), (d.y, y1 - a.y)] {
        if p.abs() < 1e-9 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return false;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }
    t0 <= t1
}

/// How many (line, button) pairs have a line running through a button it does not end at.
fn through(pos: &[Vec2], segs: &[Seg]) -> usize {
    let mut v = 0;
    for (ea, eb) in segs {
        let (a, b) = (end_at(*ea, pos), end_at(*eb, pos));
        for (k, c) in pos.iter().enumerate() {
            if is_end(*ea, k) || is_end(*eb, k) {
                continue;
            }
            if seg_hits_rect(a, b, *c) {
                v += 1;
            }
        }
    }
    v
}

fn too_close(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() < BTN.x + GAP - 0.01 && (a.y - b.y).abs() < BTN.y + GAP - 0.01
}

fn overlap_count(pos: &[Vec2]) -> usize {
    let mut n = 0;
    for i in 0..pos.len() {
        for j in (i + 1)..pos.len() {
            if too_close(pos[i], pos[j]) {
                n += 1;
            }
        }
    }
    n
}

/// Pairs that are clearly east/west or north/south of each other in `orig` but not in `p`.
fn order_viol(orig: &[Vec2], p: &[Vec2]) -> usize {
    let n = p.len().min(orig.len());
    let mut v = 0;
    for i in 0..n {
        for j in 0..n {
            if orig[j].x - orig[i].x > BTN.x * 0.6 && p[i].x > p[j].x {
                v += 1;
            }
            if orig[j].y - orig[i].y > BTN.y * 0.6 && p[i].y > p[j].y {
                v += 1;
            }
        }
    }
    v
}

/// Move buttons off any line that runs through a button it does not end at. First a local pass (move one button to the
/// nearest free spot that removes a crossing), then, when that is not enough, a small deterministic search. Moves never
/// overlap buttons, never leave the allowed area and never worsen the west/east and north/south order.
fn clear_lines(pos: &mut Vec<Vec2>, orig: &[Vec2], segs: &[Seg]) {
    let n = pos.len();
    if n == 0 || through(pos, segs) == 0 {
        return;
    }
    let (lo, hi) = (btn_lo(), btn_hi());
    for _ in 0..8 {
        if through(pos, segs) == 0 {
            break;
        }
        let mut bad: BTreeSet<usize> = BTreeSet::new();
        for (ea, eb) in segs {
            let (a, b) = (end_at(*ea, pos), end_at(*eb, pos));
            for k in 0..n {
                if is_end(*ea, k) || is_end(*eb, k) {
                    continue;
                }
                if seg_hits_rect(a, b, pos[k]) {
                    bad.insert(k);
                    for e in [ea, eb] {
                        if let End::Btn(i) = e {
                            bad.insert(*i);
                        }
                    }
                }
            }
        }
        let mut improved = false;
        for k in bad {
            let cur = through(pos, segs);
            if cur == 0 {
                break;
            }
            let base_ov = order_viol(orig, pos);
            let keep = pos[k];
            let mut best: Option<Vec2> = None;
            let mut best_v = cur;
            let mut best_d = f32::MAX;
            let mut x = lo.x;
            while x <= hi.x + 0.01 {
                let mut y = lo.y;
                while y <= hi.y + 0.01 {
                    let p = vec2(x, y);
                    y += 2.0;
                    if (0..n).any(|m| m != k && too_close(p, pos[m])) {
                        continue;
                    }
                    pos[k] = p;
                    let v = through(pos, segs);
                    let d = (p - keep).length_squared();
                    if (v < best_v || (v == best_v && best.is_some() && d < best_d)) && order_viol(orig, pos) <= base_ov {
                        best = Some(p);
                        best_v = v;
                        best_d = d;
                    }
                }
                x += 2.0;
            }
            match best {
                Some(p) if best_v < cur => {
                    pos[k] = p;
                    improved = true;
                }
                _ => pos[k] = keep,
            }
        }
        if !improved {
            break;
        }
    }
    if through(pos, segs) == 0 {
        return;
    }
    // simulated annealing from the current spots, fixed seeds so the map is the same every time
    let start = pos.clone();
    let ov0 = order_viol(orig, &start);
    let cost = |p: &[Vec2]| -> f32 {
        let d: f32 = p.iter().zip(&start).map(|(a, b)| (*a - *b).length()).sum();
        1000.0 * through(p, segs) as f32
            + 1000.0 * overlap_count(p) as f32
            + 1000.0 * order_viol(orig, p).saturating_sub(ov0) as f32
            + d * 0.5
    };
    let rnd = |seed: &mut u32| -> f32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed as f32 / 4294967296.0
    };
    const STEPS: usize = 20000;
    for attempt in 0..6u32 {
        let mut seed = 12345u32.wrapping_add(attempt * 7919);
        let mut cur = start.clone();
        let mut cc = cost(&cur);
        let mut best = cur.clone();
        let mut bc = cc;
        let mut found: Option<usize> = None;
        for it in 0..STEPS {
            let t = 200.0 * (1.0 - it as f32 / STEPS as f32) + 0.5;
            let k = ((rnd(&mut seed) * n as f32) as usize).min(n - 1);
            let old = cur[k];
            let p = if rnd(&mut seed) < 0.3 {
                vec2(lo.x + rnd(&mut seed) * (hi.x - lo.x), lo.y + rnd(&mut seed) * (hi.y - lo.y))
            } else {
                vec2(
                    (old.x + (rnd(&mut seed) - 0.5) * 30.0).clamp(lo.x, hi.x),
                    (old.y + (rnd(&mut seed) - 0.5) * 24.0).clamp(lo.y, hi.y),
                )
            };
            cur[k] = p;
            let c = cost(&cur);
            if c <= cc || rnd(&mut seed) < ((cc - c) / t).exp() {
                cc = c;
                if c < bc {
                    bc = c;
                    best = cur.clone();
                    if bc < 1000.0 && found.is_none() {
                        found = Some(it);
                    }
                }
            } else {
                cur[k] = old;
            }
            if let Some(f) = found {
                if it > f + 1500 {
                    break;
                }
            }
        }
        if bc < 1000.0 {
            *pos = best;
            return;
        }
    }
}

// ---- the layout from the nav graph ---------------------------------------------------------------------------------------------

/// Rooms that belong to the office side of a nav graph (doors, booth openings, vent mouths): never map points, only the
/// office link.
const OFFICE_SIDE: [&str; 8] = ["office", "officeL", "officeR", "doorL", "doorR", "ventL", "ventR", "ventT"];
const OFFICE_DOORS: [&str; 4] = ["doorL", "doorR", "officeL", "officeR"];

#[derive(Default)]
struct NavInfo {
    /// (id, centre on the x/z plane, size)
    rooms: Vec<(String, Vec2, Vec2)>,
    /// Room to room connections (symmetric): graph edges between nodes of different rooms and doorway `between` pairs.
    adj: BTreeMap<String, BTreeSet<String>>,
}

impl NavInfo {
    fn centre(&self, id: &str) -> Option<Vec2> {
        self.rooms.iter().find(|(r, _, _)| r == id).map(|(_, c, _)| *c)
    }
}

fn pair(v: &serde_json::Value) -> Option<Vec2> {
    let a = v.as_array()?;
    Some(vec2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

fn link_rooms(adj: &mut BTreeMap<String, BTreeSet<String>>, a: &str, b: &str) {
    if a.is_empty() || b.is_empty() || a == b {
        return;
    }
    adj.entry(a.to_string()).or_default().insert(b.to_string());
    adj.entry(b.to_string()).or_default().insert(a.to_string());
}

fn parse_nav(src: &str) -> Option<NavInfo> {
    let root: serde_json::Value = serde_json::from_str(src).ok()?;
    let mut nav = NavInfo::default();
    for (id, r) in root.get("rooms")?.as_object()? {
        let lo = pair(r.get("min")?)?;
        let hi = pair(r.get("max")?)?;
        nav.rooms.push((id.clone(), (lo + hi) * 0.5, (hi - lo).abs()));
    }
    let mut node_room: BTreeMap<String, String> = BTreeMap::new();
    for n in root.get("nodes")?.as_array()? {
        let Some(id) = n.get("id").and_then(|x| x.as_str()) else { continue };
        node_room.insert(id.to_string(), n.get("room").and_then(|x| x.as_str()).unwrap_or("").to_string());
        if let Some(b) = n.get("between").and_then(|x| x.as_array()) {
            if let (Some(a), Some(c)) = (b.first().and_then(|x| x.as_str()), b.get(1).and_then(|x| x.as_str())) {
                link_rooms(&mut nav.adj, a, c);
            }
        }
    }
    if let Some(edges) = root.get("edges").and_then(|x| x.as_array()) {
        for e in edges {
            let (Some(a), Some(b)) = (e.get(0).and_then(|x| x.as_str()), e.get(1).and_then(|x| x.as_str())) else { continue };
            if let (Some(ra), Some(rb)) = (node_room.get(a), node_room.get(b)) {
                link_rooms(&mut nav.adj, ra, rb);
            }
        }
    }
    Some(nav)
}

/// The camera numbers of a nav graph: its camera nodes' rooms in camera order.
pub fn nav_cams(nav_json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(nav_json) else { return Vec::new() };
    let mut v: Vec<(i64, String)> = Vec::new();
    if let Some(nodes) = root.get("nodes").and_then(|x| x.as_array()) {
        for n in nodes {
            if n.get("kind").and_then(|x| x.as_str()) == Some("camera") {
                let cam = n.get("cam").and_then(|x| x.as_i64()).unwrap_or(0);
                if let Some(room) = n.get("room").and_then(|x| x.as_str()) {
                    v.push((cam, room.to_string()));
                }
            }
        }
    }
    v.sort();
    v.into_iter().map(|(_, r)| r).collect()
}

/// The one layout builder every mode uses: take the mode's cameras, labels, vents and entrances (`base`) and place every CAM
/// room at its real spot (room centre on the x/z plane, from the mode's nav graph, `alias` = (camera room id, nav room id)
/// where the two differ), link rooms by the nav graph's real connections (through rooms without a camera when need be), put
/// the office where its rooms are, and mark vent and duct rooms. When the nav graph does not know every camera room, the
/// base layout is returned unchanged.
pub fn from_nav(base: &CamLayout, nav_json: &str, alias: &[(String, String)]) -> CamLayout {
    let Some(nav) = parse_nav(nav_json) else { return base.clone() };
    let nav_id = |c: &str| -> String { alias.iter().find(|(a, _)| a == c).map(|(_, n)| n.clone()).unwrap_or_else(|| c.to_string()) };
    if base.cams.is_empty() || base.cams.iter().any(|c| nav.centre(&nav_id(c)).is_none()) {
        return base.clone();
    }
    let cams_nav: Vec<(String, String)> = base.cams.iter().map(|c| (c.clone(), nav_id(c))).collect();
    let cam_of = |n: &str| -> Option<String> { cams_nav.iter().find(|(_, nn)| nn == n).map(|(c, _)| c.clone()) };
    let order = |c: &str| base.cams.iter().position(|x| x == c).unwrap_or(usize::MAX);

    let rooms: Vec<(String, Vec2)> = cams_nav.iter().filter_map(|(c, n)| nav.centre(n).map(|p| (c.clone(), p))).collect();

    let mut edge_set: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut derived_entrances: Vec<String> = Vec::new();
    for (c, n) in &cams_nav {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(n.clone());
        let mut stack: Vec<String> = nav.adj.get(n).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        while let Some(m) = stack.pop() {
            if !seen.insert(m.clone()) {
                continue;
            }
            if OFFICE_SIDE.contains(&m.as_str()) {
                if !derived_entrances.contains(c) {
                    derived_entrances.push(c.clone());
                }
            } else if let Some(c2) = cam_of(&m) {
                let (i, j) = (order(c), order(&c2));
                edge_set.insert((i.min(j), i.max(j)));
            } else if let Some(next) = nav.adj.get(&m) {
                stack.extend(next.iter().cloned());
            }
        }
    }
    let edges: Vec<(String, String)> = edge_set.iter().map(|(i, j)| (base.cams[*i].clone(), base.cams[*j].clone())).collect();

    // the real doorway rooms (those that reach the office side through the nav graph) win; the mode's own list is only
    // the fallback for a nav graph without office links
    let entrances: Vec<String> = if derived_entrances.is_empty() {
        base.entrances.iter().filter(|e| rooms.iter().any(|(r, _)| r == *e)).cloned().collect()
    } else {
        derived_entrances
    };

    let ventish = |s: &str| {
        let l = s.to_lowercase();
        l.contains("vent") || l.contains("duct") || l.contains("crawl")
    };
    let mut vents: Vec<String> = base.vents.iter().filter(|e| rooms.iter().any(|(r, _)| r == *e)).cloned().collect();
    for (c, n) in &cams_nav {
        if (ventish(c) || ventish(n)) && !vents.contains(c) {
            vents.push(c.clone());
        }
    }

    let office = if let Some(p) = nav.centre("office") {
        Some(p)
    } else {
        let doors: Vec<Vec2> = OFFICE_DOORS.iter().filter_map(|d| nav.centre(d)).collect();
        if doors.is_empty() {
            base.office
        } else {
            Some(doors.iter().fold(Vec2::ZERO, |a, b| a + *b) / doors.len() as f32)
        }
    };

    CamLayout { rooms, edges, entrances, cams: base.cams.clone(), labels: base.labels.clone(), vents, office }
}

fn relax(c: &mut Vec<(String, Vec2)>) {
    let lo = btn_lo();
    let hi = btn_hi();
    let n = c.len();
    for _ in 0..200 {
        let mut moved = false;
        for i in 0..n {
            for j in (i + 1)..n {
                let a = c[i].1;
                let b = c[j].1;
                let d = b - a;
                let over_x = BTN.x + GAP - d.x.abs();
                let over_y = BTN.y + GAP - d.y.abs();
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
            if too_close(c[i].1, c[j].1) {
                return true;
            }
        }
    }
    false
}

/// Too crowded: lay the buttons on a grid. Reading order (top to bottom, left to right) is the Godot layout; when it would
/// put a button on the wrong side of one that is clearly east/west or north/south of it, a column-major grid (columns by
/// x, rows by y inside a column) is used if it keeps the world's order better.
fn pack(c: &mut Vec<(String, Vec2)>) {
    let n = c.len();
    let orig: Vec<Vec2> = c.iter().map(|e| e.1).collect();
    // stable insertion sort with the Godot comparator (rows 6 px tall, then by x)
    let less = |pa: Vec2, pb: Vec2| pa.y < pb.y - 6.0 || ((pa.y - pb.y).abs() <= 6.0 && pa.x < pb.x);
    let mut order: Vec<usize> = (0..n).collect();
    for i in 1..n {
        let mut j = i;
        while j > 0 && less(orig[order[j]], orig[order[j - 1]]) {
            order.swap(j, j - 1);
            j -= 1;
        }
    }
    let cols = (((SIZE.x - 2.0) / (BTN.x + GAP)).floor() as usize).max(1);
    let rows = (n + cols - 1) / cols;
    let step_x = if cols > 1 { (SIZE.x - BTN.x - 2.0) / ((cols - 1) as f32).max(1.0) } else { 0.0 };
    let (lo, hi) = (btn_lo(), btn_hi());
    let step_y = if rows > 1 { (BTN.y + GAP).min((hi.y - lo.y) / ((rows - 1) as f32).max(1.0)) } else { 0.0 };
    let cell = |col: usize, row: usize| vec2(lo.x + col as f32 * step_x, lo.y + row as f32 * step_y);
    let mut reading = vec![Vec2::ZERO; n];
    for (i, idx) in order.iter().enumerate() {
        reading[*idx] = cell(i % cols, i / cols);
    }
    // column-major: sort by x, cut into columns of `rows`, sort each column by y
    let mut by_x: Vec<usize> = (0..n).collect();
    by_x.sort_by(|a, b| orig[*a].x.partial_cmp(&orig[*b].x).unwrap_or(std::cmp::Ordering::Equal));
    let mut column = vec![Vec2::ZERO; n];
    for (ci, chunk) in by_x.chunks(rows.max(1)).enumerate() {
        let mut col: Vec<usize> = chunk.to_vec();
        col.sort_by(|a, b| orig[*a].y.partial_cmp(&orig[*b].y).unwrap_or(std::cmp::Ordering::Equal));
        for (ri, idx) in col.iter().enumerate() {
            column[*idx] = cell(ci.min(cols - 1), ri);
        }
    }
    let pick = if order_viol(&orig, &column) < order_viol(&orig, &reading) { column } else { reading };
    for (i, e) in c.iter_mut().enumerate() {
        e.1 = pick[i];
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
    let yr = you_rect();
    let vy = Rect::new(MAP_POS.x + yr.x, MAP_POS.y + yr.y, yr.w, yr.h);
    fill(vy, Color::new(0.04, 0.08, 0.06, 0.95));
    stroke(vy, Color::new(0.5, 0.38, 0.14, 1.0), 1.0);
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

    const NAVS: [(&str, &str); 9] = [
        ("classic", include_str!("../data/nav/classic.json")),
        ("dayshift", include_str!("../data/nav/dayshift.json")),
        ("lure", include_str!("../data/nav/lure.json")),
        ("mask", include_str!("../data/nav/mask.json")),
        ("mixed", include_str!("../data/nav/mixed.json")),
        ("story", include_str!("../data/nav/story.json")),
        ("rooftop", include_str!("../data/nav/extra_rooftop.json")),
        ("campfire", include_str!("../data/nav/extra_campfire.json")),
        ("busstop", include_str!("../data/nav/extra_busstop.json")),
    ];

    /// The layout a mode would hand over: its cameras only (ids as in the nav graph).
    fn base_for(nav: &str) -> CamLayout {
        CamLayout { cams: nav_cams(nav), ..CamLayout::default() }
    }

    /// Node graph search that never passes through another camera room or the office side: is there a real way from room a
    /// to room b?
    fn connected_in_nav(src: &str, a: &str, b: &str, cams: &[String]) -> bool {
        let root: serde_json::Value = serde_json::from_str(src).unwrap();
        let mut room: BTreeMap<String, String> = BTreeMap::new();
        for n in root["nodes"].as_array().unwrap() {
            room.insert(n["id"].as_str().unwrap().to_string(), n["room"].as_str().unwrap_or("").to_string());
        }
        let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in root["edges"].as_array().unwrap() {
            let (x, y) = (e[0].as_str().unwrap().to_string(), e[1].as_str().unwrap().to_string());
            adj.entry(x.clone()).or_default().push(y.clone());
            adj.entry(y).or_default().push(x);
        }
        let allowed = |r: &str| r == a || r == b || (!cams.iter().any(|c| c == r) && !OFFICE_SIDE.contains(&r));
        let mut stack: Vec<String> = room.iter().filter(|(_, r)| r.as_str() == a).map(|(k, _)| k.clone()).collect();
        let mut seen: BTreeSet<String> = stack.iter().cloned().collect();
        while let Some(n) = stack.pop() {
            if room[&n] == b {
                return true;
            }
            for m in adj.get(&n).into_iter().flatten() {
                if room.get(m).map(|r| allowed(r)).unwrap_or(false) && seen.insert(m.clone()) {
                    stack.push(m.clone());
                }
            }
        }
        false
    }

    #[test]
    fn every_mode_places_every_camera_room_without_overlap() {
        for (name, src) in NAVS {
            let base = base_for(src);
            if base.cams.is_empty() {
                continue;
            }
            let l = from_nav(&base, src, &[]);
            let g = compute(&l);
            assert_eq!(g.buttons.len(), base.cams.len(), "{name}: a camera room has no button");
            for c in &base.cams {
                assert!(g.button(c).is_some(), "{name}: no button for {c}");
            }
            assert!(no_overlap(&g), "{name}: buttons overlap");
        }
    }

    #[test]
    fn buttons_keep_the_worlds_west_east_and_north_south() {
        for (name, src) in NAVS {
            let base = base_for(src);
            if base.cams.is_empty() {
                continue;
            }
            let nav = parse_nav(src).unwrap();
            let g = compute(&from_nav(&base, src, &[]));
            for a in &base.cams {
                for b in &base.cams {
                    let (_, ca, sa) = nav.rooms.iter().find(|(r, _, _)| r == a).unwrap().clone();
                    let (_, cb, sb) = nav.rooms.iter().find(|(r, _, _)| r == b).unwrap().clone();
                    let (pa, pb) = (g.button(a).unwrap(), g.button(b).unwrap());
                    if cb.x - ca.x > sa.x.max(sb.x) {
                        assert!(pa.x <= pb.x, "{name}: {a} is west of {b} but its button is not left");
                    }
                    if cb.y - ca.y > sa.y.max(sb.y) {
                        assert!(pa.y <= pb.y, "{name}: {a} is north of {b} but its button is not above");
                    }
                }
            }
        }
    }

    #[test]
    fn every_drawn_line_is_a_nav_connection() {
        for (name, src) in NAVS {
            let base = base_for(src);
            if base.cams.is_empty() {
                continue;
            }
            let l = from_nav(&base, src, &[]);
            for (a, b) in &l.edges {
                assert!(connected_in_nav(src, a, b, &base.cams), "{name}: line {a} - {b} is not in the nav graph");
            }
        }
    }

    #[test]
    fn the_main_modes_are_linked_and_have_an_office_link() {
        for (name, src) in NAVS.iter().take(6) {
            let l = from_nav(&base_for(src), src, &[]);
            assert!(!l.edges.is_empty(), "{name}: no lines");
            let linked = |r: &String| l.edges.iter().any(|(a, b)| a == r || b == r) || l.entrances.contains(r);
            for c in &l.cams {
                assert!(linked(c), "{name}: {c} has no line");
            }
        }
    }

    #[test]
    fn vents_are_marked_and_the_office_sits_between_the_ducts() {
        let src = NAVS[3].1; // mask
        let l = from_nav(&base_for(src), src, &[]);
        assert!(l.vents.iter().any(|v| v == "ductL") && l.vents.iter().any(|v| v == "crawl"));
        assert!(l.entrances.iter().any(|v| v == "ductL"));
        let office = l.office.expect("mask office");
        // the real map: the office sits between the two ducts, north of the crawl and south of the hall and ballroom
        let at = |id: &str| l.rooms.iter().find(|(r, _)| r == id).map(|(_, p)| *p).expect(id);
        assert!(at("ductL").x < office.x && office.x < at("ductR").x);
        assert!(at("hall").y < office.y && office.y < at("crawl").y);
    }

    #[test]
    fn an_unknown_camera_room_returns_the_base_layout() {
        let src = NAVS[0].1;
        let mut base = base_for(src);
        base.cams.push("nowhere".to_string());
        assert!(from_nav(&base, src, &[]).rooms.is_empty());
    }

    /// The layout a mode hands over, with the entrances a mode lists itself when its nav graph has no office links (Day
    /// Shift: the two gantries and the yard).
    fn layout_for(name: &str, src: &str) -> CamLayout {
        let mut base = base_for(src);
        if name == "dayshift" {
            base.entrances = vec!["gantryL".into(), "gantryR".into(), "yard".into()];
        }
        from_nav(&base, src, &[])
    }

    fn apart(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() >= BTN.x + GAP - 0.01 || (a.y - b.y).abs() >= BTN.y + GAP - 0.01
    }

    #[test]
    fn the_you_slot_is_the_bottom_centre_and_never_touches_a_button() {
        let you = you_rect();
        assert!((you.x + you.w * 0.5 - SIZE.x * 0.5).abs() < 0.01 && you.y + you.h <= SIZE.y);
        for (name, src) in NAVS {
            let l = layout_for(name, src);
            if l.cams.is_empty() {
                continue;
            }
            let g = compute(&l);
            assert_eq!(g.office, YOU_POS, "{name}: the office is not the YOU slot");
            for (room, c) in &g.buttons {
                let r = button_rect(*c);
                assert!(r.y + r.h + GAP <= you.y + 0.01, "{name}: {room} is too close to YOU");
                assert!(r.x >= 0.0 && r.x + r.w <= SIZE.x && r.y >= 0.0, "{name}: {room} leaves the panel");
            }
        }
    }

    #[test]
    fn buttons_keep_a_gap_in_every_mode() {
        for (name, src) in NAVS {
            let l = layout_for(name, src);
            let g = compute(&l);
            for i in 0..g.buttons.len() {
                for j in (i + 1)..g.buttons.len() {
                    assert!(apart(g.buttons[i].1, g.buttons[j].1), "{name}: {} and {} are too close", g.buttons[i].0, g.buttons[j].0);
                }
            }
        }
    }

    #[test]
    fn no_line_runs_through_a_button_it_does_not_end_at() {
        for (name, src) in NAVS {
            let l = layout_for(name, src);
            let g = compute(&l);
            let mut lines: Vec<(String, Vec2, Vec2)> = Vec::new();
            for (a, b) in &l.edges {
                if let (Some(pa), Some(pb)) = (g.point(a), g.point(b)) {
                    lines.push((format!("{a}-{b}"), pa, pb));
                }
            }
            for e in &l.entrances {
                if let Some(p) = g.point(e) {
                    lines.push((format!("{e}-YOU"), p, g.office));
                }
            }
            for (label, pa, pb) in &lines {
                for (room, c) in &g.buttons {
                    if *c == *pa || *c == *pb {
                        continue;
                    }
                    assert!(!seg_hits_rect(*pa, *pb, *c), "{name}: line {label} runs through {room}");
                }
            }
        }
    }

    #[test]
    fn the_doorway_rooms_link_to_you() {
        let src = NAVS[0].1; // classic: the two doors open on closet and corner
        let l = from_nav(&base_for(src), src, &[]);
        assert_eq!(l.entrances, vec!["closet".to_string(), "corner".to_string()]);
        let src = NAVS[2].1; // lure: the two vent mouths
        let l = from_nav(&base_for(src), src, &[]);
        assert!(l.entrances.contains(&"ventl".to_string()) && l.entrances.contains(&"ventr".to_string()));
    }

    #[test]
    fn a_line_through_a_button_is_seen_and_a_line_beside_it_is_not() {
        let c = vec2(60.0, 40.0);
        assert!(seg_hits_rect(vec2(60.0, 10.0), vec2(60.0, 80.0), c));
        assert!(!seg_hits_rect(vec2(10.0, 10.0), vec2(10.0, 80.0), c));
        assert!(!seg_hits_rect(vec2(60.0, 10.0), vec2(60.0, 20.0), c));
    }

    #[test]
    fn a_line_in_the_way_is_cleared() {
        // three rooms in a column: a-c would run through b
        let mut l = layout(&[("a", 0.0, 0.0), ("b", 0.0, 10.0), ("c", 0.0, 20.0)], &["a", "b", "c"]);
        l.edges = vec![("a".into(), "c".into())];
        let g = compute(&l);
        let (a, b, c) = (g.button("a").unwrap(), g.button("b").unwrap(), g.button("c").unwrap());
        assert!(!seg_hits_rect(a, c, b), "the a-c line still runs through b");
        assert!(apart(a, b) && apart(b, c) && apart(a, c));
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
