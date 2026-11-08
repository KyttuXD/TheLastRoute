//! The Facility's 3D map (`story_world.gd`, `story_office.gd`, `story_map.gd`): five rooms in a row (breaker room, gallery,
//! control, workshop, auditorium) joined by doorways in their shared walls, and the lift car north of Control. Rooms are at
//! the Godot coordinates (6 m apart, 5.6 x 5 m, 3.6 m high); every room has its station and its props. Godot has no security
//! cameras in the Facility, so one is mounted in each room from the classic building's mounting rule (high, in the near
//! corner, looking across the room); the mounts are in `data/story/map.json`.
//!
//! The map is built in two steps. `blueprint()` is pure (cubes, stations, lights, prop placements) so it is tested without a
//! window; `StoryWorld::new` bakes it into meshes and loads the `.bbmodel` props, which needs the GL context.
#![allow(dead_code)]

use super::sim::{rules, Puzzle, Who};
use crate::gfx::office::{collect_solids, mul3, put, StationDef, O};
use crate::gfx::{Aabb, Cube, Gfx, Group, LightSet, Omni, Spot, Textures, World, WorldBuilder, MAX_OMNI};
use crate::hunters::bbmodel::Model;
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const MAP_JSON: &str = include_str!("../../data/story/map.json");
pub const NAV_JSON: &str = include_str!("../../data/nav/story.json");
pub const BEHAVIOUR_JSON: &str = include_str!("../../data/story/behaviour.json");

/// The dial's step (six marks round the face).
pub const DIAL_STEP: f32 = std::f32::consts::TAU / 6.0;
/// The crank handle's angle per turn (rad).
pub const CRANK_STEP: f32 = 0.5;
/// Crank poses are prebuilt for 0..=CRANK_POSES-1 turns.
pub const CRANK_POSES: i32 = 17;

// ------------------------------------------------------------------------------------------------ layout data

fn fl(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn v3j(v: &Value) -> Vec3 {
    vec3(fl(&v[0]), fl(&v[1]), fl(&v[2]))
}

fn rgb(v: &Value) -> [f32; 3] {
    [fl(&v[0]), fl(&v[1]), fl(&v[2])]
}

/// A security camera: where it hangs and what it looks at.
#[derive(Clone, Debug)]
pub struct CameraDef {
    pub room: String,
    pub label: String,
    pub pos: Vec3,
    pub target: Vec3,
    pub fov: f32,
}

/// A `.bbmodel` prop placement (room-local position, facing, footprint in metres before rotation).
#[derive(Clone, Debug)]
pub struct PropDef {
    pub model: String,
    pub room: String,
    pub pos: Vec3,
    /// s = front toward +z, n = -z, e = +x, w = -x.
    pub face: char,
    pub solid: bool,
    pub min: Vec3,
    pub max: Vec3,
}

impl PropDef {
    pub fn yaw(&self) -> f32 {
        match self.face {
            'n' => std::f32::consts::PI,
            'e' => std::f32::consts::FRAC_PI_2,
            'w' => -std::f32::consts::FRAC_PI_2,
            _ => 0.0,
        }
    }

    /// The prop's box in the world, given its room's centre.
    pub fn aabb(&self, centre: Vec3) -> Aabb {
        let yaw = self.yaw();
        let (s, c) = (yaw.sin(), yaw.cos());
        let corners = [(self.min.x, self.min.z), (self.max.x, self.min.z), (self.min.x, self.max.z), (self.max.x, self.max.z)];
        let (mut x0, mut x1, mut z0, mut z1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for (x, z) in corners {
            let (rx, rz) = (x * c + z * s, -x * s + z * c);
            x0 = x0.min(rx);
            x1 = x1.max(rx);
            z0 = z0.min(rz);
            z1 = z1.max(rz);
        }
        let base = centre + self.pos;
        Aabb { min: vec3(base.x + x0, base.y + self.min.y, base.z + z0), max: vec3(base.x + x1, base.y + self.max.y, base.z + z1) }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub cell: f32,
    pub room_w: f32,
    pub room_d: f32,
    pub height: f32,
    pub door_gap: f32,
    pub door_top: f32,
    pub spawn_offset: f32,
    pub order: Vec<String>,
    pub lift: Vec3,
    pub tints: Vec<(String, [f32; 3])>,
    pub creeper_anchor: f32,
    pub creeper_base: f32,
    pub creeper_step: f32,
    pub creeper_z: Vec<(String, f32)>,
    pub creeper_side: Vec<(String, f32)>,
    pub in_car: Vec3,
    pub at_door: Vec3,
    pub q_trip_s: f32,
    pub player_start: Vec3,
    pub player_yaw: f32,
    pub player_pitch: f32,
    pub room_light: (Vec3, f32, f32, f32),
    pub lift_light: (Vec3, f32, f32, f32),
    pub light_y: (f32, f32),
    pub ambient: [f32; 3],
    pub ambient_scale: f32,
    pub beacon_color: [f32; 3],
    pub beacon_energy: f32,
    pub beacon_range: f32,
    pub beacon_half_deg: f32,
    pub beacon_offset: Vec3,
    pub cameras: Vec<CameraDef>,
    pub props: Vec<PropDef>,
    pub map_edges: Vec<(String, String)>,
    pub entrances: Vec<String>,
    pub map_pos: Vec<(String, Vec2)>,
}

impl Layout {
    pub fn parse(src: &str) -> Result<Layout, String> {
        let j: Value = serde_json::from_str(src).map_err(|e| format!("story map: {e}"))?;
        let strs = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default() };
        let mut l = Layout {
            cell: fl(&j["cell"]),
            room_w: fl(&j["roomW"]),
            room_d: fl(&j["roomD"]),
            height: fl(&j["height"]),
            door_gap: fl(&j["doorGap"]),
            door_top: fl(&j["doorTop"]),
            spawn_offset: fl(&j["spawnOffset"]),
            order: strs(&j["order"]),
            lift: v3j(&j["lift"]),
            creeper_anchor: fl(&j["creeper"]["anchor"]),
            creeper_base: fl(&j["creeper"]["base"]),
            creeper_step: fl(&j["creeper"]["step"]),
            in_car: v3j(&j["quenelle"]["inCar"]),
            at_door: v3j(&j["quenelle"]["atDoor"]),
            q_trip_s: fl(&j["quenelle"]["tripS"]),
            player_start: v3j(&j["player"]["start"]),
            player_yaw: fl(&j["player"]["yaw"]),
            player_pitch: fl(&j["player"]["pitch"]),
            ambient: rgb(&j["lights"]["ambient"]),
            ambient_scale: fl(&j["lights"]["ambientScale"]),
            beacon_color: rgb(&j["beacon"]["color"]),
            beacon_energy: fl(&j["beacon"]["energy"]),
            beacon_range: fl(&j["beacon"]["range"]),
            beacon_half_deg: fl(&j["beacon"]["halfAngleDeg"]),
            beacon_offset: v3j(&j["beacon"]["offset"]),
            ..Layout::default()
        };
        if l.order.is_empty() || l.room_w <= 0.0 || l.room_d <= 0.0 {
            return Err("story map: no rooms".to_string());
        }
        if let Some(o) = j["tints"].as_object() {
            for (k, v) in o {
                l.tints.push((k.clone(), rgb(v)));
            }
        }
        if let Some(o) = j["creeper"]["z"].as_object() {
            for (k, v) in o {
                l.creeper_z.push((k.clone(), fl(v)));
            }
        }
        if let Some(o) = j["creeper"]["side"].as_object() {
            for (k, v) in o {
                l.creeper_side.push((k.clone(), fl(v)));
            }
        }
        let room_l = &j["lights"]["room"];
        l.room_light = (rgb_vec(&room_l["color"]), fl(&room_l["energy"]), fl(&room_l["range"]), fl(&room_l["y"]));
        let lift_l = &j["lights"]["lift"];
        l.lift_light = (rgb_vec(&lift_l["color"]), fl(&lift_l["energy"]), fl(&lift_l["range"]), fl(&lift_l["y"]));
        l.light_y = (l.room_light.3, l.lift_light.3);
        for c in j["cameras"].as_array().cloned().unwrap_or_default() {
            l.cameras.push(CameraDef {
                room: c["room"].as_str().unwrap_or("").to_string(),
                label: c["label"].as_str().unwrap_or("").to_string(),
                pos: v3j(&c["pos"]),
                target: v3j(&c["target"]),
                fov: fl(&c["fov"]),
            });
        }
        for p in j["props"].as_array().cloned().unwrap_or_default() {
            l.props.push(PropDef {
                model: p["model"].as_str().unwrap_or("").to_string(),
                room: p["room"].as_str().unwrap_or("").to_string(),
                pos: v3j(&p["pos"]),
                face: p["face"].as_str().and_then(|s| s.chars().next()).unwrap_or('s'),
                solid: p["solid"].as_bool().unwrap_or(false),
                min: v3j(&p["min"]),
                max: v3j(&p["max"]),
            });
        }
        for e in j["map"]["edges"].as_array().cloned().unwrap_or_default() {
            if let (Some(a), Some(b)) = (e[0].as_str(), e[1].as_str()) {
                l.map_edges.push((a.to_string(), b.to_string()));
            }
        }
        l.entrances = strs(&j["map"]["entrances"]);
        if let Some(o) = j["map"]["pos"].as_object() {
            for (k, v) in o {
                l.map_pos.push((k.clone(), vec2(fl(&v[0]), fl(&v[1]))));
            }
        }
        Ok(l)
    }

    /// The floor centre of a room (the lift car's for "lift").
    pub fn centre(&self, room: &str) -> Vec3 {
        if room == "lift" {
            return self.lift;
        }
        match self.order.iter().position(|r| r == room) {
            Some(i) => vec3((i as f32 - 2.0) * self.cell, 0.0, 0.0),
            None => Vec3::ZERO,
        }
    }

    pub fn tint(&self, room: &str) -> [f32; 3] {
        self.tints.iter().find(|(k, _)| k == room).map(|(_, t)| *t).unwrap_or([1.0, 1.0, 1.0])
    }

    pub fn camera(&self, room: &str) -> Option<&CameraDef> {
        self.cameras.iter().find(|c| c.room == room)
    }

    /// The camera's number minus one (its position in `cameras`).
    pub fn camera_index(&self, room: &str) -> Option<usize> {
        self.cameras.iter().position(|c| c.room == room)
    }

    pub fn creeper_z(&self, who: Who) -> f32 {
        self.creeper_z.iter().find(|(k, _)| k == who.id()).map(|(_, v)| *v).unwrap_or(0.0)
    }

    /// Which way her far end lies from where you came in, before you have come in anywhere.
    pub fn default_side(&self, who: Who) -> f32 {
        self.creeper_side.iter().find(|(k, _)| k == who.id()).map(|(_, v)| *v).unwrap_or(1.0)
    }

    /// Which sides of a room have a doorway: L / R toward a neighbour, C (north) only in Control.
    pub fn has_door(&self, room: &str, dir: char) -> bool {
        rules().exit(room, dir).is_some()
    }
}

fn rgb_vec(v: &Value) -> Vec3 {
    vec3(fl(&v[0]), fl(&v[1]), fl(&v[2]))
}

/// The map, parsed once. A broken embedded file gives an empty layout (`--check` reports it).
pub fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| Layout::parse(MAP_JSON).unwrap_or_default())
}

