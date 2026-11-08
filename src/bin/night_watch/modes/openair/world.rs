//! Open Air's three outdoor sets in 3D: the ROOFTOP (a parapet roof with the stair door on the left and the hatch on the
//! right, gantries round to the doors, a lantern post and a torch stand each side), the BUS STOP (a shelter with a heat lamp
//! and a bench on a pavement beside the road, three places a fuel canister turns up, a bus that comes up the road at dawn) and
//! the CAMPFIRE (a fire pit, a log pile, firewood at the edge of the trees, a torch post each side and a tent, in a clearing
//! ringed with fir trees). Port of `openair_world.gd`, with the same coordinates, plus what the contract asks of every map
//! (9.1): security cameras on poles at the edges, props in every room (the bbmodels of `last_route/assets/props` where one
//! fits, built boxes otherwise), a skyline or a treeline round the set.
//!
//! `build_builder` is pure (cubes and solids; the tests read it); `OpenAirWorld::build` bakes it and loads the prop models,
//! which needs the GL context.
#![allow(dead_code)]

use super::data::{map_of, tun, CamDef, PropDef};
use super::sim::Side;
use crate::core::Mulberry32;
use crate::gfx::{cube_geometry, Aabb, Cube, Gfx, Group, LightSet, Omni, Spot, Textures, World, WorldBuilder};
use crate::hunters::bbmodel::Model;
use macroquad::prelude::*;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::rc::Rc;

pub const SLAB_OPEN_Y: f32 = 3.6;
pub const SLAB_CLOSED_Y: f32 = 1.2;
pub const ROOF_X: f32 = 6.5;
pub const ROOF_Z: f32 = 5.0;

const WHITE3: [f32; 3] = [1.0, 1.0, 1.0];
const STEEL: [f32; 3] = [0.62, 0.64, 0.7];
const DARK_STEEL: [f32; 3] = [0.38, 0.4, 0.46];
const BRASS: [f32; 3] = [0.7, 0.55, 0.3];
const ASPHALT: [f32; 3] = [0.45, 0.45, 0.5];
const CANVAS: [f32; 3] = [0.8, 0.55, 0.3];

fn put(g: &mut Group, name: &str, size: [f32; 3], pos: [f32; 3], kind: &str, tint: [f32; 3], solid: bool) {
    let c = g.bx(name, size, pos);
    c.set_kind(kind).set_tint(tint);
    if solid {
        c.set_solid();
    }
}

/// A cube turned about Y (the builder shortcut `bx` has no rotation).
fn put_rot(g: &mut Group, name: &str, size: [f32; 3], pos: [f32; 3], kind: &str, tint: [f32; 3], solid: bool, rot_y: f32) {
    let mut c = Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).kind(kind).tint(tint).rot_y(rot_y);
    if solid {
        c = c.solid();
    }
    g.add(c);
}

// ------------------------------------------------------------------------------------------------ props (bbmodels)

/// The prop models Open Air uses, embedded from the Last Route assets.
fn prop_text(name: &str) -> Option<&'static str> {
    Some(match name {
        "supplyCrates" => include_str!("../../../last_route/assets/props/supplyCrates.bbmodel"),
        "mallBench" => include_str!("../../../last_route/assets/props/mallBench.bbmodel"),
        "storageBarrel" => include_str!("../../../last_route/assets/props/storageBarrel.bbmodel"),
        "giantTree" => include_str!("../../../last_route/assets/props/giantTree.bbmodel"),
        "locker" => include_str!("../../../last_route/assets/props/locker.bbmodel"),
        "floorLamp" => include_str!("../../../last_route/assets/props/floorLamp.bbmodel"),
        "hazardSign" => include_str!("../../../last_route/assets/props/hazardSign.bbmodel"),
        "paSpeakerPole" => include_str!("../../../last_route/assets/props/paSpeakerPole.bbmodel"),
        "officeFileBoxes" => include_str!("../../../last_route/assets/props/officeFileBoxes.bbmodel"),
        "retroComputerTerminal" => include_str!("../../../last_route/assets/props/retroComputerTerminal.bbmodel"),
        "cableCoil" => include_str!("../../../last_route/assets/props/cableCoil.bbmodel"),
        "charredSupplyCrate" => include_str!("../../../last_route/assets/props/charredSupplyCrate.bbmodel"),
        _ => return None,
    })
}

/// Is this prop model embedded?
pub fn prop_known(name: &str) -> bool {
    prop_text(name).is_some()
}

/// Every prop model name this mode can place.
pub const PROP_NAMES: [&str; 12] = [
    "supplyCrates",
    "mallBench",
    "storageBarrel",
    "giantTree",
    "locker",
    "floorLamp",
    "hazardSign",
    "paSpeakerPole",
    "officeFileBoxes",
    "retroComputerTerminal",
    "cableCoil",
    "charredSupplyCrate",
];

/// Half extents (x, z) and height of the part of a prop that blocks the player (metres at scale 1); None when it is flat.
pub fn prop_box(name: &str) -> Option<(f32, f32, f32)> {
    Some(match name {
        "supplyCrates" => (0.38, 0.31, 0.94),
        "mallBench" => (0.88, 0.31, 0.56),
        "storageBarrel" => (0.31, 0.31, 0.94),
        "giantTree" => (0.35, 0.35, 3.94),
        "locker" => (0.31, 0.35, 2.0),
        "floorLamp" => (0.19, 0.19, 1.75),
        "hazardSign" => (0.31, 0.08, 0.63),
        "paSpeakerPole" => (0.25, 0.2, 2.0),
        "officeFileBoxes" => (0.5, 0.2, 0.88),
        "retroComputerTerminal" => (0.5, 0.3, 0.81),
        "charredSupplyCrate" => (0.38, 0.35, 0.63),
        _ => return None,
    })
}

/// The blocking box of a placed prop (turned by its yaw, scaled).
pub fn prop_aabb(def: &PropDef) -> Option<Aabb> {
    let (hx, hz, h) = prop_box(&def.model)?;
    let yaw = def.yaw_deg.to_radians();
    let (s, c) = (yaw.sin().abs(), yaw.cos().abs());
    let ex = (hx * c + hz * s) * def.scale;
    let ez = (hx * s + hz * c) * def.scale;
    let h = h * def.scale;
    Some(Aabb { min: vec3(def.pos.x - ex, def.pos.y, def.pos.z - ez), max: vec3(def.pos.x + ex, def.pos.y + h, def.pos.z + ez) })
}

// ------------------------------------------------------------------------------------------------ geometry (pure)

