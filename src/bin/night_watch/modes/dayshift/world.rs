//! Salvage Run's 3D map (`cargo_bay.gd`, `dayshift_yard.gd`, `dayshift_map.gd`): the cargo bay (the Cargo variant of the
//! shared security room, with a window in its front wall, a workbench with the four-slot rack and the budget ledger), and
//! the ship and scrap yard around it at the Godot coordinates. West of the bay: the gantry, the dock, the airlock and, off
//! the dock, the engine room and the bridge. East: the gantry and the cargo hold. South: the open scrap yard with the crane,
//! which the bay's window looks onto. The halls of the bay carry on as the two gantries.
//!
//! Contract 9.1 additions on top of the Godot map: a security camera in every room (`data/dayshift/map.json`, the same
//! spots as the camera nodes of `data/nav/dayshift.json`) and props in every room (bbmodels from `last_route/assets/props`,
//! placed from the map data, plus the Godot boxes).
//!
//! Geometry is built by pure functions (`ship_parts`, `bay_parts`, tested without a window); `Scene::build` bakes them and
//! loads the prop models (needs the GL context).
#![allow(dead_code)]

use crate::core::CamLayout;
use crate::gfx::office::{collect_solids, mul3, put, Office, OfficeConfig, OfficeParts, OfficeState, OfficeVariant, StationDef, O, SEAT, SPAWN};
use crate::gfx::office::{HALF_D, HALF_W, HEIGHT};
use crate::gfx::{cube_aabb, Cube, Gfx, Group, LightSet, Omni, Textures, World, WorldBuilder};
use crate::hunters::bbmodel::Model;
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const MAP_JSON: &str = include_str!("../../data/dayshift/map.json");

/// A day shift: the yard window shows daylight, not a night sky (Godot `Color(0.55, 0.74, 0.95)`).
pub const SKY: Color = Color::new(0.55, 0.74, 0.95, 1.0);
/// Ambient light of the whole map (Godot: colour (0.62, 0.6, 0.58) at energy 0.9, lifted a little for the textures).
pub const DAY_AMBIENT: [f32; 3] = [0.70, 0.68, 0.66];
/// The window of the bay (x of its centre) and the rack slots' spots (CargoBay constants).
pub const WINDOW_X: f32 = -1.5;

const H: f32 = 3.6;
const THICK: f32 = 0.2;
const STEEL: [f32; 3] = [0.8, 0.85, 1.0];
const RUST: [f32; 3] = [1.6, 0.9, 0.6];
const WOOD: [f32; 3] = [1.5, 1.05, 0.7];
const WOOD_DARK: [f32; 3] = [1.2, 0.8, 0.55];
const CONTAINERS: [[f32; 3]; 5] = [[1.7, 0.5, 0.4], [0.5, 1.0, 1.7], [1.8, 1.5, 0.4], [0.6, 1.6, 0.7], [1.5, 0.7, 1.6]];
const HULL: [f32; 3] = [1.35, 1.5, 1.9];
const HULL_RUST: [f32; 3] = [1.8, 1.15, 0.8];
const HULL_TEAL: [f32; 3] = [0.9, 1.6, 1.8];
const WARM: [f32; 3] = [1.8, 1.45, 1.05];

// ---- the map data -----------------------------------------------------------------------------------------------------------

/// A security camera: where it hangs, what it looks at, how wide.
#[derive(Clone, Debug)]
pub struct CamDef {
    pub room: String,
    pub label: String,
    pub pos: Vec3,
    pub target: Vec3,
    pub fov: f32,
}

impl CamDef {
    /// Yaw (radians, `FpCamera` convention: 0 looks down -Z) and pitch of the view.
    pub fn yaw_pitch(&self) -> (f32, f32) {
        let d = self.target - self.pos;
        let yaw = (-d.x).atan2(-d.z);
        let pitch = d.y.atan2((d.x * d.x + d.z * d.z).sqrt());
        (yaw, pitch)
    }
}

/// A bbmodel prop placed in the map.
#[derive(Clone, Debug)]
pub struct PropDef {
    pub model: String,
    pub pos: Vec3,
    pub yaw_deg: f32,
    /// Multiplies the model's native size (1 when the file gives none).
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub rooms: Vec<(String, Vec2)>,
    pub edges: Vec<(String, String)>,
    pub entrances: Vec<String>,
    pub origins: Vec<(String, String)>,
    pub window_stand: Vec3,
    pub slots: Vec<Vec3>,
    pub cameras: Vec<CamDef>,
    pub props: Vec<PropDef>,
}

fn vec3_of(v: &Value) -> Vec3 {
    let a: Vec<f32> = v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
    vec3(a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0), a.get(2).copied().unwrap_or(0.0))
}

impl MapData {
    pub fn parse(src: &str) -> Result<MapData, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("dayshift map: {e}"))?;
        let mut m = MapData::default();
        if let Some(o) = v.get("rooms").and_then(|r| r.as_object()) {
            for (k, p) in o {
                let a = vec3_of(&serde_json::json!([p[0], 0.0, p[1]]));
                m.rooms.push((k.clone(), vec2(a.x, a.z)));
            }
        }
        if let Some(a) = v.get("edges").and_then(|e| e.as_array()) {
            for e in a {
                m.edges.push((e[0].as_str().unwrap_or("").to_string(), e[1].as_str().unwrap_or("").to_string()));
            }
        }
        m.entrances = crate::core::jstrs(&v, "entrances");
        if let Some(o) = v.get("origins").and_then(|r| r.as_object()) {
            for (k, x) in o {
                m.origins.push((k.clone(), x.as_str().unwrap_or("").to_string()));
            }
        }
        m.window_stand = vec3_of(&v["window_stand"]);
        if let Some(a) = v.get("slots").and_then(|s| s.as_array()) {
            m.slots = a.iter().map(vec3_of).collect();
        }
        if let Some(a) = v.get("cameras").and_then(|s| s.as_array()) {
            for c in a {
                m.cameras.push(CamDef {
                    room: crate::core::jstr(c, "room"),
                    label: crate::core::jstr(c, "label"),
                    pos: vec3_of(&c["pos"]),
                    target: vec3_of(&c["target"]),
                    fov: crate::core::jf64(c, "fov", 62.0) as f32,
                });
            }
        }
        if let Some(a) = v.get("props").and_then(|s| s.as_array()) {
            for p in a {
                m.props.push(PropDef { model: crate::core::jstr(p, "model"), pos: vec3_of(&p["pos"]), yaw_deg: crate::core::jf64(p, "yaw", 0.0) as f32, scale: (crate::core::jf64(p, "scale", 1.0) as f32).max(0.05) });
            }
        }
        if m.cameras.len() != 8 {
            return Err(format!("dayshift map: {} cameras, want 8", m.cameras.len()));
        }
        if m.slots.len() != 4 {
            return Err(format!("dayshift map: {} rack slots, want 4", m.slots.len()));
        }
        Ok(m)
    }

    pub fn camera(&self, room: &str) -> Option<&CamDef> {
        self.cameras.iter().find(|c| c.room == room)
    }

    /// Camera number (1..=8) of a room, 0 for none.
    pub fn camera_number(&self, room: &str) -> usize {
        self.cameras.iter().position(|c| c.room == room).map(|i| i + 1).unwrap_or(0)
    }

    pub fn room_ids(&self) -> Vec<String> {
        self.cameras.iter().map(|c| c.room.clone()).collect()
    }
}

/// The parsed map data (empty if the embedded file is broken: `data_problems` and `--check` say so).
pub fn map() -> &'static MapData {
    static MAP: OnceLock<MapData> = OnceLock::new();
    MAP.get_or_init(|| MapData::parse(MAP_JSON).unwrap_or_default())
}

