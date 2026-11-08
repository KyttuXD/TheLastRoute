#![allow(dead_code)]
//! Masked Hours' 3D world (port of `mask_world.gd` and `mask_office.gd`): the night guard's small office with NO doors at
//! the end of a 15 m main hall, and the FNAF 2 style wing behind it: the open-plan show floor (game area, show stage,
//! prize corner, parts / service room), four party rooms off the hall, the side halls down the outer sides to the vent hub
//! behind the office, and the two dark air vent ducts to the office's side vents. Everything is cubes in named groups
//! (`Group`), plus the Blockbench props of `props.rs`. Coordinates: the office is at the origin, +z is south (behind the
//! player), the main hall runs along -z.
//!
//! `parts()` is pure (no GL) so tests can inspect the groups, solids and stations; `MaskWorld::build` bakes them.
//!
//! Differences from the Godot scene, all to stop hunters and ducts clipping through things: the two show-floor pillars sit
//! 3 m nearer the south wall (off the prize corner's lane), the ducts start outside the office's side wall, the vent hub
//! is 3.0 m high (a 2.58 m hunter fits under its pipes) and the Show Stage has a step up at its front.

use super::data::{map, rules, side_of_room};
use super::props;
use super::sim::Lit;
use crate::gfx::{Aabb, Cube, Gfx, Group, LightSet, Omni, Spot, StationDef, Textures, World, WorldBuilder};
use crate::gfx::prims::wall_with_gaps;
use macroquad::prelude::*;

pub const BALLOON_TINTS: [[f32; 3]; 5] = [[1.5, 0.5, 0.5], [0.6, 1.0, 1.5], [1.5, 1.5, 0.4], [0.7, 1.5, 0.7], [1.5, 0.7, 1.5]];
/// Wing wall height and thickness.
pub const H: f32 = 3.6;
pub const WING_WALL: [f32; 3] = [1.5, 1.2, 1.5];

/// The desk lamp's light (energy and range from the Godot OmniLight3D).
pub const LAMP_POS: Vec3 = vec3(0.95, 1.35, -0.5);

fn tint(r: f32, g: f32, b: f32) -> [f32; 3] {
    [r.min(1.5), g.min(1.5), b.min(1.5)]
}

fn bx(g: &mut Group, name: &str, kind: &str, t: [f32; 3], size: [f32; 3], pos: [f32; 3]) {
    let mut c = Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).tint(t);
    if !kind.is_empty() {
        c = c.kind(kind);
    }
    g.add(c);
}

fn sbx(g: &mut Group, name: &str, kind: &str, t: [f32; 3], size: [f32; 3], pos: [f32; 3]) {
    let mut c = Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2])).tint(t).solid();
    if !kind.is_empty() {
        c = c.kind(kind);
    }
    g.add(c);
}

fn wall(g: &mut Group, along_x: bool, fixed: f32, a: f32, b: f32, gaps: &[(f32, f32)], t: [f32; 3], name: &str, height: f32) {
    wall_with_gaps(g, along_x, fixed, a, b, gaps, height, 0.2, t, name);
}

/// A floor slab with its top at y = `lift` (a lift of 0.004 keeps it off the show floor's own top).
fn floor(g: &mut Group, x0: f32, x1: f32, z0: f32, z1: f32, t: [f32; 3], name: &str, lift: f32) {
    bx(g, name, "floor", t, [x1 - x0, 0.2, z1 - z0], [(x0 + x1) * 0.5, -0.1 + lift, (z0 + z1) * 0.5]);
}

fn ceiling(g: &mut Group, x0: f32, x1: f32, z0: f32, z1: f32, t: [f32; 3], height: f32) {
    bx(g, "CeilingShow", "concrete", t, [x1 - x0, 0.2, z1 - z0], [(x0 + x1) * 0.5, height + 0.1, (z0 + z1) * 0.5]);
}

fn group(name: &str, x: f32, y: f32, z: f32) -> Group {
    Group::new(name, vec3(x, y, z))
}

pub struct MaskParts {
    pub groups: Vec<Group>,
    pub solids: Vec<Aabb>,
    pub stations: Vec<StationDef>,
}

/// Everything the world is made of, with no GL.
pub fn parts() -> MaskParts {
    let mut groups: Vec<Group> = Vec::new();
    let mut solids: Vec<Aabb> = Vec::new();
    office(&mut groups, &mut solids);
    wing(&mut groups);
    mounts(&mut groups);
    MaskParts { groups, solids, stations: stations() }
}

// ---------------------------------------------------------------------------------------------------- the office

/// The stations of the office with the exact prompts of the Godot scene.
pub fn stations() -> Vec<StationDef> {
    let m = map();
    let st = |id: &str, prompt: &str, pos: [f32; 3], focus: [f32; 3]| StationDef {
        id: id.to_string(),
        prompt: prompt.to_string(),
        pos: vec3(pos[0], pos[1], pos[2]),
        focus: vec3(focus[0], focus[1], focus[2]),
        radius: 0.35,
        disabled: false,
    };
    vec![
        st("monitor", "MONITOR // CAMERAS", [-0.65, 1.1, -0.65], [0.0, 0.0, 0.0]),
        st("lamp", "DESK LAMP", [0.95, 1.0, -0.65], [0.0, 0.0, 0.0]),
        st("mask", "MASK // PUT ON / TAKE OFF", [0.3, 0.95, -0.35], [0.0, 0.0, 0.0]),
        st("musicbox", "MUSIC BOX // HOLD TO WIND", [-1.1, 0.95, -0.3], [0.0, 0.0, 0.0]),
        st("chair", "SIT AT DESK", [0.0, 0.0, 0.5], [0.0, 0.62, 0.0]),
        st("ventL", "LEFT VENT // HOLD FOR LIGHT", [-(m.half_w - 0.2), 0.45, 0.6], [0.0, 0.0, 0.0]),
        st("ventR", "RIGHT VENT // HOLD FOR LIGHT", [m.half_w - 0.2, 0.45, 0.6], [0.0, 0.0, 0.0]),
        st("hall_light", "HALL LIGHT // HOLD", [m.opening + 0.2, 1.4, m.front_z + 0.4], [0.0, 0.0, 0.0]),
    ]
}

