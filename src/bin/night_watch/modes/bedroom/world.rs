//! Bedtime's 3D house wing (bedroom_office.gd + bedroom_wing.gd): the child's bedroom with its closet, the long hall and the
//! rooms off it, from shaded cubes at the Godot coordinates, with bbmodel props from `last_route/assets/props` in every room.
//! `build_cubes` and `lights_for` are pure (tests run them); only `BedroomWorld::new` and `draw` touch GL.
#![allow(dead_code)]

use super::map::{BedroomMap, PropDef, RoomDef};
use super::sim::Light;
use crate::gfx::{Cube, Gfx, Group, LightSet, Omni, Spot, World, WorldBuilder};
use crate::hunters::bbmodel::Model;
use macroquad::prelude::*;

type V3 = [f32; 3];

/// Cube options, like the `opts` dictionary of `NightWatchArt.box`.
#[derive(Clone, Copy)]
struct O {
    kind: &'static str,
    tint: V3,
    solid: bool,
    ao: f32,
}

const D: O = O { kind: "", tint: [1.0, 1.0, 1.0], solid: false, ao: 0.72 };

fn c(g: &mut Group, name: &str, size: V3, pos: V3, o: O) {
    let mut cube = Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).tint(o.tint).ao(o.ao);
    if !o.kind.is_empty() {
        cube = cube.kind(o.kind);
    }
    if o.solid {
        cube = cube.solid();
    }
    g.add(cube);
}

fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

/// The three states of the closet door: held shut (A), standing ajar while nobody holds it, flung open in the death scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Closet {
    Held,
    Ajar,
    Wide,
}

impl Closet {
    pub fn group(self) -> &'static str {
        match self {
            Closet::Held => "ClosetDoorHeld",
            Closet::Ajar => "ClosetDoorAjar",
            Closet::Wide => "ClosetDoorWide",
        }
    }
    pub const ALL: [Closet; 3] = [Closet::Held, Closet::Ajar, Closet::Wide];
}

/// Where the closet door's slab centre is when the door has swung `theta` radians about its hinge (relative to the closet
/// node at (-2.6, 0, closet_z)). The hinge is at (0.05, 0, 0.62) and the slab hangs 0.6 m to its north.
pub fn closet_slab_offset(theta: f32) -> Vec3 {
    let pivot = vec3(0.05, 0.0, 0.62);
    pivot + Quat::from_rotation_y(theta) * vec3(0.0, 1.1, -0.6)
}

pub fn closet_handle_offset(theta: f32) -> Vec3 {
    let pivot = vec3(0.05, 0.0, 0.62);
    pivot + Quat::from_rotation_y(theta) * vec3(0.07, 1.1, -1.05)
}

// ------------------------------------------------------------------------------------------------ the bedroom

pub fn build_cubes(map: &BedroomMap) -> WorldBuilder {
    let mut wb = WorldBuilder::new();
    shell(&mut wb, map);
    bed(&mut wb);
    closet(&mut wb, map);
    hall(&mut wb, map);
    furniture(&mut wb);
    for r in &map.rooms {
        if matches!(r.id.as_str(), "kids_room" | "bathroom" | "landing" | "stairs" | "living_room" | "galley") {
            wing_room(&mut wb, map, r);
        }
    }
    wb
}

