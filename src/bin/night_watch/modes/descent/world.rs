//! The Descent's 3D set (`descent_world.gd`, `descent_map.gd`): the service elevator car seen from inside, the caged
//! windows, the shaft with its sills and the beams that slide past while the car drops a floor, and the rest of the
//! map at the Godot coordinates: the stairwell landings down the left, the pipe gallery and the fuse closet on the right,
//! the signal hall at the bottom, and the flights and walkways Elise takes between them. Every room is furnished.
//! The cube lists (`build_groups`) are pure; `DescentWorld` bakes them (needs GL).
#![allow(dead_code)]

use super::map::{Car, FlightDef, MapData, ModelDef, PropDef, RoomDef, View};
use crate::hunters::bbmodel::Model;
use super::sim::Side;
use crate::gfx::{blocky_prop, Cube, Gfx, Group, LightSet, Omni, Textures, World, WorldBuilder};
use macroquad::prelude::*;

const SHELL: [f32; 3] = [0.7, 0.7, 0.75];

fn c(g: &mut Group, name: &str, size: [f32; 3], pos: [f32; 3], kind: &str, tint: [f32; 3], solid: bool) {
    let mut cube = Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).tint(tint);
    if !kind.is_empty() {
        cube = cube.kind(kind);
    }
    if solid {
        cube = cube.solid();
    }
    g.add(cube);
}

// ------------------------------------------------------------------------------------------------ the car

fn build_car(g: &mut Group, car: &Car) {
    let (hw, hd, h) = (car.half_w, car.half_d, car.height);
    let (wb, wt, wx, ww) = (car.window_bottom, car.window_top, car.window_x, car.window_w);
    c(g, "Floor", [hw * 2.0 + 0.4, 0.2, hd * 2.0 + 0.4], [0.0, -0.1, 0.0], "", [1.0, 1.0, 1.0], true);
    c(g, "Ceiling", [hw * 2.0 + 0.4, 0.2, hd * 2.0 + 0.4], [0.0, h + 0.1, 0.0], "", [0.7, 0.7, 0.75], false);
    for x in [-hw - 0.1, hw + 0.1] {
        c(g, "WallSide", [0.2, h, hd * 2.0], [x, h * 0.5, 0.0], "", [0.8, 0.8, 0.85], true);
    }
    c(g, "WallFront", [hw * 2.0 + 0.4, h, 0.2], [0.0, h * 0.5, hd + 0.1], "", SHELL, true);
    // the back wall, with a window opening each side of the service panel
    let back = [0.85, 0.85, 0.9];
    c(g, "WallBackLow", [hw * 2.0, wb, 0.2], [0.0, wb * 0.5, -hd - 0.1], "", back, true);
    c(g, "WallBackHigh", [hw * 2.0, h - wt, 0.2], [0.0, wt + (h - wt) * 0.5, -hd - 0.1], "", back, true);
    let mid_h = wt - wb;
    let mid_y = wb + mid_h * 0.5;
    let edge_w = hw - (wx + ww * 0.5);
    let centre_w = (wx - ww * 0.5) * 2.0;
    c(g, "WallBackPanel", [centre_w, mid_h, 0.2], [0.0, mid_y, -hd - 0.1], "", back, true);
    for s in [-1.0f32, 1.0] {
        c(g, "WallBackEdge", [edge_w, mid_h, 0.2], [s * (hw - edge_w * 0.5), mid_y, -hd - 0.1], "", back, true);
    }
    // the service panel plate on the wall between the windows (the repair screen is drawn over it)
    c(g, "PanelPlate", [centre_w - 0.1, mid_h - 0.1, 0.05], [0.0, mid_y, -hd + 0.03], "metal", [0.5, 0.52, 0.58], false);
    for z in [-hd + 0.04, hd - 0.04] {
        c(g, "Baseboard", [hw * 2.0, 0.2, 0.08], [0.0, 0.1, z], "", SHELL, false);
        c(g, "CrownTrim", [hw * 2.0, 0.12, 0.08], [0.0, h - 0.06, z], "", [0.8, 0.8, 0.85], false);
    }
    // a hand rail along the side walls, floor stripes, a ceiling fixture
    for x in [-hw + 0.05, hw - 0.05] {
        c(g, "HandRail", [0.06, 0.06, hd * 2.0 - 0.3], [x, 1.0, 0.0], "metal", [1.0, 1.0, 1.0], false);
    }
    c(g, "FloorStripe", [2.4, 0.01, 0.12], [0.0, 0.005, 0.3], "label", [0.8, 0.8, 0.8], false);
    c(g, "CeilingFixture", [1.1, 0.08, 0.5], [0.0, h - 0.04, 0.1], "plastic", [1.0, 1.0, 1.0], false);
    c(g, "CeilingLens", [0.9, 0.02, 0.3], [0.0, h - 0.1, 0.1], "glass", [0.7, 0.62, 0.45], false);
    c(g, "LedAlarm", [0.06, 0.06, 0.02], [-1.5, 2.2, -hd + 0.03], "", [1.0, 1.0, 1.0], false);
}

