#![allow(dead_code)]
//! The classic security room (port of night_watch_office.gd): a walkable blocky 6 x 5 m room with a desk, monitor, radio and
//! lamp, a chair, a shelf, a breaker box and a vent hatch, a doorway on each side wall with a sliding slab, a control panel
//! beside it and a short dark hall behind it. Everything is cubes in named groups (see `Group`).
//!
//! `Office::parts` is pure (no GL) so tests can inspect the groups, solids and stations; `Office::build` bakes them.
//! The same shell serves the Cargo (dayshift) and Hub (mixed) variants through `OfficeConfig`.

use macroquad::prelude::{vec3, Vec3};

use super::art::{is_emissive_name, kind_for, Textures};
use super::mesh::{Aabb, Cube, Group, World, WorldBuilder};
use super::palette;
use super::shader::{LightSet, Omni, Spot};
use super::Gfx;

pub const HALF_W: f32 = 3.0;
pub const HALF_D: f32 = 2.5;
pub const HEIGHT: f32 = 3.6;
pub const DOOR_W: f32 = 1.7;
pub const DOOR_H: f32 = 3.2;
pub const SLAB_OPEN_Y: f32 = DOOR_H + 1.3;
pub const SLAB_CLOSED_Y: f32 = DOOR_H * 0.5;
/// Where the player starts (in the room, facing the desk) and the desk seat.
pub const SPAWN: Vec3 = vec3(0.0, 0.0, 0.9);
pub const SEAT: Vec3 = vec3(0.0, 0.0, -1.05);
/// The hall mouth the player cannot pass: x beyond this is the hunters' side.
pub const HALL_GUARD_X: f32 = 3.7;
/// Slab progress above which the slab blocks the doorway.
pub const SLAB_BLOCK_AT: f32 = 0.41;
/// Slab travel speed (progress per second).
pub const SLAB_SPEED: f32 = 7.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OfficeVariant {
    Classic,
    Cargo,
    Hub,
}

impl OfficeVariant {
    /// The Godot office id (palette and signature key).
    pub fn office_id(self) -> &'static str {
        match self {
            OfficeVariant::Classic => "classic_security",
            OfficeVariant::Cargo => "cargo_bay",
            OfficeVariant::Hub => "hub_office",
        }
    }
}

#[derive(Clone)]
pub struct OfficeConfig {
    pub variant: OfficeVariant,
    pub extra: Vec<Group>,
    pub drop_hall_end_walls: bool,
    pub palette: bool,
    pub signature: bool,
}

impl Default for OfficeConfig {
    fn default() -> Self {
        OfficeConfig { variant: OfficeVariant::Classic, extra: Vec::new(), drop_hall_end_walls: false, palette: true, signature: true }
    }
}

#[derive(Clone, Debug)]
pub struct StationDef {
    pub id: String,
    pub prompt: String,
    pub pos: Vec3,
    pub focus: Vec3,
    pub radius: f32,
    pub disabled: bool,
}

// ---- cube helpers (shared with building.rs and palette.rs) -------------------------------------------------------------------

/// Cube options: material kind ("" = from the name), tint, hazard axes, ao, solid.
#[derive(Clone, Copy)]
pub struct O {
    pub kind: &'static str,
    pub tint: [f32; 3],
    pub hazard: &'static str,
    pub ao: f32,
    pub solid: bool,
    pub emissive: bool,
}

impl O {
    pub fn new() -> O {
        O { kind: "", tint: [1.0, 1.0, 1.0], hazard: "", ao: 0.72, solid: false, emissive: false }
    }
    pub fn k(mut self, kind: &'static str) -> O {
        self.kind = kind;
        self
    }
    pub fn t(mut self, tint: [f32; 3]) -> O {
        self.tint = tint;
        self
    }
    /// Grey tint.
    pub fn g(mut self, x: f32) -> O {
        self.tint = [x, x, x];
        self
    }
    pub fn hz(mut self, axes: &'static str) -> O {
        self.hazard = axes;
        self
    }
    pub fn ao(mut self, v: f32) -> O {
        self.ao = v;
        self
    }
    pub fn solid(mut self) -> O {
        self.solid = true;
        self
    }
    pub fn emissive(mut self) -> O {
        self.emissive = true;
        self
    }
}

pub fn v3(a: [f32; 3]) -> Vec3 {
    vec3(a[0], a[1], a[2])
}

pub fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

/// Add one cube to a group. Kind and emissive are resolved here from the name (the NightWatchArt rules), so the result does not
/// depend on how `Group::add` falls back.
pub fn put(g: &mut Group, name: &str, size: [f32; 3], pos: [f32; 3], o: O) {
    let kind: &str = if !o.hazard.is_empty() {
        "hazard"
    } else if o.kind.is_empty() {
        kind_for(name)
    } else {
        o.kind
    };
    let mut c = Cube::new(name, v3(size), v3(pos)).kind(kind).tint(o.tint).ao(o.ao);
    if !o.hazard.is_empty() {
        c = c.hazard(o.hazard);
    }
    if o.solid {
        c = c.solid();
    }
    if o.emissive || is_emissive_name(name) {
        c = c.emissive();
    }
    g.add(c);
}

/// The solid cubes of a set of groups as world-space boxes.
pub fn collect_solids(groups: &[Group]) -> Vec<Aabb> {
    let mut out = Vec::new();
    for g in groups {
        for c in &g.cubes {
            if c.solid {
                out.push(Aabb::from_centre(g.origin + c.pos, c.size));
            }
        }
    }
    out
}

fn st(id: &str, prompt: &str, pos: [f32; 3], focus: [f32; 3], radius: f32) -> StationDef {
    StationDef { id: id.to_string(), prompt: prompt.to_string(), pos: v3(pos), focus: v3(focus), radius, disabled: false }
}

fn door_index(door: &str) -> usize {
    if door == "doorL" {
        0
    } else {
        1
    }
}

fn move_toward(current: f32, target: f32, delta: f32) -> f32 {
    if (target - current).abs() <= delta {
        target
    } else {
        current + (target - current).signum() * delta
    }
}

// ---- the dynamic state (pure) ------------------------------------------------------------------------------------------------

/// Slab progress, lamp and door lights: everything the office changes at run time. Pure, so it can be tested without a GL context.
#[derive(Clone, Debug)]
pub struct OfficeState {
    pub slab_progress: [f32; 2],
    pub door_light: [bool; 2],
    pub lamp_on: bool,
}

impl OfficeState {
    pub fn new() -> OfficeState {
        OfficeState { slab_progress: [0.0, 0.0], door_light: [false, false], lamp_on: true }
    }

