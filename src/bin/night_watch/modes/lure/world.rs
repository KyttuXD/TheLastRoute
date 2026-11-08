//! Dead Air's 3D world (ports of lure_office.gd and lure_world.gd).
//!
//! * The broadcast booth: a 6 x 5 m room lined with acoustic foam, a glass window onto a dark studio, a mixing console with the
//!   systems monitor, a boom microphone, a patch rack with LEDs and an ON AIR lamp. The two ventilation hatches in the side walls
//!   are where Xivvy comes in: each has a slatted cover that drops shut (the DOOR button beside it seals it) and a service duct
//!   behind it. The LIGHT buttons run the heater (left) and the fan (right).
//! * The radio station the cameras look into: eight enclosed rooms of different sizes (reception, two studios, the live room, the
//!   tape library, the transmitter room and two crawlspaces) with doorways in the shared walls, far from the booth.
//! * Props: bbmodels from `last_route/assets/props` where one fits (embedded), else built boxes in the mode palette.
//!
//! The geometry builders (`booth_parts`, `hall_groups`, the prop placement maths) are pure and tested without a GL context;
//! `Booth`, `Hall` and `PropMeshes` need one.
#![allow(dead_code)]

use super::map::*;
use crate::gfx::office::{put, O};
use crate::gfx::{Aabb, Cube, Gfx, Group, LightSet, Omni, Spot, StationDef, Textures, World, WorldBuilder};
use crate::hunters::bbmodel::{try_tex, Model};
use macroquad::models::{draw_mesh, Mesh};
use macroquad::prelude::*;
use std::collections::HashMap;

pub const SLAB_OPEN_Y: f32 = VENT_H + 1.3;
pub const SLAB_CLOSED_Y: f32 = VENT_H * 0.5;
/// The slab stops the player once it is this far down (Office::slab_blocks).
pub const SLAB_BLOCK_AT: f32 = 0.41;
pub const HALL_GUARD_X: f32 = 3.7;

const WALL: [f32; 3] = [0.6, 0.6, 0.75];

fn m3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn s3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn station(id: &str, prompt: &str, pos: [f32; 3], radius: f32) -> StationDef {
    StationDef { id: id.to_string(), prompt: prompt.to_string(), pos: vec3(pos[0], pos[1], pos[2]), focus: Vec3::ZERO, radius, disabled: false }
}

// ------------------------------------------------------------------------------------------------ the booth

pub struct BoothParts {
    pub groups: Vec<Group>,
    pub solids: Vec<Aabb>,
    pub stations: Vec<StationDef>,
}

/// The booth's geometry, stations and static blockers. Pure.
pub fn booth_parts() -> BoothParts {
    let mut groups: Vec<Group> = Vec::new();
    let mut solids: Vec<Aabb> = Vec::new();
    let mut stations: Vec<StationDef> = Vec::new();
    groups.push(shell());
    for side in [-1.0f32, 1.0] {
        vent_side(side, &mut groups, &mut solids, &mut stations);
    }
    window(&mut groups);
    console(&mut groups, &mut solids, &mut stations);
    rack_and_shelves(&mut groups, &mut solids);
    BoothParts { groups, solids, stations }
}

fn shell() -> Group {
    let mut g = Group::new("Shell", Vec3::ZERO);
    let w = HALF_W * 2.0 + 0.4;
    let d = HALF_D * 2.0 + 0.4;
    put(&mut g, "Floor", [w, 0.2, d], [0.0, -0.1, 0.0], O::new().k("fabric").t([0.55, 0.5, 0.65]).solid());
    put(&mut g, "Ceiling", [w, 0.2, d], [0.0, HEIGHT + 0.1, 0.0], O::new().t([0.45, 0.45, 0.55]));
    put(&mut g, "WallFront", [w, HEIGHT, 0.2], [0.0, HEIGHT * 0.5, HALF_D + 0.1], O::new().t(WALL).solid());
    // acoustic foam: rows of wedge blocks on the front wall
    for ix in 0..11 {
        for iy in 0..4 {
            let shade = 0.55 + 0.1 * ((ix + iy) % 2) as f32;
            put(&mut g, "FoamWedge", [0.5, 0.5, 0.12], [-2.5 + ix as f32 * 0.5, 0.7 + iy as f32 * 0.55, HALF_D - 0.06], O::new().k("fabric").t([shade, shade * 0.9, shade * 1.3]).ao(0.9));
        }
    }
    for z in [-HALF_D + 0.05, HALF_D - 0.05] {
        put(&mut g, "Baseboard", [HALF_W * 2.0, 0.2, 0.08], [0.0, 0.1, z], O::new().t([0.5, 0.5, 0.55]));
    }
    for x in [-1.4f32, 1.4] {
        put(&mut g, "CeilingFixture", [1.1, 0.08, 0.5], [x, HEIGHT - 0.04, 0.2], O::new().k("plastic"));
    }
    put(&mut g, "PipeBack", [HALF_W * 2.0, 0.12, 0.12], [0.0, HEIGHT - 0.2, HALF_D - 0.3], O::new().k("metal"));
    put(&mut g, "LedAlarm", [0.05, 0.05, 0.05], [2.7, 0.9, HALF_D - 0.1], O::new());
    g
}