/// The cubes and solids of a watch's set. Pure: no GL.
pub fn build_builder(watch: usize) -> WorldBuilder {
    let mut wb = WorldBuilder::new();
    match watch.min(2) {
        0 => rooftop(&mut wb),
        1 => busstop(&mut wb),
        _ => campfire(&mut wb),
    }
    cameras(&mut wb, watch);
    // props that stand where the player can walk block him
    let map = map_of(watch);
    let (x0, z0, x1, z1) = map.footprint;
    for p in &map.props {
        if p.pos.x > x0 - 1.0 && p.pos.x < x1 + 1.0 && p.pos.z > z0 - 1.0 && p.pos.z < z1 + 1.0 {
            if let Some(a) = prop_aabb(p) {
                wb.solids.push(a);
            }
        }
    }
    wb
}

fn rooftop(wb: &mut WorldBuilder) {
    let (rx, rz) = (ROOF_X, ROOF_Z);
    {
        let g = wb.group("Roof", Vec3::ZERO);
        put(g, "RoofSlab", [rx * 2.0 + 0.4, 0.3, rz * 2.0 + 0.4], [0.0, -0.15, 0.0], "concrete", [0.8, 0.8, 0.85], true);
        for z in [-rz - 0.1, rz + 0.1] {
            put(g, "ParapetWall", [rx * 2.0 + 0.4, 1.1, 0.2], [0.0, 0.55, z], "concrete", WHITE3, true);
        }
        let piece = rz - 1.0;
        for s in [-1.0f32, 1.0] {
            let x = s * (rx + 0.1);
            for zz in [-1.0 - piece * 0.5, 1.0 + piece * 0.5] {
                put(g, "ParapetWall", [0.2, 1.1, piece], [x, 0.55, zz], "concrete", WHITE3, true);
            }
            // the doorway's frame and a header
            put(g, "DoorHeader", [0.3, 0.2, 2.2], [x, 2.5, 0.0], "metal", WHITE3, false);
            for zz in [-1.0f32, 1.0] {
                put(g, "DoorFrame", [0.3, 2.5, 0.2], [x, 1.25, zz], "metal", BRASS, false);
            }
            // a stoop in the doorway so the slab has something to bar
            put(g, "DoorSill", [0.5, 0.12, 2.2], [x, 0.06, 0.0], "concrete", [0.7, 0.7, 0.75], false);
        }
        // parapet cap stones, tar seams and a painted safety line round the edge
        for z in [-rz - 0.1, rz + 0.1] {
            put(g, "ParapetCap", [rx * 2.0 + 0.6, 0.08, 0.34], [0.0, 1.14, z], "concrete", [0.9, 0.9, 0.95], false);
        }
        for z in [-rz + 0.5, rz - 0.5] {
            put(g, "EdgeStripe", [rx * 2.0 - 0.4, 0.02, 0.1], [0.0, 0.01, z], "label", [0.9, 0.8, 0.3], false);
        }
        for x in [-2.2f32, 2.2] {
            put(g, "RoofSeam", [0.08, 0.02, rz * 2.0 - 1.2], [x, 0.01, 0.0], "floor", [0.5, 0.5, 0.55], false);
        }
    }
    for (s, tag) in [(-1.0f32, "L"), (1.0, "R")] {
        // the slab: the stair door (L) / the hatch cover (R), slid down to bar it
        let x = s * (rx + 0.1);
        let name = format!("Slab{tag}");
        let g = wb.group(&name, Vec3::ZERO);
        put(g, &name, [0.18, 2.4, 2.0], [x, SLAB_OPEN_Y, 0.0], "metal", if s < 0.0 { [0.6, 0.62, 0.7] } else { BRASS }, false);
        // the player stays on the roof: an invisible block beyond the doorway
        wb.solids.push(Aabb::from_centre(vec3(s * (rx + 0.6), 1.0, 0.0), vec3(0.4, 2.0, 2.4)));
    }
    {
        // the catwalks each hunter climbs: a gantry from the far end round the corner to the door
        let g = wb.group("Catwalks", Vec3::ZERO);
        for s in [-1.0f32, 1.0] {
            put(g, "CatwalkPlatform", [2.3, 0.2, 2.4], [s * (rx + 1.35), -0.1, 0.0], "metal", WHITE3, false);
            put(g, "CatwalkPlatform", [2.0, 0.2, 6.0], [s * (rx + 2.2), -0.1, -3.6], "metal", WHITE3, false);
            put(g, "CatwalkPlatform", [2.0, 0.2, 7.0], [s * (rx + 2.2), -0.1, -9.0], "metal", WHITE3, false);
            put(g, "CatwalkRail", [0.08, 0.9, 6.0], [s * (rx + 3.1), 0.45, -3.6], "metal", WHITE3, false);
            put(g, "CatwalkRail", [0.08, 0.9, 7.0], [s * (rx + 3.1), 0.45, -9.0], "metal", WHITE3, false);
            put(g, "CatwalkRail", [0.08, 0.06, 7.0], [s * (rx + 3.1), 0.9, -9.0], "metal", DARK_STEEL, false);
            put(g, "CatwalkRail", [0.08, 0.06, 6.0], [s * (rx + 3.1), 0.9, -3.6], "metal", DARK_STEEL, false);
            for z in [-12.0f32, -9.0, -6.0, -3.0] {
                put(g, "RailPost", [0.08, 0.9, 0.08], [s * (rx + 3.1), 0.45, z], "metal", DARK_STEEL, false);
            }
            // legs down into the dark
            for z in [-11.5f32, -6.0, -1.0] {
                put(g, "CatwalkLeg", [0.18, 6.0, 0.18], [s * (rx + 2.9), -3.1, z], "metal", DARK_STEEL, false);
                put(g, "CatwalkLeg", [0.18, 6.0, 0.18], [s * (rx + 1.5), -3.1, z], "metal", DARK_STEEL, false);
            }
        }
        put(g, "CatwalkPlatform", [rx * 2.0 + 4.0, 0.2, 2.0], [0.0, -0.1, -rz - 3.2], "metal", WHITE3, false);
        put(g, "CatwalkRail", [rx * 2.0 + 4.0, 0.08, 0.1], [0.0, 0.5, -rz - 4.1], "metal", WHITE3, false);
        put(g, "CatwalkRail", [rx * 2.0 + 4.0, 0.9, 0.06], [0.0, 0.45, -rz - 4.1], "metal", DARK_STEEL, false);
        for x in [-6.0f32, -3.0, 0.0, 3.0, 6.0] {
            put(g, "CatwalkLeg", [0.18, 6.0, 0.18], [x, -3.1, -rz - 3.2], "metal", DARK_STEEL, false);
        }
    }
    {
        // the lantern post, a torch stand each side, a vent box, a few crates
        let g = wb.group("Kit", Vec3::ZERO);
        put(g, "LanternPost", [0.14, 1.9, 0.14], [0.0, 0.95, -3.0], "metal", WHITE3, true);
        put(g, "LanternGlow", [0.5, 0.5, 0.5], [0.0, 2.1, -3.0], "light", WHITE3, false);
        put(g, "LanternCage", [0.62, 0.06, 0.62], [0.0, 1.8, -3.0], "metal", DARK_STEEL, false);
        for s in [-1.0f32, 1.0] {
            put(g, "TorchStand", [0.12, 1.4, 0.12], [s * 4.8, 0.7, -1.6], "metal", WHITE3, true);
            put(g, "TorchGlow", [0.22, 0.22, 0.22], [s * 4.8, 1.5, -1.6], "light", WHITE3, false);
            put(g, "TorchBracket", [0.3, 0.05, 0.3], [s * 4.8, 1.32, -1.6], "metal", DARK_STEEL, false);
        }
        put(g, "CrateStack", [1.4, 1.0, 1.0], [-2.6, 0.5, -3.6], "wood", WHITE3, true);
        put(g, "CrateSmall", [0.9, 0.8, 0.9], [2.8, 0.4, 3.8], "wood", WHITE3, true);
        put(g, "VentBox", [1.6, 0.6, 1.2], [3.6, 0.3, -3.5], "metal", WHITE3, true);
        put(g, "VentCowl", [0.5, 0.35, 0.5], [3.6, 0.78, -3.5], "metal", DARK_STEEL, false);
        // an air-conditioning unit
        put(g, "AcUnit", [1.6, 0.9, 1.0], [-4.4, 0.45, 3.7], "metal", [0.75, 0.77, 0.82], true);
        put(g, "AcGrille", [0.9, 0.05, 0.9], [-4.4, 0.92, 3.7], "metal", [0.25, 0.27, 0.3], false);
        put(g, "AcPipe", [0.1, 0.1, 1.6], [-5.3, 0.3, 2.5], "metal", [0.8, 0.8, 0.85], false);
        // a water tank on legs
        for dx in [-0.6f32, 0.6] {
            for dz in [-0.6f32, 0.6] {
                put(g, "TankLeg", [0.1, 0.8, 0.1], [5.0 + dx, 0.4, 3.4 + dz], "wood", [0.6, 0.55, 0.5], false);
            }
        }
        put(g, "WaterTank", [1.5, 1.6, 1.5], [5.0, 1.6, 3.4], "metal", [0.55, 0.62, 0.6], true);
        put_rot(g, "WaterTank", [1.5, 1.6, 1.5], [5.0, 1.6, 3.4], "metal", [0.55, 0.62, 0.6], false, std::f32::consts::FRAC_PI_4);
        put(g, "TankLid", [1.3, 0.1, 1.3], [5.0, 2.45, 3.4], "metal", DARK_STEEL, false);
        // a chimney stack and an antenna mast with a red beacon
        put(g, "ChimneyStack", [0.8, 2.2, 0.8], [5.8, 1.1, -2.6], "concrete", [0.7, 0.62, 0.58], true);
        put(g, "ChimneyCap", [1.0, 0.12, 1.0], [5.8, 2.26, -2.6], "concrete", [0.55, 0.55, 0.6], false);
        put(g, "MastPole", [0.08, 4.5, 0.08], [-3.6, 2.25, -4.4], "metal", DARK_STEEL, true);
        put(g, "MastBar", [1.2, 0.06, 0.06], [-3.6, 3.5, -4.4], "metal", DARK_STEEL, false);
        put(g, "MastBar", [0.8, 0.06, 0.06], [-3.6, 4.0, -4.4], "metal", DARK_STEEL, false);
        put(g, "BeaconGlow", [0.16, 0.16, 0.16], [-3.6, 4.58, -4.4], "red", [1.0, 0.3, 0.3], false);
    }
    skyline(wb);
}