// ------------------------------------------------------------------------------------------------ places

/// The doorway of `room` in direction `dir`, on the floor (the lift door is in Control's north wall).
pub fn door_point(room: &str, dir: char) -> Vec3 {
    let l = layout();
    if dir == 'C' {
        return vec3(0.0, 0.0, -l.room_d * 0.5 - 0.1);
    }
    l.centre(room) + vec3(if dir == 'L' { -1.0 } else { 1.0 } * (l.room_w * 0.5 + 0.1), 0.0, 0.0)
}

/// Where the player stands after walking through a door: `dir` is the door they took from the room before `to`.
pub fn arrival(to: &str, dir: char) -> Vec3 {
    let l = layout();
    let c = l.centre(to);
    if dir == 'C' {
        return c + if to == "lift" { vec3(0.0, 0.0, 1.0) } else { vec3(0.0, 0.0, -l.room_d * 0.5 + 1.4) };
    }
    c + vec3(if dir == 'L' { 1.0 } else { -1.0 } * (l.room_w * 0.5 - l.spawn_offset), 0.0, 0.0)
}

/// Where Opalie or Porcelle stands `dist` steps from the player. `side` is which way her far end lies (-1 or 1).
pub fn creeper_spot(who: Who, room: &str, dist: i32, side: f32) -> Vec3 {
    let l = layout();
    let c = l.centre(room);
    let anchor_x = -side * (l.room_w * 0.5 - l.creeper_anchor);
    c + vec3(anchor_x + side * (l.creeper_base + dist as f32 * l.creeper_step), 0.0, l.creeper_z(who))
}

/// The room whose box holds a point (for the minimap and the camera map), by the room rectangles of the layout.
pub fn room_at(p: Vec3) -> Option<String> {
    let l = layout();
    if p.z < -l.room_d * 0.5 - 0.15 && p.x.abs() < 1.7 {
        return Some("lift".to_string());
    }
    for r in &l.order {
        let c = l.centre(r);
        if (p.x - c.x).abs() <= l.cell * 0.5 && p.z.abs() <= l.room_d * 0.5 + 0.2 {
            return Some(r.clone());
        }
    }
    None
}

// ------------------------------------------------------------------------------------------------ the blueprint

/// One prop to place from a `.bbmodel`.
#[derive(Clone, Debug)]
pub struct PropPlace {
    pub model: String,
    pub pos: Vec3,
    pub yaw: f32,
}

/// Everything the map is made of, before it touches the GPU.
pub struct Blueprint {
    pub groups: Vec<Group>,
    /// Extra solid boxes that are not cubes (the control room's furniture bodies, the .bbmodel props).
    pub solids: Vec<Aabb>,
    /// Invisible boxes in the doorways: the player goes through a door with the door button, not on foot.
    pub blockers: Vec<Aabb>,
    pub stations: Vec<StationDef>,
    pub props: Vec<PropPlace>,
    pub omni: Vec<Omni>,
}

fn st(id: &str, prompt: &str, pos: Vec3, radius: f32) -> StationDef {
    StationDef { id: id.to_string(), prompt: prompt.to_string(), pos, focus: Vec3::ZERO, radius, disabled: false }
}

fn side_name(side: f32) -> char {
    if side < 0.0 {
        'L'
    } else {
        'R'
    }
}

/// A room's side wall: full, or with a doorway (two solid pieces, a lintel, frame posts, a hazard stripe).
fn wall_x(g: &mut Group, l: &Layout, door: bool, side: f32, tint: [f32; 3]) {
    let (w, d, h) = (l.room_w, l.room_d, l.height);
    let x = side * (w * 0.5 + 0.1);
    let dir = side_name(side);
    if !door {
        put(g, &format!("Wall{dir}Full"), [0.2, h, d], [x, h * 0.5, 0.0], O::new().t(tint).solid());
        return;
    }
    let seg = (d - l.door_gap) * 0.5;
    for sgn in [-1.0f32, 1.0] {
        put(g, &format!("Wall{dir}Piece"), [0.2, h, seg], [x, h * 0.5, sgn * (l.door_gap * 0.5 + seg * 0.5)], O::new().t(tint).solid());
    }
    put(g, &format!("Wall{dir}Lintel"), [0.2, h - l.door_top, l.door_gap], [x, l.door_top + (h - l.door_top) * 0.5, 0.0], O::new().t(mul3(tint, [0.9; 3])));
    for sgn in [-1.0f32, 1.0] {
        put(g, "FramePost", [0.3, l.door_top, 0.12], [x, l.door_top * 0.5, sgn * (l.door_gap * 0.5 + 0.06)], O::new().t([0.75, 0.7, 0.65]));
    }
    put(g, "HazardStripe", [0.12, 0.08, l.door_gap], [x - side * 0.1, l.door_top - 0.05, 0.0], O::new().k("label").t([0.85; 3]));
}