pub fn data_problems() -> Vec<String> {
    match MapData::parse(MAP_JSON) {
        Ok(m) => {
            let mut p = Vec::new();
            for pr in &m.props {
                if prop_text(&pr.model).is_none() {
                    p.push(format!("prop model {} is not embedded", pr.model));
                }
            }
            p
        }
        Err(e) => vec![e],
    }
}

/// The camera screen's layout: the nine places of `dayshift_map.gd` (the crane has no camera of its own), the walkable edges,
/// the three entrances and the bay at the origin.
pub fn cam_layout() -> CamLayout {
    let m = map();
    CamLayout {
        rooms: m.rooms.clone(),
        edges: m.edges.clone(),
        entrances: m.entrances.clone(),
        cams: m.room_ids(),
        labels: m.cameras.iter().map(|c| (c.room.clone(), c.label.clone())).collect(),
        vents: Vec::new(),
        office: Some(vec2(0.0, 0.0)),
    }
}

// ---- prop models -----------------------------------------------------------------------------------------------------------

macro_rules! prop {
    ($name:literal) => {
        ($name, include_str!(concat!("../../../last_route/assets/props/", $name, ".bbmodel")))
    };
}

/// The embedded bbmodels this map uses.
const PROP_TEXTS: &[(&str, &str)] = &[
    prop!("locker"),
    prop!("supplyCrates"),
    prop!("utilityCart"),
    prop!("gasCylinderRack"),
    prop!("hazardSign"),
    prop!("maintenanceToolCabinet"),
    prop!("storageBarrel"),
    prop!("rackGear"),
    prop!("cableCoil"),
    prop!("serverRack"),
    prop!("monitorWall"),
    prop!("storageShelving"),
    prop!("labBench"),
    prop!("trashBin"),
    prop!("controlConsole"),
    prop!("cableRun"),
    prop!("irrigationManifold"),
    prop!("floorLamp"),
    prop!("labCoolingUnit"),
    prop!("maintenanceCleaningCart"),
    prop!("officeCrtSecurityConsole"),
];

pub fn prop_text(name: &str) -> Option<&'static str> {
    PROP_TEXTS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// The placed bbmodel props as meshes in world space (built once; the lit shader draws them).
pub struct PropSet {
    meshes: Vec<Mesh>,
}

impl PropSet {
    /// Needs the GL context (the models' textures).
    pub fn build(defs: &[PropDef]) -> PropSet {
        let mut models: HashMap<String, Option<Model>> = HashMap::new();
        let mut meshes = Vec::new();
        for d in defs {
            let entry = models.entry(d.model.clone()).or_insert_with(|| prop_text(&d.model).and_then(|t| Model::parse(t, None, &|_| None, None)));
            let Some(model) = entry.as_ref() else { continue };
            let (mut verts, mut idx) = (Vec::new(), Vec::new());
            model.build_scaled(&mut verts, &mut idx, d.pos, d.yaw_deg.to_radians(), Vec3::splat(model.scale * d.scale), 0.0, 0.0, WHITE);
            if verts.is_empty() {
                continue;
            }
            meshes.push(Mesh { vertices: verts, indices: idx, texture: Some(model.texture.clone()) });
        }
        PropSet { meshes }
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        if self.meshes.is_empty() {
            return;
        }
        gfx.shader.begin(lights, cam_pos);
        for m in &self.meshes {
            draw_mesh(m);
        }
        gfx.shader.end();
    }
}

// ---- the ship (dayshift_yard.gd) -----------------------------------------------------------------------------------------------

/// A wall piece on the floor plan, for the path checks.
#[derive(Clone, Debug)]
pub struct WallBox {
    pub name: String,
    pub min: (f32, f32),
    pub max: (f32, f32),
}

pub struct ShipParts {
    pub groups: Vec<Group>,
    pub walls: Vec<WallBox>,
}

fn wall(g: &mut Group, walls: &mut Vec<WallBox>, along_x: bool, fixed: f32, a: f32, b: f32, gaps: &[(f32, f32)], tint: [f32; 3], name: &str, height: f32) {
    let mut cuts = vec![a];
    for (s, e) in gaps {
        cuts.push(*s);
        cuts.push(*e);
    }
    cuts.push(b);
    let mut i = 0;
    while i + 1 < cuts.len() {
        let (s, e) = (cuts[i], cuts[i + 1]);
        i += 2;
        if e - s < 0.05 {
            continue;
        }
        let length = e - s;
        let mid = (s + e) * 0.5;
        if along_x {
            put(g, name, [length, height, THICK], [mid, height * 0.5, fixed], O::new().t(tint));
            walls.push(WallBox { name: name.to_string(), min: (s, fixed - THICK * 0.5), max: (e, fixed + THICK * 0.5) });
        } else {
            put(g, name, [THICK, height, length], [fixed, height * 0.5, mid], O::new().t(tint));
            walls.push(WallBox { name: name.to_string(), min: (fixed - THICK * 0.5, s), max: (fixed + THICK * 0.5, e) });
        }
    }
}

fn slab(g: &mut Group, x0: f32, x1: f32, z0: f32, z1: f32, tint: [f32; 3], ceiling: bool, name: &str, kind: &'static str) {
    put(g, name, [x1 - x0, 0.2, z1 - z0], [(x0 + x1) * 0.5, -0.1, (z0 + z1) * 0.5], O::new().k(kind).t(tint));
    if ceiling {
        put(g, "CeilingShip", [x1 - x0, 0.2, z1 - z0], [(x0 + x1) * 0.5, H + 0.1, (z0 + z1) * 0.5], O::new().k("concrete").t([1.1, 1.1, 1.25]));
        // ceiling lamps: glowing panels flush with the ceiling, about one per 6 m x 6 m (they read in every feed)
        let nx = (((x1 - x0) / 6.0).round() as i32).max(1);
        let nz = (((z1 - z0) / 6.0).round() as i32).max(1);
        for i in 0..nx {
            for j in 0..nz {
                let cx = x0 + (x1 - x0) * (i as f32 + 0.5) / nx as f32;
                let cz = z0 + (z1 - z0) * (j as f32 + 0.5) / nz as f32;
                put(g, "CeilingPanelLamp", [1.1, 0.06, 0.45], [cx, H - 0.03, cz], O::new().k("light"));
            }
        }
    }
}

fn container(g: &mut Group, o: [f32; 3], tint: [f32; 3], size: [f32; 3]) {
    put(g, "BodyContainer", size, [o[0], o[1] + size[1] * 0.5, o[2]], O::new().k("metal").t(tint));
    for i in 0..4 {
        put(
            g,
            "RibContainer",
            [0.06, size[1], size[2] + 0.04],
            [o[0] - size[0] * 0.4 + i as f32 * size[0] * 0.27, o[1] + size[1] * 0.5, o[2]],
            O::new().k("metal").t(mul3(tint, [0.8, 0.8, 0.8])),
        );
    }
    put(g, "LabelContainer", [0.5, 0.2, 0.04], [o[0] + size[0] * 0.3, o[1] + size[1] * 0.8, o[2] + size[2] * 0.5 + 0.03], O::new().k("label").t([1.5, 1.4, 0.7]));
}