fn vent_side(side: f32, groups: &mut Vec<Group>, solids: &mut Vec<Aabb>, stations: &mut Vec<StationDef>) {
    let tag = if side < 0.0 { "L" } else { "R" };
    let x = side * (HALF_W + 0.1);
    // the wall with the hatch cut out
    let mut wall = Group::new(&format!("Wall{tag}"), Vec3::ZERO);
    let piece = (HALF_D * 2.0 - VENT_W) * 0.5;
    put(&mut wall, &format!("Wall{tag}A"), [0.2, HEIGHT, piece], [x, HEIGHT * 0.5, -HALF_D + piece * 0.5], O::new().t(WALL).solid());
    put(&mut wall, &format!("Wall{tag}B"), [0.2, HEIGHT, piece], [x, HEIGHT * 0.5, HALF_D - piece * 0.5], O::new().t(WALL).solid());
    put(&mut wall, &format!("Wall{tag}Top"), [0.2, HEIGHT - VENT_H, VENT_W], [x, VENT_H + (HEIGHT - VENT_H) * 0.5, 0.0], O::new());
    groups.push(wall);
    let mut frame = Group::new(&format!("VentFrame{tag}"), Vec3::ZERO);
    for zz in [-VENT_W * 0.5 - 0.07, VENT_W * 0.5 + 0.07] {
        put(&mut frame, "FramePost", [0.34, VENT_H, 0.14], [x, VENT_H * 0.5, zz], O::new().t([0.5, 0.5, 0.55]));
    }
    put(&mut frame, "FrameHeader", [0.34, 0.16, VENT_W + 0.28], [x, VENT_H + 0.06, 0.0], O::new().t([0.5, 0.5, 0.55]));
    put(&mut frame, "Threshold", [0.4, 0.05, VENT_W], [x, 0.025, 0.0], O::new().k("metal").t([0.85, 0.85, 0.9]));
    groups.push(frame);
    // the cover: a slatted grille that drops shut (the group origin is its open position; the mode offsets it down)
    let mut slab = Group::new(&format!("Slab{tag}"), vec3(side * HALF_W, SLAB_OPEN_Y, 0.0));
    put(&mut slab, "SlabBody", [0.12, VENT_H, VENT_W - 0.04], [0.0, 0.0, 0.0], O::new().k("metal").t([0.7, 0.72, 0.78]));
    for i in 0..8 {
        put(&mut slab, "SlabSlat", [0.16, 0.07, VENT_W - 0.02], [0.0, -VENT_H * 0.5 + 0.2 + i as f32 * 0.27, 0.0], O::new().k("plastic").t([0.5, 0.5, 0.55]));
    }
    groups.push(slab);
    // the service duct behind it: long, narrow, dim
    let mut duct = Group::new(&format!("Duct{tag}"), Vec3::ZERO);
    let hx = side * (HALF_W + 3.25);
    put(&mut duct, &format!("Duct{tag}Floor"), [6.3, 0.2, 2.2], [hx, -0.1, 0.0], O::new().k("metal").t([0.7, 0.75, 0.85]).solid());
    put(&mut duct, &format!("Duct{tag}Ceiling"), [6.3, 0.2, 2.2], [hx, HEIGHT + 0.1, 0.0], O::new().t([0.5, 0.52, 0.6]));
    for zz in [-1.1f32, 1.1] {
        put(&mut duct, &format!("Duct{tag}Wall"), [6.3, HEIGHT, 0.2], [hx, HEIGHT * 0.5, zz], O::new().k("metal").t([0.65, 0.7, 0.8]).solid());
    }
    put(&mut duct, &format!("Duct{tag}End"), [0.2, HEIGHT, 2.2], [side * (HALF_W + 6.4), HEIGHT * 0.5, 0.0], O::new().k("metal").t([0.5, 0.55, 0.65]).solid());
    for i in 0..4 {
        put(&mut duct, "DuctRib", [0.12, HEIGHT - 0.1, 2.0], [side * (HALF_W + 0.8 + i as f32 * 1.6), HEIGHT * 0.5, 0.0], O::new().k("metal").t([0.8, 0.82, 0.9]));
    }
    groups.push(duct);
    // keeps the player out of the hatch (a guard collider, like the Godot body)
    solids.push(Aabb::from_centre(vec3(side * HALL_GUARD_X, HEIGHT * 0.5, 0.0), vec3(0.3, HEIGHT, 2.2)));
    // the panel beside the vent: SEAL (the cover) and a second button (the heater on the left, the fan on the right)
    let mut panel = Group::new(&format!("ControlPanel{tag}"), Vec3::ZERO);
    let px = side * (HALF_W - 0.06);
    put(&mut panel, "PanelHousing", [0.1, 0.78, 0.46], [px, 1.3, 1.55], O::new());
    put(&mut panel, "PanelFace", [0.05, 0.7, 0.4], [side * (HALF_W - 0.12), 1.3, 1.55], O::new().k("plastic"));
    put(&mut panel, "LabelPlate", [0.06, 0.1, 0.28], [side * (HALF_W - 0.14), 1.74, 1.55], O::new());
    let door = if side < 0.0 { "doorL" } else { "doorR" };
    let names: [&str; 2] = if side < 0.0 { ["SEAL LEFT VENT // SEAL / OPEN", "HEATER // ON / OFF"] } else { ["SEAL RIGHT VENT // SEAL / OPEN", "FAN // ON / OFF"] };
    for (i, (kind, y)) in [("Door", 1.47f32), ("Light", 1.15f32)].iter().enumerate() {
        let bx = side * (HALF_W - 0.15);
        put(&mut panel, &format!("Guard{kind}"), [0.05, 0.26, 0.3], [side * (HALF_W - 0.14), *y, 1.55], O::new().k("metal").t([0.9, 0.9, 0.95]));
        put(&mut panel, &format!("Button{tag}{kind}"), [0.06, 0.18, 0.22], [bx, *y, 1.55], O::new().k(if i == 0 { "red" } else { "light" }));
        stations.push(station(&format!("{door}_{}", kind.to_lowercase()), names[i], [bx, *y, 1.55], 0.35));
    }
    groups.push(panel);
}

fn window(groups: &mut Vec<Group>) {
    // the glass window onto the studio (north wall) and the studio behind it
    let mut wall = Group::new("WallBack", Vec3::ZERO);
    let gap = 3.2;
    let seg = (HALF_W * 2.0 + 0.4 - gap) * 0.5;
    for sgn in [-1.0f32, 1.0] {
        put(&mut wall, "WallBackPiece", [seg, HEIGHT, 0.2], [sgn * (gap * 0.5 + seg * 0.5), HEIGHT * 0.5, -HALF_D - 0.1], O::new().t(WALL).solid());
    }
    put(&mut wall, "WallBackSill", [gap, 1.0, 0.2], [0.0, 0.5, -HALF_D - 0.1], O::new().t(WALL).solid());
    put(&mut wall, "WallBackLintel", [gap, 0.5, 0.2], [0.0, HEIGHT - 0.25, -HALF_D - 0.1], O::new().t(WALL));
    groups.push(wall);
    let mut glass = Group::new("StudioWindow", vec3(0.0, 0.0, -HALF_D - 0.1));
    put(&mut glass, "WindowGlass", [gap, 1.5, 0.05], [0.0, 1.75, 0.0], O::new().k("glass").t([0.7, 0.85, 0.9]));
    for x in [-gap * 0.5, 0.0, gap * 0.5] {
        put(&mut glass, "WindowMullion", [0.1, 1.6, 0.1], [x, 1.75, 0.0], O::new().k("metal"));
    }
    groups.push(glass);
    let mut studio = Group::new("Studio", vec3(0.0, 0.0, -HALF_D - 2.2));
    put(&mut studio, "StudioFloor", [5.0, 0.2, 3.6], [0.0, -0.1, 0.0], O::new().k("fabric").t([0.4, 0.4, 0.5]));
    put(&mut studio, "StudioWall", [5.0, HEIGHT, 0.2], [0.0, HEIGHT * 0.5, -1.8], O::new().t([0.45, 0.45, 0.6]));
    for x in [-2.5f32, 2.5] {
        put(&mut studio, "StudioWall", [0.2, HEIGHT, 3.6], [x, HEIGHT * 0.5, 0.0], O::new().t([0.45, 0.45, 0.6]));
    }
    put(&mut studio, "StudioCeiling", [5.0, 0.2, 3.6], [0.0, HEIGHT + 0.1, 0.0], O::new().t([0.35, 0.35, 0.45]));
    put(&mut studio, "MicStandPole", [0.08, 1.5, 0.08], [-0.8, 0.75, -0.4], O::new().k("metal"));
    put(&mut studio, "MicHead", [0.14, 0.2, 0.14], [-0.8, 1.55, -0.4], O::new().k("plastic"));
    put(&mut studio, "MicBase", [0.4, 0.04, 0.4], [-0.8, 0.02, -0.4], O::new().k("metal"));
    put(&mut studio, "StoolSeat", [0.5, 0.5, 0.5], [1.0, 0.25, -0.6], O::new().k("fabric").t([1.2, 0.8, 0.8]));
    groups.push(studio);
    // the ON AIR lamp above the window
    let mut sign = Group::new("OnAirSign", vec3(0.0, 2.75, -HALF_D + 0.1));
    put(&mut sign, "SignPlate", [1.0, 0.3, 0.06], [0.0, 0.0, 0.0], O::new().k("plastic"));
    put(&mut sign, "SignOnAir", [0.84, 0.18, 0.03], [0.0, 0.0, 0.04], O::new().k("light").t([1.9, 0.18, 0.12]));
    groups.push(sign);
}

