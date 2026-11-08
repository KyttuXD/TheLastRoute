#![allow(dead_code)]
//! Masked Hours' data (port of `mask_data.gd` and `mask_map.gd`): the cast, the wing's rooms and graph, where each hunter
//! waits, the tuning tables of `MaskLogic` (the original's T, LEVELS, ...) and the 3D map numbers (room centres, camera
//! mounts, stand spots, the hall). Everything lives in JSON under `data/mask/` and `data/nav/mask.json`, embedded here.
//! Pure: no drawing, no GL.

use crate::core::{jf64, jstr, CamLayout};
use macroquad::prelude::{vec2, vec3, Vec2, Vec3};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::OnceLock;

pub const RULES_JSON: &str = include_str!("../../data/mask/rules.json");
pub const MAP_JSON: &str = include_str!("../../data/mask/map.json");
pub const NAV_JSON: &str = include_str!("../../data/nav/mask.json");

/// The hunters who walk the main hall (the one-at-a-time hall visitors).
pub const HALL_HUNTERS: [&str; 1] = ["thessine"];
/// The hunters who crawl the ducts (only one is ever out at a time).
pub const VENT_HUNTERS: [&str; 3] = ["verrine", "haelith", "ilyra"];
pub const RUSHER: &str = "umbriel";
pub const DUCTS: [&str; 2] = ["ductL", "ductR"];

/// Every key of the tuning table `t` the rules read.
pub const T_KEYS: [&str; 38] = [
    "hallSteps", "stepMs", "stepJitter", "windowMs", "leaveMs", "offGraceMs", "coolMs", "coolSpread", "offDrain", "ventWaitMs",
    "ventNeedMs", "ventCoolMs", "boxGraceMs", "ventMaskRate", "windPerSec", "roomMs", "roomJitter", "ductMs", "ductJitter",
    "hallCool0", "ventCool0", "rushCoolMs", "rushCool0", "flashCost", "haelithDuctMs", "haelithDuctJitter", "haelithWaitMs",
    "haelithNeedMs", "haelithFetchMs", "haelithCoolMs", "ilyraStepMs", "ilyraStepJitter", "ilyraSteps", "ilyraVentMs",
    "ilyraNeedMs", "ilyraCoolMs", "lampDrain", "camDrain",
];

/// The duct a side letter belongs to ("L" -> ductL).
pub fn duct_of(side: char) -> &'static str {
    if side == 'L' {
        "ductL"
    } else {
        "ductR"
    }
}

/// The side letter of a duct or vent room id ("ductL" / "ventL" -> 'L'); None for any other room.
pub fn side_of_room(room: &str) -> Option<char> {
    match room {
        "ductL" | "ventL" => Some('L'),
        "ductR" | "ventR" => Some('R'),
        _ => None,
    }
}

pub fn side_index(side: char) -> usize {
    if side == 'L' {
        0
    } else {
        1
    }
}

pub fn other_side(side: char) -> char {
    if side == 'L' {
        'R'
    } else {
        'L'
    }
}

// ---------------------------------------------------------------------------------------------------- rules

#[derive(Clone, Debug)]
pub struct RoomDef {
    pub id: String,
    pub cam: i32,
    pub label: String,
    pub pan: f32,
    pub map: (f32, f32),
}

#[derive(Clone, Debug, Default)]
pub struct Variety {
    pub rest_spread: f64,
    pub kill_spread: f64,
    pub kill_floor: f64,
    pub side_bias: (f64, f64),
    pub salt: u32,
    pub factors: Vec<f64>,
    pub ms: (f64, f64),
}

#[derive(Clone, Debug, Default)]
pub struct Rules {
    pub cast: Vec<String>,
    pub names: BTreeMap<String, String>,
    pub levels: BTreeMap<String, Vec<i32>>,
    pub box_decay: Vec<f64>,
    pub flash_seconds: Vec<f64>,
    pub t: BTreeMap<String, f64>,
    pub vent_from: BTreeMap<String, f64>,
    pub variety: Variety,
    pub panels: BTreeMap<String, (f64, f64)>,
    pub rooms: Vec<RoomDef>,
    pub graph: BTreeMap<String, Vec<String>>,
    pub home: BTreeMap<String, String>,
    pub tune: Vec<i32>,
    dist_office: BTreeMap<String, i32>,
}