/// A ring of dark towers round the roof, a few windows lit.
fn skyline(wb: &mut WorldBuilder) {
    let mut r = Mulberry32::new(2024);
    let g = wb.group("Skyline", Vec3::ZERO);
    for i in 0..16 {
        let a = TAU * i as f32 / 16.0 + r.range(-0.1, 0.1) as f32;
        let d = r.range(38.0, 60.0) as f32;
        let w = r.range(6.0, 12.0) as f32;
        let top = r.range(-6.0, 22.0) as f32;
        let bottom = -40.0;
        let (cx, cz) = (a.cos() * d, a.sin() * d - 6.0);
        put(g, "SkylineBlock", [w, top - bottom, w], [cx, (top + bottom) * 0.5, cz], "concrete", [0.28, 0.3, 0.36], false);
        // lit windows on the face toward the roof
        let to = vec2(-a.cos(), -a.sin());
        let along = vec2(-to.y, to.x);
        for _ in 0..5 {
            let off = r.range(-(w as f64) * 0.4, w as f64 * 0.4) as f32;
            let y = r.range(bottom as f64 + 8.0, top as f64 - 1.0) as f32;
            let p = vec2(cx, cz) + to * (w * 0.5 + 0.05) + along * off;
            put(g, "WindowGlow", [0.7, 0.9, 0.7], [p.x, y, p.y], "light", [1.0, 0.85, 0.5], false);
        }
    }
}