    /// Move the slabs toward their targets (1 = shut). Returns which slabs moved.
    pub fn animate(&mut self, door_shut: [bool; 2], dt_s: f32) -> [bool; 2] {
        let mut moved = [false, false];
        for i in 0..2 {
            let target = if door_shut[i] { 1.0 } else { 0.0 };
            let cur = self.slab_progress[i];
            if (cur - target).abs() > 1e-6 {
                self.slab_progress[i] = move_toward(cur, target, dt_s * SLAB_SPEED);
                moved[i] = true;
            }
        }
        moved
    }

    pub fn snap(&mut self, door_shut: [bool; 2]) {
        for i in 0..2 {
            self.slab_progress[i] = if door_shut[i] { 1.0 } else { 0.0 };
        }
    }

    pub fn slab_blocks(&self, door: &str) -> bool {
        self.slab_progress[door_index(door)] > SLAB_BLOCK_AT
    }

    /// The door light: a spot from the desk side toward the hall.
    pub fn door_spot(side: f32) -> Spot {
        let pos = vec3(side * 1.7, 1.55, 0.7);
        let target = vec3(side * (HALF_W + 4.2), 1.0, 0.0);
        Spot {
            pos,
            dir: (target - pos).normalize(),
            color: [1.0, 0.95, 0.82],
            energy: 5.0,
            range: 9.0,
            cos_outer: 34.0f32.to_radians().cos(),
        }
    }

    pub fn lights(&self, flashlight: Option<Spot>) -> LightSet {
        let mut omni = Vec::new();
        if self.lamp_on {
            omni.push(Omni { pos: vec3(0.9, 1.35, -1.6), color: [1.0, 0.82, 0.55], energy: 2.2, range: 5.0 });
        }
        for side in [-1.0f32, 1.0] {
            omni.push(Omni { pos: vec3(side * (HALF_W + 3.6), 2.4, 0.0), color: [0.45, 0.55, 0.9], energy: 0.14, range: 5.0 });
        }
        let mut spot = Vec::new();
        if self.door_light[0] {
            spot.push(OfficeState::door_spot(-1.0));
        }
        if self.door_light[1] {
            spot.push(OfficeState::door_spot(1.0));
        }
        if let Some(f) = flashlight {
            spot.push(f);
        }
        LightSet { ambient: [0.38, 0.40, 0.55], omni, spot, fog: None }
    }

    /// Slab progress to the model offset of the slab group (its origin is the open position).
    pub fn slab_offset(progress: f32) -> Vec3 {
        vec3(0.0, (SLAB_CLOSED_Y - SLAB_OPEN_Y) * progress.clamp(0.0, 1.0), 0.0)
    }
}

// ---- the parts (pure geometry) -----------------------------------------------------------------------------------------------

pub struct OfficeParts {
    pub groups: Vec<Group>,
    pub solids: Vec<Aabb>,
    pub stations: Vec<StationDef>,
}

pub struct Office {
    pub world: World,
    pub spawn: Vec3,
    pub stations: Vec<StationDef>,
    pub state: OfficeState,
    pub variant: OfficeVariant,
}

impl Office {
    /// Pure geometry: groups (with palette and signature passes applied), solids and stations. No GL.
    pub fn parts(cfg: &OfficeConfig) -> OfficeParts {
        let mut groups: Vec<Group> = Vec::new();
        let mut stations: Vec<StationDef> = Vec::new();
        let mut extra_solids: Vec<Aabb> = Vec::new();
        groups.push(shell());
        for side in [-1.0f32, 1.0] {
            door_side(side, cfg.drop_hall_end_walls, &mut groups, &mut stations, &mut extra_solids);
        }
        desk(&mut groups, &mut stations, &mut extra_solids);
        chair(&mut groups, &mut stations);
        wall_props(&mut groups, &mut stations, &mut extra_solids);
        extras(&mut groups);
        for g in &cfg.extra {
            groups.push(g.clone());
        }
        let office_id = cfg.variant.office_id();
        if cfg.palette {
            palette::apply(&mut groups, office_id);
        }
        if cfg.signature {
            palette::signature(&mut groups, office_id);
        }
        let mut solids = collect_solids(&groups);
        solids.extend(extra_solids);
        OfficeParts { groups, solids, stations }
    }

    pub fn build(cfg: OfficeConfig, tex: &Textures) -> Office {
        let parts = Office::parts(&cfg);
        let mut wb = WorldBuilder::new();
        wb.groups = parts.groups;
        wb.solids = parts.solids;
        wb.markers.insert("spawn".to_string(), SPAWN);
        wb.markers.insert("seat".to_string(), SEAT);
        let world = wb.bake(tex);
        let mut o = Office { world, spawn: SPAWN, stations: parts.stations, state: OfficeState::new(), variant: cfg.variant };
        o.snap_slabs([false, false]);
        o.set_button("doorL_door", false);
        o.set_button("doorR_door", false);
        o.set_lamp(true);
        o
    }

    /// Where a hunter at a door stands, from how far into her wait she is (0 = just arrived, 1 = about to attack).
    pub fn door_stand(door: &str, slot: usize, progress: f32) -> Vec3 {
        let side = if door == "doorL" { -1.0 } else { 1.0 };
        let p = progress.clamp(0.0, 1.0);
        let near = (HALF_W + 1.9) + ((HALF_W + 0.55) - (HALF_W + 1.9)) * p;
        let s = slot as f32;
        vec3(side * (near + s * 1.3), 0.0, -0.4 * s - side * 0.3)
    }

    pub fn animate(&mut self, door_shut: [bool; 2], dt_s: f32) {
        let moved = self.state.animate(door_shut, dt_s);
        for i in 0..2 {
            if moved[i] {
                self.apply_slab(i);
            }
        }
    }

    pub fn snap_slabs(&mut self, door_shut: [bool; 2]) {
        self.state.snap(door_shut);
        self.apply_slab(0);
        self.apply_slab(1);
    }

    fn apply_slab(&mut self, i: usize) {
        let name = if i == 0 { "SlabL" } else { "SlabR" };
        self.world.set_offset(name, OfficeState::slab_offset(self.state.slab_progress[i]));
    }

    pub fn set_light(&mut self, door: &str, on: bool) {
        self.state.door_light[door_index(door)] = on;
    }

    /// A panel button's look: the DOOR button is red while the door is shut, green when open; the LIGHT button glows while held.
    /// `id` is a station id such as "doorL_door" or "doorR_light".
    pub fn set_button(&mut self, id: &str, on: bool) {
        let tag = if id.starts_with("doorL") { "L" } else { "R" };
        let group = format!("ControlPanel{tag}");
        if id.ends_with("_door") {
            let cube = format!("Button{tag}DOOR");
            self.world.set_material(&group, &cube, if on { "red" } else { "green" }, [1.0, 1.0, 1.0], true);
        } else {
            let cube = format!("Button{tag}LIGHT");
            self.world.set_material(&group, &cube, if on { "light" } else { "plastic" }, [1.0, 1.0, 1.0], on);
        }
    }