fn shell(wb: &mut WorldBuilder, map: &BedroomMap) {
    let half = map.half;
    let height = map.height;
    let g = wb.group("Shell", Vec3::ZERO);
    let tint: V3 = [0.85, 0.8, 1.15];
    let span = half * 2.0 + 0.4;
    c(g, "Floor", [span, 0.2, span], [0.0, -0.1, 0.0], O { solid: true, kind: "wood", tint: [0.8, 0.7, 0.65], ..D });
    c(g, "Ceiling", [span, 0.2, span], [0.0, height + 0.1, 0.0], O { tint: [0.5, 0.5, 0.6], ..D });
    c(g, "WallNorth", [span, height, 0.2], [0.0, height * 0.5, -half - 0.1], O { solid: true, tint, ..D });
    c(g, "WallSouth", [span, height, 0.2], [0.0, height * 0.5, half + 0.1], O { solid: true, tint, ..D });
    // the west wall holds the closet recess; the east wall has the hall doorway
    for side in [-1.0f32, 1.0] {
        let tag = if side < 0.0 { "West" } else { "East" };
        let x = side * (half + 0.1);
        let door_z = map.closet_z;
        let gap = if side > 0.0 { map.hall_gap } else { map.closet_gap };
        let a_len = (door_z - gap * 0.5) - (-half);
        let b_len = half - (door_z + gap * 0.5);
        c(g, &format!("Wall{tag}A"), [0.2, height, a_len], [x, height * 0.5, -half + a_len * 0.5], O { solid: true, tint, ..D });
        c(g, &format!("Wall{tag}B"), [0.2, height, b_len], [x, height * 0.5, half - b_len * 0.5], O { solid: true, tint, ..D });
        c(g, &format!("Wall{tag}Top"), [0.2, height - 2.2, gap], [x, 2.2 + (height - 2.2) * 0.5, door_z], O { tint, ..D });
    }
    for z in [-half + 0.04, half - 0.04] {
        c(g, "Baseboard", [half * 2.0, 0.2, 0.08], [0.0, 0.1, z], O { tint: [0.7, 0.7, 0.75], ..D });
    }
    c(g, "CrownTrim", [half * 2.0, 0.1, 0.08], [0.0, height - 0.05, -half + 0.04], O { tint: [0.8, 0.8, 0.85], ..D });
    // the rug
    let rug = wb.group("Rug", vec3(0.0, 0.0, -0.6));
    c(rug, "RugMat", [2.6, 0.03, 2.0], [0.0, 0.0, 0.0], O { kind: "fabric", tint: [0.9, 0.6, 0.7], ao: 1.0, ..D });
    c(rug, "RugTrim", [2.7, 0.032, 0.1], [0.0, 0.0, 0.95], O { kind: "label", tint: [0.9, 0.9, 0.8], ao: 1.0, ..D });
    c(rug, "RugTrim", [2.7, 0.032, 0.1], [0.0, 0.0, -0.95], O { kind: "label", tint: [0.9, 0.9, 0.8], ao: 1.0, ..D });
    // the window (north wall) and its curtains
    let win = wb.group("Window", vec3(-0.2, 1.6, -half + 0.12));
    c(win, "WindowFrame", [1.4, 1.1, 0.06], [0.0, 0.0, 0.0], O { tint: [0.8, 0.7, 0.6], ..D });
    c(win, "WindowGlowPane", [1.2, 0.9, 0.03], [0.0, 0.0, 0.04], O { tint: [0.35, 0.45, 0.8], ..D });
    for x in [-0.85f32, 0.85] {
        c(win, "CurtainDrape", [0.35, 1.5, 0.1], [x, -0.1, 0.1], O { tint: [0.7, 0.55, 0.95], ..D });
    }
}

fn bed(wb: &mut WorldBuilder) {
    let g = wb.group("Bed", vec3(0.0, 0.0, 1.3));
    c(g, "BedFrame", [1.7, 0.35, 2.3], [0.0, 0.3, 0.0], O { solid: true, ..D });
    c(g, "Mattress", [1.6, 0.25, 2.15], [0.0, 0.6, 0.0], O { kind: "fabric", tint: [0.7, 0.8, 1.2], ..D });
    c(g, "BlanketTop", [1.64, 0.08, 1.2], [0.0, 0.74, -0.45], O { kind: "fabric", tint: [0.9, 0.55, 0.7], ..D });
    c(g, "Headboard", [1.7, 0.7, 0.1], [0.0, 0.75, 1.2], D);
    c(g, "Footboard", [1.7, 0.5, 0.1], [0.0, 0.55, -1.2], D);
    for x in [-0.4f32, 0.4] {
        c(g, "PillowCushion", [0.6, 0.14, 0.4], [x, 0.78, 0.85], O { kind: "fabric", tint: [1.3, 1.3, 1.4], ..D });
    }
}