/// The shell of a room: floor, ceiling, four walls (doorways as the rules' doors say), trims, pipe, ceiling panel, drain, sign.
fn shell(l: &Layout, id: &str) -> Group {
    let c = l.centre(id);
    let tint = l.tint(id);
    let (w, d, h) = (l.room_w, l.room_d, l.height);
    let mut g = Group::new(&format!("{id}/Shell"), c);
    put(&mut g, "Floor", [w + 0.4, 0.2, d + 0.4], [0.0, -0.1, 0.0], O::new().t(mul3(tint, [0.95; 3])).solid());
    put(&mut g, "Ceiling", [w + 0.4, 0.2, d + 0.4], [0.0, h + 0.1, 0.0], O::new().t(mul3(tint, [0.6; 3])));
    if id == "control" {
        let seg = (w - l.door_gap) * 0.5;
        for sgn in [-1.0f32, 1.0] {
            put(&mut g, "WallNorthPiece", [seg, h, 0.2], [sgn * (l.door_gap * 0.5 + seg * 0.5), h * 0.5, -d * 0.5 - 0.1], O::new().t(tint).solid());
        }
        put(&mut g, "WallNorthLintel", [l.door_gap, h - l.door_top, 0.2], [0.0, l.door_top + (h - l.door_top) * 0.5, -d * 0.5 - 0.1], O::new().t(tint));
    } else {
        put(&mut g, "WallNorth", [w + 0.4, h, 0.2], [0.0, h * 0.5, -d * 0.5 - 0.1], O::new().t(tint).solid());
    }
    put(&mut g, "WallSouth", [w + 0.4, h, 0.2], [0.0, h * 0.5, d * 0.5 + 0.1], O::new().t(mul3(tint, [0.9, 0.9, 0.95])).solid());
    let side_tint = mul3(tint, [1.05; 3]);
    wall_x(&mut g, l, l.has_door(id, 'L'), -1.0, side_tint);
    wall_x(&mut g, l, l.has_door(id, 'R'), 1.0, side_tint);
    for z in [-d * 0.5 + 0.04, d * 0.5 - 0.04] {
        put(&mut g, "Baseboard", [w, 0.2, 0.08], [0.0, 0.1, z], O::new().t([0.7, 0.7, 0.75]));
        put(&mut g, "CrownTrim", [w, 0.1, 0.08], [0.0, h - 0.05, z], O::new().t([0.8, 0.8, 0.85]));
    }
    put(&mut g, "PipeRoom", [w - 0.6, 0.1, 0.1], [0.0, h - 0.2, d * 0.5 - 0.2], O::new().k("metal"));
    put(&mut g, "CeilingPanel", [1.2, 0.05, 1.2], [0.0, h - 0.04, 0.0], O::new());
    put(&mut g, "FloorDrain", [0.3, 0.02, 0.3], [2.0, 0.01, 1.8], O::new().k("metal").t([0.6, 0.6, 0.65]));
    put(&mut g, "SignPlate", [1.4, 0.3, 0.05], [0.0, 2.4, d * 0.5 - 0.1], O::new().k("plastic"));
    put(&mut g, "SignText", [1.2, 0.18, 0.03], [0.0, 2.4, d * 0.5 - 0.14], O::new().t(tint).emissive());
    if id == "control" {
        put(&mut g, "FloorStripe", [w, 0.02, 0.3], [0.0, 0.01, 0.8], O::new().k("label").t([0.8; 3]));
    }
    g
}

/// Crates, a locker and a pipe run so every room has something in it besides its station.
fn decor(l: &Layout, id: &str, idx: usize) -> Group {
    let c = l.centre(id);
    let (w, d, h) = (l.room_w, l.room_d, l.height);
    let mut g = Group::new(&format!("{id}/Decor"), c);
    put(&mut g, "CrateBox", [0.7, 0.7, 0.7], [-2.2, 0.35, 1.6], O::new().t([0.9 + 0.1 * idx as f32, 0.85, 0.8]).solid());
    put(&mut g, "CrateBand", [0.74, 0.06, 0.74], [-2.2, 0.5, 1.6], O::new().k("metal").t([0.8, 0.8, 0.85]));
    put(&mut g, "CrateBox", [0.4, 0.4, 0.4], [-1.6, 0.2, 1.9], O::new().t([0.8, 0.8, 0.9]).solid());
    let cab = if id != "gallery" && id != "breaker" { [2.3, 0.0, -1.9] } else { [2.3, 0.0, 1.4] };
    put(&mut g, "LockerBody", [0.6, 1.8, 0.7], [cab[0], 0.9, cab[2]], O::new().t([0.75, 0.85, 1.0]).solid());
    put(&mut g, "LockerDoor", [0.04, 1.6, 0.6], [cab[0] - 0.32, 0.9, cab[2]], O::new().k("metal").t([0.9, 0.9, 0.95]));
    put(&mut g, "LockerHandle", [0.05, 0.08, 0.14], [cab[0] - 0.35, 1.0, cab[2] + 0.2], O::new().k("plastic"));
    put(&mut g, "PipeRun", [w - 0.4, 0.14, 0.14], [0.0, h - 0.35, -d * 0.5 + 0.25], O::new().k("metal"));
    for x in [-2.0f32, 0.0, 2.0] {
        put(&mut g, "PipeClamp", [0.18, 0.2, 0.18], [x, h - 0.35, -d * 0.5 + 0.25], O::new().k("metal").t([0.8, 0.8, 0.85]));
    }
    g
}

/// The green door button on a side wall, and its station.
fn door_buttons(l: &Layout, id: &str, g: &mut Group, stations: &mut Vec<StationDef>) {
    let c = l.centre(id);
    for dir in ['L', 'R'] {
        let Some(to) = rules().exit(id, dir) else { continue };
        let sgn = if dir == 'L' { -1.0f32 } else { 1.0 };
        put(g, "DoorButton", [0.06, 0.3, 0.3], [sgn * (l.room_w * 0.5 - 0.1), 1.2, 1.15], O::new().k("green"));
        let prompt = format!("GO {} // {}", if dir == 'L' { "LEFT" } else { "RIGHT" }, rules().label(to));
        stations.push(st(&format!("door_{id}_{dir}"), &prompt, c + vec3(sgn * (l.room_w * 0.5 - 0.15), 1.2, 1.15), 0.3));
    }
}

/// The gallery: five lamps on the north wall, each with a dial (the face, six needle poses and six mark poses per dial).
fn gallery_extras(l: &Layout, out: &mut Vec<Group>, stations: &mut Vec<StationDef>) {
    let c = l.centre("gallery");
    let mut board = Group::new("gallery/Lamps", c);
    put(&mut board, "LampBoard", [4.8, 1.6, 0.1], [0.0, 1.4, -l.room_d * 0.5 + 0.2], O::new().t([0.7, 0.7, 0.8]));
    out.push(board);
    for i in 0..5 {
        let (rx, ry, rz) = (-2.0 + i as f32, 1.4, -l.room_d * 0.5 + 0.2 + 0.12);
        let mut base = Group::new(&format!("gallery/Dial{i}"), c);
        put(&mut base, "DialFace", [0.7, 0.7, 0.06], [rx, ry, rz], O::new().k("plastic").t([1.3, 1.3, 1.4]));
        put(&mut base, "DialHub", [0.14, 0.14, 0.1], [rx, ry, rz], O::new().k("metal"));
        out.push(base);
        for p in 0..6 {
            let a = p as f32 * DIAL_STEP;
            let (sx, cy) = (a.sin(), a.cos());
            let mut needle = Group::new(&format!("gallery/Needle{i}_{p}"), c);
            for r in [0.09f32, 0.19] {
                put(&mut needle, "DialNeedle", [0.07, 0.07, 0.06], [rx + r * sx, ry + r * cy, rz + 0.06], O::new().k("red"));
            }
            out.push(needle);
            let mut mark = Group::new(&format!("gallery/Mark{i}_{p}"), c);
            put(&mut mark, "DialMarkGlow", [0.1, 0.12, 0.05], [rx + 0.3 * sx, ry + 0.3 * cy, rz + 0.05], O::new().t([1.0, 0.8, 0.3]).emissive());
            out.push(mark);
        }
        stations.push(st(&format!("dial_{i}"), "LAMP DIAL // TURN ONE STEP", c + vec3(rx, ry, rz + 0.1), 0.35));
    }
}

