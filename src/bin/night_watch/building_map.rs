//! Night Watch's building maps (schema nw.map/1, `data/maps/<mode>.json`, one file per mode, embedded like the calls).
//! A `BuildingMap` is the WHOLE building as a floor plan: every room with its outline in metres (z runs south), the floors,
//! the links between rooms (door, corridor, vent, stairs, path, window), the cameras and the rooms each one covers, the
//! player's office (or offices) and where the hunters start. It answers the questions the modes ask (who is next to whom,
//! which way is the office, which camera sees this room) and draws itself: `scene` turns the map into plain primitives,
//! `draw` paints them with macroquad and `to_svg` writes the same picture as an SVG, so the layout is testable without a window.
//! Room ids are the sim ids of `data/buildings/<mode>.json` where one exists, so a mode can feed its hunter positions in as they are.
//! 

use macroquad::prelude::{draw_circle, draw_circle_lines, draw_line, draw_rectangle, draw_text, draw_triangle, vec2, Color, Rect, Vec2};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::OnceLock;

/// The mode ids with a map, in hub order of `data.rs`.
#[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
pub const MODES: [&str; 9] =
    ["classic", "masked_hours", "dead_air", "bedtime", "facility", "descent", "salvage_run", "open_air", "mixed_shift"];

const MAP_FILES: [(&str, &str); 9] = [
    ("classic", include_str!("data/maps/classic.json")),
    ("masked_hours", include_str!("data/maps/masked_hours.json")),
    ("dead_air", include_str!("data/maps/dead_air.json")),
    ("bedtime", include_str!("data/maps/bedtime.json")),
    ("facility", include_str!("data/maps/facility.json")),
    ("descent", include_str!("data/maps/descent.json")),
    ("salvage_run", include_str!("data/maps/salvage_run.json")),
    ("open_air", include_str!("data/maps/open_air.json")),
    ("mixed_shift", include_str!("data/maps/mixed_shift.json")),
];

const SCHEMA: &str = "nw.map/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    Room,
    Hall,
    Door,
    Vent,
    Office,
    Outdoor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Door,
    Corridor,
    Vent,
    Stairs,
    Path,
    Window,
}

impl LinkKind {
    pub const ALL: [LinkKind; 6] =
        [LinkKind::Door, LinkKind::Corridor, LinkKind::Vent, LinkKind::Stairs, LinkKind::Path, LinkKind::Window];

