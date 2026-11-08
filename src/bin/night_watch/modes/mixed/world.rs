//! Mixed Shift's 3D map (port of `mixed_world.gd`, `mixed_map.gd` and `mixed_office.gd`): the transit hub around the security
//! office. A concourse with the platform and the ticket hall to the north, the west and east arcades down either side to the
//! office (its two door halls open into them), a cafe between the concourse and the office, lost property and the newsstand at
//! the arcades' south ends, and the vent run above the building (camera 9). Walls with doorway gaps and frames, windows,
//! ceiling lights, security cameras at the Godot positions, and the nine dressings (one per mode's map, `skin.rs`) built from
//! the `last_route` prop models where one fits and from boxes in the skin's palette otherwise.
//!
//! The geometry is pure (`hub_parts`, `office_extras`: groups of cubes, no GL) so tests can look at it; `HubWorld::build`
//! bakes it. The numbers live in `data/mixed/map.json`; the navigation graph the hunters walk is `data/nav/mixed.json`.
#![allow(dead_code)]

use super::data::Side;
use super::skin::{self, Skin, CAMS};
use crate::core::CamLayout;
use crate::gfx::prims::wall_with_gaps;
use crate::gfx::{Cube, Gfx, Group, LightSet, Office, Omni, StationDef, Textures, World, WorldBuilder};
use crate::hunters::bbmodel::Model;
use crate::hunters::nav::NavMap;
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const MAP_JSON: &str = include_str!("../../data/mixed/map.json");
pub const NAV_JSON: &str = include_str!("../../data/nav/mixed.json");

/// Wall height of the hub (H in the Godot file).
pub const H: f32 = 3.6;

// ------------------------------------------------------------------------------------------------ the map data

#[derive(Clone, Debug)]
pub struct Area {
    pub id: String,
    /// x0, x1, z0, z1 in building space (the office is at the origin).
    pub rect: [f32; 4],
    pub tint: [f32; 3],
    pub map: String,
    pub cam: usize,
    pub spots: Vec<(f32, f32)>,
    pub cam_pos: Vec3,
    pub cam_look: Vec3,
}

impl Area {
    pub fn centre(&self) -> Vec3 {
        vec3((self.rect[0] + self.rect[1]) * 0.5, 0.0, (self.rect[2] + self.rect[3]) * 0.5)
    }
    pub fn size(&self) -> (f32, f32) {
        (self.rect[1] - self.rect[0], self.rect[3] - self.rect[2])
    }
    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.rect[0] && x <= self.rect[1] && z >= self.rect[2] && z <= self.rect[3]
    }
}