fn build_cages(g: &mut Group, car: &Car) {
    let (hd, wb, wt, wx, ww) = (car.half_d, car.window_bottom, car.window_top, car.window_x, car.window_w);
    let metal = [0.55, 0.55, 0.6];
    for s in [-1.0f32, 1.0] {
        let cx = s * wx;
        // a cage of bars across the opening
        let bars = (ww / 0.12) as i32;
        for i in 0..=bars {
            let bx = cx - ww * 0.5 + 0.06 + i as f32 * ((ww - 0.12) / (bars.max(1) as f32));
            c(g, "CageBar", [0.03, wt - wb, 0.03], [bx, (wt + wb) * 0.5, -hd - 0.02], "metal", metal, false);
        }
        for y in [wb + 0.02, wt - 0.02, (wt + wb) * 0.5] {
            c(g, "CageRail", [ww, 0.04, 0.04], [cx, y, -hd - 0.02], "metal", metal, false);
        }
    }
}

// ------------------------------------------------------------------------------------------------ the shaft

fn build_shaft(g: &mut Group, view: &View, sill_props: &[PropDef]) {
    c(g, "ShaftWallBack", [9.0, 12.0, 0.3], [0.0, 3.0, -6.3], "concrete", [0.45, 0.47, 0.55], false);
    // the side walls have a doorway at z -5.6 for Elise's catwalk
    for x in [-4.4f32, 4.4] {
        let t = [0.4, 0.42, 0.5];
        c(g, "ShaftWallSide", [0.3, 3.0, 6.0], [x, -1.5, -3.6], "concrete", t, false);
        c(g, "ShaftWallSide", [0.3, 6.4, 6.0], [x, 5.8, -3.6], "concrete", t, false);
        c(g, "ShaftWallSide", [0.3, 2.6, 0.5], [x, 1.3, -6.35], "concrete", t, false);
        c(g, "ShaftWallSide", [0.3, 2.6, 4.5], [x, 1.3, -2.85], "concrete", t, false);
    }
    // a sill outside each window for Elise to walk along
    for s in [-1.0f32, 1.0] {
        c(g, "SillPlatform", [1.2, 0.2, 4.3], [s * view.window_x, -0.1, -3.75], "metal", [0.5, 0.5, 0.55], false);
        c(g, "SillRail", [0.06, 0.9, 4.3], [s * (view.window_x + 0.6), 0.45, -3.75], "metal", [0.6, 0.5, 0.2], false);
        for p in sill_props {
            let origin = vec3(if p.kind == "terminal" { s * view.window_x } else { s * p.at.0 }, 0.0, p.at.1);
            build_prop(g, &p.kind, origin, p.tint, p.size, Some(s));
        }
    }
}