fn f64s(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default()
}

fn pair(v: &Value) -> (f64, f64) {
    let a = f64s(v);
    (a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0))
}

impl Rules {
    pub fn parse(src: &str) -> Result<Rules, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| format!("mask rules: {e}"))?;
        let mut r = Rules::default();
        r.cast = root["cast"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        if r.cast.is_empty() {
            return Err("mask rules: no cast".to_string());
        }
        if let Some(o) = root["names"].as_object() {
            for (k, v) in o {
                r.names.insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
        }
        if let Some(o) = root["levels"].as_object() {
            for (k, v) in o {
                r.levels.insert(k.clone(), f64s(v).iter().map(|x| *x as i32).collect());
            }
        }
        r.box_decay = f64s(&root["box_decay"]);
        r.flash_seconds = f64s(&root["flash_seconds"]);
        if r.box_decay.len() < 6 || r.flash_seconds.len() < 6 {
            return Err("mask rules: box_decay and flash_seconds need six nights".to_string());
        }
        if let Some(o) = root["t"].as_object() {
            for (k, v) in o {
                r.t.insert(k.clone(), v.as_f64().unwrap_or(0.0));
            }
        }
        if let Some(o) = root["vent_from"].as_object() {
            for (k, v) in o {
                r.vent_from.insert(k.clone(), v.as_f64().unwrap_or(0.0));
            }
        }
        let va = &root["variety"];
        r.variety = Variety {
            rest_spread: jf64(va, "restSpread", 0.3),
            kill_spread: jf64(va, "killSpread", 0.2),
            kill_floor: jf64(va, "killFloor", 0.9),
            side_bias: pair(&va["sideBias"]),
            salt: jf64(va, "salt", 0.0) as u32,
            factors: f64s(&va["factors"]),
            ms: pair(&va["ms"]),
        };
        if r.variety.factors.is_empty() {
            return Err("mask rules: variety needs factors".to_string());
        }
        if let Some(o) = root["panels"].as_object() {
            for (k, v) in o {
                r.panels.insert(k.clone(), (jf64(v, "raise", 0.0), jf64(v, "lower", 0.0)));
            }
        }
        if let Some(o) = root["rooms"].as_object() {
            for (k, v) in o {
                let m = f64s(&v["map"]);
                r.rooms.push(RoomDef {
                    id: k.clone(),
                    cam: jf64(v, "cam", 0.0) as i32,
                    label: jstr(v, "label"),
                    pan: jf64(v, "pan", 0.0) as f32,
                    map: (m.first().copied().unwrap_or(0.0) as f32, m.get(1).copied().unwrap_or(0.0) as f32),
                });
            }
        }
        if let Some(o) = root["graph"].as_object() {
            for (k, v) in o {
                let list = v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                r.graph.insert(k.clone(), list);
            }
        }
        if let Some(o) = root["home"].as_object() {
            for (k, v) in o {
                r.home.insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
        }
        r.tune = f64s(&root["tune"]).iter().map(|x| *x as i32).collect();
        // breadth first distance of every room to the office
        let mut dist: BTreeMap<String, i32> = BTreeMap::new();
        dist.insert("office".to_string(), 0);
        let mut queue = VecDeque::new();
        queue.push_back("office".to_string());
        while let Some(room) = queue.pop_front() {
            let d = dist[&room];
            if let Some(next) = r.graph.get(&room) {
                for n in next {
                    if !dist.contains_key(n) {
                        dist.insert(n.clone(), d + 1);
                        queue.push_back(n.clone());
                    }
                }
            }
        }
        r.dist_office = dist;
        Ok(r)
    }

    /// One number of the tuning table `t` (0 when the key is missing; `check` reports missing keys).
    pub fn t(&self, key: &str) -> f64 {
        self.t.get(key).copied().unwrap_or(0.0)
    }

    pub fn home(&self, id: &str) -> &str {
        self.home.get(id).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn name(&self, id: &str) -> String {
        match self.names.get(id) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => crate::core::display_name(id),
        }
    }

    pub fn room(&self, id: &str) -> Option<&RoomDef> {
        self.rooms.iter().find(|r| r.id == id)
    }

    /// The camera rooms in camera order.
    pub fn cam_rooms(&self) -> Vec<String> {
        let mut v: Vec<&RoomDef> = self.rooms.iter().filter(|r| r.cam > 0).collect();
        v.sort_by_key(|r| r.cam);
        v.iter().map(|r| r.id.clone()).collect()
    }

    pub fn room_for_cam(&self, cam: i32) -> Option<String> {
        self.rooms.iter().find(|r| r.cam == cam).map(|r| r.id.clone())
    }

    /// "CAM 4  GAME AREA".
    pub fn cam_label(&self, room: &str) -> String {
        match self.room(room) {
            Some(r) => format!("CAM {}  {}", r.cam, r.label),
            None => String::new(),
        }
    }

    /// Where a sound from `room` comes from (-1 left .. +1 right).
    pub fn pan_of(&self, room: &str) -> f32 {
        self.room(room).map(|r| r.pan).unwrap_or(0.0)
    }

    /// The neighbour of `room` nearest the office (the room itself when it has none nearer).
    pub fn next_toward(&self, room: &str) -> String {
        let mut best = room.to_string();
        let mut best_d = self.dist_office.get(room).copied().unwrap_or(i32::MAX);
        if let Some(list) = self.graph.get(room) {
            for r in list {
                let d = self.dist_office.get(r).copied().unwrap_or(i32::MAX);
                if d < best_d {
                    best = r.clone();
                    best_d = d;
                }
            }
        }
        best
    }

    pub fn dist_to_office(&self, room: &str) -> i32 {
        self.dist_office.get(room).copied().unwrap_or(-1)
    }

    /// A hunter's base level on a night (0-based index, the last night is reused).
    pub fn level_of(&self, id: &str, night_idx: usize) -> i32 {
        self.levels.get(id).and_then(|l| l.get(night_idx.min(5))).copied().unwrap_or(0)
    }

    /// Problems in the tables (empty = fine).
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        for k in T_KEYS.iter() {
            if !self.t.contains_key(*k) {
                out.push(format!("mask rules: tuning key {k} is missing"));
            }
        }
        for id in &self.cast {
            if id != "glyra" && self.levels.get(id).map(|l| l.len()).unwrap_or(0) < 6 {
                out.push(format!("mask rules: {id} needs six night levels"));
            }
        }
        for id in ["thessine", "umbriel", "verrine", "haelith", "ilyra", "glyra"] {
            if self.home(id).is_empty() {
                out.push(format!("mask rules: {id} has no home room"));
            }
        }
        for (room, list) in &self.graph {
            if self.room(room).is_none() {
                out.push(format!("mask rules: graph room {room} is not in rooms"));
            }
            for n in list {
                if !self.graph.get(n).map(|l| l.contains(room)).unwrap_or(false) {
                    out.push(format!("mask rules: edge {room} - {n} is not mirrored"));
                }
            }
        }
        for r in &self.rooms {
            if self.dist_to_office(&r.id) < 0 {
                out.push(format!("mask rules: room {} cannot reach the office", r.id));
            }
        }
        out
    }
}