    pub fn set_lamp(&mut self, on: bool) {
        self.state.lamp_on = on;
        let tint = if on { [1.0, 1.0, 1.0] } else { [0.35, 0.3, 0.25] };
        self.world.set_material("DeskLampProp", "LampShade", "light", tint, on);
    }

    pub fn lights(&self, flashlight: Option<Spot>) -> LightSet {
        self.state.lights(flashlight)
    }

    pub fn draw(&self, gfx: &Gfx, cam_pos: Vec3, flashlight: Option<Spot>) {
        let l = self.lights(flashlight);
        self.world.draw(&gfx.shader, &l, cam_pos);
    }

    pub fn slab_blocks(&self, door: &str) -> bool {
        self.state.slab_blocks(door)
    }

    pub fn station_mut(&mut self, id: &str) -> Option<&mut StationDef> {
        self.stations.iter_mut().find(|s| s.id == id)
    }

    pub fn station(&self, id: &str) -> Option<&StationDef> {
        self.stations.iter().find(|s| s.id == id)
    }

    /// Extra pub item: a station added by a mode.
    pub fn add_station(&mut self, s: StationDef) {
        self.stations.push(s);
    }

    /// Extra pub item: the static solids plus a box for every slab that is shut enough to block (the slab rides along in Godot).
    pub fn all_solids(&self) -> Vec<Aabb> {
        let mut out = self.world.solids.clone();
        for (i, side) in [-1.0f32, 1.0].iter().enumerate() {
            if self.state.slab_progress[i] > SLAB_BLOCK_AT {
                let y = SLAB_OPEN_Y + (SLAB_CLOSED_Y - SLAB_OPEN_Y) * self.state.slab_progress[i];
                out.push(Aabb::from_centre(vec3(side * HALF_W, y, 0.0), vec3(0.14, DOOR_H, DOOR_W - 0.04)));
            }
        }
        out
    }

    /// Extra pub item: the world position of a door's panel station (for hunters or tests).
    pub fn station_pos(&self, id: &str) -> Option<Vec3> {
        self.station(id).map(|s| s.pos)
    }
}

// ---- the shell ---------------------------------------------------------------------------------------------------------------

fn shell() -> Group {
    let mut g = Group::new("Shell", Vec3::ZERO);
    let w = HALF_W * 2.0;
    put(&mut g, "Floor", [w + 0.4, 0.2, HALF_D * 2.0 + 0.4], [0.0, -0.1, 0.0], O::new().g(1.1));
    put(&mut g, "Ceiling", [w + 0.4, 0.2, HALF_D * 2.0 + 0.4], [0.0, HEIGHT + 0.1, 0.0], O::new().t([0.7, 0.7, 0.75]));
    put(&mut g, "WallBack", [w + 0.4, HEIGHT, 0.2], [0.0, HEIGHT * 0.5, -HALF_D - 0.1], O::new().solid());
    put(&mut g, "WallFront", [w + 0.4, HEIGHT, 0.2], [0.0, HEIGHT * 0.5, HALF_D + 0.1], O::new().solid().t([0.85, 0.85, 0.9]));
    for z in [-HALF_D + 0.04, HALF_D - 0.04] {
        put(&mut g, "Baseboard", [w, 0.22, 0.08], [0.0, 0.11, z], O::new().t([0.7, 0.7, 0.75]));
        put(&mut g, "CrownTrim", [w, 0.12, 0.08], [0.0, HEIGHT - 0.06, z], O::new().t([0.8, 0.8, 0.85]));
        for x in [-2.0f32, -0.7, 0.7, 2.0] {
            put(&mut g, "PanelLine", [0.06, HEIGHT - 0.5, 0.05], [x, HEIGHT * 0.5, z], O::new().t([0.75, 0.75, 0.8]));
        }
    }
    put(&mut g, "PipeBack", [w, 0.12, 0.12], [0.0, HEIGHT - 0.2, -HALF_D + 0.2], O::new().k("metal"));
    put(&mut g, "ConduitBack", [w, 0.07, 0.07], [0.0, HEIGHT - 0.35, -HALF_D + 0.18], O::new().k("plastic"));
    for x in [-2.5f32, -1.2, 0.0, 1.2, 2.5] {
        put(&mut g, "PipeClamp", [0.16, 0.2, 0.16], [x, HEIGHT - 0.2, -HALF_D + 0.2], O::new().k("metal").t([0.8, 0.8, 0.85]));
    }
    for x in [-1.4f32, 1.4] {
        put(&mut g, "CeilingFixture", [1.1, 0.08, 0.5], [x, HEIGHT - 0.04, 0.2], O::new().k("plastic"));
        put(&mut g, "CeilingLens", [0.9, 0.02, 0.3], [x, HEIGHT - 0.1, 0.2], O::new().k("glass").t([0.45, 0.5, 0.5]));
    }
    put(&mut g, "LedAlarm", [0.05, 0.05, 0.05], [2.7, 0.9, HALF_D - 0.1], O::new());
    put(&mut g, "FloorStripe", [2.8, 0.01, 0.12], [0.0, 0.005, -0.55], O::new().hz("y").g(0.8));
    put(&mut g, "FloorMat", [1.6, 0.02, 1.0], [0.0, 0.01, -1.1], O::new().t([0.9, 0.9, 0.95]));
    g
}

// ---- one door: frame, slab, hall, panel --------------------------------------------------------------------------------------