fn office(out: &mut Vec<Group>, solids: &mut Vec<Aabb>) {
    let m = map();
    let (hw, fz, bz, h, op) = (m.half_w, m.front_z, m.back_z, m.height, m.opening);
    let wallc = tint(1.8, 1.25, 1.9);
    let zc = (bz + fz) * 0.5;
    let w = hw * 2.0 + 0.4;
    let d = bz - fz + 0.4;

    // the shell: a small room open to the hall
    let mut s = group("OShell", 0.0, 0.0, 0.0);
    sbx(&mut s, "Floor", "floor", tint(1.5, 1.1, 1.5), [w, 0.2, d], [0.0, -0.1, zc]);
    bx(&mut s, "Ceiling", "concrete", tint(0.9, 0.8, 1.1), [w, 0.2, d], [0.0, h + 0.1, zc]);
    sbx(&mut s, "WallBack", "wall", wallc, [w, h, 0.2], [0.0, h * 0.5, bz + 0.1]);
    for sx in [-1.0f32, 1.0] {
        sbx(&mut s, "WallSide", "wall", wallc, [0.2, h, bz - fz + 0.2], [sx * (hw + 0.1), h * 0.5, zc]);
        sbx(&mut s, "WallFront", "wall", wallc, [hw - op + 0.2, h, 0.2], [sx * (op + (hw - op) * 0.5 + 0.1), h * 0.5, fz - 0.1]);
        bx(&mut s, "Baseboard", "wall", tint(1.3, 0.9, 1.1), [0.08, 0.22, bz - fz], [sx * (hw - 0.04), 0.11, zc]);
        bx(&mut s, "StripeOrange", "label", tint(1.9, 1.0, 0.45), [0.05, 0.1, bz - fz], [sx * (hw - 0.03), 1.1, zc]);
    }
    bx(&mut s, "WallHeader", "wall", wallc, [op * 2.0, h - 2.5, 0.2], [0.0, 2.5 + (h - 2.5) * 0.5, fz - 0.1]);
    bx(&mut s, "Baseboard", "wall", tint(1.3, 0.9, 1.1), [hw * 2.0, 0.22, 0.08], [0.0, 0.11, bz - 0.04]);
    bx(&mut s, "StripeOrange", "label", tint(1.9, 1.0, 0.45), [hw * 2.0, 0.1, 0.05], [0.0, 1.1, bz - 0.03]);
    for x in [-2.0f32, 2.0] {
        bx(&mut s, "CeilingFixture", "plastic", [1.0, 1.0, 1.0], [1.0, 0.08, 0.5], [x, h - 0.04, 0.3]);
        bx(&mut s, "CeilingLens", "glass", tint(0.6, 0.55, 0.7), [0.8, 0.02, 0.3], [x, h - 0.1, 0.3]);
    }
    out.push(s);

    // the front vent: a decorative grille above the opening (sealed)
    let mut fv = group("OVentFront", 0.0, 2.85, fz);
    bx(&mut fv, "VentFrame", "metal", tint(0.5, 0.5, 0.6), [1.0, 0.5, 0.1], [0.0, 0.0, -0.05]);
    for i in 0..4 {
        bx(&mut fv, "VentLouver", "metal", tint(0.4, 0.4, 0.5), [0.9, 0.04, 0.06], [0.0, -0.15 + i as f32 * 0.1, 0.02]);
    }
    out.push(fv);

    // the desk: monitor, keyboard, papers, a pizza box, the lamp, the mask, the music box
    let mut desk = group("ODesk", 0.0, 0.0, -0.5);
    bx(&mut desk, "DeskTop", "wood", tint(1.6, 1.1, 0.8), [2.5, 0.08, 0.85], [0.0, 0.76, 0.0]);
    bx(&mut desk, "DeskEdge", "wood", tint(1.9, 1.3, 0.9), [2.54, 0.04, 0.89], [0.0, 0.8, 0.0]);
    for x in [-1.15f32, 1.15] {
        bx(&mut desk, "DeskLeg", "wood", tint(1.6, 1.1, 0.8), [0.1, 0.76, 0.75], [x, 0.38, 0.0]);
    }
    bx(&mut desk, "DeskFrontPanel", "wood", tint(1.3, 0.9, 0.7), [2.2, 0.55, 0.04], [0.0, 0.45, 0.38]);
    solids.push(Aabb::from_centre(vec3(0.0, 0.42, -0.5), vec3(2.5, 0.85, 0.85)));
    out.push(desk);

    let mut mon = group("OMonitor", -0.65, 0.8, -0.65);
    bx(&mut mon, "MonitorBody", "plastic", [1.0, 1.0, 1.0], [0.78, 0.55, 0.1], [0.0, 0.32, 0.0]);
    bx(&mut mon, "MonitorScreen", "screen", [0.8, 1.0, 0.9], [0.68, 0.45, 0.02], [0.0, 0.32, 0.06]);
    bx(&mut mon, "MonitorFoot", "plastic", [1.0, 1.0, 1.0], [0.2, 0.06, 0.2], [0.0, 0.03, 0.0]);
    out.push(mon);

    let mut kb = group("OKeyboard", -0.65, 0.8, -0.25);
    bx(&mut kb, "KeyboardBody", "plastic", tint(1.2, 1.2, 1.3), [0.5, 0.03, 0.18], [0.0, 0.015, 0.0]);
    out.push(kb);

    let mut papers = group("OPapers", 0.0, 0.8, -0.7);
    for i in 0..3 {
        let mut c = Cube::new("PaperSheet", vec3(0.32, 0.01, 0.4), vec3(0.0, 0.006 + i as f32 * 0.01, 0.0)).kind("paper").tint(tint(1.5, 1.5, 1.4)).ao(1.0);
        c.shade = 1.0;
        papers.add(c);
    }
    out.push(papers);

    let mut pizza = group("OPizza", 0.95, 0.8, -0.2);
    bx(&mut pizza, "PizzaBoxBody", "wood", tint(1.5, 1.4, 1.1), [0.4, 0.06, 0.4], [0.0, 0.03, 0.0]);
    out.push(pizza);

    let mut fan = group("OFan", 0.45, 0.8, -0.75);
    bx(&mut fan, "FanBase", "plastic", [1.0, 1.0, 1.0], [0.18, 0.04, 0.18], [0.0, 0.02, 0.0]);
    bx(&mut fan, "FanStem", "metal", [1.0, 1.0, 1.0], [0.04, 0.28, 0.04], [0.0, 0.18, 0.0]);
    out.push(fan);
    // the blades turn: two crossed pairs shown by turns (a quarter-turn flicker)
    for (name, rot) in [("FanA", 0.0f32), ("FanB", std::f32::consts::FRAC_PI_4)] {
        let mut b = group(name, 0.45, 1.14, -0.7);
        b.add(Cube::new("FanBlade", vec3(0.34, 0.02, 0.06), Vec3::ZERO).kind("plastic").tint(tint(1.5, 1.5, 1.6)).rot_y(rot));
        b.add(Cube::new("FanBlade", vec3(0.06, 0.02, 0.34), Vec3::ZERO).kind("plastic").tint(tint(1.5, 1.5, 1.6)).rot_y(rot));
        out.push(b);
    }

    let mut lamp = group("DeskLampProp", 0.95, 0.8, -0.65);
    bx(&mut lamp, "LampBase", "metal", [1.0, 1.0, 1.0], [0.18, 0.04, 0.18], [0.0, 0.02, 0.0]);
    bx(&mut lamp, "LampArm", "metal", [1.0, 1.0, 1.0], [0.04, 0.22, 0.04], [0.0, 0.15, 0.0]);
    bx(&mut lamp, "LampShade", "light", [1.0, 1.0, 1.0], [0.22, 0.14, 0.22], [0.0, 0.28, 0.0]);
    out.push(lamp);

    let mut mask = group("OMask", 0.3, 0.8, -0.35);
    bx(&mut mask, "MaskFace", "plastic", tint(1.8, 1.75, 1.6), [0.22, 0.28, 0.05], [0.0, 0.16, 0.0]);
    bx(&mut mask, "MaskEyes", "plastic", tint(0.25, 0.25, 0.3), [0.14, 0.04, 0.03], [0.0, 0.2, 0.03]);
    out.push(mask);

    let mut box_ = group("OMusicBox", -1.1, 0.8, -0.3);
    bx(&mut box_, "BoxMusicBody", "wood", tint(1.2, 0.9, 1.1), [0.42, 0.22, 0.3], [0.0, 0.11, 0.0]);
    bx(&mut box_, "BoxMusicLid", "wood", tint(1.3, 1.0, 1.2), [0.44, 0.05, 0.32], [0.0, 0.24, 0.0]);
    bx(&mut box_, "HandleCrank", "metal", [1.0, 1.0, 1.0], [0.05, 0.16, 0.05], [0.27, 0.16, 0.0]);
    out.push(box_);
    let mut gauge = group("GaugeOffice", -1.1, 1.1, -0.16);
    bx(&mut gauge, "GlowGauge", "light", [1.0, 1.0, 1.0], [0.3, 0.03, 0.03], [0.0, 0.0, 0.0]);
    out.push(gauge);

    // the side vents, with the eyeshine that is the only trace of a hunter in a dark vent
    for side in [-1.0f32, 1.0] {
        let tag = if side < 0.0 { "L" } else { "R" };
        let mut v = group(&format!("OVent{tag}"), side * hw, 0.45, 0.6);
        sbx(&mut v, "VentFrame", "metal", [1.0, 1.0, 1.0], [0.1, 0.54, 0.84], [-side * 0.05, 0.0, 0.0]);
        for i in 0..5 {
            bx(&mut v, "VentLouver", "metal", tint(0.4, 0.4, 0.5), [0.06, 0.035, 0.7], [-side * 0.12, -0.2 + i as f32 * 0.1, 0.0]);
        }
        for sz in [-0.38f32, 0.38] {
            for sy in [-0.24f32, 0.24] {
                bx(&mut v, "VentScrew", "metal", tint(1.3, 1.3, 1.3), [0.05, 0.05, 0.05], [-side * 0.13, sy, sz]);
            }
        }
        out.push(v);
        let mut gl = group(&format!("GlintVent{tag}"), side * (hw - 0.2), 0.5, 0.6);
        for gz in [-0.12f32, 0.12] {
            bx(&mut gl, "GlowEye", "light", [1.0, 1.0, 1.0], [0.04, 0.04, 0.06], [0.0, 0.0, gz]);
        }
        out.push(gl);
    }

    // a velvet rope across the opening (the player stays in the office), the hall light switch beside it
    let mut rope = group("OHallRope", 0.0, 0.0, fz + 0.1);
    for x in [-op + 0.1, 0.0, op - 0.1] {
        bx(&mut rope, "PostRope", "metal", tint(1.6, 1.4, 0.8), [0.1, 1.0, 0.1], [x, 0.5, 0.0]);
        bx(&mut rope, "KnobRope", "metal", tint(1.7, 1.5, 0.8), [0.18, 0.08, 0.18], [x, 1.02, 0.0]);
    }
    for x in [-op * 0.5, op * 0.5] {
        bx(&mut rope, "RopeVelvet", "fabric", tint(1.9, 0.3, 0.4), [op - 0.2, 0.06, 0.06], [x, 0.88, 0.0]);
    }
    solids.push(Aabb::from_centre(vec3(0.0, 0.7, fz + 0.1), vec3(op * 2.0, 1.4, 0.2)));
    out.push(rope);
    let mut sw = group("HallSwitch", op + 0.2, 1.4, fz + 0.05);
    bx(&mut sw, "PanelFace", "metal", [1.0, 1.0, 1.0], [0.3, 0.4, 0.06], [0.0, 0.0, 0.0]);
    bx(&mut sw, "ButtonHall", "plastic", [1.0, 1.0, 1.0], [0.18, 0.18, 0.06], [0.0, 0.0, 0.05]);
    out.push(sw);

    // kids' drawings on the back wall, a poster, a balloon on a string, a stack of party hats
    let mut art = group("ODrawings", 0.0, 1.8, bz);
    for i in 0..5 {
        let x = -2.0 + i as f32;
        let y = (i % 2) as f32 * 0.25 - 0.1;
        bx(&mut art, "DrawingPaper", "paper", tint(1.6, 1.6, 1.4), [0.45, 0.6, 0.03], [x, y, -0.03]);
        bx(&mut art, "DrawingBlob", "fabric", BALLOON_TINTS[i], [0.3, 0.3, 0.02], [x, y, -0.06]);
    }
    out.push(art);
    let mut poster = group("OPoster", -hw, 1.9, -0.4);
    bx(&mut poster, "PosterPaper", "fabric", tint(1.9, 1.0, 0.5), [0.04, 1.0, 0.75], [0.03, 0.0, 0.0]);
    bx(&mut poster, "PosterTitle", "paper", tint(1.4, 1.4, 1.4), [0.04, 0.2, 0.5], [0.06, 0.32, 0.0]);
    out.push(poster);
    let mut balloon = group("OBalloon", -2.8, 0.0, 2.9);
    bx(&mut balloon, "StringBalloon", "plastic", tint(1.6, 1.6, 1.6), [0.02, 1.8, 0.02], [0.0, 0.9, 0.0]);
    bx(&mut balloon, "BalloonCube", "fabric", BALLOON_TINTS[0], [0.42, 0.5, 0.42], [0.0, 2.0, 0.0]);
    out.push(balloon);
    let mut hats = group("OPartyHats", 2.8, 0.0, 2.9);
    for i in 0..3 {
        bx(&mut hats, "HatCone", "fabric", BALLOON_TINTS[i + 1], [0.3, 0.26 - i as f32 * 0.04, 0.3], [0.0, 0.13 + i as f32 * 0.26, 0.0]);
    }
    out.push(hats);
}