static RULES: OnceLock<(Rules, Option<String>)> = OnceLock::new();

fn load_rules() -> &'static (Rules, Option<String>) {
    RULES.get_or_init(|| match Rules::parse(RULES_JSON) {
        Ok(r) => (r, None),
        Err(e) => (Rules::default(), Some(e)),
    })
}

/// The embedded rules, or the parse error.
pub fn try_rules() -> Result<&'static Rules, String> {
    match load_rules() {
        (r, None) => Ok(r),
        (_, Some(e)) => Err(e.clone()),
    }
}

/// The embedded rules. After a failed load this is an empty table; the mode checks `load_error()` first and refuses to
/// start, and `check()` says why.
pub fn rules() -> &'static Rules {
    &load_rules().0
}

// ---------------------------------------------------------------------------------------------------- the rules' formulas

/// Seconds of flashlight per night, scaled to the shorter night (rounded).
pub fn flash_seconds(night_idx: usize) -> f64 {
    let r = rules();
    let base = r.flash_seconds.get(night_idx.min(5)).copied().unwrap_or(100.0);
    (base * crate::core::TIME_SCALE).round().max(1.0)
}

/// How long the dark lasts before Nyxie arrives.
pub fn dark_grace_ms(level: i32) -> f64 {
    (12500.0 - level as f64 * 450.0).max(4000.0)
}