fn door_side(side: f32, drop_end: bool, groups: &mut Vec<Group>, stations: &mut Vec<StationDef>, solids: &mut Vec<Aabb>) {
    let tag = if side < 0.0 { "L" } else { "R" };
    let door = if side < 0.0 { "doorL" } else { "doorR" };
    let side_name = if side < 0.0 { "LEFT" } else { "RIGHT" };
    let x = side * (HALF_W + 0.1);

    // the wall with the doorway
    let mut wall = Group::new(&format!("Wall{tag}"), Vec3::ZERO);
    let piece = (HALF_D * 2.0 - DOOR_W) * 0.5;
    put(&mut wall, &format!("Wall{tag}A"), [0.2, HEIGHT, piece], [x, HEIGHT * 0.5, -HALF_D + piece * 0.5], O::new().solid());
    put(&mut wall, &format!("Wall{tag}B"), [0.2, HEIGHT, piece], [x, HEIGHT * 0.5, HALF_D - piece * 0.5], O::new().solid());
    put(&mut wall, &format!("Wall{tag}Top"), [0.2, HEIGHT - DOOR_H, DOOR_W], [x, DOOR_H + (HEIGHT - DOOR_H) * 0.5, 0.0], O::new());
    let ix = side * (HALF_W - 0.04);
    for zz in [-HALF_D + piece * 0.5, HALF_D - piece * 0.5] {
        put(&mut wall, "Baseboard", [0.08, 0.22, piece], [ix, 0.11, zz], O::new().t([0.7, 0.7, 0.75]));
    }
    put(&mut wall, "CrownTrim", [0.08, 0.12, HALF_D * 2.0], [ix, HEIGHT - 0.06, 0.0], O::new().t([0.8, 0.8, 0.85]));
    groups.push(wall);

    // the frame
    let mut frame = Group::new(&format!("DoorFrame{tag}"), Vec3::ZERO);
    for zz in [-DOOR_W * 0.5 - 0.07, DOOR_W * 0.5 + 0.07] {
        put(&mut frame, "FramePost", [0.34, DOOR_H, 0.14], [x, DOOR_H * 0.5, zz], O::new().t([0.75, 0.7, 0.65]));
        put(&mut frame, "Seal", [0.1, DOOR_H - 0.1, 0.05], [side * (HALF_W - 0.1), DOOR_H * 0.5, zz * 0.92], O::new().k("plastic"));
    }
    put(&mut frame, "FrameHeader", [0.34, 0.16, DOOR_W + 0.28], [x, DOOR_H + 0.06, 0.0], O::new().t([0.75, 0.7, 0.65]));
    put(&mut frame, "HazardStripe", [0.2, 0.09, DOOR_W], [side * (HALF_W - 0.02), DOOR_H - 0.06, 0.0], O::new().hz("x").g(0.85));
    put(&mut frame, "Threshold", [0.4, 0.05, DOOR_W], [x, 0.025, 0.0], O::new().k("metal").t([0.85, 0.85, 0.9]));
    groups.push(frame);

    // the slab (group origin is its OPEN position; World::set_offset slides it down)
    let mut slab = Group::new(&format!("Slab{tag}"), vec3(side * HALF_W, SLAB_OPEN_Y, 0.0));
    put(&mut slab, "SlabBody", [0.14, DOOR_H, DOOR_W - 0.04], [0.0, 0.0, 0.0], O::new());
    for i in 0..4 {
        let by = -DOOR_H * 0.5 + 0.3 + i as f32 * 0.6;
        if i == 0 || i == 3 {
            put(&mut slab, "SlabStripe", [0.2, 0.12, DOOR_W - 0.06], [0.0, by, 0.0], O::new().hz("x").g(1.0));
        } else {
            put(&mut slab, "SlabRibBand", [0.18, 0.1, DOOR_W - 0.06], [0.0, by, 0.0], O::new().k("metal").t([0.85, 0.85, 0.9]));
            put(&mut slab, "SlabRibStep", [0.2, 0.04, DOOR_W - 0.14], [0.0, by, 0.0], O::new().k("metal").t([1.05, 1.05, 1.1]));
        }
    }
    for zz in [-0.7f32, 0.7] {
        for yy in [-0.95f32, 0.0, 0.95] {
            put(&mut slab, "SlabBolt", [0.18, 0.07, 0.07], [0.0, yy, zz], O::new().k("metal").g(1.1));
        }
    }
    put(&mut slab, "SlabHandle", [0.2, 0.08, 0.5], [-side * 0.02, 0.05, 0.4], O::new().k("metal").g(1.2));
    put(&mut slab, "SlabRib", [0.16, 0.5, 0.05], [0.0, 0.0, -0.45], O::new().k("metal").t([0.85, 0.85, 0.9]));
    groups.push(slab);

    // the hall behind the doorway
    let mut hall = Group::new(&format!("Hall{tag}"), Vec3::ZERO);
    let hx = side * (HALF_W + 3.25);
    put(&mut hall, &format!("Hall{tag}Floor"), [6.3, 0.2, 3.2], [hx, -0.1, 0.0], O::new().t([0.85, 0.85, 0.9]));
    put(&mut hall, &format!("Hall{tag}Ceiling"), [6.3, 0.2, 3.2], [hx, HEIGHT + 0.1, 0.0], O::new().t([0.6, 0.6, 0.65]));
    put(&mut hall, &format!("Hall{tag}WallN"), [6.3, HEIGHT, 0.2], [hx, HEIGHT * 0.5, -1.7], O::new().solid().t([0.8, 0.8, 0.85]));
    put(&mut hall, &format!("Hall{tag}WallS"), [6.3, HEIGHT, 0.2], [hx, HEIGHT * 0.5, 1.7], O::new().solid().t([0.8, 0.8, 0.85]));
    if !drop_end {
        put(&mut hall, &format!("Hall{tag}End"), [0.2, HEIGHT, 3.2], [side * (HALF_W + 6.4), HEIGHT * 0.5, 0.0], O::new().solid().t([0.7, 0.7, 0.75]));
    }
    for zz in [-1.55f32, 1.55] {
        put(&mut hall, "Baseboard", [6.2, 0.2, 0.08], [hx, 0.1, zz], O::new().t([0.7, 0.7, 0.75]));
        put(&mut hall, "PipeHall", [6.2, 0.1, 0.12], [hx, HEIGHT - 0.2, zz], O::new().k("metal"));
    }
    put(&mut hall, "HallStripe", [6.2, 0.01, 0.2], [hx, 0.005, 0.0], O::new().hz("y").g(0.7));
    if !drop_end {
        put(&mut hall, "HallEndDoor", [0.08, 1.9, 1.0], [side * (HALF_W + 6.28), 1.0, 0.0], O::new().k("metal").t([0.8, 0.8, 0.85]));
    }
    put(&mut hall, "SignExit", [0.05, 0.2, 0.7], [side * (HALF_W + 6.28), 2.2, 0.0], O::new().t([0.5, 1.0, 0.6]));
    groups.push(hall);
    // the guard the player cannot pass
    solids.push(Aabb::from_centre(vec3(side * HALL_GUARD_X, HEIGHT * 0.5, 0.0), vec3(0.3, HEIGHT, 3.2)));

    // the control panel: housing, screws, conduit, a name plate, a DOOR button above a LIGHT button
    let mut panel = Group::new(&format!("ControlPanel{tag}"), Vec3::ZERO);
    let px = side * (HALF_W - 0.06);
    put(&mut panel, "PanelHousing", [0.1, 0.78, 0.46], [px, 1.3, 1.55], O::new());
    put(&mut panel, "PanelFace", [0.05, 0.7, 0.4], [side * (HALF_W - 0.12), 1.3, 1.55], O::new().k("plastic"));
    for zz in [1.37f32, 1.73] {
        for yy in [0.96f32, 1.64] {
            put(&mut panel, "PanelBolt", [0.07, 0.05, 0.05], [side * (HALF_W - 0.13), yy, zz], O::new().k("metal").g(1.2));
        }
    }
    put(&mut panel, "ConduitPanel", [0.07, 0.07, 0.07], [px, 1.86, 1.55], O::new().k("plastic"));
    put(&mut panel, "ConduitRise", [0.07, 0.9, 0.07], [px, 2.35, 1.55], O::new().k("plastic"));
    put(&mut panel, "LabelPlate", [0.06, 0.1, 0.28], [side * (HALF_W - 0.14), 1.74, 1.55], O::new());
    for (what, y, label) in [("door", 1.47f32, "DOOR"), ("light", 1.15f32, "LIGHT")] {
        let bx = side * (HALF_W - 0.15);
        put(&mut panel, &format!("Guard{label}"), [0.05, 0.26, 0.3], [side * (HALF_W - 0.14), y, 1.55], O::new().k("metal").t([0.9, 0.9, 0.95]));
        // the DOOR button is built as set_button(.., false) leaves it: green
        let kind = if what == "door" { "green" } else { "light" };
        put(&mut panel, &format!("Button{tag}{label}"), [0.06, 0.18, 0.22], [bx, y, 1.55], O::new().k(kind).emissive());
        let text = if what == "door" { format!("{side_name} DOOR // SHUT / OPEN") } else { format!("{side_name} DOOR LIGHT // HOLD") };
        stations.push(st(&format!("{door}_{what}"), &text, [bx, y, 1.55], [0.0, 0.0, 0.0], 0.2));
    }
    groups.push(panel);
}