// ---------------------------------------------------------------------------------------------------- the wing

fn balloons(parent_name: &str, x: f32, y: f32, z: f32, n: usize) -> Group {
    let mut g = group(parent_name, x, y, z);
    for i in 0..n {
        let h = 2.0 + (i % 3) as f32 * 0.35;
        let px = -0.3 + i as f32 * 0.2;
        bx(&mut g, "StringBalloon", "plastic", tint(1.6, 1.6, 1.6), [0.02, h, 0.02], [px, h * 0.5, 0.05 * i as f32]);
        bx(&mut g, "BalloonCube", "fabric", BALLOON_TINTS[i % 5], [0.42, 0.5, 0.42], [px, h + 0.2, 0.05 * i as f32]);
    }
    bx(&mut g, "WeightBalloon", "metal", [1.0, 1.0, 1.0], [0.3, 0.3, 0.3], [0.0, 0.15, 0.0]);
    g
}

fn party_table(name: &str, x: f32, y: f32, z: f32, t: [f32; 3]) -> Group {
    let mut g = group(name, x, y, z);
    bx(&mut g, "TableTop", "fabric", t, [1.8, 0.08, 1.0], [0.0, 0.78, 0.0]);
    for sx in [-0.8f32, 0.8] {
        for sz in [-0.4f32, 0.4] {
            bx(&mut g, "TableLeg", "metal", [1.0, 1.0, 1.0], [0.08, 0.76, 0.08], [sx, 0.38, sz]);
        }
    }
    bx(&mut g, "CakeTable", "fabric", tint(1.8, 1.5, 1.6), [0.32, 0.22, 0.32], [0.0, 0.93, 0.0]);
    for (k, hx) in [-0.6f32, 0.6].iter().enumerate() {
        bx(&mut g, "HatTable", "fabric", BALLOON_TINTS[(k * 2 + 3) % 5], [0.16, 0.24, 0.16], [*hx, 0.94, 0.25]);
    }
    for sx in [-0.5f32, 0.5] {
        for sz in [-0.85f32, 0.85] {
            bx(&mut g, "StoolTable", "plastic", tint(1.3, 0.8, 0.8), [0.36, 0.45, 0.36], [sx, 0.22, sz]);
        }
    }
    g
}