fn closet(wb: &mut WorldBuilder, map: &BedroomMap) {
    let origin = vec3(-map.half - 0.1, 0.0, map.closet_z);
    let g = wb.group("Closet", origin);
    c(g, "ClosetInside", [1.0, 2.3, 1.2], [-0.6, 1.15, 0.0], O { tint: [0.3, 0.3, 0.35], ..D });
    c(g, "ClosetFrame", [0.15, 2.3, 0.1], [0.0, 1.15, -0.65], D);
    c(g, "ClosetFrame", [0.15, 2.3, 0.1], [0.0, 1.15, 0.65], D);
    c(g, "ClosetFrame", [0.15, 0.1, 1.4], [0.0, 2.3, 0.0], D);
    c(g, "ClothesBox", [0.9, 0.5, 0.9], [-0.35, 0.25, 0.0], O { tint: [0.7, 0.7, 0.8], ..D });
    // the door, once per state (the cubes cannot rotate at run time): held shut, ajar, flung open
    for (state, theta) in [(Closet::Held, 0.0), (Closet::Ajar, map.closet_ajar), (Closet::Wide, map.closet_wide)] {
        let d = wb.group(state.group(), origin);
        let slab = closet_slab_offset(theta);
        let handle = closet_handle_offset(theta);
        d.add(
            Cube::new("ClosetDoorSlab", vec3(0.08, 2.2, 1.2), slab).kind("wood").tint([1.0, 0.85, 0.75]).rot_y(theta),
        );
        d.add(Cube::new("ClosetDoorHandle", vec3(0.1, 0.12, 0.08), handle).kind("metal").rot_y(theta));
    }
}

fn hall(wb: &mut WorldBuilder, map: &BedroomMap) {
    let half = map.half;
    let hx = half + 4.0;
    let height = map.height;
    let g = wb.group("Hall", Vec3::ZERO);
    c(g, "HallFloor", [8.0, 0.2, 1.8], [hx + 0.2, -0.1, -0.9], O { solid: true, tint: [0.7, 0.65, 0.7], ..D });
    c(g, "HallCeiling", [8.0, 0.2, 1.8], [hx + 0.2, height + 0.1, -0.9], O { tint: [0.5, 0.5, 0.55], ..D });
    // the hall's walls have doorways where the wing's rooms open off it: north the bathroom and the landing, south the kids'
    // room; the east end is open to the living room
    let hall_rect = map.rect("hall");
    let sides = map.sides_of("hall");
    let c0 = hall_rect.center();
    let north: Vec<f32> = sides.north.iter().map(|o| o + c0.x).collect();
    let south: Vec<f32> = sides.south.iter().map(|o| o + c0.x).collect();
    hall_wall(g, half, half + 8.2, -1.9, &north, "HallWallNorth", height);
    hall_wall(g, half, half + 8.2, 0.1, &south, "HallWallSouth", height);
    c(g, "HallRunner", [7.6, 0.03, 0.7], [hx, 0.02, -0.9], O { kind: "fabric", tint: [0.7, 0.5, 0.9], ao: 1.0, ..D });
    for fx in [3.6f32, 7.5] {
        c(g, "HallFrame", [0.5, 0.8, 0.06], [fx, 1.7, -1.78], O { tint: [0.9, 0.8, 0.7], ..D });
    }
    c(g, "HallBox", [0.2, 0.2, 0.2], [half + 6.0, 0.1, -1.6], D);
}

/// One hall wall (x from `x0` to `x1`) with a doorway of 1.6 m at each x in `gaps`.
fn hall_wall(g: &mut Group, x0: f32, x1: f32, z: f32, gaps: &[f32], name: &str, height: f32) {
    let mut gaps: Vec<f32> = gaps.to_vec();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut cursor = x0;
    let lintel_h = height - 2.2;
    let tint: V3 = [0.8, 0.75, 0.95];
    for gx in gaps {
        let lo = gx - 0.8;
        c(g, name, [lo - cursor, height, 0.2], [(cursor + lo) * 0.5, height * 0.5, z], O { solid: true, tint, ..D });
        c(g, &format!("{name}Lintel"), [1.6, lintel_h, 0.2], [gx, 2.2 + lintel_h * 0.5, z], O { tint: [0.75, 0.7, 0.9], ..D });
        cursor = gx + 0.8;
    }
    c(g, name, [x1 - cursor, height, 0.2], [(cursor + x1) * 0.5, height * 0.5, z], O { solid: true, tint, ..D });
}