#[derive(Clone, Debug)]
pub struct WallDef {
    pub along_x: bool,
    pub fixed: f32,
    pub a: f32,
    pub b: f32,
    pub gaps: Vec<(f32, f32)>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct SignDef {
    pub map: String,
    pub p: Vec3,
    pub along_x: bool,
}

#[derive(Clone, Debug)]
pub struct MapData {
    pub height: f32,
    pub thick: f32,
    pub areas: Vec<Area>,
    pub walls: Vec<WallDef>,
    pub signs: Vec<SignDef>,
    pub sim_to_map: Vec<(String, String)>,
    pub layout_rooms: Vec<(String, Vec2)>,
    pub layout_edges: Vec<(String, String)>,
    pub layout_entrances: Vec<String>,
    pub layout_vents: Vec<String>,
    pub layout_office: Vec2,
    pub vent_origin: Vec3,
    pub vent_cam_from: Vec3,
    pub vent_cam_look: Vec3,
    pub vent_spot_x: f32,
    pub vent_stand: [Vec3; 3],
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn v3(v: &Value) -> Vec3 {
    let a = v.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    vec3(a.first().map(f).unwrap_or(0.0), a.get(1).map(f).unwrap_or(0.0), a.get(2).map(f).unwrap_or(0.0))
}

fn pairs(v: &Value) -> Vec<(f32, f32)> {
    v.as_array().map(|a| a.iter().map(|p| (f(&p[0]), f(&p[1]))).collect()).unwrap_or_default()
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect()).unwrap_or_default()
}

impl MapData {
    pub fn parse(src: &str) -> Result<MapData, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("mixed map: {e}"))?;
        let mut areas = Vec::new();
        for (id, a) in v["areas"].as_object().ok_or("mixed map: no areas")? {
            let r = a["rect"].as_array().ok_or_else(|| format!("mixed map: area {id} has no rect"))?;
            if r.len() < 4 {
                return Err(format!("mixed map: area {id} needs four numbers"));
            }
            let t = v3(&a["tint"]);
            areas.push(Area {
                id: id.clone(),
                rect: [f(&r[0]), f(&r[1]), f(&r[2]), f(&r[3])],
                tint: [t.x, t.y, t.z],
                map: a["map"].as_str().unwrap_or("").to_string(),
                cam: a["cam"].as_u64().unwrap_or(0) as usize,
                spots: pairs(&a["spots"]),
                cam_pos: v3(&a["cam_pos"]),
                cam_look: v3(&a["cam_look"]),
            });
        }
        let mut walls = Vec::new();
        for w in v["walls"].as_array().ok_or("mixed map: no walls")? {
            walls.push(WallDef {
                along_x: w["along_x"].as_bool().unwrap_or(true),
                fixed: f(&w["fixed"]),
                a: f(&w["a"]),
                b: f(&w["b"]),
                gaps: pairs(&w["gaps"]),
                name: w["name"].as_str().unwrap_or("WallHub").to_string(),
            });
        }
        let mut signs = Vec::new();
        for s in v["signs"].as_array().ok_or("mixed map: no signs")? {
            signs.push(SignDef { map: s["map"].as_str().unwrap_or("").to_string(), p: v3(&s["p"]), along_x: s["along_x"].as_bool().unwrap_or(true) });
        }
        let mut sim_to_map = Vec::new();
        for (k, m) in v["sim_to_map"].as_object().ok_or("mixed map: no sim_to_map")? {
            sim_to_map.push((k.clone(), m.as_str().unwrap_or("").to_string()));
        }
        let lay = &v["layout"];
        let mut layout_rooms = Vec::new();
        for (k, p) in lay["rooms"].as_object().ok_or("mixed map: no layout rooms")? {
            layout_rooms.push((k.clone(), vec2(f(&p[0]), f(&p[1]))));
        }
        let layout_edges = lay["edges"].as_array().map(|a| a.iter().map(|e| (e[0].as_str().unwrap_or("").to_string(), e[1].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
        let run = &v["vent_run"];
        let stand = &v["vent_stand"];
        Ok(MapData {
            height: f(&v["height"]),
            thick: f(&v["thick"]),
            areas,
            walls,
            signs,
            sim_to_map,
            layout_rooms,
            layout_edges,
            layout_entrances: names(&lay["entrances"]),
            layout_vents: names(&lay["vents"]),
            layout_office: vec2(f(&lay["office"][0]), f(&lay["office"][1])),
            vent_origin: v3(&run["origin"]),
            vent_cam_from: v3(&run["cam_from"]),
            vent_cam_look: v3(&run["cam_look"]),
            vent_spot_x: f(&run["spot_x"]),
            vent_stand: [v3(&stand["L"]), v3(&stand["R"]), v3(&stand["T"])],
        })
    }

    pub fn area(&self, sim_room: &str) -> Option<&Area> {
        self.areas.iter().find(|a| a.id == sim_room)
    }
}

static MAP: OnceLock<MapData> = OnceLock::new();

pub fn map() -> &'static MapData {
    MAP.get_or_init(|| MapData::parse(MAP_JSON).unwrap_or_else(|e| panic!("{e}")))
}

/// The map room a sim room is played in ("stage" -> "platform"); an unknown room maps to itself.
pub fn map_for(sim_room: &str) -> String {
    for (k, m) in &map().sim_to_map {
        if k == sim_room {
            return m.clone();
        }
    }
    sim_room.to_string()
}

pub fn sim_for(map_room: &str) -> String {
    for (k, m) in &map().sim_to_map {
        if m == map_room {
            return k.clone();
        }
    }
    map_room.to_string()
}

/// The navigation graph the hunters walk (data/nav/mixed.json).
pub fn nav() -> NavMap {
    try_nav().unwrap_or_else(|e| panic!("{e}"))
}

/// The same, as an error when the graph does not parse or does not validate. Never an empty map: a broken graph refuses the
/// mode with a message (`MixedGame` shows it, `--check` reports it) instead of letting nobody walk.
pub fn try_nav() -> Result<NavMap, String> {
    parse_nav(NAV_JSON)
}

/// Parse and validate a nav graph source (the embedded one, or a broken one in a test).
pub fn parse_nav(src: &str) -> Result<NavMap, String> {
    let n = NavMap::from_json(src).map_err(|e| format!("mixed nav graph: {e}"))?;
    let problems = n.validate();
    if problems.is_empty() {
        Ok(n)
    } else {
        Err(format!("mixed nav graph: {}", problems.join("; ")))
    }
}

pub fn centre_of(sim_room: &str) -> Vec3 {
    map().area(sim_room).map(|a| a.centre()).unwrap_or(Vec3::ZERO)
}

/// The feed camera of a sim room: high in a corner of the room, looking at its middle. The vent run's camera looks down the duct.
pub fn camera_transform(sim_room: &str) -> (Vec3, Vec3) {
    let m = map();
    if sim_room == "vent" {
        return (m.vent_origin + m.vent_cam_from, m.vent_origin + m.vent_cam_look);
    }
    match m.area(sim_room) {
        Some(a) => (a.cam_pos, a.cam_look),
        None => (vec3(0.0, 3.0, 4.0), vec3(0.0, 0.8, 0.0)),
    }
}

/// Where a hunter stands in a room (slot 0 or 1).
pub fn stand_position(sim_room: &str, slot: usize) -> Vec3 {
    centre_of(sim_room) + if slot == 0 { vec3(1.0, 0.0, 0.6) } else { vec3(-1.0, 0.0, -0.6) }
}

/// A spot of the vent run (0 the far hatch .. 4 the right branch).
pub fn vent_spot_pos(spot: usize) -> Vec3 {
    let m = map();
    m.vent_origin + vec3(m.vent_spot_x + spot as f32, 0.0, 0.0)
}

/// Where a vent crawler stands in the office when she is at her vent (her face in the opening).
pub fn vent_stand(side: Side) -> Vec3 {
    map().vent_stand[side.idx()]
}

/// The minimap for a skin: the hub's layout with that skin's labels.
pub fn layout_for(skin_idx: usize) -> CamLayout {
    let m = map();
    let s = skin::skin(skin_idx);
    CamLayout {
        rooms: m.layout_rooms.clone(),
        edges: m.layout_edges.clone(),
        entrances: m.layout_entrances.clone(),
        cams: CAMS.iter().map(|c| c.to_string()).collect(),
        labels: CAMS.iter().map(|c| (c.to_string(), s.label(c).to_string())).collect(),
        vents: m.layout_vents.clone(),
        office: Some(m.layout_office),
    }
}

// ------------------------------------------------------------------------------------------------ lights

/// The player's view: the skin's ambient light with the office's own lamps.
pub fn world_lights(office_lights: LightSet, skin_idx: usize) -> LightSet {
    let mut l = office_lights;
    l.ambient = skin::skin(skin_idx).light;
    l
}

/// A camera feed: the skin's ambient light, brightened so a security picture can be read, and one warm lamp over the room.
pub fn feed_lights(skin_idx: usize, sim_room: &str) -> LightSet {
    let s = skin::skin(skin_idx);
    let amb = [(s.light[0] * 1.9).min(1.0), (s.light[1] * 1.9).min(1.0), (s.light[2] * 1.9).min(1.0)];
    let mut omni = Vec::new();
    if sim_room == "vent" {
        let m = map();
        omni.push(Omni { pos: m.vent_origin + vec3(0.0, 2.0, 0.0), color: [0.8, 0.85, 1.0], energy: 1.2, range: 9.0 });
    } else if let Some(a) = map().area(sim_room) {
        omni.push(Omni { pos: a.centre() + vec3(0.0, 3.2, 0.0), color: [1.0, 0.92, 0.75], energy: 1.2, range: 14.0 });
    }
    LightSet { ambient: amb, omni, spot: Vec::new(), fog: None }
}

// ------------------------------------------------------------------------------------------------ cube helpers

/// A tint from Godot's HDR-ish colours: the brightest channel is brought to 1.
fn fix(c: [f32; 3]) -> [f32; 3] {
    let m = c[0].max(c[1]).max(c[2]);
    if m > 1.0 {
        [c[0] / m, c[1] / m, c[2] / m]
    } else {
        c
    }
}

fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn put(g: &mut Group, name: &str, kind: &str, tint: [f32; 3], size: [f32; 3], pos: [f32; 3]) {
    g.add(Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).kind(kind).tint(fix(tint)));
}

const BALLOON: [[f32; 3]; 5] = [[1.9, 0.5, 0.5], [0.6, 1.0, 1.9], [1.9, 1.6, 0.4], [0.7, 1.8, 0.7], [1.8, 0.7, 1.7]];

/// A prop model to place: a key of `prop_text`, the spot (y is the floor or the surface it stands on) and the facing.
#[derive(Clone, Debug)]
pub struct PropPlace {
    pub key: &'static str,
    pub pos: Vec3,
    pub yaw: f32,
    pub tint: [f32; 3],
}

