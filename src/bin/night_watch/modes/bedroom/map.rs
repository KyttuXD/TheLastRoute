//! Bedtime's floor plan: the house wing around the child's bedroom (bedroom_map.gd) and the numbers of the bedroom shell,
//! lights and prop placements, read from `data/bedroom/map.json`. Pure: no drawing, no GL.
//!
//! Coordinates are metres in the bedroom's space: the bedroom's centre is the origin, x runs east along the hall, z south.
#![allow(dead_code)]

use macroquad::prelude::{vec3, Vec3};
use serde_json::Value;
use std::sync::OnceLock;

pub const MAP_JSON: &str = include_str!("../../data/bedroom/map.json");

/// One room of the wing.
#[derive(Clone, Debug)]
pub struct RoomDef {
    pub id: String,
    pub min: (f32, f32),
    pub max: (f32, f32),
    pub theme: String,
}

impl RoomDef {
    pub fn center(&self) -> Vec3 {
        vec3((self.min.0 + self.max.0) * 0.5, 0.0, (self.min.1 + self.max.1) * 0.5)
    }
    pub fn width(&self) -> f32 {
        self.max.0 - self.min.0
    }
    pub fn depth(&self) -> f32 {
        self.max.1 - self.min.1
    }
    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.min.0 && x <= self.max.0 && z >= self.min.1 && z <= self.max.1
    }
}

/// The doorway between two rooms that share a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Door {
    /// (x, z) of the doorway's middle.
    pub point: (f32, f32),
    /// True for a wall running north-south (the doorway is crossed along x), false for one running east-west.
    pub along_z: bool,
}