/// The bedroom's boxed furniture that no bbmodel replaces (the rest is in the prop list of `data/bedroom/map.json`).
fn furniture(wb: &mut WorldBuilder) {
    let half = 2.5f32;
    let shelf = wb.group("ToyShelf", vec3(-1.2, 0.0, -half + 0.46));
    c(shelf, "ShelfBody", [1.4, 1.2, 0.35], [0.0, 0.6, 0.0], O { solid: true, ..D });
    for (i, k) in ["red", "green", "light", "glass"].iter().enumerate() {
        c(shelf, "ToyCube", [0.25, 0.25, 0.25], [-0.5 + i as f32 * 0.33, 1.32, 0.0], O { kind: *k, tint: [0.8, 0.8, 0.9], ..D });
    }
    // posters on the west and north walls
    for i in 0..3 {
        let (x, z) = if i < 1 { (-half + 0.08, -0.2) } else { (1.4 + (i as f32 - 1.0) * 0.8, -half + 0.1) };
        let size = if i < 1 { [0.04, 0.7, 0.5] } else { [0.5, 0.7, 0.04] };
        let p = wb.group(&format!("Poster{i}"), vec3(x, 1.7, z));
        c(p, "PosterPage", size, [0.0, 0.0, 0.0], O { kind: "fabric", tint: [1.0 + 0.2 * i as f32, 0.8, 1.3 - 0.2 * i as f32], ..D });
    }
    // a toy block on the toy chest, and the clutter on the floor
    let block = wb.group("ToyBlock", vec3(-1.8, 0.75, 2.2));
    c(block, "ToyBlock", [0.2, 0.2, 0.2], [0.0, 0.0, 0.0], O { kind: "red", ..D });
    let clutter = wb.group("Clutter", vec3(0.6, 0.0, 0.2));
    c(clutter, "ClutterSock", [0.3, 0.12, 0.2], [0.0, 0.06, 0.0], O { kind: "fabric", tint: [1.2, 0.9, 1.1], ..D });
    c(clutter, "ClutterShoe", [0.25, 0.08, 0.3], [0.5, 0.04, 0.4], O { kind: "plastic", ..D });
    c(clutter, "ClutterBall", [0.18, 0.18, 0.18], [-0.4, 0.09, -0.3], O { kind: "red", ..D });
}

// ------------------------------------------------------------------------------------------------ the wing

fn theme_tint(theme: &str) -> V3 {
    match theme {
        "kids" => [0.8, 1.1, 1.3],
        "bath" => [0.85, 1.25, 1.25],
        "landing" => [1.0, 0.9, 1.2],
        "stairs" => [0.9, 0.85, 1.1],
        "living" => [1.2, 0.95, 1.0],
        "galley" => [1.2, 1.15, 0.85],
        _ => [1.0, 1.0, 1.0],
    }
}