/// The embedded prop models (from `last_route/assets/props`, read-only).
pub fn prop_text(key: &str) -> Option<&'static str> {
    Some(match key {
        "mallBench" => include_str!("../../../last_route/assets/props/mallBench.bbmodel"),
        "waitingChairs" => include_str!("../../../last_route/assets/props/waitingChairs.bbmodel"),
        "vendingMachine" => include_str!("../../../last_route/assets/props/vendingMachine.bbmodel"),
        "restaurantPartyTable" => include_str!("../../../last_route/assets/props/restaurantPartyTable.bbmodel"),
        "arcadeCabinet" => include_str!("../../../last_route/assets/props/arcadeCabinet.bbmodel"),
        "micStand" => include_str!("../../../last_route/assets/props/micStand.bbmodel"),
        "stageSpeaker" => include_str!("../../../last_route/assets/props/stageSpeaker.bbmodel"),
        "singleBed" => include_str!("../../../last_route/assets/props/singleBed.bbmodel"),
        "bedroomWardrobe" => include_str!("../../../last_route/assets/props/bedroomWardrobe.bbmodel"),
        "toyChest" => include_str!("../../../last_route/assets/props/toyChest.bbmodel"),
        "labBench" => include_str!("../../../last_route/assets/props/labBench.bbmodel"),
        "controlConsole" => include_str!("../../../last_route/assets/props/controlConsole.bbmodel"),
        "storageBarrel" => include_str!("../../../last_route/assets/props/storageBarrel.bbmodel"),
        "supplyCrates" => include_str!("../../../last_route/assets/props/supplyCrates.bbmodel"),
        _ => return None,
    })
}

pub const PROP_KEYS: [&str; 14] = [
    "mallBench",
    "waitingChairs",
    "vendingMachine",
    "restaurantPartyTable",
    "arcadeCabinet",
    "micStand",
    "stageSpeaker",
    "singleBed",
    "bedroomWardrobe",
    "toyChest",
    "labBench",
    "controlConsole",
    "storageBarrel",
    "supplyCrates",
];

fn key_of(name: &str) -> &'static str {
    PROP_KEYS.iter().copied().find(|k| *k == name).unwrap_or("supplyCrates")
}

// ------------------------------------------------------------------------------------------------ the hub's geometry

pub struct HubParts {
    pub groups: Vec<Group>,
    pub props: Vec<PropPlace>,
}

/// Everything of the hub for one skin, as plain cubes in named groups: HubShell (floors, ceilings, walls, frames), HubFit
/// (signs, windows, lights, cameras), HubDress (the props), VentRun, and MusicBox (moved to the winder's room by the game).
pub fn hub_parts(skin_idx: usize) -> HubParts {
    let m = map();
    let sk = skin::skin(skin_idx);
    let wall_tint = mul([1.3, 1.25, 1.4], sk.tint);
    let mut shell = Group::new("HubShell", Vec3::ZERO);
    for a in &m.areas {
        let (w, l) = a.size();
        let c = a.centre();
        put(&mut shell, "FloorHub", "floor", mul(a.tint, sk.tint), [w, 0.2, l], [c.x, -0.1, c.z]);
        put(&mut shell, "CeilingHub", "concrete", [0.8, 0.8, 0.9], [w, 0.2, l], [c.x, H + 0.1, c.z]);
    }
    for w in &m.walls {
        wall_with_gaps(&mut shell, w.along_x, w.fixed, w.a, w.b, &w.gaps, H, m.thick, fix(wall_tint), &w.name);
        // a frame round every doorway
        for (s, e) in &w.gaps {
            for edge in [*s, *e] {
                let (size, pos) = if w.along_x { ([0.34, 2.6, 0.34], [edge, 1.3, w.fixed]) } else { ([0.34, 2.6, 0.34], [w.fixed, 1.3, edge]) };
                put(&mut shell, "DoorFrameHub", "metal", [0.45, 0.47, 0.55], size, pos);
            }
            let mid = (s + e) * 0.5;
            let len = (e - s) + 0.34;
            let (size, pos) = if w.along_x { ([len, 0.2, 0.34], [mid, 2.66, w.fixed]) } else { ([0.34, 0.2, len], [w.fixed, 2.66, mid]) };
            put(&mut shell, "DoorFrameHub", "metal", [0.45, 0.47, 0.55], size, pos);
        }
    }

    let mut fit = Group::new("HubFit", Vec3::ZERO);
    for s in &m.signs {
        let size = if s.along_x { [3.0, 0.5, 0.1] } else { [0.1, 0.5, 3.0] };
        put(&mut fit, "SignHub", "label", [1.5, 1.2, 0.5], size, [s.p.x, s.p.y, s.p.z]);
    }
    // ceiling light panels: one per small room, a row in the long arcades
    for a in &m.areas {
        let (w, l) = a.size();
        let c = a.centre();
        let n = if l > 20.0 { 3 } else if w > 14.0 { 2 } else { 1 };
        for i in 0..n {
            let t = if n == 1 { 0.0 } else { (i as f32 / (n - 1) as f32 - 0.5) * 0.7 };
            let (px, pz) = if l > w { (c.x, c.z + t * l) } else { (c.x + t * w, c.z) };
            put(&mut fit, "CeilingPanelHub", "light", [1.0, 0.95, 0.8], [1.6, 0.06, 0.8], [px, H - 0.04, pz]);
        }
    }
    // windows on the outer walls: glass with a pale sky behind it
    for x in [-6.0f32, 0.0, 6.0] {
        put(&mut fit, "GlassWindow", "glass", [0.5, 0.7, 0.9], [3.0, 1.3, 0.05], [x, 1.9, -45.86]);
    }
    for (x, z) in [(-19.86f32, -20.0f32), (-19.86, -5.0), (19.86, -20.0), (19.86, -5.0)] {
        put(&mut fit, "GlassWindow", "glass", [0.5, 0.7, 0.9], [0.05, 1.3, 3.0], [x, 1.9, z]);
    }
    put(&mut fit, "GlassWindow", "glass", [0.5, 0.7, 0.9], [2.4, 1.3, 0.05], [-17.5, 1.9, -45.86]);
    // the platform edge stripe and vent grilles (the way into the vent run)
    put(&mut fit, "StripeEdge", "label", [1.0, 0.8, 0.2], [19.0, 0.02, 0.4], [0.0, 0.011, -31.0]);
    for (x, z) in [(-19.86f32, 11.0f32), (19.86, 11.0), (-19.86, 3.0), (19.86, 3.0)] {
        put(&mut fit, "VentGrilleHub", "metal", [0.6, 0.62, 0.68], [0.08, 0.7, 1.1], [x, 2.2, z]);
        for i in 0..4 {
            put(&mut fit, "VentGrilleBar", "metal", [0.8, 0.8, 0.85], [0.1, 0.04, 1.0], [x, 1.95 + i as f32 * 0.17, z]);
        }
    }
    // the security cameras, at the Godot positions
    for a in &m.areas {
        put(&mut fit, "CamBody", "plastic", [0.3, 0.3, 0.34], [0.34, 0.22, 0.5], [a.cam_pos.x, a.cam_pos.y + 0.2, a.cam_pos.z]);
        put(&mut fit, "CamBracket", "metal", [0.5, 0.5, 0.55], [0.08, 0.3, 0.08], [a.cam_pos.x, a.cam_pos.y + 0.42, a.cam_pos.z]);
        put(&mut fit, "LedCam", "green", [1.0, 1.0, 1.0], [0.05, 0.05, 0.05], [a.cam_pos.x, a.cam_pos.y + 0.28, a.cam_pos.z - 0.26]);
    }

    // the vent run (CAM 9) above the building
    let mut run = Group::new("VentRun", m.vent_origin);
    put(&mut run, "DuctFloor", "metal", [0.7, 0.72, 0.78], [8.0, 0.2, 2.2], [0.0, -0.1, 0.0]);
    for sz in [-1.1f32, 1.1] {
        put(&mut run, "DuctSide", "metal", [0.6, 0.62, 0.7], [8.0, 2.4, 0.15], [0.0, 1.2, sz]);
    }
    put(&mut run, "DuctTop", "metal", [0.5, 0.52, 0.6], [8.0, 0.15, 2.4], [0.0, 2.45, 0.0]);
    put(&mut run, "DuctEnd", "metal", [0.4, 0.4, 0.45], [0.15, 2.4, 2.2], [-4.0, 1.2, 0.0]);
    for i in 0..7 {
        put(&mut run, "DuctRib", "metal", [0.7, 0.7, 0.75], [0.08, 0.12, 2.1], [-3.0 + i as f32, 2.3, 0.0]);
        for rz in [-1.0f32, 1.0] {
            put(&mut run, "DuctRib", "metal", [0.7, 0.7, 0.75], [0.08, 2.3, 0.12], [-3.0 + i as f32, 1.2, rz]);
        }
    }
    for bz in [-1.0f32, 1.0] {
        put(&mut run, "GlowDuct", "light", [0.8, 0.8, 1.0], [0.8, 0.02, 0.4], [if bz < 0.0 { 0.5 } else { 1.5 }, 0.01, bz * 0.7]);
    }

    // the music box (moved to the winder's room)
    let mut mb = Group::new("MusicBox", Vec3::ZERO);
    put(&mut mb, "BoxMusicBody", "wood", [1.2, 0.9, 1.1], [0.5, 0.3, 0.4], [0.0, 0.15, 0.0]);
    put(&mut mb, "GlowGauge", "light", [1.0, 1.0, 1.0], [0.3, 0.03, 0.03], [0.0, 0.36, 0.2]);

    let mut dress = Group::new("HubDress", Vec3::ZERO);
    let mut props = Vec::new();
    for a in &m.areas {
        let c = a.centre();
        for (k, (sx, sz)) in a.spots.iter().enumerate() {
            dress_spot(&mut dress, &mut props, sk, vec3(*sx, 0.0, *sz), k, c);
        }
    }
    HubParts { groups: vec![shell, fit, dress, run, mb], props }
}