/// The beams and rails that slide past while the car drops a floor, and the stairwell and pipes of every level.
fn build_dress(g: &mut Group, view: &View) {
    let gap = view.beam_gap;
    for k in -4i32..=4 {
        let y = k as f32 * gap;
        c(g, "BeamCross", [8.4, 0.25, 0.35], [0.0, y, -6.0], "metal", [0.5, 0.52, 0.6], false);
        for x in [-3.2f32, 3.2] {
            c(g, "BeamArm", [0.35, 0.25, 4.6], [x, y + 0.5, -3.6], "metal", [0.5, 0.5, 0.58], false);
        }
        c(g, "Sign", [0.5, 0.18, 0.08], [-3.9, y + 1.0, -5.8], "", [0.9, 0.7, 0.3], false);
        // the stairwell level: a landing on the left wall and a flight of steps up to the next one
        c(g, "StairLanding", [1.0, 0.15, 1.2], [-3.7, y, -5.2], "metal", [0.55, 0.55, 0.6], false);
        for i in 0..5 {
            c(g, "StairStep", [0.7, 0.12, 0.42], [-3.8, y + 0.12 + 0.55 * i as f32, -4.5 + 0.5 * i as f32], "metal", [0.5, 0.5, 0.55], false);
        }
        // the service pipes on the right wall with a valve and a fuse box per level
        for dx in [0.0f32, 0.22, 0.44] {
            let tint = if dx < 0.3 { [0.6, 0.45, 0.35] } else { [0.5, 0.55, 0.65] };
            c(g, "PipeRun", [0.14, 3.0, 0.14], [3.6 + dx, y + 1.5, -5.7], "metal", tint, false);
        }
        c(g, "PipeValve", [0.36, 0.3, 0.3], [3.8, y + 1.2, -5.5], "metal", [0.8, 0.25, 0.2], false);
        c(g, "FuseCabinet", [0.7, 0.5, 0.2], [1.6, y + 1.5, -6.05], "metal", [0.45, 0.5, 0.5], false);
        c(g, "LedLevel", [0.08, 0.08, 0.04], [1.8, y + 1.6, -5.92], "", [1.0, 1.0, 1.0], false);
    }
    for x in [-3.9f32, -1.9, 1.9, 3.9] {
        c(g, "RailTall", [0.15, 24.0, 0.15], [x, 0.0, -5.9], "metal", [0.6, 0.6, 0.65], false);
    }
}

// ------------------------------------------------------------------------------------------------ props

/// A prop of the map file. The common kinds come from the shared blocky set; the shaft's own are built here. `side` is
/// Some(sign) for props on a sill (the terminal faces the sill).
pub fn build_prop(g: &mut Group, kind: &str, o: Vec3, tint: [f32; 3], size: f32, side: Option<f32>) {
    match kind {
        "breaker" => {
            c(g, "BreakerBody", [0.7, 1.6, 0.3], [o.x, o.y + 0.8, o.z], "metal", tint, true);
            c(g, "BreakerDoor", [0.6, 0.5, 0.04], [o.x, o.y + 1.0, o.z + 0.17], "metal", [tint[0] * 0.7, tint[1] * 0.7, tint[2] * 0.7], false);
            c(g, "LedBreaker", [0.08, 0.08, 0.04], [o.x + 0.22, o.y + 1.45, o.z + 0.17], "", [1.0, 1.0, 1.0], false);
            c(g, "BreakerLever", [0.06, 0.22, 0.06], [o.x - 0.15, o.y + 1.0, o.z + 0.2], "plastic", [0.8, 0.25, 0.2], false);
        }
        "pipes" => {
            c(g, "PipeRun", [0.16, 3.0, 0.16], [o.x, o.y + 1.5, o.z], "metal", tint, false);
            c(g, "PipeBand", [0.22, 0.1, 0.22], [o.x, o.y + 1.0, o.z], "plastic", [0.3, 0.3, 0.34], false);
            c(g, "PipeBand", [0.22, 0.1, 0.22], [o.x, o.y + 2.0, o.z], "plastic", [0.3, 0.3, 0.34], false);
        }
        "valve" => {
            c(g, "ValveBody", [0.3, 0.3, 0.3], [o.x, o.y + 0.9, o.z], "metal", tint, true);
            c(g, "ValveWheel", [0.34, 0.04, 0.34], [o.x, o.y + 1.1, o.z], "metal", [0.8, 0.7, 0.2], false);
            c(g, "ValveStem", [0.06, 0.8, 0.06], [o.x, o.y + 0.4, o.z], "metal", [0.5, 0.5, 0.55], false);
        }
        "console" => {
            c(g, "ConsoleBase", [1.4, 0.9, 0.6], [o.x, o.y + 0.45, o.z], "metal", tint, true);
            c(g, "ConsoleTop", [1.5, 0.08, 0.7], [o.x, o.y + 0.94, o.z], "plastic", [0.3, 0.3, 0.34], false);
            c(g, "ScreenSignal", [1.2, 0.5, 0.04], [o.x, o.y + 1.3, o.z - 0.25], "", [0.6, 0.95, 0.8], false);
            c(g, "ButtonRow", [1.0, 0.04, 0.16], [o.x, o.y + 0.99, o.z + 0.1], "", [0.96, 0.72, 0.28], false);
        }
        "terminal" => {
            let s = side.unwrap_or(1.0);
            let _ = s;
            c(g, "TerminalBody", [0.6, 0.8, 0.14], [o.x, o.y + 1.1, o.z], "metal", tint, false);
            c(g, "ScreenTerm", [0.46, 0.5, 0.03], [o.x, o.y + 1.15, o.z + 0.085], "", [0.5, 0.95, 0.75], false);
            c(g, "ButtonStrip", [0.4, 0.05, 0.03], [o.x, o.y + 0.82, o.z + 0.085], "", [0.96, 0.72, 0.28], false);
        }
        "seat" => {
            c(g, "SeatCrate", [0.45, 0.44, 0.45], [o.x, o.y + 0.22, o.z], "wood", tint, true);
        }
        "crate" if size > 0.0 => {
            c(g, "CrateBox", [size, size, size], [o.x, o.y + size * 0.5, o.z], "wood", tint, true);
            c(g, "CrateBox", [size * 0.8, size * 0.8, size * 0.8], [o.x, o.y + size * 1.4, o.z], "wood", [tint[0] * 0.85, tint[1] * 0.85, tint[2] * 0.85], false);
        }
        other => blocky_prop(g, other, o, tint),
    }
}