fn wing_room(wb: &mut WorldBuilder, map: &BedroomMap, r: &RoomDef) {
    let height = map.height;
    let (hw, hd) = (r.width() * 0.5, r.depth() * 0.5);
    let origin = r.center();
    let tint = theme_tint(&r.theme);
    let shell = wb.group(&format!("Room_{}", r.id), origin);
    c(
        shell,
        "Floor",
        [r.width(), 0.2, r.depth()],
        [0.0, -0.1, 0.0],
        O { solid: true, kind: if r.theme == "bath" { "concrete" } else { "wood" }, tint: mul(tint, [0.9, 0.9, 0.9]), ..D },
    );
    c(shell, "Ceiling", [r.width(), 0.2, r.depth()], [0.0, height + 0.1, 0.0], O { tint: mul(tint, [0.55, 0.55, 0.6]), ..D });
    let sides = map.sides_of(&r.id);
    for (side, doors) in [("N", &sides.north), ("S", &sides.south), ("W", &sides.west), ("E", &sides.east)] {
        wing_wall(shell, map, side, hw, hd, doors, tint);
    }
    c(shell, "Baseboard", [r.width() - 0.4, 0.2, 0.08], [0.0, 0.1, -hd + 0.14], O { tint: [0.7, 0.7, 0.75], ..D });
    let at = |x: f32, y: f32, z: f32| origin + vec3(x, y, z);
    match r.theme.as_str() {
        "kids" => {
            // the toy chest is a prop; its three toys sit on top
            let chest = wb.group(&format!("Room_{}/Toys", r.id), at(hw - 0.8, 0.0, -hd + 0.6));
            for (i, k) in ["red", "green", "light"].iter().enumerate() {
                c(chest, "ToyCube", [0.22, 0.22, 0.22], [-0.25 + i as f32 * 0.25, 0.74, 0.0], O { kind: *k, ..D });
            }
            let rug = wb.group(&format!("Room_{}/Rug", r.id), at(0.3, 0.01, 0.0));
            c(rug, "RugMat", [1.6, 0.03, 1.6], [0.0, 0.0, 0.0], O { kind: "fabric", tint: [1.2, 0.7, 0.8], ao: 1.0, ..D });
        }
        "landing" => {
            let rail = wb.group(&format!("Room_{}/Railing", r.id), at(0.0, 0.0, hd - 0.4));
            c(rail, "RailTop", [hw * 2.0 - 0.4, 0.06, 0.06], [0.0, 1.0, 0.0], O { kind: "metal", ..D });
            for i in 0..5 {
                let x = -hw + 0.4 + i as f32 * (hw * 2.0 - 0.8) / 4.0;
                c(rail, "RailPost", [0.05, 1.0, 0.05], [x, 0.5, 0.0], O { kind: "metal", ..D });
            }
            let rug = wb.group(&format!("Room_{}/Rug", r.id), at(0.0, 0.01, -0.4));
            c(rug, "RugMat", [1.4, 0.03, 1.8], [0.0, 0.0, 0.0], O { kind: "fabric", tint: [0.8, 0.6, 1.0], ao: 1.0, ..D });
            let win = wb.group(&format!("Room_{}/Window", r.id), at(hw - 0.2, 1.6, 0.0));
            c(win, "WindowGlowPane", [0.05, 0.9, 0.9], [0.0, 0.0, 0.0], O { tint: [0.35, 0.45, 0.8], ..D });
        }
        "stairs" => {
            let steps = wb.group(&format!("Room_{}/Steps", r.id), at(0.0, 0.0, hd - 0.4));
            for i in 0..7 {
                let n = (i + 1) as f32;
                c(steps, "StepBlock", [hw * 2.0 - 0.8, 0.2 * n, 0.45], [0.0, 0.1 * n, -(i as f32) * 0.5], O { kind: "wood", ..D });
            }
            let rail = wb.group(&format!("Room_{}/StairRail", r.id), at(hw - 0.3, 0.0, hd - 0.4));
            c(rail, "RailTop", [0.06, 0.06, 3.4], [0.0, 1.1, -1.7], O { kind: "metal", ..D });
        }
        _ => {}
    }
}

fn wing_wall(g: &mut Group, map: &BedroomMap, side: &str, hw: f32, hd: f32, doors: &[f32], tint: V3) {
    let along_x = side == "N" || side == "S";
    let half = if along_x { hw } else { hd };
    let fixed = if along_x {
        (hd - 0.1) * if side == "N" { -1.0 } else { 1.0 }
    } else {
        (hw - 0.1) * if side == "W" { -1.0 } else { 1.0 }
    };
    let mut cuts: Vec<f32> = doors.to_vec();
    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let gap = map.door_gap;
    let top = map.door_top;
    let mut cursor = -half;
    for dc in cuts {
        wing_piece(g, along_x, fixed, cursor, dc - gap * 0.5, tint, map.height);
        let h = map.height - top;
        let (size, pos) = if along_x {
            ([gap, h, 0.2], [dc, top + h * 0.5, fixed])
        } else {
            ([0.2, h, gap], [fixed, top + h * 0.5, dc])
        };
        c(g, &format!("Wall{side}Lintel"), size, pos, O { tint: mul(tint, [0.9, 0.9, 0.9]), ..D });
        cursor = dc + gap * 0.5;
    }
    wing_piece(g, along_x, fixed, cursor, half, tint, map.height);
}

fn wing_piece(g: &mut Group, along_x: bool, fixed: f32, a: f32, b: f32, tint: V3, height: f32) {
    if b - a < 0.05 {
        return;
    }
    let mid = (a + b) * 0.5;
    let (size, pos) = if along_x {
        ([b - a, height, 0.2], [mid, height * 0.5, fixed])
    } else {
        ([0.2, height, b - a], [fixed, height * 0.5, mid])
    };
    c(g, "WallPiece", size, pos, O { solid: true, tint, ..D });
}