fn prop(props: &mut Vec<PropPlace>, key: &str, pos: Vec3, centre: Vec3) {
    props.push(PropPlace { key: key_of(key), pos, yaw: crate::hunters::yaw_toward(pos, centre), tint: [1.0, 1.0, 1.0] });
}

/// One dressing spot of a room (the same three corner spots as Godot's `_dress`; the lanes along the middle stay clear).
fn dress_spot(g: &mut Group, props: &mut Vec<PropPlace>, sk: &Skin, sp: Vec3, k: usize, centre: Vec3) {
    let p = |x: f32, y: f32, z: f32| [sp.x + x, y, sp.z + z];
    let b = |i: usize| BALLOON[i % 5];
    match sk.kit {
        "bench" => {
            match k % 3 {
                0 => prop(props, "mallBench", sp, centre),
                1 => prop(props, "waitingChairs", sp, centre),
                _ => prop(props, "vendingMachine", sp, centre),
            }
            if k % 2 == 0 {
                put(g, "BoardHub", "plastic", [0.4, 0.4, 0.45], [1.4, 0.8, 0.1], p(0.0, 2.4, -0.4));
                put(g, "ScreenBoardHub", "screen", [1.0, 1.0, 1.0], [1.2, 0.6, 0.04], p(0.0, 2.4, -0.33));
            } else {
                put(g, "CaseHub", "fabric", b(k), [0.7, 0.5, 0.3], p(1.4, 0.25, 0.3));
            }
        }
        "party" => {
            match k % 3 {
                0 => {
                    prop(props, "restaurantPartyTable", sp, centre);
                    put(g, "CakeTable", "fabric", [1.8, 1.5, 1.6], [0.32, 0.22, 0.32], p(0.0, 0.93, 0.0));
                }
                1 => {
                    for i in 0..4 {
                        let h = 2.0 + (i % 3) as f32 * 0.35;
                        put(g, "StringBalloon", "plastic", [1.6, 1.6, 1.6], [0.02, h, 0.02], p(-0.3 + i as f32 * 0.2, h * 0.5, 0.0));
                        put(g, "BalloonCube", "fabric", b(i), [0.42, 0.5, 0.42], p(-0.3 + i as f32 * 0.2, h + 0.2, 0.0));
                    }
                }
                _ => prop(props, "arcadeCabinet", sp, centre),
            }
            put(g, "StreamerHub", "fabric", b(k + 2), [5.0, 0.08, 0.1], p(0.0, 3.0, 0.0));
        }
        "yard" => match k % 3 {
            0 => {
                put(g, "BodyContainer", "metal", b(k), [3.0, 1.4, 1.3], p(0.0, 0.7, 0.0));
                for i in 0..4 {
                    put(g, "RibContainer", "metal", [0.9, 0.7, 0.6], [0.06, 1.4, 1.34], p(-1.2 + i as f32 * 0.8, 0.7, 0.0));
                }
            }
            1 => {
                prop(props, "supplyCrates", sp, centre);
                for i in 0..5 {
                    let t = 1.0 - 0.08 * i as f32;
                    put(g, "ScrapBlock", "metal", [1.6 * t, 0.9, 0.6], [0.9 + (i % 3) as f32 * 0.3, 0.6 + (i % 2) as f32 * 0.4, 0.9], p(1.6 + (i % 3) as f32 * 0.9, 0.35 + (i / 3) as f32 * 0.7, -0.3 + (i % 2) as f32 * 0.6));
                }
            }
            _ => {
                for i in 0..3 {
                    prop(props, "storageBarrel", sp + vec3(i as f32 * 0.8, 0.0, (i % 2) as f32 * 0.5), centre);
                }
                put(g, "LegCrane", "metal", [1.7, 1.4, 0.3], [0.3, 3.0, 0.3], p(-1.0, 1.5, 0.0));
                put(g, "BeamCrane", "metal", [1.7, 1.4, 0.3], [3.0, 0.3, 0.3], p(0.4, 3.0, 0.0));
            }
        },
        "pizzeria" => match k % 3 {
            0 => {
                prop(props, "restaurantPartyTable", sp, centre);
                for sx in [-0.5f32, 0.5] {
                    put(g, "HatPizza", "plastic", b((sx > 0.0) as usize + k % 3), [0.2, 0.3, 0.2], p(sx, 0.97, 0.3));
                }
                put(g, "CakePizza", "fabric", [1.8, 1.5, 1.6], [0.3, 0.2, 0.3], p(0.0, 0.92, 0.0));
            }
            1 => {
                put(g, "RugPizza", "fabric", [1.7, 0.5, 0.5], [2.4, 0.02, 1.6], p(0.0, 0.01, 0.0));
                put(g, "BannerPizza", "fabric", [1.7, 1.4, 0.4], [3.0, 0.5, 0.05], p(0.0, 2.8, 0.0));
            }
            _ => prop(props, "arcadeCabinet", sp, centre),
        },
        "studio" => match k % 3 {
            0 => {
                put(g, "DeskStudio", "wood", [0.9, 0.8, 0.7], [1.8, 0.08, 0.8], p(0.0, 0.76, 0.0));
                for sx in [-0.8f32, 0.8] {
                    put(g, "DeskLegStudio", "metal", [0.6, 0.6, 0.65], [0.08, 0.72, 0.7], p(sx, 0.36, 0.0));
                }
                prop(props, "micStand", sp + vec3(0.3, 0.8, 0.0), centre);
                put(g, "ReelStudio", "metal", [1.2, 1.2, 1.3], [0.3, 0.06, 0.3], p(-0.5, 0.83, 0.0));
            }
            1 => {
                prop(props, "stageSpeaker", sp, centre);
                put(g, "SpeakerStudio", "plastic", [0.5, 0.5, 0.55], [0.7, 0.7, 0.6], p(1.1, 0.35, 0.0));
            }
            _ => {
                put(g, "SignOnAir", "label", [1.9, 0.3, 0.3], [1.0, 0.35, 0.08], p(0.0, 2.6, 0.0));
                put(g, "RackStudio", "metal", [0.6, 0.65, 0.7], [0.9, 0.5, 0.9], p(0.0, 0.25, 0.0));
            }
        },
        "house" => match k % 3 {
            0 => {
                prop(props, "singleBed", sp, centre);
                put(g, "BlanketHouse", "fabric", b(k), [0.9, 0.06, 1.5], p(2.0, 0.05, 0.0));
            }
            1 => prop(props, "bedroomWardrobe", sp, centre),
            _ => {
                prop(props, "toyChest", sp, centre);
                for i in 0..3 {
                    put(g, "BlockHouse", "plastic", b(i), [0.25, 0.25, 0.25], p(-0.6 - i as f32 * 0.2, 0.13, 0.3 * (i % 2) as f32));
                }
            }
        },
        "lab" => match k % 3 {
            0 => {
                prop(props, "labBench", sp, centre);
                put(g, "BeakerLab", "plastic", [0.6, 1.8, 1.2], [0.16, 0.3, 0.16], p(0.5, 1.05, 0.0));
            }
            1 => {
                for i in 0..3 {
                    put(g, "PipeLab", "metal", [1.2 * (1.0 - 0.1 * i as f32), 1.1, 0.6], [0.12, 3.0, 0.12], p(-0.4 + i as f32 * 0.4, 1.5, 0.0));
                }
            }
            _ => {
                prop(props, "controlConsole", sp, centre);
                put(g, "GlowStatusLab", "label", [0.4, 1.9, 0.6], [0.2, 0.2, 0.06], p(0.5, 1.9, 0.3));
            }
        },
        "shaft" => match k % 3 {
            0 => {
                put(g, "PillarShaft", "metal", [0.6, 0.65, 0.8], [0.6, 3.4, 0.6], p(0.0, 1.7, 0.0));
                put(g, "CableShaft", "metal", [0.3, 0.3, 0.35], [0.05, 3.0, 0.05], p(0.45, 1.6, 0.0));
            }
            1 => {
                put(g, "BaseShaft", "metal", [1.3, 1.2, 0.5], [1.6, 0.1, 1.6], p(0.0, 0.05, 0.0));
                for sx in [-0.75f32, 0.75] {
                    put(g, "FrameShaft", "metal", [1.3, 1.2, 0.5], [0.1, 2.6, 0.1], p(sx, 1.3, 0.0));
                }
                put(g, "BeamShaft", "metal", [1.3, 1.2, 0.5], [1.6, 0.1, 0.1], p(0.0, 2.6, 0.0));
            }
            _ => {
                for i in 0..4 {
                    put(g, "WeightShaft", "metal", [0.5, 0.5, 0.58], [0.9, 0.3, 0.6], p(0.0, 0.15 + i as f32 * 0.32, 0.0));
                }
            }
        },
        _ => match k % 3 {
            0 => {
                put(g, "GroundCamp", "fabric", [0.7, 0.9, 0.6], [2.0, 0.1, 2.0], p(0.0, 0.05, 0.0));
                for sx in [-0.5f32, 0.5] {
                    put(g, "TentCamp", "fabric", [1.5 * (1.0 - 0.15 * (sx > 0.0) as i32 as f32), 0.9, 0.4], [1.0, 1.4, 1.8], p(sx, 0.75, 0.0));
                }
            }
            1 => {
                for i in 0..3 {
                    put(g, "LogCamp", "wood", [1.2, 0.8, 0.5], [1.2, 0.3, 0.3], p(0.0, 0.15, (i as f32 - 1.0) * 0.5));
                }
            }
            _ => {
                put(g, "PostCamp", "wood", [1.1, 0.8, 0.6], [0.1, 2.2, 0.1], p(0.0, 1.1, 0.0));
                put(g, "SignLanternCamp", "label", [1.9, 1.5, 0.6], [0.3, 0.4, 0.3], p(0.0, 2.3, 0.0));
            }
        },
    }
}

