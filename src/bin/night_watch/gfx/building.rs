#![allow(dead_code)]
//! The building the classic cameras look into (port of night_watch_building.gd, night_watch_paths.gd and the classic layout of
//! night_watch_cam_map.gd / night_watch_map_registry.gd): the eight camera rooms of the classic building as small blocky rooms on
//! a grid far from the office, each with a camera mount and two floor spots for hunters.
//!
//! All groups are named `bld/<room>/<Prop>` so a feed pass can use `World::cull_draw(&["bld/"])`.
//! `Building::parts` is pure (no GL) for tests; `Building::build` bakes.

use macroquad::prelude::{vec2, vec3, Vec3};

use crate::core::{building, CamLayout, Graph};

use super::art::Textures;
use super::mesh::{Group, World, WorldBuilder};
use super::office::{mul3, put, Office, O};
use super::shader::LightSet;
use super::Gfx;

pub const ORIGIN: Vec3 = vec3(-8.0, 0.0, -40.0);
pub const CELL: f32 = 8.0;
pub const ROOM_W: f32 = 6.0;
pub const ROOM_H: f32 = 3.6;
pub const DOOR_GAP: f32 = 1.8;
pub const DOOR_TOP: f32 = 3.2;
/// NightWatchPaths: the room's open south mouth, the lane point outside it, the office door waypoint distance.
pub const SOUTH_MOUTH: f32 = 2.6;
pub const LANE: f32 = 4.0;
pub const DOOR_OUTER: f32 = 3.4;

/// Wall tint per room (multiplied with the wall texture).
pub fn room_tint(id: &str) -> [f32; 3] {
    match id {
        "stage" => [1.25, 0.8, 1.0],
        "dining" => [1.3, 1.1, 0.85],
        "backstage" => [0.85, 0.95, 1.25],
        "westhall" => [1.0, 1.05, 1.15],
        "closet" => [1.2, 1.0, 0.85],
        "kitchen" => [0.95, 1.25, 1.2],
        "easthall" => [1.0, 1.05, 1.15],
        "corner" => [1.2, 0.95, 1.25],
        _ => [1.0, 1.0, 1.0],
    }
}

/// Rooms that have a doorway in their north / west / east wall (the south side of every room is open to the camera lane).
pub fn has_doorway(side: char, room: &str) -> bool {
    match side {
        'N' => matches!(room, "dining" | "westhall" | "closet" | "corner"),
        'W' => matches!(room, "stage" | "dining" | "easthall"),
        'E' => matches!(room, "backstage" | "westhall" | "dining"),
        _ => false,
    }
}

/// A wall piece on the XZ plane (world rectangle) for the path validator.
#[derive(Clone, Debug)]
pub struct WallBox {
    pub room: String,
    pub name: String,
    pub min: (f32, f32),
    pub size: (f32, f32),
}

/// (camera number, grid) of a camera room; None for doors and unknown ids.
fn room_info(id: &str) -> Option<(i32, (i32, i32))> {
    building().rooms.iter().find(|r| r.id == id && r.cam > 0).map(|r| (r.cam, r.grid))
}

/// The camera rooms in file order.
fn cam_room_ids() -> Vec<String> {
    building().rooms.iter().filter(|r| r.cam > 0).map(|r| r.id.clone()).collect()
}

pub struct BuildingParts {
    pub groups: Vec<Group>,
    pub walls: Vec<WallBox>,
}

pub struct Building {
    pub world: World,
    pub walls: Vec<WallBox>,
}

impl Building {
    /// Pure geometry of all eight rooms.
    pub fn parts() -> BuildingParts {
        let mut groups = Vec::new();
        let mut walls = Vec::new();
        for id in cam_room_ids() {
            let centre = Building::room_centre(&id);
            room_groups(&id, centre, &mut groups, &mut walls);
        }
        BuildingParts { groups, walls }
    }

    pub fn build(tex: &Textures) -> Building {
        let parts = Building::parts();
        let mut wb = WorldBuilder::new();
        wb.groups = parts.groups;
        let world = wb.bake(tex);
        Building { world, walls: parts.walls }
    }

    /// The room's centre on the floor; Vec3::ZERO for ids that are not camera rooms.
    pub fn room_centre(room: &str) -> Vec3 {
        match room_info(room) {
            Some((_, g)) => ORIGIN + vec3(g.0 as f32 * CELL, 0.0, g.1 as f32 * CELL),
            None => Vec3::ZERO,
        }
    }

    /// World position of hunter slot 0 or 1 in a room (the second stands a little to the side and further back).
    pub fn stand_position(room: &str, slot: usize) -> Vec3 {
        if room_info(room).is_none() {
            return Vec3::ZERO;
        }
        let mut offset = if slot == 0 { vec3(-0.6, 0.0, 0.4) } else { vec3(1.0, 0.0, -0.6) };
        if room == "stage" {
            offset += vec3(0.0, 0.45, -1.4);
        }
        Building::room_centre(room) + offset
    }

    /// The feed camera's mount: high in the open (south) corner of the room, looking in. (position, target)
    pub fn camera_transform(room: &str) -> (Vec3, Vec3) {
        let c = Building::room_centre(room);
        (c + vec3(-1.2, 2.9, 4.4), c + vec3(0.0, 0.8, -0.8))
    }