// ---- the desk, monitor, radio, lamp ------------------------------------------------------------------------------------------

fn desk(groups: &mut Vec<Group>, stations: &mut Vec<StationDef>, solids: &mut Vec<Aabb>) {
    let mut d = Group::new("Desk", vec3(0.0, 0.0, -1.95));
    put(&mut d, "DeskTop", [2.4, 0.08, 0.95], [0.0, 0.76, 0.0], O::new());
    put(&mut d, "DeskEdge", [2.44, 0.04, 0.99], [0.0, 0.8, 0.0], O::new().g(1.15));
    for x in [-1.1f32, 1.1] {
        put(&mut d, "DeskLeg", [0.1, 0.76, 0.85], [x, 0.38, 0.0], O::new());
    }
    put(&mut d, "DeskBackPanel", [2.1, 0.5, 0.04], [0.0, 0.45, -0.4], O::new().g(0.8));
    for x in [-0.8f32, -0.3] {
        put(&mut d, "DeskDrawer", [0.42, 0.22, 0.04], [x, 0.55, 0.43], O::new().g(1.05));
        put(&mut d, "DeskHandle", [0.16, 0.03, 0.04], [x, 0.57, 0.47], O::new().k("metal"));
    }
    groups.push(d);
    solids.push(Aabb::from_centre(vec3(0.0, 0.45, -1.95), vec3(2.4, 0.9, 0.95)));

    let mut m = Group::new("Monitor", vec3(0.0, 0.0, -2.2));
    put(&mut m, "MonitorCase", [0.96, 0.64, 0.09], [0.0, 1.12, 0.0], O::new());
    put(&mut m, "MonitorBezel", [0.88, 0.56, 0.02], [0.0, 1.12, 0.05], O::new().t([0.7, 0.7, 0.75]));
    put(&mut m, "MonitorScreen", [0.8, 0.48, 0.02], [0.0, 1.12, 0.06], O::new());
    put(&mut m, "MonitorNeck", [0.14, 0.18, 0.12], [0.0, 0.9, -0.02], O::new());
    put(&mut m, "MonitorBase", [0.4, 0.04, 0.26], [0.0, 0.82, 0.0], O::new());
    put(&mut m, "LedPower", [0.04, 0.03, 0.02], [0.4, 0.83 + 0.2, 0.07], O::new());
    put(&mut m, "CableMonitor", [0.05, 0.4, 0.05], [0.3, 0.6, -0.1], O::new().k("plastic").t([0.6, 0.6, 0.65]));
    groups.push(m);

    let mut kb = Group::new("Keyboard", vec3(0.0, 0.8, -1.6));
    put(&mut kb, "KeyboardBase", [0.52, 0.03, 0.18], [0.0, 0.0, 0.0], O::new());
    for i in 0..3 {
        put(&mut kb, "KeyboardKeys", [0.46, 0.015, 0.035], [0.0, 0.02, -0.05 + i as f32 * 0.05], O::new().k("metal").t([0.9, 0.9, 0.95]));
    }
    put(&mut kb, "KeyboardMouse", [0.07, 0.03, 0.1], [0.45, 0.0, 0.0], O::new());
    groups.push(kb);

    let mut papers = Group::new("Papers", vec3(-0.75, 0.8, -1.75));
    for i in 0..3 {
        let f = i as f32;
        put(&mut papers, "PaperSheet", [0.28, 0.012, 0.36], [0.01 * f, 0.007 + f * 0.012, 0.01 * f], O::new().k("paper").g(1.2).ao(1.0));
    }
    put(&mut papers, "ClipboardBoard", [0.3, 0.015, 0.4], [0.45, 0.01, 0.0], O::new().k("wood"));
    put(&mut papers, "ClipboardClip", [0.1, 0.02, 0.05], [0.45, 0.025, -0.18], O::new().k("metal"));
    groups.push(papers);

    let mut cup = Group::new("Cup", vec3(-1.0, 0.8, -1.5));
    put(&mut cup, "CupBody", [0.14, 0.14, 0.14], [0.0, 0.07, 0.0], O::new().t([1.3, 1.3, 1.2]));
    put(&mut cup, "CupHandle", [0.05, 0.08, 0.04], [0.09, 0.07, 0.0], O::new());
    put(&mut cup, "CupCoffee", [0.1, 0.02, 0.1], [0.0, 0.145, 0.0], O::new().k("wood").t([0.5, 0.4, 0.35]));
    groups.push(cup);

    let mut radio = Group::new("Radio", vec3(-0.35, 0.8, -2.1));
    put(&mut radio, "RadioCase", [0.3, 0.16, 0.14], [0.0, 0.08, 0.0], O::new().g(1.1));
    put(&mut radio, "RadioGrille", [0.1, 0.1, 0.02], [-0.07, 0.08, 0.075], O::new().k("metal").t([0.7, 0.7, 0.75]));
    put(&mut radio, "RadioDial", [0.05, 0.05, 0.03], [0.06, 0.1, 0.075], O::new().k("metal").g(1.2));
    put(&mut radio, "RadioAntenna", [0.015, 0.2, 0.015], [0.11, 0.2, -0.03], O::new().k("metal"));
    put(&mut radio, "LedRadio", [0.025, 0.02, 0.02], [0.1, 0.14, 0.075], O::new());
    groups.push(radio);

    let mut lamp = Group::new("DeskLampProp", vec3(0.9, 0.8, -1.7));
    put(&mut lamp, "LampBase", [0.18, 0.04, 0.18], [0.0, 0.02, 0.0], O::new());
    put(&mut lamp, "LampArm", [0.04, 0.22, 0.04], [0.0, 0.15, 0.0], O::new());
    put(&mut lamp, "LampArmTop", [0.18, 0.04, 0.04], [0.06, 0.27, 0.0], O::new());
    // the shade is lit (kind light, emissive) while the lamp is on
    put(&mut lamp, "LampShade", [0.22, 0.14, 0.22], [0.1, 0.4 - 0.12, 0.0], O::new().k("light").emissive());
    groups.push(lamp);

    stations.push(st("monitor", "MONITOR // CAMERAS", [0.0, 1.12, -2.0], [0.0, 0.0, 0.0], 0.5));
    stations.push(st("lamp", "DESK LAMP", [0.9, 1.0, -1.7], [0.0, 0.0, 0.0], 0.35));
}