fn wing(out: &mut Vec<Group>) {
    let m = map();
    let wallc = WING_WALL;
    let mut wing_groups: Vec<Group> = Vec::new();

    // the show floor: perimeter walls, the south wall with the hall's mouth and the two outer halls' mouths, the piers
    let mut sh = group("WShell", 0.0, 0.0, 0.0);
    floor(&mut sh, -19.0, 19.0, -40.0, -17.0, tint(1.2, 1.1, 1.4), "FloorShow", 0.0);
    ceiling(&mut sh, -19.0, 19.0, -40.0, -17.0, tint(0.9, 0.8, 1.1), H);
    wall(&mut sh, true, -40.0, -19.0, 19.0, &[], wallc, "WallShow", H);
    wall(&mut sh, false, -19.0, -40.0, -17.0, &[], wallc, "WallShow", H);
    wall(&mut sh, false, 19.0, -40.0, -17.0, &[], wallc, "WallShow", H);
    wall(&mut sh, true, -17.0, -19.0, 19.0, &[(-13.7, -11.3), (-1.9, 1.9), (11.3, 13.7)], wallc, "WallShow", H);
    for x in [-11.0f32, 11.0] {
        wall(&mut sh, false, x, -31.0, -28.0, &[], wallc, "PierShow", H);
        wall(&mut sh, false, x, -21.0, -19.0, &[], wallc, "PierShow", H);
    }
    wing_groups.push(sh);

    // the main hall (x +-1.9, z -17 to -1.8) with two party-room arches on each side
    let gaps = [(-14.6, -11.0), (-9.0, -5.4)];
    let mut hall = group("WMainHall", 0.0, 0.0, 0.0);
    floor(&mut hall, -1.9, 1.9, -17.0, -1.8, tint(1.1, 1.05, 1.3), "FloorHall", 0.0);
    ceiling(&mut hall, -1.9, 1.9, -17.0, -1.8, tint(0.9, 0.8, 1.1), H);
    wall(&mut hall, false, -1.9, -17.0, -1.8, &gaps, wallc, "WallHall", H);
    wall(&mut hall, false, 1.9, -17.0, -1.8, &gaps, wallc, "WallHall", H);
    for i in 0..5 {
        bx(&mut hall, "RunnerHall", "fabric", tint(1.8, 0.5, 0.6), [1.2, 0.012, 2.4], [0.0, 0.006, -3.0 - i as f32 * 2.9]);
    }
    for i in 0..6 {
        bx(&mut hall, "StreamerHall", "fabric", BALLOON_TINTS[i % 5], [3.6, 0.08, 0.1], [0.0, 2.7, -3.0 - i as f32 * 2.4]);
    }
    wing_groups.push(hall);

    // the game area: a checkerboard of three-metre tiles, party tables, balloons, streamers, pillars, a sign
    let c = m.centre("ballroom");
    let mut ga = group("WGameArea", c.x, c.y, c.z);
    for ix in 0..7 {
        for iz in 0..4 {
            if (ix + iz) % 2 == 0 {
                bx(&mut ga, "FloorTile", "floor", tint(1.5, 1.4, 1.5), [3.0, 0.012, 3.0], [-9.0 + ix as f32 * 3.0, 0.006, -7.5 + iz as f32 * 3.0 + 1.5]);
            }
        }
    }
    for i in 0..5 {
        bx(&mut ga, "StreamerGame", "fabric", BALLOON_TINTS[i], [22.0, 0.08, 0.1], [0.0, 3.0 - (i % 2) as f32 * 0.15, -6.5 + i as f32 * 3.2]);
    }
    for x in [-9.5f32, 9.5] {
        bx(&mut ga, "PillarGame", "concrete", tint(1.4, 1.2, 1.5), [0.5, H, 0.5], [x, H * 0.5, 3.0]);
    }
    bx(&mut ga, "SignGame", "label", tint(1.5, 1.0, 0.5), [3.0, 0.5, 0.3], [0.0, 2.6, 6.8]);
    wing_groups.push(ga);
    let tables = [(-5.0f32, -4.5f32), (5.0, -4.5), (-5.0, 3.5), (5.0, 3.5)];
    for (i, (tx, tz)) in tables.iter().enumerate() {
        wing_groups.push(party_table(&format!("WGameTable{i}"), c.x + tx, 0.0, c.z + tz, BALLOON_TINTS[(i + 1) % 5]));
    }
    wing_groups.push(balloons("WGameBalloonsW", c.x - 8.0, 0.0, c.z - 6.0, 4));
    wing_groups.push(balloons("WGameBalloonsE", c.x + 8.0, 0.0, c.z - 6.0, 4));

    // the show stage: a platform with a step up at its front, curtains and spots (no performer statues)
    let c = m.centre("gallery");
    let mut st = group("WShowStage", c.x, c.y, c.z);
    bx(&mut st, "PlatformStage", "wood", tint(1.5, 1.0, 0.8), [12.0, 0.7, 5.0], [0.0, 0.35, -2.0]);
    bx(&mut st, "PlatformEdge", "label", tint(1.5, 1.3, 0.7), [12.2, 0.08, 5.2], [0.0, 0.74, -2.0]);
    bx(&mut st, "StepStage", "wood", tint(1.4, 0.9, 0.7), [2.4, 0.35, 0.6], [0.0, 0.175, 0.8]);
    for sx in [-5.6f32, 5.6] {
        bx(&mut st, "CurtainStage", "fabric", tint(1.9, 0.4, 0.5), [0.8, 3.0, 0.3], [sx, 1.9, -4.4]);
    }
    bx(&mut st, "ValanceStage", "fabric", tint(1.9, 0.4, 0.5), [12.0, 0.5, 0.3], [0.0, 3.0, -4.4]);
    bx(&mut st, "SignStage", "label", tint(1.5, 1.1, 0.5), [5.0, 0.7, 0.1], [0.0, 2.4, -4.6]);
    for x in [-3.5f32, 0.0, 3.5] {
        bx(&mut st, "SpotStage", "plastic", [1.0, 1.0, 1.0], [0.5, 0.25, 0.4], [x, 3.2, -1.5]);
        bx(&mut st, "GlowSpot", "light", [1.0, 1.0, 1.0], [0.3, 0.1, 0.3], [x, 3.05, -1.4]);
    }
    wing_groups.push(st);
    // KT 2026-11-08: no performer statues on the stage (Godot had three).

    // the prize corner: shelves of plush boxes behind the counter, the big music box, balloons, streamers
    let c = m.centre("music");
    let mut pc = group("WPrizeCorner", c.x, c.y, c.z);
    floor(&mut pc, -3.5, 3.5, -6.0, 6.0, tint(1.5, 1.3, 1.0), "FloorPrize", 0.004);
    for i in 0..3 {
        bx(&mut pc, "ShelfPrize", "wood", tint(1.4, 0.9, 0.7), [0.6, 0.05, 4.8], [-3.2, 1.4 + i as f32 * 0.55, 0.0]);
        for j in 0..6 {
            bx(&mut pc, "PlushPrize", "fabric", BALLOON_TINTS[(i + j) % 5], [0.4, 0.4, 0.5], [-3.2, 1.65 + i as f32 * 0.55, -2.1 + j as f32 * 0.85]);
        }
    }
    bx(&mut pc, "SignPrize", "label", tint(1.5, 1.1, 0.5), [0.1, 0.7, 4.0], [-3.4, 3.0, 0.0]);
    bx(&mut pc, "StreamerPrize", "fabric", BALLOON_TINTS[2], [7.0, 0.08, 0.1], [0.0, 3.0, -4.0]);
    bx(&mut pc, "StreamerPrize", "fabric", BALLOON_TINTS[3], [7.0, 0.08, 0.1], [0.0, 3.0, 4.0]);
    wing_groups.push(pc);
    let mut mb = group("WMusicBoxBig", c.x + 0.6, 0.0, c.z);
    bx(&mut mb, "BoxPuppet", "fabric", tint(1.8, 0.7, 1.4), [1.4, 1.0, 1.2], [0.0, 0.5, 0.0]);
    bx(&mut mb, "LidPuppet", "fabric", tint(1.9, 1.5, 0.5), [1.5, 0.2, 1.3], [0.0, 1.1, 0.0]);
    bx(&mut mb, "HandleCrank", "metal", [1.0, 1.0, 1.0], [0.12, 0.7, 0.12], [0.85, 0.7, 0.0]);
    wing_groups.push(mb);
    let mut bg = group("GaugeBig", c.x + 0.6, 1.3, c.z + 0.62);
    bx(&mut bg, "GlowGauge", "light", [1.0, 1.0, 1.0], [1.0, 0.08, 0.04], [0.0, 0.0, 0.0]);
    wing_groups.push(bg);
    wing_groups.push(balloons("WPrizeBalloonsA", c.x + 2.4, 0.0, c.z - 3.5, 5));
    wing_groups.push(balloons("WPrizeBalloonsB", c.x - 1.2, 0.0, c.z - 4.8, 4));

    // the parts / service room: shelves of spare heads, limbs in a bin, a sign
    let c = m.centre("costume");
    let mut pr = group("WPartsService", c.x, c.y, c.z);
    floor(&mut pr, -3.5, 3.5, -6.0, 6.0, tint(0.9, 1.0, 1.2), "FloorParts", 0.004);
    for i in 0..3 {
        bx(&mut pr, "ShelfParts", "metal", [1.0, 1.0, 1.0], [0.6, 0.05, 5.2], [3.0, 0.5 + i as f32 * 0.7, 0.0]);
        for j in 0..5 {
            let tt = BALLOON_TINTS[(i + j) % 5];
            bx(&mut pr, "HeadParts", "fabric", [tt[0] * 0.8, tt[1] * 0.8, tt[2] * 0.8], [0.4, 0.4, 0.4], [3.0, 0.75 + i as f32 * 0.7, -2.0 + j as f32]);
            bx(&mut pr, "EyeParts", "plastic", tint(1.8, 1.8, 1.8), [0.1, 0.08, 0.04], [2.7, 0.8 + i as f32 * 0.7, -2.0 + j as f32]);
        }
    }
    for i in 0..4 {
        bx(&mut pr, "LimbParts", "metal", tint(1.2, 1.2, 1.3), [0.14, 0.14, 0.9], [-1.5 + (i % 2) as f32 * 0.3, 1.0, 3.8 + (i as f32 - 1.5) * 0.1]);
    }
    bx(&mut pr, "SignParts", "label", tint(1.5, 1.2, 0.5), [0.9, 0.5, 0.1], [0.0, 2.3, -5.9]);
    wing_groups.push(pr);
    wing_groups.push(balloons("WPartsBalloons", c.x - 2.4, 0.0, c.z, 3));

    // the four party rooms off the hall (two each side): a table (a prop), balloons, streamers, a sign
    let mut n = 0;
    for side in [-1.0f32, 1.0] {
        for (z0, z1) in [(-14.6f32, -11.0f32), (-9.0, -5.4)] {
            n += 1;
            let (x_in, x_out) = (side * 1.9, side * 8.0);
            let (xa, xb) = (x_in.min(x_out), x_in.max(x_out));
            let mid = (z0 + z1) * 0.5;
            let mut room = group(&format!("WPartyRoom{n}"), 0.0, 0.0, 0.0);
            floor(&mut room, xa, xb, z0 - 0.4, z1 + 0.4, tint(1.6, 1.4, 1.2), "FloorParty", 0.0);
            ceiling(&mut room, xa, xb, z0 - 0.4, z1 + 0.4, tint(0.9, 0.8, 1.1), H);
            wall(&mut room, false, x_out, z0 - 0.4, z1 + 0.4, &[], wallc, "WallParty", H);
            wall(&mut room, true, z0 - 0.4, xa, xb, &[], wallc, "WallParty", H);
            wall(&mut room, true, z1 + 0.4, xa, xb, &[], wallc, "WallParty", H);
            for i in 0..3 {
                bx(&mut room, "StreamerParty", "fabric", BALLOON_TINTS[(n + i) % 5], [0.1, 0.08, 5.0], [side * (3.0 + i as f32 * 1.6), 3.0, mid]);
            }
            bx(&mut room, "SignParty", "label", tint(1.5, 1.1, 0.5), [0.1, 0.5, 1.6], [side * 7.9, 2.4, mid]);
            wing_groups.push(room);
            wing_groups.push(balloons(&format!("WPartyBalloons{n}"), side * 7.3, 0.0, z0 + 0.2, 3));
        }
    }

    // the left and right halls: dark service corridors down the outer sides to the vent hub
    for side in [-1.0f32, 1.0] {
        let tag = if side < 0.0 { "L" } else { "R" };
        let x = side * 12.5;
        let dim = [wallc[0] * 0.8, wallc[1] * 0.8, wallc[2] * 0.9];
        let mut sd = group(&format!("WSideHall{tag}"), 0.0, 0.0, 0.0);
        floor(&mut sd, x - 1.2, x + 1.2, -17.0, 3.0, tint(0.9, 0.9, 1.1), "FloorSide", 0.0);
        ceiling(&mut sd, x - 1.2, x + 1.2, -17.0, 3.0, tint(0.7, 0.7, 0.9), H);
        wall(&mut sd, false, x - 1.2, -17.0, 3.0, &[], dim, "WallSide", H);
        wall(&mut sd, false, x + 1.2, -17.0, 3.0, &[], dim, "WallSide", H);
        for i in 0..4 {
            bx(&mut sd, "PosterSide", "fabric", tint(1.2, 0.9, 0.5), [0.9, 0.1, 0.06], [x, 2.4, -15.0 + i as f32 * 4.6]);
            bx(&mut sd, "PennantSide", "fabric", BALLOON_TINTS[i], [0.08, 0.6, 0.5], [x - side * 1.1, 1.5, -13.0 + i as f32 * 4.6]);
        }
        wing_groups.push(sd);
    }

    // the vent hub (the hunters' nest): pipes, a sign, crates (props)
    let hub_h = 3.0;
    let hw = [0.9f32, 0.9, 1.1];
    let mut hub = group("WVentHub", 0.0, 0.0, 0.0);
    floor(&mut hub, -14.0, 14.0, 3.0, 10.0, tint(0.8, 0.8, 1.0), "FloorHub", 0.0);
    ceiling(&mut hub, -14.0, 14.0, 3.0, 10.0, tint(0.6, 0.6, 0.8), hub_h);
    wall(&mut hub, true, 10.0, -14.0, 14.0, &[], hw, "WallHub", hub_h);
    wall(&mut hub, true, 3.0, -14.0, 14.0, &[(-13.7, -11.3), (-8.9, -7.1), (7.1, 8.9), (11.3, 13.7)], hw, "WallHub", hub_h);
    wall(&mut hub, false, -14.0, 3.0, 10.0, &[], hw, "WallHub", hub_h);
    wall(&mut hub, false, 14.0, 3.0, 10.0, &[], hw, "WallHub", hub_h);
    for i in 0..3 {
        bx(&mut hub, "PipeHub", "metal", [1.0, 1.0, 1.0], [26.0, 0.15, 0.15], [0.0, 2.75, 4.5 + i as f32 * 2.0]);
    }
    bx(&mut hub, "SignHub", "label", tint(1.4, 1.0, 0.5), [3.0, 0.4, 0.1], [0.0, 2.0, 9.85]);
    wing_groups.push(hub);

    // the two air vent ducts: from the hub north, then east / west to the office's side vents (low, dark, a ribbed tube)
    for side in [-1.0f32, 1.0] {
        let tag = if side < 0.0 { "L" } else { "R" };
        let (x0, x1) = (side * 3.5, side * 8.8);
        let (xa, xb) = (x0.min(x1), x0.max(x1));
        let mid = (xa + xb) * 0.5;
        let len = xb - xa;
        let mut d = group(&format!("WDuct{tag}"), 0.0, 0.0, 0.0);
        bx(&mut d, "DuctFloor", "metal", tint(1.7, 1.7, 1.85), [len, 0.1, 1.6], [mid, 0.05, 0.6]);
        bx(&mut d, "DuctSide", "metal", tint(1.5, 1.5, 1.65), [len, 1.8, 0.1], [mid, 0.9, -0.2]);
        bx(&mut d, "DuctSide", "metal", tint(1.5, 1.5, 1.65), [len, 1.8, 0.1], [mid, 0.9, 1.4]);
        bx(&mut d, "DuctTop", "metal", tint(1.3, 1.3, 1.45), [len, 0.1, 1.8], [mid, 1.85, 0.6]);
        bx(&mut d, "DuctFloor", "metal", tint(1.7, 1.7, 1.85), [1.6, 0.1, 2.0], [side * 8.0, 0.05, 2.2]);
        bx(&mut d, "DuctEnd", "metal", tint(1.1, 1.1, 1.25), [0.1, 1.8, 1.7], [side * 8.9, 0.9, 0.6]);
        for i in 0..5 {
            bx(&mut d, "DuctRib", "metal", tint(1.7, 1.7, 1.85), [0.08, 0.12, 1.7], [side * (3.9 + i as f32 * 1.0), 1.75, 0.6]);
        }
        wing_groups.push(d);
        let mut gl = group(&format!("GlintDuct{tag}"), side * 5.4, 0.9, 0.6);
        for gz in [-0.1f32, 0.1] {
            bx(&mut gl, "GlowEye", "light", [1.0, 1.0, 1.0], [0.04, 0.04, 0.06], [0.0, 0.0, gz]);
        }
        wing_groups.push(gl);
    }

    // the wing is not walkable for the player: only the office collides
    for g in wing_groups.iter_mut() {
        for c in g.cubes.iter_mut() {
            c.solid = false;
        }
    }
    out.extend(wing_groups);
}