/// The ship and the yard as groups (pure geometry).
pub fn ship_parts() -> ShipParts {
    let mut groups: Vec<Group> = Vec::new();
    let mut walls: Vec<WallBox> = Vec::new();

    // gantries: the bay's halls carried on (their end walls are dropped by the bay)
    for (tag, side) in [("L", -1.0f32), ("R", 1.0f32)] {
        let mut g = Group::new(&format!("ShipGantry{tag}"), Vec3::ZERO);
        let x0 = (side * 9.3).min(side * 14.5);
        let x1 = (side * 9.3).max(side * 14.5);
        slab(&mut g, x0, x1, -1.7, 1.7, [1.3, 1.4, 1.7], true, "FloorShip", "metal");
        wall(&mut g, &mut walls, true, -1.7, x0, x1, &[], HULL, "WallShip", H);
        wall(&mut g, &mut walls, true, 1.7, x0, x1, &[], HULL, "WallShip", H);
        for i in 0..4 {
            put(&mut g, "GratingGantry", [0.1, 0.1, 3.2], [side * (10.0 + i as f32 * 1.2), 0.02, 0.0], O::new().k("metal").t([0.7, 0.7, 0.8]));
        }
        put(&mut g, "SignGantry", [0.5, 0.3, 0.1], [side * 12.0, 2.4, -1.6], O::new().k("label").t([1.5, 1.2, 0.5]));
        groups.push(g);
    }

    // the dock
    {
        let mut d = Group::new("ShipDock", Vec3::ZERO);
        slab(&mut d, -26.0, -14.5, -9.0, 9.0, [1.2, 1.3, 1.6], true, "FloorShip", "concrete");
        wall(&mut d, &mut walls, false, -26.0, -9.0, 9.0, &[(-2.0, 2.0)], HULL, "WallShip", H);
        wall(&mut d, &mut walls, false, -14.5, -9.0, 9.0, &[(-1.7, 1.7)], HULL, "WallShip", H);
        wall(&mut d, &mut walls, true, -9.0, -26.0, -14.5, &[(-22.0, -19.0)], HULL, "WallShip", H);
        wall(&mut d, &mut walls, true, 9.0, -26.0, -14.5, &[(-22.0, -19.0)], HULL, "WallShip", H);
        // containers stacked in the corners (the lanes along z = 0 and x = -20.5 stay clear), a forklift, a yellow lane line
        container(&mut d, [-23.5, 0.0, 6.0], CONTAINERS[0], [3.0, 1.4, 1.3]);
        container(&mut d, [-23.5, 1.4, 6.0], CONTAINERS[1], [3.0, 1.4, 1.3]);
        container(&mut d, [-17.0, 0.0, 6.2], CONTAINERS[2], [3.0, 1.4, 1.3]);
        container(&mut d, [-23.5, 0.0, -6.2], CONTAINERS[3], [3.0, 1.4, 1.3]);
        container(&mut d, [-16.5, 0.0, -6.0], CONTAINERS[4], [2.4, 1.4, 1.3]);
        put(&mut d, "BodyForklift", [1.2, 0.9, 1.8], [-17.5, 0.55, 3.0], O::new().k("metal").t([1.8, 1.5, 0.3]));
        put(&mut d, "MastForklift", [0.9, 1.0, 0.1], [-17.5, 1.0, 2.1], O::new().k("metal"));
        put(&mut d, "ForkForklift", [0.9, 0.08, 0.8], [-17.5, 0.3, 1.7], O::new().k("metal").t([0.7, 0.7, 0.8]));
        put(&mut d, "LaneLine", [10.0, 0.012, 0.15], [-20.2, 0.007, 0.0], O::new().k("label").t([1.7, 1.4, 0.4]));
        put(&mut d, "SignDock", [0.5, 0.2, 0.06], [-25.8, 2.4, 0.0], O::new().k("label").t([1.5, 1.2, 0.5]));
        groups.push(d);
    }

    // the airlock
    {
        let mut a = Group::new("ShipAirlock", Vec3::ZERO);
        slab(&mut a, -32.0, -26.0, -3.0, 3.0, [1.5, 1.2, 0.9], true, "FloorShip", "metal");
        wall(&mut a, &mut walls, false, -32.0, -3.0, 3.0, &[], HULL_RUST, "WallShip", H);
        wall(&mut a, &mut walls, true, -3.0, -32.0, -26.0, &[], HULL_RUST, "WallShip", H);
        wall(&mut a, &mut walls, true, 3.0, -32.0, -26.0, &[], HULL_RUST, "WallShip", H);
        // a heavy ring door frame at the dock side, a wheel, warning lights (glowing cubes), a lever
        for sz in [-1.9f32, 1.9] {
            put(&mut a, "FrameAirlock", [0.4, 2.8, 0.3], [-26.2, 1.4, sz], O::new().k("metal").t([1.6, 0.7, 0.4]));
        }
        put(&mut a, "FrameAirlock", [0.4, 0.3, 4.1], [-26.2, 2.9, 0.0], O::new().k("metal").t([1.6, 0.7, 0.4]));
        put(&mut a, "WheelAirlock", [0.2, 0.9, 0.9], [-31.8, 1.5, 0.0], O::new().k("metal").t([0.7, 0.7, 0.8]));
        for sz in [-2.4f32, 2.4] {
            put(&mut a, "GlowLampAirlock", [0.2, 0.2, 0.2], [-26.4, 2.8, sz], O::new());
        }
        put(&mut a, "LeverAirlock", [0.1, 0.6, 0.1], [-29.0, 0.3, 2.4], O::new().k("metal").t([1.7, 0.5, 0.4]));
        groups.push(a);
    }

    // the engine room
    {
        let mut e = Group::new("ShipEngine", Vec3::ZERO);
        slab(&mut e, -26.0, -14.5, -22.0, -9.0, [1.5, 1.2, 1.0], true, "FloorShip", "metal");
        wall(&mut e, &mut walls, false, -26.0, -22.0, -9.0, &[], HULL_RUST, "WallShip", H);
        wall(&mut e, &mut walls, false, -14.5, -22.0, -9.0, &[(-17.0, -14.0)], HULL_RUST, "WallShip", H);
        wall(&mut e, &mut walls, true, -22.0, -26.0, -14.5, &[], HULL_RUST, "WallShip", H);
        put(&mut e, "BlockEngine", [5.0, 2.2, 3.0], [-21.0, 1.1, -18.0], O::new().k("metal").t([1.2, 0.7, 0.5]));
        for i in 0..4 {
            put(&mut e, "PistonEngine", [0.5, 0.5, 3.2], [-23.0 + i as f32 * 1.4, 2.45, -18.0], O::new().k("metal").t(STEEL));
        }
        for i in 0..3 {
            let z = -12.0 - i as f32 * 2.2;
            put(&mut e, "PipeEngine", [0.25, 3.0, 0.25], [-24.5, 1.5, z], O::new().k("metal").t([1.6, 0.8, 0.4]));
            put(&mut e, "GlowGaugeEngine", [0.2, 0.2, 0.06], [-24.4, 1.8, z + 0.2], O::new());
        }
        put(&mut e, "CrateEngine", [1.2, 1.0, 1.0], [-16.5, 0.5, -12.0], O::new().k("wood"));
        groups.push(e);
    }

    // the bridge
    {
        let mut b = Group::new("ShipBridge", Vec3::ZERO);
        slab(&mut b, -14.5, 2.0, -22.0, -10.0, [0.9, 1.4, 1.8], true, "FloorShip", "floor");
        wall(&mut b, &mut walls, true, -22.0, -14.5, 2.0, &[], HULL_TEAL, "WallShip", H);
        wall(&mut b, &mut walls, true, -10.0, -14.5, 2.0, &[], HULL_TEAL, "WallShip", H);
        wall(&mut b, &mut walls, false, 2.0, -22.0, -10.0, &[], HULL_TEAL, "WallShip", H);
        put(&mut b, "ConsoleBridge", [7.0, 1.0, 1.0], [-6.0, 0.5, -20.6], O::new().k("metal").t([0.7, 0.8, 1.1]));
        for i in 0..5 {
            put(&mut b, "ScreenBridge", [0.7, 0.4, 0.04], [-8.5 + i as f32 * 1.3, 1.25, -20.9], O::new().emissive());
        }
        put(&mut b, "ChairBridge", [0.8, 0.9, 0.8], [-6.0, 0.45, -17.5], O::new().k("fabric").t([1.7, 0.5, 0.5]));
        put(&mut b, "TableMapBridge", [2.0, 0.08, 2.0], [-1.0, 1.0, -16.0], O::new().k("metal").t([0.7, 0.8, 1.1]));
        for (lx, lz) in [(-1.8f32, -16.8f32), (-0.2, -16.8), (-1.8, -15.2), (-0.2, -15.2)] {
            put(&mut b, "LegMapBridge", [0.08, 0.96, 0.08], [lx, 0.48, lz], O::new().k("metal"));
        }
        put(&mut b, "GlowMapBridge", [1.6, 0.02, 1.6], [-1.0, 1.06, -16.0], O::new());
        put(&mut b, "WindowBridge", [4.0, 1.4, 0.1], [-6.0, 2.0, -21.9], O::new().k("glass").t([0.6, 0.9, 1.2]));
        groups.push(b);
    }

    // the cargo hold
    {
        let mut h = Group::new("ShipHold", Vec3::ZERO);
        slab(&mut h, 14.5, 26.0, -9.0, 9.0, [1.5, 1.3, 1.0], true, "FloorShip", "wood");
        wall(&mut h, &mut walls, false, 26.0, -9.0, 9.0, &[], WARM, "WallShip", H);
        wall(&mut h, &mut walls, false, 14.5, -9.0, 9.0, &[(-1.7, 1.7)], WARM, "WallShip", H);
        wall(&mut h, &mut walls, true, -9.0, 14.5, 26.0, &[], WARM, "WallShip", H);
        wall(&mut h, &mut walls, true, 9.0, 14.5, 26.0, &[(18.0, 21.0)], WARM, "WallShip", H);
        for i in 0..3 {
            put(&mut h, "ShelfHold", [0.6, 0.05, 12.0], [25.4, 0.7 + i as f32 * 0.8, -2.0], O::new().k("metal"));
            for j in 0..5 {
                put(&mut h, "BoxHold", [0.45, 0.5, 0.9], [25.4, 1.0 + i as f32 * 0.8, -6.5 + j as f32 * 2.3], O::new().k("wood").t(CONTAINERS[(i + j) % 5]));
            }
        }
        for p in [[17.0f32, -6.5], [21.0, -6.5], [17.0, 6.0]] {
            put(&mut h, "PalletHold", [1.6, 0.18, 1.6], [p[0], 0.09, p[1]], O::new().k("wood"));
            put(&mut h, "StackHold", [1.4, 1.0, 1.4], [p[0], 0.68, p[1]], O::new().k("wood").t(RUST));
        }
        for p in [[23.0f32, 6.5], [24.0, 5.5], [24.0, 7.5]] {
            put(&mut h, "BarrelHold", [0.7, 0.95, 0.7], [p[0], 0.48, p[1]], O::new().k("metal").t([1.7, 0.6, 0.4]));
        }
        put(&mut h, "SignHold", [0.5, 0.2, 0.06], [14.7, 2.4, 0.0], O::new().k("label").t([1.5, 1.2, 0.5]));
        groups.push(h);
    }

    // the scrap yard
    {
        let mut y = Group::new("ShipYard", Vec3::ZERO);
        slab(&mut y, -26.0, 26.0, 9.0, 28.0, [1.25, 1.3, 1.2], false, "GroundYard", "concrete");
        slab(&mut y, -14.5, 14.5, 2.7, 9.0, [1.25, 1.3, 1.2], false, "GroundYard", "concrete");
        // a fence round the outside
        let fence = [1.4, 1.5, 1.8];
        wall(&mut y, &mut walls, true, 28.0, -26.0, 26.0, &[], fence, "FenceYard", 2.2);
        wall(&mut y, &mut walls, false, -26.0, 9.0, 28.0, &[], fence, "FenceYard", 2.2);
        wall(&mut y, &mut walls, false, 26.0, 9.0, 28.0, &[], fence, "FenceYard", 2.2);
        // scrap piles of rusty cubes (clear of the lanes down x -20.5, x -1.5, x 19.5 and along z 18), a wrecked hull plate, drums
        let piles = [[-14.0f32, 14.0], [-8.0, 24.0], [6.0, 25.0], [15.0, 12.0], [-22.0, 22.0], [22.0, 24.0]];
        for p in piles {
            for k in 0..6usize {
                let kf = k as f32;
                put(
                    &mut y,
                    "ScrapBlock",
                    [1.0 + (k % 3) as f32 * 0.4, 0.6 + (k % 2) as f32 * 0.4, 1.0 + (k % 2) as f32 * 0.5],
                    [p[0] - 0.9 + (k % 3) as f32 * 0.9, 0.35 + (k / 3) as f32 * 0.7, p[1] - 0.4 + (k % 2) as f32 * 0.8],
                    O::new().k("metal").t(mul3(RUST, [1.0 - 0.08 * kf, 1.0, 1.0])),
                );
            }
        }
        y.add(Cube::new("HullPlateYard", vec3(6.0, 0.4, 3.0), vec3(-12.0, 1.6, 20.0)).kind("metal").tint([0.9, 1.0, 1.3]).rot_y(0.4));
        // the crane: two legs, a boom, a cable and a hook (the crane's spot is where Ysolde waits)
        let yellow = [1.7, 1.4, 0.3];
        for sx in [-1.6f32, 1.6] {
            put(&mut y, "LegCrane", [0.3, 7.0, 0.3], [10.0 + sx, 3.5, 22.0], O::new().k("metal").t(yellow));
        }
        put(&mut y, "BeamCrane", [4.0, 0.3, 0.3], [10.0, 7.0, 22.0], O::new().k("metal").t(yellow));
        put(&mut y, "BoomCrane", [0.3, 0.3, 6.0], [10.0, 7.3, 19.5], O::new().k("metal").t(yellow));
        put(&mut y, "CableCrane", [0.05, 4.5, 0.05], [10.0, 4.7, 17.5], O::new().k("plastic"));
        put(&mut y, "HookCrane", [0.5, 0.4, 0.5], [10.0, 2.3, 17.5], O::new().k("metal").t([1.6, 0.6, 0.4]));
        put(&mut y, "GlowLampCrane", [0.4, 0.3, 0.1], [10.0, 7.7, 22.4], O::new());
        // the window's strip: a lamp post, a bench
        put(&mut y, "PostYard", [0.1, 2.6, 0.1], [1.8, 1.3, 5.0], O::new().k("metal"));
        put(&mut y, "GlowYardLamp", [0.5, 0.12, 0.3], [1.5, 2.6, 5.0], O::new());
        put(&mut y, "BenchSeatYard", [1.4, 0.08, 0.4], [3.4, 0.45, 5.4], O::new().k("wood").t(WOOD));
        for bx in [-0.6f32, 0.6] {
            put(&mut y, "BenchLegYard", [0.08, 0.42, 0.34], [3.4 + bx, 0.21, 5.4], O::new().k("metal"));
        }
        groups.push(y);
    }

    // the security cameras: a housing with a lit lens at each camera spot (a pole under the yard camera)
    {
        let mut c = Group::new("ShipCameras", Vec3::ZERO);
        for cam in &map().cameras {
            let p = cam.pos;
            // the housing and its lamp hang BEHIND the lens, so a feed never looks at its own camera (the lamp used to hang
            // 0.3 m in front of it: the green box in the middle of every feed)
            let dir = (cam.target - cam.pos).normalize_or_zero();
            let back = p - dir * 0.4;
            put(&mut c, "CamHousing", [0.34, 0.22, 0.42], [back.x, back.y, back.z], O::new().k("metal").t([0.45, 0.45, 0.5]));
            let led = p - dir * 0.15 + vec3(0.0, 0.14, 0.0);
            put(&mut c, "LedCamera", [0.08, 0.08, 0.08], [led.x, led.y, led.z], O::new());
            if p.y > 3.7 {
                put(&mut c, "CamPole", [0.1, p.y - 0.1, 0.1], [p.x, (p.y - 0.1) * 0.5, p.z + 0.5], O::new().k("metal"));
            } else {
                put(&mut c, "CamBracket", [0.12, 0.3, 0.12], [p.x, p.y + 0.25, p.z], O::new().k("metal"));
            }
        }
        groups.push(c);
    }

    ShipParts { groups, walls }
}

