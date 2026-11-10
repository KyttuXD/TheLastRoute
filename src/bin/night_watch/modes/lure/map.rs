//! Dead Air's floor plan (port of lure_map.gd): a radio station, not Classic's grid of cells. A wide reception sits on top of a
//! plus-shaped floor: a tall studio A and studio B either side of the big live room, the transmitter room below it with the tape
//! library beside it, and two crawlspaces that run out of the transmitter room toward the broadcast booth. The numbers live in
//! `data/lure/map.json`. Coordinates are metres on the XZ plane, relative to ORIGIN. Pure logic: no GL.
#![allow(dead_code)]

use super::data::{tuning, ROOMS};
use crate::core::{CamLayout, jf64, jstr};
use macroquad::prelude::{vec2, vec3, Vec3};
use serde_json::Value;
use std::sync::OnceLock;

pub const MAP_JSON: &str = include_str!("../../data/lure/map.json");

#[derive(Clone, Debug)]
pub struct MapRoom {
    /// The station room id ("reception").
    pub id: String,
    /// The rules' room id ("lobby").
    pub sim: String,
    pub x0: f32,
    pub z0: f32,
    pub x1: f32,
    pub z1: f32,
    pub theme: String,
}

impl MapRoom {
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn d(&self) -> f32 {
        self.z1 - self.z0
    }
}

/// A doorway between two rooms that share a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Door {
    /// The doorway's centre in map coordinates.
    pub point: (f32, f32),
    /// True when the wall runs along z (a west / east wall).
    pub along_z: bool,
}

/// Door centres along each wall of a room, in the room's local coordinates (x for N / S walls, z for W / E walls).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doors {
    pub n: Vec<f32>,
    pub s: Vec<f32>,
    pub w: Vec<f32>,
    pub e: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub model: String,
    /// A sim room id, or "booth".
    pub room: String,
    /// Local to the room's centre (the booth: to the booth's origin).
    pub x: f32,
    pub z: f32,
    pub y: f32,
    /// Which way the front looks: N (-z), S (+z), E (+x), W (-x).
    pub face: char,
    /// The built-box group this model replaces.
    pub replaces: Option<String>,
}

impl PropDef {
    /// Yaw (radians) that turns a front pointing to -z toward `face`.
    pub fn yaw(&self) -> f32 {
        match self.face {
            'E' => -std::f32::consts::FRAC_PI_2,
            'W' => std::f32::consts::FRAC_PI_2,
            'S' => std::f32::consts::PI,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TaskSpot {
    pub room: String,
    pub task: String,
    pub p: (f32, f32),
    pub pace_to: Option<(f32, f32)>,
    pub look: Vec3,
}

#[derive(Clone, Debug)]
pub struct LureMap {
    pub origin: Vec3,
    pub height: f32,
    pub crawl_height: f32,
    pub door_gap: f32,
    pub door_top: f32,
    pub cam_height: f32,
    pub cam_vent_height: f32,
    pub cam_inset: f32,
    pub cam_target: Vec3,
    pub slot0: (f32, f32),
    pub slot1: (f32, f32),
    pub vent0: (f32, f32),
    pub vent1: (f32, f32),
    pub rooms: Vec<MapRoom>,
    pub edges: Vec<(String, String)>,
    pub props: Vec<PropDef>,
    pub tasks: Vec<TaskSpot>,
}

fn arr(v: &Value, key: &str) -> Vec<f32> {
    v.get(key).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).map(|f| f as f32).collect()).unwrap_or_default()
}

fn pair(v: &Value, key: &str) -> (f32, f32) {
    let a = arr(v, key);
    (a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0))
}

fn vec_of(v: &Value, key: &str) -> Vec3 {
    let a = arr(v, key);
    vec3(a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0), a.get(2).copied().unwrap_or(0.0))
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