// ------------------------------------------------------------------------------------------------ lights

fn spot_of(eye: Vec3, d: &super::map::SpotDef) -> Spot {
    let pos = eye + d.offset;
    Spot {
        pos,
        dir: (d.target - pos).normalize_or_zero(),
        color: d.color,
        energy: d.energy,
        range: d.range,
        cos_outer: d.half_angle_deg.to_radians().cos(),
    }
}

fn omni_of(d: &super::map::OmniDef) -> Omni {
    Omni { pos: d.pos, color: d.color, energy: d.energy, range: d.range }
}

/// The dark house: a faint window glow, the hall and living room glows, and the flashlight that is on. `reveal` is the admin
/// switch that lights the room so you can see who stands where.
pub fn lights_for(map: &BedroomMap, light: Light, reveal: bool) -> LightSet {
    let l = &map.lights;
    let k = if reveal { l.reveal_energy } else { l.ambient_energy };
    let mut set = LightSet {
        ambient: [l.ambient[0] * k, l.ambient[1] * k, l.ambient[2] * k],
        omni: vec![omni_of(&l.moon), omni_of(&l.hall_glow), omni_of(&l.living_glow)],
        spot: Vec::new(),
        fog: None,
    };
    match light {
        Light::Hall => set.spot.push(spot_of(map.eye, &l.hall_spot)),
        Light::Bed => set.spot.push(spot_of(map.eye, &l.bed_spot)),
        Light::Dark => {}
    }
    set
}

// ------------------------------------------------------------------------------------------------ props

/// The bbmodel text of a prop from `last_route/assets/props`, by file stem.
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
        "bedroomNightstand",
        "bedsideLamp",
        "woodenChair",
        "bedroomDresser",
        "normalBookcase",
        "toyChest",
        "ceilingLight",
        "bedroomWallClock",
        "laundryHamper",
        "normalEntryBench",
        "floorLamp",
        "singleBed",
        "houseBathtub",
        "bathroomSink",
        "bathroomMirror",
        "houseToilet",
        "houseBathMat",
        "pottedPlant",
        "loungeCouch",
        "normalTvConsole",
        "normalCoffeeTable",
        "normalKitchenCounter",
        "normalKitchenSink",
        "normalKitchenStove",
        "normalKitchenFridge",
        "woodenTable",
    )
}

/// One prop as a mesh: the model stands on its placement point (props are authored around the origin, front toward +z),
/// turned `yaw_deg` about the vertical axis.
fn prop_mesh(model: &Model, p: &PropDef) -> Mesh {
    let (mut verts, mut idx) = (Vec::new(), Vec::new());
    model.build_scaled(&mut verts, &mut idx, p.pos, p.yaw_deg.to_radians(), Vec3::splat(model.scale * p.scale), 0.0, 0.0, WHITE);
    Mesh { vertices: verts, indices: idx, texture: Some(model.texture.clone()) }
}

// ------------------------------------------------------------------------------------------------ the baked world

pub struct BedroomWorld {
    pub world: World,
    props: Vec<Mesh>,
    closet: Closet,
}

impl BedroomWorld {
    /// Needs the GL context (textures).
    pub fn new(gfx: &Gfx, map: &BedroomMap) -> BedroomWorld {
        let world = build_cubes(map).bake(&gfx.tex);
        let mut cache: Vec<(String, Option<Model>)> = Vec::new();
        let mut props = Vec::new();
        for p in &map.props {
            if !cache.iter().any(|(k, _)| *k == p.model) {
                let m = prop_text(&p.model).and_then(|t| Model::parse(t, None, &|_| None, None));
                if let Some(m) = &m {
                    m.texture.set_filter(FilterMode::Nearest);
                }
                cache.push((p.model.clone(), m));
            }
            if let Some((_, Some(m))) = cache.iter().find(|(k, _)| *k == p.model) {
                let mesh = prop_mesh(m, p);
                if !mesh.vertices.is_empty() {
                    props.push(mesh);
                }
            }
        }
        let mut me = BedroomWorld { world, props, closet: Closet::Ajar };
        me.set_closet(Closet::Ajar);
        me
    }