    fn parse(s: &str) -> Option<LinkKind> {
        Some(match s {
            "door" => LinkKind::Door,
            "corridor" => LinkKind::Corridor,
            "vent" => LinkKind::Vent,
            "stairs" => LinkKind::Stairs,
            "path" => LinkKind::Path,
            "window" => LinkKind::Window,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Door => "DOOR",
            LinkKind::Corridor => "CORRIDOR",
            LinkKind::Vent => "VENT",
            LinkKind::Stairs => "STAIRS",
            LinkKind::Path => "PATH",
            LinkKind::Window => "WINDOW",
        }
    }
}

impl RoomKind {
    fn parse(s: &str) -> Option<RoomKind> {
        Some(match s {
            "room" => RoomKind::Room,
            "hall" => RoomKind::Hall,
            "door" => RoomKind::Door,
            "vent" => RoomKind::Vent,
            "office" => RoomKind::Office,
            "outdoor" => RoomKind::Outdoor,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Floor {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Room {
    pub id: String,
    pub label: String,
    /// The room's name in the Godot map when the sim id differs from it (Mixed Shift).
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub alias: Option<String>,
    pub kind: RoomKind,
    /// Index into `BuildingMap::floors`.
    pub floor: usize,
    /// The outline in metres; a rect is its four corners.
    pub poly: Vec<Vec2>,
    pub bounds: Rect,
    /// True when a camera feed shows the room (or the map has no cameras at all and every room is heard or seen directly,
    /// or it is the office). False rooms are drawn dimmed: the hunters there are not visible.
    pub live: bool,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    pub kind: LinkKind,
    /// False for a link a hunter cannot use (a window you only look through).
    pub walk: bool,
    /// Drawing waypoints between the two room centres, in metres.
    pub via: Vec<Vec2>,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub id: u32,
    pub label: String,
    /// Indexes into `BuildingMap::rooms`.
    pub covers: Vec<usize>,
}

/// Where a hunter starts, for modes that want a default marker.
#[derive(Clone, Debug)]
pub struct Spawn {
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub id: String,
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub room: usize,
}

#[derive(Debug)]
pub struct BuildingMap {
    pub mode: String,
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub theme: String,
    pub floors: Vec<Floor>,
    pub rooms: Vec<Room>,
    pub links: Vec<Link>,
    pub cameras: Vec<Camera>,
    /// Indexes of the player's rooms: one office, or three for Open Air's watches.
    pub offices: Vec<usize>,
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub spawns: Vec<Spawn>,
    index: HashMap<String, usize>,
    /// Per room, the indexes of the links touching it.
    adjacency: Vec<Vec<usize>>,
}

// ---- loading -------------------------------------------------------------------------------------------------------

/// Every mode's map, parsed once. Panics on a bad embedded file: that is a content bug.
pub fn all() -> &'static [BuildingMap] {
    static MAPS: OnceLock<Vec<BuildingMap>> = OnceLock::new();
    MAPS.get_or_init(|| {
        MAP_FILES
            .iter()
            .map(|(mode, text)| {
                let map = BuildingMap::parse(text).unwrap_or_else(|e| panic!("night_watch data/maps/{mode}.json: {e}"));
                assert_eq!(map.mode, *mode, "night_watch data/maps/{mode}.json names mode '{}'", map.mode);
                map
            })
            .collect()
    })
}

/// The map of `mode_id`, if the mode has one.
pub fn load(mode_id: &str) -> Option<&'static BuildingMap> {
    all().iter().find(|m| m.mode == mode_id)
}

fn text_of<'a>(v: &'a Value, key: &str, ctx: &str) -> Result<&'a str, String> {
    v[key].as_str().ok_or_else(|| format!("{ctx}: missing string '{key}'"))
}

fn number(v: &Value, ctx: &str) -> Result<f32, String> {
    v.as_f64().map(|n| n as f32).ok_or_else(|| format!("{ctx}: not a number"))
}

fn point(v: &Value, ctx: &str) -> Result<Vec2, String> {
    match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => Ok(vec2(number(x, ctx)?, number(y, ctx)?)),
        _ => Err(format!("{ctx}: a point is [x, z]")),
    }
}

fn bounds_of(poly: &[Vec2]) -> Rect {
    let (mut lo, mut hi) = (vec2(f32::MAX, f32::MAX), vec2(f32::MIN, f32::MIN));
    for p in poly {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    Rect::new(lo.x, lo.y, hi.x - lo.x, hi.y - lo.y)
}

#[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
fn polygon_contains(poly: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn rect_pts(r: Rect) -> Vec<Vec2> {
    vec![vec2(r.x, r.y), vec2(r.x + r.w, r.y), vec2(r.x + r.w, r.y + r.h), vec2(r.x, r.y + r.h)]
}

impl BuildingMap {
    /// Parses and validates one nw.map/1 file. The error names the first thing wrong.
    pub fn parse(text: &str) -> Result<BuildingMap, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        if v["schema"] != SCHEMA {
            return Err(format!("schema must be {SCHEMA}"));
        }
        let mode = text_of(&v, "mode", "map")?.to_string();
        let theme = v["theme"].as_str().unwrap_or("").to_string();

        let mut floors = Vec::new();
        for f in v["floors"].as_array().ok_or("map: missing 'floors'")? {
            let id = text_of(f, "id", "floor")?.to_string();
            if floors.iter().any(|g: &Floor| g.id == id) {
                return Err(format!("floor '{id}' is listed twice"));
            }
            floors.push(Floor { label: f["label"].as_str().unwrap_or(&id).to_string(), id });
        }
        if floors.is_empty() {
            return Err("map has no floors".into());
        }

        let mut rooms: Vec<Room> = Vec::new();
        let mut index = HashMap::new();
        for r in v["rooms"].as_array().ok_or("map: missing 'rooms'")? {
            let id = text_of(r, "id", "room")?.to_string();
            let ctx = format!("room '{id}'");
            if index.insert(id.clone(), rooms.len()).is_some() {
                return Err(format!("{ctx} is listed twice"));
            }
            let floor = match r["floor"].as_str() {
                None => 0,
                Some(f) => floors.iter().position(|g| g.id == f).ok_or_else(|| format!("{ctx}: unknown floor '{f}'"))?,
            };
            let poly = if let Some(rect) = r["rect"].as_array() {
                let n = rect.iter().map(|n| number(n, &ctx)).collect::<Result<Vec<_>, _>>()?;
                let &[x0, z0, x1, z1] = n.as_slice() else { return Err(format!("{ctx}: rect is [x0, z0, x1, z1]")) };
                if x1 <= x0 || z1 <= z0 {
                    return Err(format!("{ctx}: rect has no area"));
                }
                rect_pts(Rect::new(x0, z0, x1 - x0, z1 - z0))
            } else if let Some(pts) = r["poly"].as_array() {
                let poly = pts.iter().map(|p| point(p, &ctx)).collect::<Result<Vec<_>, _>>()?;
                if poly.len() < 3 {
                    return Err(format!("{ctx}: poly needs 3 points"));
                }
                poly
            } else {
                return Err(format!("{ctx}: needs a rect or a poly"));
            };
            let kind = match r["kind"].as_str() {
                None => RoomKind::Room,
                Some(k) => RoomKind::parse(k).ok_or_else(|| format!("{ctx}: unknown kind '{k}'"))?,
            };
            rooms.push(Room {
                label: r["label"].as_str().unwrap_or(&id).to_string(),
                alias: r["alias"].as_str().map(str::to_string),
                id,
                kind,
                floor,
                bounds: bounds_of(&poly),
                poly,
                live: false,
            });
        }
        if let Some(f) = (0..floors.len()).find(|f| !rooms.iter().any(|r| r.floor == *f)) {
            return Err(format!("floor '{}' has no rooms", floors[f].id));
        }

        let room_ref = |id: &str, ctx: &str| index.get(id).copied().ok_or_else(|| format!("{ctx}: unknown room '{id}'"));

        let mut links: Vec<Link> = Vec::new();
        for l in v["links"].as_array().ok_or("map: missing 'links'")? {
            let (a, b) = (text_of(l, "a", "link")?, text_of(l, "b", "link")?);
            let ctx = format!("link {a} - {b}");
            let (ai, bi) = (room_ref(a, &ctx)?, room_ref(b, &ctx)?);
            if ai == bi {
                return Err(format!("{ctx}: joins a room to itself"));
            }
            if links.iter().any(|k| (k.a, k.b) == (ai, bi) || (k.a, k.b) == (bi, ai)) {
                return Err(format!("{ctx}: listed twice"));
            }
            let kind = LinkKind::parse(l["kind"].as_str().unwrap_or("door")).ok_or_else(|| format!("{ctx}: unknown kind"))?;
            let via = match l["via"].as_array() {
                None => Vec::new(),
                Some(pts) => pts.iter().map(|p| point(p, &ctx)).collect::<Result<Vec<_>, _>>()?,
            };
            links.push(Link { a: ai, b: bi, kind, walk: l["walk"].as_bool().unwrap_or(true), via });
        }

        let mut cameras: Vec<Camera> = Vec::new();
        for c in v["cameras"].as_array().ok_or("map: missing 'cameras'")? {
            let id = c["id"].as_u64().ok_or("camera: missing 'id'")? as u32;
            let ctx = format!("camera {id}");
            if cameras.iter().any(|k| k.id == id) {
                return Err(format!("{ctx} is listed twice"));
            }
            let covers = c["covers"]
                .as_array()
                .ok_or_else(|| format!("{ctx}: missing 'covers'"))?
                .iter()
                .map(|r| room_ref(r.as_str().unwrap_or(""), &ctx))
                .collect::<Result<Vec<_>, _>>()?;
            if covers.is_empty() {
                return Err(format!("{ctx} covers no room"));
            }
            cameras.push(Camera { id, label: c["label"].as_str().unwrap_or("").to_string(), covers });
        }

        let offices = v["offices"]
            .as_array()
            .ok_or("map: missing 'offices'")?
            .iter()
            .map(|r| room_ref(r.as_str().unwrap_or(""), "office"))
            .collect::<Result<Vec<_>, _>>()?;
        if offices.is_empty() {
            return Err("map has no office".into());
        }

        let mut spawns = Vec::new();
        for s in v["spawns"].as_array().ok_or("map: missing 'spawns'")? {
            let id = text_of(s, "id", "spawn")?.to_string();
            let room = room_ref(text_of(s, "room", "spawn")?, &format!("spawn {id}"))?;
            spawns.push(Spawn { id, room });
        }

        let blind = cameras.is_empty();
        for (i, room) in rooms.iter_mut().enumerate() {
            room.live = blind || offices.contains(&i) || cameras.iter().any(|c| c.covers.contains(&i));
        }
        let mut adjacency = vec![Vec::new(); rooms.len()];
        for (i, l) in links.iter().enumerate() {
            adjacency[l.a].push(i);
            adjacency[l.b].push(i);
        }
        Ok(BuildingMap { mode, theme, floors, rooms, links, cameras, offices, spawns, index, adjacency })
    }

    // ---- queries ---------------------------------------------------------------------------------------------------

    pub fn room_index(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub fn room(&self, id: &str) -> Option<&Room> {
        self.room_index(id).map(|i| &self.rooms[i])
    }

    fn other(&self, link: usize, room: usize) -> usize {
        let l = &self.links[link];
        if l.a == room {
            l.b
        } else {
            l.a
        }
    }

    /// The rooms a hunter can step to from `id` (walkable links, both directions).
    pub fn neighbours(&self, id: &str) -> Vec<&str> {
        let Some(i) = self.room_index(id) else { return Vec::new() };
        self.adjacency[i]
            .iter()
            .filter(|l| self.links[**l].walk)
            .map(|l| self.rooms[self.other(*l, i)].id.as_str())
            .collect()
    }

    /// True when `a` and `b` are joined by a walkable link.
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn rooms_adjacent(&self, a: &str, b: &str) -> bool {
        self.link_between(a, b).is_some_and(|l| l.walk)
    }

    /// The link joining `a` and `b`, walkable or not.
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn link_between(&self, a: &str, b: &str) -> Option<&Link> {
        let (ai, bi) = (self.room_index(a)?, self.room_index(b)?);
        self.adjacency[ai].iter().map(|l| &self.links[*l]).find(|l| (l.a, l.b) == (ai, bi) || (l.a, l.b) == (bi, ai))
    }

    /// The room whose outline holds the point `p` (metres) on `floor`.
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn room_at(&self, floor: usize, p: Vec2) -> Option<&Room> {
        self.rooms.iter().find(|r| r.floor == floor && r.bounds.contains(p) && polygon_contains(&r.poly, p))
    }

    /// The shortest walk from `from` to `to`, both ends included, over every walkable link.
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn path(&self, from: &str, to: &str) -> Option<Vec<&str>> {
        self.path_where(from, to, |_| true)
    }

    /// The shortest walk that only uses links `allow` accepts (a hunter that cannot use vents, say).
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn path_where(&self, from: &str, to: &str, allow: impl Fn(&Link) -> bool) -> Option<Vec<&str>> {
        let (start, goal) = (self.room_index(from)?, self.room_index(to)?);
        let mut came: Vec<Option<usize>> = vec![None; self.rooms.len()];
        let mut seen = vec![false; self.rooms.len()];
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(at) = queue.pop_front() {
            if at == goal {
                let mut walk = vec![goal];
                let mut cur = goal;
                while let Some(prev) = came[cur] {
                    walk.push(prev);
                    cur = prev;
                }
                return Some(walk.iter().rev().map(|i| self.rooms[*i].id.as_str()).collect());
            }
            for l in &self.adjacency[at] {
                let link = &self.links[*l];
                let next = self.other(*l, at);
                if link.walk && allow(link) && !seen[next] {
                    seen[next] = true;
                    came[next] = Some(at);
                    queue.push_back(next);
                }
            }
        }
        None
    }

    pub fn camera(&self, id: u32) -> Option<&Camera> {
        self.cameras.iter().find(|c| c.id == id)
    }

    /// The first camera whose feed shows `room`.
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn camera_for_room(&self, room: &str) -> Option<&Camera> {
        let i = self.room_index(room)?;
        self.cameras.iter().find(|c| c.covers.contains(&i))
    }

    /// The player's first office.
    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn office(&self) -> &Room {
        &self.rooms[self.offices[0]]
    }

    pub fn is_office(&self, room: &str) -> bool {
        self.room_index(room).is_some_and(|i| self.offices.contains(&i))
    }

    #[allow(dead_code)] // read once the other modes use their maps (only Classic does today)
    pub fn is_live(&self, room: &str) -> bool {
        self.room(room).is_some_and(|r| r.live)
    }

    /// Where two rooms share a wall: the middle of the shared stretch and whether the wall runs north to south.
    /// None when the rooms do not touch along at least 0.6 m (or are not rectangles' neighbours).
    pub fn door_between(&self, a: &str, b: &str) -> Option<(Vec2, bool)> {
        let (ra, rb) = (self.room(a)?, self.room(b)?);
        if ra.floor != rb.floor {
            return None;
        }
        let (x, y) = (ra.bounds, rb.bounds);
        let near = |p: f32, q: f32| (p - q).abs() < 1e-3;
        let span = |lo0: f32, hi0: f32, lo1: f32, hi1: f32| (lo0.max(lo1), hi0.min(hi1));
        for wall in [x.x + x.w, x.x] {
            if near(wall, y.x) || near(wall, y.x + y.w) {
                let (lo, hi) = span(x.y, x.y + x.h, y.y, y.y + y.h);
                if hi - lo >= 0.6 {
                    return Some((vec2(wall, (lo + hi) * 0.5), true));
                }
            }
        }
        for wall in [x.y + x.h, x.y] {
            if near(wall, y.y) || near(wall, y.y + y.h) {
                let (lo, hi) = span(x.x, x.x + x.w, y.x, y.x + y.w);
                if hi - lo >= 0.6 {
                    return Some((vec2((lo + hi) * 0.5, wall), false));
                }
            }
        }
        None
    }
}

impl Room {
    /// True when the two outlines share floor area (touching walls do not count).
    #[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
    pub fn overlaps(&self, other: &Room) -> bool {
        if self.floor != other.floor {
            return false;
        }
        let (a, b) = (self.bounds, other.bounds);
        let (lo, hi) = (vec2(a.x.max(b.x), a.y.max(b.y)), vec2((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h)));
        if hi.x - lo.x <= 1e-3 || hi.y - lo.y <= 1e-3 {
            return false;
        }
        let step = 0.25;
        let mut x = lo.x + 0.01;
        while x < hi.x {
            let mut y = lo.y + 0.01;
            while y < hi.y {
                let p = vec2(x, y);
                if polygon_contains(&self.poly, p) && polygon_contains(&other.poly, p) {
                    return true;
                }
                y += step;
            }
            x += step;
        }
        false
    }
}

// ---- the picture ---------------------------------------------------------------------------------------------------

/// One drawing primitive, in screen pixels. `scene` makes them, `draw` paints them and `to_svg` writes them.
#[derive(Clone, Debug)]
pub enum Prim {
    /// A filled polygon (convex or star-shaped around its centre).
    Fill { pts: Vec<Vec2>, color: Color },
    /// A closed outline.
    Outline { pts: Vec<Vec2>, color: Color, width: f32 },
    Line { a: Vec2, b: Vec2, color: Color, width: f32 },
    Circle { c: Vec2, r: f32, fill: Color, edge: Option<Color> },
    /// `at` is the left end of the baseline.
    Text { at: Vec2, text: String, size: f32, color: Color },
}

/// What to show on top of the plan.
pub struct MapView<'a> {
    /// The camera whose feed is on screen; it and the rooms it covers are highlighted.
    pub active_camera: Option<u32>,
    /// Hunter markers as (room id, hunter id). Rooms that are not on the map are ignored.
    pub hunters: &'a [(&'a str, &'a str)],
    /// None draws every floor, laid out side by side; Some(i) shows floor i alone as a tab.
    pub floor: Option<usize>,
    pub legend: bool,
}

impl Default for MapView<'_> {
    fn default() -> Self {
        MapView { active_camera: None, hunters: &[], floor: None, legend: true }
    }
}

const BG: Color = Color::new(0.035, 0.030, 0.025, 1.0);
const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.420, 0.405, 0.360, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const RED: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const FRAME: Color = Color::new(0.16, 0.15, 0.13, 1.0);

const MARGIN: f32 = 10.0;
const TITLE_H: f32 = 28.0;
const LEGEND_H: f32 = 40.0;
const FLOOR_LABEL_H: f32 = 18.0;
const FLOOR_GAP: f32 = 26.0;
const FLOOR_PAD_M: f32 = 1.0;
const MAX_SCALE: f32 = 40.0;
const CAM_ICON: Vec2 = Vec2::new(22.0, 13.0);

fn kind_fill(kind: RoomKind) -> Color {
    match kind {
        RoomKind::Room => Color::new(0.13, 0.15, 0.14, 1.0),
        RoomKind::Hall => Color::new(0.11, 0.13, 0.16, 1.0),
        RoomKind::Door => Color::new(0.19, 0.16, 0.11, 1.0),
        RoomKind::Vent => Color::new(0.10, 0.15, 0.12, 1.0),
        RoomKind::Office => Color::new(0.22, 0.17, 0.08, 1.0),
        RoomKind::Outdoor => Color::new(0.09, 0.15, 0.09, 1.0),
    }
}

fn link_color(kind: LinkKind) -> Color {
    match kind {
        LinkKind::Door => Color::new(0.80, 0.76, 0.62, 1.0),
        LinkKind::Corridor => Color::new(0.30, 0.34, 0.38, 1.0),
        LinkKind::Vent => Color::new(0.45, 0.78, 0.60, 1.0),
        LinkKind::Stairs => Color::new(0.55, 0.65, 0.95, 1.0),
        LinkKind::Path => Color::new(0.55, 0.50, 0.38, 1.0),
        LinkKind::Window => Color::new(0.60, 0.80, 0.95, 1.0),
    }
}

fn dim(c: Color, k: f32) -> Color {
    Color::new(c.r * k, c.g * k, c.b * k, c.a)
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, a)
}

