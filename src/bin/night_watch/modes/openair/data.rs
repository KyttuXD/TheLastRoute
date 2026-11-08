//! Open Air's data: the tunables (`data/openair/tunables.json`, the numbers of openair_logic.gd and openair_data.gd), the
//! three maps (`data/openair/map.json`: spawn, stations, cameras, lights, props, tent, the camera map layout) and the nav
//! graph of each watch (`data/nav/extra_*.json`). Pure: no drawing, no GL, no window.
#![allow(dead_code)]

use crate::core::{jf64, jstr, CamLayout};
use crate::hunters::behaviour::BehaviourData;
use crate::hunters::nav::{NavKind, NavMap, NavNode};
use macroquad::prelude::{vec2, vec3, Vec3};
use serde_json::Value;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::OnceLock;

pub const TUNABLES_JSON: &str = include_str!("../../data/openair/tunables.json");
pub const MAP_JSON: &str = include_str!("../../data/openair/map.json");
pub const BEHAVIOUR_JSON: &str = include_str!("../../data/openair/behaviour.json");
pub const NAV_ROOFTOP_JSON: &str = include_str!("../../data/nav/extra_rooftop.json");
pub const NAV_BUSSTOP_JSON: &str = include_str!("../../data/nav/extra_busstop.json");
pub const NAV_CAMPFIRE_JSON: &str = include_str!("../../data/nav/extra_campfire.json");

/// The mode's id in the menu and the registry (the 2D id); the code folder is `openair`.
pub const MODE_ID: &str = "extra";
/// Who may be on the floor of any watch, in the order of the Custom Night sliders.
pub const CAST: [&str; 4] = ["ysolde", "verrine", "wynelle", "quenelle"];

// ------------------------------------------------------------------------------------------------ tunables

macro_rules! tunables {
    ($($name:ident),* $(,)?) => {
        /// Every plain number of the rules, by the name it has in the JSON file.
        #[derive(Clone, Debug, Default)]
        pub struct Nums {
            $(pub $name: f64,)*
        }

        impl Nums {
            fn read(v: &Value) -> Nums {
                Nums { $($name: v.get(stringify!($name)).and_then(|x| x.as_f64()).unwrap_or(0.0),)* }
            }
            /// Keys the file does not hold.
            fn missing(v: &Value) -> Vec<&'static str> {
                let mut out = Vec::new();
                $(if v.get(stringify!($name)).and_then(|x| x.as_f64()).is_none() { out.push(stringify!($name)); })*
                out
            }
        }
    };
}

tunables!(
    start_dist, at_ms, leave_ms, gust_ms, gust_jitter_ms, relight_ms, dark_bonus, torch_drain, torch_regen, oil_base,
    oil_per_bar, relight_oil, cold_drain, lamp_warm, lamp_fuel, chill_line, chill_per_point, chill_recover, can_every_ms,
    can_fuel, max_cans, camp_drain, log_fuel, start_logs, gather_ms, out_grace_ms, fuel_steal, camp_at_ms, push_ms,
    push_back, lost_ms, lost_back, tent_pace, back_pace, low_fire, low_fire_bonus, arrive_spread_ms, gust_first_spread,
    side_switch, mood_every_ms, mood_every_spread, mood_pace, mood_calm, weather_last_spread, can_spread, can_first_ms,
    can_first_spread, cold_spread, cold_every_ms, cold_every_spread, bus_ms,
);

#[derive(Clone, Debug)]
pub struct WatchInfo {
    pub id: String,
    pub label: String,
    pub hunter: String,
}

/// Weather of one watch: a squall or shower every so often and for a while; `mult` is how much harder it bites.
#[derive(Clone, Debug)]
pub struct WeatherDef {
    pub kind: String,
    pub every_ms: f64,
    pub lasts_ms: f64,
    pub mult: f64,
    pub first: f64,
}