// ------------------------------------------------------------------------------------------------ the office's extras

pub const VENT_POS: [Vec3; 3] = [vec3(-2.3, 0.45, -2.5), vec3(2.3, 0.45, -2.5), vec3(0.0, 3.55, -1.95)];

/// The two new vent hatches (right, in the back wall; top, in the ceiling) and, for each of the three, a shutter (shown while
/// it is sealed) and a pale flash (the vent light's glowing opening). Groups `VentHatchR`, `VentCeiling`, `VentShutter{L,R,T}`,
/// `VentFlash{L,R,T}`.
pub fn office_extras() -> Vec<Group> {
    let mut out = Vec::new();
    let mut r = Group::new("VentHatchR", VENT_POS[1]);
    put(&mut r, "VentFrame", "wood", [1.0, 1.0, 1.0], [0.84, 0.54, 0.1], [0.0, 0.0, 0.05]);
    for i in 0..5 {
        put(&mut r, "VentLouver", "metal", [0.7, 0.7, 0.75], [0.7, 0.035, 0.06], [0.0, -0.2 + i as f32 * 0.1, 0.12]);
    }
    out.push(r);
    let mut t = Group::new("VentCeiling", VENT_POS[2]);
    put(&mut t, "VentFrameTop", "metal", [1.0, 1.0, 1.0], [0.9, 0.06, 0.9], [0.0, 0.0, 0.0]);
    for i in 0..4 {
        put(&mut t, "VentLouverTop", "metal", [0.7, 0.7, 0.75], [0.8, 0.03, 0.06], [0.0, -0.04, -0.3 + i as f32 * 0.2]);
    }
    out.push(t);
    for side in Side::ALL {
        let at = VENT_POS[side.idx()];
        let mut sh = Group::new(&format!("VentShutter{}", side.key()), at);
        let mut fl = Group::new(&format!("VentFlash{}", side.key()), at);
        if side == Side::T {
            put(&mut sh, "VentShutterSlab", "metal", [0.6, 0.6, 0.65], [1.0, 0.1, 1.0], [0.0, -0.08, 0.0]);
            put(&mut fl, "GlowFlashTop", "light", [1.0, 1.0, 1.0], [0.6, 0.02, 0.6], [0.0, -0.1, 0.0]);
        } else {
            put(&mut sh, "VentShutterSlab", "metal", [0.6, 0.6, 0.65], [0.9, 0.6, 0.08], [0.0, 0.0, 0.18]);
            put(&mut fl, "GlowFlash", "light", [1.0, 1.0, 1.0], [0.6, 0.36, 0.02], [0.0, 0.0, 0.14]);
        }
        out.push(sh);
        out.push(fl);
    }
    out
}