/// Doorway offsets (from the room's centre) on each side of a room.
#[derive(Clone, Debug, Default)]
pub struct Sides {
    pub north: Vec<f32>,
    pub south: Vec<f32>,
    pub west: Vec<f32>,
    pub east: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub id: String,
    pub model: String,
    pub room: String,
    pub pos: Vec3,
    pub yaw_deg: f32,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct OmniDef {
    pub pos: Vec3,
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpotDef {
    /// From the pillow (the eye).
    pub offset: Vec3,
    pub target: Vec3,
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
    pub half_angle_deg: f32,
}

#[derive(Clone, Debug)]
pub struct LightDefs {
    pub ambient: [f32; 3],
    pub ambient_energy: f32,
    pub reveal_energy: f32,
    pub background: [f32; 3],
    pub moon: OmniDef,
    pub hall_glow: OmniDef,
    pub living_glow: OmniDef,
    pub hall_spot: SpotDef,
    pub bed_spot: SpotDef,
}

/// Where a hunter stands in her death scene, and which light finds her.
#[derive(Clone, Debug)]
pub struct ScareDef {
    pub id: String,
    pub pos: Vec3,
    pub light: String,
}

#[derive(Clone, Debug)]
pub struct BedroomMap {
    pub rooms: Vec<RoomDef>,
    pub edges: Vec<(String, String)>,
    pub entrances: Vec<String>,
    /// (name, x) of the hall's three waypoints.
    pub hall_nodes: Vec<(String, f32)>,
    pub half: f32,
    pub height: f32,
    pub eye: Vec3,
    pub doorway: Vec3,
    pub creep_from: Vec3,
    pub creep_to: Vec3,
    pub closet_z: f32,
    pub closet_ajar: f32,
    pub closet_wide: f32,
    pub hall_gap: f32,
    pub closet_gap: f32,
    pub door_gap: f32,
    pub door_top: f32,
    pub lights: LightDefs,
    pub scares: Vec<ScareDef>,
    pub props: Vec<PropDef>,
}

fn f(v: &Value, default: f32) -> f32 {
    v.as_f64().map(|x| x as f32).unwrap_or(default)
}

fn v3(v: &Value) -> Vec3 {
    let a = v.as_array();
    let g = |i: usize| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    vec3(g(0), g(1), g(2))
}

fn c3(v: &Value) -> [f32; 3] {
    let p = v3(v);
    [p.x, p.y, p.z]
}

fn pair(v: &Value) -> (f32, f32) {
    let a = v.as_array();
    let g = |i: usize| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    (g(0), g(1))
}

fn omni(v: &Value) -> OmniDef {
    OmniDef { pos: v3(&v["pos"]), color: c3(&v["color"]), energy: f(&v["energy"], 1.0), range: f(&v["range"], 5.0) }
}

fn spot(v: &Value) -> SpotDef {
    SpotDef {
        offset: v3(&v["offset"]),
        target: v3(&v["target"]),
        color: c3(&v["color"]),
        energy: f(&v["energy"], 1.0),
        range: f(&v["range"], 8.0),
        half_angle_deg: f(&v["half_angle_deg"], 30.0),
    }
}

impl BedroomMap {
    /// Parses a map document. The embedded one is checked by the tests and `--check`.
    pub fn parse(src: &str) -> Result<BedroomMap, String> {
        let j: Value = serde_json::from_str(src).map_err(|e| format!("bedroom map json: {e}"))?;
        let rooms_obj = j["rooms"].as_object().ok_or("bedroom map json: no rooms")?;
        let mut rooms = Vec::new();
        for (id, r) in rooms_obj {
            rooms.push(RoomDef {
                id: id.clone(),
                min: pair(&r["min"]),
                max: pair(&r["max"]),
                theme: r["theme"].as_str().unwrap_or("office").to_string(),
            });
        }
        let edges = j["edges"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| Some((e.get(0)?.as_str()?.to_string(), e.get(1)?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let entrances = j["entrances"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let hall_nodes = j["hall_nodes"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), f(v, 0.0))).collect())
            .unwrap_or_default();
        let b = &j["bedroom"];
        let l = &j["lights"];
        let w = &j["wing"];
        let scares = j["scare"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| ScareDef {
                        id: k.clone(),
                        pos: v3(&v["pos"]),
                        light: v["light"].as_str().unwrap_or("bed").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let props = j["props"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| PropDef {
                        id: p["id"].as_str().unwrap_or("").to_string(),
                        model: p["model"].as_str().unwrap_or("").to_string(),
                        room: p["room"].as_str().unwrap_or("").to_string(),
                        pos: v3(&p["pos"]),
                        yaw_deg: f(&p["yaw"], 0.0),
                        scale: f(&p["scale"], 1.0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let map = BedroomMap {
            rooms,
            edges,
            entrances,
            hall_nodes,
            half: f(&b["half"], 2.5),
            height: f(&b["height"], 3.6),
            eye: v3(&b["eye"]),
            doorway: v3(&b["doorway"]),
            creep_from: v3(&b["creep_from"]),
            creep_to: v3(&b["creep_to"]),
            closet_z: f(&b["closet_z"], -0.9),
            closet_ajar: f(&b["closet_ajar"], -0.22),
            closet_wide: f(&b["closet_wide"], -1.3),
            hall_gap: f(&b["hall_gap"], 1.1),
            closet_gap: f(&b["closet_gap"], 1.2),
            door_gap: f(&w["door_gap"], 1.6),
            door_top: f(&w["door_top"], 3.2),
            lights: LightDefs {
                ambient: c3(&l["ambient"]),
                ambient_energy: f(&l["ambient_energy"], 0.55),
                reveal_energy: f(&l["reveal_energy"], 2.4),
                background: c3(&l["background"]),
                moon: omni(&l["moon"]),
                hall_glow: omni(&l["hall_glow"]),
                living_glow: omni(&l["living_glow"]),
                hall_spot: spot(&l["hall_spot"]),
                bed_spot: spot(&l["bed_spot"]),
            },
            scares,
            props,
        };
        Ok(map)
    }

    pub fn room(&self, id: &str) -> Option<&RoomDef> {
        self.rooms.iter().find(|r| r.id == id)
    }

    /// The rectangle of a room. Unknown ids give the bedroom's, so callers need no unwrap.
    pub fn rect(&self, id: &str) -> &RoomDef {
        self.room(id).unwrap_or(&self.rooms[0])
    }

    pub fn center(&self, id: &str) -> Vec3 {
        self.rect(id).center()
    }

    /// The room whose rectangle holds the point.
    pub fn room_at(&self, x: f32, z: f32) -> Option<&str> {
        self.rooms.iter().find(|r| r.contains(x, z)).map(|r| r.id.as_str())
    }

    /// The doorway between two rooms that share a wall (a gap of more than 1.5 m), as `door_between` of the Godot map.
    pub fn door_between(&self, a: &str, b: &str) -> Option<Door> {
        let (ra, rb) = (self.room(a)?, self.room(b)?);
        let eq = |x: f32, y: f32| (x - y).abs() < 1e-4;
        for (p, q) in [(ra.max.0, rb.min.0), (rb.max.0, ra.min.0)] {
            if eq(p, q) {
                let lo = ra.min.1.max(rb.min.1);
                let hi = ra.max.1.min(rb.max.1);
                if hi - lo > 1.5 {
                    return Some(Door { point: (p, (lo + hi) * 0.5), along_z: true });
                }
            }
        }
        for (p, q) in [(ra.max.1, rb.min.1), (rb.max.1, ra.min.1)] {
            if eq(p, q) {
                let lo = ra.min.0.max(rb.min.0);
                let hi = ra.max.0.min(rb.max.0);
                if hi - lo > 1.5 {
                    return Some(Door { point: ((lo + hi) * 0.5, p), along_z: false });
                }
            }
        }
        None
    }

    /// The doorways of a room by side, as offsets from its centre (`_doors_of` of the Godot wing).
    pub fn sides_of(&self, id: &str) -> Sides {
        let mut out = Sides::default();
        let Some(r) = self.room(id) else { return out };
        let c = r.center();
        for (a, b) in &self.edges {
            if a != id && b != id {
                continue;
            }
            let other = if a == id { b } else { a };
            let Some(d) = self.door_between(id, other) else { continue };
            if d.along_z {
                if (d.point.0 - r.min.0).abs() < 1e-4 {
                    out.west.push(d.point.1 - c.z);
                } else {
                    out.east.push(d.point.1 - c.z);
                }
            } else if (d.point.1 - r.min.1).abs() < 1e-4 {
                out.north.push(d.point.0 - c.x);
            } else {
                out.south.push(d.point.0 - c.x);
            }
        }
        out
    }

    /// The hall waypoint nearest an x (the key suffix: "near", "mid", "far").
    pub fn hall_node_for(&self, x: f32) -> &str {
        let mut best = "near";
        let mut best_d = f32::MAX;
        for (k, hx) in &self.hall_nodes {
            let d = (hx - x).abs();
            if d < best_d {
                best_d = d;
                best = k;
            }
        }
        best
    }

    /// Where Haelith stands when she is `p` percent of the way to the bed.
    pub fn creep_position(&self, p: f32) -> Vec3 {
        self.creep_from.lerp(self.creep_to, (p / 100.0).clamp(0.0, 1.0))
    }

    pub fn scare(&self, id: &str) -> Option<&ScareDef> {
        self.scares.iter().find(|s| s.id == id)
    }

    /// The x of the hall's east end.
    pub fn hall_end_x(&self) -> f32 {
        self.rect("hall").max.0
    }
}

/// The embedded map. A broken file is a bug the tests and `--check` report; the fallback is an empty single-room map.
pub fn map() -> &'static BedroomMap {
    static MAP: OnceLock<BedroomMap> = OnceLock::new();
    MAP.get_or_init(|| {
        BedroomMap::parse(MAP_JSON).unwrap_or_else(|_| BedroomMap {
            rooms: vec![RoomDef { id: "bedroom".into(), min: (-2.7, -2.6), max: (2.7, 2.6), theme: "office".into() }],
            edges: Vec::new(),
            entrances: Vec::new(),
            hall_nodes: Vec::new(),
            half: 2.5,
            height: 3.6,
            eye: vec3(0.0, 0.95, 1.9),
            doorway: vec3(2.6, 0.0, -0.9),
            creep_from: vec3(-1.7, 0.0, -2.0),
            creep_to: vec3(0.15, 0.0, 0.75),
            closet_z: -0.9,
            closet_ajar: -0.22,
            closet_wide: -1.3,
            hall_gap: 1.1,
            closet_gap: 1.2,
            door_gap: 1.6,
            door_top: 3.2,
            lights: LightDefs {
                ambient: [0.3, 0.3, 0.5],
                ambient_energy: 0.55,
                reveal_energy: 2.4,
                background: [0.015, 0.015, 0.03],
                moon: OmniDef { pos: Vec3::ZERO, color: [1.0; 3], energy: 0.0, range: 1.0 },
                hall_glow: OmniDef { pos: Vec3::ZERO, color: [1.0; 3], energy: 0.0, range: 1.0 },
                living_glow: OmniDef { pos: Vec3::ZERO, color: [1.0; 3], energy: 0.0, range: 1.0 },
                hall_spot: SpotDef { offset: Vec3::ZERO, target: Vec3::Z, color: [1.0; 3], energy: 0.0, range: 1.0, half_angle_deg: 30.0 },
                bed_spot: SpotDef { offset: Vec3::ZERO, target: Vec3::Z, color: [1.0; 3], energy: 0.0, range: 1.0, half_angle_deg: 30.0 },
            },
            scares: Vec::new(),
            props: Vec::new(),
        })
    })
}

/// Why this mode cannot start, if so: Bedroom cannot start: the map, the rules or the nav file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    BedroomMap::parse(MAP_JSON).err().or_else(super::sim::load_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(BedroomMap::parse("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    #[test]
    fn the_embedded_map_parses_with_nine_rooms_and_eight_doorways() {
        let m = BedroomMap::parse(MAP_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.rooms.len(), 9);
        assert_eq!(m.edges.len(), 8);
        for (a, b) in &m.edges {
            assert!(m.door_between(a, b).is_some(), "{a} - {b} share no wall");
        }
        assert_eq!(m.eye, vec3(0.0, 0.95, 1.9));
        assert!(m.props.len() > 30);
    }

    #[test]
    fn doorways_land_where_the_godot_map_puts_them() {
        let m = map();
        let d = m.door_between("bedroom", "hall").expect("bedroom-hall");
        assert_eq!(d, Door { point: (2.7, -0.9), along_z: true });
        let d = m.door_between("hall", "living_room").expect("hall-living");
        assert!((d.point.0 - 10.7).abs() < 1e-4 && (d.point.1 + 0.9).abs() < 1e-4);
        let d = m.door_between("hall", "bathroom").expect("hall-bath");
        assert_eq!((d.point, d.along_z), ((5.5, -2.0), false));
        let d = m.door_between("hall", "landing").expect("hall-landing");
        assert!((d.point.0 - 9.3).abs() < 1e-4 && !d.along_z);
        let d = m.door_between("hall", "kids_room").expect("hall-kids");
        assert_eq!((d.point, d.along_z), ((6.0, 0.2), false));
        // rooms that share no wall have no doorway
        assert!(m.door_between("bedroom", "galley").is_none());
        assert!(m.door_between("wardrobe", "hall").is_none());
    }

    #[test]
    fn the_hall_walls_have_the_gaps_the_bedroom_leaves_open() {
        let m = map();
        let h = m.sides_of("hall");
        // the hall's north side opens to the bathroom and the landing, its south side to the kids' room
        let c = m.center("hall");
        let mut north: Vec<f32> = h.north.iter().map(|o| o + c.x).collect();
        north.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        assert_eq!(north.len(), 2);
        assert!((north[0] - 5.5).abs() < 1e-4 && (north[1] - 9.3).abs() < 1e-4);
        assert_eq!(h.south.len(), 1);
        assert!((h.south[0] + c.x - 6.0).abs() < 1e-4);
        assert_eq!(h.east.len(), 1, "the living room");
        assert_eq!(h.west.len(), 1, "the bedroom");
    }

    #[test]
    fn hall_nodes_pick_the_nearest_waypoint() {
        let m = map();
        assert_eq!(m.hall_node_for(2.7), "near");
        assert_eq!(m.hall_node_for(5.5), "mid");
        assert_eq!(m.hall_node_for(9.3), "far");
        assert_eq!(m.hall_node_for(10.7), "far");
    }

    #[test]
    fn haelith_creeps_in_a_straight_line() {
        let m = map();
        assert_eq!(m.creep_position(0.0), vec3(-1.7, 0.0, -2.0));
        assert!((m.creep_position(100.0) - vec3(0.15, 0.0, 0.75)).length() < 1e-4);
        assert_eq!(m.creep_position(500.0), m.creep_position(100.0));
        let mid = m.creep_position(50.0);
        assert!((mid.x - (-0.775)).abs() < 1e-4 && (mid.z - (-0.625)).abs() < 1e-4);
    }

    #[test]
    fn every_prop_is_inside_its_room_and_has_a_model() {
        let m = map();
        for p in &m.props {
            let r = m.room(&p.room).unwrap_or_else(|| panic!("{} is in unknown room {}", p.id, p.room));
            assert!(r.contains(p.pos.x, p.pos.z), "{} at ({}, {}) is outside {}", p.id, p.pos.x, p.pos.z, p.room);
            assert!(!p.model.is_empty(), "{}", p.id);
        }
    }

    #[test]
    fn rooms_at_points() {
        let m = map();
        assert_eq!(m.room_at(0.0, 0.0), Some("bedroom"));
        assert_eq!(m.room_at(6.0, -0.9), Some("hall"));
        assert_eq!(m.room_at(14.2, -1.0), Some("living_room"));
        assert_eq!(m.room_at(40.0, 40.0), None);
    }
}