#[derive(Clone, Debug)]
pub struct Tunables {
    pub n: Nums,
    pub levels: BTreeMap<String, i32>,
    pub pan: BTreeMap<String, f64>,
    pub names: BTreeMap<String, String>,
    pub watches: Vec<WatchInfo>,
    /// By watch index (0 rooftop has none).
    pub weather: [Option<WeatherDef>; 3],
    /// Keys the file lacks (empty when it is whole).
    pub missing: Vec<String>,
}

impl Tunables {
    pub fn parse(src: &str) -> Result<Tunables, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("openair tunables: {e}"))?;
        let mut levels = BTreeMap::new();
        if let Some(o) = v.get("levels").and_then(|x| x.as_object()) {
            for (k, x) in o {
                levels.insert(k.clone(), x.as_f64().unwrap_or(0.0) as i32);
            }
        }
        let mut pan = BTreeMap::new();
        if let Some(o) = v.get("pan").and_then(|x| x.as_object()) {
            for (k, x) in o {
                pan.insert(k.clone(), x.as_f64().unwrap_or(0.0));
            }
        }
        let mut names = BTreeMap::new();
        if let Some(o) = v.get("names").and_then(|x| x.as_object()) {
            for (k, x) in o {
                names.insert(k.clone(), x.as_str().unwrap_or("").to_string());
            }
        }
        let mut watches = Vec::new();
        if let Some(a) = v.get("watches").and_then(|x| x.as_array()) {
            for w in a {
                watches.push(WatchInfo { id: jstr(w, "id"), label: jstr(w, "label"), hunter: jstr(w, "hunter") });
            }
        }
        let mut weather: [Option<WeatherDef>; 3] = [None, None, None];
        for (i, slot) in weather.iter_mut().enumerate() {
            if let Some(w) = v.get("weather").and_then(|x| x.get(i.to_string())) {
                *slot = Some(WeatherDef {
                    kind: jstr(w, "kind"),
                    every_ms: jf64(w, "every_ms", 0.0),
                    lasts_ms: jf64(w, "lasts_ms", 0.0),
                    mult: jf64(w, "mult", 1.0),
                    first: jf64(w, "first", 0.0),
                });
            }
        }
        let missing = Nums::missing(&v).into_iter().map(|s| s.to_string()).collect();
        Ok(Tunables { n: Nums::read(&v), levels, pan, names, watches, weather, missing })
    }
}

fn load_tun() -> &'static Result<Tunables, String> {
    static T: OnceLock<Result<Tunables, String>> = OnceLock::new();
    T.get_or_init(|| {
        let t = Tunables::parse(TUNABLES_JSON)?;
        if !t.missing.is_empty() {
            return Err(format!("openair tunables: missing {}", t.missing.join(", ")));
        }
        Ok(t)
    })
}

/// The embedded tunables, or why they are unusable (a parse error or missing keys).
pub fn try_tun() -> Result<&'static Tunables, String> {
    load_tun().as_ref().map_err(|e| e.clone())
}

/// The embedded tunables. After a failed load this is an empty table (all zero); the mode checks `load_error()` first and
/// refuses to start, and `--check` reports the error.
pub fn tun() -> &'static Tunables {
    static EMPTY: OnceLock<Tunables> = OnceLock::new();
    match load_tun() {
        Ok(t) => t,
        Err(_) => EMPTY.get_or_init(|| Tunables {
            n: Nums::default(),
            levels: BTreeMap::new(),
            pan: BTreeMap::new(),
            names: BTreeMap::new(),
            watches: Vec::new(),
            weather: [None, None, None],
            missing: Vec::new(),
        }),
    }
}

/// The listed AI level of a hunter (a Custom Night sets its own).
pub fn default_level(id: &str) -> i32 {
    tun().levels.get(id).copied().unwrap_or(0)
}

/// Where a hunter's steps are heard, -1 left to 1 right.
pub fn pan_of(id: &str) -> f32 {
    tun().pan.get(id).copied().unwrap_or(0.0) as f32
}

pub fn name_of(id: &str) -> String {
    tun().names.get(id).cloned().unwrap_or_else(|| id.to_string())
}

