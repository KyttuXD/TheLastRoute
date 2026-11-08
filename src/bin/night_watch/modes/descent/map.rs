//! The Descent's map data: the view numbers of `descent_world.gd` (sill, camera, lamps), the map file
//! (`data/descent/map.json`: the car, the stairwell landings, the pipe gallery, the fuse closet, the signal hall, the
//! flights between them and the props on the sill) and the navigation graph (`data/nav/descent.json`).
//! Metres, building space (x right, z back from the car), levels 3 m apart in y: `descent_map.gd`.
//! Pure: no drawing.
#![allow(dead_code)]

use super::sim::{doc, Side};
use crate::hunters::nav::NavMap;
use macroquad::prelude::{vec3, Vec3};
use serde_json::Value;
use std::sync::OnceLock;

pub const MAP_JSON: &str = include_str!("../../data/descent/map.json");
pub const NAV_JSON: &str = include_str!("../../data/nav/descent.json");

fn num(v: &Value, key: &str, d: f64) -> f32 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(d) as f32
}

fn arr3(v: &Value, key: &str, d: [f32; 3]) -> [f32; 3] {
    match v.get(key).and_then(|a| a.as_array()) {
        Some(a) if a.len() >= 3 => [
            a[0].as_f64().unwrap_or(d[0] as f64) as f32,
            a[1].as_f64().unwrap_or(d[1] as f64) as f32,
            a[2].as_f64().unwrap_or(d[2] as f64) as f32,
        ],
        _ => d,
    }
}

fn v3(v: &Value, key: &str, d: [f32; 3]) -> Vec3 {
    let a = arr3(v, key, d);
    vec3(a[0], a[1], a[2])
}

// ------------------------------------------------------------------------------------------------ the view

/// The numbers of `descent_world.gd` that are not geometry: where Elise walks, the camera, the lamps.
#[derive(Clone, Debug)]
pub struct View {
    pub elise_speed: f32,
    pub sill_far: f32,
    pub sill_near: f32,
    pub window_x: f32,
    pub ride_drop: f32,
    pub beam_gap: f32,
    pub cam_pos: Vec3,
    pub cam_pitch_deg: f32,
    pub cam_fov_deg: f32,
    pub lamp_pos: Vec3,
    pub lamp_calm: [f32; 3],
    pub lamp_alarm: [f32; 3],
    pub lamp_energy_calm: f32,
    pub lamp_energy_alarm: f32,
    pub lamp_range: f32,
    pub sill_light_y: f32,
    pub sill_light_z: f32,
    pub sill_light_colour: [f32; 3],
    pub sill_light_energy: f32,
    pub sill_light_range: f32,
    pub ambient: [f32; 3],
    pub background: [f32; 3],
}

impl View {
    pub fn parse(doc: &Value) -> View {
        let v = doc.get("view").cloned().unwrap_or(Value::Null);
        let cam = v.get("camera").cloned().unwrap_or(Value::Null);
        let lamp = v.get("lamp").cloned().unwrap_or(Value::Null);
        let sl = v.get("sill_light").cloned().unwrap_or(Value::Null);
        View {
            elise_speed: num(&v, "elise_speed", 1.4),
            sill_far: num(&v, "sill_far", -5.6),
            sill_near: num(&v, "sill_near", -1.95),
            window_x: num(&v, "window_x", 1.1),
            ride_drop: num(&v, "ride_drop", 3.0),
            beam_gap: num(&v, "beam_gap", 3.0),
            cam_pos: v3(&cam, "pos", [0.0, 1.5, 1.1]),
            cam_pitch_deg: num(&cam, "pitch_deg", -3.0),
            cam_fov_deg: num(&cam, "fov_deg", 62.0),
            lamp_pos: v3(&lamp, "pos", [0.0, 3.35, 0.1]),
            lamp_calm: arr3(&lamp, "calm", [1.0, 0.86, 0.62]),
            lamp_alarm: arr3(&lamp, "alarm", [1.0, 0.35, 0.28]),
            lamp_energy_calm: num(&lamp, "energy_calm", 1.5),
            lamp_energy_alarm: num(&lamp, "energy_alarm", 1.0),
            lamp_range: num(&lamp, "range", 5.0),
            sill_light_y: num(&sl, "y", 2.3),
            sill_light_z: num(&sl, "z", -2.9),
            sill_light_colour: arr3(&sl, "colour", [0.45, 0.55, 0.85]),
            sill_light_energy: num(&sl, "energy", 0.8),
            sill_light_range: num(&sl, "range", 5.0),
            ambient: arr3(&v, "ambient", [0.34, 0.36, 0.5]),
            background: arr3(&v, "background", [0.02, 0.02, 0.03]),
        }
    }