// ------------------------------------------------------------------------------------------------ rooms and flights

fn build_room(g: &mut Group, r: &RoomDef) {
    let (cx, cy, cz) = (r.centre.x, r.centre.y, r.centre.z);
    let (w, d) = r.size;
    c(g, "RoomFloor", [w + 0.2, 0.2, d + 0.2], [cx, cy - 0.1, cz], "floor", r.tone, true);
    for side in r.walls.chars() {
        match side {
            'n' => c(g, "RoomWall", [w + 0.2, 3.0, 0.15], [cx, cy + 1.5, cz - d * 0.5], "wall", r.tone, true),
            's' => c(g, "RoomWall", [w + 0.2, 3.0, 0.15], [cx, cy + 1.5, cz + d * 0.5], "wall", r.tone, true),
            'e' => c(g, "RoomWall", [0.15, 3.0, d + 0.2], [cx + w * 0.5, cy + 1.5, cz], "wall", r.tone, true),
            'w' => c(g, "RoomWall", [0.15, 3.0, d + 0.2], [cx - w * 0.5, cy + 1.5, cz], "wall", r.tone, true),
            _ => {}
        }
    }
    if r.lamp {
        c(g, "LampCable", [0.03, 0.6, 0.03], [cx, cy + 2.9, cz], "metal", [0.3, 0.3, 0.34], false);
        c(g, "LampGlow", [0.4, 0.12, 0.4], [cx, cy + 2.55, cz], "", [1.0, 0.9, 0.6], false);
    }
    // a stripe along the floor edge so the landing reads from the shaft
    c(g, "EdgeStripe", [w, 0.01, 0.1], [cx, cy + 0.005, cz + d * 0.5 - 0.1], "label", [0.8, 0.8, 0.8], false);
    for p in &r.props {
        build_prop(g, &p.kind, vec3(cx + p.at.0, cy, cz + p.at.1), p.tint, p.size, None);
    }
}