/// Custom nights: the music box's loss per second for Glyra's level.
pub fn box_decay_for_level(level: i32) -> f64 {
    if level > 0 {
        0.3 + level as f64 * 0.25
    } else {
        0.0
    }
}

/// The quick flashes of the hall light that send Umbriel home.
pub fn rush_flashes(level: i32) -> i32 {
    3 + if level >= 10 { 1 } else { 0 } + if level >= 16 { 1 } else { 0 }
}

pub fn rush_run_ms(level: i32) -> f64 {
    (3000.0 - level as f64 * 90.0).max(1200.0)
}

pub fn rush_door_ms(level: i32) -> f64 {
    (2400.0 - level as f64 * 70.0).max(1000.0)
}

/// Who is on the floor on a night (0-based): the hunters with a level above 0, and Glyra once the music box matters.
pub fn active_hunters(level_idx: usize) -> Vec<String> {
    let r = rules();
    let mut out = Vec::new();
    for id in &r.cast {
        if id == "glyra" {
            if level_idx >= 1 {
                out.push(id.clone());
            }
        } else if r.level_of(id, level_idx) > 0 {
            out.push(id.clone());
        }
    }
    out
}

/// The first night each hunter is on the floor (Zellia introduces her that night).
pub fn first_night(id: &str) -> i32 {
    if id == "glyra" {
        return 2;
    }
    let r = rules();
    for i in 0..6 {
        if r.level_of(id, i) > 0 {
            return i as i32 + 1;
        }
    }
    0
}

// ---------------------------------------------------------------------------------------------------- the 3D map

#[derive(Clone, Debug)]
pub struct CameraDef {
    pub room: String,
    pub mount: Vec3,
    pub target: Vec3,
    pub fov: f32,
}

impl CameraDef {
    /// Yaw of the view in the player convention (0 looks down -Z, forward = (-sin, ., -cos)).
    pub fn yaw(&self) -> f32 {
        let d = self.target - self.mount;
        (-d.x).atan2(-d.z)
    }

    pub fn pitch(&self) -> f32 {
        let d = self.target - self.mount;
        d.y.atan2((d.x * d.x + d.z * d.z).sqrt())
    }
}

#[derive(Clone, Debug)]
pub struct MapData {
    pub half_w: f32,
    pub front_z: f32,
    pub back_z: f32,
    pub height: f32,
    pub opening: f32,
    pub spawn: Vec3,
    pub hall_near: f32,
    pub hall_far: f32,
    pub rush_door: f32,
    pub office_walker: Vec3,
    pub vent_stand: [Vec3; 2],
    pub centres: BTreeMap<String, Vec3>,
    pub stand_offsets: Vec<Vec3>,
    pub stand_lift: BTreeMap<String, Vec3>,
    pub duct_x: BTreeMap<String, f32>,
    pub glyra_offset: Vec3,
    pub glyra_advance: f32,
    pub nyxie_from_z: f32,
    pub nyxie_to: Vec3,
    pub cameras: BTreeMap<String, CameraDef>,
    pub cam_map: CamLayout,
}

fn v3(v: &Value) -> Vec3 {
    let a = f64s(v);
    vec3(a.first().copied().unwrap_or(0.0) as f32, a.get(1).copied().unwrap_or(0.0) as f32, a.get(2).copied().unwrap_or(0.0) as f32)
}

fn v2(v: &Value) -> Vec2 {
    let a = f64s(v);
    vec2(a.first().copied().unwrap_or(0.0) as f32, a.get(1).copied().unwrap_or(0.0) as f32)
}