/// The stations of the three vents (the Classic hatch is the left one).
pub fn vent_station(side: Side) -> StationDef {
    let (id, prompt, pos, radius) = match side {
        Side::L => ("ventL", "LEFT VENT // HOLD TO SEAL", vec3(-2.3, 0.45, -2.3), 0.5),
        Side::R => ("ventR", "RIGHT VENT // HOLD TO SEAL", vec3(2.3, 0.45, -2.3), 0.5),
        Side::T => ("ventT", "TOP VENT // HOLD TO SEAL", vec3(0.0, 2.45, -1.95), 0.6),
    };
    StationDef { id: id.to_string(), prompt: prompt.to_string(), pos, focus: Vec3::ZERO, radius, disabled: false }
}

/// Make the Classic office a hub office: no door lights (the pulsing light is the tell), the breaker box not used, three vents.
pub fn tweak_office(office: &mut Office) {
    for door in ["doorL", "doorR"] {
        let tag = if door == "doorL" { "L" } else { "R" };
        if let Some(st) = office.station_mut(&format!("{door}_light")) {
            st.disabled = true;
            st.prompt = String::new();
        }
        office.world.set_cube_scale(&format!("ControlPanel{tag}"), &format!("Button{tag}LIGHT"), Vec3::ZERO);
    }
    if let Some(st) = office.station_mut("breaker") {
        st.prompt = "BREAKER BOX // NOT USED IN THIS MODE".to_string();
    }
    let left = vent_station(Side::L);
    match office.station_mut("vent") {
        Some(st) => {
            st.id = left.id;
            st.prompt = left.prompt;
        }
        None => office.add_station(left),
    }
    office.add_station(vent_station(Side::R));
    office.add_station(vent_station(Side::T));
    for side in Side::ALL {
        office.world.set_visible(&format!("VentShutter{}", side.key()), false);
        office.world.set_visible(&format!("VentFlash{}", side.key()), false);
    }
}

// ------------------------------------------------------------------------------------------------ the baked hub

/// The hub for one skin: the baked cubes and the prop meshes. Needs a GL context.
pub struct HubWorld {
    pub world: World,
    props: Vec<Mesh>,
    pub skin: usize,
}

impl HubWorld {
    pub fn build(tex: &Textures, skin_idx: usize) -> HubWorld {
        let parts = hub_parts(skin_idx);
        let mut wb = WorldBuilder::new();
        for g in parts.groups {
            wb.add_group(g);
        }
        let world = wb.bake(tex);
        let mut models: HashMap<&'static str, Option<Model>> = HashMap::new();
        let mut props = Vec::new();
        for p in &parts.props {
            let model = models.entry(p.key).or_insert_with(|| prop_text(p.key).and_then(|t| Model::parse(t, None, &|_| None, None)));
            let Some(model) = model.as_ref() else { continue };
            // the model's feet at the spot, its middle on it, turned to face the room
            let pivot = vec3((model.min.x + model.max.x) * 0.5, model.min.y, (model.min.z + model.max.z) * 0.5) * model.scale;
            let at = p.pos - Mat3::from_rotation_y(p.yaw) * pivot;
            let (mut v, mut i) = (Vec::new(), Vec::new());
            model.build(&mut v, &mut i, at, p.yaw, 0.0, 0.0, Color::new(p.tint[0], p.tint[1], p.tint[2], 1.0));
            if !v.is_empty() {
                props.push(Mesh { vertices: v, indices: i, texture: Some(model.texture.clone()) });
            }
        }
        let mut hub = HubWorld { world, props, skin: skin_idx };
        hub.show_box(None);
        hub
    }

    /// The music box sits in a room (None hides it).
    pub fn show_box(&mut self, at: Option<Vec3>) {
        match at {
            Some(p) => {
                self.world.set_offset("MusicBox", p);
                self.world.set_visible("MusicBox", true);
            }
            None => self.world.set_visible("MusicBox", false),
        }
    }