impl LureMap {
    pub fn parse(text: &str) -> Result<LureMap, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("lure map: {e}"))?;
        let cam = v.get("camera").cloned().unwrap_or(Value::Null);
        let stand = v.get("stand").cloned().unwrap_or(Value::Null);
        let mut m = LureMap {
            origin: vec_of(&v, "origin"),
            height: jf64(&v, "height", 3.6) as f32,
            crawl_height: jf64(&v, "crawl_height", 2.6) as f32,
            door_gap: jf64(&v, "door_gap", 1.6) as f32,
            door_top: jf64(&v, "door_top", 3.2) as f32,
            cam_height: jf64(&cam, "height", 2.8) as f32,
            cam_vent_height: jf64(&cam, "vent_height", 2.2) as f32,
            cam_inset: jf64(&cam, "inset", 0.5) as f32,
            cam_target: vec_of(&cam, "target"),
            slot0: pair(&stand, "slot0"),
            slot1: pair(&stand, "slot1"),
            vent0: pair(&stand, "vent0"),
            vent1: pair(&stand, "vent1"),
            rooms: Vec::new(),
            edges: Vec::new(),
            props: Vec::new(),
            tasks: Vec::new(),
        };
        for r in v.get("rooms").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            let rect = arr(&r, "rect");
            if rect.len() != 4 {
                return Err(format!("lure map: room {} needs a rect of four numbers", jstr(&r, "id")));
            }
            m.rooms.push(MapRoom { id: jstr(&r, "id"), sim: jstr(&r, "sim"), x0: rect[0], z0: rect[1], x1: rect[2], z1: rect[3], theme: jstr(&r, "theme") });
        }
        for e in v.get("edges").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            let a = e.as_array().cloned().unwrap_or_default();
            if a.len() >= 2 {
                m.edges.push((a[0].as_str().unwrap_or("").to_string(), a[1].as_str().unwrap_or("").to_string()));
            }
        }
        for p in v.get("props").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            let face = jstr(&p, "face").chars().next().unwrap_or('N');
            let rep = jstr(&p, "replaces");
            m.props.push(PropDef {
                model: jstr(&p, "model"),
                room: jstr(&p, "room"),
                x: jf64(&p, "x", 0.0) as f32,
                z: jf64(&p, "z", 0.0) as f32,
                y: jf64(&p, "y", 0.0) as f32,
                face,
                replaces: if rep.is_empty() { None } else { Some(rep) },
            });
        }
        for t in v.get("tasks").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            let pace = if t.get("pace_to").is_some() { Some(pair(&t, "pace_to")) } else { None };
            m.tasks.push(TaskSpot { room: jstr(&t, "room"), task: jstr(&t, "task"), p: pair(&t, "p"), pace_to: pace, look: vec_of(&t, "look") });
        }
        m.validate().map(|_| m)
    }

    pub fn validate(&self) -> Result<(), String> {
        for sim in ROOMS {
            if self.by_sim(sim).is_none() {
                return Err(format!("lure map: no room for {sim}"));
            }
        }
        for (a, b) in &self.edges {
            if self.room(a).is_none() || self.room(b).is_none() {
                return Err(format!("lure map: edge {a} - {b} names an unknown room"));
            }
            if self.door_between(a, b).is_none() {
                return Err(format!("lure map: {a} and {b} share no wall"));
            }
        }
        Ok(())
    }

    pub fn room(&self, map_id: &str) -> Option<&MapRoom> {
        self.rooms.iter().find(|r| r.id == map_id)
    }

    pub fn by_sim(&self, sim: &str) -> Option<&MapRoom> {
        self.rooms.iter().find(|r| r.sim == sim)
    }

    pub fn sim_to_map(&self, sim: &str) -> Option<&str> {
        self.by_sim(sim).map(|r| r.id.as_str())
    }

    pub fn map_to_sim(&self, map_id: &str) -> Option<&str> {
        self.room(map_id).map(|r| r.sim.as_str())
    }

    /// The room centre in the world (ORIGIN included).
    pub fn centre(&self, map_id: &str) -> Vec3 {
        match self.room(map_id) {
            Some(r) => self.origin + vec3((r.x0 + r.x1) * 0.5, 0.0, (r.z0 + r.z1) * 0.5),
            None => Vec3::ZERO,
        }
    }

    pub fn centre_of_sim(&self, sim: &str) -> Vec3 {
        match self.sim_to_map(sim) {
            Some(id) => self.centre(id),
            None => Vec3::ZERO,
        }
    }

    /// The doorway between two rooms that share a wall (map coordinates).
    pub fn door_between(&self, a: &str, b: &str) -> Option<Door> {
        let (ra, rb) = (self.room(a)?, self.room(b)?);
        for (p, q) in [(ra.x1, rb.x0), (rb.x1, ra.x0)] {
            if approx(p, q) {
                let lo = ra.z0.max(rb.z0);
                let hi = ra.z1.min(rb.z1);
                if hi - lo > 1.5 {
                    return Some(Door { point: (p, (lo + hi) * 0.5), along_z: true });
                }
            }
        }
        for (p, q) in [(ra.z1, rb.z0), (rb.z1, ra.z0)] {
            if approx(p, q) {
                let lo = ra.x0.max(rb.x0);
                let hi = ra.x1.min(rb.x1);
                if hi - lo > 1.5 {
                    return Some(Door { point: ((lo + hi) * 0.5, p), along_z: false });
                }
            }
        }
        None
    }

    /// The doorways of a room along each wall, in the room's local coordinates.
    pub fn doors_of(&self, map_id: &str) -> Doors {
        let mut out = Doors::default();
        let Some(r) = self.room(map_id) else { return out };
        let (cx, cz) = ((r.x0 + r.x1) * 0.5, (r.z0 + r.z1) * 0.5);
        for (a, b) in &self.edges {
            if a != map_id && b != map_id {
                continue;
            }
            let other = if a == map_id { b } else { a };
            let Some(d) = self.door_between(map_id, other) else { continue };
            let (px, pz) = d.point;
            if d.along_z {
                if approx(px, r.x0) {
                    out.w.push(pz - cz);
                } else {
                    out.e.push(pz - cz);
                }
            } else if approx(pz, r.z0) {
                out.n.push(px - cx);
            } else {
                out.s.push(px - cx);
            }
        }
        out
    }

    /// The world positions of every doorway of a room, in edge order.
    pub fn door_points(&self, map_id: &str) -> Vec<Vec3> {
        let mut out = Vec::new();
        for (a, b) in &self.edges {
            if a != map_id && b != map_id {
                continue;
            }
            let other = if a == map_id { b } else { a };
            if let Some(d) = self.door_between(map_id, other) {
                out.push(self.origin + vec3(d.point.0, 0.0, d.point.1));
            }
        }
        out
    }

    pub fn is_vent(&self, sim: &str) -> bool {
        sim.starts_with("vent")
    }

    /// World position of hunter slot 0 or 1 in a room (keyed by the rules' room id).
    pub fn stand_position(&self, sim: &str, slot: usize) -> Vec3 {
        let c = self.centre_of_sim(sim);
        let o = if self.is_vent(sim) {
            if slot == 0 {
                self.vent0
            } else {
                self.vent1
            }
        } else if slot == 0 {
            self.slot0
        } else {
            self.slot1
        };
        c + vec3(o.0, 0.0, o.1)
    }

    /// The feed camera's mount: high in the room's north-west corner, looking at the middle of the floor.
    pub fn camera_transform(&self, sim: &str) -> (Vec3, Vec3) {
        let Some(r) = self.by_sim(sim) else { return (Vec3::ZERO, vec3(0.0, 0.0, -1.0)) };
        let c = self.centre(&r.id);
        let h = if self.is_vent(sim) { self.cam_vent_height } else { self.cam_height };
        let from = c + vec3(-r.w() * 0.5 + self.cam_inset, h, -r.d() * 0.5 + self.cam_inset);
        let to = c + vec3(r.w() * self.cam_target.x, self.cam_target.y, r.d() * self.cam_target.z);
        (from, to)
    }

    /// The camera screen's minimap: camera n is the n-th room of `ROOMS`.
    pub fn layout(&self) -> CamLayout {
        let t = tuning();
        let mut l = CamLayout::default();
        for r in &self.rooms {
            let c = self.centre(&r.id);
            l.rooms.push((r.id.clone(), vec2(c.x, c.z)));
        }
        l.edges = self.edges.clone();
        l.entrances = vec!["crawl_l".to_string(), "crawl_r".to_string()];
        l.vents = l.entrances.clone();
        for sim in ROOMS {
            if let Some(id) = self.sim_to_map(sim) {
                l.cams.push(id.to_string());
                l.labels.push((id.to_string(), t.label(sim)));
            }
        }
        l
    }

    pub fn tasks_in(&self, sim: &str) -> Vec<&TaskSpot> {
        self.tasks.iter().filter(|t| t.room == sim).collect()
    }

    pub fn props_in(&self, room: &str) -> Vec<&PropDef> {
        self.props.iter().filter(|p| p.room == room).collect()
    }

    /// True when a built-box group of that name is replaced by a bbmodel prop in that room.
    pub fn replaced(&self, room: &str, group: &str) -> bool {
        self.props.iter().any(|p| p.room == room && p.replaces.as_deref() == Some(group))
    }
}