fn busstop(wb: &mut WorldBuilder) {
    let map = map_of(1);
    {
        let g = wb.group("Street", Vec3::ZERO);
        put(g, "PavementSlab", [30.0, 0.3, 8.0], [0.0, -0.15, 3.0], "concrete", [0.75, 0.75, 0.8], true);
        put(g, "RoadSlab", [60.0, 0.3, 7.0], [0.0, -0.2, -4.0], "floor", ASPHALT, false);
        let mut x = -24;
        while x <= 24 {
            put(g, "RoadStripe", [1.6, 0.02, 0.14], [x as f32, 0.0, -4.0], "label", [0.9, 0.9, 0.8], false);
            x += 4;
        }
        put(g, "Kerb", [30.0, 0.22, 0.3], [0.0, 0.0, -0.4], "concrete", WHITE3, true);
        put(g, "BackFence", [30.0, 1.2, 0.3], [0.0, 0.6, 7.2], "wall", WHITE3, true);
        for s in [-1.0f32, 1.0] {
            put(g, "SideFence", [0.3, 1.2, 8.0], [s * 15.0, 0.6, 3.0], "wall", WHITE3, true);
        }
        // the far pavement and the street across the road
        put(g, "FarPavement", [60.0, 0.3, 5.0], [0.0, -0.15, -9.5], "concrete", [0.7, 0.7, 0.75], false);
        for i in 0..4 {
            let x = -22.0 + i as f32 * 12.0;
            let h = 7.0 + (i % 3) as f32 * 3.0;
            put(g, "ShopFront", [9.0, h, 8.0], [x, h * 0.5, -16.0], "wall", [0.4 + 0.05 * i as f32, 0.42, 0.5], false);
            put(g, "ShopWindowGlow", [2.2, 1.4, 0.1], [x - 2.0, 2.0, -11.95], "light", [1.0, 0.82, 0.5], false);
            put(g, "ShopWindowGlow", [2.2, 1.4, 0.1], [x + 2.0, 2.0, -11.95], "light", [0.6, 0.8, 1.0], false);
            put(g, "ShopAwning", [8.0, 0.1, 1.2], [x, 3.2, -12.4], "fabric", [0.6, 0.3, 0.3], false);
        }
        // road bollards, a lamp post further down the road, a bin and a hedge behind the fence
        for x in [-12.0f32, -2.0, 8.0, 12.0] {
            put(g, "Bollard", [0.2, 0.7, 0.2], [x, 0.35, -0.9], "metal", [0.9, 0.9, 0.3], false);
        }
        put(g, "StopPole2", [0.12, 3.4, 0.12], [-14.0, 1.7, -0.6], "metal", WHITE3, true);
        put(g, "StreetLampGlow", [0.5, 0.12, 0.3], [-13.7, 3.4, -0.6], "light", [0.9, 0.95, 1.0], false);
        put(g, "TrashBin", [0.5, 0.9, 0.5], [2.4, 0.45, 6.4], "plastic", [0.3, 0.45, 0.35], true);
        for x in [-12.0f32, -6.0, 6.0, 12.0] {
            put(g, "Hedge", [3.0, 1.5, 0.9], [x, 0.75, 7.9], "fabric", [0.2, 0.4, 0.25], false);
        }
    }
    {
        // the shelter: roof, posts, glass back, a bench, the sign
        let g = wb.group("Shelter", Vec3::ZERO);
        put(g, "ShelterRoof", [5.0, 0.15, 2.4], [0.0, 2.4, 3.6], "metal", [0.6, 0.62, 0.7], false);
        for x in [-2.3f32, 2.3] {
            for z in [2.5f32, 4.7] {
                put(g, "ShelterPost", [0.12, 2.4, 0.12], [x, 1.2, z], "metal", WHITE3, true);
            }
        }
        put(g, "ShelterGlass", [4.6, 1.6, 0.06], [0.0, 1.35, 4.8], "glass", [0.5, 0.6, 0.65], false);
        put(g, "ShelterGlass", [0.06, 1.6, 2.0], [-2.3, 1.35, 3.6], "glass", [0.5, 0.6, 0.65], false);
        put(g, "Bench", [2.0, 0.45, 0.5], [0.6, 0.25, 4.3], "wood", WHITE3, true);
        put(g, "BenchBack", [2.0, 0.4, 0.08], [0.6, 0.68, 4.52], "wood", [0.8, 0.75, 0.7], false);
        put(g, "StopPole", [0.1, 2.6, 0.1], [3.2, 1.3, 1.6], "metal", WHITE3, true);
        put(g, "StopSignBoard", [0.9, 0.5, 0.06], [3.2, 2.7, 1.6], "label", [0.3, 0.6, 0.9], false);
        put(g, "TimetableBoard", [0.7, 1.0, 0.05], [1.6, 1.3, 4.72], "paper", [0.85, 0.85, 0.8], false);
        put(g, "AdPanelGlow", [1.0, 1.5, 0.05], [-1.6, 1.4, 4.72], "light", [0.7, 0.85, 1.0], false);
    }
    {
        // the heat lamp: a post with a hood and a glowing element; the street light
        let g = wb.group("HeatLamp", Vec3::ZERO);
        put(g, "LampPost", [0.14, 1.7, 0.14], [-1.5, 0.85, 3.0], "metal", WHITE3, true);
        put(g, "LampHood", [0.6, 0.12, 0.6], [-1.5, 1.75, 3.0], "metal", WHITE3, false);
        put(g, "LampGlow", [0.4, 0.08, 0.4], [-1.5, 1.66, 3.0], "light", WHITE3, false);
        put(g, "LampBase", [0.5, 0.1, 0.5], [-1.5, 0.05, 3.0], "metal", DARK_STEEL, false);
        put(g, "StreetPost", [0.12, 3.4, 0.12], [-6.0, 1.7, -0.6], "metal", WHITE3, true);
        put(g, "StreetLampGlow", [0.5, 0.12, 0.3], [-5.8, 3.38, -0.6], "light", [0.9, 0.95, 1.0], false);
    }
    // three places a canister turns up (hidden until one does)
    for (i, spot) in map.cans.iter().enumerate() {
        let g = wb.group(&format!("Can{i}"), *spot);
        put(g, "FuelCanister", [0.3, 0.5, 0.3], [0.0, 0.25, 0.0], "metal", [0.9, 0.35, 0.25], false);
        put(g, "FuelCap", [0.12, 0.1, 0.12], [0.0, 0.55, 0.0], "metal", WHITE3, false);
        put(g, "FuelHandle", [0.2, 0.05, 0.05], [0.0, 0.52, 0.0], "metal", DARK_STEEL, false);
    }
    {
        // the bus, off the end of the road until dawn
        let z = map.bus.map(|b| b.2).unwrap_or(-4.4);
        let g = wb.group("Bus", vec3(0.0, 0.0, z));
        put(g, "BusBody", [6.0, 2.2, 2.2], [0.0, 1.4, 0.0], "metal", [0.9, 0.75, 0.3], false);
        put(g, "BusWindow", [5.6, 0.7, 2.3], [0.0, 1.8, 0.0], "glass", [0.5, 0.6, 0.7], false);
        put(g, "BusHeadGlow", [0.1, 0.3, 0.5], [3.05, 0.8, 0.6], "light", WHITE3, false);
        put(g, "BusHeadGlow", [0.1, 0.3, 0.5], [3.05, 0.8, -0.6], "light", WHITE3, false);
        put(g, "BusTailGlow", [0.1, 0.25, 0.4], [-3.05, 0.8, 0.7], "red", [1.0, 0.3, 0.3], false);
        put(g, "BusTailGlow", [0.1, 0.25, 0.4], [-3.05, 0.8, -0.7], "red", [1.0, 0.3, 0.3], false);
        put(g, "BusStripe", [6.02, 0.2, 2.22], [0.0, 1.0, 0.0], "red", [0.8, 0.3, 0.25], false);
        for x in [-1.9f32, 1.9] {
            put(g, "BusWheel", [0.9, 0.9, 2.5], [x, 0.45, 0.0], "metal", [0.2, 0.2, 0.22], false);
        }
    }
}