    pub fn has_doorway(side: char, room: &str) -> bool {
        has_doorway(side, room)
    }

    /// Extra pub item: begin the feed target for a room. The mode then calls `draw_feed_world`, draws hunters, and ends with `gfx.feed.end()`.
    pub fn begin_feed(gfx: &Gfx, room: &str) -> Vec3 {
        let (pos, target) = Building::camera_transform(room);
        gfx.feed.begin(pos, target, Vec3::Y);
        pos
    }

    /// Extra pub item: the building drawn with the bright feed lighting, inside an already begun camera.
    pub fn draw_feed_world(&self, gfx: &Gfx, cam_pos: Vec3) {
        self.world.draw(&gfx.shader, &LightSet::feed(), cam_pos);
    }

    /// The whole feed pass for a room with nothing else in it (begin, draw, end). Modes that draw hunters into the feed use
    /// `begin_feed` / `draw_feed_world` / `gfx.feed.end()` instead.
    pub fn draw_feed_pass(&self, gfx: &Gfx, room: &str) {
        let pos = Building::begin_feed(gfx, room);
        self.draw_feed_world(gfx, pos);
        gfx.feed.end();
    }

    /// NightWatchPaths: per room C (centre), S (south mouth), L (lane) nodes; same-row lane points linked, doorways link neighbouring
    /// centres, north doorways link a room's lane to the room below; DO/D office nodes with a jump edge from the hall's lane.
    pub fn paths(office_origin: Vec3) -> Graph {
        let mut g = Graph::new();
        let ids = cam_room_ids();
        for id in &ids {
            let c = Building::room_centre(id);
            g.add_node(&format!("C:{id}"), c);
            g.add_node(&format!("S:{id}"), c + vec3(0.0, 0.0, SOUTH_MOUTH));
            g.add_node(&format!("L:{id}"), c + vec3(0.0, 0.0, LANE));
            g.link(&format!("C:{id}"), &format!("S:{id}"));
            g.link(&format!("S:{id}"), &format!("L:{id}"));
        }
        // (grid x, grid y) -> room id, in file order
        let rows: Vec<((i32, i32), String)> = ids.iter().filter_map(|id| room_info(id).map(|(_, gr)| (gr, id.clone()))).collect();
        let at = |gx: i32, gy: i32| rows.iter().find(|(gr, _)| *gr == (gx, gy)).map(|(_, id)| id.clone());
        for ((gx, gy), id) in &rows {
            if let Some(right) = at(gx + 1, *gy) {
                g.link(&format!("L:{id}"), &format!("L:{right}"));
                if has_doorway('E', id) && has_doorway('W', &right) {
                    g.link(&format!("C:{id}"), &format!("C:{right}"));
                }
            }
            if let Some(below) = at(*gx, gy + 1) {
                if has_doorway('N', &below) {
                    g.link(&format!("L:{id}"), &format!("C:{below}"));
                }
            }
        }
        // the office doors hang off the halls: DO (down the hall) is one jump from the hall's lane point
        for door in ["doorL", "doorR"] {
            let hall = if door == "doorL" { "westhall" } else { "easthall" };
            let stand = office_origin + Office::door_stand(door, 0, 0.0);
            let dir = if door == "doorL" { -1.0 } else { 1.0 };
            let outer = stand + vec3(dir, 0.0, 0.0) * DOOR_OUTER;
            g.add_node(&format!("DO:{door}"), outer);
            g.add_node(&format!("D:{door}"), stand);
            g.link_jump(&format!("L:{hall}"), &format!("DO:{door}"));
            g.link(&format!("DO:{door}"), &format!("D:{door}"));
        }
        g
    }

    /// The waypoint a sim room hangs from.
    pub fn key_for(room: &str) -> String {
        if room.starts_with("door") {
            format!("D:{room}")
        } else {
            format!("C:{room}")
        }
    }

    /// The route a hunter takes when the sim moves her from one room to another.
    pub fn legs_for(graph: &Graph, from_room: &str, to_room: &str) -> Vec<crate::core::Leg> {
        graph.route(&Building::key_for(from_room), &Building::key_for(to_room))
    }

    /// The route from wherever a hunter stands now to a sim room.
    pub fn legs_from(graph: &Graph, pos: Vec3, to_room: &str) -> Vec<crate::core::Leg> {
        graph.legs_from(pos, &Building::key_for(to_room))
    }
}

