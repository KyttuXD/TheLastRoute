#![cfg(test)]
//! Camera coverage (contract 9.1: every mode with a monitor has cameras that actually show their rooms).
//!
//! For every camera of classic, mask, lure, mixed, dayshift, story and the three Open Air sets, every stand spot and task spot of
//! the camera's room (the nav graph's `stand` and `task` nodes, plus the modes' own stand slots) must project inside the camera's
//! view frustum (position, yaw, pitch, vertical FOV, the 16:9 feed) and not lie behind it. A room that has no stand or task node
//! is checked at its hub node instead, so a camera can never pass by having nothing to look at.
//!
//! A spot counts as seen when it lands inside the frame with a 5% margin (a hunter's feet at the very edge of the picture are
//! not "in view"), in front of the near plane and nearer than the feed's far plane.

use crate::gfx::FpCamera;
use crate::hunters::nav::{NavKind, NavMap};
use macroquad::prelude::{vec3, Vec3};

/// The feed target's aspect (`FeedTarget`, 480x270).
const ASPECT: f32 = 16.0 / 9.0;
/// The vertical FOV `FeedTarget::begin` uses for classic, lure and mixed.
const FEED_FOV_DEG: f32 = 62.0;
/// A spot must land within this fraction of the half-frame, so it is not cut by the border.
const EDGE: f32 = 0.95;
const NEAR: f32 = 0.05;
const FAR: f32 = 80.0;
/// A hunter stands at least this far from a room camera (metres), or she fills the picture and her head is cropped.
const MIN_DIST_ROOM: f32 = 2.5;
/// A hunter's head height for the framing check (the shortest of the cast stands about 1.6 m).
const HEAD_Y: f32 = 1.6;

/// One camera as the feed renders it.
struct View {
    label: String,
    pos: Vec3,
    target: Vec3,
    fov_deg: f32,
    /// The nav rooms this camera watches.
    rooms: Vec<String>,
    /// The nearest a stand or task spot may be to the camera (metres): nearer and the hunter fills the frame.
    min_dist: Option<f32>,
}

impl View {
    fn new(label: &str, pos: Vec3, target: Vec3, fov_deg: f32, rooms: &[&str]) -> View {
        View { label: label.to_string(), pos, target, fov_deg, rooms: rooms.iter().map(|r| r.to_string()).collect(), min_dist: None }
    }

    /// Also check how a hunter is framed: her feet at least `min_dist` metres (across the floor) from the camera, her head inside the
    /// frame. Rooms use `MIN_DIST_ROOM`, tighter spaces (ducts, the lift) a shorter distance.
    fn framed(mut self, min_dist: f32) -> View {
        self.min_dist = Some(min_dist);
        self
    }

    /// Why a hunter standing on `p` is badly framed: too near the camera, or her head is cut off by the frame.
    fn framing(&self, what: &str, p: Vec3) -> Option<String> {
        let min = self.min_dist?;
        let dist = ((p.x - self.pos.x).powi(2) + (p.z - self.pos.z).powi(2)).sqrt();
        if dist < min {
            return Some(format!("{}: {} is {:.2} m from the camera across the floor (minimum {:.1} m): the hunter fills the frame", self.label, what, dist, min));
        }
        let head = p + Vec3::Y * HEAD_Y;
        match self.project(head) {
            Some((x, y, _)) if x.abs() <= EDGE && y.abs() <= EDGE => None,
            _ => Some(format!("{}: {} puts the hunter's head outside the frame", self.label, what)),
        }
    }

    fn fp(&self) -> FpCamera {
        let d = self.target - self.pos;
        let yaw = (-d.x).atan2(-d.z);
        let pitch = d.y.atan2((d.x * d.x + d.z * d.z).sqrt());
        FpCamera { pos: self.pos, yaw, pitch, fov_deg: self.fov_deg, near: NEAR, far: FAR }
    }

    /// (x, y) in half-frame units (-1..1 is on screen) and the depth along the view, or None when the point is behind the camera.
    fn project(&self, p: Vec3) -> Option<(f32, f32, f32)> {
        let cam = self.fp();
        let f = cam.forward();
        let right = f.cross(Vec3::Y).normalize();
        let up = right.cross(f);
        let d = p - cam.pos;
        let depth = d.dot(f);
        if depth <= NEAR {
            return None;
        }
        let t = (cam.fov_deg * 0.5).to_radians().tan();
        Some((d.dot(right) / (depth * t * ASPECT), d.dot(up) / (depth * t), depth))
    }