/// A flight of concrete blocks from `a` to `b` (any direction), or a flat walkway when the ends are level.
fn build_flight(g: &mut Group, f: &FlightDef) {
    let (dx, dy, dz) = (f.b.x - f.a.x, f.b.y - f.a.y, f.b.z - f.a.z);
    let hlen = (dx * dx + dz * dz).sqrt().max(0.01);
    let rot = dx.atan2(dz);
    let tint = [0.55, 0.55, 0.6];
    if dy.abs() < 0.05 {
        let mid = (f.a + f.b) * 0.5;
        g.add(Cube::new("Walkway", vec3(f.width, 0.2, hlen), vec3(mid.x, f.a.y - 0.1, mid.z)).kind("metal").tint(tint).rot_y(rot).solid());
        return;
    }
    let n = ((dy.abs() / 0.25).ceil() as usize).max(3);
    let low = f.a.y.min(f.b.y) - 0.2;
    for i in 0..n {
        let t = (i as f32 + 0.5) / n as f32;
        let top = f.a.y + dy * t;
        let h = top - low;
        let cube = Cube::new("StairBlock", vec3(f.width, h, hlen / n as f32 + 0.01), vec3(f.a.x + dx * t, low + h * 0.5, f.a.z + dz * t))
            .kind("concrete")
            .tint(tint)
            .rot_y(rot)
            .solid();
        g.add(cube);
    }
}

/// All the groups of the set: the car, the cages, the shaft, the sliding dress, the map's rooms and flights.
pub fn build_groups(map: &MapData, view: &View) -> Vec<Group> {
    let mut car = Group::new("car", Vec3::ZERO);
    build_car(&mut car, &map.car);
    let mut cages = Group::new("cages", Vec3::ZERO);
    build_cages(&mut cages, &map.car);
    let mut shaft = Group::new("shaft", Vec3::ZERO);
    build_shaft(&mut shaft, view, &map.sill_props);
    let mut dress = Group::new("dress", Vec3::ZERO);
    build_dress(&mut dress, view);
    let mut rooms = Group::new("rooms", Vec3::ZERO);
    for r in &map.rooms {
        build_room(&mut rooms, r);
    }
    for f in &map.flights {
        build_flight(&mut rooms, f);
    }
    vec![car, cages, shaft, dress, rooms]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The lights of the set: the car's lamp (it reddens and dims as she closes in) and one dim lamp over each sill.
pub fn lights(view: &View, closeness: f32) -> LightSet {
    let t = (closeness / 100.0).clamp(0.0, 1.0);
    let mut omni = vec![Omni {
        pos: view.lamp_pos,
        color: lerp3(view.lamp_calm, view.lamp_alarm, t),
        energy: view.lamp_energy_calm + (view.lamp_energy_alarm - view.lamp_energy_calm) * t,
        range: view.lamp_range,
    }];
    for s in [-1.0f32, 1.0] {
        omni.push(Omni {
            pos: vec3(s * view.window_x, view.sill_light_y, view.sill_light_z),
            color: view.sill_light_colour,
            energy: view.sill_light_energy,
            range: view.sill_light_range,
        });
    }
    LightSet { ambient: view.ambient, omni, spot: Vec::new(), fog: None }
}

/// The bbmodel text of a prop from `last_route/assets/props`, by file stem (the models the Descent rooms use).
pub fn prop_text(model: &str) -> Option<&'static str> {
    macro_rules! props {
        ($($name:literal),* $(,)?) => {
            match model {
                $($name => Some(include_str!(concat!("../../../last_route/assets/props/", $name, ".bbmodel"))),)*
                _ => None,
            }
        };
    }
    props!(
        "supplyCrates",
        "gasCylinderRack",
        "filingCabinet",
        "trashBin",
        "cableCoil",
        "maintenanceToolCabinet",
        "storageBarrel",
        "serverRack",
        "storageShelving",
        "irrigationManifold",
        "breakerCabinet",
        "rackGear",
    )
}

/// Where a room's model stands (world space).
pub fn model_origin(r: &RoomDef, m: &ModelDef) -> Vec3 {
    vec3(r.centre.x + m.at.0, r.centre.y + m.y, r.centre.z + m.at.1)
}