// --- the booth (lure_office.gd): the part the nav graph and the hunters need to know ---------------------------------------------

pub const HALF_W: f32 = 3.0;
pub const HALF_D: f32 = 2.5;
pub const HEIGHT: f32 = 3.6;
pub const VENT_W: f32 = 1.5;
pub const VENT_H: f32 = 2.2;
/// Where the player starts (feet).
pub const SPAWN: Vec3 = Vec3::new(0.0, 0.0, 0.9);
/// How far down the duct a hunter at the hatch stands when she has just arrived, and when she has waited the whole time.
pub const DOOR_FAR: f32 = HALF_W + 1.9;
pub const DOOR_NEAR: f32 = HALF_W + 0.55;

/// Where a hunter at a vent opening stands, from how far into her wait she is (0..1): well down the duct, then up to the
/// hatch. `door` is "officeL" / "doorL" (left) or the right ones; `slot` 1 stands a step further back.
pub fn door_stand(door: &str, slot: usize, progress: f32) -> Vec3 {
    let side = if door == "doorL" || door == "officeL" { -1.0 } else { 1.0 };
    let near = DOOR_FAR + (DOOR_NEAR - DOOR_FAR) * progress.clamp(0.0, 1.0);
    vec3(side * (near + slot as f32 * 1.3), 0.0, -0.4 * slot as f32)
}