/// The width of `text` at `size`, estimated (no font is loaded when a scene is built, so tests need no window).
fn text_width(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.56
}

fn centroid(pts: &[Vec2]) -> Vec2 {
    pts.iter().copied().sum::<Vec2>() / pts.len() as f32
}

/// Where a floor sits on screen: `min` (metres) maps to `frame`'s top-left, `scale` is pixels per metre.
#[derive(Clone, Copy, Debug)]
struct FloorFrame {
    floor: usize,
    min: Vec2,
    scale: f32,
    frame: Rect,
}

impl FloorFrame {
    fn px(&self, p: Vec2) -> Vec2 {
        vec2(self.frame.x, self.frame.y) + (p - self.min) * self.scale
    }

    fn rect_px(&self, b: Rect) -> Rect {
        let o = self.px(vec2(b.x, b.y));
        Rect::new(o.x, o.y, b.w * self.scale, b.h * self.scale)
    }
}

/// The cameras' icon sits in the top-right corner of a room, or above a room too small to hold it.
fn cam_icon_centre(b: Rect) -> Vec2 {
    if b.w < 70.0 || b.h < 36.0 {
        vec2(b.x + b.w * 0.5, b.y - CAM_ICON.y * 0.5 - 2.0)
    } else {
        vec2(b.x + b.w - CAM_ICON.x * 0.5 - 4.0, b.y + CAM_ICON.y * 0.5 + 4.0)
    }
}

fn dashed(out: &mut Vec<Prim>, a: Vec2, b: Vec2, on: f32, off: f32, color: Color, width: f32) {
    let len = a.distance(b);
    if len < 0.5 {
        return;
    }
    let dir = (b - a) / len;
    let mut t = 0.0;
    while t < len {
        let end = (t + on).min(len);
        out.push(Prim::Line { a: a + dir * t, b: a + dir * end, color, width });
        t += on + off;
    }
}

impl BuildingMap {
    fn shown_floors(&self, view: &MapView) -> Vec<usize> {
        match view.floor {
            Some(i) => vec![i.min(self.floors.len() - 1)],
            None => (0..self.floors.len()).collect(),
        }
    }