/// "ROOFTOP", "BUS STOP", "CAMPFIRE".
pub fn watch_label(watch: usize) -> String {
    tun().watches.get(watch.min(2)).map(|w| w.label.clone()).unwrap_or_default()
}

/// The hunter who keeps a watch: verrine, wynelle, ysolde.
pub fn watch_hunter(watch: usize) -> &'static str {
    ["verrine", "wynelle", "ysolde"][watch.min(2)]
}

/// Who is on the floor on a watch: that watch's hunter (and, at the campfire, Ysolde, Verrine on the left and Quenelle on the right).
pub fn active_hunters(watch: usize) -> Vec<&'static str> {
    if watch.min(2) == 2 {
        vec!["ysolde", "verrine", "quenelle"]
    } else {
        vec![watch_hunter(watch)]
    }
}

/// A Custom Night's watches, in order: each whose hunter has a level above 0 (the campfire runs when Ysolde or Quenelle is on).
pub fn custom_watches(ai: &BTreeMap<String, i32>) -> Vec<usize> {
    let lvl = |id: &str| ai.get(id).copied().unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..3usize {
        let on = if i == 2 { lvl("ysolde") > 0 || lvl("quenelle") > 0 } else { lvl(watch_hunter(i)) > 0 };
        if on {
            out.push(i);
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ maps

#[derive(Clone, Debug)]
pub struct CamDef {
    pub id: String,
    pub label: String,
    /// The nav rooms this camera watches (a hunter staring into it stands in one of them).
    pub rooms: Vec<String>,
    pub pos: Vec3,
    pub target: Vec3,
    pub fov: f32,
    /// A pole under the camera, from `base_y` up.
    pub pole: bool,
    pub base_y: f32,
}

#[derive(Clone, Debug)]
pub struct StationSpec {
    pub id: String,
    /// May hold `{action}` tokens.
    pub prompt: String,
    pub pos: Vec3,
    pub focus: Vec3,
    pub radius: f32,
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub model: String,
    pub pos: Vec3,
    pub yaw_deg: f32,
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct LightDef {
    pub id: String,
    pub pos: Vec3,
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
}

#[derive(Clone, Debug)]
pub struct TentDef {
    pub x: f32,
    pub z: f32,
    pub w: f32,
    pub h: f32,
    pub inside: Vec3,
    pub outside: Vec3,
}

impl TentDef {
    /// The floor of the tent (Rect2.has_point: the far edges are outside).
    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.x && x < self.x + self.w && z >= self.z && z < self.z + self.h
    }
}

#[derive(Clone, Debug)]
pub struct WatchMap {
    pub id: String,
    pub nav_json: &'static str,
    pub spawn: Vec3,
    pub spawn_yaw: f32,
    pub spawn_pitch: f32,
    /// (min x, min z, max x, max z) the player may walk in.
    pub footprint: (f32, f32, f32, f32),
    pub ambient: [f32; 3],
    pub lights: Vec<LightDef>,
    pub stations: Vec<StationSpec>,
    pub cameras: Vec<CamDef>,
    pub layout: CamLayout,
    pub props: Vec<PropDef>,
    pub tent: Option<TentDef>,
    pub cans: Vec<Vec3>,
    /// The bus stop's road: first and last nav node of Wynelle's walk.
    pub road: Option<(String, String)>,
    /// The bus: x at the start, x at the stop, z.
    pub bus: Option<(f32, f32, f32)>,
}

impl WatchMap {
    pub fn camera(&self, id: &str) -> Option<&CamDef> {
        self.cameras.iter().find(|c| c.id == id)
    }
    pub fn station(&self, id: &str) -> Option<&StationSpec> {
        self.stations.iter().find(|s| s.id == id)
    }
    pub fn light(&self, id: &str) -> Option<&LightDef> {
        self.lights.iter().find(|l| l.id == id)
    }
    /// Camera ids in order (the numbers of the camera keys).
    pub fn camera_ids(&self) -> Vec<String> {
        self.cameras.iter().map(|c| c.id.clone()).collect()
    }
}

fn v3(v: Option<&Value>) -> Vec3 {
    let a = v.and_then(|x| x.as_array());
    let g = |i: usize| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    vec3(g(0), g(1), g(2))
}

fn c3(v: Option<&Value>) -> [f32; 3] {
    let p = v3(v);
    [p.x, p.y, p.z]
}

fn pair(v: Option<&Value>) -> (f32, f32) {
    let a = v.and_then(|x| x.as_array());
    let g = |i: usize| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    (g(0), g(1))
}

fn nav_text(name: &str) -> &'static str {
    match name {
        "rooftop" => NAV_ROOFTOP_JSON,
        "busstop" => NAV_BUSSTOP_JSON,
        _ => NAV_CAMPFIRE_JSON,
    }
}

fn parse_watch(w: &Value) -> WatchMap {
    let fp = w.get("footprint").and_then(|x| x.as_array());
    let f = |i: usize| fp.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    let mut lights = Vec::new();
    if let Some(a) = w.get("lights").and_then(|x| x.as_array()) {
        for l in a {
            lights.push(LightDef {
                id: jstr(l, "id"),
                pos: v3(l.get("pos")),
                color: c3(l.get("color")),
                energy: jf64(l, "energy", 1.0) as f32,
                range: jf64(l, "range", 8.0) as f32,
            });
        }
    }
    let mut stations = Vec::new();
    if let Some(a) = w.get("stations").and_then(|x| x.as_array()) {
        for s in a {
            stations.push(StationSpec {
                id: jstr(s, "id"),
                prompt: jstr(s, "prompt"),
                pos: v3(s.get("pos")),
                focus: v3(s.get("focus")),
                radius: jf64(s, "radius", 0.5) as f32,
            });
        }
    }
    let mut cameras = Vec::new();
    if let Some(a) = w.get("cameras").and_then(|x| x.as_array()) {
        for c in a {
            let rooms = c
                .get("rooms")
                .and_then(|x| x.as_array())
                .map(|r| r.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            cameras.push(CamDef {
                id: jstr(c, "id"),
                label: jstr(c, "label"),
                rooms,
                pos: v3(c.get("pos")),
                target: v3(c.get("target")),
                fov: jf64(c, "fov", 66.0) as f32,
                pole: c.get("pole").and_then(|x| x.as_bool()).unwrap_or(true),
                base_y: jf64(c, "base_y", 0.0) as f32,
            });
        }
    }
    let mut props = Vec::new();
    if let Some(a) = w.get("props").and_then(|x| x.as_array()) {
        for p in a {
            props.push(PropDef {
                model: jstr(p, "model"),
                pos: v3(p.get("pos")),
                yaw_deg: jf64(p, "yaw", 0.0) as f32,
                scale: jf64(p, "scale", 1.0) as f32,
            });
        }
    }
    let mut layout = CamLayout::default();
    if let Some(l) = w.get("layout") {
        if let Some(o) = l.get("rooms").and_then(|x| x.as_object()) {
            for (id, p) in o {
                let (x, z) = pair(Some(p));
                layout.rooms.push((id.clone(), vec2(x, z)));
            }
        }
        if let Some(a) = l.get("edges").and_then(|x| x.as_array()) {
            for e in a {
                if let Some(p) = e.as_array() {
                    if p.len() >= 2 {
                        layout.edges.push((p[0].as_str().unwrap_or("").to_string(), p[1].as_str().unwrap_or("").to_string()));
                    }
                }
            }
        }
        if let Some(a) = l.get("entrances").and_then(|x| x.as_array()) {
            layout.entrances = a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect();
        }
    }
    layout.cams = cameras.iter().map(|c| c.id.clone()).collect();
    layout.labels = cameras.iter().map(|c| (c.id.clone(), c.label.clone())).collect();
    let tent = w.get("tent").map(|t| {
        let zone = t.get("zone").and_then(|x| x.as_array());
        let g = |i: usize| zone.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        TentDef { x: g(0), z: g(1), w: g(2), h: g(3), inside: v3(t.get("inside")), outside: v3(t.get("outside")) }
    });
    let cans = w.get("cans").and_then(|x| x.as_array()).map(|a| a.iter().map(|p| v3(Some(p))).collect()).unwrap_or_default();
    let road = w.get("road").map(|r| (jstr(r, "from"), jstr(r, "to")));
    let bus = w.get("bus").map(|b| (jf64(b, "from_x", -40.0) as f32, jf64(b, "to_x", -3.5) as f32, jf64(b, "z", 0.0) as f32));
    WatchMap {
        id: jstr(w, "id"),
        nav_json: nav_text(&jstr(w, "nav")),
        spawn: v3(w.get("spawn")),
        spawn_yaw: jf64(w, "spawn_yaw", 0.0) as f32,
        spawn_pitch: jf64(w, "spawn_pitch", 0.0) as f32,
        footprint: (f(0), f(1), f(2), f(3)),
        ambient: c3(w.get("ambient")),
        lights,
        stations,
        cameras,
        layout,
        props,
        tent,
        cans,
        road,
        bus,
    }
}

/// Parse the map file: exactly three watches (rooftop, bus stop, campfire), or why not.
pub fn parse_maps(src: &str) -> Result<Vec<WatchMap>, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("openair map: {e}"))?;
    let list = v.get("watches").and_then(|w| w.as_array()).ok_or_else(|| "openair map: no \"watches\" list".to_string())?;
    if list.len() != 3 {
        return Err(format!("openair map: {} watches, not 3", list.len()));
    }
    Ok(list.iter().map(parse_watch).collect())
}

fn load_maps() -> &'static Result<Vec<WatchMap>, String> {
    static M: OnceLock<Result<Vec<WatchMap>, String>> = OnceLock::new();
    M.get_or_init(|| parse_maps(MAP_JSON))
}