// ---- the cargo bay (cargo_bay.gd) --------------------------------------------------------------------------------------------

/// Seconds-free look of a rack piece: the material kind, the tint, whether it glows and how tall it is (a multiple of its box).
pub fn piece_style(kind: &str) -> (&'static str, [f32; 3], bool, f32) {
    match kind {
        "parts" => ("metal", [0.7, 1.0, 1.3], false, 1.5),
        "prize" => ("light", [1.6, 1.3, 0.6], true, 1.9),
        _ => ("metal", [0.9, 0.9, 1.0], false, 1.0),
    }
}

/// How many of the six bar segments show for a progress (0..1): none at zero.
pub fn bar_segments(progress: f32) -> usize {
    if progress <= 0.0 {
        0
    } else {
        ((progress.clamp(0.0, 1.0) * 6.0).ceil() as usize).clamp(1, 6)
    }
}

pub const BAR_SEGMENTS: usize = 6;

fn bay_extra_groups() -> Vec<Group> {
    let mut groups = Vec::new();
    let slots = &map().slots;

    // the workbench
    let mut bench = Group::new("Workbench", vec3(1.7, 0.0, 1.25));
    put(&mut bench, "BenchTop", [2.2, 0.08, 0.7], [0.0, 0.86, 0.0], O::new().k("wood").t(WOOD));
    for bx in [-1.0f32, 1.0] {
        put(&mut bench, "BenchLeg", [0.1, 0.86, 0.6], [bx, 0.43, 0.0], O::new().k("metal").solid());
    }
    put(&mut bench, "BenchShelf", [2.0, 0.05, 0.5], [0.0, 0.3, 0.0], O::new().k("wood").t(WOOD_DARK));
    groups.push(bench);

    // the four rack slots: a pad, a piece (hidden until one turns up) and a bar of six segments
    let mut rack = Group::new("Slots", Vec3::ZERO);
    for (i, p) in slots.iter().enumerate() {
        put(&mut rack, &format!("SlotPad{i}"), [0.36, 0.03, 0.4], [p.x, p.y - 0.07, p.z], O::new().k("plastic"));
        put(&mut rack, &format!("PieceSlot{i}"), [0.26, 0.2, 0.26], [p.x, p.y + 0.04, p.z], O::new().k("metal"));
        for k in 0..BAR_SEGMENTS {
            put(
                &mut rack,
                &format!("Bar{i}_{k}"),
                [0.05, 0.03, 0.03],
                [p.x - 0.15 + 0.025 + k as f32 * 0.05, p.y + 0.24, p.z + 0.18],
                O::new().k("light").emissive(),
            );
        }
    }
    groups.push(rack);

    // the budget ledger on the desk (the monitor shows the cameras)
    let mut ledger = Group::new("Ledger", vec3(0.62, 0.8, -1.5));
    put(&mut ledger, "LedgerBook", [0.36, 0.03, 0.46], [0.0, 0.02, 0.0], O::new().k("fabric").t([1.5, 0.6, 0.55]));
    put(&mut ledger, "LedgerPaper", [0.3, 0.02, 0.4], [0.0, 0.045, 0.0], O::new().k("label").t([1.2, 1.2, 1.1]).ao(1.0));
    groups.push(ledger);

    // a stack of crates by the right door, wall signs, the window frame
    let mut stack = Group::new("CrateStack", vec3(2.5, 0.0, -1.55));
    put(&mut stack, "CrateBig", [0.7, 0.5, 0.7], [0.0, 0.25, 0.0], O::new().k("wood").t(WOOD).solid());
    put(&mut stack, "CrateSmall", [0.5, 0.4, 0.5], [0.05, 0.7, 0.0], O::new().k("wood").t(WOOD_DARK));
    groups.push(stack);

    let mut signs = Group::new("BaySigns", Vec3::ZERO);
    let tints = [[1.7f32, 1.4, 0.4], [1.7, 0.5, 0.45], [0.6, 0.8, 1.7], [1.7, 1.4, 0.4]];
    for (i, t) in tints.iter().enumerate() {
        put(&mut signs, "SignColour", [0.36, 0.14, 0.04], [-1.5 + i as f32, 1.5, -HALF_D + 0.04], O::new().k("label").t(*t));
    }
    groups.push(signs);

    let mut win = Group::new("WindowFrame", vec3(WINDOW_X, 1.55, HALF_D));
    for sx in [-0.72f32, 0.72] {
        put(&mut win, "FrameWindow", [0.08, 1.4, 0.16], [sx, 0.0, 0.0], O::new().k("wood"));
    }
    for sy in [-0.7f32, 0.7] {
        put(&mut win, "FrameWindowH", [1.5, 0.08, 0.16], [0.0, sy, 0.0], O::new().k("wood"));
    }
    put(&mut win, "FrameWindowBar", [0.05, 1.4, 0.05], [0.0, 0.0, 0.0], O::new().k("wood"));
    put(&mut win, "SillRed", [1.6, 0.06, 0.2], [0.0, -0.74, -0.06], O::new().k("red"));
    groups.push(win);
    groups
}