/// The security cameras themselves, small boxes at their mounts (a camera never sees its own).
fn mounts(out: &mut Vec<Group>) {
    let mut g = group("WCameras", 0.0, 0.0, 0.0);
    for (room, cam) in map().cameras.iter() {
        let p = cam.mount;
        bx(&mut g, &format!("CamBody_{room}"), "plastic", tint(0.5, 0.5, 0.55), [0.3, 0.2, 0.3], [p.x, p.y + 0.15, p.z]);
        let d = (cam.target - cam.mount).normalize_or_zero();
        let lens = p + d * 0.18;
        bx(&mut g, &format!("CamLens_{room}"), "glass", tint(0.6, 0.7, 0.8), [0.12, 0.12, 0.12], [lens.x, lens.y + 0.15, lens.z]);
    }
    out.push(g);
}

// ---------------------------------------------------------------------------------------------------- lights

fn spot(pos: Vec3, target: Vec3, color: [f32; 3], energy: f32, range: f32, half_deg: f32) -> Spot {
    Spot { pos, dir: (target - pos).normalize_or_zero(), color, energy, range, cos_outer: half_deg.to_radians().cos() }
}

/// The light for what the player sees from the office: dim ambient, the desk lamp, the hall light and a held vent light.
pub fn office_lights(lit: Lit, lamp_on: bool, flashlight: Option<Spot>) -> LightSet {
    let m = map();
    let mut l = LightSet { ambient: [0.30, 0.32, 0.46], omni: Vec::new(), spot: Vec::new(), fog: None };
    if lamp_on {
        l.omni.push(Omni { pos: LAMP_POS, color: [1.0, 0.82, 0.55], energy: 2.2, range: 5.0 });
    }
    if lit.h {
        l.spot.push(spot(vec3(0.0, 2.7, m.front_z + 0.3), vec3(0.0, 0.6, -14.0), [1.0, 0.97, 0.85], 9.0, 20.0, 24.0));
    }
    if lit.l {
        l.spot.push(spot(vec3(-0.7, 1.5, 1.7), vec3(-m.half_w, 0.6, 0.6), [1.0, 0.97, 0.85], 6.0, 5.0, 38.0));
    }
    if lit.r {
        l.spot.push(spot(vec3(0.7, 1.5, 1.7), vec3(m.half_w, 0.6, 0.6), [1.0, 0.97, 0.85], 6.0, 5.0, 38.0));
    }
    if let Some(f) = flashlight {
        l.spot.push(f);
    }
    l
}