    pub fn get() -> &'static View {
        static VIEW: OnceLock<View> = OnceLock::new();
        VIEW.get_or_init(|| View::parse(doc()))
    }
}

/// X of a side's window and sill.
pub fn sill_x(side: Side) -> f32 {
    side.sign() * View::get().window_x
}

/// Where Elise stands on `side` when she is `closeness` (0..100) of the way in: far down the sill, then at the glass
/// (`elise_position` of the Godot world).
pub fn sill_pos(side: Side, closeness: f64) -> Vec3 {
    let v = View::get();
    let t = (closeness / 100.0).clamp(0.0, 1.0) as f32;
    vec3(sill_x(side), 0.0, v.sill_far + (v.sill_near - v.sill_far) * t)
}

/// The nav room of a side's sill.
pub fn sill_room(side: Side) -> &'static str {
    match side {
        Side::L => "sill_l",
        Side::R => "sill_r",
    }
}

/// The room she comes from when a ride starts: the stairwell on the left, the pipe gallery on the right.
pub fn start_room(side: Side) -> &'static str {
    match side {
        Side::L => "stair_1",
        Side::R => "pipe_gallery",
    }
}

/// What she looks at from the sill: back at the car, at the camera.
pub fn look_target(_side: Side) -> Vec3 {
    View::get().cam_pos
}

/// The navigation graph of the shaft.
pub fn nav() -> NavMap {
    NavMap::from_json(NAV_JSON).unwrap_or_else(|_| NavMap::new("descent"))
}

/// The nav graph as an error when it does not parse or validate (start refusal and `--check`).
pub fn load_error() -> Option<String> {
    match NavMap::from_json(NAV_JSON) {
        Err(e) => Some(format!("descent nav graph: {e}")),
        Ok(n) => {
            let p = n.validate();
            if p.is_empty() { None } else { Some(format!("descent nav graph: {}", p.join("; "))) }
        }
    }
}

// ------------------------------------------------------------------------------------------------ the map file

#[derive(Clone, Debug)]
pub struct Car {
    pub half_w: f32,
    pub half_d: f32,
    pub height: f32,
    pub window_x: f32,
    pub window_w: f32,
    pub window_bottom: f32,
    pub window_top: f32,
}

impl Default for Car {
    fn default() -> Car {
        Car { half_w: 1.7, half_d: 1.4, height: 3.6, window_x: 1.1, window_w: 0.9, window_bottom: 0.85, window_top: 3.2 }
    }
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub kind: String,
    /// (x, z) from the room centre (the sill props use the mirrored x of the side and the absolute z).
    pub at: (f32, f32),
    pub tint: [f32; 3],
    pub size: f32,
}

/// A bbmodel prop of `last_route/assets/props` standing in a room: `at` is (x, z) from the room centre, `yaw_deg` turns it
/// about the vertical axis (front toward +z at 0), `y` lifts it (on a table), `scale` multiplies the model's own scale.
#[derive(Clone, Debug)]
pub struct ModelDef {
    pub model: String,
    pub at: (f32, f32),
    pub y: f32,
    pub yaw_deg: f32,
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct RoomDef {
    pub id: String,
    pub centre: Vec3,
    /// Width (x) and depth (z) of the platform.
    pub size: (f32, f32),
    pub tone: [f32; 3],
    /// Which sides get a wall: letters from "nsew".
    pub walls: String,
    pub lamp: bool,
    pub props: Vec<PropDef>,
    /// The bbmodel dressing of the room (data-driven, `models` in the map file).
    pub models: Vec<ModelDef>,
}

#[derive(Clone, Debug)]
pub struct FlightDef {
    pub id: String,
    pub a: Vec3,
    pub b: Vec3,
    pub width: f32,
}

#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub theme: String,
    pub car: Car,
    pub rooms: Vec<RoomDef>,
    pub flights: Vec<FlightDef>,
    pub sill_props: Vec<PropDef>,
}

fn models_of(v: &Value) -> Vec<ModelDef> {
    let mut out = Vec::new();
    for p in v.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let (x, z) = match p.get("at").and_then(|a| a.as_array()) {
            Some(a) if a.len() >= 2 => (a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32),
            _ => (0.0, 0.0),
        };
        out.push(ModelDef {
            model: p.get("model").and_then(|k| k.as_str()).unwrap_or("").to_string(),
            at: (x, z),
            y: num(p, "y", 0.0),
            yaw_deg: num(p, "yaw", 0.0),
            scale: num(p, "scale", 1.0),
        });
    }
    out
}