impl MapData {
    pub fn parse(src: &str, rules: &Rules) -> Result<MapData, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| format!("mask map: {e}"))?;
        let o = &root["office"];
        let mut centres = BTreeMap::new();
        if let Some(c) = root["centres"].as_object() {
            for (k, v) in c {
                centres.insert(k.clone(), v3(v));
            }
        }
        let mut stand_lift = BTreeMap::new();
        if let Some(c) = root["stand_lift"].as_object() {
            for (k, v) in c {
                stand_lift.insert(k.clone(), v3(v));
            }
        }
        let mut duct_x = BTreeMap::new();
        if let Some(c) = root["duct_x"].as_object() {
            for (k, v) in c {
                duct_x.insert(k.clone(), v.as_f64().unwrap_or(0.0) as f32);
            }
        }
        let mut cameras = BTreeMap::new();
        if let Some(c) = root["cameras"].as_object() {
            for (k, v) in c {
                cameras.insert(
                    k.clone(),
                    CameraDef { room: k.clone(), mount: v3(&v["mount"]), target: v3(&v["target"]), fov: jf64(v, "fov", 62.0) as f32 },
                );
            }
        }
        let offsets: Vec<Vec3> = root["stand_offsets"].as_array().map(|a| a.iter().map(v3).collect()).unwrap_or_default();
        if offsets.len() < 3 {
            return Err("mask map: stand_offsets needs three slots".to_string());
        }
        let cm = &root["cam_map"];
        let mut layout = CamLayout::default();
        if let Some(rooms) = cm["rooms"].as_object() {
            for (k, v) in rooms {
                layout.rooms.push((k.clone(), v2(v)));
            }
        }
        if let Some(e) = cm["edges"].as_array() {
            for p in e {
                let a = p.as_array();
                if let Some(a) = a {
                    if a.len() >= 2 {
                        layout.edges.push((a[0].as_str().unwrap_or("").to_string(), a[1].as_str().unwrap_or("").to_string()));
                    }
                }
            }
        }
        layout.entrances = cm["entrances"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        layout.vents = cm["vents"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        layout.cams = rules.cam_rooms();
        layout.labels = rules.rooms.iter().filter(|r| r.cam > 0).map(|r| (r.id.clone(), r.label.clone())).collect();
        layout.office = Some(v2(&cm["office"]));
        Ok(MapData {
            half_w: jf64(o, "half_w", 3.4) as f32,
            front_z: jf64(o, "front_z", -1.8) as f32,
            back_z: jf64(o, "back_z", 3.6) as f32,
            height: jf64(o, "height", 3.7) as f32,
            opening: jf64(o, "opening", 2.5) as f32,
            spawn: v3(&o["spawn"]),
            hall_near: jf64(&root["hall"], "near", 3.3) as f32,
            hall_far: jf64(&root["hall"], "far", 15.0) as f32,
            rush_door: jf64(&root["hall"], "rush_door", 2.6) as f32,
            office_walker: v3(&root["hall"]["office_walker"]),
            vent_stand: [v3(&root["vent_stand"]["L"]), v3(&root["vent_stand"]["R"])],
            centres,
            stand_offsets: offsets,
            stand_lift,
            duct_x,
            glyra_offset: v3(&root["glyra_offset"]),
            glyra_advance: jf64(&root, "glyra_advance", 3.0) as f32,
            nyxie_from_z: jf64(&root["nyxie"], "from_z", -3.3) as f32,
            nyxie_to: v3(&root["nyxie"]["to"]),
            cameras,
            cam_map: layout,
        })
    }

    pub fn centre(&self, room: &str) -> Vec3 {
        self.centres.get(room).copied().unwrap_or(Vec3::ZERO)
    }

    pub fn camera(&self, room: &str) -> Option<&CameraDef> {
        self.cameras.get(room)
    }

    /// The camera mount of a room (the thing a hunter faces); the origin when the room has none.
    pub fn camera_origin(&self, room: &str) -> Option<Vec3> {
        self.cameras.get(room).map(|c| c.mount)
    }

    pub fn vent_stand_of(&self, side: char) -> Vec3 {
        self.vent_stand[side_index(side)]
    }

    /// Where a hunter stands in a room as the feed sees her (`slot` 0 / 1 / 2; `at` "far" / "stand" / "near" in the ducts).
    /// The main hall and the office use their own spots. The Show Stage spots are ON the platform.
    pub fn stand_at(&self, room: &str, slot: usize, at: &str) -> Vec3 {
        if room == "hall" {
            return vec3(0.0, 0.0, -(self.hall_near + slot as f32 * 1.2));
        }
        if room == "office" {
            return self.office_walker + vec3(slot as f32 * 0.9, 0.0, 0.0);
        }
        if room == "ductL" || room == "ductR" {
            let side = if room == "ductL" { -1.0 } else { 1.0 };
            let x = self.duct_x.get(at).copied().unwrap_or_else(|| self.duct_x.get("stand").copied().unwrap_or(5.4));
            return vec3(side * x, 0.0, 0.6);
        }
        let c = self.centre(room);
        let off = self.stand_offsets[slot.min(2)];
        let lift = self.stand_lift.get(room).copied().unwrap_or(Vec3::ZERO);
        c + off + lift
    }

    /// The hall walker's spot after `step` steps of `steps` (far end to the last step before the opening).
    pub fn hall_pos(&self, step: i32, steps: f64) -> Vec3 {
        let k = (step as f64 / steps.max(1.0)).clamp(0.0, 1.0) as f32;
        vec3(0.0, 0.0, -(self.hall_far + (self.hall_near - self.hall_far) * k))
    }

    /// Umbriel's spot while she runs the hall (0 = far end, 1 = at the doors).
    pub fn rush_pos(&self, progress: f32) -> Vec3 {
        let p = progress.clamp(0.0, 1.0);
        vec3(0.0, 0.0, -(self.hall_far + (self.rush_door - self.hall_far) * p))
    }

    /// Glyra's spot: by the music box, coming 3 m toward the office as the box stays empty (`back` 0..1).
    pub fn glyra_pos(&self, back: f32) -> Vec3 {
        self.stand_at("music", 0, "stand") + self.glyra_offset + vec3(0.0, 0.0, self.glyra_advance * back.clamp(0.0, 1.0))
    }

    /// Nyxie's walk down the dark hall as the blackout runs out (`t` 0..1).
    pub fn nyxie_pos(&self, t: f32) -> Vec3 {
        let from = vec3(0.0, 0.0, self.nyxie_from_z);
        from.lerp(self.nyxie_to, t.clamp(0.0, 1.0))
    }

    pub fn validate(&self) -> Vec<String> {
        self.validate_against(rules())
    }

    pub fn validate_against(&self, rules: &Rules) -> Vec<String> {
        let mut out = Vec::new();
        for room in rules.cam_rooms() {
            if !self.cameras.contains_key(&room) {
                out.push(format!("mask map: camera room {room} has no camera"));
            }
        }
        for room in ["ballroom", "music", "costume", "gallery", "crawl", "ductL", "ductR"] {
            if !self.centres.contains_key(room) {
                out.push(format!("mask map: room {room} has no centre"));
            }
        }
        out
    }
}