/// The workshop: a bench with six parts on it, and a rack.
fn workshop_extras(l: &Layout, out: &mut Vec<Group>, stations: &mut Vec<StationDef>) {
    let c = l.centre("workshop");
    let bz = -l.room_d * 0.5 + 0.7;
    let mut bench = Group::new("workshop/Bench", c);
    put(&mut bench, "BenchBody", [4.6, 0.9, 0.9], [0.0, 0.45, bz], O::new().solid());
    put(&mut bench, "BenchTop", [4.7, 0.06, 1.0], [0.0, 0.93, bz], O::new().k("wood").t([1.2, 1.1, 1.0]));
    out.push(bench);
    for i in 0..6 {
        let x = -2.0 + i as f32 * 0.8;
        let mut part = Group::new(&format!("workshop/Part{i}"), c);
        let kind = ["metal", "red", "green"][i % 3];
        put(&mut part, "PartBlock", [0.3, 0.2, 0.3], [x, 0.96 + 0.1, bz], O::new().k(kind).t([1.1; 3]));
        put(&mut part, "PartKnob", [0.1, 0.12, 0.1], [x, 0.96 + 0.26, bz], O::new().k("plastic"));
        out.push(part);
        stations.push(st(&format!("part_{i}"), "TAKE THE PART", c + vec3(x, 0.96 + 0.15, bz), 0.3));
    }
    let mut racks = Group::new("workshop/Racks", c);
    put(&mut racks, "RackBody", [0.5, 2.0, 1.6], [2.4, 1.0, 1.4], O::new().k("metal").solid());
    for y in [0.5f32, 1.0, 1.5] {
        put(&mut racks, "RackShelf", [0.52, 0.05, 1.5], [2.4, y, 1.4], O::new().k("metal").t([1.1; 3]));
    }
    out.push(racks);
}

/// The auditorium: the stage with its curtains, the crank (a pose per turn) and two rows of seats.
fn auditorium_extras(l: &Layout, out: &mut Vec<Group>, stations: &mut Vec<StationDef>) {
    let c = l.centre("auditorium");
    let mut stage = Group::new("auditorium/Stage", c);
    put(&mut stage, "PlatformDeck", [4.2, 0.4, 1.8], [0.0, 0.2, -1.4], O::new().solid());
    put(&mut stage, "PlatformEdge", [4.3, 0.06, 1.9], [0.0, 0.42, -1.4], O::new().t([1.2; 3]));
    for x in [-2.4f32, 2.4] {
        put(&mut stage, "CurtainDrape", [0.5, 2.4, 0.14], [x, 1.4, -2.3], O::new().t([1.1, 0.35, 0.4]));
    }
    out.push(stage);
    let mut crank = Group::new("auditorium/Crank", c);
    put(&mut crank, "CrankBox", [0.5, 0.9, 0.5], [0.0, 0.45, 0.2], O::new().k("metal").t([0.9, 0.9, 0.95]).solid());
    out.push(crank);
    for k in 0..CRANK_POSES {
        let th = k as f32 * CRANK_STEP;
        let (s, co) = (th.sin(), th.cos());
        let mut handle = Group::new(&format!("auditorium/CrankPose{k}"), c);
        let pivot = vec3(0.0, 0.95, 0.5);
        handle.add(Cube::new("CrankArm", vec3(0.7, 0.08, 0.08), pivot + vec3(0.3 * co, 0.0, -0.3 * s)).kind("metal").rot_y(th));
        handle.add(Cube::new("CrankGrip", vec3(0.1, 0.25, 0.1), pivot + vec3(0.62 * co, 0.1, -0.62 * s)).kind("red"));
        out.push(handle);
    }
    stations.push(st("crank", "STAGE CRANK // TURN", c + vec3(0.0, 1.0, 0.5), 0.4));
    let mut seats = Group::new("auditorium/Seats", c);
    for row in 0..2 {
        for col in 0..4 {
            let (x, z) = (-1.8 + col as f32 * 1.2, 1.2 + row as f32 * 0.9);
            put(&mut seats, "ChairSeat", [0.5, 0.08, 0.5], [x, 0.45, z], O::new());
            put(&mut seats, "ChairBack", [0.5, 0.5, 0.08], [x, 0.75, z + 0.22], O::new());
        }
    }
    out.push(seats);
}

/// The breaker room: a board with six numbered switches (the plate, and the lever up or down).
fn breaker_extras(l: &Layout, out: &mut Vec<Group>, stations: &mut Vec<StationDef>) {
    let c = l.centre("breaker");
    let bz = -l.room_d * 0.5 + 0.2;
    let mut board = Group::new("breaker/Breakers", c);
    put(&mut board, "BreakerBoard", [4.6, 1.8, 0.12], [0.0, 1.4, bz], O::new().k("metal").t([0.8, 0.8, 0.85]));
    out.push(board);
    for i in 0..6 {
        let x = -1.9 + i as f32 * 0.76;
        let mut plate = Group::new(&format!("breaker/Switch{i}"), c);
        put(&mut plate, "SwitchPlate", [0.3, 0.7, 0.06], [x, 1.4, bz + 0.1], O::new().k("plastic").t([1.3, 1.3, 1.35]));
        out.push(plate);
        let mut up = Group::new(&format!("breaker/SwUp{i}"), c);
        put(&mut up, "SwitchLever", [0.08, 0.3, 0.08], [x, 1.4 + 0.15, bz + 0.15], O::new().k("red"));
        out.push(up);
        // PI * 0.8 of rotation about the lever's pivot: the lever lies down and to the left
        let (dx, dy) = (-(0.8 * std::f32::consts::PI).sin() * 0.15, (0.8 * std::f32::consts::PI).cos() * 0.15);
        let mut down = Group::new(&format!("breaker/SwDown{i}"), c);
        put(&mut down, "SwitchLever", [0.08, 0.3, 0.08], [x + dx, 1.4 + dy, bz + 0.15], O::new().k("red"));
        out.push(down);
        stations.push(st(&format!("switch_{i}"), "BREAKER // FLIP", c + vec3(x, 1.4, bz + 0.2), 0.3));
    }
}