fn props_of(v: &Value) -> Vec<PropDef> {
    let mut out = Vec::new();
    if let Some(list) = v.as_array() {
        for p in list {
            let at = p.get("at").and_then(|a| a.as_array());
            let (x, z) = match at {
                Some(a) if a.len() >= 2 => (a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32),
                _ => (0.0, 0.0),
            };
            out.push(PropDef {
                kind: p.get("kind").and_then(|k| k.as_str()).unwrap_or("crate").to_string(),
                at: (x, z),
                tint: arr3(p, "tint", [1.0, 1.0, 1.0]),
                size: num(p, "size", 0.0),
            });
        }
    }
    out
}

impl MapData {
    pub fn parse(src: &str) -> Result<MapData, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| format!("descent map json: {e}"))?;
        let c = root.get("car").cloned().unwrap_or(Value::Null);
        let d = Car::default();
        let car = Car {
            half_w: num(&c, "half_w", d.half_w as f64),
            half_d: num(&c, "half_d", d.half_d as f64),
            height: num(&c, "height", d.height as f64),
            window_x: num(&c, "window_x", d.window_x as f64),
            window_w: num(&c, "window_w", d.window_w as f64),
            window_bottom: num(&c, "window_bottom", d.window_bottom as f64),
            window_top: num(&c, "window_top", d.window_top as f64),
        };
        let mut rooms = Vec::new();
        for r in root.get("rooms").and_then(|r| r.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let size = match r.get("size").and_then(|s| s.as_array()) {
                Some(s) if s.len() >= 2 => (s[0].as_f64().unwrap_or(3.0) as f32, s[1].as_f64().unwrap_or(2.4) as f32),
                _ => (3.0, 2.4),
            };
            rooms.push(RoomDef {
                id: r.get("id").and_then(|s| s.as_str()).ok_or_else(|| "descent map json: a room has no id".to_string())?.to_string(),
                centre: v3(r, "centre", [0.0, 0.0, 0.0]),
                size,
                tone: arr3(r, "tone", [0.5, 0.5, 0.55]),
                walls: r.get("walls").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                lamp: r.get("lamp").and_then(|b| b.as_bool()).unwrap_or(false),
                props: props_of(r.get("props").unwrap_or(&Value::Null)),
                models: models_of(r.get("models").unwrap_or(&Value::Null)),
            });
        }
        let mut flights = Vec::new();
        for f in root.get("flights").and_then(|r| r.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            flights.push(FlightDef {
                id: f.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                a: v3(f, "a", [0.0, 0.0, 0.0]),
                b: v3(f, "b", [0.0, 0.0, 0.0]),
                width: num(f, "width", 0.9),
            });
        }
        Ok(MapData {
            theme: root.get("theme").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            car,
            rooms,
            flights,
            sill_props: props_of(root.get("sill_props").unwrap_or(&Value::Null)),
        })
    }

    pub fn get() -> &'static MapData {
        static MAP: OnceLock<MapData> = OnceLock::new();
        MAP.get_or_init(|| MapData::parse(MAP_JSON).unwrap_or_default())
    }

    pub fn room(&self, id: &str) -> Option<&RoomDef> {
        self.rooms.iter().find(|r| r.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_file_has_the_seven_rooms_of_the_godot_map() {
        let m = MapData::parse(MAP_JSON).expect("map json");
        let ids: Vec<&str> = m.rooms.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["stair_1", "stair_2", "stair_3", "stair_4", "pipe_gallery", "fuse_closet", "signal_hall"]);
        // descent_map.gd: rooms at these centres, levels 3 m apart
        let c = |id: &str| m.room(id).map(|r| r.centre).unwrap_or(Vec3::ZERO);
        assert_eq!(c("stair_1"), vec3(-6.0, 0.0, -6.0));
        assert_eq!(c("stair_2"), vec3(-6.0, -3.0, -10.0));
        assert_eq!(c("stair_3"), vec3(-6.0, -6.0, -14.0));
        assert_eq!(c("stair_4"), vec3(-6.0, -9.0, -18.0));
        assert_eq!(c("pipe_gallery"), vec3(6.0, -3.0, -7.0));
        assert_eq!(c("fuse_closet"), vec3(7.0, -6.0, -12.0));
        assert_eq!(c("signal_hall"), vec3(0.0, -9.0, -17.0));
    }

    #[test]
    fn every_room_is_furnished() {
        let m = MapData::parse(MAP_JSON).expect("map json");
        for r in &m.rooms {
            assert!(r.props.len() >= 3, "{} has props", r.id);
            assert!(r.lamp, "{} has a lamp", r.id);
        }
        assert!(m.sill_props.len() >= 4);
        assert!(m.flights.len() >= 7);
    }

    #[test]
    fn the_car_is_the_godot_car() {
        let m = MapData::parse(MAP_JSON).expect("map json");
        assert_eq!((m.car.half_w, m.car.half_d, m.car.height), (1.7, 1.4, 3.6));
        assert_eq!((m.car.window_x, m.car.window_w, m.car.window_bottom, m.car.window_top), (1.1, 0.9, 0.85, 3.2));
    }

    #[test]
    fn the_view_numbers_are_the_godot_numbers() {
        let v = View::parse(doc());
        assert_eq!((v.sill_far, v.sill_near, v.window_x), (-5.6, -1.95, 1.1));
        assert_eq!(v.cam_pos, vec3(0.0, 1.5, 1.1));
        assert_eq!((v.cam_pitch_deg, v.cam_fov_deg), (-3.0, 62.0));
        assert_eq!((v.ride_drop, v.beam_gap, v.elise_speed), (3.0, 3.0, 1.4));
    }

    #[test]
    fn elise_stands_far_down_the_sill_then_at_the_glass() {
        let far = sill_pos(Side::L, 0.0);
        let near = sill_pos(Side::R, 100.0);
        assert_eq!(far, vec3(-1.1, 0.0, -5.6));
        assert_eq!(near, vec3(1.1, 0.0, -1.95));
        let mid = sill_pos(Side::L, 50.0);
        assert!((mid.z - (-3.775)).abs() < 1e-5);
        assert_eq!(sill_pos(Side::L, 250.0).z, -1.95, "closeness clamps");
        assert_eq!(sill_pos(Side::R, -4.0).z, -5.6);
    }

    #[test]
    fn the_nav_graph_is_sound() {
        let n = nav();
        assert!(n.len() > 40);
        let problems = n.validate();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn every_map_room_has_a_hub_and_a_stand_in_the_nav_graph_at_its_centre() {
        let n = nav();
        let m = MapData::get();
        for r in &m.rooms {
            let hub = n.pos(&format!("C:{}", r.id)).unwrap_or_else(|| panic!("hub of {}", r.id));
            assert_eq!(hub, r.centre, "{} hub sits at the map centre", r.id);
            assert!(n.stand_node(&r.id, 0).is_some(), "{} has a stand spot", r.id);
        }
    }

    #[test]
    fn every_flight_ends_on_nav_nodes_at_its_ports() {
        let n = nav();
        let m = MapData::get();
        let mut checked = 0;
        for f in &m.flights {
            let (ka, kb) = (format!("F:{}:a", f.id), format!("F:{}:b", f.id));
            if let (Some(pa), Some(pb)) = (n.pos(&ka), n.pos(&kb)) {
                assert_eq!(pa, f.a, "{ka}");
                assert_eq!(pb, f.b, "{kb}");
                checked += 1;
            }
        }
        assert!(checked >= 7, "{checked} flights have matching nav ports");
    }

    #[test]
    fn both_sills_reach_the_stairwell_and_the_gallery_on_the_graph() {
        let n = nav();
        for (a, b) in [("C:stair_1", "C:sill_l"), ("C:pipe_gallery", "C:sill_r"), ("C:signal_hall", "C:sill_l"), ("C:signal_hall", "C:sill_r"), ("C:stair_4", "C:fuse_closet")] {
            let d = n.distance(a, b);
            assert!(d.is_some(), "{a} to {b} is walkable");
        }
        assert!(n.distance("C:stair_1", "C:sill_l").unwrap_or(99.0) < 6.0);
    }

    #[test]
    fn room_task_spots_stand_beside_a_model_and_are_reachable() {
        let n = nav();
        let m = MapData::get();
        for room in ["stair_3", "fuse_closet", "signal_hall"] {
            let spots = n.task_spots(room, None);
            assert!(!spots.is_empty(), "{room} has a task spot");
            let r = m.rooms.iter().find(|r| r.id == room).expect("room");
            for t in spots {
                let look = t.look.expect("look");
                let near = r.models.iter().any(|md| {
                    let (x, z) = (r.centre.x + md.at.0, r.centre.z + md.at.1);
                    ((look.x - x).powi(2) + (look.z - z).powi(2)).sqrt() < 0.8
                });
                assert!(near, "{} looks at a model", t.key);
                assert!(n.distance(&format!("C:{room}"), "C:sill_l").is_some(), "{room} is walkable from the sill");
            }
        }
    }

    #[test]
    fn every_sill_task_spot_has_a_prop_beside_it() {
        let n = nav();
        let m = MapData::get();
        for side in [Side::L, Side::R] {
            let room = sill_room(side);
            for t in n.task_spots(room, None) {
                assert!(t.look.is_some(), "{}", t.key);
            }
            assert!(!n.task_spots(room, None).is_empty());
            assert!(m.sill_props.iter().any(|p| p.kind == "terminal"));
        }
    }
}