/// The classic camera screen layout (night_watch_map_registry.gd classic_layout): the eight camera rooms at their grid positions
/// (x, z), the graph edges between camera rooms, the two halls as entrances, camera order and labels.
pub fn classic_layout() -> CamLayout {
    let b = building();
    let mut rooms: Vec<(String, macroquad::prelude::Vec2)> = Vec::new();
    for r in b.rooms.iter().filter(|r| r.cam > 0) {
        let c = ORIGIN + vec3(r.grid.0 as f32 * CELL, 0.0, r.grid.1 as f32 * CELL);
        rooms.push((r.id.clone(), vec2(c.x, c.z)));
    }
    let has = |id: &str| rooms.iter().any(|(r, _)| r == id);
    let mut edges: Vec<(String, String)> = Vec::new();
    for r in b.rooms.iter() {
        if let Some(adj) = b.adj.get(&r.id) {
            for n in adj {
                // the two door nodes belong to the office, not the map
                if r.id < *n && has(&r.id) && has(n) {
                    edges.push((r.id.clone(), n.clone()));
                }
            }
        }
    }
    let mut cams: Vec<(i32, String)> = b.rooms.iter().filter(|r| r.cam > 0).map(|r| (r.cam, r.id.clone())).collect();
    cams.sort();
    let cams: Vec<String> = cams.into_iter().map(|(_, id)| id).collect();
    let labels: Vec<(String, String)> = cams
        .iter()
        .map(|id| (id.clone(), b.rooms.iter().find(|r| &r.id == id).map(|r| r.label.clone()).unwrap_or_default()))
        .collect();
    CamLayout {
        rooms,
        edges,
        entrances: vec!["westhall".to_string(), "easthall".to_string()],
        cams,
        labels,
        vents: Vec::new(),
        office: None,
    }
}

// ---- geometry ----------------------------------------------------------------------------------------------------------------

fn room_groups(id: &str, centre: Vec3, groups: &mut Vec<Group>, walls: &mut Vec<WallBox>) {
    let tint = room_tint(id);
    let half = ROOM_W * 0.5;
    let mk = |name: &str, local: Vec3| Group::new(&format!("bld/{id}/{name}"), centre + local);

    // the shell: floor tiles, ceiling, three walls (the south side is open so the feed sees in)
    let mut shell = mk("Shell", Vec3::ZERO);
    put(&mut shell, "Floor", [ROOM_W, 0.2, ROOM_W], [0.0, -0.1, 0.0], O::new().t(mul3(tint, [0.95, 0.95, 0.95])));
    put(&mut shell, "Ceiling", [ROOM_W, 0.2, ROOM_W], [0.0, ROOM_H + 0.1, 0.0], O::new().t(mul3(tint, [0.6, 0.6, 0.6])));
    wall(&mut shell, centre, 'N', id, tint, walls);
    wall(&mut shell, centre, 'W', id, mul3(tint, [1.05, 1.05, 1.05]), walls);
    wall(&mut shell, centre, 'E', id, mul3(tint, [1.05, 1.05, 1.05]), walls);
    put(&mut shell, "Baseboard", [ROOM_W, 0.24, 0.08], [0.0, 0.12, -half + 0.14], O::new().t([0.7, 0.7, 0.75]));
    for sx in [-half + 0.14, half - 0.14] {
        put(&mut shell, "Baseboard", [0.08, 0.24, ROOM_W], [sx, 0.12, 0.0], O::new().t([0.7, 0.7, 0.75]));
        put(&mut shell, "CrownTrim", [0.08, 0.12, ROOM_W], [sx, ROOM_H - 0.06, 0.0], O::new().t([0.85, 0.85, 0.9]));
    }
    put(&mut shell, "CrownTrim", [ROOM_W, 0.12, 0.08], [0.0, ROOM_H - 0.06, -half + 0.14], O::new().t([0.85, 0.85, 0.9]));
    for x in [-2.0f32, 0.0, 2.0] {
        put(&mut shell, "PanelLine", [0.06, ROOM_H - 0.6, 0.05], [x, ROOM_H * 0.5, -half + 0.12], O::new().t([0.8, 0.8, 0.85]));
    }
    put(&mut shell, "PipeRoom", [ROOM_W - 0.4, 0.11, 0.11], [0.0, ROOM_H - 0.22, -half + 0.2], O::new().k("metal"));
    put(&mut shell, "CeilingPanel", [1.4, 0.05, 1.4], [0.0, ROOM_H - 0.04, 0.0], O::new());
    put(&mut shell, "FloorStripe", [ROOM_W, 0.02, 0.3], [0.0, 0.01, 0.8], O::new().hz("y").g(0.8));
    put(&mut shell, "LedSmoke", [0.05, 0.05, 0.05], [-half + 0.2, 2.6, half - 0.3], O::new());
    groups.push(shell);

    // a vent grille on the north wall, wall bolts, a floor drain and a little clutter in every room
    let mut detail = mk("Details", Vec3::ZERO);
    put(&mut detail, "VentGrille", [0.7, 0.4, 0.06], [1.9, 2.5, -half + 0.14], O::new().k("metal"));
    for i in 0..3 {
        put(&mut detail, "VentSlat", [0.6, 0.03, 0.04], [1.9, 2.4 + i as f32 * 0.1, -half + 0.19], O::new().k("plastic"));
    }
    for x in [-2.6f32, -1.0, 1.0, 2.6] {
        put(&mut detail, "WallBolt", [0.06, 0.06, 0.04], [x, 0.4, -half + 0.12], O::new().k("metal").g(1.2));
    }
    put(&mut detail, "FloorDrain", [0.3, 0.02, 0.3], [2.0, 0.01, 1.6], O::new().k("metal").t([0.6, 0.6, 0.65]));
    put(&mut detail, "ClutterBox", [0.2, 0.2, 0.2], [-2.4, 0.1, 2.0], O::new().k("wood").t([0.9, 0.85, 0.8]));
    put(&mut detail, "ClutterCan", [0.16, 0.16, 0.16], [-2.15, 0.08, 2.1], O::new().k("metal").t([0.8, 1.1, 0.8]));
    put(&mut detail, "ClutterPaper", [0.3, 0.02, 0.22], [-2.3, 0.02, 1.7], O::new().k("paper").t([1.2, 1.2, 1.1]).ao(1.0));
    groups.push(detail);

    match id {
        "stage" => stage(&mk, groups),
        "dining" => dining(&mk, groups),
        "backstage" => backstage(&mk, groups, half),
        "westhall" | "easthall" => hall(&mk, groups, id, half),
        "closet" => closet(&mk, groups),
        "kitchen" => kitchen(&mk, groups),
        "corner" => corner(&mk, groups),
        _ => {}
    }
}