    /// Draw the hub and its props with `lights`.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        if !self.props.is_empty() {
            gfx.shader.begin(lights, cam_pos);
            for m in &self.props {
                draw_mesh(m);
            }
            gfx.shader.end();
        }
    }

    pub fn prop_count(&self) -> usize {
        self.props.len()
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::nav::Pace;

    #[test]
    fn the_map_parses_to_the_godot_layout() {
        let m = map();
        assert_eq!(m.areas.len(), 8);
        assert_eq!(m.walls.len(), 22);
        assert_eq!(m.signs.len(), 6);
        assert_eq!(m.layout_rooms.len(), 9);
        assert_eq!(m.layout_edges.len(), 10);
        let st = m.area("stage").unwrap();
        assert_eq!(st.rect, [-9.5, 9.5, -46.0, -30.0]);
        assert_eq!(st.centre(), vec3(0.0, 0.0, -38.0));
        assert_eq!(st.map, "platform");
        assert_eq!(m.vent_origin, vec3(-8.0, 0.0, -62.0));
        assert_eq!(map_for("stage"), "platform");
        assert_eq!(map_for("vent"), "duct");
        assert_eq!(sim_for("lost_property"), "closet");
        assert_eq!(sim_for("nowhere"), "nowhere");
    }

    #[test]
    fn every_map_room_is_a_camera_of_the_skin_list() {
        let m = map();
        for a in &m.areas {
            assert!(CAMS.contains(&a.map.as_str()), "{}", a.map);
            assert_eq!(CAMS[a.cam - 1], a.map, "camera number of {}", a.id);
        }
        assert_eq!(CAMS[8], "duct");
    }

    #[test]
    fn cameras_sit_high_in_a_corner_of_their_room_and_look_into_it() {
        for a in &map().areas {
            let (pos, target) = camera_transform(&a.id);
            assert!(a.contains(pos.x, pos.z), "{} camera is inside the room", a.id);
            assert_eq!(pos.y, 3.0);
            assert!(a.contains(target.x, target.z), "{} camera looks into the room", a.id);
            assert!(target.y < pos.y);
            let c = a.centre();
            assert!((pos.x - c.x).abs() > 0.2 * a.size().0 - 0.01, "a corner");
        }
        let (p, t) = camera_transform("vent");
        assert!((p.x - -4.4).abs() < 1e-4 && p.y == 1.7 && p.z == -62.0);
        assert!(t.x < p.x, "the duct camera looks down the run");
    }

    #[test]
    fn stand_positions_follow_the_godot_slots() {
        assert_eq!(stand_position("stage", 0), vec3(1.0, 0.0, -37.4));
        assert_eq!(stand_position("stage", 1), vec3(-1.0, 0.0, -38.6));
        assert_eq!(vent_spot_pos(0), vec3(-10.5, 0.0, -62.0));
        assert_eq!(vent_spot_pos(4), vec3(-6.5, 0.0, -62.0));
        assert_eq!(vent_stand(Side::T), vec3(0.0, 0.0, -1.3));
    }

    #[test]
    fn dressing_spots_are_inside_their_rooms_and_every_room_has_some() {
        for a in &map().areas {
            assert!(a.spots.len() >= 2, "{}", a.id);
            for (x, z) in &a.spots {
                assert!(a.contains(*x, *z), "{} spot {x},{z}", a.id);
            }
        }
    }

    #[test]
    fn each_skin_builds_a_full_hub_with_props_in_every_room() {
        for i in 0..9 {
            let parts = hub_parts(i);
            let names: Vec<&str> = parts.groups.iter().map(|g| g.name.as_str()).collect();
            for want in ["HubShell", "HubFit", "HubDress", "VentRun", "MusicBox"] {
                assert!(names.contains(&want), "skin {i} lacks {want}");
            }
            let cubes: usize = parts.groups.iter().map(|g| g.cubes.len()).sum();
            assert!(cubes > 200, "skin {i} has only {cubes} cubes");
            // every room has something standing in it
            for a in &map().areas {
                let boxed = parts.groups.iter().find(|g| g.name == "HubDress").unwrap().cubes.iter().any(|c| a.contains(c.pos.x, c.pos.z));
                let modelled = parts.props.iter().any(|p| a.contains(p.pos.x, p.pos.z));
                assert!(boxed || modelled, "skin {i}: nothing in {}", a.id);
            }
            for p in &parts.props {
                assert!(prop_text(p.key).is_some(), "{}", p.key);
            }
        }
    }

    #[test]
    fn the_pizzeria_uses_prop_models_where_they_fit() {
        let parts = hub_parts(1);
        assert!(parts.props.iter().any(|p| p.key == "restaurantPartyTable"));
        assert!(parts.props.iter().any(|p| p.key == "arcadeCabinet"));
        let camp = hub_parts(8);
        assert!(camp.props.is_empty(), "the campsite is built from boxes");
    }

    #[test]
    fn walls_have_gaps_with_frames_and_the_halls_open_into_the_arcades() {
        let m = map();
        let parts = hub_parts(0);
        let shell = parts.groups.iter().find(|g| g.name == "HubShell").unwrap();
        let frames = shell.cubes.iter().filter(|c| c.name == "DoorFrameHub").count();
        let gaps: usize = m.walls.iter().map(|w| w.gaps.len()).sum();
        assert_eq!(frames, gaps * 3);
        // the west and east inner walls leave the hall mouth open
        assert!(m.walls.iter().any(|w| !w.along_x && w.fixed == -9.5 && w.gaps.contains(&(-1.7, 1.7))));
        assert!(m.walls.iter().any(|w| !w.along_x && w.fixed == 9.5 && w.gaps.contains(&(-1.7, 1.7))));
    }

    #[test]
    fn office_extras_have_a_shutter_and_a_flash_per_vent() {
        let g = office_extras();
        for side in Side::ALL {
            assert!(g.iter().any(|x| x.name == format!("VentShutter{}", side.key())));
            assert!(g.iter().any(|x| x.name == format!("VentFlash{}", side.key())));
        }
        assert!(g.iter().any(|x| x.name == "VentHatchR") && g.iter().any(|x| x.name == "VentCeiling"));
        let flash = g.iter().find(|x| x.name == "VentFlashL").unwrap();
        assert!(flash.cubes.iter().all(|c| c.emissive), "the glow is emissive by its name");
        assert_eq!(vent_station(Side::T).id, "ventT");
        assert!(vent_station(Side::L).prompt.contains("HOLD TO SEAL"));
    }

    #[test]
    fn the_minimap_layout_carries_the_skins_labels() {
        let l = layout_for(1);
        assert_eq!(l.rooms.len(), 9);
        assert_eq!(l.cams.len(), 9);
        assert!(l.labels.iter().any(|(r, t)| r == "platform" && t == "SHOW STAGE"));
        assert_eq!(l.vents, vec!["duct".to_string()]);
        assert!(l.office.is_some());
    }

    #[test]
    fn a_broken_nav_graph_is_an_error_never_an_empty_map() {
        assert!(try_nav().is_ok());
        assert!(parse_nav("{ not json").is_err());
        // a link to a node that does not exist
        let bad = r#"{"id":"x","rooms":{"a":{"min":[0,0],"max":[1,1]}},"nodes":[{"id":"A","kind":"waypoint","room":"a","p":[0,0,0]}],"edges":[["A","GHOST"]]}"#;
        let e = parse_nav(bad).unwrap_err();
        assert!(e.contains("GHOST"), "{e}");
        // the real graph has the rooms the mode needs
        let real = nav();
        for r in ["office", "doorL", "doorR", "vent", "ventL", "ventR", "ventT"] {
            assert!(real.rooms().iter().any(|x| x.id == r), "{r}");
        }
    }

    #[test]
    fn the_vent_run_reaches_the_office_vents_by_a_hidden_walked_shaft_not_a_jump() {
        let v: Value = serde_json::from_str(NAV_JSON).unwrap();
        for e in v["edges"].as_array().unwrap() {
            assert_eq!(e.as_array().unwrap().len(), 2, "no jump edges remain: {e}");
        }
        let nav = nav();
        for (from, to, at_least) in [("V:3", "VS:L", 55.0), ("V:4", "VS:R", 55.0), ("C:vent", "VS:T", 50.0)] {
            let legs = nav.route(from, to);
            assert!(!legs.is_empty(), "{from} to {to}");
            assert!(legs.iter().all(|l| !l.jump), "{from} to {to} has a jump leg");
            let len = nav.distance(from, to).unwrap();
            assert!(len > at_least, "{from} to {to} is {len} m");
            // the shaft is a duct room: no camera room holds a crawler there
            assert!(legs.iter().any(|l| nav.node(&l.key).map_or(false, |n| n.room.starts_with("airduct"))));
        }
        // the island leaves the building alone: no walkway joins the vent nodes to the hall nodes
        assert!(nav.route("V:0", "C:dining").is_empty());
    }

    #[test]
    fn the_office_has_a_node_on_each_side_linked_to_that_door_only() {
        let nav = nav();
        for (o, d) in [("O:officeL", "D:doorL"), ("O:officeR", "D:doorR")] {
            assert_eq!(nav.room_at(nav.pos(o).unwrap()), Some("office"));
            let r = nav.route(o, d);
            assert_eq!(r.len(), 2, "{o} to {d}");
            assert!(r.iter().all(|l| !l.jump));
        }
        let via = nav.route("O:officeL", "O:officeR");
        assert!(via.len() > 8, "the office is not a shortcut between the doors: {:?}", via.iter().map(|l| l.key.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn the_nav_graph_is_valid_and_every_camera_room_is_reachable() {
        let nav = nav();
        assert_eq!(nav.validate(), Vec::<String>::new());
        let rooms = ["stage", "dining", "backstage", "westhall", "closet", "kitchen", "easthall", "corner"];
        for a in rooms {
            for b in rooms {
                let legs = nav.route_rooms(a, b);
                assert!(!legs.is_empty() || a == b, "no walk from {a} to {b}");
            }
            assert!(nav.route_rooms(a, "doorL").len() > 0, "{a} to the left door");
            assert!(nav.route_rooms(a, "doorR").len() > 0, "{a} to the right door");
            assert!(!nav.cameras(a).is_empty(), "{a} has a camera node");
        }
        assert!(!nav.cameras("vent").is_empty());
        // the vent crawler's run reaches the three office vents over the jump edges
        for key in ["VS:L", "VS:R", "VS:T"] {
            assert!(!nav.route("V:0", key).is_empty(), "{key}");
        }
    }

    #[test]
    fn nav_camera_nodes_sit_where_the_map_puts_the_cameras() {
        let nav = nav();
        for a in &map().areas {
            let cam = nav.cameras(&a.id);
            assert_eq!(cam.len(), 1, "{}", a.id);
            assert!(cam[0].p.distance(a.cam_pos) < 0.01, "{}", a.id);
            assert_eq!(cam[0].cam as usize, a.cam);
        }
        let v = nav.cameras("vent")[0];
        assert_eq!(v.cam, 9);
        assert!(v.p.distance(camera_transform("vent").0) < 0.01);
    }

    #[test]
    fn nav_stands_match_the_godot_stand_positions() {
        let nav = nav();
        for a in &map().areas {
            for slot in 0..2 {
                let n = nav.stand_node(&a.id, slot).unwrap();
                assert!(n.p.distance(stand_position(&a.id, slot)) < 0.01, "{} slot {slot}", a.id);
                assert!(n.look.is_some());
            }
        }
    }

    #[test]
    fn the_rooms_contain_their_centres() {
        let nav = nav();
        for a in &map().areas {
            assert_eq!(nav.room_at(a.centre()), Some(a.id.as_str()), "{}", a.id);
        }
        assert_eq!(nav.room_at(vent_spot_pos(2)), Some("vent"));
        assert_eq!(nav.room_at(vec3(-6.0, 0.0, 0.0)), Some("doorL"));
        assert_eq!(nav.room_at(vec3(6.0, 0.0, 0.0)), Some("doorR"));
    }

    #[test]
    fn every_room_has_task_spots_for_the_idle_behaviour() {
        let nav = nav();
        for a in &map().areas {
            let spots = nav.task_spots(&a.id, None);
            assert!(spots.len() >= 4, "{} has {} task spots", a.id, spots.len());
            assert!(spots.iter().any(|n| n.task.as_deref() == Some("pace") && n.pace_to.is_some()), "{} paces", a.id);
            for s in spots {
                assert!(a.contains(s.p.x, s.p.z), "{} task spot {} is inside", a.id, s.key);
            }
        }
    }

    #[test]
    fn a_walk_from_the_stage_to_the_left_door_goes_through_the_west_arcade() {
        let nav = nav();
        let legs = nav.route_rooms("stage", "doorL");
        let keys: Vec<&str> = legs.iter().map(|l| l.key.as_str()).collect();
        assert!(keys.contains(&"C:backstage") || keys.contains(&"DW:stage|dining"), "{keys:?}");
        assert!(keys.contains(&"C:westhall"), "{keys:?}");
        assert_eq!(*keys.last().unwrap(), "D:doorL");
        let _ = Pace::Walk;
    }

    #[test]
    fn the_lights_are_the_skins() {
        let office = LightSet::office_default();
        let l = world_lights(office, 2);
        assert_eq!(l.ambient, skin::skin(2).light);
        let fl = feed_lights(0, "stage");
        assert_eq!(fl.omni.len(), 1);
        assert!(fl.ambient.iter().all(|c| *c <= 1.0));
        assert_eq!(feed_lights(0, "vent").omni.len(), 1);
    }
}