fn console(groups: &mut Vec<Group>, solids: &mut Vec<Aabb>, stations: &mut Vec<StationDef>) {
    let mut desk = Group::new("Console", vec3(0.0, 0.0, -1.6));
    put(&mut desk, "DeskTop", [2.6, 0.08, 0.95], [0.0, 0.76, 0.0], O::new());
    for x in [-1.2f32, 1.2] {
        put(&mut desk, "DeskLeg", [0.1, 0.76, 0.85], [x, 0.38, 0.0], O::new());
    }
    put(&mut desk, "DeskBackPanel", [2.2, 0.5, 0.04], [0.0, 0.45, -0.4], O::new().t([0.8, 0.8, 0.8]));
    groups.push(desk);
    solids.push(Aabb::from_centre(vec3(0.0, 0.45, -1.6), vec3(2.6, 0.9, 0.95)));
    // the mixing board: a sloped block with rows of faders and a level LED
    let mut mixer = Group::new("MixingBoard", vec3(-0.7, 0.8, -1.55));
    put(&mut mixer, "MixerBody", [1.2, 0.1, 0.55], [0.0, 0.05, 0.0], O::new().k("plastic"));
    for i in 0..8 {
        let x = -0.5 + i as f32 * 0.14;
        put(&mut mixer, "MixerFader", [0.05, 0.03, 0.22], [x, 0.12, 0.05], O::new().k("metal").t([1.2, 1.2, 1.25]));
        put(&mut mixer, "MixerKnob", [0.1, 0.02, 0.06], [x, 0.11, -0.15], O::new().k(if i % 3 == 0 { "red" } else { "plastic" }));
    }
    put(&mut mixer, "LedMixer", [0.04, 0.04, 0.04], [0.52, 0.12, -0.2], O::new());
    groups.push(mixer);
    // the systems monitor: a screen on an arm
    let mut monitor = Group::new("Monitor", vec3(0.7, 0.0, -2.0));
    put(&mut monitor, "MonitorCase", [0.96, 0.64, 0.09], [0.0, 1.2, 0.0], O::new());
    put(&mut monitor, "MonitorScreen", [0.8, 0.48, 0.02], [0.0, 1.2, 0.06], O::new());
    put(&mut monitor, "MonitorNeck", [0.14, 0.4, 0.12], [0.0, 0.95, -0.02], O::new());
    put(&mut monitor, "MonitorBase", [0.4, 0.04, 0.26], [0.0, 0.82, 0.0], O::new());
    groups.push(monitor);
    // the boom microphone and headphones
    let mut boom = Group::new("BoomMic", vec3(-1.2, 0.8, -1.9));
    put(&mut boom, "BoomPost", [0.06, 0.8, 0.06], [0.0, 0.4, 0.0], O::new().k("metal"));
    put(&mut boom, "BoomArm", [0.7, 0.05, 0.05], [0.3, 0.8, 0.0], O::new().k("metal"));
    put(&mut boom, "BoomCapsule", [0.12, 0.16, 0.12], [0.65, 0.76, 0.0], O::new().k("plastic"));
    groups.push(boom);
    let mut phones = Group::new("Headphones", vec3(1.1, 0.8, -1.4));
    put(&mut phones, "PhonesBand", [0.3, 0.04, 0.12], [0.0, 0.18, 0.0], O::new().k("plastic"));
    for x in [-0.15f32, 0.15] {
        put(&mut phones, "PhonesCup", [0.1, 0.14, 0.14], [x, 0.08, 0.0], O::new().k("fabric").t([0.8, 0.8, 0.9]));
    }
    groups.push(phones);
    // the desk lamp
    let mut lamp = Group::new("DeskLampProp", vec3(1.0, 0.8, -1.9));
    put(&mut lamp, "LampBase", [0.18, 0.04, 0.18], [0.0, 0.02, 0.0], O::new());
    put(&mut lamp, "LampArm", [0.04, 0.22, 0.04], [0.0, 0.15, 0.0], O::new());
    put(&mut lamp, "LampShade", [0.22, 0.14, 0.22], [0.1, 0.28, 0.0], O::new());
    groups.push(lamp);
    stations.push(station("monitor", "SYSTEMS MONITOR // CAMERAS, LURE, REBOOT", [0.7, 1.2, -1.95], 0.5));
    stations.push(station("lamp", "DESK LAMP", [1.0, 1.0, -1.85], 0.35));
    // the chair
    let mut chair = Group::new("Chair", vec3(0.0, 0.0, -0.9));
    put(&mut chair, "ChairSeat", [0.46, 0.1, 0.46], [0.0, 0.5, 0.0], O::new().solid());
    put(&mut chair, "ChairBack", [0.44, 0.5, 0.08], [0.0, 0.8, 0.2], O::new());
    put(&mut chair, "ChairColumn", [0.08, 0.3, 0.08], [0.0, 0.3, 0.0], O::new().k("metal"));
    put(&mut chair, "ChairBaseA", [0.5, 0.05, 0.08], [0.0, 0.1, 0.0], O::new().k("metal"));
    put(&mut chair, "ChairBaseB", [0.08, 0.05, 0.5], [0.0, 0.1, 0.0], O::new().k("metal"));
    groups.push(chair);
    let mut chair_station = station("chair", "SIT AT THE CONSOLE", [0.0, 0.0, -0.85], 0.5);
    chair_station.focus = vec3(0.0, 0.62, 0.0);
    stations.push(chair_station);
}

fn rack_and_shelves(groups: &mut Vec<Group>, solids: &mut Vec<Aabb>) {
    let mut rack = Group::new("PatchRack", vec3(-2.7, 0.0, -1.7));
    put(&mut rack, "RackBody", [0.5, 2.0, 0.7], [0.0, 1.0, 0.0], O::new().k("metal").t([0.6, 0.6, 0.68]));
    for i in 0..6 {
        put(&mut rack, "RackUnit", [0.04, 0.2, 0.6], [0.26, 0.3 + i as f32 * 0.28, 0.0], O::new().k("plastic").t([0.9, 0.9, 1.0]));
        put(&mut rack, "LedRack", [0.03, 0.04, 0.04], [0.29, 0.35 + i as f32 * 0.28, 0.2], O::new());
    }
    groups.push(rack);
    solids.push(Aabb::from_centre(vec3(-2.7, 1.0, -1.7), vec3(0.5, 2.0, 0.7)));
    let mut vinyl = Group::new("VinylShelf", vec3(2.7, 0.0, -1.5));
    put(&mut vinyl, "VinylShelfBody", [0.4, 1.6, 1.4], [0.0, 0.8, 0.0], O::new());
    let kinds = ["red", "fabric", "green", "label", "metal"];
    for i in 0..3 {
        put(&mut vinyl, "VinylShelfBoard", [0.42, 0.04, 1.4], [0.0, 0.3 + i as f32 * 0.5, 0.0], O::new());
        for (j, k) in kinds.iter().enumerate() {
            put(&mut vinyl, "VinylSleeve", [0.3, 0.3, 0.06], [0.0, 0.47 + i as f32 * 0.5, -0.5 + j as f32 * 0.25], O::new().k(*k).t([0.9, 0.9, 1.0]));
        }
    }
    groups.push(vinyl);
    solids.push(Aabb::from_centre(vec3(2.7, 0.8, -1.5), vec3(0.4, 1.6, 1.4)));
    let mut clock = Group::new("WallClock", vec3(-1.9, 2.2, HALF_D - 0.1));
    put(&mut clock, "ClockFrame", [0.5, 0.5, 0.05], [0.0, 0.0, 0.0], O::new().k("plastic"));
    put(&mut clock, "ClockFace", [0.42, 0.42, 0.03], [0.0, 0.0, -0.03], O::new());
    groups.push(clock);
    let mut poster = Group::new("Poster", vec3(1.8, 1.9, HALF_D - 0.1));
    put(&mut poster, "PosterFrame", [0.9, 1.2, 0.04], [0.0, 0.0, 0.0], O::new());
    put(&mut poster, "PosterPaper", [0.78, 1.08, 0.03], [0.0, 0.0, -0.03], O::new().k("fabric").t([1.2, 0.7, 0.8]));
    groups.push(poster);
}

/// Where the booth's lights are: the desk lamp (warm), and a faint glow in each duct.
pub const LAMP_POS: Vec3 = Vec3::new(1.1, 1.35, -1.6);