fn near(a: Vec3, b: Vec3) -> bool {
    a.distance(b) < 1e-3
}

/// The bay's pure geometry: the shared Cargo office (palette and signature on, hall ends dropped) with the front wall cut for
/// the window, the rack stations and the ledger added, and the monitor left on (the cameras are on in this port).
pub fn bay_parts() -> OfficeParts {
    let cfg = OfficeConfig { variant: OfficeVariant::Cargo, extra: bay_extra_groups(), drop_hall_end_walls: true, palette: true, signature: true };
    let mut parts = Office::parts(&cfg);
    // cut the window into the front wall: the one full-width "WallFront" cube becomes a wall with a gap
    let mut old: Option<(Cube, Vec3)> = None;
    for g in parts.groups.iter_mut() {
        if let Some(i) = g.cubes.iter().position(|c| c.name == "WallFront") {
            let cube = g.cubes.remove(i);
            old = Some((cube, g.origin));
            break;
        }
    }
    if let Some((front, origin)) = old {
        let was = cube_aabb(&front, origin);
        parts.solids.retain(|a| !(near(a.min, was.min) && near(a.max, was.max)));
        let hw = HALF_W;
        let z = HALF_D + 0.1;
        let x0 = WINDOW_X - 0.7;
        let x1 = WINDOW_X + 0.7;
        let mut wall = Group::new("WallFrontWindow", Vec3::ZERO);
        let piece = |name: &str, size: [f32; 3], pos: [f32; 3]| {
            Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).kind(&front.kind).tint(front.tint).ao(front.ao).solid()
        };
        wall.add(piece("WallFrontA", [x0 + hw + 0.2, HEIGHT, 0.2], [(x0 - hw - 0.2) * 0.5, HEIGHT * 0.5, z]));
        wall.add(piece("WallFrontB", [hw + 0.2 - x1, HEIGHT, 0.2], [(x1 + hw + 0.2) * 0.5, HEIGHT * 0.5, z]));
        wall.add(piece("WallFrontSill", [x1 - x0, 0.9, 0.2], [WINDOW_X, 0.45, z]));
        wall.add(piece("WallFrontHead", [x1 - x0, HEIGHT - 2.2, 0.2], [WINDOW_X, 2.2 + (HEIGHT - 2.2) * 0.5, z]));
        parts.solids.extend(collect_solids(std::slice::from_ref(&wall)));
        parts.groups.push(wall);
    }
    // the rack: one station per slot (the prompt follows the piece on it), and the ledger
    for (i, p) in map().slots.iter().enumerate() {
        parts.stations.push(StationDef { id: format!("rack{i}"), prompt: String::new(), pos: *p, focus: Vec3::ZERO, radius: 0.25, disabled: true });
    }
    parts.stations.push(StationDef { id: "ledger".to_string(), prompt: "BUDGET LEDGER".to_string(), pos: vec3(0.62, 0.95, -1.5), focus: Vec3::ZERO, radius: 0.3, disabled: false });
    for s in parts.stations.iter_mut() {
        if s.id == "breaker" {
            s.prompt = "BREAKER BOX // NOT USED THIS WEEK".to_string();
        }
    }
    parts
}