fn chair(groups: &mut Vec<Group>, stations: &mut Vec<StationDef>) {
    let mut c = Group::new("Chair", vec3(0.0, 0.0, -1.1));
    put(&mut c, "ChairSeat", [0.46, 0.1, 0.46], [0.0, 0.5, 0.0], O::new().solid());
    put(&mut c, "ChairBack", [0.44, 0.5, 0.08], [0.0, 0.8, 0.2], O::new());
    put(&mut c, "ChairArmL", [0.1, 0.06, 0.3], [-0.28, 0.66, 0.0], O::new().k("plastic"));
    put(&mut c, "ChairArmR", [0.1, 0.06, 0.3], [0.28, 0.66, 0.0], O::new().k("plastic"));
    put(&mut c, "ChairColumn", [0.08, 0.3, 0.08], [0.0, 0.3, 0.0], O::new().k("metal"));
    put(&mut c, "ChairBaseA", [0.5, 0.05, 0.08], [0.0, 0.1, 0.0], O::new().k("metal"));
    put(&mut c, "ChairBaseB", [0.08, 0.05, 0.5], [0.0, 0.1, 0.0], O::new().k("metal"));
    for x in [-0.22f32, 0.22] {
        for z in [-0.22f32, 0.22] {
            put(&mut c, "ChairWheel", [0.07, 0.07, 0.07], [x, 0.035, z], O::new().k("plastic").g(0.7));
        }
    }
    groups.push(c);
    stations.push(st("chair", "SIT AT DESK", [0.0, 0.0, -1.05], [0.0, 0.62, 0.0], 0.5));
}

// ---- breaker box, vent hatch, shelf, clock, poster, bin ----------------------------------------------------------------------

fn wall_props(groups: &mut Vec<Group>, stations: &mut Vec<StationDef>, solids: &mut Vec<Aabb>) {
    let mut b = Group::new("BreakerBox", vec3(2.3, 1.5, -HALF_D));
    put(&mut b, "BreakerHousing", [0.64, 0.84, 0.14], [0.0, 0.0, 0.07], O::new().solid());
    put(&mut b, "BreakerDoor", [0.58, 0.78, 0.04], [0.0, 0.0, 0.16], O::new().g(1.1));
    for i in 0..2 {
        put(&mut b, "BreakerHinge", [0.05, 0.12, 0.05], [-0.31, -0.25 + i as f32 * 0.5, 0.15], O::new().k("metal").g(1.2));
    }
    for i in 0..4 {
        let x = -0.17 + i as f32 * 0.11;
        put(&mut b, "BreakerSwitch", [0.07, 0.12, 0.05], [x, 0.0, 0.2], O::new().k("plastic").g(1.7));
        put(&mut b, "LedBreaker", [0.03, 0.03, 0.03], [x, 0.12, 0.2], O::new());
    }
    put(&mut b, "LabelBreaker", [0.46, 0.1, 0.04], [0.0, 0.3, 0.2], O::new());
    put(&mut b, "ConduitBreaker", [0.07, 0.9, 0.07], [0.22, 0.85, 0.1], O::new().k("plastic"));
    groups.push(b);
    stations.push(st("breaker", "BREAKER BOX // NOT USED TONIGHT", [2.3, 1.5, -HALF_D + 0.2], [0.0, 0.0, 0.0], 0.45));

    let mut v = Group::new("VentHatch", vec3(-2.3, 0.45, -HALF_D));
    put(&mut v, "VentFrame", [0.84, 0.54, 0.1], [0.0, 0.0, 0.05], O::new().solid());
    for i in 0..5 {
        put(&mut v, "VentLouver", [0.7, 0.035, 0.06], [0.0, -0.2 + i as f32 * 0.1, 0.12], O::new().k("metal").t([0.7, 0.7, 0.75]));
    }
    for sx in [-0.38f32, 0.38] {
        for sy in [-0.24f32, 0.24] {
            put(&mut v, "VentScrew", [0.05, 0.05, 0.05], [sx, sy, 0.13], O::new().k("metal").g(1.3));
        }
    }
    groups.push(v);
    stations.push(st("vent", "VENT HATCH // SEALED", [-2.3, 0.45, -HALF_D + 0.2], [0.0, 0.0, 0.0], 0.5));

    let mut s = Group::new("ShelfUnit", vec3(-2.7, 0.0, -1.0));
    for x in [-0.18f32, 0.18] {
        for z in [-0.6f32, 0.6] {
            put(&mut s, "ShelfPost", [0.05, 1.8, 0.05], [x, 0.9, z], O::new().k("metal"));
        }
    }
    for i in 0..4 {
        put(&mut s, "ShelfBoard", [0.42, 0.04, 1.3], [0.0, 0.25 + i as f32 * 0.5, 0.0], O::new());
    }
    solids.push(Aabb::from_centre(vec3(-2.7, 0.9, -1.0), vec3(0.42, 1.8, 1.3)));
    for i in 0..3 {
        let f = i as f32;
        put(&mut s, "ShelfBox", [0.32, 0.26, 0.4], [0.0, 0.41 + f * 0.5, -0.3 + (i % 2) as f32 * 0.5], O::new().k("wood").t([0.85 + 0.1 * f, 0.85, 0.8]));
    }
    put(&mut s, "ShelfCrate", [0.3, 0.18, 0.3], [0.0, 0.34, 0.45], O::new().k("wood").t([0.7, 0.7, 0.75]));
    groups.push(s);

    let mut bin = Group::new("Bin", vec3(1.55, 0.0, -1.55));
    put(&mut bin, "BinBody", [0.3, 0.38, 0.3], [0.0, 0.19, 0.0], O::new().solid().t([0.9, 0.9, 1.0]));
    put(&mut bin, "BinRim", [0.34, 0.04, 0.34], [0.0, 0.4, 0.0], O::new().g(1.2));
    groups.push(bin);

    let mut clock = Group::new("WallClock", vec3(1.4, 2.2, -HALF_D));
    put(&mut clock, "ClockFrame", [0.58, 0.58, 0.05], [0.0, 0.0, 0.03], O::new().k("plastic"));
    put(&mut clock, "ClockFace", [0.5, 0.5, 0.03], [0.0, 0.0, 0.06], O::new());
    put(&mut clock, "ClockHandA", [0.03, 0.18, 0.02], [0.0, 0.07, 0.09], O::new().k("plastic"));
    put(&mut clock, "ClockHandB", [0.14, 0.03, 0.02], [0.05, 0.0, 0.09], O::new().k("plastic"));
    groups.push(clock);

    let mut poster = Group::new("Poster", vec3(-1.4, 1.8, -HALF_D));
    put(&mut poster, "PosterFrame", [1.0, 1.3, 0.04], [0.0, 0.0, 0.02], O::new());
    put(&mut poster, "PosterPaper", [0.88, 1.18, 0.03], [0.0, 0.0, 0.05], O::new().k("fabric").t([1.1, 0.9, 1.0]));
    put(&mut poster, "PosterTitle", [0.6, 0.14, 0.02], [0.0, 0.38, 0.07], O::new().k("label").g(0.9));
    groups.push(poster);
}