/// One wall of a room: a single slab, or two slabs and a lintel around a doorway.
fn wall(shell: &mut Group, room_centre: Vec3, side: char, id: &str, tint: [f32; 3], walls: &mut Vec<WallBox>) {
    let half = ROOM_W * 0.5;
    let along_x = side == 'N';
    let centre = if side == 'N' {
        vec3(0.0, ROOM_H * 0.5, -half)
    } else {
        vec3(if side == 'W' { -half } else { half }, ROOM_H * 0.5, 0.0)
    };
    let thick = 0.2;
    let wname = format!("Wall{side}");
    let mut record = |c: Vec3, length: f32, name: &str| {
        let w = room_centre + c;
        let size = if along_x { (length, thick) } else { (thick, length) };
        walls.push(WallBox { room: id.to_string(), name: name.to_string(), min: (w.x - size.0 * 0.5, w.z - size.1 * 0.5), size });
    };
    if !has_doorway(side, id) {
        let size = if along_x { [ROOM_W, ROOM_H, thick] } else { [thick, ROOM_H, ROOM_W] };
        put(shell, &wname, size, [centre.x, centre.y, centre.z], O::new().t(tint));
        record(centre, ROOM_W, &wname);
        return;
    }
    let seg = (ROOM_W - DOOR_GAP) * 0.5;
    let off = DOOR_GAP * 0.5 + seg * 0.5;
    for sign in [-1.0f32, 1.0] {
        let c = if along_x { centre + vec3(sign * off, 0.0, 0.0) } else { centre + vec3(0.0, 0.0, sign * off) };
        let size = if along_x { [seg, ROOM_H, thick] } else { [thick, ROOM_H, seg] };
        let name = if sign < 0.0 { wname.clone() } else { format!("{wname}2") };
        put(shell, &name, size, [c.x, c.y, c.z], O::new().t(tint));
        record(c, seg, &wname);
    }
    let lintel_h = ROOM_H - DOOR_TOP;
    let size = if along_x { [DOOR_GAP, lintel_h, thick] } else { [thick, lintel_h, DOOR_GAP] };
    put(shell, &format!("{wname}Lintel"), size, [centre.x, DOOR_TOP + lintel_h * 0.5, centre.z], O::new().t(mul3(tint, [0.9, 0.9, 0.9])));
}

type Mk<'a> = &'a dyn Fn(&str, Vec3) -> Group;

fn stage(mk: Mk, groups: &mut Vec<Group>) {
    let mut s = mk("Stage", vec3(0.0, 0.0, -1.8));
    put(&mut s, "PlatformDeck", [4.4, 0.45, 2.0], [0.0, 0.225, 0.0], O::new());
    put(&mut s, "PlatformEdge", [4.5, 0.06, 2.1], [0.0, 0.46, 0.0], O::new().g(1.2));
    for i in 0..2 {
        let f = i as f32;
        put(&mut s, "PlatformStep", [1.4, 0.15 + f * 0.15, 0.3], [0.0, 0.075 + f * 0.075, 1.15 + (1.0 - f) * 0.3], O::new());
    }
    groups.push(s);
    let mut c = mk("Curtains", Vec3::ZERO);
    for x in [-2.4f32, 2.4] {
        put(&mut c, "CurtainDrape", [0.5, 2.6, 0.14], [x, 1.4, -2.7], O::new().t([1.0, 0.35, 0.4]));
        put(&mut c, "CurtainTie", [0.1, 0.1, 0.18], [x + if x < 0.0 { 0.3 } else { -0.3 }, 1.0, -2.68], O::new().k("label"));
    }
    put(&mut c, "CurtainValance", [5.4, 0.3, 0.16], [0.0, 2.7, -2.7], O::new().t([1.0, 0.35, 0.4]));
    groups.push(c);
    let mut l = mk("StageLights", vec3(0.0, 2.8, -1.2));
    put(&mut l, "LightBar", [4.0, 0.08, 0.08], [0.0, 0.0, 0.0], O::new().k("metal"));
    for x in [-1.6f32, -0.5, 0.5, 1.6] {
        put(&mut l, "LightCan", [0.22, 0.22, 0.3], [x, -0.15, 0.0], O::new().k("plastic"));
        put(&mut l, "GlowCan", [0.14, 0.14, 0.04], [x, -0.15, 0.17], O::new());
    }
    groups.push(l);
    let mut m = mk("Microphone", vec3(0.0, 0.46, -0.9));
    put(&mut m, "MicBase", [0.3, 0.04, 0.3], [0.0, 0.02, 0.0], O::new().k("metal"));
    put(&mut m, "MicStand", [0.04, 1.2, 0.04], [0.0, 0.6, 0.0], O::new().k("metal"));
    put(&mut m, "MicHead", [0.08, 0.16, 0.08], [0.0, 1.28, 0.0], O::new().k("plastic"));
    groups.push(m);
}