fn campfire(wb: &mut WorldBuilder) {
    {
        let g = wb.group("Clearing", Vec3::ZERO);
        put(g, "ClearingSlab", [34.0, 0.3, 34.0], [0.0, -0.15, -2.0], "floor", [0.55, 0.62, 0.5], true);
        for i in 0..10 {
            let a = TAU * i as f32 / 10.0;
            put_rot(g, "FireStone", [0.4, 0.28, 0.5], [a.cos() * 0.95, 0.12, a.sin() * 0.95], "concrete", [0.7, 0.7, 0.75], true, -a);
        }
        put(g, "Kindling", [1.0, 0.15, 0.2], [0.0, 0.1, 0.0], "wood", [0.6, 0.5, 0.4], false);
        put_rot(g, "Kindling", [1.0, 0.15, 0.2], [0.0, 0.14, 0.0], "wood", [0.55, 0.45, 0.35], false, 1.2);
        // log seats round the fire
        for deg in [200.0f32, 245.0, 290.0] {
            let a = deg.to_radians();
            put_rot(g, "SeatLog", [1.4, 0.3, 0.4], [a.cos() * 2.4, 0.15, a.sin() * 2.4], "wood", [0.55, 0.45, 0.38], false, -a + std::f32::consts::FRAC_PI_2);
        }
        // a footpath of flat stones to the tent and the firewood
        for i in 0..5 {
            put(g, "StepStone", [0.5, 0.05, 0.5], [3.9 - i as f32 * 0.2 + 0.2, 0.02, 3.4 + i as f32 * 0.5], "concrete", [0.6, 0.6, 0.65], false);
        }
    }
    {
        // the flames: their size follows the fire
        let g = wb.group("Fire", Vec3::ZERO);
        put(g, "FireGlow", [0.8, 0.5, 0.8], [0.0, 0.3, 0.0], "light", [1.0, 0.7, 0.35], false);
        put(g, "FireTipGlow", [0.4, 0.7, 0.4], [0.0, 0.62, 0.0], "light", [1.0, 0.85, 0.5], false);
    }
    {
        // the trees: a ring of trunks with tiered fir crowns (a fixed pattern, so every run looks the same)
        let mut r = Mulberry32::new(4141);
        let g = wb.group("Trees", Vec3::ZERO);
        for i in 0..46 {
            let a = TAU * i as f32 / 46.0 + r.range(-0.06, 0.06) as f32;
            let rad = r.range(11.5, 15.5) as f32;
            let h = r.range(4.0, 6.5) as f32;
            let (px, pz) = (a.cos() * rad, a.sin() * rad - 2.0);
            put(g, "TreeTrunk", [0.5, h, 0.5], [px, h * 0.5, pz], "wood", [0.5, 0.42, 0.34], true);
            for tier in 0..4 {
                let w = 3.4 - tier as f32 * 0.75;
                let ty = 1.5 + tier as f32 * (h - 1.0) / 4.0;
                put(g, "TreeCrown", [w, (h - 1.0) / 4.0 + 0.3, w], [px, ty + 0.5, pz], "fabric", [0.16, 0.36, 0.22], false);
            }
        }
    }
    {
        // the log pile (it shrinks as the logs go on the fire), firewood at the edge of the trees
        let g = wb.group("LogPile", vec3(3.4, 0.0, -2.6));
        for i in 0..10usize {
            put(g, &format!("PileLog{i}"), [0.9, 0.22, 0.22], [(i % 2) as f32 * 0.08, 0.12 + 0.22 * (i / 2) as f32, ((i % 5) as f32 - 2.0) * 0.16], "wood", WHITE3, false);
        }
    }
    {
        let g = wb.group("Firewood", Vec3::ZERO);
        put(g, "WoodStack", [2.4, 1.0, 0.9], [0.0, 0.5, 6.2], "wood", WHITE3, true);
        put(g, "WoodPost", [0.12, 1.4, 0.12], [-1.4, 0.7, 6.2], "metal", WHITE3, true);
        put(g, "WoodPost", [0.12, 1.4, 0.12], [1.4, 0.7, 6.2], "metal", WHITE3, true);
        put(g, "WoodRoof", [3.0, 0.08, 1.3], [0.0, 1.45, 6.2], "wood", [0.5, 0.42, 0.34], false);
        put(g, "ChoppingBlock", [0.5, 0.45, 0.5], [2.4, 0.22, 5.2], "wood", [0.6, 0.5, 0.4], true);
    }
    for (s, tag) in [(-1.0f32, "L"), (1.0, "R")] {
        let g = wb.group(&format!("TorchPost{tag}"), Vec3::ZERO);
        put(g, "TorchStand", [0.12, 1.6, 0.12], [s * 3.9, 0.8, 0.5], "metal", WHITE3, true);
        put(g, "TorchGlow", [0.22, 0.22, 0.22], [s * 3.9, 1.7, 0.5], "light", WHITE3, false);
        put(g, "TorchBracket", [0.3, 0.05, 0.3], [s * 3.9, 1.5, 0.5], "metal", DARK_STEEL, false);
    }
    {
        // the tent: a small blocky tent on the right, open towards the fire
        let g = wb.group("Tent", Vec3::ZERO);
        put(g, "TentFloor", [2.6, 0.08, 2.6], [6.0, 0.04, 2.0], "fabric", [0.5, 0.45, 0.4], false);
        put(g, "TentWallBack", [0.08, 1.6, 2.6], [7.3, 0.8, 2.0], "fabric", CANVAS, true);
        for z in [0.7f32, 3.3] {
            put(g, "TentWallSide", [2.6, 1.6, 0.08], [6.0, 0.8, z], "fabric", CANVAS, true);
        }
        put(g, "TentRoof", [2.8, 0.08, 3.0], [6.0, 1.7, 2.0], "fabric", [0.85, 0.6, 0.35], false);
        put(g, "TentRidge", [2.8, 0.08, 0.08], [6.0, 1.9, 2.0], "metal", WHITE3, false);
        put(g, "TentPeg", [0.05, 0.3, 0.05], [4.5, 0.15, 0.5], "metal", DARK_STEEL, false);
        put(g, "TentPeg", [0.05, 0.3, 0.05], [4.5, 0.15, 3.5], "metal", DARK_STEEL, false);
        put(g, "Bedroll", [0.8, 0.14, 1.8], [6.7, 0.12, 2.0], "fabric", [0.3, 0.4, 0.55], false);
    }
}