/// The booth's light colour follows the weather and the air: bluer when cold, warmer when hot, dimmer when the air is heavy.
pub fn booth_ambient(has_climate: bool, cold: f64, heat: f64, temp: f64, stress: f64) -> [f32; 3] {
    let base = [0.38f32, 0.40, 0.55];
    let cold_t = if has_climate { (cold / 100.0).max((60.0 - temp) / 60.0).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let heat_t = if has_climate { (heat / 100.0).max((temp - 90.0) / 40.0).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    let c = lerp3(lerp3(base, [0.3, 0.5, 0.95], cold_t * 0.6), [0.95, 0.5, 0.25], heat_t * 0.6);
    let e = 1.0 + (0.55 - 1.0) * (stress / 100.0).clamp(0.0, 1.0) as f32;
    s3(c, e)
}

/// The booth's lights: lamp omni, the two duct glows, an optional flashlight.
pub fn booth_lights(lamp_on: bool, ambient: [f32; 3], flashlight: Option<Spot>) -> LightSet {
    let mut omni = Vec::new();
    if lamp_on {
        omni.push(Omni { pos: LAMP_POS, color: [1.0, 0.78, 0.5], energy: 2.0, range: 5.0 });
    }
    for side in [-1.0f32, 1.0] {
        omni.push(Omni { pos: vec3(side * (HALF_W + 3.6), 2.4, 0.0), color: [0.5, 0.6, 0.95], energy: 0.14, range: 5.0 });
    }
    let mut spot = Vec::new();
    if let Some(f) = flashlight {
        spot.push(f);
    }
    LightSet { ambient, omni, spot, fog: None }
}

fn move_toward(current: f32, target: f32, delta: f32) -> f32 {
    if (target - current).abs() <= delta {
        target
    } else {
        current + (target - current).signum() * delta
    }
}

/// The slab group's model offset for a progress 0..1 (its origin is the open position).
pub fn slab_offset(progress: f32) -> Vec3 {
    vec3(0.0, (SLAB_CLOSED_Y - SLAB_OPEN_Y) * progress.clamp(0.0, 1.0), 0.0)
}

pub struct Booth {
    pub world: World,
    pub stations: Vec<StationDef>,
    pub slab: [f32; 2],
    extra_solids: Vec<Aabb>,
    pub props: PropMeshes,
    lamp_on: bool,
}

impl Booth {
    /// Needs a live GL context.
    pub fn new(tex: &Textures, kit: &PropKit) -> Booth {
        let parts = booth_parts();
        let mut wb = WorldBuilder::new();
        for g in parts.groups {
            wb.add_group(g);
        }
        let mut b = Booth { world: wb.bake(tex), stations: parts.stations, slab: [0.0, 0.0], extra_solids: parts.solids, props: kit.build(Area::Booth), lamp_on: true };
        b.set_button("doorL_door", false);
        b.set_button("doorR_door", false);
        b.set_button("doorL_light", false);
        b.set_button("doorR_light", false);
        b.set_lamp(true);
        b.apply_slabs();
        b
    }

    fn apply_slabs(&mut self) {
        self.world.set_offset("SlabL", slab_offset(self.slab[0]));
        self.world.set_offset("SlabR", slab_offset(self.slab[1]));
    }

    /// The slabs drop at 7 per second.
    pub fn animate(&mut self, shut: [bool; 2], dt_s: f32) {
        for i in 0..2 {
            let target = if shut[i] { 1.0 } else { 0.0 };
            self.slab[i] = move_toward(self.slab[i], target, dt_s * 7.0);
        }
        self.apply_slabs();
    }

    pub fn snap_slabs(&mut self, shut: [bool; 2]) {
        for i in 0..2 {
            self.slab[i] = if shut[i] { 1.0 } else { 0.0 };
        }
        self.apply_slabs();
    }

    /// A button's colour: the door buttons are green when open and red when sealed, the light buttons glow when on.
    pub fn set_button(&mut self, id: &str, on: bool) {
        let tag = if id.starts_with("doorL") { "L" } else { "R" };
        let group = format!("ControlPanel{tag}");
        if id.ends_with("_door") {
            self.world.set_material(&group, &format!("Button{tag}Door"), if on { "red" } else { "green" }, [1.0, 1.0, 1.0], true);
        } else {
            self.world.set_material(&group, &format!("Button{tag}Light"), if on { "light" } else { "plastic" }, [1.0, 1.0, 1.0], on);
        }
    }

    pub fn set_lamp(&mut self, on: bool) {
        self.lamp_on = on;
        if on {
            self.world.set_material("DeskLampProp", "LampShade", "light", [1.0, 1.0, 1.0], true);
        } else {
            self.world.set_material("DeskLampProp", "LampShade", "light", [0.35, 0.3, 0.25], false);
        }
    }

    /// The static solids plus a box for every slab that is down far enough to block.
    pub fn all_solids(&self) -> Vec<Aabb> {
        let mut out = self.world.solids.clone();
        out.extend(self.extra_solids.iter().copied());
        for (i, side) in [-1.0f32, 1.0].iter().enumerate() {
            if self.slab[i] > SLAB_BLOCK_AT {
                let y = SLAB_OPEN_Y + (SLAB_CLOSED_Y - SLAB_OPEN_Y) * self.slab[i];
                out.push(Aabb::from_centre(vec3(side * HALF_W, y, 0.0), vec3(0.14, VENT_H, VENT_W - 0.04)));
            }
        }
        out
    }

    pub fn slab_blocks(&self, door: &str) -> bool {
        let i = if door == "doorL" || door == "officeL" { 0 } else { 1 };
        self.slab[i] > SLAB_BLOCK_AT
    }

    /// Draw the booth and its props with these lights.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        self.props.draw(gfx, lights, cam_pos);
    }
}

// ------------------------------------------------------------------------------------------------ the station

const TINT_RECEPTION: [f32; 3] = [1.3, 1.05, 0.8];
const TINT_STUDIO: [f32; 3] = [0.9, 0.85, 1.3];
const TINT_LIVE: [f32; 3] = [1.2, 0.85, 0.95];
const TINT_TAPES: [f32; 3] = [1.2, 1.15, 0.8];
const TINT_TX: [f32; 3] = [0.85, 1.05, 1.15];
const TINT_CRAWL: [f32; 3] = [0.85, 0.95, 1.05];

pub fn theme_tint(theme: &str) -> [f32; 3] {
    match theme {
        "reception" => TINT_RECEPTION,
        "studio" => TINT_STUDIO,
        "live" => TINT_LIVE,
        "tapes" => TINT_TAPES,
        "transmitter" => TINT_TX,
        _ => TINT_CRAWL,
    }
}

/// The group name of a station room's part: "hall.lobby.shell".
pub fn hall_group(sim: &str, part: &str) -> String {
    format!("hall.{sim}.{part}")
}

/// One wall: slabs either side of each doorway and a lintel over it. The wall sits just inside the room, so two neighbours' walls
/// stand side by side. `doors` are door centres along the wall in the room's local coordinates.
fn hall_wall(g: &mut Group, side: char, hw: f32, hd: f32, doors: &[f32], tint: [f32; 3]) {
    let m = map();
    let along_x = side == 'N' || side == 'S';
    let length = if along_x { hw * 2.0 } else { hd * 2.0 };
    let half = length * 0.5;
    let fixed = if along_x { (hd - 0.1) * if side == 'N' { -1.0 } else { 1.0 } } else { (hw - 0.1) * if side == 'W' { -1.0 } else { 1.0 } };
    let mut cuts: Vec<f32> = doors.to_vec();
    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let name = format!("Wall{side}");
    let mut cursor = -half;
    let piece = |g: &mut Group, a: f32, b: f32| {
        if b - a < 0.05 {
            return;
        }
        let mid = (a + b) * 0.5;
        let (size, pos) = if along_x { ([b - a, m.height, 0.2], [mid, m.height * 0.5, fixed]) } else { ([0.2, m.height, b - a], [fixed, m.height * 0.5, mid]) };
        put(g, &name, size, pos, O::new().t(tint).solid());
    };
    for dc in cuts {
        piece(g, cursor, dc - m.door_gap * 0.5);
        let h = m.height - m.door_top;
        let (size, pos) = if along_x { ([m.door_gap, h, 0.2], [dc, m.door_top + h * 0.5, fixed]) } else { ([0.2, h, m.door_gap], [fixed, m.door_top + h * 0.5, dc]) };
        put(g, &format!("{name}Lintel"), size, pos, O::new().t(m3(tint, [0.9, 0.9, 0.9])));
        cursor = dc + m.door_gap * 0.5;
    }
    piece(g, cursor, half);
}

/// Every group of the station: per room a shell and a themed set of props, named `hall.<sim>.<part>`, with the room centre as the
/// group origin. A part a bbmodel replaces (data/lure/map.json "replaces") is left out. Pure.
pub fn hall_groups() -> Vec<Group> {
    let m = map();
    let mut out = Vec::new();
    for r in &m.rooms {
        let c = m.centre(&r.id);
        let (hw, hd) = (r.w() * 0.5, r.d() * 0.5);
        let tint = theme_tint(&r.theme);
        let height = if r.theme == "crawl" { m.crawl_height } else { m.height };
        let sim = r.sim.as_str();
        let mut shell = Group::new(&hall_group(sim, "shell"), c);
        put(&mut shell, "Floor", [r.w(), 0.2, r.d()], [0.0, -0.1, 0.0], O::new().t(m3(tint, [0.95, 0.95, 0.95])));
        put(&mut shell, "Ceiling", [r.w(), 0.2, r.d()], [0.0, height + 0.1, 0.0], O::new().t(m3(tint, [0.6, 0.6, 0.6])));
        let doors = m.doors_of(&r.id);
        hall_wall(&mut shell, 'N', hw, hd, &doors.n, tint);
        hall_wall(&mut shell, 'S', hw, hd, &doors.s, tint);
        hall_wall(&mut shell, 'W', hw, hd, &doors.w, m3(tint, [1.05, 1.05, 1.05]));
        hall_wall(&mut shell, 'E', hw, hd, &doors.e, m3(tint, [1.05, 1.05, 1.05]));
        put(&mut shell, "PipeRoom", [r.w() - 0.6, 0.12, 0.12], [0.0, height - 0.22, -hd + 0.3], O::new().k("metal"));
        put(&mut shell, "CeilingPanel", [1.4, 0.05, 1.4], [0.0, height - 0.04, 0.0], O::new());
        put(&mut shell, "LedSmoke", [0.05, 0.05, 0.05], [-hw + 0.3, height - 0.5, hd - 0.4], O::new());
        put(&mut shell, "FloorStripe", [r.w(), 0.02, 0.3], [0.0, 0.01, 0.8], O::new().k("label").t([0.8, 0.8, 0.8]));
        out.push(shell);
        let mut details = Group::new(&hall_group(sim, "details"), c);
        put(&mut details, "VentGrille", [0.7, 0.4, 0.06], [hw - 1.0, 2.4, -hd + 0.14], O::new().k("metal"));
        out.push(details);
        out.push(camera_group(sim, c));
        match r.theme.as_str() {
            "reception" => reception(&mut out, sim, c, hw, hd),
            "studio" => studio(&mut out, sim, c, hw, hd, r.id == "studio_a"),
            "live" => live(&mut out, sim, c, hw, hd),
            "tapes" => tapes(&mut out, sim, c, hw, hd),
            "transmitter" => transmitter(&mut out, sim, c, hw, hd),
            _ => crawl(&mut out, sim, c, hw, hd),
        }
    }
    out
}

/// The security camera on the wall: a housing on a bracket, turned toward what it looks at, with a red light. It hangs exactly
/// where the feed's camera is (`LureMap::camera_transform`), so the feed is what this camera sees.
fn camera_group(sim: &str, centre: Vec3) -> Group {
    let m = map();
    let (from, to) = m.camera_transform(sim);
    let local = from - centre;
    let d = to - from;
    let yaw = d.x.atan2(d.z);
    let dir = vec3(yaw.sin(), 0.0, yaw.cos());
    let mut g = Group::new(&hall_group(sim, "camera"), centre);
    g.add(Cube::new("CamBracket", vec3(0.06, 0.3, 0.06), local + vec3(0.0, 0.2, 0.0)).kind("metal"));
    g.add(Cube::new("CamBody", vec3(0.2, 0.18, 0.42), local).kind("metal").tint([0.55, 0.57, 0.62]).rot_y(yaw));
    g.add(Cube::new("CamLens", vec3(0.1, 0.1, 0.06), local + dir * 0.23).kind("glass").tint([0.3, 0.35, 0.4]).rot_y(yaw));
    g.add(Cube::new("LedRec", vec3(0.04, 0.04, 0.04), local + vec3(0.0, 0.11, 0.0) - dir * 0.1).kind("red"));
    g
}

fn part(sim: &str, name: &str, c: Vec3, local: [f32; 3]) -> Group {
    Group::new(&hall_group(sim, name), c + vec3(local[0], local[1], local[2]))
}

fn reception(out: &mut Vec<Group>, sim: &str, c: Vec3, hw: f32, hd: f32) {
    let m = map();
    let mut desk = part(sim, "ReceptionDesk", c, [0.0, 0.0, -hd + 1.0]);
    put(&mut desk, "FrontBody", [3.2, 1.0, 0.8], [0.0, 0.5, 0.0], O::new().k("wood").t([1.2, 0.9, 0.8]));
    put(&mut desk, "FrontTop", [3.3, 0.06, 0.9], [0.0, 1.03, 0.0], O::new().k("metal").t([1.2, 1.2, 1.25]));
    put(&mut desk, "SwitchboardBody", [0.4, 0.25, 0.3], [0.8, 1.19, 0.0], O::new().k("plastic"));
    out.push(desk);
    let mut sign = part(sim, "StationSign", c, [0.0, 2.5, -hd + 0.2]);
    put(&mut sign, "SignPlate", [3.4, 0.5, 0.06], [0.0, 0.0, 0.0], O::new().k("plastic"));
    put(&mut sign, "SignOnAirLogo", [3.0, 0.3, 0.03], [0.0, 0.0, 0.04], O::new().t([1.0, 0.45, 0.35]));
    out.push(sign);
    if !m.replaced(sim, "Sofa") {
        let mut sofa = part(sim, "Sofa", c, [hw - 1.2, 0.0, 0.8]);
        put(&mut sofa, "SofaSeat", [0.9, 0.45, 2.2], [0.0, 0.225, 0.0], O::new().k("fabric").t([0.9, 0.6, 0.7]));
        put(&mut sofa, "SofaBack", [0.2, 0.8, 2.2], [0.4, 0.65, 0.0], O::new().k("fabric").t([0.8, 0.5, 0.65]));
        out.push(sofa);
    }
    let mut posters = part(sim, "Posters", c, [-hw + 0.15, 1.7, 0.4]);
    for i in 0..3 {
        put(&mut posters, "PosterPage", [0.04, 0.8, 0.55], [0.0, 0.0, -1.2 + i as f32 * 1.2], O::new().k("fabric").t([1.0 + 0.2 * i as f32, 0.8, 1.3 - 0.2 * i as f32]));
    }
    out.push(posters);
}

fn studio(out: &mut Vec<Group>, sim: &str, c: Vec3, hw: f32, hd: f32, tall_left: bool) {
    let m = map();
    let mut foam = part(sim, "Foam", c, [0.0, 0.0, 0.0]);
    let wall_x = if tall_left { -hw + 0.18 } else { hw - 0.18 };
    for iz in 0..6 {
        for iy in 0..3 {
            let shade = 0.55 + 0.1 * ((iz + iy) % 2) as f32;
            put(&mut foam, "FoamWedge", [0.12, 0.5, 0.5], [wall_x, 0.8 + iy as f32 * 0.55, -hd + 1.0 + iz as f32 * 1.5], O::new().k("fabric").t([shade, shade * 0.9, shade * 1.3]).ao(0.9));
        }
    }
    out.push(foam);
    if !m.replaced(sim, "MicStand") {
        let mut mic = part(sim, "MicStand", c, [0.0, 0.0, -hd * 0.3]);
        put(&mut mic, "MicBase", [0.4, 0.04, 0.4], [0.0, 0.02, 0.0], O::new().k("metal"));
        put(&mut mic, "MicPole", [0.05, 1.5, 0.05], [0.0, 0.75, 0.0], O::new().k("metal"));
        put(&mut mic, "MicHead", [0.16, 0.22, 0.16], [0.0, 1.55, 0.1], O::new().k("plastic"));
        out.push(mic);
    }
    let mut desk = part(sim, "GuestDesk", c, [0.0, 0.0, hd * 0.2]);
    put(&mut desk, "DeskTop", [2.4, 0.08, 0.9], [0.0, 0.75, 0.0], O::new());
    for x in [-1.0f32, 1.0] {
        put(&mut desk, "DeskLeg", [0.1, 0.75, 0.8], [x, 0.375, 0.0], O::new());
    }
    for x in [-0.8f32, 0.8] {
        put(&mut desk, "StoolSeat", [0.45, 0.06, 0.45], [x, 0.45, 0.9], O::new().k("fabric").t([1.2, 0.8, 0.8]));
        put(&mut desk, "StoolLeg", [0.06, 0.45, 0.06], [x, 0.22, 0.9], O::new().k("metal"));
    }
    out.push(desk);
    let mut win = part(sim, "ControlWindow", c, [0.0, 1.7, -hd + 0.15]);
    put(&mut win, "WindowGlass", [2.8, 1.0, 0.06], [0.0, 0.0, 0.0], O::new().k("glass"));
    put(&mut win, "WindowFrame", [3.0, 1.2, 0.04], [0.0, 0.0, -0.04], O::new());
    out.push(win);
}

fn live(out: &mut Vec<Group>, sim: &str, c: Vec3, hw: f32, hd: f32) {
    let m = map();
    if !m.replaced(sim, "DrumKit") {
        let mut kit = part(sim, "DrumKit", c, [-1.4, 0.0, -hd + 1.4]);
        put(&mut kit, "DrumKick", [0.7, 0.45, 0.7], [0.0, 0.225, 0.0], O::new().k("red"));
        for x in [-0.7f32, 0.7] {
            put(&mut kit, "DrumTom", [0.4, 0.25, 0.4], [x, 0.8, 0.2], O::new().k("plastic").t([1.2, 1.2, 1.3]));
        }
        put(&mut kit, "CymbalDisc", [0.5, 0.04, 0.5], [1.0, 1.2, 0.0], O::new().k("metal").t([1.4, 1.3, 0.8]));
        put(&mut kit, "CymbalStand", [0.04, 1.2, 0.04], [1.0, 0.6, 0.0], O::new().k("metal"));
        out.push(kit);
    }
    if !m.replaced(sim, "Piano") {
        let mut piano = part(sim, "Piano", c, [1.8, 0.0, -hd + 1.0]);
        put(&mut piano, "PianoBody", [1.8, 0.9, 0.7], [0.0, 0.45, 0.0], O::new().t([0.45, 0.4, 0.5]));
        put(&mut piano, "PianoKeys", [1.7, 0.05, 0.3], [0.0, 0.92, 0.3], O::new().k("label").t([1.3, 1.3, 1.2]));
        out.push(piano);
    }
    for (x, tag) in [(-hw + 0.6, "L"), (hw - 0.6, "R")] {
        if m.replaced(sim, &format!("Amp{tag}")) {
            continue;
        }
        let mut amp = part(sim, &format!("Amp{tag}"), c, [x, 0.0, 0.3]);
        put(&mut amp, "AmpBody", [0.6, 0.9, 0.5], [0.0, 0.45, 0.0], O::new().k("plastic").t([0.8, 0.8, 0.9]));
        put(&mut amp, "AmpGrille", [0.4, 0.4, 0.04], [0.0, 0.45, 0.27], O::new().k("fabric").t([0.6, 0.6, 0.7]));
        out.push(amp);
    }
    let mut rug = part(sim, "Rug", c, [0.0, 0.01, hd - 1.6]);
    put(&mut rug, "RugMat", [3.4, 0.03, 1.8], [0.0, 0.0, 0.0], O::new().k("fabric").t([0.9, 0.6, 0.7]).ao(1.0));
    out.push(rug);
}

fn tapes(out: &mut Vec<Group>, sim: &str, c: Vec3, hw: f32, hd: f32) {
    let kinds = ["metal", "red", "plastic", "label"];
    for i in 0..3 {
        let mut shelf = part(sim, &format!("TapeShelf{i}"), c, [-hw + 1.2 + i as f32 * 1.7, 0.0, -hd + 1.0]);
        put(&mut shelf, "ShelfBody", [1.4, 2.2, 0.5], [0.0, 1.1, 0.0], O::new());
        for y in [0.5f32, 1.1, 1.7] {
            put(&mut shelf, "ShelfBoard", [1.42, 0.05, 0.52], [0.0, y, 0.0], O::new());
            for (j, k) in kinds.iter().enumerate() {
                put(&mut shelf, "TapeReel", [0.26, 0.26, 0.3], [-0.5 + j as f32 * 0.34, y + 0.17, 0.0], O::new().k(*k).t([0.9, 0.9, 1.0]));
            }
        }
        out.push(shelf);
    }
    let mut table = part(sim, "ListeningTable", c, [0.0, 0.0, hd - 1.4]);
    put(&mut table, "TableTop", [2.2, 0.08, 0.9], [0.0, 0.75, 0.0], O::new());
    for x in [-1.0f32, 1.0] {
        put(&mut table, "TableLeg", [0.1, 0.75, 0.8], [x, 0.375, 0.0], O::new());
    }
    put(&mut table, "ReelDeck", [0.6, 0.18, 0.4], [0.0, 0.88, 0.0], O::new().k("plastic"));
    out.push(table);
}

fn transmitter(out: &mut Vec<Group>, sim: &str, c: Vec3, hw: f32, hd: f32) {
    for i in 0..3 {
        let mut rack = part(sim, &format!("TxRack{i}"), c, [-hw + 1.0 + i as f32 * 1.3, 0.0, -hd + 0.8]);
        put(&mut rack, "RackBody", [1.0, 2.2, 0.8], [0.0, 1.1, 0.0], O::new().k("metal").t([0.6, 0.6, 0.68]));
        for j in 0..5 {
            put(&mut rack, "RackUnit", [1.02, 0.18, 0.04], [0.0, 0.3 + j as f32 * 0.4, 0.42], O::new().k("plastic").t([0.9, 0.9, 1.0]));
            put(&mut rack, "LedRack", [0.05, 0.05, 0.04], [0.4, 0.34 + j as f32 * 0.4, 0.45], O::new());
        }
        out.push(rack);
    }
    let mut coil = part(sim, "Transformer", c, [hw - 1.4, 0.0, 0.4]);
    put(&mut coil, "TransformerBody", [1.2, 1.4, 1.2], [0.0, 0.7, 0.0], O::new().k("metal").t([0.7, 0.75, 0.8]));
    for i in 0..4 {
        put(&mut coil, "InsulatorPost", [0.14, 0.4, 0.14], [-0.4 + i as f32 * 0.27, 1.6, 0.0], O::new().k("glass"));
    }
    out.push(coil);
    let mut mast = part(sim, "AntennaMast", c, [hw - 0.7, 0.0, -hd + 0.7]);
    put(&mut mast, "MastPipe", [0.12, map().height, 0.12], [0.0, map().height * 0.5, 0.0], O::new().k("metal"));
    put(&mut mast, "MastBase", [0.5, 0.2, 0.5], [0.0, 0.1, 0.0], O::new().k("metal"));
    out.push(mast);
}

fn crawl(out: &mut Vec<Group>, sim: &str, c: Vec3, hw: f32, hd: f32) {
    let mut duct = part(sim, "Duct", c, [0.0, 0.0, 0.0]);
    put(&mut duct, "DuctTube", [hw * 2.0 - 0.8, 1.1, 1.4], [0.0, 0.55, 0.0], O::new().k("metal").t([0.9, 0.95, 1.05]));
    for x in [-1.2f32, 0.0, 1.2] {
        put(&mut duct, "DuctBand", [0.14, 1.2, 1.5], [if hw > 1.5 { x } else { 0.0 }, 0.55, 0.0], O::new().k("metal").t([0.7, 0.7, 0.75]));
    }
    put(&mut duct, "DuctHatch", [0.5, 0.6, 0.06], [0.0, 0.55, 0.74], O::new().k("plastic"));
    out.push(duct);
    let mut riser = part(sim, "DuctRiser", c, [hw - 0.6, 0.0, -hd + 0.6]);
    put(&mut riser, "RiserTube", [0.7, 2.4, 0.7], [0.0, 1.2, 0.0], O::new().k("metal").t([0.8, 0.85, 0.95]));
    out.push(riser);
}

/// The lights of a camera feed: a flat bright ambient (the shared feed look, a little dimmer) and a ceiling light in the room.
pub fn feed_lights(sim: &str) -> LightSet {
    let m = map();
    let c = m.centre_of_sim(sim);
    let h = if m.is_vent(sim) { m.crawl_height } else { m.height };
    let base = LightSet::feed();
    LightSet {
        ambient: s3(base.ambient, 0.62),
        omni: vec![Omni { pos: c + vec3(0.0, h - 0.6, 0.0), color: [1.0, 0.92, 0.8], energy: 1.2, range: 10.0 }],
        spot: Vec::new(),
        fog: None,
    }
}

pub struct Hall {
    pub world: World,
    pub props: PropMeshes,
}

impl Hall {
    /// Needs a live GL context.
    pub fn new(tex: &Textures, kit: &PropKit) -> Hall {
        let mut wb = WorldBuilder::new();
        for g in hall_groups() {
            wb.add_group(g);
        }
        Hall { world: wb.bake(tex), props: kit.build(Area::Hall) }
    }

    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        self.props.draw(gfx, lights, cam_pos);
    }
}