fn dining(mk: Mk, groups: &mut Vec<Group>) {
    let tables: [(f32, f32); 4] = [(-1.8, -1.2), (1.8, -1.2), (-1.8, 1.4), (1.8, 1.4)];
    for (i, (tx, tz)) in tables.iter().enumerate() {
        let mut t = mk(&format!("Table{i}"), vec3(*tx, 0.0, *tz));
        put(&mut t, "TableTop", [1.2, 0.08, 1.2], [0.0, 0.75, 0.0], O::new());
        put(&mut t, "TableCloth", [1.26, 0.03, 1.26], [0.0, 0.8, 0.0], O::new().k("fabric").t([1.3, 0.8, 0.8]));
        put(&mut t, "TableLeg", [0.12, 0.75, 0.12], [0.0, 0.375, 0.0], O::new());
        put(&mut t, "TableFoot", [0.5, 0.03, 0.5], [0.0, 0.02, 0.0], O::new().k("metal"));
        put(&mut t, "TableVase", [0.1, 0.16, 0.1], [0.0, 0.89, 0.0], O::new().k("glass"));
        for cx in [-0.8f32, 0.8] {
            put(&mut t, "ChairSeat", [0.4, 0.06, 0.4], [cx, 0.46, 0.0], O::new());
            put(&mut t, "ChairBack", [0.06, 0.4, 0.4], [cx + if cx < 0.0 { -0.17 } else { 0.17 }, 0.7, 0.0], O::new());
            for lz in [-0.15f32, 0.15] {
                put(&mut t, "ChairLeg", [0.05, 0.43, 0.05], [cx, 0.215, lz], O::new());
            }
        }
        groups.push(t);
    }
}

fn backstage(mk: Mk, groups: &mut Vec<Group>, half: f32) {
    let mut c = mk("Crates", vec3(-1.3, 0.0, -1.95));
    for p in [[-0.5f32, 0.45, 0.0], [0.5, 0.45, -0.05], [0.0, 1.35, -0.02]] {
        put(&mut c, "CrateBox", [0.9, 0.9, 0.9], p, O::new());
        put(&mut c, "CrateBand", [0.94, 0.08, 0.94], [p[0], p[1] + 0.3, p[2]], O::new().k("metal").t([0.8, 0.8, 0.85]));
    }
    groups.push(c);
    let mut r = mk("CostumeRack", vec3(1.6, 0.0, -1.8));
    put(&mut r, "RackRail", [1.6, 0.05, 0.05], [0.0, 1.7, 0.0], O::new().k("metal"));
    for x in [-0.75f32, 0.75] {
        put(&mut r, "RackPost", [0.05, 1.7, 0.05], [x, 0.85, 0.0], O::new().k("metal"));
        put(&mut r, "RackFoot", [0.05, 0.05, 0.5], [x, 0.025, 0.0], O::new().k("metal"));
    }
    for i in 0..5 {
        let f = i as f32;
        let odd = (i % 2) as f32;
        put(&mut r, "RackCostume", [0.22, 0.9 - odd * 0.2, 0.12], [-0.6 + f * 0.3, 1.2 - odd * 0.1, 0.0], O::new().k("fabric").t([0.8 + 0.15 * f, 1.0 - 0.1 * f, 1.2]));
    }
    groups.push(r);
    let mut m = mk("Mirror", vec3(-half + 0.2, 1.4, 0.4));
    put(&mut m, "MirrorFrame", [0.08, 1.4, 0.9], [0.0, 0.0, 0.0], O::new());
    put(&mut m, "MirrorGlass", [0.05, 1.3, 0.8], [0.04, 0.0, 0.0], O::new().k("glass"));
    groups.push(m);
}