/// The light for a camera feed: bright and flat, except the dark ducts, which are black unless their vent light is held on
/// that camera.
pub fn feed_lights(room: &str, lit: Lit) -> LightSet {
    let mut l = LightSet::feed();
    if let Some(side) = side_of_room(room) {
        if room.starts_with("duct") {
            if lit.side(side) {
                let sx = if side == 'L' { -1.0 } else { 1.0 };
                l.omni.push(Omni { pos: vec3(sx * 7.5, 1.5, 0.6), color: [1.0, 0.95, 0.8], energy: 2.2, range: 7.0 });
            } else {
                for a in l.ambient.iter_mut() {
                    *a *= 0.19 / 1.15;
                }
            }
        }
    }
    l
}

// ---------------------------------------------------------------------------------------------------- the baked world

pub struct MaskWorld {
    pub world: World,
    pub stations: Vec<StationDef>,
    pub props: Vec<Mesh>,
    lamp_on: bool,
    hall_button_on: bool,
    fan_flip: bool,
    box_pct: f32,
}

impl MaskWorld {
    /// Needs the GL context (textures, prop atlases).
    pub fn build(tex: &Textures) -> MaskWorld {
        let p = parts();
        let mut wb = WorldBuilder::new();
        wb.groups = p.groups;
        wb.solids = p.solids;
        wb.marker("spawn", map().spawn);
        let world = wb.bake(tex);
        let mut w = MaskWorld { world, stations: p.stations, props: props::load(), lamp_on: true, hall_button_on: true, fan_flip: false, box_pct: -1.0 };
        w.world.set_visible("FanB", false);
        for g in ["GlintVentL", "GlintVentR", "GlintDuctL", "GlintDuctR"] {
            w.world.set_visible(g, false);
        }
        w.set_lamp(false);
        w.set_hall_button(false);
        w.set_box(100.0);
        w
    }