    /// Why the point is not seen, or None when it is.
    fn problem(&self, what: &str, p: Vec3) -> Option<String> {
        match self.project(p) {
            None => Some(format!("{}: {} at ({:.2}, {:.2}, {:.2}) is behind the camera", self.label, what, p.x, p.y, p.z)),
            Some((x, y, depth)) => {
                if x.abs() > EDGE || y.abs() > EDGE || depth > FAR {
                    Some(format!(
                        "{}: {} at ({:.2}, {:.2}, {:.2}) is outside the frame (x {:.2}, y {:.2}, depth {:.1})",
                        self.label, what, p.x, p.y, p.z, x, y, depth
                    ))
                } else {
                    None
                }
            }
        }
    }
}

type Extra = Vec<(String, Vec3)>;

/// The stand and task spots of a room, or its hub node when it has neither.
fn room_spots(nav: &NavMap, room: &str) -> Vec<(String, Vec3)> {
    let mut v: Vec<(String, Vec3)> =
        nav.nodes().iter().filter(|n| n.room == room && (n.kind == NavKind::Stand || n.kind == NavKind::Task)).map(|n| (n.key.clone(), n.p)).collect();
    if v.is_empty() {
        if let Some(n) = nav.hub_key(room).and_then(|k| nav.node(&k)) {
            v.push((n.key.clone(), n.p));
        }
    }
    v
}

/// All problems of one view: every spot of its rooms, plus `extra` spots (the mode's own stand slots).
fn problems(view: &View, nav: &NavMap, extra: &[(String, Vec3)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut checked = 0;
    for room in &view.rooms {
        for (key, p) in room_spots(nav, room) {
            checked += 1;
            out.extend(view.problem(&format!("spot {key}"), p));
            out.extend(view.framing(&format!("spot {key}"), p));
        }
    }
    for (what, p) in extra {
        checked += 1;
        out.extend(view.problem(what, *p));
        out.extend(view.framing(what, *p));
    }
    if checked == 0 && !view.rooms.is_empty() {
        out.push(format!("{}: nothing to look at in {:?}", view.label, view.rooms));
    }
    out
}

fn nav_of(src: &str, mode: &str) -> NavMap {
    NavMap::from_json(src).unwrap_or_else(|e| panic!("{mode} nav: {e}"))
}

fn assert_covered(mode: &str, views: &[(View, Extra)], nav: &NavMap) {
    assert!(!views.is_empty(), "{mode} has no cameras");
    let mut all = Vec::new();
    for (v, extra) in views {
        all.extend(problems(v, nav, extra));
    }
    assert!(all.is_empty(), "{mode}: cameras that miss their rooms:\n  {}", all.join("\n  "));
}

fn slots(f: impl Fn(usize) -> Vec3) -> Extra {
    (0..2).map(|s| (format!("stand slot {s}"), f(s))).collect()
}

// ------------------------------------------------------------------------------------------------ the helper itself

#[test]
fn a_point_dead_ahead_is_in_the_middle_of_the_frame() {
    let v = View::new("t", vec3(0.0, 2.0, 0.0), vec3(0.0, 2.0, -10.0), 62.0, &[]);
    let (x, y, d) = v.project(vec3(0.0, 2.0, -5.0)).expect("ahead");
    assert!(x.abs() < 1e-5 && y.abs() < 1e-5 && (d - 5.0).abs() < 1e-4);
    assert!(v.problem("p", vec3(0.0, 2.0, -5.0)).is_none());
}

#[test]
fn a_point_behind_the_camera_is_rejected() {
    let v = View::new("t", vec3(0.0, 2.0, 0.0), vec3(0.0, 2.0, -10.0), 62.0, &[]);
    assert!(v.project(vec3(0.0, 2.0, 3.0)).is_none());
    assert!(v.problem("p", vec3(0.0, 2.0, 3.0)).expect("behind").contains("behind"));
}

#[test]
fn the_frame_edges_follow_the_fov_and_the_aspect() {
    let v = View::new("t", vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0), 62.0, &[]);
    let t = 31.0f32.to_radians().tan();
    // exactly on the vertical edge at depth 10
    let (_, y, _) = v.project(vec3(0.0, 10.0 * t, -10.0)).expect("ahead");
    assert!((y - 1.0).abs() < 1e-4);
    // the horizontal edge is 16/9 wider
    let (x, _, _) = v.project(vec3(10.0 * t * ASPECT, 0.0, -10.0)).expect("ahead");
    assert!((x - 1.0).abs() < 1e-4);
    // inside the margin is seen, past it is not
    assert!(v.problem("in", vec3(0.0, 10.0 * t * 0.9, -10.0)).is_none());
    assert!(v.problem("out", vec3(0.0, 10.0 * t * 0.99, -10.0)).is_some());
    assert!(v.problem("far", vec3(0.0, 0.0, -FAR - 1.0)).is_some());
}