/// Control (the Facility's office): the monitor banks either side of the lift door, the desk with its terminal and the note,
/// the chair you hide under, the task board, a vent, a cabinet, a cable run, the three door buttons.
fn control_extras(l: &Layout, out: &mut Vec<Group>, solids: &mut Vec<Aabb>, stations: &mut Vec<StationDef>) {
    let c = l.centre("control");
    let (w, d) = (l.room_w, l.room_d);
    for side in [-1.0f32, 1.0] {
        let name = if side < 0.0 { "L" } else { "R" };
        let mut bank = Group::new(&format!("control/MonitorBank{name}"), c + vec3(side * 1.85, 0.0, -d * 0.5 + 0.2));
        put(&mut bank, "BankFrame", [2.0, 1.5, 0.12], [0.0, 1.75, 0.0], O::new().k("metal").t([0.7, 0.72, 0.8]));
        put(&mut bank, "BankLedge", [2.0, 0.5, 0.3], [0.0, 0.75, 0.1], O::new().k("metal").t([0.8, 0.82, 0.9]));
        for row in 0..2 {
            for col in 0..2 {
                let lit = 0.5 + 0.25 * ((row + col) % 2) as f32;
                put(&mut bank, "WallScreen", [0.84, 0.6, 0.05], [-0.47 + col as f32 * 0.94, 1.45 + row as f32 * 0.7, 0.08], O::new().t([lit, 1.0, 0.8]).emissive());
            }
        }
        for i in 0..6 {
            let kind = if i % 3 == 0 { "red" } else { "plastic" };
            put(&mut bank, "BankSwitch", [0.08, 0.05, 0.08], [-0.8 + i as f32 * 0.32, 1.03, 0.2], O::new().k(kind));
        }
        out.push(bank);
        solids.push(Aabb::from_centre(c + vec3(side * 1.85, 0.45, -d * 0.5 + 0.3), vec3(2.0, 0.9, 0.35)));
    }
    stations.push(st("monitor", "MONITOR WALL // WHAT IS NEXT ON THE LIST", c + vec3(-1.85, 1.6, -d * 0.5 + 0.4), 0.6));

    let mut desk = Group::new("control/Desk", c + vec3(-1.2, 0.0, 1.9));
    put(&mut desk, "DeskTop", [2.0, 0.08, 0.9], [0.0, 0.76, 0.0], O::new());
    for x in [-0.9f32, 0.9] {
        put(&mut desk, "DeskLeg", [0.1, 0.76, 0.8], [x, 0.38, 0.0], O::new());
    }
    put(&mut desk, "TerminalScreen", [0.6, 0.45, 0.06], [0.0, 1.05, -0.1], O::new().emissive());
    put(&mut desk, "KeypadBase", [0.7, 0.04, 0.3], [0.0, 0.8, 0.2], O::new().k("plastic"));
    for i in 0..3 {
        for j in 0..4 {
            put(&mut desk, "KeypadKeys", [0.1, 0.03, 0.05], [-0.2 + i as f32 * 0.2, 0.83, 0.1 + j as f32 * 0.06], O::new().k("metal").t([1.1, 1.1, 1.15]));
        }
    }
    out.push(desk);
    solids.push(Aabb::from_centre(c + vec3(-1.2, 0.45, 1.9), vec3(2.0, 0.9, 0.9)));
    let mut note = Group::new("control/Note", c + vec3(-0.2, 0.8, 1.9));
    put(&mut note, "NotePaper", [0.42, 0.012, 0.3], [0.0, 0.01, 0.0], O::new().k("label").t([1.2, 1.2, 1.1]).ao(1.0));
    out.push(note);
    stations.push(st("terminal", "TERMINAL // KEY IN THE CODE ON THE NOTE", c + vec3(-1.2, 1.0, 1.7), 0.5));

    let mut chair = Group::new("control/DeskChair", c + vec3(1.4, 0.0, 1.6));
    put(&mut chair, "ChairSeat", [0.5, 0.08, 0.5], [0.0, 0.45, 0.0], O::new());
    put(&mut chair, "ChairBack", [0.5, 0.5, 0.08], [0.0, 0.75, 0.22], O::new());
    put(&mut chair, "ChairColumn", [0.08, 0.45, 0.08], [0.0, 0.22, 0.0], O::new().k("metal"));
    out.push(chair);
    stations.push(st("hide_spot", "HIDE UNDER THE DESK // HOLD H", c + vec3(1.4, 0.6, 1.8), 0.5));

    let mut board = Group::new("control/TaskBoard", c + vec3(2.0, 1.7, d * 0.5 - 0.1));
    put(&mut board, "BoardFrame", [1.4, 1.0, 0.06], [0.0, 0.0, 0.0], O::new().k("wood"));
    put(&mut board, "BoardPaper", [1.28, 0.88, 0.03], [0.0, 0.0, -0.04], O::new().k("label").t([1.2, 1.2, 1.1]));
    for i in 0..5 {
        put(&mut board, "BoardLabel", [0.9, 0.05, 0.02], [-0.1, 0.32 - i as f32 * 0.16, -0.06], O::new().t([0.7; 3]));
    }
    out.push(board);
    let mut vent = Group::new("control/VentGrille", c + vec3(-2.2, 0.5, -d * 0.5 + 0.1));
    put(&mut vent, "VentFrame", [0.8, 0.5, 0.06], [0.0, 0.0, 0.0], O::new().k("metal"));
    for i in 0..4 {
        put(&mut vent, "VentLouver", [0.7, 0.04, 0.05], [0.0, -0.15 + i as f32 * 0.1, 0.04], O::new().k("metal").t([0.7, 0.7, 0.75]));
    }
    out.push(vent);
    let mut cabinet = Group::new("control/Cabinet", c + vec3(2.3, 0.0, -0.2));
    put(&mut cabinet, "LockerBody", [0.5, 1.5, 0.7], [0.0, 0.75, 0.0], O::new().t([0.75, 0.85, 1.0]));
    for i in 0..3 {
        put(&mut cabinet, "LockerDoor", [0.04, 0.4, 0.6], [-0.27, 0.3 + i as f32 * 0.45, 0.0], O::new().k("metal").t([0.9, 0.9, 0.95]));
    }
    out.push(cabinet);
    solids.push(Aabb::from_centre(c + vec3(2.3, 0.75, -0.2), vec3(0.5, 1.5, 0.7)));
    let mut cables = Group::new("control/Cables", c + vec3(0.0, 0.03, -1.9));
    for i in 0..3 {
        put(&mut cables, "CableRun", [0.05, 0.04, 1.4], [-0.3 + i as f32 * 0.3, 0.0, 0.0], O::new().k("plastic").t([0.6 + 0.2 * i as f32, 0.6, 0.8]));
    }
    out.push(cables);

    // the door buttons: left and right through the walls, and the lift door in the north wall
    let mut plates = Group::new("control/DoorPlates", c);
    door_buttons(l, "control", &mut plates, stations);
    put(&mut plates, "DoorButton", [0.3, 0.3, 0.06], [1.5, 1.2, -d * 0.5 + 0.1], O::new().k("green"));
    stations.push(st("door_control_C", "LIFT DOOR // GO IN", c + vec3(1.5, 1.2, -d * 0.5 + 0.1), 0.3));
    out.push(plates);
    let _ = w;
}

/// The lift: the car north of Control (floor, ceiling, three walls, rails, the floor number), its call button, the door button,
/// and the slab that rises in Control's north wall.
fn lift_groups(l: &Layout, out: &mut Vec<Group>, stations: &mut Vec<StationDef>) {
    let c = l.lift;
    let h = l.height;
    let mut car = Group::new("lift/Car", c);
    put(&mut car, "Floor", [3.2, 0.2, 3.2], [0.0, -0.1, 0.0], O::new().k("metal").t([0.9, 0.9, 0.95]).solid());
    put(&mut car, "Ceiling", [3.2, 0.2, 3.2], [0.0, h + 0.1, 0.0], O::new().t([0.6, 0.6, 0.65]));
    put(&mut car, "WallWest", [0.2, h, 3.2], [-1.6, h * 0.5, 0.0], O::new().k("metal").t([0.9, 0.95, 1.0]).solid());
    put(&mut car, "WallEast", [0.2, h, 3.2], [1.6, h * 0.5, 0.0], O::new().k("metal").t([0.9, 0.95, 1.0]).solid());
    put(&mut car, "WallNorth", [3.2, h, 0.2], [0.0, h * 0.5, -1.7], O::new().k("metal").t([0.85, 0.9, 1.0]).solid());
    for x in [-1.45f32, 1.45] {
        put(&mut car, "LiftRail", [0.1, 2.0, 0.06], [x, 1.2, -1.6], O::new().k("metal").t([1.2, 1.2, 1.25]));
    }
    put(&mut car, "SignFloorNumber", [1.0, 0.25, 0.04], [0.0, 2.5, -1.58], O::new().t([0.5, 1.0, 0.6]).emissive());
    put(&mut car, "PanelHousing", [0.1, 0.5, 0.36], [1.48, 1.3, 0.2], O::new());
    put(&mut car, "CallButton", [0.06, 0.22, 0.22], [1.4, 1.3, 0.2], O::new().k("red"));
    put(&mut car, "DoorButton", [0.06, 0.3, 0.3], [-1.4, 1.2, 1.4], O::new().k("green"));
    out.push(car);
    stations.push(st("lift_button", "CALL BUTTON // HOLD E", c + vec3(1.4, 1.3, 0.2), 0.35));
    stations.push(st("door_lift_C", "LIFT DOOR // GO OUT TO CONTROL", c + vec3(-1.4, 1.2, 1.4), 0.3));
    let mut slab = Group::new("lift/Slab", vec3(0.0, l.door_top * 0.5, -l.room_d * 0.5 - 0.1));
    put(&mut slab, "SlabBody", [l.door_gap - 0.04, l.door_top, 0.12], [0.0, 0.0, 0.0], O::new().k("metal").t([0.9, 0.9, 0.95]));
    for i in 0..3 {
        put(&mut slab, "SlabStripe", [l.door_gap - 0.02, 0.08, 0.14], [0.0, -0.8 + i as f32 * 0.8, 0.0], O::new().k("label").t([0.8; 3]));
    }
    out.push(slab);
}