// ---- extra detail ------------------------------------------------------------------------------------------------------------

fn extras(groups: &mut Vec<Group>) {
    let mut bolts = Group::new("WallBolts", Vec3::ZERO);
    for x in [-2.6f32, -1.5, -0.4, 0.7, 1.8, 2.6] {
        put(&mut bolts, "WallBolt", [0.06, 0.06, 0.04], [x, 0.32, -HALF_D + 0.03], O::new().k("metal").g(1.2));
        put(&mut bolts, "WallBolt", [0.06, 0.06, 0.04], [x, 0.32, HALF_D - 0.03], O::new().k("metal").g(1.2));
    }
    for z in [-2.0f32, -0.9, 0.0, 1.0, 2.0] {
        put(&mut bolts, "WallBolt", [0.04, 0.06, 0.06], [-HALF_W + 0.03, 0.32, z], O::new().k("metal").g(1.2));
        put(&mut bolts, "WallBolt", [0.04, 0.06, 0.06], [HALF_W - 0.03, 0.32, z], O::new().k("metal").g(1.2));
    }
    for x in [-0.2f32, 2.0] {
        put(&mut bolts, "OutletPlate", [0.12, 0.18, 0.03], [x, 0.5, -HALF_D + 0.04], O::new().k("plastic").g(1.5));
        put(&mut bolts, "OutletSocket", [0.05, 0.06, 0.02], [x, 0.5, -HALF_D + 0.065], O::new().k("plastic").g(0.5));
    }
    groups.push(bolts);

    let mut cables = Group::new("FloorCables", Vec3::ZERO);
    put(&mut cables, "CableRun", [0.05, 0.03, 1.1], [0.3, 0.02, -2.0], O::new().k("plastic"));
    put(&mut cables, "CableRun", [1.0, 0.03, 0.05], [0.8, 0.02, -2.4], O::new().k("plastic"));
    put(&mut cables, "CableRun", [0.05, 0.03, 0.9], [1.3, 0.02, -1.95], O::new().k("plastic").t([0.8, 0.8, 0.9]));
    put(&mut cables, "CableClamp", [0.14, 0.05, 0.14], [1.3, 0.025, -1.5], O::new().k("metal"));
    groups.push(cables);

    let mut ext = Group::new("Extinguisher", vec3(2.8, 0.0, -0.9));
    put(&mut ext, "ExtBracket", [0.05, 0.9, 0.3], [0.1, 0.6, 0.0], O::new().k("metal"));
    put(&mut ext, "ExtBody", [0.22, 0.6, 0.22], [0.0, 0.55, 0.0], O::new().k("red"));
    put(&mut ext, "ExtHead", [0.1, 0.1, 0.1], [0.0, 0.9, 0.0], O::new().k("metal"));
    put(&mut ext, "ExtNozzle", [0.05, 0.05, 0.25], [-0.1, 0.92, 0.0], O::new().k("plastic"));
    put(&mut ext, "ExtBand", [0.24, 0.1, 0.24], [0.0, 0.6, 0.0], O::new().hz("xz").g(0.9));
    groups.push(ext);

    let mut board = Group::new("NoticeBoard", vec3(1.3, 1.7, HALF_D));
    put(&mut board, "BoardCork", [1.2, 0.8, 0.05], [0.0, 0.0, -0.03], O::new().k("wood").t([1.1, 0.9, 0.7]));
    for i in 0..4 {
        put(&mut board, "BoardNote", [0.22, 0.28, 0.01], [-0.4 + i as f32 * 0.27, 0.08 - (i % 2) as f32 * 0.12, -0.07], O::new().k("label").t([1.2, 1.2, 1.1]).ao(1.0));
    }
    put(&mut board, "BoardTrim", [1.28, 0.05, 0.07], [0.0, 0.42, -0.03], O::new().k("metal"));
    groups.push(board);

    let mut grille = Group::new("CeilingGrille", vec3(2.0, HEIGHT - 0.02, 1.4));
    put(&mut grille, "GrilleFrame", [0.7, 0.04, 0.7], [0.0, 0.0, 0.0], O::new().k("metal"));
    for i in 0..4 {
        put(&mut grille, "GrilleSlat", [0.6, 0.02, 0.05], [0.0, -0.03, -0.24 + i as f32 * 0.16], O::new().k("plastic"));
    }
    groups.push(grille);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> OfficeParts {
        Office::parts(&OfficeConfig::default())
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    fn group<'a>(p: &'a OfficeParts, name: &str) -> &'a Group {
        p.groups.iter().find(|g| g.name == name).unwrap_or_else(|| panic!("no group {name}"))
    }

    #[test]
    fn door_stand_matches_godot() {
        assert!(near(Office::door_stand("doorL", 0, 0.0), vec3(-4.9, 0.0, 0.3)));
        assert!(near(Office::door_stand("doorR", 1, 1.0), vec3(4.85, 0.0, -0.7)));
        // progress is clamped
        assert!(near(Office::door_stand("doorR", 0, 5.0), Office::door_stand("doorR", 0, 1.0)));
    }

    #[test]
    fn groups_named_for_set_material_and_set_offset() {
        let p = parts();
        for name in ["SlabL", "SlabR", "DeskLampProp", "ControlPanelL", "ControlPanelR", "Desk", "Shell", "HallL", "HallR"] {
            group(&p, name);
        }
        assert!(group(&p, "DeskLampProp").cubes.iter().any(|c| c.name == "LampShade" && c.emissive));
        for (g, n) in [("ControlPanelL", "ButtonLDOOR"), ("ControlPanelL", "ButtonLLIGHT"), ("ControlPanelR", "ButtonRDOOR"), ("ControlPanelR", "ButtonRLIGHT")] {
            assert!(group(&p, g).cubes.iter().any(|c| c.name == n), "{n}");
        }
        // slab groups start at the open height
        assert!((group(&p, "SlabL").origin.y - SLAB_OPEN_Y).abs() < 1e-5);
        assert!((group(&p, "SlabR").origin.x - 3.0).abs() < 1e-5);
    }

    #[test]
    fn stations_have_the_exact_prompts() {
        let p = parts();
        let find = |id: &str| p.stations.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("station {id}")).prompt.clone();
        assert_eq!(find("doorL_door"), "LEFT DOOR // SHUT / OPEN");
        assert_eq!(find("doorR_door"), "RIGHT DOOR // SHUT / OPEN");
        assert_eq!(find("doorL_light"), "LEFT DOOR LIGHT // HOLD");
        assert_eq!(find("doorR_light"), "RIGHT DOOR LIGHT // HOLD");
        assert_eq!(find("monitor"), "MONITOR // CAMERAS");
        assert_eq!(find("lamp"), "DESK LAMP");
        assert_eq!(find("chair"), "SIT AT DESK");
        assert_eq!(find("breaker"), "BREAKER BOX // NOT USED TONIGHT");
        assert_eq!(find("vent"), "VENT HATCH // SEALED");
        assert_eq!(p.stations.len(), 9);
    }

    #[test]
    fn extra_groups_are_merged_and_hall_ends_can_drop() {
        let mut cfg = OfficeConfig::default();
        cfg.extra.push(Group::new("Rack", vec3(1.0, 0.0, 1.0)));
        cfg.drop_hall_end_walls = true;
        let p = Office::parts(&cfg);
        assert!(p.groups.iter().any(|g| g.name == "Rack"));
        assert!(!group(&p, "HallL").cubes.iter().any(|c| c.name == "HallLEnd" || c.name == "HallEndDoor"));
        let full = parts();
        assert!(group(&full, "HallL").cubes.iter().any(|c| c.name == "HallLEnd"));
    }

    #[test]
    fn solids_include_the_guards_and_the_desk_but_no_floors() {
        let p = parts();
        // hall guard at x = 3.7 blocks the hall mouth
        assert!(p.solids.iter().any(|a| a.contains_xz(3.7, 0.0, 0.0)));
        assert!(p.solids.iter().any(|a| a.contains_xz(-3.7, 0.0, 0.0)));
        // the desk
        assert!(p.solids.iter().any(|a| a.contains_xz(0.0, -1.95, 0.0)));
        // the spawn is free
        assert!(!p.solids.iter().any(|a| a.contains_xz(SPAWN.x, SPAWN.z, 0.0)));
        // the room centre is free (no floor slab counted as a wall)
        assert!(!p.solids.iter().any(|a| a.contains_xz(0.0, 0.0, 0.0)));
    }

    #[test]
    fn slabs_move_at_seven_per_second_and_block_past_041() {
        let mut s = OfficeState::new();
        assert!(!s.slab_blocks("doorL"));
        let moved = s.animate([true, false], 0.05);
        assert_eq!(moved, [true, false]);
        assert!((s.slab_progress[0] - 0.35).abs() < 1e-5);
        assert!(!s.slab_blocks("doorL"));
        s.animate([true, false], 0.02);
        assert!(s.slab_blocks("doorL"));
        assert!(!s.slab_blocks("doorR"));
        for _ in 0..40 {
            s.animate([true, false], 0.05);
        }
        assert_eq!(s.slab_progress[0], 1.0);
        s.animate([false, false], 1.0);
        assert_eq!(s.slab_progress[0], 0.0);
        s.snap([true, true]);
        assert_eq!(s.slab_progress, [1.0, 1.0]);
        // slab offset: open = 0, shut = closed - open
        assert!((OfficeState::slab_offset(1.0).y - (SLAB_CLOSED_Y - SLAB_OPEN_Y)).abs() < 1e-5);
        assert_eq!(OfficeState::slab_offset(0.0).y, 0.0);
    }

    #[test]
    fn lights_follow_lamp_door_lights_and_flashlight() {
        let mut s = OfficeState::new();
        let l = s.lights(None);
        assert_eq!(l.omni.len(), 3);
        assert!(l.spot.is_empty());
        assert_eq!(l.ambient, [0.38, 0.40, 0.55]);
        assert!((l.omni[0].energy - 2.2).abs() < 1e-6 && (l.omni[0].range - 5.0).abs() < 1e-6);
        s.door_light = [true, true];
        let fl = Spot { pos: Vec3::ZERO, dir: vec3(0.0, 0.0, -1.0), color: [1.0, 1.0, 1.0], energy: 3.0, range: 8.0, cos_outer: 0.9 };
        let l = s.lights(Some(fl));
        assert_eq!(l.spot.len(), 3);
        assert!((l.spot[0].energy - 5.0).abs() < 1e-6);
        assert!((l.spot[0].cos_outer - 34.0f32.to_radians().cos()).abs() < 1e-6);
        // the left door light points to the left
        assert!(l.spot[0].dir.x < 0.0 && l.spot[1].dir.x > 0.0);
        s.lamp_on = false;
        assert_eq!(s.lights(None).omni.len(), 2);
    }

    #[test]
    fn variants_have_ids_and_default_config_is_classic() {
        assert_eq!(OfficeVariant::Classic.office_id(), "classic_security");
        assert_eq!(OfficeVariant::Cargo.office_id(), "cargo_bay");
        assert_eq!(OfficeVariant::Hub.office_id(), "hub_office");
        let c = OfficeConfig::default();
        assert_eq!(c.variant, OfficeVariant::Classic);
        assert!(c.palette && c.signature && !c.drop_hall_end_walls && c.extra.is_empty());
    }

    #[test]
    fn every_cube_has_a_kind_and_finite_numbers() {
        for variant in [OfficeVariant::Classic, OfficeVariant::Cargo, OfficeVariant::Hub] {
            let p = Office::parts(&OfficeConfig { variant, ..OfficeConfig::default() });
            for g in &p.groups {
                for c in &g.cubes {
                    assert!(!c.kind.is_empty(), "{} has no kind", c.name);
                    assert!(c.size.x > 0.0 && c.size.y > 0.0 && c.size.z > 0.0, "{} has no size", c.name);
                    assert!(c.pos.is_finite() && c.size.is_finite());
                }
            }
        }
    }
}