    pub fn solids(&self) -> &[Aabb] {
        &self.world.solids
    }

    pub fn station_index(&self, id: &str) -> Option<usize> {
        self.stations.iter().position(|s| s.id == id)
    }

    pub fn set_lamp(&mut self, on: bool) {
        if self.lamp_on != on {
            self.lamp_on = on;
            if on {
                self.world.set_material("DeskLampProp", "LampShade", "light", [1.0, 1.0, 1.0], true);
            } else {
                self.world.set_material("DeskLampProp", "LampShade", "light", [0.35, 0.3, 0.25], false);
            }
        }
    }

    pub fn set_hall_button(&mut self, on: bool) {
        if self.hall_button_on != on {
            self.hall_button_on = on;
            if on {
                self.world.set_material("HallSwitch", "ButtonHall", "light", [1.0, 1.0, 1.0], true);
            } else {
                self.world.set_material("HallSwitch", "ButtonHall", "plastic", [1.0, 1.0, 1.0], false);
            }
        }
    }

    /// The gauge bars on the music box (the desk one and the big one): their length is the winding left.
    pub fn set_box(&mut self, percent: f32) {
        if (self.box_pct - percent).abs() < 0.05 {
            return;
        }
        self.box_pct = percent;
        let k = (percent / 100.0).clamp(0.02, 1.0);
        self.world.set_cube_scale("GaugeOffice", "GlowGauge", vec3(k, 1.0, 1.0));
        self.world.set_offset("GaugeOffice", vec3(-0.15 * (1.0 - k), 0.0, 0.0));
        self.world.set_cube_scale("GaugeBig", "GlowGauge", vec3(k, 1.0, 1.0));
        self.world.set_offset("GaugeBig", vec3(-0.5 * (1.0 - k), 0.0, 0.0));
    }

    /// Eyeshine in the dark vents and ducts.
    pub fn set_glints(&mut self, vent_l: bool, vent_r: bool, duct_l: bool, duct_r: bool) {
        self.world.set_visible("GlintVentL", vent_l);
        self.world.set_visible("GlintVentR", vent_r);
        self.world.set_visible("GlintDuctL", duct_l);
        self.world.set_visible("GlintDuctR", duct_r);
    }

    /// The desk fan's blades, a quarter-turn flicker.
    pub fn flip_fan(&mut self) {
        self.fan_flip = !self.fan_flip;
        self.world.set_visible("FanA", !self.fan_flip);
        self.world.set_visible("FanB", self.fan_flip);
    }