/// The parsed map (embedded). A broken file gives an empty map, which the tests and `check_data` report.
pub fn map() -> &'static LureMap {
    static M: OnceLock<LureMap> = OnceLock::new();
    M.get_or_init(|| {
        LureMap::parse(MAP_JSON).unwrap_or_else(|_| LureMap {
            origin: Vec3::ZERO,
            height: 3.6,
            crawl_height: 2.6,
            door_gap: 1.6,
            door_top: 3.2,
            cam_height: 2.8,
            cam_vent_height: 2.2,
            cam_inset: 0.5,
            cam_target: Vec3::ZERO,
            slot0: (0.8, 0.6),
            slot1: (-1.0, -0.6),
            vent0: (0.0, 0.9),
            vent1: (-1.2, 0.9),
            rooms: Vec::new(),
            edges: Vec::new(),
            props: Vec::new(),
            tasks: Vec::new(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn the_map_parses_with_eight_rooms_and_nine_doorways() {
        let m = LureMap::parse(MAP_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.rooms.len(), 8);
        assert_eq!(m.edges.len(), 9);
        assert_eq!(m.origin, vec3(-8.0, 0.0, -40.0));
        assert!(m.props.len() >= 30);
        for sim in ROOMS {
            assert!(m.by_sim(sim).is_some(), "{sim}");
        }
    }

    #[test]
    fn centres_and_doors_follow_the_godot_plan() {
        let m = map();
        assert!(close(m.centre("reception"), vec3(-8.0, 0.0, -37.5)));
        assert!(close(m.centre("studio_a"), vec3(-15.0, 0.0, -34.5)));
        assert_eq!(m.door_between("reception", "studio_a"), Some(Door { point: (-4.0, 2.5), along_z: true }));
        assert_eq!(m.door_between("studio_b", "tape_library"), Some(Door { point: (7.0, 11.0), along_z: false }));
        assert_eq!(m.door_between("transmitter", "crawl_l"), Some(Door { point: (-4.0, 13.5), along_z: true }));
        assert_eq!(m.door_between("transmitter", "crawl_r"), Some(Door { point: (0.0, 17.0), along_z: false }));
        assert_eq!(m.door_between("reception", "tape_library"), None);
        assert_eq!(m.door_between("studio_a", "studio_b"), None);
    }

    #[test]
    fn a_rooms_doors_sit_on_the_right_walls() {
        let m = map();
        let r = m.doors_of("reception");
        assert_eq!(r.w, vec![0.0]);
        assert_eq!(r.e, vec![0.0]);
        assert!(r.n.is_empty() && r.s.is_empty());
        let t = m.doors_of("transmitter");
        assert_eq!(t.n, vec![0.0]); // to the live room
        assert_eq!(t.e, vec![0.0]); // to the tape library
        assert_eq!(t.w, vec![-0.5]); // to the left crawlspace (z 13.5 against the centre 14)
        assert_eq!(t.s, vec![0.0]); // to the right crawlspace
    }

    #[test]
    fn stand_spots_and_cameras() {
        let m = map();
        let lobby = m.centre_of_sim("lobby");
        assert!(close(m.stand_position("lobby", 0), lobby + vec3(0.8, 0.0, 0.6)));
        assert!(close(m.stand_position("lobby", 1), lobby + vec3(-1.0, 0.0, -0.6)));
        let v = m.centre_of_sim("ventl");
        assert!(close(m.stand_position("ventl", 0), v + vec3(0.0, 0.0, 0.9)));
        assert!(close(m.stand_position("ventr", 1), m.centre_of_sim("ventr") + vec3(-1.2, 0.0, 0.9)));
        let (from, to) = m.camera_transform("lobby");
        assert!(close(from, vec3(-11.5, 2.8, -39.5)));
        assert!(close(to, vec3(-8.0 + 8.0 * 0.12, 0.7, -37.5 + 5.0 * 0.12)));
        let (vf, _) = m.camera_transform("ventr");
        assert!((vf.y - 2.2).abs() < 1e-5, "the crawlspace cameras hang lower");
    }

    #[test]
    fn the_cam_layout_has_eight_cameras_in_rules_order() {
        let l = map().layout();
        assert_eq!(l.cams.len(), 8);
        assert_eq!(l.cams[0], "reception");
        assert_eq!(l.cams[3], "live_room");
        assert_eq!(l.cams[7], "crawl_r");
        assert_eq!(l.entrances, vec!["crawl_l".to_string(), "crawl_r".to_string()]);
        assert_eq!(l.rooms.len(), 8);
        assert_eq!(l.edges.len(), 9);
        let (_, name) = &l.labels[0];
        assert_eq!(name, "RECEPTION");
    }

    #[test]
    fn props_face_with_the_yaw_convention() {
        let mut p = map().props[0].clone();
        p.face = 'N';
        assert_eq!(p.yaw(), 0.0);
        p.face = 'S';
        assert!((p.yaw() - std::f32::consts::PI).abs() < 1e-6);
        p.face = 'E';
        assert!(p.yaw() < 0.0);
    }

    #[test]
    fn the_door_stand_creeps_up_the_duct() {
        assert_eq!(door_stand("officeL", 0, 0.0), vec3(-4.9, 0.0, 0.0));
        let near = door_stand("officeR", 0, 1.0);
        assert!((near.x - 3.55).abs() < 1e-5 && near.z == 0.0);
        let slot1 = door_stand("doorR", 1, 0.0);
        assert!((slot1.x - 6.2).abs() < 1e-5 && (slot1.z + 0.4).abs() < 1e-6);
        assert!(door_stand("officeL", 0, 5.0).x > -3.55 - 1e-5, "progress is clamped");
    }

    #[test]
    fn every_replaced_group_belongs_to_a_room_that_has_it() {
        let m = map();
        assert!(m.replaced("ballpit", "DrumKit"));
        assert!(m.replaced("lobby", "Sofa"));
        assert!(!m.replaced("lobby", "DrumKit"));
    }
}