/// The three maps (rooftop, bus stop, campfire), parsed once; empty when the file is broken (`load_error()` says why and the
/// mode refuses to start).
pub fn maps() -> &'static Vec<WatchMap> {
    static NONE: OnceLock<Vec<WatchMap>> = OnceLock::new();
    match load_maps() {
        Ok(m) => m,
        Err(_) => NONE.get_or_init(Vec::new),
    }
}

/// A map with nothing in it, handed out by `map_of` after a failed load so no caller indexes an empty list.
fn empty_map() -> &'static WatchMap {
    static E: OnceLock<WatchMap> = OnceLock::new();
    E.get_or_init(|| WatchMap {
        id: String::new(),
        nav_json: "",
        spawn: Vec3::ZERO,
        spawn_yaw: 0.0,
        spawn_pitch: 0.0,
        footprint: (0.0, 0.0, 0.0, 0.0),
        ambient: [0.0; 3],
        lights: Vec::new(),
        stations: Vec::new(),
        cameras: Vec::new(),
        layout: CamLayout::default(),
        props: Vec::new(),
        tent: None,
        cans: Vec::new(),
        road: None,
        bus: None,
    })
}

pub fn map_of(watch: usize) -> &'static WatchMap {
    let all = maps();
    if all.is_empty() {
        return empty_map();
    }
    &all[watch.min(all.len() - 1)]
}