/// One room model as a mesh (the model stands on its placement point, turned about the vertical axis).
fn model_mesh(model: &Model, pos: Vec3, m: &ModelDef) -> Mesh {
    let (mut verts, mut idx) = (Vec::new(), Vec::new());
    model.build_scaled(&mut verts, &mut idx, pos, m.yaw_deg.to_radians(), Vec3::splat(model.scale * m.scale), 0.0, 0.0, WHITE);
    Mesh { vertices: verts, indices: idx, texture: Some(model.texture.clone()) }
}

/// The baked set.
pub struct DescentWorld {
    pub world: World,
    props: Vec<Mesh>,
    pub view: View,
    pub map: MapData,
    ride: f32,
}

impl DescentWorld {
    pub fn new(tex: &Textures) -> DescentWorld {
        let map = MapData::get().clone();
        let view = View::get().clone();
        let mut wb = WorldBuilder::new();
        for g in build_groups(&map, &view) {
            wb.add_group(g);
        }
        let mut world = wb.bake(tex);
        world.set_offset("dress", Vec3::ZERO);
        let mut cache: Vec<(String, Option<Model>)> = Vec::new();
        let mut props = Vec::new();
        for r in &map.rooms {
            for m in &r.models {
                if !cache.iter().any(|(k, _)| *k == m.model) {
                    let parsed = prop_text(&m.model).and_then(|t| Model::parse(t, None, &|_| None, None));
                    if let Some(p) = &parsed {
                        p.texture.set_filter(FilterMode::Nearest);
                    }
                    cache.push((m.model.clone(), parsed));
                }
                if let Some((_, Some(model))) = cache.iter().find(|(k, _)| *k == m.model) {
                    let mesh = model_mesh(model, model_origin(r, m), m);
                    if !mesh.vertices.is_empty() {
                        props.push(mesh);
                    }
                }
            }
        }
        DescentWorld { world, props, view, map, ride: 0.0 }
    }

    /// 0..1 through a ride: the shaft slides up past the windows by one beam gap (it looks the same at both ends).
    pub fn set_ride(&mut self, progress: f32) {
        let p = progress.clamp(0.0, 1.0);
        self.ride = p;
        let s = p * p * (3.0 - 2.0 * p);
        self.world.set_offset("dress", vec3(0.0, self.view.ride_drop * s, 0.0));
    }

    pub fn ride(&self) -> f32 {
        self.ride
    }

    pub fn lights(&self, closeness: f32) -> LightSet {
        lights(&self.view, closeness)
    }

    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        gfx.shader.begin(lights, cam_pos);
        for m in &self.props {
            crate::gfx::mesh::draw_mesh_safe(m);
        }
        gfx.shader.end();
    }

    pub fn prop_count(&self) -> usize {
        self.props.len()
    }
}