    /// The metres a floor needs: its rooms and the drawn waypoints of its links, padded.
    fn floor_bounds(&self, floor: usize) -> Rect {
        let mut pts = Vec::new();
        for r in self.rooms.iter().filter(|r| r.floor == floor) {
            pts.extend(r.poly.iter().copied());
        }
        for l in &self.links {
            if self.rooms[l.a].floor == floor && self.rooms[l.b].floor == floor {
                pts.extend(l.via.iter().copied());
            }
        }
        let b = bounds_of(&pts);
        Rect::new(b.x - FLOOR_PAD_M, b.y - FLOOR_PAD_M, b.w + 2.0 * FLOOR_PAD_M, b.h + 2.0 * FLOOR_PAD_M)
    }

    /// Packs the shown floors into rows and columns inside `content` at the largest common scale.
    fn frames(&self, content: Rect, shown: &[usize]) -> Vec<FloorFrame> {
        let sizes: Vec<(usize, Rect)> = shown.iter().map(|f| (*f, self.floor_bounds(*f))).collect();
        let mut best: Option<(f32, usize)> = None;
        for cols in 1..=sizes.len() {
            let rows: Vec<&[(usize, Rect)]> = sizes.chunks(cols).collect();
            let mut s = MAX_SCALE;
            let mut sum_h = 0.0;
            for row in &rows {
                let w: f32 = row.iter().map(|(_, b)| b.w).sum();
                let gaps = (row.len() - 1) as f32 * FLOOR_GAP;
                s = s.min((content.w - gaps) / w);
                sum_h += row.iter().map(|(_, b)| b.h).fold(0.0, f32::max);
            }
            let label_gaps = rows.len() as f32 * FLOOR_LABEL_H + (rows.len() - 1) as f32 * FLOOR_GAP;
            s = s.min((content.h - label_gaps) / sum_h);
            if best.map_or(true, |(bs, _)| s > bs + 1e-3) {
                best = Some((s, cols));
            }
        }
        let (scale, cols) = best.unwrap_or((1.0, 1));
        let scale = scale.max(0.5);
        let rows: Vec<&[(usize, Rect)]> = sizes.chunks(cols).collect();
        let block_h: f32 = rows.iter().map(|r| r.iter().map(|(_, b)| b.h).fold(0.0, f32::max) * scale + FLOOR_LABEL_H).sum::<f32>()
            + (rows.len() - 1) as f32 * FLOOR_GAP;
        let mut y = content.y + ((content.h - block_h) * 0.5).max(0.0);
        let mut out = Vec::new();
        for row in rows {
            let row_w: f32 = row.iter().map(|(_, b)| b.w * scale).sum::<f32>() + (row.len() - 1) as f32 * FLOOR_GAP;
            let row_h = row.iter().map(|(_, b)| b.h).fold(0.0, f32::max) * scale;
            let mut x = content.x + ((content.w - row_w) * 0.5).max(0.0);
            for (floor, b) in row {
                out.push(FloorFrame {
                    floor: *floor,
                    min: vec2(b.x, b.y),
                    scale,
                    frame: Rect::new(x, y + FLOOR_LABEL_H, b.w * scale, b.h * scale),
                });
                x += b.w * scale + FLOOR_GAP;
            }
            y += row_h + FLOOR_LABEL_H + FLOOR_GAP;
        }
        out
    }

    fn content_rect(&self, area: Rect, view: &MapView) -> Rect {
        let legend = if view.legend { LEGEND_H } else { 0.0 };
        Rect::new(area.x + MARGIN, area.y + TITLE_H, area.w - 2.0 * MARGIN, (area.h - TITLE_H - legend - MARGIN).max(1.0))
    }

    /// The room under the screen point `pos`, for clicking on the plan.
    #[allow(dead_code)] // for clicking a room on the map; the shell has no map clicks yet
    pub fn room_at_screen(&self, area: Rect, view: &MapView, pos: Vec2) -> Option<&Room> {
        let shown = self.shown_floors(view);
        for fr in self.frames(self.content_rect(area, view), &shown) {
            let world = fr.min + (pos - vec2(fr.frame.x, fr.frame.y)) / fr.scale;
            if let Some(room) = self.room_at(fr.floor, world) {
                return Some(room);
            }
        }
        None
    }

    /// The camera whose icon is under the screen point `pos`.
    pub fn camera_at_screen(&self, area: Rect, view: &MapView, pos: Vec2) -> Option<&Camera> {
        let shown = self.shown_floors(view);
        for fr in self.frames(self.content_rect(area, view), &shown) {
            for cam in &self.cameras {
                for room in cam.covers.iter().map(|i| &self.rooms[*i]).filter(|r| r.floor == fr.floor) {
                    let c = cam_icon_centre(fr.rect_px(room.bounds));
                    if (pos.x - c.x).abs() <= CAM_ICON.x * 0.5 && (pos.y - c.y).abs() <= CAM_ICON.y * 0.5 {
                        return Some(cam);
                    }
                }
            }
        }
        None
    }

    /// The whole plan as primitives in `area` (screen pixels).
    pub fn scene(&self, area: Rect, view: &MapView) -> Vec<Prim> {
        let mut out = vec![Prim::Fill { pts: rect_pts(area), color: BG }];
        let shown = self.shown_floors(view);
        self.title(&mut out, area, view);
        for fr in self.frames(self.content_rect(area, view), &shown) {
            self.floor_scene(&mut out, &fr, view);
        }
        if view.legend {
            self.legend(&mut out, Rect::new(area.x + MARGIN, area.y + area.h - LEGEND_H, area.w - 2.0 * MARGIN, LEGEND_H - 6.0));
        }
        out
    }

    fn title(&self, out: &mut Vec<Prim>, area: Rect, view: &MapView) {
        let name = format!("{} - BUILDING MAP", self.mode.replace('_', " ").to_uppercase());
        out.push(Prim::Text { at: vec2(area.x + MARGIN, area.y + 19.0), text: name, size: 18.0, color: INK });
        if view.floor.is_some() {
            // the tab strip: every floor, the shown one lit
            let shown = view.floor.map(|i| i.min(self.floors.len() - 1));
            let mut x = area.x + area.w - MARGIN;
            for (i, f) in self.floors.iter().enumerate().rev() {
                let w = text_width(&f.label, 12.0) + 14.0;
                x -= w;
                let lit = shown == Some(i);
                let tab = Rect::new(x, area.y + 5.0, w - 4.0, 20.0);
                out.push(Prim::Fill { pts: rect_pts(tab), color: if lit { with_alpha(AMBER, 0.25) } else { FRAME } });
                out.push(Prim::Outline { pts: rect_pts(tab), color: if lit { AMBER } else { DIM }, width: 1.0 });
                out.push(Prim::Text { at: vec2(tab.x + 6.0, tab.y + 14.0), text: f.label.clone(), size: 12.0, color: if lit { INK } else { DIM } });
            }
        }
    }