static MAP: OnceLock<Result<MapData, String>> = OnceLock::new();
static INERT: OnceLock<MapData> = OnceLock::new();

/// An all-zero placeholder so `map()` can hand out a reference after a failed load. It carries no gameplay numbers of its own
/// (no camera, no centre): the mode refuses to start while `load_error()` is set, so nothing plays on it.
fn inert_map() -> MapData {
    MapData {
        half_w: 0.0,
        front_z: 0.0,
        back_z: 0.0,
        height: 0.0,
        opening: 0.0,
        spawn: Vec3::ZERO,
        hall_near: 0.0,
        hall_far: 0.0,
        rush_door: 0.0,
        office_walker: Vec3::ZERO,
        vent_stand: [Vec3::ZERO, Vec3::ZERO],
        centres: BTreeMap::new(),
        stand_offsets: vec![Vec3::ZERO; 3],
        stand_lift: BTreeMap::new(),
        duct_x: BTreeMap::new(),
        glyra_offset: Vec3::ZERO,
        glyra_advance: 0.0,
        nyxie_from_z: 0.0,
        nyxie_to: Vec3::ZERO,
        cameras: BTreeMap::new(),
        cam_map: CamLayout::default(),
    }
}

fn load_map() -> &'static Result<MapData, String> {
    MAP.get_or_init(|| MapData::parse(MAP_JSON, rules()))
}

/// The embedded map numbers, or the parse error.
pub fn try_map() -> Result<&'static MapData, String> {
    load_map().as_ref().map_err(|e| e.clone())
}

/// The embedded map numbers. After a failed load this is an inert placeholder; the mode checks `load_error()` first and
/// refuses to start, and `check()` reports the error.
pub fn map() -> &'static MapData {
    match load_map() {
        Ok(m) => m,
        Err(_) => INERT.get_or_init(inert_map),
    }
}

/// Why Masked Hours cannot start (a rules, map or nav file that does not parse), if so. The mode shows this on the shift
/// screen instead of playing.
pub fn load_error() -> Option<String> {
    if let Some(e) = &load_rules().1 {
        return Some(e.clone());
    }
    if let Err(e) = load_map() {
        return Some(e.clone());
    }
    crate::hunters::NavMap::from_json(NAV_JSON).err().map(|e| format!("mask nav: {e}"))
}