/// Where Elise stands on `side` when she is `closeness` (0..100) of the way in.
pub fn elise_position(side: Side, closeness: f64) -> Vec3 {
    super::map::sill_pos(side, closeness)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> Vec<Group> {
        build_groups(MapData::get(), View::get())
    }

    fn group<'a>(gs: &'a [Group], name: &str) -> &'a Group {
        gs.iter().find(|g| g.name == name).expect("group")
    }

    fn count(g: &Group, name: &str) -> usize {
        g.cubes.iter().filter(|c| c.name == name).count()
    }

    #[test]
    fn every_descent_room_is_dressed_with_bbmodel_props() {
        for r in &MapData::get().rooms {
            assert!(r.models.len() >= 2, "{} has bbmodel props", r.id);
            for m in &r.models {
                let t = prop_text(&m.model).unwrap_or_else(|| panic!("{}: {} has no model text", r.id, m.model));
                assert!(t.contains("\"elements\""), "{}", m.model);
            }
        }
        assert!(prop_text("noSuchProp").is_none());
    }

    #[test]
    fn room_models_stay_on_the_floor_and_off_the_flight_ports() {
        let map = MapData::get();
        for r in &map.rooms {
            for m in &r.models {
                let o = model_origin(r, m);
                assert!(m.at.0.abs() <= r.size.0 * 0.5 && m.at.1.abs() <= r.size.1 * 0.5, "{} {} on the floor", r.id, m.model);
                for f in &map.flights {
                    for end in [f.a, f.b] {
                        if (end.y - r.centre.y).abs() < 0.1 {
                            let d = ((o.x - end.x).powi(2) + (o.z - end.z).powi(2)).sqrt();
                            assert!(d > 0.6, "{} {} blocks flight {} ({d:.2} m)", r.id, m.model, f.id);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn room_models_leave_the_hunter_stand_free() {
        for r in &MapData::get().rooms {
            for m in &r.models {
                let d = (m.at.0 * m.at.0 + (m.at.1 + 0.3).powi(2)).sqrt();
                assert!(d > 0.5, "{} {} sits on the hunter stand", r.id, m.model);
            }
        }
    }

    #[test]
    fn the_set_has_its_five_groups() {
        let gs = groups();
        let names: Vec<&str> = gs.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["car", "cages", "shaft", "dress", "rooms"]);
    }

    #[test]
    fn the_car_has_the_godot_walls_and_a_window_opening_each_side() {
        let gs = groups();
        let car = group(&gs, "car");
        assert_eq!(count(car, "WallSide"), 2);
        assert_eq!(count(car, "WallBackEdge"), 2);
        assert_eq!(count(car, "WallBackPanel"), 1);
        assert_eq!(count(car, "PanelPlate"), 1);
        // no solid wall cube covers the window opening (x 0.65..1.55, y 0.85..3.2 on either side)
        for c in car.cubes.iter().filter(|c| c.name.starts_with("WallBack")) {
            for s in [-1.0f32, 1.0] {
                let (lo, hi) = (c.pos.x - c.size.x * 0.5, c.pos.x + c.size.x * 0.5);
                let (ylo, yhi) = (c.pos.y - c.size.y * 0.5, c.pos.y + c.size.y * 0.5);
                let wx = s * 1.1;
                let covers_x = lo < wx && hi > wx;
                let covers_y = ylo < 2.0 && yhi > 2.0;
                assert!(!(covers_x && covers_y), "{} covers the window at {wx}", c.name);
            }
        }
        let floor = car.cubes.iter().find(|c| c.name == "Floor").expect("floor");
        assert!((floor.size.x - 3.8).abs() < 1e-5 && (floor.size.z - 3.2).abs() < 1e-5);
    }

    #[test]
    fn each_window_has_a_cage_of_bars() {
        let gs = groups();
        let cages = group(&gs, "cages");
        assert_eq!(count(cages, "CageBar"), 2 * 8, "int(0.9 / 0.12) + 1 bars per window");
        assert_eq!(count(cages, "CageRail"), 6);
    }

    #[test]
    fn the_shaft_has_two_sills_a_doorway_in_each_side_wall_and_the_sill_props() {
        let gs = groups();
        let shaft = group(&gs, "shaft");
        assert_eq!(count(shaft, "SillPlatform"), 2);
        assert_eq!(count(shaft, "ShaftWallSide"), 8, "four pieces per side wall around the doorway");
        let terms: Vec<f32> = shaft.cubes.iter().filter(|c| c.name == "TerminalBody").map(|c| c.pos.x).collect();
        assert_eq!(terms.len(), 2);
        assert!(terms.contains(&-1.1) && terms.contains(&1.1), "a terminal on each sill: {terms:?}");
        assert!(count(shaft, "SeatCrate") == 2 && count(shaft, "LockerBody") == 2);
        // the doorway is open between y 0 and 2.6 at z -6.1..-5.1
        for c in shaft.cubes.iter().filter(|c| c.name == "ShaftWallSide") {
            let (z0, z1) = (c.pos.z - c.size.z * 0.5, c.pos.z + c.size.z * 0.5);
            let (y0, y1) = (c.pos.y - c.size.y * 0.5, c.pos.y + c.size.y * 0.5);
            let blocks = z0 < -5.6 && z1 > -5.6 && y0 < 1.3 && y1 > 1.3;
            assert!(!blocks, "the doorway at z -5.6 is open");
        }
    }

    #[test]
    fn the_dress_has_nine_levels_of_beams_stairs_and_pipes() {
        let gs = groups();
        let dress = group(&gs, "dress");
        assert_eq!(count(dress, "BeamCross"), 9);
        assert_eq!(count(dress, "StairStep"), 45);
        assert_eq!(count(dress, "PipeRun"), 27);
        assert_eq!(count(dress, "RailTall"), 4);
        let ys: Vec<f32> = dress.cubes.iter().filter(|c| c.name == "BeamCross").map(|c| c.pos.y).collect();
        assert!(ys.contains(&-12.0) && ys.contains(&0.0) && ys.contains(&12.0));
    }

    #[test]
    fn every_room_is_a_furnished_platform_at_its_map_position() {
        let gs = groups();
        let rooms = group(&gs, "rooms");
        let map = MapData::get();
        for r in &map.rooms {
            let floor = rooms
                .cubes
                .iter()
                .find(|c| c.name == "RoomFloor" && (c.pos.x - r.centre.x).abs() < 1e-4 && (c.pos.z - r.centre.z).abs() < 1e-4)
                .unwrap_or_else(|| panic!("floor of {}", r.id));
            assert!((floor.pos.y - (r.centre.y - 0.1)).abs() < 1e-5, "{} floor level", r.id);
            let near = rooms
                .cubes
                .iter()
                .filter(|c| c.name != "RoomFloor" && c.name != "EdgeStripe" && c.name != "RoomWall")
                .filter(|c| (c.pos.x - r.centre.x).abs() < r.size.0 * 0.5 + 0.8 && (c.pos.z - r.centre.z).abs() < r.size.1 * 0.5 + 0.8)
                .filter(|c| c.pos.y > r.centre.y - 0.2 && c.pos.y < r.centre.y + 3.5)
                .count();
            assert!(near >= 6, "{} holds props ({near} cubes)", r.id);
        }
    }

    #[test]
    fn flights_start_and_end_on_their_ports_and_rise_to_the_level_below() {
        let gs = groups();
        let rooms = group(&gs, "rooms");
        let blocks: Vec<&Cube> = rooms.cubes.iter().filter(|c| c.name == "StairBlock").collect();
        assert!(blocks.len() >= 6 * 12, "{} stair blocks", blocks.len());
        // the first f1 block sits near (-7, -7) and its top is near the upper landing; all tops lie between the levels
        let f1 = MapData::get().flights.iter().find(|f| f.id == "f1").expect("f1");
        let first = blocks.iter().min_by(|a, b| {
            let da = (a.pos.x - f1.a.x).powi(2) + (a.pos.z - f1.a.z).powi(2);
            let db = (b.pos.x - f1.a.x).powi(2) + (b.pos.z - f1.a.z).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        let first = first.expect("block");
        let top = first.pos.y + first.size.y * 0.5;
        assert!(top < 0.0 && top > -0.4, "the top step is just under the upper landing: {top}");
        assert!(rooms.cubes.iter().any(|c| c.name == "Walkway"), "the catwalks to the shaft doorways");
    }

    #[test]
    fn the_lamp_reddens_and_dims_as_she_closes_in() {
        let v = View::get();
        let calm = lights(v, 0.0);
        let hot = lights(v, 100.0);
        assert_eq!(calm.omni.len(), 3);
        assert_eq!(calm.omni[0].color, [1.0, 0.86, 0.62]);
        for (a, b) in hot.omni[0].color.iter().zip([1.0_f32, 0.35, 0.28]) { assert!((a - b).abs() < 1e-5); }
        assert_eq!(calm.omni[0].energy, 1.5);
        assert_eq!(hot.omni[0].energy, 1.0);
        assert_eq!(calm.ambient, [0.34, 0.36, 0.5]);
        let mid = lights(v, 50.0);
        assert!(mid.omni[0].color[1] < calm.omni[0].color[1] && mid.omni[0].color[1] > hot.omni[0].color[1]);
        // one dim lamp over each sill, in the shaft
        assert_eq!(calm.omni[1].pos.x, -1.1);
        assert_eq!(calm.omni[2].pos.x, 1.1);
        assert_eq!(calm.omni[1].pos.z, -2.9);
    }

    #[test]
    fn elise_stands_on_her_sill() {
        let p = elise_position(Side::R, 0.0);
        assert_eq!(p, vec3(1.1, 0.0, -5.6));
    }
}