/// Build the whole map as data.
pub fn blueprint() -> Blueprint {
    let l = layout();
    let mut groups: Vec<Group> = Vec::new();
    let mut solids: Vec<Aabb> = Vec::new();
    let mut blockers: Vec<Aabb> = Vec::new();
    let mut stations: Vec<StationDef> = Vec::new();
    let mut omni: Vec<Omni> = Vec::new();
    for (idx, id) in l.order.iter().enumerate() {
        let c = l.centre(id);
        groups.push(shell(l, id));
        if id != "control" {
            groups.push(decor(l, id, idx));
            let mut plates = Group::new(&format!("{id}/DoorPlates"), c);
            door_buttons(l, id, &mut plates, &mut stations);
            groups.push(plates);
        }
        let (col, energy, range, y) = l.room_light;
        omni.push(Omni { pos: c + vec3(0.0, y, 0.0), color: [col.x, col.y, col.z], energy, range });
        // an invisible bar in each doorway: doors are used with the door button
        for dir in ['L', 'R'] {
            if l.has_door(id, dir) {
                let sgn = if dir == 'L' { -1.0f32 } else { 1.0 };
                blockers.push(Aabb::from_centre(c + vec3(sgn * (l.room_w * 0.5 + 0.1), l.door_top * 0.5, 0.0), vec3(0.6, l.door_top, l.door_gap)));
            }
        }
    }
    blockers.push(Aabb::from_centre(vec3(0.0, l.door_top * 0.5, -l.room_d * 0.5 - 0.1), vec3(l.door_gap, l.door_top, 0.6)));
    gallery_extras(l, &mut groups, &mut stations);
    workshop_extras(l, &mut groups, &mut stations);
    auditorium_extras(l, &mut groups, &mut stations);
    breaker_extras(l, &mut groups, &mut stations);
    control_extras(l, &mut groups, &mut solids, &mut stations);
    lift_groups(l, &mut groups, &mut stations);
    let (col, energy, range, y) = l.lift_light;
    omni.push(Omni { pos: l.lift + vec3(0.0, y, 0.0), color: [col.x, col.y, col.z], energy, range });
    // props from the shared .bbmodel set
    let mut props = Vec::new();
    for p in &l.props {
        let c = l.centre(&p.room);
        props.push(PropPlace { model: p.model.clone(), pos: c + p.pos, yaw: p.yaw() });
        if p.solid {
            solids.push(p.aabb(c));
        }
    }
    Blueprint { groups, solids, blockers, stations, props, omni }
}

impl Blueprint {
    /// Every solid box the sim positions walk among (cubes marked solid, plus the extras), without the doorway bars.
    pub fn hard_solids(&self) -> Vec<Aabb> {
        let mut out = collect_solids(&self.groups);
        out.extend(self.solids.iter().copied());
        out
    }
}

// ------------------------------------------------------------------------------------------------ props

/// The embedded `.bbmodel` of a prop.
pub fn prop_text(name: &str) -> Option<&'static str> {
    Some(match name {
        "breakerCabinet" => include_str!("../../../last_route/assets/props/breakerCabinet.bbmodel"),
        "serverRack" => include_str!("../../../last_route/assets/props/serverRack.bbmodel"),
        "cableCoil" => include_str!("../../../last_route/assets/props/cableCoil.bbmodel"),
        "gasCylinderRack" => include_str!("../../../last_route/assets/props/gasCylinderRack.bbmodel"),
        "displayCase" => include_str!("../../../last_route/assets/props/displayCase.bbmodel"),
        "framedWallPortrait" => include_str!("../../../last_route/assets/props/framedWallPortrait.bbmodel"),
        "officeWaterCooler" => include_str!("../../../last_route/assets/props/officeWaterCooler.bbmodel"),
        "filingCabinet" => include_str!("../../../last_route/assets/props/filingCabinet.bbmodel"),
        "storageShelving" => include_str!("../../../last_route/assets/props/storageShelving.bbmodel"),
        "supplyCrates" => include_str!("../../../last_route/assets/props/supplyCrates.bbmodel"),
        "stageSpeaker" => include_str!("../../../last_route/assets/props/stageSpeaker.bbmodel"),
        "micStand" => include_str!("../../../last_route/assets/props/micStand.bbmodel"),
        "stageMonitor" => include_str!("../../../last_route/assets/props/stageMonitor.bbmodel"),
        "trashBin" => include_str!("../../../last_route/assets/props/trashBin.bbmodel"),
        _ => return None,
    })
}