/// A camera on a pole at each mount of the map: pole, housing, lens and a red light. (The Godot game has no cameras here.)
fn cameras(wb: &mut WorldBuilder, watch: usize) {
    for c in &map_of(watch).cameras {
        let g = wb.group(&format!("Cam_{}", c.id), Vec3::ZERO);
        let fwd = (c.target - c.pos).normalize_or_zero();
        let yaw = (-fwd.x).atan2(-fwd.z);
        if c.pole {
            let h = (c.pos.y - c.base_y - 0.1).max(0.1);
            put(g, "CamPole", [0.1, h, 0.1], [c.pos.x, c.base_y + h * 0.5, c.pos.z], "metal", DARK_STEEL, false);
        }
        put_rot(g, "CamBody", [0.26, 0.2, 0.4], [c.pos.x, c.pos.y, c.pos.z], "plastic", [0.5, 0.5, 0.55], false, yaw);
        let lens = c.pos + fwd * 0.22;
        put_rot(g, "CamLens", [0.12, 0.12, 0.06], [lens.x, lens.y, lens.z], "plastic", [0.1, 0.1, 0.15], false, yaw);
        put_rot(g, "CamRecGlow", [0.05, 0.05, 0.05], [c.pos.x, c.pos.y + 0.13, c.pos.z], "red", [1.0, 0.2, 0.2], false, yaw);
    }
}

// ------------------------------------------------------------------------------------------------ particles (pure)