/// The bay as a playable office (baked). Needs the GL context.
pub fn build_bay(tex: &Textures) -> Office {
    let parts = bay_parts();
    let mut wb = WorldBuilder::new();
    wb.groups = parts.groups;
    wb.solids = parts.solids;
    wb.markers.insert("spawn".to_string(), SPAWN);
    wb.markers.insert("seat".to_string(), SEAT);
    let world = wb.bake(tex);
    let mut o = Office { world, spawn: SPAWN, stations: parts.stations, state: OfficeState::new(), variant: OfficeVariant::Cargo };
    o.snap_slabs([false, false]);
    o.set_button("doorL_door", false);
    o.set_button("doorR_door", false);
    o.set_lamp(true);
    for i in 0..4 {
        set_slot(&mut o, i, None);
    }
    o
}

/// A piece on slot `i` (kind and progress 0..1), or none: its kind tints it, the bar shows the progress.
pub fn set_slot(office: &mut Office, i: usize, piece: Option<(&str, f32)>) {
    let tiny = Vec3::splat(0.0001);
    let cube = format!("PieceSlot{i}");
    match piece {
        Some((kind, progress)) => {
            let (mat, tint, glow, tall) = piece_style(kind);
            office.world.set_material("Slots", &cube, mat, tint, glow);
            office.world.set_cube_scale("Slots", &cube, vec3(1.0, tall, 1.0));
            let shown = bar_segments(progress);
            for k in 0..BAR_SEGMENTS {
                office.world.set_cube_scale("Slots", &format!("Bar{i}_{k}"), if k < shown { Vec3::ONE } else { tiny });
            }
        }
        None => {
            office.world.set_cube_scale("Slots", &cube, tiny);
            for k in 0..BAR_SEGMENTS {
                office.world.set_cube_scale("Slots", &format!("Bar{i}_{k}"), tiny);
            }
        }
    }
}

/// The lights of the day shift: the bay's own (desk lamp, door lights) over a daylight ambient.
pub fn day_lights(office: &Office, flashlight: Option<crate::gfx::Spot>) -> LightSet {
    let mut l = office.lights(flashlight);
    l.ambient = DAY_AMBIENT;
    l
}

// ---- the whole scene -------------------------------------------------------------------------------------------------------

pub struct Scene {
    pub office: Office,
    pub ship: World,
    pub props: PropSet,
}

impl Scene {
    /// Needs the GL context.
    pub fn build(tex: &Textures) -> Scene {
        let office = build_bay(tex);
        let parts = ship_parts();
        let mut wb = WorldBuilder::new();
        wb.groups = parts.groups;
        let ship = wb.bake(tex);
        let props = PropSet::build(&map().props);
        Scene { office, ship, props }
    }

    /// Everything, lit by `lights`, from `cam_pos`.
    pub fn draw_all(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.office.world.draw(&gfx.shader, lights, cam_pos);
        self.ship.draw(&gfx.shader, lights, cam_pos);
        self.props.draw(gfx, lights, cam_pos);
    }
}

/// The lights of a security feed: the shared flat feed ambient, a little dimmer, plus the ceiling lamps of the room (the same
/// spots as the `CeilingPanelLamp` cubes) so every room reads with depth. The open yard gets four lamps along its length.
pub fn feed_lights(room: &str) -> LightSet {
    let base = LightSet::feed();
    let warm = [1.0, 0.93, 0.8];
    let at = |x: f32, y: f32, z: f32, range: f32| Omni { pos: vec3(x, y, z), color: warm, energy: 1.3, range };
    let omni = match room {
        "airlock" => vec![at(-29.0, H - 0.5, 0.0, 9.0)],
        "dockL" => vec![at(-20.2, H - 0.5, -4.5, 12.0), at(-20.2, H - 0.5, 4.5, 12.0)],
        "gantryL" => vec![at(-11.0, H - 0.5, 0.0, 10.0)],
        "gantryR" => vec![at(11.0, H - 0.5, 0.0, 10.0)],
        "engine" => vec![at(-20.2, H - 0.5, -12.5, 11.0), at(-20.2, H - 0.5, -18.0, 11.0)],
        "bridge" => vec![at(-9.0, H - 0.5, -16.0, 11.0), at(-2.0, H - 0.5, -16.0, 11.0)],
        "hold" => vec![at(20.0, H - 0.5, -4.5, 12.0), at(20.0, H - 0.5, 4.5, 12.0)],
        _ => vec![at(-15.0, 4.0, 18.0, 16.0), at(-5.0, 4.0, 18.0, 16.0), at(5.0, 4.0, 18.0, 16.0), at(15.0, 4.0, 18.0, 16.0)],
    };
    LightSet { ambient: [base.ambient[0] * 0.8, base.ambient[1] * 0.8, base.ambient[2] * 0.8], omni, spot: Vec::new(), fog: None }
}

/// Starts drawing into the feed target from a security camera (its own field of view); end with `gfx.feed.end()`.
pub fn begin_feed(gfx: &Gfx, cam: &CamDef) {
    set_camera(&Camera3D {
        position: cam.pos,
        target: cam.target,
        up: Vec3::Y,
        fovy: cam.fov.to_radians(),
        aspect: Some(16.0 / 9.0),
        render_target: Some(gfx.feed.rt.clone()),
        z_near: 0.05,
        z_far: 120.0,
        ..Default::default()
    });
    clear_background(SKY);
}