    fn floor_scene(&self, out: &mut Vec<Prim>, fr: &FloorFrame, view: &MapView) {
        out.push(Prim::Text { at: vec2(fr.frame.x, fr.frame.y - 5.0), text: self.floors[fr.floor].label.clone(), size: 13.0, color: AMBER });
        out.push(Prim::Outline { pts: rect_pts(fr.frame), color: FRAME, width: 1.0 });

        // links first, so the rooms cover their middle and only the connection between them shows
        for link in &self.links {
            let (ra, rb) = (&self.rooms[link.a], &self.rooms[link.b]);
            if ra.floor != fr.floor || rb.floor != fr.floor {
                continue;
            }
            let mut pts = vec![fr.px(centroid(&ra.poly))];
            pts.extend(link.via.iter().map(|p| fr.px(*p)));
            pts.push(fr.px(centroid(&rb.poly)));
            let color = link_color(link.kind);
            for pair in pts.windows(2) {
                match link.kind {
                    LinkKind::Corridor => out.push(Prim::Line { a: pair[0], b: pair[1], color: with_alpha(color, 0.55), width: (fr.scale * 1.4).clamp(3.0, 22.0) }),
                    LinkKind::Vent => dashed(out, pair[0], pair[1], 6.0, 4.0, color, 2.0),
                    LinkKind::Window => dashed(out, pair[0], pair[1], 2.0, 3.0, color, 1.5),
                    LinkKind::Path => out.push(Prim::Line { a: pair[0], b: pair[1], color: with_alpha(color, 0.8), width: 3.0 }),
                    LinkKind::Stairs => dashed(out, pair[0], pair[1], 1.5, 4.0, color, 2.5),
                    LinkKind::Door => out.push(Prim::Line { a: pair[0], b: pair[1], color: with_alpha(color, 0.7), width: 3.0 }),
                }
            }
        }

        let active: Vec<usize> = view
            .active_camera
            .and_then(|id| self.camera(id))
            .map(|c| c.covers.clone())
            .unwrap_or_default();

        for (i, room) in self.rooms.iter().enumerate().filter(|(_, r)| r.floor == fr.floor) {
            let pts: Vec<Vec2> = room.poly.iter().map(|p| fr.px(*p)).collect();
            let b = fr.rect_px(room.bounds);
            let (fill, edge, ink) = if room.live {
                (kind_fill(room.kind), dim(INK, 0.55), INK)
            } else {
                (dim(kind_fill(room.kind), 0.55), dim(INK, 0.28), DIM)
            };
            out.push(Prim::Fill { pts: pts.clone(), color: fill });
            if active.contains(&i) {
                out.push(Prim::Fill { pts: pts.clone(), color: with_alpha(AMBER, 0.14) });
            }
            let office = self.offices.contains(&i);
            let (edge, width) = if office {
                (AMBER, 2.5)
            } else if active.contains(&i) {
                (AMBER, 2.0)
            } else {
                (edge, 1.5)
            };
            out.push(Prim::Outline { pts, color: edge, width });
            label(out, &room.label, b, ink);
            if office {
                out.push(Prim::Text { at: vec2(b.x + 4.0, b.y + b.h - 4.0), text: "YOU".into(), size: 11.0, color: AMBER });
            }
        }

        // doors on the walls rooms share
        for link in self.links.iter().filter(|l| l.kind == LinkKind::Door) {
            let (a, b) = (&self.rooms[link.a], &self.rooms[link.b]);
            if let Some((mid, vertical)) = self.door_between(&a.id, &b.id) {
                let wall = if vertical { a.bounds.h.min(b.bounds.h) } else { a.bounds.w.min(b.bounds.w) };
                let half = 0.9_f32.min(wall * 0.3);
                let (p, q) = if vertical { (vec2(mid.x, mid.y - half), vec2(mid.x, mid.y + half)) } else { (vec2(mid.x - half, mid.y), vec2(mid.x + half, mid.y)) };
                out.push(Prim::Line { a: fr.px(p), b: fr.px(q), color: link_color(LinkKind::Door), width: 4.0 });
            }
        }

        // stairs and other links that leave this floor
        for (i, room) in self.rooms.iter().enumerate().filter(|(_, r)| r.floor == fr.floor) {
            let b = fr.rect_px(room.bounds);
            let mut row = 0.0;
            let mut told: Vec<usize> = Vec::new();
            for l in &self.adjacency[i] {
                let other = &self.rooms[self.other(*l, i)];
                if other.floor == fr.floor || told.contains(&other.floor) {
                    continue;
                }
                told.push(other.floor);
                let arrow = if other.floor > fr.floor { "v" } else { "^" };
                let text = format!("{arrow} {}", self.floors[other.floor].label);
                if text_width(&text, 10.0) + 8.0 <= b.w {
                    out.push(Prim::Text { at: vec2(b.x + 4.0, b.y + b.h - 6.0 - row), text, size: 10.0, color: link_color(LinkKind::Stairs) });
                    row += 11.0;
                }
            }
        }

        // cameras
        for cam in &self.cameras {
            for room in cam.covers.iter().map(|i| &self.rooms[*i]).filter(|r| r.floor == fr.floor) {
                let c = cam_icon_centre(fr.rect_px(room.bounds));
                let lit = view.active_camera == Some(cam.id);
                camera_icon(out, c, cam.id, lit);
            }
        }

        // hunters
        let mut stacked: HashMap<&str, usize> = HashMap::new();
        for (room_id, hunter) in view.hunters {
            let Some(room) = self.room(room_id).filter(|r| r.floor == fr.floor) else { continue };
            let n = stacked.entry(room.id.as_str()).or_insert(0);
            let b = fr.rect_px(room.bounds);
            let c = vec2(b.x + 12.0 + *n as f32 * 17.0, b.y + b.h * 0.5 + 12.0);
            *n += 1;
            out.push(Prim::Circle { c, r: 7.5, fill: RED, edge: Some(INK) });
            let letter: String = hunter.chars().take(1).flat_map(char::to_uppercase).collect();
            out.push(Prim::Text { at: vec2(c.x - 3.5, c.y + 4.0), text: letter, size: 12.0, color: BG });
        }
    }

    fn legend(&self, out: &mut Vec<Prim>, area: Rect) {
        #[derive(Clone, Copy)]
        enum Swatch {
            Camera,
            Active,
            Office,
            Hunter,
            Live,
            Static,
            Link(LinkKind),
        }
        let mut items = vec![(Swatch::Office, "OFFICE"), (Swatch::Hunter, "HUNTER")];
        if !self.cameras.is_empty() {
            items.push((Swatch::Camera, "CAMERA"));
            items.push((Swatch::Active, "ACTIVE CAMERA"));
            items.push((Swatch::Live, "ON A FEED"));
        }
        if self.rooms.iter().any(|r| !r.live) {
            items.push((Swatch::Static, "NO FEED"));
        }
        for kind in LinkKind::ALL {
            if self.links.iter().any(|l| l.kind == kind) {
                items.push((Swatch::Link(kind), kind.label()));
            }
        }
        let (mut x, mut y) = (area.x, area.y + 12.0);
        for (swatch, name) in items {
            let w = 40.0 + text_width(name, 11.0) + 14.0;
            if x + w > area.x + area.w {
                x = area.x;
                y += 17.0;
            }
            let c = vec2(x + 12.0, y - 4.0);
            match swatch {
                Swatch::Camera => camera_icon(out, c, 1, false),
                Swatch::Active => camera_icon(out, c, 1, true),
                Swatch::Office => {
                    let r = Rect::new(c.x - 9.0, c.y - 6.0, 18.0, 12.0);
                    out.push(Prim::Fill { pts: rect_pts(r), color: kind_fill(RoomKind::Office) });
                    out.push(Prim::Outline { pts: rect_pts(r), color: AMBER, width: 2.0 });
                }
                Swatch::Hunter => out.push(Prim::Circle { c, r: 6.0, fill: RED, edge: Some(INK) }),
                Swatch::Live | Swatch::Static => {
                    let live = matches!(swatch, Swatch::Live);
                    let r = Rect::new(c.x - 9.0, c.y - 6.0, 18.0, 12.0);
                    out.push(Prim::Fill { pts: rect_pts(r), color: if live { kind_fill(RoomKind::Room) } else { dim(kind_fill(RoomKind::Room), 0.55) } });
                    out.push(Prim::Outline { pts: rect_pts(r), color: if live { dim(INK, 0.55) } else { dim(INK, 0.28) }, width: 1.5 });
                }
                Swatch::Link(kind) => {
                    let (a, b) = (vec2(c.x - 10.0, c.y), vec2(c.x + 10.0, c.y));
                    let color = link_color(kind);
                    match kind {
                        LinkKind::Vent => dashed(out, a, b, 5.0, 3.0, color, 2.0),
                        LinkKind::Window => dashed(out, a, b, 2.0, 3.0, color, 1.5),
                        LinkKind::Stairs => dashed(out, a, b, 1.5, 4.0, color, 2.5),
                        LinkKind::Corridor => out.push(Prim::Line { a, b, color: with_alpha(color, 0.8), width: 6.0 }),
                        _ => out.push(Prim::Line { a, b, color, width: 3.0 }),
                    }
                }
            }
            out.push(Prim::Text { at: vec2(x + 32.0, y), text: name.to_string(), size: 11.0, color: DIM });
            x += w;
        }
    }

    /// Paints the plan with macroquad. Call between frames like any other draw.
    pub fn draw(&self, area: Rect, view: &MapView) {
        for prim in self.scene(area, view) {
            paint(&prim);
        }
    }

    /// The same picture as an SVG document of `width` x `height` pixels.
    #[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
    pub fn to_svg(&self, width: f32, height: f32, view: &MapView) -> String {
        let prims = self.scene(Rect::new(0.0, 0.0, width, height), view);
        let mut s = String::new();
        let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#);
        for prim in &prims {
            let _ = writeln!(s, "{}", svg_prim(prim));
        }
        s.push_str("</svg>\n");
        s
    }
}