/// A repeatable pseudo-random 0..1 from an index and a salt.
pub fn hash01(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9e37_79b1) ^ salt.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297a_2d39);
    x ^= x >> 15;
    (x & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// What falls: (count, half width x, half width z, centre z, speed range, cube size, colour). None on the rooftop.
fn weather_params(watch: usize) -> Option<(u32, f32, f32, f32, (f32, f32), Vec3, [f32; 3])> {
    match watch {
        1 => Some((170, 14.0, 10.0, 0.0, (1.2, 2.0), vec3(0.06, 0.06, 0.06), [0.95, 0.97, 1.0])),
        2 => Some((260, 14.0, 14.0, -2.0, (7.0, 9.0), vec3(0.02, 0.35, 0.02), [0.6, 0.7, 0.95])),
        _ => None,
    }
}

/// Where each flake or streak is at `time_s`: falling from 8 m to the ground, again and again; snow also drifts sideways.
pub fn particle_positions(watch: usize, time_s: f32) -> Vec<Vec3> {
    let Some((count, ex, ez, cz, (v0, v1), _, _)) = weather_params(watch) else { return Vec::new() };
    let snow = watch == 1;
    (0..count)
        .map(|i| {
            let v = v0 + hash01(i, 4) * (v1 - v0);
            let phase = hash01(i, 5);
            let fall = (time_s * v / 8.0 + phase).fract();
            let drift = if snow { (time_s * 0.7 + phase * TAU).sin() * 0.4 } else { 0.0 };
            vec3((hash01(i, 1) * 2.0 - 1.0) * ex + drift, 8.0 - fall * 8.0, cz + (hash01(i, 2) * 2.0 - 1.0) * ez)
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------ the baked set

/// What the lights of the set need to know this frame.
#[derive(Clone, Copy, Debug)]
pub struct SceneState {
    pub lantern: bool,
    pub lamp_on: bool,
    /// 0..100
    pub fire: f64,
    pub weather_on: bool,
    pub torch_side: Option<Side>,
    /// The player's feet (the torch is held from here).
    pub feet: Vec3,
    pub time_s: f32,
}

pub struct OpenAirWorld {
    pub watch: usize,
    pub world: World,
    props: Vec<Mesh>,
    pub props_placed: usize,
    slab_progress: [f32; 2],
    bus_range: Option<(f32, f32)>,
}

impl OpenAirWorld {
    /// Needs the GL context (textures, prop models).
    pub fn build(watch: usize, tex: &Textures) -> OpenAirWorld {
        let watch = watch.min(2);
        let wb = build_builder(watch);
        let world = wb.bake(tex);
        let map = map_of(watch);
        let mut models: HashMap<String, Option<Rc<Model>>> = HashMap::new();
        let mut props = Vec::new();
        for p in &map.props {
            let model = models
                .entry(p.model.clone())
                .or_insert_with(|| prop_text(&p.model).and_then(|t| Model::parse(t, None, &|_| None, None)).map(Rc::new))
                .clone();
            if let Some(m) = model {
                let (mut verts, mut idx) = (Vec::new(), Vec::new());
                m.build_scaled(&mut verts, &mut idx, p.pos, p.yaw_deg.to_radians(), Vec3::splat(m.scale * p.scale), 0.0, 0.0, WHITE);
                if !verts.is_empty() {
                    props.push(Mesh { vertices: verts, indices: idx, texture: Some(m.texture.clone()) });
                }
            }
        }
        let props_placed = props.len();
        let mut w = OpenAirWorld { watch, world, props, props_placed, slab_progress: [0.0, 0.0], bus_range: map.bus.map(|b| (b.0, b.1)) };
        w.set_bus(0.0);
        w.set_cans(&[]);
        w.set_pile(tun().n.start_logs as i32);
        w.animate([false, false], 0.0, true);
        w
    }

    pub fn solids(&self) -> &[Aabb] {
        &self.world.solids
    }

    /// The rooftop's barriers slide down to bar (true = shut) and up to open; with `snap` they jump.
    pub fn animate(&mut self, bars: [bool; 2], dt: f32, snap: bool) {
        if self.watch != 0 {
            return;
        }
        for (i, tag) in ["L", "R"].iter().enumerate() {
            let target = if bars[i] { 1.0 } else { 0.0 };
            let p = &mut self.slab_progress[i];
            *p = if snap { target } else { move_toward(*p, target, dt * 4.0) };
            let dy = (SLAB_OPEN_Y - SLAB_CLOSED_Y) * *p;
            self.world.set_offset(&format!("Slab{tag}"), vec3(0.0, -dy, 0.0));
        }
    }

    /// 0 open .. 1 shut, per side.
    pub fn slab_progress(&self) -> [f32; 2] {
        self.slab_progress
    }

    pub fn set_lantern(&mut self, on: bool) {
        if self.watch != 0 {
            return;
        }
        if on {
            self.world.set_material("Kit", "LanternGlow", "light", WHITE3, true);
        } else {
            self.world.set_material("Kit", "LanternGlow", "metal", [0.25, 0.25, 0.28], false);
        }
    }

    pub fn set_lamp(&mut self, on: bool) {
        if self.watch != 1 {
            return;
        }
        if on {
            self.world.set_material("HeatLamp", "LampGlow", "light", [1.0, 0.65, 0.3], true);
        } else {
            self.world.set_material("HeatLamp", "LampGlow", "metal", [0.3, 0.3, 0.33], false);
        }
    }

    /// The campfire's flames and glow follow the fire (0..100).
    pub fn set_fire(&mut self, fire_percent: f64, time_s: f32) {
        if self.watch != 2 {
            return;
        }
        let t = (fire_percent / 100.0).clamp(0.0, 1.0) as f32;
        let q = (time_s * 12.0).floor();
        let fl = 1.0 + 0.07 * (q * 2.3).sin();
        self.world.set_cube_scale("Fire", "FireGlow", vec3((0.4 + 0.6 * t) * fl, 0.15 + 0.85 * t, (0.4 + 0.6 * t) * fl));
        self.world.set_cube_scale("Fire", "FireTipGlow", vec3((0.3 + 0.7 * t) * fl, 0.1 + 0.9 * t * fl, (0.3 + 0.7 * t) * fl));
        self.world.set_visible("Fire", t > 0.01);
    }

    /// The pile on the clearing shrinks as the logs go on the fire (at most ten are drawn).
    pub fn set_pile(&mut self, logs: i32) {
        if self.watch != 2 {
            return;
        }
        for i in 0..10 {
            let s = if (i as i32) < logs.min(10) { Vec3::ONE } else { Vec3::ZERO };
            self.world.set_cube_scale("LogPile", &format!("PileLog{i}"), s);
        }
    }

    /// Which of the three canisters lie on the ground.
    pub fn set_cans(&mut self, present: &[usize]) {
        if self.watch != 1 {
            return;
        }
        for i in 0..3usize {
            self.world.set_visible(&format!("Can{i}"), present.contains(&i));
        }
    }

    /// 0 to 1: the bus comes up the road to the stop.
    pub fn set_bus(&mut self, progress: f64) {
        if let Some((from, to)) = self.bus_range {
            let x = from + (to - from) * progress.clamp(0.0, 1.0) as f32;
            self.world.set_offset("Bus", vec3(x, 0.0, 0.0));
        }
    }

    /// Hide or show one camera's own housing (the feed from it must not look through its own body).
    pub fn set_camera_visible(&mut self, id: &str, visible: bool) {
        self.world.set_visible(&format!("Cam_{id}"), visible);
    }

    /// The lights of this frame: the lantern, the heat lamp and street light, or the fire; the torch spot; the night ambient.
    pub fn lights(&self, st: &SceneState) -> LightSet {
        let map = map_of(self.watch);
        let k = if st.weather_on { 0.8 } else { 1.0 };
        let mut l = LightSet {
            ambient: [map.ambient[0] * k, map.ambient[1] * k, map.ambient[2] * k],
            omni: Vec::new(),
            spot: Vec::new(),
            fog: None,
        };
        for d in &map.lights {
            let energy = match (self.watch, d.id.as_str()) {
                (0, "lantern") => {
                    if st.lantern {
                        d.energy
                    } else {
                        0.05
                    }
                }
                (1, "lamp") => {
                    if st.lamp_on {
                        d.energy
                    } else {
                        0.0
                    }
                }
                (2, "fire") => {
                    let t = (st.fire / 100.0).clamp(0.0, 1.0) as f32;
                    let flicker = 0.95 + 0.05 * (st.time_s * 9.0).sin();
                    (0.1 + d.energy * t) * flicker
                }
                _ => d.energy,
            };
            l.omni.push(Omni { pos: d.pos, color: d.color, energy, range: d.range });
        }
        if let Some(side) = st.torch_side {
            let sx = if side == Side::L { -1.0 } else { 1.0 };
            let pos = st.feet + vec3(0.0, 1.4, 0.0);
            let target = st.feet + vec3(sx * 6.0, 0.6, -1.0);
            l.spot.push(Spot { pos, dir: (target - pos).normalize_or_zero(), color: [1.0, 0.72, 0.4], energy: 4.0, range: 9.0, cos_outer: 38.0f32.to_radians().cos() });
        }
        l
    }

    /// The lights of a camera feed: the same night, but the ambient lifted so the set reads on the monitor.
    pub fn feed_lights(&self, st: &SceneState) -> LightSet {
        let mut l = self.lights(st);
        l.ambient = feed_ambient(l.ambient);
        l
    }

    /// The whole set: cubes, props and the falling weather, from `cam_pos`.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3, weather_time: Option<f32>) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        gfx.shader.begin(lights, cam_pos);
        for m in &self.props {
            draw_mesh(m);
        }
        gfx.shader.end();
        if let Some(t) = weather_time {
            if let Some(mesh) = self.weather_mesh(gfx, t) {
                gfx.shader.begin(lights, cam_pos);
                draw_mesh(&mesh);
                gfx.shader.end();
            }
        }
    }

    /// The snow or rain as one mesh of small unlit cubes.
    pub fn weather_mesh(&self, gfx: &Gfx, time_s: f32) -> Option<Mesh> {
        let (_, _, _, _, _, size, color) = weather_params(self.watch)?;
        let mut verts: Vec<Vertex> = Vec::new();
        let mut idx: Vec<u16> = Vec::new();
        for p in particle_positions(self.watch, time_s) {
            let c = Cube::new("Flake", size, p).kind("light").tint(color).emissive();
            let (v, ix) = cube_geometry(&c, size, Vec3::ZERO);
            let base = verts.len() as u16;
            verts.extend(v);
            idx.extend(ix.iter().map(|k| k + base));
        }
        Some(Mesh { vertices: verts, indices: idx, texture: Some(gfx.white.clone()) })
    }
}

/// `a` moved toward `b` by at most `step`.
pub fn move_toward(a: f32, b: f32, step: f32) -> f32 {
    if (b - a).abs() <= step {
        b
    } else {
        a + step * (b - a).signum()
    }
}

/// The night ambient as a camera feed shows it: lifted (the monitors are not dark-adapted), blue kept, never above 1.
pub fn feed_ambient(a: [f32; 3]) -> [f32; 3] {
    [(a[0] * 1.9 + 0.04).min(1.0), (a[1] * 1.9 + 0.04).min(1.0), (a[2] * 1.9 + 0.04).min(1.0)]
}

/// The camera position the feed is rendered from: just in front of the housing, so the housing is not in the picture.
pub fn feed_position(c: &CamDef) -> Vec3 {
    c.pos + (c.target - c.pos).normalize_or_zero() * 0.3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::RAMPS;

    fn cubes_of(wb: &WorldBuilder) -> Vec<(&str, &Cube)> {
        wb.groups.iter().flat_map(|g| g.cubes.iter().map(move |c| (g.name.as_str(), c))).collect()
    }

    #[test]
    fn feed_ambient_is_lifted_but_stays_a_night_blue() {
        for watch in 0..3usize {
            let a = map_of(watch).ambient;
            let f = feed_ambient(a);
            for i in 0..3 {
                assert!(f[i] > a[i] && f[i] <= 1.0);
            }
            assert!(f[2] > f[0], "still a blue night");
            assert!(f[0] + f[1] + f[2] >= 1.3, "readable on the monitor");
        }
    }

    #[test]
    fn each_set_has_groups_cubes_and_solids() {
        for watch in 0..3usize {
            let wb = build_builder(watch);
            let cubes = cubes_of(&wb);
            assert!(cubes.len() > 60, "watch {watch} is built of {} cubes", cubes.len());
            assert!(!wb.solids.is_empty() || cubes.iter().any(|(_, c)| c.solid));
            for cam in &map_of(watch).cameras {
                assert!(wb.groups.iter().any(|g| g.name == format!("Cam_{}", cam.id)), "camera {} is built", cam.id);
            }
        }
        let roof = build_builder(0);
        for name in ["Roof", "SlabL", "SlabR", "Catwalks", "Kit", "Skyline"] {
            assert!(roof.groups.iter().any(|g| g.name == name), "{name}");
        }
        let bus = build_builder(1);
        for name in ["Street", "Shelter", "HeatLamp", "Can0", "Can1", "Can2", "Bus"] {
            assert!(bus.groups.iter().any(|g| g.name == name), "{name}");
        }
        let camp = build_builder(2);
        for name in ["Clearing", "Fire", "Trees", "LogPile", "Firewood", "TorchPostL", "TorchPostR", "Tent"] {
            assert!(camp.groups.iter().any(|g| g.name == name), "{name}");
        }
    }

    #[test]
    fn the_campfire_has_the_46_trees_of_the_godot_ring() {
        let camp = build_builder(2);
        let trees = camp.groups.iter().find(|g| g.name == "Trees").expect("trees");
        assert_eq!(trees.cubes.iter().filter(|c| c.name == "TreeTrunk").count(), 46);
        assert_eq!(trees.cubes.iter().filter(|c| c.name == "TreeCrown").count(), 46 * 4);
        let pile = camp.groups.iter().find(|g| g.name == "LogPile").expect("pile");
        assert_eq!(pile.cubes.len(), 10);
    }

    #[test]
    fn only_the_glowing_things_glow() {
        for watch in 0..3usize {
            let wb = build_builder(watch);
            for (g, c) in cubes_of(&wb) {
                if c.emissive {
                    assert!(c.name.contains("Glow") || c.name.contains("Sign"), "{} in {g} glows by accident on watch {watch}", c.name);
                }
            }
        }
    }

    #[test]
    fn every_cube_has_a_known_material() {
        for watch in 0..3usize {
            let wb = build_builder(watch);
            for (g, c) in cubes_of(&wb) {
                assert!(RAMPS.iter().any(|(k, _)| *k == c.kind), "{} in {g} has kind '{}'", c.name, c.kind);
            }
        }
    }

    #[test]
    fn the_godot_stations_stand_on_something() {
        // the rooftop's stations are in reach of the roof floor
        let map = map_of(0);
        for s in &map.stations {
            assert!(s.pos.x.abs() < ROOF_X && s.pos.z.abs() < ROOF_Z, "{} is on the roof", s.id);
        }
        // the heat lamp station sits on the lamp post
        assert!((map_of(1).station("lamp").unwrap().pos.x - -1.5).abs() < 0.01);
    }

    #[test]
    fn every_prop_model_is_embedded_and_parses_as_json() {
        for name in PROP_NAMES {
            let text = prop_text(name).unwrap_or_else(|| panic!("{name} is embedded"));
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(v["elements"].is_array(), "{name} has elements");
        }
        for w in 0..3usize {
            for p in &map_of(w).props {
                assert!(prop_text(&p.model).is_some(), "watch {w}: prop {} is not embedded", p.model);
            }
        }
    }

    #[test]
    fn props_in_reach_block_the_player() {
        let def = PropDef { model: "locker".to_string(), pos: vec3(4.6, 0.0, -4.4), yaw_deg: 0.0, scale: 1.0 };
        let a = prop_aabb(&def).expect("locker blocks");
        assert!((a.max.y - 2.0).abs() < 0.001 && a.min.x < 4.6 && a.max.x > 4.6);
        let turned = PropDef { yaw_deg: 90.0, ..def.clone() };
        let b = prop_aabb(&turned).expect("blocks");
        assert!((b.max.x - b.min.x) > (a.max.x - a.min.x) - 0.001, "a quarter turn swaps the footprint");
        assert!(prop_aabb(&PropDef { model: "cableCoil".to_string(), ..def }).is_none(), "a coil is flat");
    }

    #[test]
    fn weather_falls_inside_its_box_and_never_below_the_ground() {
        for (watch, ex, ez, cz) in [(1usize, 14.0f32, 10.0f32, 0.0f32), (2, 14.0, 14.0, -2.0)] {
            for t in [0.0f32, 1.3, 7.7, 100.0] {
                let ps = particle_positions(watch, t);
                assert!(ps.len() > 100);
                for p in ps {
                    assert!(p.y > -0.001 && p.y <= 8.001, "{p:?}");
                    assert!(p.x.abs() <= ex + 0.5 && (p.z - cz).abs() <= ez + 0.001);
                }
            }
        }
        assert!(particle_positions(0, 1.0).is_empty(), "no weather on the roof");
        let a = particle_positions(2, 1.0);
        let b = particle_positions(2, 1.0);
        assert_eq!(a, b, "a pure function of time");
        assert_ne!(particle_positions(2, 1.0), particle_positions(2, 1.2));
    }

    #[test]
    fn the_hash_is_in_range_and_varies() {
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..200 {
            let h = hash01(i, 3);
            assert!((0.0..1.0).contains(&h));
            seen.insert((h * 1000.0) as i32);
        }
        assert!(seen.len() > 100);
    }

    #[test]
    fn slabs_move_toward_their_target() {
        assert_eq!(move_toward(0.0, 1.0, 0.25), 0.25);
        assert_eq!(move_toward(0.9, 1.0, 0.25), 1.0);
        assert_eq!(move_toward(1.0, 0.0, 0.4), 0.6);
    }

    #[test]
    fn the_feed_camera_sits_in_front_of_its_housing() {
        let c = map_of(0).camera("roof").expect("roof camera");
        let p = feed_position(c);
        assert!((p.distance(c.pos) - 0.3).abs() < 0.001);
        assert!(p.distance(c.target) < c.pos.distance(c.target));
    }
}