/// The nav graph of a map: the file's nodes plus a camera node for every room a camera watches (the stare targets).
pub fn nav_for(map: &WatchMap) -> Result<NavMap, String> {
    let mut nav = NavMap::from_json(map.nav_json).map_err(|e| format!("openair nav {}: {e}", map.id))?;
    for c in &map.cameras {
        for room in &c.rooms {
            let key = format!("CAM:{room}");
            let mut node = NavNode::new(&key, NavKind::Camera, room, c.pos);
            node.cam = map.cameras.iter().position(|x| x.id == c.id).map(|i| i as i32 + 1).unwrap_or(0);
            nav.add_node(node);
        }
    }
    Ok(nav)
}

/// The nav graph of a watch, or the parse error of its file.
pub fn try_build_nav(watch: usize) -> Result<NavMap, String> {
    nav_for(map_of(watch))
}

/// The nav graph of a watch. A nav file that does not parse gives an empty graph here only because the mode has already
/// refused to start (`load_error()`); `--check` reports the error.
pub fn build_nav(watch: usize) -> NavMap {
    try_build_nav(watch).unwrap_or_else(|_| NavMap::new("extra"))
}

/// The behaviour data of this mode, or the parse error.
pub fn try_behaviour() -> Result<Rc<BehaviourData>, String> {
    BehaviourData::parse(BEHAVIOUR_JSON).map(Rc::new).map_err(|e| format!("openair behaviour: {e}"))
}