    pub fn closet(&self) -> Closet {
        self.closet
    }

    /// Show the door in one state and hide the other two.
    pub fn set_closet(&mut self, state: Closet) {
        self.closet = state;
        for s in Closet::ALL {
            self.world.set_visible(s.group(), s == state);
        }
    }

    pub fn prop_count(&self) -> usize {
        self.props.len()
    }

    /// The cubes, then the props, under the same lights.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        gfx.shader.begin(lights, cam_pos);
        for m in &self.props {
            crate::gfx::mesh::draw_mesh_safe(m);
        }
        gfx.shader.end();
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::map::map;

    fn group<'a>(wb: &'a WorldBuilder, name: &str) -> &'a Group {
        wb.groups.iter().find(|g| g.name == name).unwrap_or_else(|| panic!("no group {name}"))
    }

    #[test]
    fn the_world_has_every_part_of_the_house() {
        let wb = build_cubes(map());
        for name in ["Shell", "Rug", "Window", "Bed", "Closet", "Hall", "ToyShelf", "ClosetDoorHeld", "ClosetDoorAjar", "ClosetDoorWide"] {
            assert!(!group(&wb, name).cubes.is_empty(), "{name}");
        }
        for r in ["kids_room", "bathroom", "landing", "stairs", "living_room", "galley"] {
            assert!(!group(&wb, &format!("Room_{r}")).cubes.is_empty(), "{r}");
        }
        let total: usize = wb.groups.iter().map(|g| g.cubes.len()).sum();
        assert!(total > 120, "{total}");
    }

    #[test]
    fn the_bedroom_shell_matches_the_godot_numbers() {
        let wb = build_cubes(map());
        let s = group(&wb, "Shell");
        let floor = s.cubes.iter().find(|c| c.name == "Floor").expect("floor");
        assert_eq!(floor.size, vec3(5.4, 0.2, 5.4));
        assert_eq!(floor.pos, vec3(0.0, -0.1, 0.0));
        let north = s.cubes.iter().find(|c| c.name == "WallNorth").expect("north wall");
        assert_eq!(north.pos, vec3(0.0, 1.8, -2.6));
        // east wall: a 1.1 m doorway at z = -0.9
        let ea = s.cubes.iter().find(|c| c.name == "WallEastA").expect("east A");
        let eb = s.cubes.iter().find(|c| c.name == "WallEastB").expect("east B");
        assert!((ea.size.z - (-0.9 - 0.55 + 2.5)).abs() < 1e-5);
        assert!((eb.size.z - (2.5 - (-0.9 + 0.55))).abs() < 1e-5);
        // the west wall's closet recess is 1.2 m
        let wa = s.cubes.iter().find(|c| c.name == "WallWestA").expect("west A");
        assert!((wa.size.z - (-0.9 - 0.6 + 2.5)).abs() < 1e-5);
    }

    #[test]
    fn the_hall_has_doorways_for_the_rooms_off_it() {
        let wb = build_cubes(map());
        let h = group(&wb, "Hall");
        let lintels = h.cubes.iter().filter(|c| c.name.ends_with("Lintel")).count();
        assert_eq!(lintels, 3, "bathroom, landing and kids' room");
        let north_pieces = h.cubes.iter().filter(|c| c.name == "HallWallNorth").count();
        assert_eq!(north_pieces, 3);
        // the wall pieces never cover a doorway
        for c in h.cubes.iter().filter(|c| c.name == "HallWallNorth") {
            let (a, b) = (c.pos.x - c.size.x * 0.5, c.pos.x + c.size.x * 0.5);
            for gx in [5.5f32, 9.3] {
                assert!(!(a < gx - 0.01 && b > gx + 0.01), "wall covers the doorway at {gx}");
            }
        }
    }

    #[test]
    fn wing_rooms_cut_their_doorways() {
        let wb = build_cubes(map());
        // the living room has a west doorway (to the hall) and a north one (to the galley)
        let r = group(&wb, "Room_living_room");
        let lintels: Vec<_> = r.cubes.iter().filter(|c| c.name.ends_with("Lintel")).collect();
        assert_eq!(lintels.len(), 2);
        // the galley has one doorway (south), the stairs one (south)
        for room in ["Room_galley", "Room_stairs", "Room_kids_room", "Room_bathroom"] {
            let n = group(&wb, room).cubes.iter().filter(|c| c.name.ends_with("Lintel")).count();
            assert_eq!(n, 1, "{room}");
        }
        assert_eq!(group(&wb, "Room_landing").cubes.iter().filter(|c| c.name.ends_with("Lintel")).count(), 2);
    }

    #[test]
    fn the_closet_door_swings_about_its_hinge() {
        // held shut: the slab hangs 0.6 m north of the hinge
        let held = closet_slab_offset(0.0);
        assert!((held - vec3(0.05, 1.1, 0.02)).length() < 1e-5, "{held:?}");
        // wide open it has swung out into the room (toward +x) and the handle with it
        let wide = closet_slab_offset(-1.3);
        assert!(wide.x > held.x + 0.4, "{wide:?}");
        assert!(closet_handle_offset(-1.3).x > closet_handle_offset(0.0).x + 0.5);
        // the handle stays 0.45 m past the slab's centre line from the hinge
        let wb = build_cubes(map());
        for (name, theta) in [("ClosetDoorHeld", 0.0), ("ClosetDoorAjar", -0.22), ("ClosetDoorWide", -1.3)] {
            let g = group(&wb, name);
            assert_eq!(g.cubes.len(), 2, "{name}");
            assert!((g.cubes[0].rot_y - theta).abs() < 1e-6, "{name}");
            assert_eq!(g.origin, vec3(-2.6, 0.0, -0.9));
        }
    }

    #[test]
    fn lights_follow_the_flashlight_that_is_on() {
        let m = map();
        let dark = lights_for(m, Light::Dark, false);
        assert!(dark.spot.is_empty());
        assert_eq!(dark.omni.len(), 3);
        assert!((dark.ambient[2] - 0.5 * 0.55).abs() < 1e-6);
        let hall = lights_for(m, Light::Hall, false);
        assert_eq!(hall.spot.len(), 1);
        let s = hall.spot[0];
        assert!(s.pos.distance(vec3(0.25, 1.05, 1.9)) < 1e-5);
        assert!(s.dir.x > 0.9, "the hall light shines east down the hall: {:?}", s.dir);
        assert!((s.dir.length() - 1.0).abs() < 1e-5);
        assert!((s.cos_outer - 18f32.to_radians().cos()).abs() < 1e-6);
        assert_eq!((s.energy, s.range), (6.0, 12.0));
        let bed = lights_for(m, Light::Bed, false);
        let b = bed.spot[0];
        assert!(b.pos.distance(vec3(-0.25, 1.05, 1.9)) < 1e-5);
        assert!(b.dir.y < -0.2 && b.dir.z < -0.9, "{:?}", b.dir);
        assert!((b.cos_outer - 38f32.to_radians().cos()).abs() < 1e-6);
        // the admin reveal lights the room
        let rev = lights_for(m, Light::Dark, true);
        assert!(rev.ambient[0] > 4.0 * dark.ambient[0]);
    }

    #[test]
    fn every_prop_has_a_model_text_that_looks_like_a_bbmodel() {
        for p in &map().props {
            let t = prop_text(&p.model).unwrap_or_else(|| panic!("{} ({}) has no model text", p.id, p.model));
            assert!(t.contains("\"elements\""), "{}", p.model);
        }
        assert!(prop_text("noSuchProp").is_none());
    }

    #[test]
    fn every_room_has_props() {
        let m = map();
        for r in &m.rooms {
            if matches!(r.id.as_str(), "wardrobe" | "stairs") {
                continue; // a closet recess and a stair flight, built from boxes
            }
            assert!(m.props.iter().any(|p| p.room == r.id), "{} has no prop", r.id);
        }
    }

    #[test]
    fn the_bed_matches_the_pillow() {
        let wb = build_cubes(map());
        let b = group(&wb, "Bed");
        assert_eq!(b.origin, vec3(0.0, 0.0, 1.3));
        let frame = b.cubes.iter().find(|c| c.name == "BedFrame").expect("frame");
        assert!(frame.solid);
        assert_eq!(b.cubes.iter().filter(|c| c.name == "PillowCushion").count(), 2);
    }
}