// ------------------------------------------------------------------------------------------------ props (bbmodels)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Area {
    Booth,
    Hall,
}

const ATLAS: &[u8] = include_bytes!("../../../last_route/assets/props/blocky_atlas.png");

macro_rules! prop_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../../../last_route/assets/props/", $name, ".bbmodel")))),*]
    };
}

/// The props this mode places (all are in last_route/assets/props).
pub const PROP_FILES: &[(&str, &str)] = prop_files!(
    "micStand", "paSpeakerPole", "keyboardStand", "cableCoil", "cableRun", "drumKit", "uprightPianoMusic", "ampStack", "stageSpeaker",
    "loungeCouch", "waitingChairs", "pottedPlant", "magazineRack", "floorLamp", "bedroomWallClock", "filingCabinet", "officeFileBoxes",
    "supplyCrates", "serverRack", "breakerCabinet", "rackGear", "deskMic",
);

/// Where a model's origin goes so its footprint centre and its base land on `pos` after a turn of `yaw`: the model's bounds are in
/// bbmodel units, `scale` takes them to metres.
pub fn anchored(pos: Vec3, yaw: f32, min: Vec3, max: Vec3, scale: f32) -> Vec3 {
    let anchor = vec3((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5) * scale;
    pos - Mat3::from_rotation_y(yaw) * anchor
}

/// The parsed prop models (needs the atlas texture, so GL).
pub struct PropKit {
    models: HashMap<&'static str, Model>,
}

impl PropKit {
    pub fn new() -> PropKit {
        let atlas = try_tex(ATLAS);
        let mut models = HashMap::new();
        for (name, text) in PROP_FILES {
            if let Some(tex) = atlas.clone() {
                if let Some(m) = Model::parse(text, Some(tex), &|_: &str| None::<Vec<u8>>, None) {
                    models.insert(*name, m);
                }
            }
        }
        PropKit { models }
    }

    pub fn has(&self, model: &str) -> bool {
        self.models.contains_key(model)
    }

    /// Bake every placement of an area into a few meshes.
    pub fn build(&self, area: Area) -> PropMeshes {
        let m = map();
        let mut meshes: Vec<Mesh> = Vec::new();
        let mut verts: Vec<Vertex> = Vec::new();
        let mut idx: Vec<u16> = Vec::new();
        let mut texture: Option<Texture2D> = None;
        for p in &m.props {
            if (p.room == "booth") != (area == Area::Booth) {
                continue;
            }
            let Some(model) = self.models.get(p.model.as_str()) else { continue };
            let origin = if area == Area::Booth { Vec3::ZERO } else { m.centre_of_sim(&p.room) };
            let pos = origin + vec3(p.x, p.y, p.z);
            let at = anchored(pos, p.yaw(), model.min, model.max, model.scale);
            let (mut tv, mut ti) = (Vec::new(), Vec::new());
            model.build_scaled(&mut tv, &mut ti, at, p.yaw(), Vec3::splat(model.scale), 0.0, 0.0, WHITE);
            if verts.len() + tv.len() > 60_000 {
                meshes.push(Mesh { vertices: std::mem::take(&mut verts), indices: std::mem::take(&mut idx), texture: texture.clone() });
            }
            let base = verts.len() as u16;
            verts.extend(tv);
            idx.extend(ti.into_iter().map(|i| i + base));
            texture = Some(model.texture.clone());
        }
        if !verts.is_empty() {
            meshes.push(Mesh { vertices: verts, indices: idx, texture });
        }
        PropMeshes { meshes }
    }
}

pub struct PropMeshes {
    meshes: Vec<Mesh>,
}

impl PropMeshes {
    pub fn empty() -> PropMeshes {
        PropMeshes { meshes: Vec::new() }
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

    pub fn triangles(&self) -> usize {
        self.meshes.iter().map(|m| m.indices.len() / 3).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(groups: &[Group]) -> Vec<String> {
        groups.iter().map(|g| g.name.clone()).collect()
    }

    #[test]
    fn the_booth_has_every_part_the_game_drives() {
        let p = booth_parts();
        let n = names(&p.groups);
        for want in ["Shell", "SlabL", "SlabR", "ControlPanelL", "ControlPanelR", "DeskLampProp", "StudioWindow", "Console", "Monitor", "PatchRack", "VinylShelf", "DuctL", "DuctR", "OnAirSign"] {
            assert!(n.iter().any(|x| x == want), "{want}");
        }
        let ids: Vec<&str> = p.stations.iter().map(|s| s.id.as_str()).collect();
        for want in ["monitor", "doorL_door", "doorR_door", "doorL_light", "doorR_light", "lamp", "chair"] {
            assert!(ids.contains(&want), "{want}");
        }
        let prompt = |id: &str| p.stations.iter().find(|s| s.id == id).map(|s| s.prompt.clone()).unwrap_or_default();
        assert_eq!(prompt("doorL_door"), "SEAL LEFT VENT // SEAL / OPEN");
        assert_eq!(prompt("doorR_light"), "FAN // ON / OFF");
        assert_eq!(prompt("monitor"), "SYSTEMS MONITOR // CAMERAS, LURE, REBOOT");
        assert!(!p.solids.is_empty());
    }

    #[test]
    fn the_buttons_and_the_lamp_shade_can_be_re_skinned_by_name() {
        let p = booth_parts();
        let has = |g: &str, c: &str| p.groups.iter().find(|x| x.name == g).map(|x| x.cubes.iter().any(|q| q.name == c)).unwrap_or(false);
        assert!(has("ControlPanelL", "ButtonLDoor") && has("ControlPanelL", "ButtonLLight"));
        assert!(has("ControlPanelR", "ButtonRDoor") && has("ControlPanelR", "ButtonRLight"));
        assert!(has("DeskLampProp", "LampShade"));
    }

    #[test]
    fn the_slabs_start_open_above_the_hatch_and_drop_to_the_middle() {
        let p = booth_parts();
        let slab = p.groups.iter().find(|g| g.name == "SlabL").expect("slab");
        assert_eq!(slab.origin, vec3(-3.0, SLAB_OPEN_Y, 0.0));
        assert!((SLAB_OPEN_Y - 3.5).abs() < 1e-6 && (SLAB_CLOSED_Y - 1.1).abs() < 1e-6);
        assert_eq!(slab_offset(0.0), Vec3::ZERO);
        assert!((slab_offset(1.0).y + 2.4).abs() < 1e-6);
        assert_eq!(slab_offset(2.0), slab_offset(1.0));
        assert_eq!(slab.cubes.iter().filter(|c| c.name == "SlabSlat").count(), 8);
    }

    #[test]
    fn the_hatch_is_cut_into_the_side_walls() {
        let p = booth_parts();
        let wall = p.groups.iter().find(|g| g.name == "WallL").expect("wall");
        let piece = (HALF_D * 2.0 - VENT_W) * 0.5;
        let a = wall.cubes.iter().find(|c| c.name == "WallLA").expect("A");
        assert!((a.size.z - piece).abs() < 1e-6);
        let top = wall.cubes.iter().find(|c| c.name == "WallLTop").expect("top");
        assert!((top.size.y - (HEIGHT - VENT_H)).abs() < 1e-6 && (top.size.z - VENT_W).abs() < 1e-6);
        assert!((a.pos.x + 3.1).abs() < 1e-6);
    }

    #[test]
    fn the_light_follows_the_weather_and_the_air() {
        let calm = booth_ambient(false, 0.0, 0.0, 70.0, 0.0);
        assert_eq!(calm, [0.38, 0.40, 0.55]);
        let cold = booth_ambient(true, 100.0, 0.0, 30.0, 0.0);
        assert!(cold[2] > calm[2] && cold[0] < calm[0], "bluer: {cold:?}");
        let hot = booth_ambient(true, 0.0, 100.0, 120.0, 0.0);
        assert!(hot[0] > calm[0] && hot[2] < calm[2], "warmer: {hot:?}");
        let heavy = booth_ambient(false, 0.0, 0.0, 70.0, 100.0);
        assert!((heavy[0] - 0.38 * 0.55).abs() < 1e-6, "dimmer: {heavy:?}");
        let l = booth_lights(true, calm, None);
        assert_eq!(l.omni.len(), 3);
        assert_eq!(booth_lights(false, calm, None).omni.len(), 2);
    }

    #[test]
    fn every_room_has_a_shell_with_doorways_and_props() {
        let g = hall_groups();
        let n = names(&g);
        for sim in super::super::data::ROOMS {
            assert!(n.contains(&hall_group(sim, "shell")), "{sim}");
        }
        assert!(n.contains(&hall_group("lobby", "camera")));
        for want in ["hall.lobby.ReceptionDesk", "hall.ballpit.Rug", "hall.prize.TapeShelf2", "hall.storage.TxRack2", "hall.ventl.Duct", "hall.arcade.GuestDesk", "hall.party.Foam"] {
            assert!(n.iter().any(|x| x == want), "{want}");
        }
        // the shell of the reception: two walls with a doorway cut (west and east), a lintel over each
        let shell = g.iter().find(|x| x.name == "hall.lobby.shell").expect("shell");
        let lintels = shell.cubes.iter().filter(|c| c.name.ends_with("Lintel")).count();
        assert_eq!(lintels, 2);
        let storage = g.iter().find(|x| x.name == "hall.storage.shell").expect("shell");
        assert_eq!(storage.cubes.iter().filter(|c| c.name.ends_with("Lintel")).count(), 4);
    }

    #[test]
    fn each_camera_hangs_where_its_feed_looks_from() {
        let m = map();
        for sim in super::super::data::ROOMS {
            let g = hall_groups().into_iter().find(|g| g.name == hall_group(sim, "camera")).expect("camera group");
            let body = g.cubes.iter().find(|c| c.name == "CamBody").expect("body");
            let world = g.origin + body.pos;
            let (from, _) = m.camera_transform(sim);
            assert!((world - from).length() < 1e-4, "{sim}");
            assert!(g.cubes.iter().any(|c| c.name == "LedRec"));
        }
    }

    #[test]
    fn a_replaced_box_group_is_left_out() {
        let n = names(&hall_groups());
        assert!(!n.contains(&"hall.lobby.Sofa".to_string()), "the couch bbmodel replaces it");
        assert!(!n.contains(&"hall.ballpit.DrumKit".to_string()));
        assert!(!n.contains(&"hall.ballpit.Piano".to_string()));
        assert!(!n.contains(&"hall.ballpit.AmpL".to_string()));
        assert!(!n.contains(&"hall.arcade.MicStand".to_string()));
        assert!(n.contains(&"hall.ballpit.Rug".to_string()));
    }

    #[test]
    fn the_hall_sits_where_the_map_says() {
        let g = hall_groups();
        let shell = g.iter().find(|x| x.name == "hall.lobby.shell").expect("shell");
        assert_eq!(shell.origin, vec3(-8.0, 0.0, -37.5));
        let crawl = g.iter().find(|x| x.name == "hall.ventl.shell").expect("shell");
        let ceiling = crawl.cubes.iter().find(|c| c.name == "Ceiling").expect("ceiling");
        assert!((ceiling.pos.y - 2.7).abs() < 1e-5, "the crawlspace ceiling is 2.6 high");
    }

    #[test]
    fn feed_lights_put_a_ceiling_light_in_the_room() {
        let l = feed_lights("prize");
        assert_eq!(l.omni.len(), 1);
        let c = map().centre_of_sim("prize");
        assert!((l.omni[0].pos.x - c.x).abs() < 1e-5 && (l.omni[0].pos.y - 3.0).abs() < 1e-5);
    }

    #[test]
    fn props_are_anchored_by_their_footprint_and_base() {
        let min = vec3(0.0, 0.0, 0.0);
        let max = vec3(32.0, 16.0, 16.0); // 2 x 1 x 1 m at 1/16
        let at = anchored(vec3(5.0, 0.0, 5.0), 0.0, min, max, 1.0 / 16.0);
        assert!((at - vec3(4.0, 0.0, 4.5)).length() < 1e-5);
        // turned a half turn the offset flips
        let at = anchored(vec3(5.0, 0.0, 5.0), std::f32::consts::PI, min, max, 1.0 / 16.0);
        assert!((at - vec3(6.0, 0.0, 5.5)).length() < 1e-4);
        // a prop 1 unit above the floor stays there
        let at = anchored(vec3(0.0, 0.8, 0.0), 0.0, vec3(-8.0, 0.0, -8.0), vec3(8.0, 8.0, 8.0), 1.0 / 16.0);
        assert!((at.y - 0.8).abs() < 1e-6 && at.x.abs() < 1e-6);
    }

    #[test]
    fn every_placed_model_is_embedded() {
        let have: Vec<&str> = PROP_FILES.iter().map(|(n, _)| *n).collect();
        for p in &map().props {
            assert!(have.contains(&p.model.as_str()), "{} is not embedded", p.model);
        }
        for (name, text) in PROP_FILES {
            assert!(text.contains("elements"), "{name} is a bbmodel");
        }
    }

    #[test]
    fn every_prop_sits_inside_its_room() {
        let m = map();
        for p in &m.props {
            if p.room == "booth" {
                assert!(p.x.abs() <= HALF_W && p.z.abs() <= HALF_D, "{} in the booth", p.model);
                continue;
            }
            let r = m.by_sim(&p.room).unwrap_or_else(|| panic!("room {}", p.room));
            assert!(p.x.abs() <= r.w() * 0.5 && p.z.abs() <= r.d() * 0.5, "{} at ({}, {}) in {}", p.model, p.x, p.z, p.room);
        }
    }
}