/// Every problem in Masked Hours' data files: parse errors, missing keys, a nav graph with a hole in it, a task spot
/// without a pose. Used by `--check` and the tests.
pub fn check() -> Vec<String> {
    check_sources(RULES_JSON, MAP_JSON, NAV_JSON)
}

/// `check()` on the given file texts (so a test can feed a broken one).
pub fn check_sources(rules_src: &str, map_src: &str, nav_src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let rules = match Rules::parse(rules_src) {
        Ok(r) => r,
        Err(e) => {
            out.push(e);
            return out;
        }
    };
    out.extend(rules.validate());
    match MapData::parse(map_src, &rules) {
        Ok(m) => out.extend(m.validate_against(&rules)),
        Err(e) => out.push(e),
    }
    match crate::hunters::NavMap::from_json(nav_src) {
        Ok(n) => {
            out.extend(n.validate().into_iter().map(|p| format!("mask nav: {p}")));
            for room in rules.cam_rooms() {
                if n.hub_key(&room).is_none() {
                    out.push(format!("mask nav: camera room {room} has no hub node"));
                }
            }
        }
        Err(e) => out.push(format!("mask nav: {e}")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_file_is_an_error_not_a_fallback() {
        assert!(Rules::parse("{ not json").is_err());
        let rules = Rules::parse(RULES_JSON).expect("rules");
        assert!(MapData::parse("{ not json", &rules).is_err());
        assert!(MapData::parse("{}", &rules).is_err(), "a map without stand_offsets is refused, not patched");
        let p = check_sources("{ not json", MAP_JSON, NAV_JSON);
        assert!(!p.is_empty() && p[0].contains("mask rules"), "{p:?}");
        let p = check_sources(RULES_JSON, "{ not json", NAV_JSON);
        assert!(p.iter().any(|m| m.contains("mask map")), "{p:?}");
        let p = check_sources(RULES_JSON, MAP_JSON, "{ not json");
        assert!(p.iter().any(|m| m.contains("mask nav")), "{p:?}");
    }

    #[test]
    fn the_embedded_files_load_without_an_error() {
        assert_eq!(load_error(), None);
        assert!(try_rules().is_ok() && try_map().is_ok());
        assert!(!map().cameras.is_empty(), "the real map, not the inert one");
    }

    #[test]
    fn the_inert_map_has_no_gameplay_numbers() {
        let m = inert_map();
        assert!(m.cameras.is_empty() && m.centres.is_empty());
        assert_eq!(m.half_w, 0.0);
    }

    #[test]
    fn the_embedded_files_are_clean() {
        let problems = check();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_tables_carry_the_godot_numbers() {
        let r = rules();
        assert_eq!(r.variety.salt, 0x3d1f_8a57);
        assert_eq!(r.level_of("thessine", 0), 3);
        assert_eq!(r.level_of("thessine", 5), 17);
        assert_eq!(r.level_of("umbriel", 1), 0);
        assert_eq!(r.level_of("ilyra", 4), 3);
        assert_eq!(r.box_decay, vec![2.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(r.t("stepMs"), 2300.0);
        assert_eq!(r.t("windowMs"), 3500.0);
        assert_eq!(r.t("ventNeedMs"), 1400.0);
        assert_eq!(r.vent_from.get("haelith").copied(), Some(45_000.0));
        assert_eq!(r.panels.get("mask").copied(), Some((200.0, 150.0)));
        assert_eq!(r.panels.get("monitor").copied(), Some((250.0, 200.0)));
        assert_eq!(r.home("verrine"), "crawl");
        assert_eq!(r.tune.len(), 13);
    }

    #[test]
    fn the_graph_walks_toward_the_office() {
        let r = rules();
        assert_eq!(r.next_toward("costume"), "ballroom");
        assert_eq!(r.next_toward("ballroom"), "hall");
        assert_eq!(r.next_toward("hall"), "office");
        assert_eq!(r.next_toward("crawl"), "ductL");
        assert_eq!(r.next_toward("ductR"), "ventR");
        assert_eq!(r.next_toward("ventL"), "office");
        assert_eq!(r.dist_to_office("costume"), 3);
        assert_eq!(r.dist_to_office("crawl"), 3);
    }

    #[test]
    fn camera_rooms_run_in_camera_order() {
        let r = rules();
        assert_eq!(r.cam_rooms(), vec!["music", "costume", "gallery", "ballroom", "crawl", "ductL", "ductR", "hall"]);
        assert_eq!(r.room_for_cam(8).as_deref(), Some("hall"));
        assert_eq!(r.cam_label("ballroom"), "CAM 4  GAME AREA");
        assert_eq!(r.room_for_cam(9), None);
    }

    #[test]
    fn the_formulas_match_the_godot_numbers() {
        assert_eq!(flash_seconds(0), 66.0);
        assert_eq!(flash_seconds(1), 56.0);
        assert_eq!(flash_seconds(5), 28.0);
        assert_eq!(dark_grace_ms(0), 12500.0);
        assert_eq!(dark_grace_ms(12), 7100.0);
        assert_eq!(dark_grace_ms(40), 4000.0);
        assert_eq!(rush_flashes(3), 3);
        assert_eq!(rush_flashes(10), 4);
        assert_eq!(rush_flashes(16), 5);
        assert_eq!(rush_run_ms(10), 2100.0);
        assert_eq!(rush_door_ms(10), 1700.0);
        assert!((box_decay_for_level(4) - 1.3).abs() < 1e-9);
        assert_eq!(box_decay_for_level(0), 0.0);
    }

    #[test]
    fn who_is_on_the_floor_each_night() {
        assert_eq!(active_hunters(0), vec!["thessine", "nyxie"]);
        assert!(active_hunters(1).contains(&"glyra".to_string()));
        assert!(active_hunters(2).contains(&"umbriel".to_string()));
        assert_eq!(first_night("umbriel"), 3);
        assert_eq!(first_night("ilyra"), 5);
        assert_eq!(first_night("glyra"), 2);
    }

    #[test]
    fn stand_spots_match_the_godot_offsets() {
        let m = map();
        let near = |a: Vec3, b: Vec3| (a - b).length() < 1e-4;
        assert!(near(m.stand_at("ballroom", 0, "stand"), vec3(0.9, 0.0, -23.4)));
        assert!(near(m.stand_at("music", 1, "stand"), vec3(-15.4, 0.0, -25.6)));
        assert!(near(m.stand_at("hall", 1, "stand"), vec3(0.0, 0.0, -4.5)));
        assert!(near(m.stand_at("office", 0, "stand"), vec3(0.0, 0.0, -1.25)));
        assert!(near(m.stand_at("ductL", 0, "far"), vec3(-7.2, 0.0, 0.6)));
        assert!(near(m.stand_at("ductR", 0, "near"), vec3(3.5, 0.0, 0.6)));
        // the show stage spots are on the platform
        assert!((m.stand_at("gallery", 0, "stand").y - 0.7).abs() < 1e-6);
    }

    #[test]
    fn the_hall_and_the_rush_run_toward_the_office() {
        let m = map();
        assert!((m.hall_pos(0, 4.0).z + 15.0).abs() < 1e-5);
        assert!((m.hall_pos(4, 4.0).z + 3.3).abs() < 1e-5);
        assert!((m.hall_pos(2, 4.0).z + 9.15).abs() < 1e-5);
        assert_eq!(m.rush_pos(0.0).z, -15.0);
        assert!((m.rush_pos(1.0).z + 2.6).abs() < 1e-6);
    }

    #[test]
    fn camera_yaw_and_pitch_follow_the_mount() {
        let m = map();
        let c = m.camera("hall").expect("hall camera");
        assert!(c.yaw().abs() < 1e-5, "the hall camera looks straight down -Z");
        assert!(c.pitch() < 0.0, "and a little down");
        let d = m.camera("ductL").expect("duct camera");
        assert!((d.yaw() + std::f32::consts::FRAC_PI_2).abs() < 1e-4, "ductL looks toward +X: yaw -90 degrees");
    }

    #[test]
    fn the_cam_map_has_every_camera() {
        let l = &map().cam_map;
        assert_eq!(l.cams.len(), 8);
        assert!(l.rooms.iter().any(|(r, _)| r == "party4"));
        assert_eq!(l.entrances, vec!["hall", "ductL", "ductR"]);
        assert_eq!(l.office.map(|o| o.y), Some(2.0));
    }
}