fn hall(mk: Mk, groups: &mut Vec<Group>, id: &str, half: f32) {
    let side = if id == "westhall" { -1.0f32 } else { 1.0 };
    let mut l = mk("Lockers", vec3(side * (half - 0.35), 0.0, 1.4));
    for i in 0..3 {
        let f = i as f32;
        put(&mut l, "LockerBody", [0.55, 2.1, 0.7], [0.0, 1.05, -f * 0.74], O::new().t([0.75 + 0.1 * f, 0.85, 1.0]));
        put(&mut l, "LockerDoor", [0.05, 1.9, 0.6], [-side * 0.29, 1.05, -f * 0.74], O::new().k("metal").t([0.9, 0.9, 0.95]));
        put(&mut l, "LockerHandle", [0.05, 0.06, 0.18], [-side * 0.32, 1.1, -f * 0.74 + 0.2], O::new().k("plastic").g(1.4));
        for v in 0..3 {
            put(&mut l, "LockerVent", [0.05, 0.02, 0.3], [-side * 0.32, 1.7 + v as f32 * 0.06, -f * 0.74], O::new().k("plastic"));
        }
    }
    groups.push(l);
    let mut r = mk("Runner", Vec3::ZERO);
    put(&mut r, "RunnerMat", [0.9, 0.04, 4.8], [0.0, 0.02, 0.0], O::new().k("fabric").t([0.9, 0.8, 1.0]));
    for z in [-2.2f32, 2.2] {
        put(&mut r, "RunnerTrim", [0.9, 0.045, 0.1], [0.0, 0.025, z], O::new().hz("y").g(0.9));
    }
    groups.push(r);
    let mut s = mk("Signs", vec3(0.0, 2.2, -half + 0.2));
    put(&mut s, "SignPlate", [1.4, 0.3, 0.05], [0.0, 0.0, 0.0], O::new().k("plastic"));
    put(&mut s, "SignText", [1.2, 0.14, 0.03], [0.0, 0.0, 0.03], O::new());
    groups.push(s);
    let mut e = mk("EmergencyLight", vec3(side * (half - 0.25), 2.5, -0.5));
    put(&mut e, "EmergencyHousing", [0.2, 0.14, 0.2], [0.0, 0.0, 0.0], O::new().k("plastic"));
    put(&mut e, "GlowEmergency", [0.14, 0.08, 0.04], [0.0, 0.0, 0.11], O::new().k("red"));
    groups.push(e);
}

fn closet(mk: Mk, groups: &mut Vec<Group>) {
    let mut s = mk("Shelves", vec3(-2.4, 0.0, -0.4));
    for z in [-1.8f32, 1.8] {
        put(&mut s, "ShelfSide", [0.5, 2.4, 0.06], [0.0, 1.2, z], O::new());
    }
    for i in 0..4 {
        put(&mut s, "ShelfBoard", [0.5, 0.05, 3.6], [0.0, 0.3 + i as f32 * 0.6, 0.0], O::new());
    }
    for i in 0..7 {
        let f = i as f32;
        put(
            &mut s,
            "ShelfBox",
            [0.36, 0.2 + (i % 3) as f32 * 0.06, 0.34],
            [0.0, 0.45 + (i % 4) as f32 * 0.6, -1.5 + f * 0.5],
            O::new().k("wood").t([0.8 + (i % 3) as f32 * 0.12, 0.85, 0.8]),
        );
    }
    groups.push(s);
    let mut b = mk("MopBucket", vec3(1.6, 0.0, -2.0));
    put(&mut b, "BucketBody", [0.42, 0.34, 0.42], [0.0, 0.17, 0.0], O::new().k("plastic").t([1.2, 1.4, 1.8]));
    put(&mut b, "MopHandle", [0.04, 1.5, 0.04], [0.1, 0.9, 0.0], O::new());
    put(&mut b, "MopHead", [0.18, 0.2, 0.18], [0.1, 0.35, 0.0], O::new().k("fabric").g(1.3));
    groups.push(b);
    let mut p = mk("Paint", vec3(0.6, 0.0, -2.4));
    for i in 0..3 {
        let f = i as f32;
        put(&mut p, "PaintCan", [0.22, 0.22, 0.22], [-0.3 + f * 0.28, 0.11, 0.0], O::new().k("metal").t([0.8 + f * 0.2, 1.0 - f * 0.2, 0.8]));
    }
    groups.push(p);
}

fn kitchen(mk: Mk, groups: &mut Vec<Group>) {
    let mut c = mk("Counter", vec3(0.0, 0.0, -2.3));
    put(&mut c, "CounterBody", [4.6, 0.9, 0.9], [0.0, 0.45, 0.0], O::new());
    put(&mut c, "CounterTop", [4.7, 0.06, 1.0], [0.0, 0.93, 0.0], O::new().k("metal").t([1.2, 1.2, 1.25]));
    for x in [-1.8f32, -0.6, 0.6, 1.8] {
        put(&mut c, "CounterDoor", [0.9, 0.5, 0.04], [x, 0.5, 0.47], O::new().k("plastic").g(1.3));
        put(&mut c, "CounterHandle", [0.12, 0.03, 0.04], [x, 0.7, 0.5], O::new().k("metal"));
    }
    put(&mut c, "SinkBasin", [0.7, 0.04, 0.5], [1.4, 0.97, 0.0], O::new().k("metal").t([0.6, 0.6, 0.65]));
    put(&mut c, "TapNeck", [0.06, 0.3, 0.06], [1.4, 1.1, -0.3], O::new().k("metal"));
    groups.push(c);
    let mut h = mk("Hood", vec3(-1.2, 2.0, -2.45));
    put(&mut h, "HoodBody", [1.6, 0.5, 0.7], [0.0, 0.25, 0.0], O::new().k("metal"));
    put(&mut h, "HoodDuct", [0.4, 0.8, 0.4], [0.0, 0.9, 0.0], O::new().k("metal"));
    for i in 0..3 {
        put(&mut h, "HangingPan", [0.08, 0.5, 0.06], [-0.5 + i as f32 * 0.5, -0.45, 0.2], O::new().k("metal").t([0.6, 0.6, 0.65]));
    }
    groups.push(h);
    let mut f = mk("Fridge", vec3(2.1, 0.0, 1.5));
    put(&mut f, "FridgeBody", [1.0, 2.0, 1.0], [0.0, 1.0, 0.0], O::new().t([1.2, 1.25, 1.3]));
    put(&mut f, "FridgeHandle", [0.05, 0.6, 0.06], [-0.52, 1.3, 0.3], O::new().k("metal"));
    put(&mut f, "FridgeSeam", [0.9, 0.04, 0.04], [0.0, 1.4, -0.52], O::new().k("plastic"));
    groups.push(f);
    let mut s = mk("Stove", vec3(-1.5, 0.0, -2.3));
    for x in [-0.3f32, 0.3] {
        for z in [-0.2f32, 0.2] {
            put(&mut s, "StoveBurner", [0.28, 0.03, 0.28], [x, 0.97, z], O::new().k("plastic"));
        }
    }
    groups.push(s);
}