#[test]
fn a_spot_right_under_a_high_camera_is_caught() {
    // the mask crawlspace camera as it was (Godot's aim): the locker spot under the mount fell below the picture
    let old = View::new("old crawl", vec3(11.5, 2.8, 4.2), vec3(0.0, 0.7, 7.5), 62.0, &["crawl"]);
    assert!(old.problem("locker", vec3(10.0, 0.0, 7.2)).is_some());
    // the lure lobby camera as it was: the pace spot near the wall
    let lobby = View::new("old lobby", vec3(-11.5, 2.8, -39.5), vec3(-8.0 + 8.0 * 0.12, 0.7, -37.5 + 5.0 * 0.12), 62.0, &["lobby"]);
    assert!(lobby.problem("pace", vec3(-10.8, 0.0, -36.6)).is_some());
}

#[test]
fn a_room_with_nothing_to_look_at_is_reported() {
    let nav = nav_of(r#"{"id":"t","rooms":{"a":{"min":[0,0],"max":[1,1]}},"nodes":[],"edges":[]}"#, "test");
    let v = View::new("empty", vec3(0.0, 3.0, 0.0), vec3(0.0, 0.0, -3.0), 62.0, &["a"]);
    assert!(problems(&v, &nav, &[]).iter().any(|p| p.contains("nothing to look at")));
}

// ------------------------------------------------------------------------------------------------ one test per mode

#[test]
fn classic_cameras_show_their_rooms() {
    use crate::gfx::Building;
    let nav = NavMap::classic();
    let rooms = crate::core::cam_rooms();
    assert_eq!(rooms.len(), 8);
    let views: Vec<(View, Extra)> = rooms
        .iter()
        .map(|r| {
            let (pos, target) = Building::camera_transform(r);
            (View::new(&format!("classic {r}"), pos, target, FEED_FOV_DEG, &[r.as_str()]), slots(|s| Building::stand_position(r, s)))
        })
        .collect();
    assert_covered("classic", &views, &nav);
}

#[test]
fn mask_cameras_show_their_rooms() {
    use super::mask::data::{map, rules, NAV_JSON};
    let nav = nav_of(NAV_JSON, "mask");
    let m = map();
    let views: Vec<(View, Extra)> = rules()
        .cam_rooms()
        .iter()
        .map(|r| {
            let c = m.camera(r).unwrap_or_else(|| panic!("mask camera room {r} has no camera"));
            // a hunter keeps her distance: 2.5 m in a room, 2 m in a duct, 1 m in the hall (she walks up to the office doors)
            let min = if r.starts_with("duct") { 2.0 } else if r == "hall" { 1.0 } else { MIN_DIST_ROOM };
            // the mode's own stand spots, and the duct waypoint the hunters walk through on their way to the vent (the one before it,
            // DL1 / DR1, is the side entrance from the crawlspace and is out of this camera's picture)
            let mut extra: Extra = Vec::new();
            for slot in 0..3 {
                for at in ["far", "stand", "near"] {
                    // (the hall's spots are by the office doors, under its high camera, and are not part of this feed's picture)
                    if r != "hall" && (at == "stand" || r.starts_with("duct")) {
                        extra.push((format!("stand slot {slot} ({at})"), m.stand_at(r, slot, at)));
                    }
                }
            }
            if r.starts_with("duct") {
                for n in nav.nodes().iter().filter(|n| n.room == *r && n.kind == NavKind::Waypoint && (n.key == "DL2" || n.key == "DR2")) {
                    extra.push((format!("duct waypoint {}", n.key), n.p));
                }
            }
            (View::new(&format!("mask {r}"), c.mount, c.target, c.fov, &[r.as_str()]).framed(min), extra)
        })
        .collect();
    assert_eq!(views.len(), 8);
    assert_covered("mask", &views, &nav);
}

#[test]
fn lure_cameras_show_their_rooms() {
    use super::lure::data::ROOMS;
    use super::lure::map::map;
    use super::lure::presence::LURE_NAV_JSON;
    let nav = nav_of(LURE_NAV_JSON, "lure");
    let m = map();
    let views: Vec<(View, Extra)> = ROOMS
        .iter()
        .map(|&sim| {
            let (pos, target) = m.camera_transform(sim);
            (View::new(&format!("lure {sim}"), pos, target, FEED_FOV_DEG, &[sim]), slots(|s| m.stand_position(sim, s)))
        })
        .collect();
    assert_covered("lure", &views, &nav);
}

#[test]
fn dayshift_cameras_show_their_rooms() {
    use super::dayshift::stage::load_nav;
    use super::dayshift::world::map;
    let nav = load_nav();
    let views: Vec<(View, Extra)> =
        map().cameras.iter().map(|c| (View::new(&format!("dayshift {}", c.room), c.pos, c.target, c.fov, &[c.room.as_str()]), Vec::new())).collect();
    assert_eq!(views.len(), 8);
    assert_covered("dayshift", &views, &nav);
}

#[test]
fn story_cameras_show_their_rooms() {
    use super::story::world::{layout, NAV_JSON};
    let nav = nav_of(NAV_JSON, "story");
    let views: Vec<(View, Extra)> =
        layout().cameras.iter().map(|c| (View::new(&format!("story {}", c.room), c.pos, c.target, c.fov, &[c.room.as_str()]).framed(MIN_DIST_ROOM), Vec::new())).collect();
    assert_eq!(views.len(), 6);
    assert_covered("story", &views, &nav);
}

#[test]
fn openair_cameras_show_their_rooms() {
    use super::openair::data::maps;
    use super::openair::world::feed_position;
    assert_eq!(maps().len(), 3);
    for w in maps() {
        let nav = nav_of(w.nav_json, &w.id);
        let views: Vec<(View, Extra)> = w
            .cameras
            .iter()
            .map(|c| {
                let rooms: Vec<&str> = c.rooms.iter().map(|r| r.as_str()).collect();
                (View::new(&format!("openair {} {}", w.id, c.id), feed_position(c), c.target, c.fov, &rooms), Vec::new())
            })
            .collect();
        assert_covered(&format!("openair {}", w.id), &views, &nav);
        // every stand and task spot of the set is on some camera's list of rooms (a camera with no rooms watches the open set)
        for n in nav.nodes().iter().filter(|n| n.kind == NavKind::Stand || n.kind == NavKind::Task) {
            assert!(w.cameras.iter().any(|c| c.rooms.contains(&n.room)), "openair {}: spot {} (room {}) is on no camera", w.id, n.key, n.room);
        }
    }
}

/// Mixed is read only here (its cameras are not part of the F6 fixes); this test names every mixed spot a camera misses.
#[test]
fn mixed_cameras_show_their_rooms() {
    use super::mixed::world::{camera_transform, map, nav, stand_position, vent_spot_pos};
    let nav = nav();
    let mut views: Vec<(View, Extra)> = map()
        .areas
        .iter()
        .map(|a| {
            let (pos, target) = camera_transform(&a.id);
            (View::new(&format!("mixed {}", a.id), pos, target, FEED_FOV_DEG, &[a.id.as_str()]), slots(|s| stand_position(&a.id, s)))
        })
        .collect();
    let (pos, target) = camera_transform("vent");
    views.push((View::new("mixed vent", pos, target, FEED_FOV_DEG, &["vent"]), (0..5).map(|i| (format!("vent spot {i}"), vent_spot_pos(i))).collect()));
    assert_covered("mixed", &views, &nav);
}