fn camera_icon(out: &mut Vec<Prim>, c: Vec2, id: u32, lit: bool) {
    let body = Rect::new(c.x - CAM_ICON.x * 0.5, c.y - CAM_ICON.y * 0.5, CAM_ICON.x, CAM_ICON.y);
    if lit {
        out.push(Prim::Circle { c, r: CAM_ICON.x * 0.62, fill: with_alpha(AMBER, 0.22), edge: Some(AMBER) });
    }
    out.push(Prim::Fill { pts: rect_pts(body), color: if lit { AMBER } else { Color::new(0.08, 0.10, 0.09, 1.0) } });
    out.push(Prim::Outline { pts: rect_pts(body), color: if lit { INK } else { dim(INK, 0.7) }, width: 1.2 });
    out.push(Prim::Circle { c: vec2(body.x + 5.0, c.y), r: 2.2, fill: if lit { BG } else { RED }, edge: None });
    let text = id.to_string();
    out.push(Prim::Text { at: vec2(body.x + 10.0, c.y + 4.0), text, size: 11.0, color: if lit { BG } else { INK } });
}

/// Centres `text` in `b`, on one line or two, at the largest size that fits. A room too small for the smallest size gets no label.
fn label(out: &mut Vec<Prim>, text: &str, b: Rect, color: Color) {
    let avail_w = b.w - 8.0;
    let mid = text.len() / 2;
    let split = text.match_indices(' ').map(|(i, _)| i).min_by_key(|i| i.abs_diff(mid));
    let mut size = 14.0;
    while size >= 8.0 {
        let one = text_width(text, size) <= avail_w && size + 4.0 <= b.h;
        let two = split.map(|i| (text[..i].to_string(), text[i + 1..].to_string())).filter(|(l, r)| {
            text_width(l, size).max(text_width(r, size)) <= avail_w && size * 2.2 + 4.0 <= b.h
        });
        let lines: Vec<String> = if one {
            vec![text.to_string()]
        } else if let Some((l, r)) = two {
            vec![l, r]
        } else {
            size -= 1.0;
            continue;
        };
        let top = b.y + b.h * 0.5 - lines.len() as f32 * size * 0.55 + size * 0.45;
        for (n, line) in lines.iter().enumerate() {
            let w = text_width(line, size);
            out.push(Prim::Text { at: vec2(b.x + (b.w - w) * 0.5, top + n as f32 * size * 1.1), text: line.clone(), size, color });
        }
        return;
    }
}

fn paint(prim: &Prim) {
    match prim {
        Prim::Fill { pts, color } => match pts.as_slice() {
            [a, _, c, _] if pts_are_rect(pts) => draw_rectangle(a.x, a.y, c.x - a.x, c.y - a.y, *color),
            _ => {
                let mid = centroid(pts);
                for i in 0..pts.len() {
                    draw_triangle(mid, pts[i], pts[(i + 1) % pts.len()], *color);
                }
            }
        },
        Prim::Outline { pts, color, width } => {
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                draw_line(a.x, a.y, b.x, b.y, *width, *color);
            }
        }
        Prim::Line { a, b, color, width } => draw_line(a.x, a.y, b.x, b.y, *width, *color),
        Prim::Circle { c, r, fill, edge } => {
            draw_circle(c.x, c.y, *r, *fill);
            if let Some(edge) = edge {
                draw_circle_lines(c.x, c.y, *r, 1.5, *edge);
            }
        }
        Prim::Text { at, text, size, color } => {
            draw_text(text, at.x, at.y, *size, *color);
        }
    }
}

fn pts_are_rect(pts: &[Vec2]) -> bool {
    pts.len() == 4 && pts[0].y == pts[1].y && pts[1].x == pts[2].x && pts[2].y == pts[3].y && pts[3].x == pts[0].x
}

#[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
fn svg_color(c: Color) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgb({},{},{})\" fill-opacity=\"{:.2}", byte(c.r), byte(c.g), byte(c.b), c.a)
}

#[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
fn svg_stroke(c: Color) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgb({},{},{})\" stroke-opacity=\"{:.2}", byte(c.r), byte(c.g), byte(c.b), c.a)
}

#[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
fn svg_points(pts: &[Vec2]) -> String {
    pts.iter().map(|p| format!("{:.1},{:.1}", p.x, p.y)).collect::<Vec<_>>().join(" ")
}