fn corner(mk: Mk, groups: &mut Vec<Group>) {
    let mut p = mk("Pillar", vec3(-2.0, 0.0, -2.0));
    put(&mut p, "PillarBody", [0.7, 3.2, 0.7], [0.0, 1.6, 0.0], O::new());
    put(&mut p, "PillarFoot", [0.85, 0.25, 0.85], [0.0, 0.125, 0.0], O::new().g(1.2));
    put(&mut p, "PillarCap", [0.85, 0.2, 0.85], [0.0, 3.0, 0.0], O::new().g(1.2));
    put(&mut p, "PillarSign", [0.1, 0.5, 0.05], [0.0, 1.5, 0.38], O::new().k("label"));
    groups.push(p);
    let mut b = mk("Bench", vec3(2.0, 0.0, -2.3));
    put(&mut b, "BenchSeat", [1.6, 0.1, 0.55], [0.0, 0.5, 0.0], O::new());
    put(&mut b, "BenchBack", [1.6, 0.5, 0.08], [0.0, 0.8, -0.25], O::new());
    for x in [-0.7f32, 0.7] {
        put(&mut b, "BenchLeg", [0.1, 0.5, 0.45], [x, 0.25, 0.0], O::new().k("metal"));
    }
    groups.push(b);
    let mut v = mk("VendingMachine", vec3(0.3, 0.0, -2.2));
    put(&mut v, "VendBody", [1.0, 1.9, 0.8], [0.0, 0.95, 0.0], O::new().k("plastic").t([1.6, 0.9, 0.9]));
    put(&mut v, "VendGlass", [0.7, 1.2, 0.04], [-0.08, 1.1, 0.42], O::new().k("glass"));
    put(&mut v, "SignVend", [0.7, 0.08, 0.03], [-0.08, 1.75, 0.44], O::new());
    for i in 0..4 {
        put(&mut v, "VendShelf", [0.62, 0.03, 0.2], [-0.08, 0.75 + i as f32 * 0.28, 0.35], O::new().k("metal"));
    }
    put(&mut v, "VendKeypad", [0.14, 0.5, 0.04], [0.38, 1.2, 0.42], O::new().k("plastic"));
    groups.push(v);
    let mut pl = mk("Plant", vec3(-0.3, 0.0, 1.9));
    put(&mut pl, "PlantPot", [0.4, 0.4, 0.4], [0.0, 0.2, 0.0], O::new().k("concrete").t([1.3, 0.9, 0.8]));
    for i in 0..4 {
        let f = i as f32;
        put(
            &mut pl,
            "PlantLeaf",
            [0.12, 0.5 + 0.1 * f, 0.12],
            [-0.1 + (i % 2) as f32 * 0.2, 0.65 + 0.05 * f, -0.08 + (i / 2) as f32 * 0.16],
            O::new().k("green").t([0.6, 1.0, 0.6]),
        );
    }
    groups.push(pl);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn room_centres_follow_the_grid() {
        assert!(near(Building::room_centre("stage"), vec3(0.0, 0.0, -40.0)));
        assert!(near(Building::room_centre("backstage"), vec3(-8.0, 0.0, -40.0)));
        assert!(near(Building::room_centre("corner"), vec3(8.0, 0.0, -24.0)));
        assert!(near(Building::room_centre("doorL"), Vec3::ZERO));
        assert!(near(Building::room_centre("nowhere"), Vec3::ZERO));
    }

    #[test]
    fn camera_transform_for_dining() {
        let (pos, target) = Building::camera_transform("dining");
        // dining is grid (1,1): centre (0, 0, -32)
        assert!(near(pos, vec3(-1.2, 2.9, -27.6)));
        assert!(near(target, vec3(0.0, 0.8, -32.8)));
    }

    #[test]
    fn stand_positions_and_the_stage_platform() {
        let c = Building::room_centre("kitchen");
        assert!(near(Building::stand_position("kitchen", 0), c + vec3(-0.6, 0.0, 0.4)));
        assert!(near(Building::stand_position("kitchen", 1), c + vec3(1.0, 0.0, -0.6)));
        let s = Building::room_centre("stage");
        assert!(near(Building::stand_position("stage", 0), s + vec3(-0.6, 0.45, -1.0)));
        assert!(near(Building::stand_position("stage", 1), s + vec3(1.0, 0.45, -2.0)));
        assert!(near(Building::stand_position("doorL", 0), Vec3::ZERO));
    }

    #[test]
    fn doorway_table() {
        assert!(Building::has_doorway('N', "dining"));
        assert!(!Building::has_doorway('N', "stage"));
        assert!(Building::has_doorway('W', "stage"));
        assert!(Building::has_doorway('E', "backstage"));
        assert!(!Building::has_doorway('E', "kitchen"));
        assert!(!Building::has_doorway('S', "dining"));
    }

    #[test]
    fn paths_link_the_hall_to_the_door_with_a_jump_and_route_from_the_stage() {
        let g = Building::paths(Vec3::ZERO);
        // L:westhall - DO:doorL is a jump edge: routing there marks the final leg
        let r = g.route("L:westhall", "DO:doorL");
        assert_eq!(r.len(), 2);
        assert!(!r[0].jump && r[1].jump);
        let legs = Building::legs_for(&g, "stage", "doorL");
        assert!(legs.len() > 4);
        assert!(!legs[0].jump);
        assert_eq!(legs[0].key, "C:stage");
        assert_eq!(legs.last().map(|l| l.key.as_str()), Some("D:doorL"));
        assert_eq!(legs.iter().filter(|l| l.jump).count(), 1);
        let right = Building::legs_for(&g, "stage", "doorR");
        assert_eq!(right.last().map(|l| l.key.as_str()), Some("D:doorR"));
        assert!(near(g.pos("D:doorL").unwrap_or(Vec3::ZERO), vec3(-4.9, 0.0, 0.3)));
        assert!(near(g.pos("DO:doorL").unwrap_or(Vec3::ZERO), vec3(-8.3, 0.0, 0.3)));
        assert!(near(g.pos("DO:doorR").unwrap_or(Vec3::ZERO), vec3(8.3, 0.0, -0.3)));
        // the hunter walks from the stage platform: nearest node is the stage centre
        let from = Building::legs_from(&g, Building::stand_position("stage", 0), "doorR");
        assert_eq!(from.last().map(|l| l.key.as_str()), Some("D:doorR"));
    }

    #[test]
    fn key_for_rooms_and_doors() {
        assert_eq!(Building::key_for("stage"), "C:stage");
        assert_eq!(Building::key_for("doorL"), "D:doorL");
    }

    #[test]
    fn eight_rooms_with_named_groups_and_walls() {
        let p = Building::parts();
        for room in ["stage", "dining", "backstage", "westhall", "closet", "kitchen", "easthall", "corner"] {
            assert!(p.groups.iter().any(|g| g.name == format!("bld/{room}/Shell")), "{room} shell");
            assert!(p.groups.iter().any(|g| g.name == format!("bld/{room}/Details")), "{room} details");
        }
        assert!(p.groups.iter().all(|g| g.name.starts_with("bld/")));
        // dining has four tables; the stage has its platform
        assert_eq!(p.groups.iter().filter(|g| g.name.starts_with("bld/dining/Table")).count(), 4);
        assert!(p.groups.iter().any(|g| g.name == "bld/stage/Stage"));
        // a doorway wall is two pieces, a plain wall is one
        let dining_n = p.walls.iter().filter(|w| w.room == "dining" && w.name == "WallN").count();
        let stage_n = p.walls.iter().filter(|w| w.room == "stage" && w.name == "WallN").count();
        assert_eq!(dining_n, 2);
        assert_eq!(stage_n, 1);
        // the piece beside a doorway is (6 - 1.8) / 2 = 2.1 long
        let w = p.walls.iter().find(|w| w.room == "dining" && w.name == "WallN").map(|w| w.size);
        assert_eq!(w.map(|s| (s.0 * 10.0).round()), Some(21.0));
        for g in &p.groups {
            for c in &g.cubes {
                assert!(!c.kind.is_empty() && c.size.is_finite() && c.pos.is_finite());
            }
        }
    }

    #[test]
    fn classic_layout_matches_the_grid_and_graph() {
        let l = classic_layout();
        assert_eq!(l.rooms.len(), 8);
        assert_eq!(l.cams.len(), 8);
        assert_eq!(l.cams[0], "stage");
        assert_eq!(l.cams[7], "corner");
        assert_eq!(l.entrances, vec!["westhall".to_string(), "easthall".to_string()]);
        assert!(l.vents.is_empty() && l.office.is_none());
        let stage = l.rooms.iter().find(|(id, _)| id == "stage").map(|(_, p)| *p);
        assert_eq!(stage.map(|p| (p.x, p.y)), Some((0.0, -40.0)));
        // door nodes are not map rooms; edges are listed once
        assert!(l.edges.iter().all(|(a, b)| a < b && a != "doorL" && b != "doorL" && a != "doorR" && b != "doorR"));
        assert!(l.edges.contains(&("dining".to_string(), "stage".to_string())));
        assert!(l.labels.contains(&("stage".to_string(), "SHOW STAGE".to_string())));
        assert_eq!(l.labels.len(), 8);
    }
}