    /// The whole scene (cubes and props) with these lights.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, lights, cam_pos);
        gfx.shader.begin(lights, cam_pos);
        for m in &self.props {
            draw_mesh(m);
        }
        gfx.shader.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Bindings, InputFrame, Save};
    use crate::player::Player;

    fn find<'a>(p: &'a MaskParts, name: &str) -> Option<&'a Group> {
        p.groups.iter().find(|g| g.name == name)
    }

    #[test]
    fn the_office_has_the_groups_the_game_addresses() {
        let p = parts();
        for name in [
            "OShell", "ODesk", "OMonitor", "DeskLampProp", "OMask", "OMusicBox", "GaugeOffice", "OVentL", "OVentR", "GlintVentL", "GlintVentR",
            "OHallRope", "HallSwitch", "FanA", "FanB", "GaugeBig", "GlintDuctL", "GlintDuctR", "WShell", "WMainHall", "WGameArea", "WShowStage",
            "WPrizeCorner", "WPartsService", "WVentHub", "WDuctL", "WDuctR", "WSideHallL", "WSideHallR", "WCameras",
        ] {
            assert!(find(&p, name).is_some(), "group {name}");
        }
        assert!(find(&p, "DeskLampProp").map(|g| g.cubes.iter().any(|c| c.name == "LampShade")).unwrap_or(false));
        assert!(find(&p, "HallSwitch").map(|g| g.cubes.iter().any(|c| c.name == "ButtonHall")).unwrap_or(false));
        assert!(find(&p, "GaugeOffice").map(|g| g.cubes.iter().any(|c| c.name == "GlowGauge")).unwrap_or(false));
        assert_eq!(p.groups.iter().filter(|g| g.name.starts_with("WPartyRoom")).count(), 4);
        // KT removed the stage statues: none may come back.
        assert_eq!(p.groups.iter().filter(|g| g.name.starts_with("WPerformer")).count(), 0);
    }

    #[test]
    fn group_names_are_unique() {
        let p = parts();
        let mut names: Vec<&str> = p.groups.iter().map(|g| g.name.as_str()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
    }

    #[test]
    fn the_stations_carry_the_godot_prompts() {
        let st = stations();
        let ids: Vec<&str> = st.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["monitor", "lamp", "mask", "musicbox", "chair", "ventL", "ventR", "hall_light"]);
        let prompt = |id: &str| st.iter().find(|s| s.id == id).map(|s| s.prompt.clone()).unwrap_or_default();
        assert_eq!(prompt("monitor"), "MONITOR // CAMERAS");
        assert_eq!(prompt("mask"), "MASK // PUT ON / TAKE OFF");
        assert_eq!(prompt("musicbox"), "MUSIC BOX // HOLD TO WIND");
        assert_eq!(prompt("ventL"), "LEFT VENT // HOLD FOR LIGHT");
        assert_eq!(prompt("ventR"), "RIGHT VENT // HOLD FOR LIGHT");
        assert_eq!(prompt("hall_light"), "HALL LIGHT // HOLD");
        assert_eq!(prompt("chair"), "SIT AT DESK");
        assert_eq!(prompt("lamp"), "DESK LAMP");
    }

    fn walk(p: &mut Player, actions: &[&'static str], seconds: f32, solids: &[Aabb]) {
        let b = Bindings::from_save(&Save::default());
        let mut f = InputFrame::default();
        for a in actions {
            f.down.insert(*a);
        }
        f.dt = 0.05;
        let mut t = 0.0;
        while t < seconds {
            p.update(&f, &b, solids, 0.0075);
            t += 0.05;
        }
    }

    #[test]
    fn the_player_stays_in_the_office_the_desk_and_the_rope_block_the_way() {
        let parts = parts();
        let mut solids = parts.solids.clone();
        for g in &parts.groups {
            for c in g.cubes.iter().filter(|c| c.solid) {
                solids.push(Aabb::from_centre(g.origin + c.pos, c.size));
            }
        }
        let spawn = map().spawn;
        let mut p = Player::new(spawn);
        walk(&mut p, &["move_forward"], 3.0, &solids);
        assert!(p.feet.z > -0.2 && p.feet.z < spawn.z, "stopped at the desk, z = {}", p.feet.z);
        // round the desk's end and up to the rope
        let mut q = Player::new(vec3(2.0, 0.0, 1.0));
        walk(&mut q, &["move_forward"], 4.0, &solids);
        assert!(q.feet.z > map().front_z, "the rope keeps her in, z = {}", q.feet.z);
        let mut r = Player::new(spawn);
        walk(&mut r, &["move_right"], 4.0, &solids);
        assert!(r.feet.x < map().half_w, "the side wall holds, x = {}", r.feet.x);
    }

    #[test]
    fn aiming_at_the_monitor_picks_the_monitor_station() {
        let st = stations();
        let parts = parts();
        let mut p = Player::new(map().spawn);
        let eye = p.eye();
        let mon = st.iter().find(|s| s.id == "monitor").expect("monitor").pos;
        let d = mon - eye;
        let yaw = (-d.x).atan2(-d.z);
        let pitch = d.y.atan2((d.x * d.x + d.z * d.z).sqrt());
        p.teleport(map().spawn, yaw, pitch);
        let mut solids = parts.solids.clone();
        for g in &parts.groups {
            for c in g.cubes.iter().filter(|c| c.solid) {
                solids.push(Aabb::from_centre(g.origin + c.pos, c.size));
            }
        }
        let i = p.aim(&st, &solids).expect("something under the crosshair");
        assert_eq!(st[i].id, "monitor");
    }

    #[test]
    fn the_wing_does_not_collide_with_the_player() {
        let p = parts();
        for g in p.groups.iter().filter(|g| g.name.starts_with('W')) {
            assert!(g.cubes.iter().all(|c| !c.solid), "{}", g.name);
        }
    }

    #[test]
    fn the_office_walls_are_solid_and_there_is_no_door() {
        let p = parts();
        let shell = find(&p, "OShell").expect("shell");
        assert!(shell.cubes.iter().any(|c| c.name == "WallBack" && c.solid));
        assert_eq!(shell.cubes.iter().filter(|c| c.name == "WallSide").count(), 2);
        // the front is two wall pieces with an opening between them and a header over it
        assert_eq!(shell.cubes.iter().filter(|c| c.name == "WallFront").count(), 2);
        assert!(shell.cubes.iter().any(|c| c.name == "WallHeader" && !c.solid));
    }

    #[test]
    fn every_camera_has_a_mount_box_and_looks_into_a_room() {
        let p = parts();
        let cams = find(&p, "WCameras").expect("cameras");
        for room in rules().cam_rooms() {
            assert!(cams.cubes.iter().any(|c| c.name == format!("CamBody_{room}")), "{room}");
        }
    }

    #[test]
    fn lights_follow_what_is_held() {
        let none = office_lights(Lit::default(), false, None);
        assert!(none.omni.is_empty() && none.spot.is_empty());
        let both = office_lights(Lit { l: true, r: false, h: true }, true, None);
        assert_eq!(both.omni.len(), 1);
        assert_eq!(both.spot.len(), 2);
        assert!((both.omni[0].energy - 2.2).abs() < 1e-6 && both.omni[0].range == 5.0);
        assert!((both.spot[0].energy - 9.0).abs() < 1e-6 && both.spot[0].range == 20.0);
        assert!((both.spot[0].cos_outer - 24.0f32.to_radians().cos()).abs() < 1e-6);
        assert!(both.spot[0].dir.z < 0.0, "the hall light shines down the hall");
        assert!(both.spot[1].dir.x < 0.0, "the left vent light shines left");
    }

    #[test]
    fn the_ducts_are_dark_unless_their_light_is_held() {
        let dark = feed_lights("ductL", Lit::default());
        assert!(dark.omni.is_empty());
        let normal = LightSet::feed();
        assert!(dark.ambient[0] < normal.ambient[0] * 0.2);
        let lit = feed_lights("ductL", Lit { l: true, r: false, h: false });
        assert_eq!(lit.omni.len(), 1);
        assert!(lit.omni[0].pos.x < 0.0);
        // the other duct's light does nothing for this camera
        assert!(feed_lights("ductL", Lit { l: false, r: true, h: false }).omni.is_empty());
        // ordinary cameras are always bright
        assert!((feed_lights("ballroom", Lit::default()).ambient[0] - normal.ambient[0]).abs() < 1e-6);
        assert!(feed_lights("hall", Lit { l: true, r: true, h: true }).omni.is_empty());
    }

    #[test]
    fn the_stage_platform_and_its_step_make_a_climb() {
        let p = parts();
        let st = find(&p, "WShowStage").expect("stage");
        let top = |n: &str| st.cubes.iter().find(|c| c.name == n).map(|c| c.pos.y + c.size.y * 0.5).unwrap_or(0.0);
        assert!((top("PlatformStage") - 0.7).abs() < 1e-5);
        assert!((top("StepStage") - 0.35).abs() < 1e-5);
    }
}