#[allow(dead_code)] // used by the map tests and for SVG export, not by the executable
fn svg_prim(prim: &Prim) -> String {
    match prim {
        Prim::Fill { pts, color } => format!("<polygon points=\"{}\" fill=\"{}\"/>", svg_points(pts), svg_color(*color)),
        Prim::Outline { pts, color, width } => {
            format!("<polygon points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" stroke-linejoin=\"round\"/>", svg_points(pts), svg_stroke(*color))
        }
        Prim::Line { a, b, color, width } => format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\" stroke-width=\"{width}\"/>",
            a.x, a.y, b.x, b.y, svg_stroke(*color)
        ),
        Prim::Circle { c, r, fill, edge } => {
            let stroke = edge.map_or(String::new(), |e| format!(" stroke=\"{}\" stroke-width=\"1.5\"", svg_stroke(e)));
            format!("<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{r}\" fill=\"{}\"{stroke}/>", c.x, c.y, svg_color(*fill))
        }
        Prim::Text { at, text, size, color } => {
            let safe = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            format!("<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{size}\" font-family=\"monospace\" fill=\"{}\">{safe}</text>", at.x, at.y, svg_color(*color))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data;

    fn each_map(mut check: impl FnMut(&BuildingMap)) {
        for map in all() {
            check(map);
        }
    }

    #[test]
    fn every_mode_has_a_map_that_loads() {
        assert_eq!(all().len(), MODES.len());
        for mode in MODES {
            let map = load(mode).unwrap_or_else(|| panic!("no map for {mode}"));
            assert_eq!(map.mode, mode);
            assert!(!map.theme.is_empty(), "{mode} has no theme");
        }
        assert!(load("no_such_mode").is_none());
    }

    #[test]
    fn the_maps_are_whole_buildings() {
        // Classic: the eight camera rooms, both halls, the two door nodes and the office
        let classic = load("classic").unwrap();
        assert!(classic.rooms.len() >= 11);
        assert_eq!(classic.cameras.len(), 8);
        for map in all() {
            assert!(map.rooms.len() >= 6, "{} is too small to be a building map", map.mode);
        }
    }

    #[test]
    fn adjacency_is_symmetric() {
        each_map(|map| {
            for a in &map.rooms {
                for b in map.neighbours(&a.id) {
                    assert!(map.neighbours(b).contains(&a.id.as_str()), "{}: {} -> {b} has no way back", map.mode, a.id);
                    assert!(map.rooms_adjacent(&a.id, b) && map.rooms_adjacent(b, &a.id));
                }
            }
            for a in &map.rooms {
                for b in &map.rooms {
                    assert_eq!(map.rooms_adjacent(&a.id, &b.id), map.rooms_adjacent(&b.id, &a.id));
                }
            }
        });
    }

    #[test]
    fn every_camera_covers_an_existing_room() {
        each_map(|map| {
            for (n, cam) in map.cameras.iter().enumerate() {
                assert_eq!(cam.id as usize, n + 1, "{}: cameras are numbered 1..n in order", map.mode);
                assert!(!cam.covers.is_empty());
                for room in &cam.covers {
                    assert!(*room < map.rooms.len(), "{}: camera {} covers a missing room", map.mode, cam.id);
                    assert!(map.camera_for_room(&map.rooms[*room].id).is_some());
                }
            }
        });
    }

    #[test]
    fn every_link_endpoint_exists_and_waypoints_stay_near_the_floor() {
        each_map(|map| {
            for l in &map.links {
                assert!(l.a < map.rooms.len() && l.b < map.rooms.len());
                assert_ne!(l.a, l.b);
            }
        });
    }

    #[test]
    fn the_office_exists_and_the_whole_building_reaches_it() {
        each_map(|map| {
            assert!(!map.offices.is_empty());
            for o in &map.offices {
                assert!(map.is_office(&map.rooms[*o].id));
            }
            let office = map.office().id.as_str();
            for room in &map.rooms {
                // windows are not walkable (Salvage Run's yard reaches the bay through the gantries and docks)
                assert!(map.path(&room.id, office).is_some(), "{}: {} cannot reach the office", map.mode, room.id);
            }
        });
    }

    #[test]
    fn rooms_on_a_floor_never_overlap() {
        each_map(|map| {
            for (i, a) in map.rooms.iter().enumerate() {
                for b in &map.rooms[i + 1..] {
                    assert!(!a.overlaps(b), "{}: {} overlaps {}", map.mode, a.id, b.id);
                }
            }
        });
    }

    #[test]
    fn spawns_and_ids_are_sane() {
        each_map(|map| {
            for s in &map.spawns {
                assert!(s.room < map.rooms.len());
                assert!(!s.id.is_empty());
            }
            for r in &map.rooms {
                assert_eq!(map.room(&r.id).map(|x| x.id.as_str()), Some(r.id.as_str()));
                assert!(!r.label.is_empty());
            }
        });
    }

    #[test]
    fn rooms_without_a_feed_are_dimmed_and_cameraless_maps_are_all_live() {
        let classic = load("classic").unwrap();
        assert!(classic.is_live("stage"));
        assert!(!classic.is_live("doorL"));
        assert!(classic.is_live("office"));
        let bedtime = load("bedtime").unwrap();
        assert!(bedtime.rooms.iter().all(|r| r.live));
        let mixed = load("mixed_shift").unwrap();
        assert!(!mixed.is_live("ventL"));
        assert!(mixed.is_live("vent"));
    }

    #[test]
    fn classic_queries_answer_like_the_building() {
        let map = load("classic").unwrap();
        assert!(map.rooms_adjacent("stage", "dining"));
        assert!(!map.rooms_adjacent("stage", "kitchen"));
        assert!(!map.rooms_adjacent("stage", "no_such_room"));
        assert_eq!(map.camera_for_room("kitchen").map(|c| c.id), Some(6));
        assert!(map.camera_for_room("doorL").is_none());
        let mut around: Vec<&str> = map.neighbours("dining");
        around.sort_unstable();
        assert_eq!(around, ["easthall", "kitchen", "stage", "westhall"]);
        assert_eq!(map.path("stage", "office"), Some(vec!["stage", "dining", "westhall", "doorL", "office"]));
        assert_eq!(map.path("closet", "closet"), Some(vec!["closet"]));
        assert!(map.path("stage", "nowhere").is_none());
        // a path that must not use the right side
        let west_only = map.path_where("kitchen", "office", |l| l.kind != LinkKind::Corridor);
        assert!(west_only.is_none(), "both halls reach the doors by corridor");
    }

    #[test]
    fn vents_are_a_second_way_in() {
        let map = load("masked_hours").unwrap();
        let no_vents = map.path_where("crawl", "office", |l| l.kind != LinkKind::Vent).expect("the halls and the main hall");
        let any = map.path("crawl", "office").unwrap();
        assert!(any.len() < no_vents.len(), "{any:?} should be shorter than {no_vents:?}");
        assert_eq!(any, ["crawl", "ductL", "ventL", "office"]);
    }

    #[test]
    fn room_at_finds_the_room_under_a_point_on_its_floor() {
        let map = load("classic").unwrap();
        assert_eq!(map.room_at(0, vec2(0.0, 0.0)).map(|r| r.id.as_str()), Some("stage"));
        assert_eq!(map.room_at(0, vec2(-8.0, 16.0)).map(|r| r.id.as_str()), Some("closet"));
        assert!(map.room_at(0, vec2(100.0, 100.0)).is_none());
        // a polygon room: the vent hub is a trapezoid, so its cut corner is outside it
        let masked = load("masked_hours").unwrap();
        assert_eq!(masked.room_at(0, vec2(0.0, 7.0)).map(|r| r.id.as_str()), Some("crawl"));
        assert!(masked.room_at(0, vec2(11.8, 8.8)).is_none());
        // two floors: Open Air's roof is above the yard but they do not share a point
        let open = load("open_air").unwrap();
        assert_eq!(open.room_at(0, vec2(0.0, 0.0)).map(|r| r.id.as_str()), Some("roof"));
        assert_eq!(open.room_at(1, vec2(0.0, -24.0)).map(|r| r.id.as_str()), Some("yard"));
        assert!(open.room_at(1, vec2(0.0, 0.0)).is_none());
    }

    #[test]
    fn doors_sit_on_the_walls_rooms_share() {
        let map = load("masked_hours").unwrap();
        let (at, vertical) = map.door_between("ballroom", "music").expect("they share a wall");
        assert!(vertical);
        assert_eq!(at.x, -9.0);
        assert!(map.door_between("ballroom", "ductL").is_none());
        let (at, vertical) = map.door_between("hall", "ballroom").unwrap();
        assert!(!vertical);
        assert_eq!(at, vec2(0.0, -18.0));
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        let ok = r#"{"schema":"nw.map/1","mode":"t","floors":[{"id":"f"}],
            "rooms":[{"id":"a","rect":[0,0,2,2]},{"id":"b","rect":[2,0,4,2]}],
            "links":[{"a":"a","b":"b"}],"cameras":[{"id":1,"covers":["a"]}],"offices":["b"],"spawns":[]}"#;
        assert!(BuildingMap::parse(ok).is_ok());
        let bad = [
            (ok.replace("nw.map/1", "nw.map/9"), "schema"),
            (ok.replace(r#""b":"b""#, r#""b":"zz""#), "unknown room 'zz'"),
            (ok.replace(r#""covers":["a"]"#, r#""covers":["zz"]"#), "unknown room 'zz'"),
            (ok.replace(r#""offices":["b"]"#, r#""offices":["zz"]"#), "unknown room 'zz'"),
            (ok.replace(r#""offices":["b"]"#, r#""offices":[]"#), "no office"),
            (ok.replace(r#"{"id":"b","rect":[2,0,4,2]}"#, r#"{"id":"a","rect":[2,0,4,2]}"#), "listed twice"),
            (ok.replace("[0,0,2,2]", "[0,0,0,2]"), "no area"),
            (ok.replace(r#""spawns":[]"#, r#""spawns":[{"id":"h","room":"zz"}]"#), "unknown room 'zz'"),
            (ok.replace(r#"[{"a":"a","b":"b"}]"#, r#"[{"a":"a","b":"a"}]"#), "itself"),
            ("{".to_string(), "not JSON"),
        ];
        for (text, want) in bad {
            let err = BuildingMap::parse(&text).expect_err(want);
            assert!(err.contains(want), "'{err}' should mention '{want}'");
        }
    }

    // ---- the maps agree with the mode data ---------------------------------------------------------------------------

    fn assert_linked(map: &BuildingMap, a: &str, b: &str) {
        assert!(map.rooms_adjacent(a, b), "{}: the mode graph joins {a} and {b} but the map does not", map.mode);
    }

    #[test]
    fn classic_and_masked_hours_maps_keep_their_graphs_and_camera_numbers() {
        for mode in ["classic", "masked_hours"] {
            let map = load(mode).unwrap();
            let building = data::building(mode);
            for (room, next) in building["graph"].as_object().unwrap() {
                for n in next.as_array().unwrap() {
                    assert_linked(map, room, n.as_str().unwrap());
                }
            }
            for (room, info) in building["rooms"].as_object().unwrap() {
                let cam = info["cam"].as_i64().unwrap();
                assert!(map.room(room).is_some(), "{mode}: {room} is missing from the map");
                if cam > 0 {
                    assert_eq!(map.camera_for_room(room).map(|c| i64::from(c.id)), Some(cam), "{mode}: {room}");
                } else {
                    assert!(map.camera_for_room(room).is_none(), "{mode}: {room} has no camera");
                }
            }
        }
    }

    #[test]
    fn dead_air_map_keeps_its_graph_and_rooms() {
        let map = load("dead_air").unwrap();
        let b = data::building("dead_air");
        for row in b["graph"].as_array().unwrap() {
            for n in row["next"].as_array().unwrap() {
                assert_linked(map, row["room"].as_str().unwrap(), n.as_str().unwrap());
            }
        }
        for (n, room) in b["cam_rooms"].as_array().unwrap().iter().enumerate() {
            assert_eq!(map.camera_for_room(room.as_str().unwrap()).map(|c| c.id as usize), Some(n + 1));
        }
        for r in b["rooms"].as_array().unwrap() {
            let id = r["id"].as_str().unwrap();
            assert_eq!(map.room(id).map(|x| x.label.as_str()), r["label"].as_str(), "{id}");
        }
    }

    #[test]
    fn mixed_shift_map_keeps_its_routes_and_vents() {
        let map = load("mixed_shift").unwrap();
        let b = data::building("mixed_shift");
        for route in b["routes"].as_array().unwrap().iter().chain(b["vent_routes"].as_array().unwrap()) {
            let rooms: Vec<&str> = route["rooms"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
            for pair in rooms.windows(2).filter(|p| p[0] != p[1]) {
                assert_linked(map, pair[0], pair[1]);
            }
        }
        for pair in b["vent_links"].as_array().unwrap() {
            assert_linked(map, pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        }
        for (n, room) in b["cam_rooms"].as_array().unwrap().iter().enumerate() {
            assert_eq!(map.camera_for_room(room.as_str().unwrap()).map(|c| c.id as usize), Some(n + 1));
        }
    }

    #[test]
    fn facility_and_bedtime_maps_keep_their_rooms() {
        let facility = load("facility").unwrap();
        for room in data::list(data::building("facility"), "rooms") {
            let id = room["id"].as_str().unwrap();
            assert!(facility.room(id).is_some(), "facility: {id}");
            for exit in room["exits"].as_object().unwrap().values().filter_map(Value::as_str).filter(|e| !e.is_empty()) {
                assert_linked(facility, id, exit);
            }
        }
        let bedtime = load("bedtime").unwrap();
        assert!(bedtime.rooms_adjacent("bedroom", "hall") && bedtime.rooms_adjacent("bedroom", "wardrobe"));
    }

    // ---- drawing ----------------------------------------------------------------------------------------------------

    const AREA: Rect = Rect { x: 0.0, y: 0.0, w: 1280.0, h: 720.0 };

    fn texts(prims: &[Prim]) -> Vec<&str> {
        prims.iter().filter_map(|p| if let Prim::Text { text, .. } = p { Some(text.as_str()) } else { None }).collect()
    }

    #[test]
    fn every_mode_draws_inside_its_area_with_every_room_labelled() {
        each_map(|map| {
            let prims = map.scene(AREA, &MapView::default());
            for prim in &prims {
                let pts: Vec<Vec2> = match prim {
                    Prim::Fill { pts, .. } | Prim::Outline { pts, .. } => pts.clone(),
                    Prim::Line { a, b, .. } => vec![*a, *b],
                    Prim::Circle { c, .. } => vec![*c],
                    Prim::Text { at, .. } => vec![*at],
                };
                for p in pts {
                    assert!(p.x >= -30.0 && p.x <= AREA.w + 30.0 && p.y >= -30.0 && p.y <= AREA.h + 30.0, "{}: {prim:?} is off screen", map.mode);
                }
            }
            let shown = texts(&prims);
            for room in map.rooms.iter().filter(|r| r.bounds.w * r.bounds.h > 12.0) {
                let first = room.label.split(' ').next().unwrap();
                assert!(shown.iter().any(|t| t.contains(first)), "{}: no label for {}", map.mode, room.label);
            }
            for floor in &map.floors {
                assert!(shown.contains(&floor.label.as_str()), "{}: floor {} is not labelled", map.mode, floor.label);
            }
        });
    }

    #[test]
    fn markers_cameras_and_the_office_are_drawn() {
        let map = load("classic").unwrap();
        let view = MapView { active_camera: Some(4), hunters: &[("stage", "dovelle"), ("stage", "ravelle"), ("nowhere", "ghost")], ..Default::default() };
        let prims = map.scene(AREA, &view);
        let hunters = prims.iter().filter(|p| matches!(p, Prim::Circle { fill, r, .. } if *fill == RED && *r > 5.0)).count();
        assert_eq!(hunters, 2 + 1, "two on the plan and one legend swatch");
        let shown = texts(&prims);
        assert!(shown.contains(&"D") && shown.contains(&"R") && !shown.contains(&"G"));
        assert!(shown.contains(&"YOU"));
        for n in 1..=8 {
            assert!(shown.contains(&n.to_string().as_str()), "camera {n}");
        }
        // the active camera is lit: an amber filled body
        assert!(prims.iter().any(|p| matches!(p, Prim::Fill { color, .. } if *color == AMBER)));
        assert!(!texts(&map.scene(AREA, &MapView { legend: false, ..Default::default() })).contains(&"ACTIVE CAMERA"));
        assert!(shown.contains(&"ACTIVE CAMERA"));
    }

    #[test]
    fn floors_lay_out_side_by_side_or_as_a_tab() {
        let map = load("descent").unwrap();
        let frames = map.frames(map.content_rect(AREA, &MapView::default()), &[0, 1, 2, 3]);
        assert_eq!(frames.len(), 4);
        let scale = frames[0].scale;
        assert!(frames.iter().all(|f| (f.scale - scale).abs() < 1e-4), "one scale for every floor");
        for (i, a) in frames.iter().enumerate() {
            for b in &frames[i + 1..] {
                let (ra, rb) = (a.frame, b.frame);
                assert!(ra.x + ra.w <= rb.x + 0.01 || rb.x + rb.w <= ra.x + 0.01 || ra.y + ra.h <= rb.y || rb.y + rb.h <= ra.y, "floors overlap");
            }
        }
        let tab = MapView { floor: Some(2), ..Default::default() };
        let one = map.frames(map.content_rect(AREA, &tab), &[2]);
        assert!(one[0].scale > scale, "a single floor fills the area");
        let shown = map.scene(AREA, &tab);
        assert!(texts(&shown).contains(&"LANDING 3") && !texts(&shown).contains(&"LANDING 1"));
        assert!(texts(&shown).iter().any(|t| t.starts_with("v ") || t.starts_with("^ ")), "stairs out of the floor are marked");
    }

    #[test]
    fn clicks_find_rooms_and_cameras() {
        let map = load("classic").unwrap();
        let view = MapView::default();
        let frames = map.frames(map.content_rect(AREA, &view), &[0]);
        let stage = frames[0].px(vec2(0.0, 0.0));
        assert_eq!(map.room_at_screen(AREA, &view, stage).map(|r| r.id.as_str()), Some("stage"));
        assert!(map.room_at_screen(AREA, &view, vec2(2.0, 2.0)).is_none());
        let icon = cam_icon_centre(frames[0].rect_px(map.room("kitchen").unwrap().bounds));
        assert_eq!(map.camera_at_screen(AREA, &view, icon).map(|c| c.id), Some(6));
        assert!(map.camera_at_screen(AREA, &view, stage).is_none());
    }

    #[test]
    fn svg_screenshots_render_for_every_mode() {
        // NW_MAP_SVG_DIR=<folder> cargo test --bin night_watch svg_screenshots writes one <mode>.svg (and <mode>_<floor>.svg per tab) to look at
        let dir = std::env::var("NW_MAP_SVG_DIR").ok();
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).expect("the screenshot folder");
        }
        each_map(|map| {
            let hunters: Vec<(&str, &str)> = map.spawns.iter().map(|s| (map.rooms[s.room].id.as_str(), s.id.as_str())).collect();
            let active = map.cameras.first().map(|c| c.id);
            let view = MapView { active_camera: active, hunters: &hunters, ..Default::default() };
            let svg = map.to_svg(1280.0, 720.0, &view);
            assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
            assert!(svg.contains("<polygon") && svg.contains("<text"));
            assert_eq!(svg.matches("<svg").count(), 1);
            if let Some(dir) = &dir {
                std::fs::write(format!("{dir}/{}.svg", map.mode), &svg).expect("write the svg");
                if map.floors.len() > 1 {
                    for f in 0..map.floors.len() {
                        let tab = MapView { floor: Some(f), ..view_clone(&view) };
                        std::fs::write(format!("{dir}/{}_{}.svg", map.mode, map.floors[f].id), map.to_svg(1280.0, 720.0, &tab)).expect("write the svg");
                    }
                }
            }
        });
    }

    fn view_clone<'a>(v: &MapView<'a>) -> MapView<'a> {
        MapView { active_camera: v.active_camera, hunters: v.hunters, floor: v.floor, legend: v.legend }
    }
}