/// Why this mode cannot start, if so: the Day Shift map does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    MapData::parse(MAP_JSON).err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::nav::NavMap;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(MapData::parse("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn nav() -> NavMap {
        super::super::stage::load_nav()
    }

    #[test]
    fn the_map_data_parses_with_eight_cameras_and_four_slots() {
        let m = MapData::parse(MAP_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.cameras.len(), 8);
        assert_eq!(m.slots.len(), 4);
        assert_eq!(m.rooms.len(), 9);
        assert_eq!(m.entrances, vec!["gantryL".to_string(), "gantryR".to_string(), "yard".to_string()]);
        assert!(m.window_stand.distance(vec3(-1.5, 0.0, 3.45)) < 1e-5);
        assert!(data_problems().is_empty(), "{:?}", data_problems());
        assert_eq!(m.camera_number("gantryL"), 3);
        assert_eq!(m.camera_number("crane"), 0);
        // the rack slots are the Godot SLOT_POS
        assert!(m.slots[0].distance(vec3(0.95, 0.96, 1.25)) < 1e-5);
        assert!(m.slots[3].distance(vec3(2.45, 0.96, 1.25)) < 1e-5);
        assert!(MapData::parse("{").is_err());
    }

    #[test]
    fn the_nine_rooms_are_at_the_godot_centres() {
        let m = map();
        let at = |id: &str| m.rooms.iter().find(|(r, _)| r == id).map(|(_, p)| *p).expect("room");
        assert_eq!(at("airlock"), vec2(-29.0, 0.0));
        assert_eq!(at("dockL"), vec2(-20.0, 0.0));
        assert_eq!(at("gantryL"), vec2(-12.0, 0.0));
        assert_eq!(at("engine"), vec2(-20.0, -15.5));
        assert_eq!(at("bridge"), vec2(-6.0, -16.0));
        assert_eq!(at("hold"), vec2(20.0, 0.0));
        assert_eq!(at("gantryR"), vec2(12.0, 0.0));
        assert_eq!(at("yard"), vec2(0.0, 18.0));
        assert_eq!(at("crane"), vec2(10.0, 20.0));
        assert_eq!(m.edges.len(), 8);
    }

    #[test]
    fn every_camera_hangs_in_its_room_and_agrees_with_the_nav_graph() {
        let nav = nav();
        for c in &map().cameras {
            let rect = nav.rooms().iter().find(|r| r.id == c.room).unwrap_or_else(|| panic!("room {}", c.room));
            assert!(rect.contains(vec3(c.pos.x, 0.0, c.pos.z)), "{} camera is outside its room", c.room);
            assert!(c.pos.y > 2.5 && c.pos.y < 4.5, "{} camera height", c.room);
            assert!(c.fov >= 50.0 && c.fov <= 80.0);
            assert!(!c.label.is_empty());
            let node = nav.cameras(&c.room);
            assert_eq!(node.len(), 1);
            assert!(node[0].p.distance(c.pos) < 1e-4, "{}: nav camera node and map camera differ", c.room);
            // it looks at something in front of it and not straight up or down
            let (yaw, pitch) = c.yaw_pitch();
            assert!(yaw.is_finite() && pitch.abs() < 1.0, "{} pitch {}", c.room, pitch);
            assert!(c.target.distance(c.pos) > 3.0);
        }
        // the camera numbers are 1..=8 in room order
        let cams = cam_layout().cams;
        assert_eq!(cams.len(), 8);
        assert_eq!(cams[0], "airlock");
        assert_eq!(cams[7], "yard");
    }

    #[test]
    fn the_camera_layout_lists_the_crane_without_a_camera() {
        let l = cam_layout();
        assert!(l.rooms.iter().any(|(r, _)| r == "crane"));
        assert!(!l.cams.iter().any(|r| r == "crane"));
        assert_eq!(l.labels.len(), 8);
        assert_eq!(l.office, Some(vec2(0.0, 0.0)));
        assert!(l.edges.contains(&("yard".to_string(), "crane".to_string())));
        assert!(l.vents.is_empty());
    }

    fn inside(w: &WallBox, x: f32, z: f32) -> bool {
        let e = 0.02;
        x > w.min.0 + e && x < w.max.0 - e && z > w.min.1 + e && z < w.max.1 - e
    }

    #[test]
    fn no_nav_edge_walks_through_a_ship_wall() {
        let nav = nav();
        let ship = ship_parts();
        let g = nav.to_graph();
        // every pair of nodes a route joins: sample the walked segments of every route between hubs and spots
        let keys: Vec<String> = nav.nodes().iter().filter(|n| n.kind.walkable()).map(|n| n.key.clone()).collect();
        let mut checked = 0;
        for a in &keys {
            for b in ["C:airlock", "C:hold", "C:crane", "W:stand", "D:L0", "D:R0", "C:bridge"] {
                let legs = g.route(a, b);
                assert!(!legs.is_empty(), "{a} -> {b}");
                for pair in legs.windows(2) {
                    if pair[1].jump {
                        continue;
                    }
                    let (p, q) = (pair[0].p, pair[1].p);
                    let n = ((p.distance(q) / 0.05).ceil() as i32).max(1);
                    for s in 0..=n {
                        let t = s as f32 / n as f32;
                        let at = p.lerp(q, t);
                        for w in &ship.walls {
                            assert!(!inside(w, at.x, at.z), "{a} -> {b}: {} -> {} passes through {} at ({:.2},{:.2})", pair[0].key, pair[1].key, w.name, at.x, at.z);
                        }
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 200);
    }

    #[test]
    fn every_doorway_node_sits_in_a_gap() {
        let nav = nav();
        let ship = ship_parts();
        for n in nav.nodes() {
            if n.kind == crate::hunters::nav::NavKind::Doorway {
                for w in &ship.walls {
                    assert!(!inside(w, n.p.x, n.p.z), "{} is inside a wall", n.key);
                }
            }
        }
    }

    #[test]
    fn the_ship_has_the_rooms_of_the_godot_map_and_props_in_each() {
        let ship = ship_parts();
        let names: Vec<&str> = ship.groups.iter().map(|g| g.name.as_str()).collect();
        for want in ["ShipGantryL", "ShipGantryR", "ShipDock", "ShipAirlock", "ShipEngine", "ShipBridge", "ShipHold", "ShipYard", "ShipCameras"] {
            assert!(names.contains(&want), "{want}");
        }
        for g in &ship.groups {
            assert!(!g.cubes.is_empty());
            for c in &g.cubes {
                assert!(c.size.x > 0.0 && c.size.y > 0.0 && c.size.z > 0.0, "{} {}", g.name, c.name);
                assert!(c.pos.is_finite() && c.size.is_finite());
            }
        }
        // a camera cube per camera
        let cams = ship.groups.iter().find(|g| g.name == "ShipCameras").expect("cameras");
        assert_eq!(cams.cubes.iter().filter(|c| c.name == "CamHousing").count(), 8);
        // the ship's boxes put something in every camera room (built boxes) ...
        let nav = nav();
        let mut per_room: HashMap<String, usize> = HashMap::new();
        for g in ship.groups.iter().filter(|g| g.name != "ShipCameras") {
            for c in &g.cubes {
                let p = g.origin + c.pos;
                if c.name.starts_with("Wall") || c.name.starts_with("Floor") || c.name.starts_with("Ceiling") || c.name.starts_with("Ground") || c.name.starts_with("Fence") {
                    continue;
                }
                if let Some(r) = nav.room_at(vec3(p.x, 0.0, p.z)) {
                    *per_room.entry(r.to_string()).or_insert(0) += 1;
                }
            }
        }
        // ... and the placed bbmodel props add more to every room
        for p in &map().props {
            if let Some(r) = nav.room_at(vec3(p.pos.x, 0.0, p.pos.z)) {
                *per_room.entry(r.to_string()).or_insert(0) += 1;
            } else {
                panic!("prop {} at {:?} is in no room", p.model, p.pos);
            }
        }
        for r in ["airlock", "dockL", "gantryL", "engine", "bridge", "hold", "gantryR", "yard"] {
            assert!(per_room.get(r).copied().unwrap_or(0) >= 5, "room {r} is nearly empty: {:?}", per_room.get(r));
        }
    }

    #[test]
    fn every_prop_model_is_embedded_and_placed_in_the_open() {
        let m = map();
        assert!(m.props.len() >= 25);
        for p in &m.props {
            let t = prop_text(&p.model).unwrap_or_else(|| panic!("model {}", p.model));
            assert!(t.contains("\"elements\""), "{}", p.model);
            assert!(p.pos.is_finite() && p.pos.y >= 0.0 && p.pos.y < 3.0);
        }
        // no prop stands in a doorway lane: the hubs and lane nodes are at least 0.4 m from every prop centre
        let nav = nav();
        for n in nav.nodes().iter().filter(|n| n.kind.walkable() && n.task.is_none()) {
            for p in &m.props {
                let d = vec3(n.p.x - p.pos.x, 0.0, n.p.z - p.pos.z).length();
                assert!(d > 0.45, "{} is {:.2} m from a {}", n.key, d, p.model);
            }
        }
    }

    #[test]
    fn task_spots_are_clear_of_the_props_and_look_at_something() {
        let nav = nav();
        let m = map();
        for n in nav.nodes().iter().filter(|n| n.task.is_some()) {
            let look = n.look.expect("look");
            let reach = if n.pace_to.is_some() { 12.0 } else { 4.0 };
            assert!(look.distance(n.p) > 0.4 && look.distance(n.p) < reach, "{} looks {:.2} m away", n.key, look.distance(n.p));
            for p in &m.props {
                let d = vec3(n.p.x - p.pos.x, 0.0, n.p.z - p.pos.z).length();
                assert!(d > 0.35, "{} stands inside a {}", n.key, p.model);
            }
        }
    }

    #[test]
    fn the_bay_has_a_window_in_the_front_wall_and_no_hall_ends() {
        let p = bay_parts();
        // nothing solid fills the window opening
        let probe = vec3(WINDOW_X, 1.5, HALF_D + 0.1);
        assert!(!p.solids.iter().any(|a| probe.x >= a.min.x && probe.x <= a.max.x && probe.y >= a.min.y && probe.y <= a.max.y && probe.z >= a.min.z && probe.z <= a.max.z));
        // but the wall beside it is still there
        let beside = vec3(2.0, 1.5, HALF_D + 0.1);
        assert!(p.solids.iter().any(|a| beside.x >= a.min.x && beside.x <= a.max.x && beside.y >= a.min.y && beside.y <= a.max.y && beside.z >= a.min.z && beside.z <= a.max.z));
        let names: Vec<&str> = p.groups.iter().flat_map(|g| g.cubes.iter().map(|c| c.name.as_str())).collect();
        assert!(names.contains(&"WallFrontSill") && names.contains(&"WallFrontHead") && names.contains(&"FrameWindow"));
        assert!(!names.contains(&"WallFront"));
        assert!(!names.contains(&"HallLEnd") && !names.contains(&"HallREnd") && !names.contains(&"HallEndDoor"));
        // the Godot extras
        for want in ["BenchTop", "PieceSlot0", "PieceSlot3", "Bar2_5", "LedgerBook", "CrateBig", "SignColour", "SillRed"] {
            assert!(names.contains(&want), "{want}");
        }
        // the signature of the cargo bay is on
        assert!(p.groups.iter().any(|g| g.name == "Signature"));
    }

    #[test]
    fn the_bay_keeps_its_stations_and_adds_the_rack_and_ledger() {
        let p = bay_parts();
        for id in ["doorL_door", "doorR_door", "doorL_light", "doorR_light", "lamp", "chair", "monitor", "breaker", "rack0", "rack1", "rack2", "rack3", "ledger"] {
            assert!(p.stations.iter().any(|s| s.id == id), "{id}");
        }
        // the monitor is on in this port (cameras), the breaker is not used
        assert!(!p.stations.iter().find(|s| s.id == "monitor").map(|s| s.disabled).unwrap_or(true));
        assert!(p.stations.iter().find(|s| s.id == "breaker").map(|s| s.prompt.contains("NOT USED")).unwrap_or(false));
        // an empty slot has nothing to aim at until a piece turns up
        assert!(p.stations.iter().filter(|s| s.id.starts_with("rack")).all(|s| s.disabled));
        let r2 = p.stations.iter().find(|s| s.id == "rack2").map(|s| s.pos).unwrap_or_default();
        assert!(r2.distance(vec3(1.95, 0.96, 1.25)) < 1e-5);
    }

    #[test]
    fn the_workbench_legs_are_solid_but_the_slots_are_clear() {
        let p = bay_parts();
        let leg = Vec3::new(0.7, 0.4, 1.25);
        assert!(p.solids.iter().any(|a| leg.x >= a.min.x && leg.x <= a.max.x && leg.z >= a.min.z && leg.z <= a.max.z && leg.y >= a.min.y && leg.y <= a.max.y));
        // the player's spawn is not inside a solid
        assert!(!p.solids.iter().any(|a| SPAWN.x >= a.min.x && SPAWN.x <= a.max.x && SPAWN.z >= a.min.z && SPAWN.z <= a.max.z && a.max.y > 0.4));
    }

    #[test]
    fn rack_pieces_are_styled_by_kind_and_bars_fill_in_sixths() {
        assert_eq!(piece_style("scrap").0, "metal");
        assert!(piece_style("prize").2, "a prize glows");
        assert!(piece_style("parts").3 > piece_style("scrap").3);
        assert!(piece_style("prize").3 > piece_style("parts").3);
        assert_eq!(bar_segments(0.0), 0);
        assert_eq!(bar_segments(0.01), 1);
        assert_eq!(bar_segments(0.5), 3);
        assert_eq!(bar_segments(1.0), 6);
        assert_eq!(bar_segments(2.0), 6);
    }

    #[test]
    fn the_lights_keep_the_bay_and_set_a_day_ambient() {
        let state = OfficeState::new();
        let l = state.lights(None);
        assert!(!l.omni.is_empty());
        assert!(DAY_AMBIENT.iter().all(|c| *c > 0.5 && *c < 1.0));
    }

    #[test]
    fn no_camera_housing_or_lamp_hangs_in_front_of_its_own_lens() {
        let ship = ship_parts();
        let cams = ship.groups.iter().find(|g| g.name == "ShipCameras").expect("cameras");
        for c in &map().cameras {
            let dir = (c.target - c.pos).normalize();
            for cube in cams.cubes.iter().filter(|q| q.name == "CamHousing" || q.name == "LedCamera") {
                let rel = cube.pos - c.pos;
                if rel.length() < 0.6 {
                    // the cube's nearest point to the view direction stays behind the lens plane
                    let reach = cube.size.length() * 0.5;
                    assert!(rel.dot(dir) + reach <= 0.06, "{} {} is in front of the {} lens", cube.name, rel.dot(dir), c.room);
                }
            }
        }
    }

    #[test]
    fn every_feed_room_has_lamps_and_every_ceilinged_room_has_lamp_panels() {
        for c in &map().cameras {
            let l = feed_lights(&c.room);
            assert!(!l.omni.is_empty() && l.omni.len() <= crate::gfx::MAX_OMNI, "{}", c.room);
            assert!(l.ambient.iter().all(|a| *a > 0.5));
        }
        let ship = ship_parts();
        for name in ["ShipDock", "ShipAirlock", "ShipEngine", "ShipBridge", "ShipHold"] {
            let g = ship.groups.iter().find(|g| g.name == name).expect("group");
            assert!(g.cubes.iter().any(|c| c.name == "CeilingPanelLamp"), "{name}");
        }
    }
}