/// The behaviour data of this mode (the shared defaults with the four hunters of Open Air). A broken file gives the shared
/// defaults here only because the mode has already refused to start (`load_error()`).
pub fn behaviour() -> Rc<BehaviourData> {
    try_behaviour().unwrap_or_else(|_| BehaviourData::embedded())
}

/// Why Open Air cannot start, if so: the tunables, the map file, a nav file or the behaviour file that does not parse. The
/// mode shows this on the shift screen instead of playing.
pub fn load_error() -> Option<String> {
    load_error_of(load_tun().as_ref().err().cloned(), load_maps().as_ref().err().cloned())
}

fn load_error_of(tun: Option<String>, maps: Option<String>) -> Option<String> {
    if let Some(e) = tun.or(maps) {
        return Some(e);
    }
    for w in 0..3usize {
        if let Err(e) = try_build_nav(w) {
            return Some(e);
        }
    }
    try_behaviour().err()
}

/// A prompt with its `{action}` tokens replaced by the bound key (upper case): `key` maps an action name to its text.
pub fn fill_prompt(template: &str, key: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => {
                let action = &rest[open + 1..open + close];
                out.push_str(&key(action).to_uppercase());
                rest = &rest[open + close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunables_are_whole_and_hold_the_listed_numbers() {
        let t = tun();
        assert!(t.missing.is_empty(), "missing keys: {:?}", t.missing);
        assert_eq!(t.n.at_ms, 5200.0);
        assert_eq!(t.n.leave_ms, 450.0);
        assert_eq!(t.n.gust_ms, 22000.0);
        assert_eq!(t.n.camp_drain, 1.8);
        assert_eq!(t.n.out_grace_ms, 10000.0);
        assert_eq!(t.n.tent_pace, 3.25);
        assert_eq!(t.n.start_logs as i32, 6);
        assert_eq!(t.n.bus_ms, 18000.0);
        assert_eq!(default_level("verrine"), 9);
        assert_eq!(default_level("quenelle"), 13);
        assert_eq!(watch_label(2), "CAMPFIRE");
        assert_eq!(t.watches.len(), 3);
    }

    #[test]
    fn weather_exists_for_the_bus_stop_and_the_campfire_only() {
        let t = tun();
        assert!(t.weather[0].is_none());
        let snow = t.weather[1].as_ref().expect("bus stop weather");
        assert_eq!((snow.kind.as_str(), snow.every_ms, snow.lasts_ms, snow.mult, snow.first), ("snow", 34000.0, 9000.0, 1.8, 20000.0));
        let rain = t.weather[2].as_ref().expect("campfire weather");
        assert_eq!((rain.kind.as_str(), rain.mult), ("rain", 2.2));
    }

    #[test]
    fn the_cast_and_the_watches_follow_the_godot_tables() {
        assert_eq!(active_hunters(0), vec!["verrine"]);
        assert_eq!(active_hunters(1), vec!["wynelle"]);
        assert_eq!(active_hunters(2), vec!["ysolde", "verrine", "quenelle"]);
        let mut ai = BTreeMap::new();
        ai.insert("verrine".to_string(), 5);
        ai.insert("quenelle".to_string(), 3);
        assert_eq!(custom_watches(&ai), vec![0, 2]);
        ai.insert("wynelle".to_string(), 1);
        assert_eq!(custom_watches(&ai), vec![0, 1, 2]);
        assert!(custom_watches(&BTreeMap::new()).is_empty());
    }

    #[test]
    fn broken_files_are_errors_not_empty_data() {
        assert!(Tunables::parse("{ not json").is_err());
        assert!(parse_maps("{ not json").is_err());
        assert!(parse_maps("{}").is_err(), "no watches list");
        assert!(parse_maps("{\"watches\": []}").is_err(), "three watches are required");
        let mut broken = map_of(0).clone();
        broken.nav_json = "{ not json";
        let e = nav_for(&broken).expect_err("a nav file that does not parse");
        assert!(e.contains("openair nav"), "{e}");
        assert!(BehaviourData::parse("{ not json").is_err());
    }

    #[test]
    fn the_embedded_files_load_without_an_error() {
        assert_eq!(load_error(), None);
        assert!(try_tun().is_ok());
        assert!(try_build_nav(1).is_ok() && try_behaviour().is_ok());
        assert_eq!(maps().len(), 3);
    }

    #[test]
    fn a_load_error_is_reported_before_anything_else() {
        let e = load_error_of(Some("openair tunables: x".to_string()), Some("openair map: y".to_string()));
        assert_eq!(e.as_deref(), Some("openair tunables: x"));
        let e = load_error_of(None, Some("openair map: y".to_string()));
        assert_eq!(e.as_deref(), Some("openair map: y"));
    }

    #[test]
    fn the_empty_map_is_safe_to_index() {
        let m = empty_map();
        assert!(m.cameras.is_empty() && m.nav_json.is_empty());
    }

    #[test]
    fn three_maps_with_cameras_stations_and_props() {
        let all = maps();
        assert_eq!(all.len(), 3);
        for (i, m) in all.iter().enumerate() {
            assert!(!m.cameras.is_empty(), "watch {i} has cameras");
            assert!(m.cameras.len() <= 8, "the camera keys go to 8");
            assert!(!m.stations.is_empty() && !m.props.is_empty() && !m.lights.is_empty());
            assert_eq!(m.layout.cams.len(), m.cameras.len());
            for c in &m.layout.cams {
                assert!(m.layout.rooms.iter().any(|(r, _)| r == c), "camera {c} is a room of the layout");
            }
        }
        assert_eq!(all[0].stations.len(), 5);
        assert_eq!(all[1].cans.len(), 3);
        assert!(all[2].tent.is_some() && all[1].road.is_some() && all[1].bus.is_some());
    }

    #[test]
    fn the_tent_floor_has_the_godot_rect() {
        let t = map_of(2).tent.clone().expect("tent");
        assert!(t.contains(6.2, 2.0));
        assert!(!t.contains(3.9, 2.0));
        assert!(!t.contains(7.2, 2.0), "the far edge is outside, as Rect2.has_point");
    }

    #[test]
    fn prompts_take_the_bound_keys() {
        let s = fill_prompt("BAR THE STAIR DOOR  {nw_door_left}", &|a| if a == "nw_door_left" { "z".to_string() } else { "?".to_string() });
        assert_eq!(s, "BAR THE STAIR DOOR  Z");
        assert_eq!(fill_prompt("PLAIN", &|_| "x".to_string()), "PLAIN");
        assert_eq!(fill_prompt("A {open", &|_| "x".to_string()), "A {open");
    }

    #[test]
    fn every_nav_graph_is_valid_and_cameras_have_nodes() {
        for w in 0..3 {
            let nav = build_nav(w);
            let problems = nav.validate();
            assert!(problems.is_empty(), "watch {w}: {problems:?}");
            for c in &map_of(w).cameras {
                for room in &c.rooms {
                    assert!(nav.rooms().iter().any(|r| r.id == *room), "watch {w}: camera {} watches unknown room {room}", c.id);
                    assert!(nav.cameras(room).iter().any(|n| n.p == c.pos), "watch {w}: room {room} has the camera node");
                }
            }
        }
    }

    #[test]
    fn behaviour_data_is_valid_against_the_pose_file() {
        let b = behaviour();
        let poses = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
        assert!(b.validate(&poses).is_empty(), "{:?}", b.validate(&poses));
        for id in CAST {
            assert!(b.hunters.contains_key(id), "{id} has her own behaviour entry");
        }
    }
}