/// One mesh per placed prop (needs the GL context: the models carry their own atlas).
fn build_props(places: &[PropPlace]) -> Vec<Mesh> {
    let mut cache: HashMap<String, Option<Model>> = HashMap::new();
    let mut out = Vec::new();
    for p in places {
        let entry = cache.entry(p.model.clone()).or_insert_with(|| prop_text(&p.model).and_then(|t| Model::parse(t, None, &|_| None, None)));
        if let Some(m) = entry {
            let (mut verts, mut idx) = (Vec::new(), Vec::new());
            m.build(&mut verts, &mut idx, p.pos, p.yaw, 0.0, 0.0, WHITE);
            if !verts.is_empty() {
                out.push(Mesh { vertices: verts, indices: idx, texture: Some(m.texture.clone()) });
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ the baked world

fn move_toward(current: f32, target: f32, delta: f32) -> f32 {
    if (target - current).abs() <= delta {
        target
    } else {
        current + (target - current).signum() * delta
    }
}

pub struct StoryWorld {
    pub world: World,
    /// What the player collides with: the world's solids and the doorway bars.
    pub solids: Vec<Aabb>,
    pub stations: Vec<StationDef>,
    props: Vec<Mesh>,
    omni: Vec<Omni>,
    pub lift_progress: f32,
    pub lift_open: bool,
    shown: Option<Puzzle>,
}

impl StoryWorld {
    pub fn new(tex: &Textures) -> StoryWorld {
        let bp = blueprint();
        let mut wb = WorldBuilder::new();
        for g in bp.groups {
            wb.add_group(g);
        }
        wb.solids = bp.solids;
        let world = wb.bake(tex);
        let mut solids = world.solids.clone();
        solids.extend(bp.blockers.iter().copied());
        let props = build_props(&bp.props);
        let mut w = StoryWorld { world, solids, stations: bp.stations, props, omni: bp.omni, lift_progress: 0.0, lift_open: false, shown: None };
        w.show_puzzle(&Puzzle::None);
        w.snap();
        w
    }

    pub fn station_index(&self, id: &str) -> Option<usize> {
        self.stations.iter().position(|s| s.id == id)
    }

    fn set_disabled(&mut self, id: &str, disabled: bool) {
        if let Some(s) = self.stations.iter_mut().find(|s| s.id == id) {
            s.disabled = disabled;
        }
    }

    /// Show the current puzzle on its station: dials with their needles and marks, the parts left on the bench, the breakers
    /// with their levers, the crank's turns. Stations of what is not in play are disabled.
    pub fn show_puzzle(&mut self, z: &Puzzle) {
        if self.shown.as_ref() == Some(z) {
            return;
        }
        self.shown = Some(z.clone());
        for i in 0..5i32 {
            let on = matches!(z, Puzzle::Dials { n, .. } if i < *n);
            self.world.set_visible(&format!("gallery/Dial{i}"), on);
            for p in 0..6i32 {
                let needle = matches!(z, Puzzle::Dials { n, pos, .. } if i < *n && pos.get(i as usize) == Some(&p));
                let mark = matches!(z, Puzzle::Dials { n, target, .. } if i < *n && target.get(i as usize) == Some(&p));
                self.world.set_visible(&format!("gallery/Needle{i}_{p}"), needle);
                self.world.set_visible(&format!("gallery/Mark{i}_{p}"), mark);
            }
            self.set_disabled(&format!("dial_{i}"), !on);
        }
        for i in 0..6i32 {
            let on = matches!(z, Puzzle::Collect { n, taken } if i < *n && !taken.contains(&i));
            self.world.set_visible(&format!("workshop/Part{i}"), on);
            self.set_disabled(&format!("part_{i}"), !on);
        }
        for i in 0..6i32 {
            let live = matches!(z, Puzzle::Switches { n, .. } if i < *n);
            let flipped = match z {
                Puzzle::Switches { order, next, .. } => order.iter().position(|o| *o == i).map_or(false, |rank| (rank as i32) < *next),
                _ => false,
            };
            self.world.set_visible(&format!("breaker/Switch{i}"), live);
            self.world.set_visible(&format!("breaker/SwUp{i}"), live && !flipped);
            self.world.set_visible(&format!("breaker/SwDown{i}"), live && flipped);
            self.set_disabled(&format!("switch_{i}"), !live);
        }
        let turns = match z {
            Puzzle::Crank { done, .. } => (*done).clamp(0, CRANK_POSES - 1),
            _ => 0,
        };
        for k in 0..CRANK_POSES {
            self.world.set_visible(&format!("auditorium/CrankPose{k}"), k == turns);
        }
        self.set_disabled("crank", !matches!(z, Puzzle::Crank { .. }));
        self.set_disabled("terminal", !matches!(z, Puzzle::Code { .. }));
    }

    pub fn set_lift_open(&mut self, on: bool) {
        self.lift_open = on;
    }

    fn apply_lift(&mut self) {
        let l = layout();
        let rise = l.door_top + 1.3 - l.door_top * 0.5;
        self.world.set_offset("lift/Slab", vec3(0.0, self.lift_progress * rise, 0.0));
    }

    /// The lift slab glides at 2 per second.
    pub fn animate(&mut self, dt_s: f32) {
        let target = if self.lift_open { 1.0 } else { 0.0 };
        self.lift_progress = move_toward(self.lift_progress, target, dt_s * 2.0);
        self.apply_lift();
    }

    pub fn snap(&mut self) {
        self.lift_progress = if self.lift_open { 1.0 } else { 0.0 };
        self.apply_lift();
    }

    /// The lights for a view from `cam`: the four room lights nearest to it (the shader takes four), the beacon if it is on.
    pub fn lights_at(&self, cam: Vec3, beacon: Option<Spot>) -> LightSet {
        let l = layout();
        let mut omni = self.omni.clone();
        omni.sort_by(|a, b| a.pos.distance_squared(cam).partial_cmp(&b.pos.distance_squared(cam)).unwrap_or(std::cmp::Ordering::Equal));
        omni.truncate(MAX_OMNI);
        let k = l.ambient_scale;
        LightSet { ambient: [l.ambient[0] * k, l.ambient[1] * k, l.ambient[2] * k], omni, spot: beacon.into_iter().collect(), fog: None }
    }

    /// Draw the map and its props with `lights` from `cam_pos`.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        gfx.shader.begin(lights, cam_pos);
        for m in &self.props {
            draw_mesh(m);
        }
        gfx.shader.end();
    }

    pub fn prop_count(&self) -> usize {
        self.props.len()
    }
}

/// Project a world point to screen pixels with a first-person camera (None behind it).
pub fn project(cam: &crate::gfx::FpCamera, aspect: f32, p: Vec3) -> Option<Vec2> {
    let view = Mat4::look_at_rh(cam.pos, cam.pos + cam.forward(), Vec3::Y);
    let proj = Mat4::perspective_rh_gl(cam.fov_deg.to_radians(), aspect.max(0.01), cam.near, cam.far);
    let clip = proj * view * vec4(p.x, p.y, p.z, 1.0);
    if clip.w <= 0.001 {
        return None;
    }
    let ndc = vec2(clip.x / clip.w, clip.y / clip.w);
    if ndc.x.abs() > 1.3 || ndc.y.abs() > 1.3 {
        return None;
    }
    Some(vec2((ndc.x + 1.0) * 0.5 * screen_width(), (1.0 - ndc.y) * 0.5 * screen_height()))
}

// ------------------------------------------------------------------------------------------------ tests

/// Why this mode cannot start, if so: the Story map, nav file or behaviour file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    Layout::parse(MAP_JSON).err().or_else(|| crate::hunters::NavMap::from_json(NAV_JSON).err().map(|e| format!("story nav: {e}"))).or_else(|| crate::hunters::BehaviourData::parse(BEHAVIOUR_JSON).err().map(|e| format!("story behaviour: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::NavMap;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(Layout::parse("{ not json").is_err());
        assert!(crate::hunters::BehaviourData::parse("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn approx(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn the_embedded_map_parses() {
        let l = Layout::parse(MAP_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(l.order, vec!["breaker", "gallery", "control", "workshop", "auditorium"]);
        assert_eq!((l.room_w, l.room_d, l.height), (5.6, 5.0, 3.6));
        assert_eq!(l.cameras.len(), 6);
        assert_eq!(l.props.len(), 21);
        assert!(l.cameras.iter().all(|c| c.fov > 30.0 && c.fov < 100.0));
    }

    #[test]
    fn rooms_sit_at_the_godot_coordinates() {
        let l = layout();
        assert!(approx(l.centre("breaker"), vec3(-12.0, 0.0, 0.0)));
        assert!(approx(l.centre("gallery"), vec3(-6.0, 0.0, 0.0)));
        assert!(approx(l.centre("control"), Vec3::ZERO));
        assert!(approx(l.centre("workshop"), vec3(6.0, 0.0, 0.0)));
        assert!(approx(l.centre("auditorium"), vec3(12.0, 0.0, 0.0)));
        assert!(approx(l.centre("lift"), vec3(0.0, 0.0, -4.1)));
    }

    #[test]
    fn doors_and_arrivals_follow_the_godot_functions() {
        assert!(approx(door_point("gallery", 'R'), vec3(-6.0 + 2.9, 0.0, 0.0)));
        assert!(approx(door_point("control", 'L'), vec3(-2.9, 0.0, 0.0)));
        assert!(approx(door_point("control", 'C'), vec3(0.0, 0.0, -2.6)));
        // through the left door you come in at the right-hand end of the next room
        assert!(approx(arrival("gallery", 'L'), vec3(-6.0 + 1.3, 0.0, 0.0)));
        assert!(approx(arrival("workshop", 'R'), vec3(6.0 - 1.3, 0.0, 0.0)));
        assert!(approx(arrival("lift", 'C'), vec3(0.0, 0.0, -3.1)));
        assert!(approx(arrival("control", 'C'), vec3(0.0, 0.0, -1.1)));
    }

    #[test]
    fn creepers_close_in_from_their_far_end() {
        // Opalie starts with her far end on the left: at 3 steps she is 2.15 m left of the room's middle
        let a3 = creeper_spot(Who::Opalie, "gallery", 3, -1.0);
        let a0 = creeper_spot(Who::Opalie, "gallery", 0, -1.0);
        assert!(approx(a3, vec3(-6.0 + 1.3 - (0.9 + 3.0 * 0.85), 0.0, 0.5)));
        assert!(a0.x > a3.x, "each step is nearer the door she watches");
        assert!((a0.x - a3.x - 3.0 * 0.85).abs() < 1e-4);
        let p = creeper_spot(Who::Porcelle, "workshop", 2, 1.0);
        assert!((p.z + 0.4).abs() < 1e-4 && p.x < 6.0 + 2.8);
    }

    #[test]
    fn every_room_and_the_lift_are_built() {
        let bp = blueprint();
        for room in ["breaker", "gallery", "control", "workshop", "auditorium"] {
            assert!(bp.groups.iter().any(|g| g.name == format!("{room}/Shell")), "{room} has no shell");
            assert!(bp.omni.iter().any(|o| (o.pos.x - layout().centre(room).x).abs() < 0.01), "{room} has no light");
        }
        assert!(bp.groups.iter().any(|g| g.name == "lift/Car"));
        assert!(bp.groups.iter().any(|g| g.name == "lift/Slab"));
        assert_eq!(bp.omni.len(), 6);
        // the dynamic pieces the game toggles by name
        for name in ["gallery/Dial4", "gallery/Needle4_5", "gallery/Mark0_0", "workshop/Part5", "breaker/Switch5", "breaker/SwUp0", "breaker/SwDown5", "auditorium/CrankPose16"] {
            assert!(bp.groups.iter().any(|g| g.name == name), "missing group {name}");
        }
    }

    #[test]
    fn every_station_exists_with_the_godot_prompts() {
        let bp = blueprint();
        let prompt = |id: &str| bp.stations.iter().find(|s| s.id == id).map(|s| s.prompt.clone());
        assert_eq!(prompt("terminal").as_deref(), Some("TERMINAL // KEY IN THE CODE ON THE NOTE"));
        assert_eq!(prompt("hide_spot").as_deref(), Some("HIDE UNDER THE DESK // HOLD H"));
        assert_eq!(prompt("lift_button").as_deref(), Some("CALL BUTTON // HOLD E"));
        assert_eq!(prompt("crank").as_deref(), Some("STAGE CRANK // TURN"));
        assert_eq!(prompt("door_control_L").as_deref(), Some("GO LEFT // GALLERY"));
        assert_eq!(prompt("door_control_R").as_deref(), Some("GO RIGHT // WORKSHOP"));
        assert_eq!(prompt("door_gallery_L").as_deref(), Some("GO LEFT // BREAKER ROOM"));
        assert_eq!(prompt("door_lift_C").as_deref(), Some("LIFT DOOR // GO OUT TO CONTROL"));
        assert_eq!(bp.stations.iter().filter(|s| s.id.starts_with("dial_")).count(), 5);
        assert_eq!(bp.stations.iter().filter(|s| s.id.starts_with("part_")).count(), 6);
        assert_eq!(bp.stations.iter().filter(|s| s.id.starts_with("switch_")).count(), 6);
        // one door station per door of the rules
        let doors = bp.stations.iter().filter(|s| s.id.starts_with("door_")).count();
        let expected: usize = rules().rooms.iter().map(|r| r.exits.len()).sum();
        assert_eq!(doors, expected);
        // a station for every room's task kind
        let ids: Vec<&str> = bp.stations.iter().map(|s| s.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "station ids are unique");
    }

    #[test]
    fn dial_stations_sit_on_the_lamp_board() {
        let bp = blueprint();
        let s = bp.stations.iter().find(|s| s.id == "dial_0").map(|s| s.pos).unwrap_or(Vec3::ZERO);
        assert!(approx(s, vec3(-6.0 - 2.0, 1.4, -2.5 + 0.2 + 0.12 + 0.1)));
        let k = bp.stations.iter().find(|s| s.id == "crank").map(|s| s.pos).unwrap_or(Vec3::ZERO);
        assert!(approx(k, vec3(12.0, 1.0, 0.5)));
    }

    #[test]
    fn walls_have_gaps_only_where_the_rules_have_doors() {
        let bp = blueprint();
        let shell_of = |room: &str| bp.groups.iter().find(|g| g.name == format!("{room}/Shell")).map(|g| g.cubes.len()).unwrap_or(0);
        // the breaker room has one door, the gallery two
        let has = |room: &str, name: &str| bp.groups.iter().find(|g| g.name == format!("{room}/Shell")).map_or(false, |g| g.cubes.iter().any(|c| c.name == name));
        assert!(has("breaker", "WallLFull") && has("breaker", "WallRPiece"));
        assert!(has("gallery", "WallLPiece") && has("gallery", "WallRPiece"));
        assert!(has("auditorium", "WallLPiece") && has("auditorium", "WallRFull"));
        assert!(has("control", "WallNorthLintel") && !has("gallery", "WallNorthLintel"));
        assert!(shell_of("control") > 20);
        assert_eq!(bp.blockers.len(), 9, "eight doorways in the row's walls and the lift door");
    }

    #[test]
    fn props_stay_inside_their_rooms() {
        let l = layout();
        for p in &l.props {
            assert!(prop_text(&p.model).is_some(), "{} is not embedded", p.model);
            let c = l.centre(&p.room);
            let b = p.aabb(c);
            let (hw, hd) = if p.room == "lift" { (1.5, 1.5) } else { (l.room_w * 0.5, l.room_d * 0.5) };
            assert!(b.min.x >= c.x - hw - 0.1 && b.max.x <= c.x + hw + 0.1, "{} sticks out sideways", p.model);
            assert!(b.min.z >= c.z - hd - 0.1 && b.max.z <= c.z + hd + 0.1, "{} sticks out front or back", p.model);
        }
    }

    #[test]
    fn a_prop_box_turns_with_its_facing() {
        let p = PropDef { model: "x".into(), room: "control".into(), pos: vec3(1.0, 0.0, 1.0), face: 'e', solid: true, min: vec3(-0.5, 0.0, -0.2), max: vec3(0.5, 1.0, 0.3) };
        let b = p.aabb(Vec3::ZERO);
        // facing +x turns the length (x) onto z
        assert!((b.max.z - b.min.z - 1.0).abs() < 1e-4 && (b.max.x - b.min.x - 0.5).abs() < 1e-4);
        assert!((p.yaw() - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn every_camera_room_is_reachable_on_the_nav_graph() {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        let problems = nav.validate();
        assert!(problems.is_empty(), "{problems:?}");
        for cam in &layout().cameras {
            let hub = format!("C:{}", cam.room);
            assert!(!nav.route("C:control", &hub).is_empty(), "no way from control to {}", cam.room);
            assert!(!nav.route(&hub, "C:control").is_empty());
            assert!(!nav.cameras(&cam.room).is_empty(), "{} has no camera node for the stare", cam.room);
            assert!(nav.rooms().iter().any(|r| r.id == cam.room), "{} has no room rectangle", cam.room);
        }
    }

    #[test]
    fn camera_nodes_sit_at_the_camera_mounts() {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        for cam in &layout().cameras {
            let n = nav.cameras(&cam.room);
            let node = n.first().unwrap_or_else(|| panic!("no camera node in {}", cam.room));
            assert!(approx(node.p, cam.pos), "{}: nav camera {:?} vs mount {:?}", cam.room, node.p, cam.pos);
        }
    }

    #[test]
    fn every_hunter_task_spot_is_reachable_and_clear_of_props() {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        let solids = blueprint().hard_solids();
        let mut spots = 0;
        for n in nav.nodes() {
            if n.kind != crate::hunters::NavKind::Task {
                continue;
            }
            spots += 1;
            let legs = nav.route("C:control", &n.key);
            assert!(!legs.is_empty(), "{} is not reachable", n.key);
            for pair in legs.windows(2) {
                if pair[1].jump {
                    continue;
                }
                for k in 0..=20 {
                    let t = k as f32 / 20.0;
                    let p = pair[0].p.lerp(pair[1].p, t);
                    for s in &solids {
                        // walls and floors are not obstacles on the walk: look at boxes of furniture height only
                        if s.max.y <= 0.3 || s.min.y >= 1.6 || s.max.y - s.min.y > 2.5 {
                            continue;
                        }
                        let inside = p.x > s.min.x + 0.02 && p.x < s.max.x - 0.02 && p.z > s.min.z + 0.02 && p.z < s.max.z - 0.02 && p.y < s.max.y - 0.2;
                        let near_end = p.distance(legs[legs.len() - 1].p) < 0.7;
                        assert!(!inside || near_end, "the walk to {} passes through a box at {:?}", n.key, p);
                    }
                }
            }
        }
        assert!(spots >= 15);
    }

    #[test]
    fn the_player_start_is_in_control_and_clear() {
        let l = layout();
        assert_eq!(room_at(l.player_start).as_deref(), Some("control"));
        let bp = blueprint();
        for s in bp.hard_solids() {
            if s.max.y > 0.3 && s.min.y < 1.6 {
                assert!(!s.contains_xz(l.player_start.x, l.player_start.z, 0.24), "the start is inside a solid");
            }
        }
    }

    #[test]
    fn room_at_finds_rooms_and_the_lift() {
        assert_eq!(room_at(vec3(-12.0, 0.0, 0.0)).as_deref(), Some("breaker"));
        assert_eq!(room_at(vec3(6.5, 0.0, 1.0)).as_deref(), Some("workshop"));
        assert_eq!(room_at(vec3(0.0, 0.0, -4.0)).as_deref(), Some("lift"));
        assert_eq!(room_at(vec3(40.0, 0.0, 0.0)), None);
    }

    #[test]
    fn projection_puts_a_point_ahead_in_the_middle() {
        let cam = crate::gfx::FpCamera::new(vec3(0.0, 1.7, 0.0), 0.0, 0.0);
        // screen_width() needs a window; the maths is checked through the matrices directly
        let view = Mat4::look_at_rh(cam.pos, cam.pos + cam.forward(), Vec3::Y);
        let proj = Mat4::perspective_rh_gl(cam.fov_deg.to_radians(), 16.0 / 9.0, cam.near, cam.far);
        let clip = proj * view * vec4(0.0, 1.7, -3.0, 1.0);
        assert!(clip.w > 0.0 && (clip.x / clip.w).abs() < 1e-4 && (clip.y / clip.w).abs() < 1e-4);
        let behind = proj * view * vec4(0.0, 1.7, 3.0, 1.0);
        assert!(behind.w < 0.0);
    }

    #[test]
    fn the_behaviour_file_matches_the_task_spots() {
        let data = crate::hunters::BehaviourData::parse(BEHAVIOUR_JSON).unwrap_or_else(|e| panic!("{e}"));
        let ids = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
        let problems = data.validate(&ids);
        assert!(problems.is_empty(), "{problems:?}");
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        for task in data.tasks.keys() {
            let any = nav.nodes().iter().any(|n| n.task.as_deref() == Some(task.as_str()));
            assert!(any, "no spot for task {task}");
        }
        // every resident has something to do in her own rooms
        for (who, rooms) in [("opalie", vec!["gallery"]), ("porcelle", vec!["workshop", "auditorium"]), ("quenelle", vec!["lift"])] {
            let p = data.params_for(who);
            let usable = p.tasks.iter().filter(|(_, w)| *w > 0.0).any(|(t, _)| rooms.iter().any(|r| !nav.task_spots(r, Some(t.as_str())).is_empty()));
            assert!(usable, "{who} has no task spot in her rooms");
        }
    }
}
