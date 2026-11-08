//! Night Watch data: the clock constants and formulas, the nine mode menus, and the classic building, rules and cast
//! (all compiled in through `crate::assets`). JSON is read by hand through `serde_json::Value`. Pure logic.
#![allow(dead_code)]

use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const NIGHT_MS: f64 = 270_000.0;
pub const HOUR_MS: f64 = 45_000.0;
pub const TUNED_NIGHT_MS: f64 = 480_000.0;
pub const TIME_SCALE: f64 = NIGHT_MS / TUNED_NIGHT_MS; // 0.5625
pub const NIGHTS: i32 = 7; // classic
pub const MAX_AI: i32 = 20;
pub const POINTS_PER_LEVEL: i32 = 10;
pub const RATINGS: [(f64, &str); 6] = [
    (1.0, "Unbeatable!"),
    (0.8, "Perfect!"),
    (0.6, "Stupendous!"),
    (0.4, "Amazing!"),
    (0.2, "Fantastic!"),
    (0.0, "Great Job!"),
];

/// The nine mode ids, in menu order.
pub const MODE_ID_LIST: [&str; 9] = ["classic", "mask", "lure", "bedroom", "story", "descent", "dayshift", "extra", "mixed"];

// ---- small JSON readers (also used by calls.rs and save.rs) -------------------------------------------------------

pub fn jf64(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(default)
}
pub fn ji32(v: &Value, key: &str, default: i32) -> i32 {
    match v.get(key) {
        Some(x) => x.as_f64().map(|f| f as i32).unwrap_or(default),
        None => default,
    }
}
pub fn jstr(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
pub fn jbool(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(default)
}
pub fn jstrs(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}
pub fn jf64s(v: &Value, key: &str) -> Vec<f64> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_f64()).collect())
        .unwrap_or_default()
}
pub fn ji32s(v: &Value, key: &str) -> Vec<i32> {
    jf64s(v, key).into_iter().map(|f| f as i32).collect()
}

// ---- clock and level formulas -------------------------------------------------------------------------------------

/// min(5, floor(elapsed / HOUR_MS)).
pub fn hour_index(elapsed_ms: f64) -> i32 {
    ((elapsed_ms / HOUR_MS).floor() as i32).min(5)
}

/// "12 AM" for 0, else "{h} AM".
pub fn hour_label(h: i32) -> String {
    format!("{} AM", if h == 0 { 12 } else { h })
}

/// AI steps up as the night goes on: nothing before 3 AM, +1 at 3 AM, +2 from 4 AM.
pub fn ai_bump(hour: i32) -> i32 {
    if hour >= 4 {
        2
    } else if hour >= 3 {
        1
    } else {
        0
    }
}

/// A hunter at level 0 never wakes up.
pub fn effective_level(base: i32, hour: i32) -> i32 {
    if base > 0 {
        base + ai_bump(hour)
    } else {
        0
    }
}

/// How often a hunter moves: a three-second window that slides with her level.
pub fn move_window(level: i32) -> (f64, f64) {
    let lo = (13000.0 - level as f64 * 500.0).max(2500.0);
    (lo, lo + 3000.0)
}

pub fn rating_of(score: i32, max: i32) -> &'static str {
    let share = if max > 0 { score as f64 / max as f64 } else { 0.0 };
    for &(min, name) in RATINGS.iter() {
        if share >= min {
            return name;
        }
    }
    RATINGS[RATINGS.len() - 1].1
}

/// The id with a capital first letter ("dovelle" -> "Dovelle").
pub fn display_name(id: &str) -> String {
    let mut c = id.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The hunter's level on a night (classic table; the last entry is reused past the end). 0 for an unknown id.
pub fn level_for(id: &str, night: i32) -> i32 {
    match cast().iter().find(|c| c.id == id) {
        Some(c) if !c.levels.is_empty() => c.levels[(night.clamp(1, c.levels.len() as i32) - 1) as usize],
        _ => 0,
    }
}

/// Nyxie's level; 0 on Kathy's night.
pub fn nyxie_level(night: i32) -> i32 {
    if night == rules().kathy_night {
        return 0;
    }
    level_for("nyxie", night)
}

pub fn is_kathy_night(night: i32, custom: bool) -> bool {
    night == rules().kathy_night && !custom
}

/// How long the dark lasts before Nyxie reaches the office once the power is out.
pub fn blackout_ms(level: i32) -> f64 {
    (11500.0 - level as f64 * 400.0).max(4000.0)
}

/// The door light's tell: while Kathy stands at a door and its light is on, the light dips at these moments.
pub fn kathy_dip_on(elapsed_ms: f64) -> bool {
    let r = rules();
    if r.kathy_dip_cycle_ms <= 0.0 {
        return false;
    }
    let t = elapsed_ms % r.kathy_dip_cycle_ms;
    r.kathy_dips.iter().any(|d| t >= d.0 && t < d.1)
}

/// How long a shut door must stay shut on Kathy this time (2D rollHold): lo plus an exponential with mean `tail`,
/// never above hi; u is a draw in [0, 1).
pub fn roll_hold(u: f64, lo: f64, hi: f64, tail: f64) -> f64 {
    if tail <= 0.0 {
        return (lo + u * (hi - lo)).round();
    }
    (lo - tail * (1.0 - u * (1.0 - (-(hi - lo) / tail).exp())).ln()).round()
}

/// Breadth-first distance from every room to `target` along the graph.
pub fn distances_to(target: &str, graph: &BuildingGraph) -> HashMap<String, i32> {
    let mut dist: HashMap<String, i32> = HashMap::new();
    dist.insert(target.to_string(), 0);
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(target.to_string());
    while let Some(room) = queue.pop_front() {
        let d = dist[&room];
        if let Some(next) = graph.adj.get(&room) {
            for n in next {
                if !dist.contains_key(n) {
                    dist.insert(n.clone(), d + 1);
                    queue.push_back(n.clone());
                }
            }
        }
    }
    dist
}

/// Hunters on the floor on a night: Kathy alone on her night, else every floor hunter above level 0 (floor order).
pub fn active_hunters(night: i32) -> Vec<String> {
    if night == rules().kathy_night {
        return vec!["kathy".to_string()];
    }
    floor_hunters().iter().filter(|c| level_for(&c.id, night) > 0).map(|c| c.id.clone()).collect()
}

/// Index into the per-night tables (they run to the tuned nights; later nights reuse the last).
pub fn night_index(night: i32) -> usize {
    (night.clamp(1, rules().tuned_nights.max(1)) - 1) as usize
}

// ---- modes --------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub ai: Vec<(String, i32)>,
}

#[derive(Clone, Debug)]
pub struct ModeEntry {
    pub id: String,
    pub order: i32,
    pub num: i32,
    pub label: String,
    pub title: String,
    pub tag: String,
    pub blurb: String,
    pub hint: String,
    pub levels: Vec<String>,
    pub scored: usize,
    pub open_all: bool,
    pub art: Vec<String>,
    pub cast: Vec<String>,
    /// Custom Night sliders: (id, label). None for mixed.
    pub custom: Option<Vec<(String, String)>>,
    pub presets: Vec<Preset>,
    /// Extra: the `customLevel` of the menu file (the level a Custom Night plays as), if any.
    pub custom_level: Option<i32>,
    /// Extra: the `office` name of the menu file.
    pub office: String,
}

impl ModeEntry {
    /// Parse one mode file (nw.mode/1).
    pub fn parse(text: &str) -> Result<ModeEntry, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("mode json: {e}"))?;
        let m = root.get("menu").ok_or_else(|| "mode json: no menu".to_string())?;
        let id = jstr(m, "id");
        if id.is_empty() {
            return Err("mode json: menu has no id".to_string());
        }
        let custom = match m.get("custom") {
            Some(Value::Array(a)) => Some(a.iter().map(|c| (jstr(c, "id"), jstr(c, "label"))).collect()),
            _ => None,
        };
        let mut presets = Vec::new();
        if let Some(Value::Array(a)) = m.get("presets") {
            for p in a {
                let mut ai = Vec::new();
                if let Some(Value::Object(o)) = p.get("ai") {
                    for (k, v) in o {
                        ai.push((k.clone(), v.as_f64().unwrap_or(0.0) as i32));
                    }
                }
                presets.push(Preset { id: jstr(p, "id"), name: jstr(p, "name"), ai });
            }
        }
        let levels = jstrs(m, "levels");
        Ok(ModeEntry {
            order: ji32(&root, "order", 1000),
            num: ji32(m, "num", 0),
            label: jstr(m, "label"),
            title: jstr(m, "title"),
            tag: jstr(m, "tag"),
            blurb: jstr(m, "blurb"),
            hint: jstr(m, "hint"),
            scored: m.get("scored").and_then(|x| x.as_f64()).map(|f| f as usize).unwrap_or(levels.len()),
            open_all: jbool(m, "open", false),
            art: jstrs(m, "art"),
            cast: jstrs(m, "cast"),
            custom,
            presets,
            custom_level: m.get("customLevel").and_then(|x| x.as_f64()).map(|f| f as i32),
            office: jstr(m, "office"),
            levels,
            id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ModesData {
    pub modes: Vec<ModeEntry>,
}

impl ModesData {
    /// Parses the embedded blobs in `crate::assets`; a file that fails to parse is skipped (`--check` reports it).
    pub fn load() -> ModesData {
        let mut modes: Vec<ModeEntry> = MODE_ID_LIST
            .iter()
            .filter_map(|id| ModeEntry::parse(crate::assets::mode_json(id)).ok())
            .collect();
        modes.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        ModesData { modes }
    }
    pub fn get(&self, id: &str) -> Option<&ModeEntry> {
        self.modes.iter().find(|m| m.id == id)
    }
}

// ---- building -----------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct BuildingRoom {
    pub id: String,
    pub cam: i32,
    pub label: String,
    pub pan: f64,
    pub grid: (i32, i32),
    pub map: (f64, f64),
}

#[derive(Clone, Debug, Default)]
pub struct BuildingGraph {
    pub rooms: Vec<BuildingRoom>,
    pub edges: Vec<(String, String)>,
    pub adj: HashMap<String, Vec<String>>,
    /// Extra: Kathy's rooms (building paths.kathy).
    pub kathy_start: String,
    pub kathy_fade_to: Vec<String>,
    pub kathy_doors: Vec<String>,
    pub kathy_meet_rooms: Vec<String>,
}

impl BuildingGraph {
    pub fn parse(text: &str) -> Result<BuildingGraph, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("building json: {e}"))?;
        let mut g = BuildingGraph::default();
        let rooms = root.get("rooms").and_then(|r| r.as_object()).ok_or_else(|| "building json: no rooms".to_string())?;
        for (id, r) in rooms {
            let pair = |key: &str, d: (f64, f64)| -> (f64, f64) {
                match r.get(key).and_then(|a| a.as_array()) {
                    Some(a) if a.len() >= 2 => (a[0].as_f64().unwrap_or(d.0), a[1].as_f64().unwrap_or(d.1)),
                    _ => d,
                }
            };
            let grid = pair("grid", (-1.0, -1.0));
            g.rooms.push(BuildingRoom {
                id: id.clone(),
                cam: ji32(r, "cam", 0),
                label: jstr(r, "label"),
                pan: jf64(r, "pan", 0.0),
                grid: (grid.0 as i32, grid.1 as i32),
                map: pair("map", (0.0, 0.0)),
            });
        }
        if let Some(graph) = root.get("graph").and_then(|r| r.as_object()) {
            for (id, list) in graph {
                let next: Vec<String> = list
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                for n in &next {
                    let seen = g.edges.iter().any(|(a, b)| (a == id && b == n) || (a == n && b == id));
                    if !seen {
                        g.edges.push((id.clone(), n.clone()));
                    }
                }
                g.adj.insert(id.clone(), next);
            }
        }
        if let Some(k) = root.get("paths").and_then(|p| p.get("kathy")) {
            g.kathy_start = jstr(k, "start");
            g.kathy_fade_to = jstrs(k, "fade_to");
            g.kathy_doors = jstrs(k, "doors");
            g.kathy_meet_rooms = jstrs(k, "meet_rooms");
        }
        Ok(g)
    }
    pub fn room(&self, id: &str) -> Option<&BuildingRoom> {
        self.rooms.iter().find(|r| r.id == id)
    }
}

/// Room ids that have a camera, in camera order.
pub fn cam_rooms() -> Vec<String> {
    let mut v: Vec<&BuildingRoom> = building().rooms.iter().filter(|r| r.cam > 0).collect();
    v.sort_by_key(|r| r.cam);
    v.iter().map(|r| r.id.clone()).collect()
}

/// The room a camera number looks at ("" when none).
pub fn room_for_cam(cam: i32) -> String {
    building().rooms.iter().find(|r| r.cam == cam).map(|r| r.id.clone()).unwrap_or_default()
}

// ---- rules --------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Rules {
    pub night_ms: f64,
    pub tuned_night_ms: f64,
    pub nights: i32,
    pub tuned_nights: i32,
    pub door_grace_ms: Vec<f64>,
    pub night_pace: Vec<f64>,
    pub light_push_ms: Vec<f64>,
    pub door_capacity: i32,
    pub t_closed_ms: f64,
    pub focus: f64,
    pub power_base_tuned: f64,
    pub power_per_device_tuned: f64,
    pub kathy_night: i32,
    pub kathy_level: i32,
    pub kathy_pace_scale: f64,
    pub kathy_grace_scale: f64,
    pub kathy_hold_ms: Vec<f64>,
    pub kathy_hold_tail_ms: f64,
    pub kathy_hum_ms: f64,
    pub kathy_glow_period_ms: f64,
    pub kathy_glow_on_ms: f64,
    pub kathy_dip_cycle_ms: f64,
    pub kathy_dips: Vec<(f64, f64)>,
    pub kathy_miss_lock_ms: f64,
}

impl Default for Rules {
    /// The shipped classic values (used only when the embedded file fails to parse).
    fn default() -> Self {
        Rules {
            night_ms: NIGHT_MS,
            tuned_night_ms: TUNED_NIGHT_MS,
            nights: 7,
            tuned_nights: 6,
            door_grace_ms: vec![9000.0, 8200.0, 7500.0, 6900.0, 6400.0, 5800.0],
            night_pace: vec![1.3, 1.24, 1.18, 1.12, 1.06, 1.03],
            light_push_ms: vec![2000.0, 2500.0, 3000.0, 3500.0, 4000.0, 4500.0],
            door_capacity: 2,
            t_closed_ms: 450.0,
            focus: 0.75,
            power_base_tuned: 0.1,
            power_per_device_tuned: 0.07,
            kathy_night: 7,
            kathy_level: 20,
            kathy_pace_scale: 0.65,
            kathy_grace_scale: 0.45,
            kathy_hold_ms: vec![2500.0, 20000.0],
            kathy_hold_tail_ms: 4500.0,
            kathy_hum_ms: 1100.0,
            kathy_glow_period_ms: 1500.0,
            kathy_glow_on_ms: 450.0,
            kathy_dip_cycle_ms: 2600.0,
            kathy_dips: vec![(0.0, 180.0), (780.0, 900.0), (1500.0, 1760.0)],
            kathy_miss_lock_ms: 2000.0,
        }
    }
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("rules json: {e}"))?;
        let p = root.get("params").ok_or_else(|| "rules json: no params".to_string())?;
        let d = Rules::default();
        let k = p.get("kathy").cloned().unwrap_or(Value::Null);
        let dips: Vec<(f64, f64)> = p
            .get("kathy_dips")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        let x = x.as_array()?;
                        Some((x.first()?.as_f64()?, x.get(1)?.as_f64()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Rules {
            night_ms: jf64(p, "night_ms", d.night_ms),
            tuned_night_ms: jf64(p, "tuned_night_ms", d.tuned_night_ms),
            nights: ji32(p, "nights", d.nights),
            tuned_nights: ji32(p, "tuned_nights", d.tuned_nights),
            door_grace_ms: jf64s(p, "door_grace_ms"),
            night_pace: jf64s(p, "night_pace"),
            light_push_ms: jf64s(p, "light_push_ms"),
            door_capacity: ji32(p, "door_capacity", d.door_capacity),
            t_closed_ms: jf64(p, "t_closed_ms", d.t_closed_ms),
            focus: jf64(p, "focus", d.focus),
            power_base_tuned: jf64(p, "power_base_tuned", d.power_base_tuned),
            power_per_device_tuned: jf64(p, "power_per_device_tuned", d.power_per_device_tuned),
            kathy_night: ji32(p, "kathy_night", d.kathy_night),
            kathy_level: ji32(&k, "level", d.kathy_level),
            kathy_pace_scale: jf64(&k, "pace_scale", d.kathy_pace_scale),
            kathy_grace_scale: jf64(&k, "grace_scale", d.kathy_grace_scale),
            kathy_hold_ms: jf64s(&k, "hold_ms"),
            kathy_hold_tail_ms: jf64(&k, "hold_tail_ms", d.kathy_hold_tail_ms),
            kathy_hum_ms: jf64(&k, "hum_ms", d.kathy_hum_ms),
            kathy_glow_period_ms: jf64(&k, "glow_period_ms", d.kathy_glow_period_ms),
            kathy_glow_on_ms: jf64(&k, "glow_on_ms", d.kathy_glow_on_ms),
            kathy_dip_cycle_ms: jf64(p, "kathy_dip_cycle_ms", d.kathy_dip_cycle_ms),
            kathy_dips: dips,
            kathy_miss_lock_ms: jf64(p, "kathy_miss_lock_ms", d.kathy_miss_lock_ms),
        })
    }
    /// Percent per second, the eight-minute-night rate scaled to the shorter night.
    pub fn power_base(&self) -> f64 {
        self.power_base_tuned / (self.night_ms / self.tuned_night_ms)
    }
    pub fn power_per_device(&self) -> f64 {
        self.power_per_device_tuned / (self.night_ms / self.tuned_night_ms)
    }
    /// door_grace_ms for a night (index clamps to the tuned nights).
    pub fn door_grace(&self, night: i32) -> f64 {
        self.door_grace_ms.get(night_index(night)).or(self.door_grace_ms.last()).copied().unwrap_or(0.0)
    }
    pub fn pace(&self, night: i32) -> f64 {
        self.night_pace.get(night_index(night)).or(self.night_pace.last()).copied().unwrap_or(1.0)
    }
    pub fn light_push(&self, night: i32) -> f64 {
        self.light_push_ms.get(night_index(night)).or(self.light_push_ms.last()).copied().unwrap_or(0.0)
    }
}

// ---- cast ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct CastDef {
    pub id: String,
    pub floor: Option<i32>,
    pub doors: Vec<String>,
    pub start: String,
    pub retreat_to: String,
    pub push_to: String,
    pub levels: Vec<i32>,
    pub alternate: bool,
    pub only_when_cams_down: bool,
    pub freezes_when_watched: bool,
    pub power_out: bool,
    pub solo_night: bool,
    /// (partner id, min level, patience ms)
    pub combo: Option<(String, i32, f64)>,
}

pub fn parse_cast(text: &str) -> Result<Vec<CastDef>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("cast json: {e}"))?;
    let list = root.get("hunters").and_then(|h| h.as_array()).ok_or_else(|| "cast json: no hunters".to_string())?;
    let mut out = Vec::new();
    for h in list {
        let id = jstr(h, "id");
        if id.is_empty() {
            return Err("cast json: a hunter has no id".to_string());
        }
        out.push(CastDef {
            floor: h.get("floor").and_then(|x| x.as_f64()).map(|f| f as i32),
            doors: jstrs(h, "doors"),
            start: jstr(h, "start"),
            retreat_to: jstr(h, "retreat_to"),
            push_to: jstr(h, "push_to"),
            levels: ji32s(h, "levels"),
            alternate: jbool(h, "alternate", false),
            only_when_cams_down: jbool(h, "only_when_cams_down", false),
            freezes_when_watched: jbool(h, "freezes_when_watched", false),
            power_out: jbool(h, "power_out", false),
            solo_night: jbool(h, "solo_night", false),
            combo: h.get("combo").map(|c| (jstr(c, "with"), ji32(c, "min_level", 0), jf64(c, "patience_ms", 0.0))),
            id,
        });
    }
    Ok(out)
}

pub fn building() -> &'static BuildingGraph {
    static B: OnceLock<BuildingGraph> = OnceLock::new();
    B.get_or_init(|| BuildingGraph::parse(crate::assets::BUILDING_JSON).unwrap_or_default())
}

pub fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| Rules::parse(crate::assets::RULES_JSON).unwrap_or_default())
}

/// The cast in file order.
pub fn cast() -> &'static Vec<CastDef> {
    static C: OnceLock<Vec<CastDef>> = OnceLock::new();
    C.get_or_init(|| parse_cast(crate::assets::CAST_JSON).unwrap_or_default())
}

/// The hunters with a floor slot, in slot order (dovelle, ravelle, aki, ilyra): the order the floor wakes them.
pub fn floor_hunters() -> Vec<&'static CastDef> {
    let mut v: Vec<&'static CastDef> = cast().iter().filter(|c| c.floor.is_some()).collect();
    v.sort_by_key(|c| c.floor.unwrap_or(0));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_constants_agree() {
        assert_eq!(HOUR_MS * 6.0, NIGHT_MS);
        assert_eq!(TIME_SCALE, 0.5625);
        let r = rules();
        assert_eq!(r.night_ms, NIGHT_MS);
        assert_eq!(r.tuned_night_ms, TUNED_NIGHT_MS);
        assert_eq!(r.nights, NIGHTS);
    }

    #[test]
    fn hour_helpers() {
        assert_eq!(hour_index(0.0), 0);
        assert_eq!(hour_index(44_999.0), 0);
        assert_eq!(hour_index(45_000.0), 1);
        assert_eq!(hour_index(270_000.0), 5);
        assert_eq!(hour_index(900_000.0), 5);
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(3), "3 AM");
    }

    #[test]
    fn ai_steps_up_from_three_am() {
        assert_eq!((ai_bump(2), ai_bump(3), ai_bump(4), ai_bump(5)), (0, 1, 2, 2));
        assert_eq!(effective_level(5, 4), 7);
        assert_eq!(effective_level(0, 5), 0);
    }

    #[test]
    fn move_window_slides_with_the_level() {
        assert_eq!(move_window(1), (12500.0, 15500.0));
        assert_eq!(move_window(10), (8000.0, 11000.0));
        assert_eq!(move_window(20), (3000.0, 6000.0));
        assert_eq!(move_window(40), (2500.0, 5500.0));
    }

    #[test]
    fn ratings_by_share() {
        assert_eq!(rating_of(200, 200), "Unbeatable!");
        assert_eq!(rating_of(160, 200), "Perfect!");
        assert_eq!(rating_of(130, 200), "Stupendous!");
        assert_eq!(rating_of(90, 200), "Amazing!");
        assert_eq!(rating_of(50, 200), "Fantastic!");
        assert_eq!(rating_of(0, 200), "Great Job!");
        assert_eq!(rating_of(5, 0), "Great Job!");
    }

    #[test]
    fn names_are_capitalised() {
        assert_eq!(display_name("dovelle"), "Dovelle");
        assert_eq!(display_name(""), "");
    }

    #[test]
    fn classic_levels_follow_the_cast_table() {
        assert_eq!(level_for("dovelle", 1), 2);
        assert_eq!(level_for("dovelle", 6), 16);
        assert_eq!(level_for("dovelle", 7), 16);
        assert_eq!(level_for("ravelle", 1), 0);
        assert_eq!(level_for("ilyra", 4), 5);
        assert_eq!(level_for("nobody", 1), 0);
        assert_eq!(nyxie_level(1), 3);
        assert_eq!(nyxie_level(6), 13);
        assert_eq!(nyxie_level(7), 0);
    }

    #[test]
    fn kathy_owns_night_seven_unless_custom() {
        assert!(is_kathy_night(7, false));
        assert!(!is_kathy_night(7, true));
        assert!(!is_kathy_night(6, false));
        assert_eq!(active_hunters(7), vec!["kathy".to_string()]);
        assert_eq!(active_hunters(1), vec!["dovelle".to_string()]);
        assert_eq!(active_hunters(4).len(), 4);
    }

    #[test]
    fn blackout_runs_eleven_seconds_down_to_four() {
        assert_eq!(blackout_ms(1), 11100.0);
        assert_eq!(blackout_ms(0), 11500.0);
        assert_eq!(blackout_ms(30), 4000.0);
    }

    #[test]
    fn kathy_dips_follow_the_cycle() {
        assert!(kathy_dip_on(0.0));
        assert!(kathy_dip_on(100.0));
        assert!(!kathy_dip_on(300.0));
        assert!(kathy_dip_on(800.0));
        assert!(kathy_dip_on(2600.0 * 5.0 + 1600.0));
        assert!(!kathy_dip_on(2000.0));
    }

    #[test]
    fn roll_hold_stays_in_bounds() {
        for i in 0..100 {
            let u = i as f64 / 100.0;
            let h = roll_hold(u, 2500.0, 20000.0, 4500.0);
            assert!((2500.0..=20000.0).contains(&h), "{h}");
        }
        assert_eq!(roll_hold(0.0, 2500.0, 20000.0, 4500.0), 2500.0);
        assert!(roll_hold(0.999999, 2500.0, 20000.0, 4500.0) <= 20000.0);
        assert_eq!(roll_hold(0.5, 1000.0, 3000.0, 0.0), 2000.0);
        // monotone in u
        assert!(roll_hold(0.9, 2500.0, 20000.0, 4500.0) > roll_hold(0.1, 2500.0, 20000.0, 4500.0));
    }

    #[test]
    fn distances_on_the_classic_graph() {
        let g = building();
        let d = distances_to("doorL", g);
        assert_eq!(d["doorL"], 0);
        assert_eq!(d["westhall"], 1);
        assert_eq!(d["stage"], 3); // stage - backstage - westhall - doorL
        assert_eq!(d["dining"], 2);
        let d = distances_to("doorR", g);
        assert_eq!(d["kitchen"], 3);
        assert_eq!(d.len(), g.rooms.len());
    }

    #[test]
    fn building_has_eight_cameras() {
        let g = building();
        assert_eq!(g.rooms.len(), 10);
        assert_eq!(cam_rooms().len(), 8);
        assert_eq!(cam_rooms()[0], "stage");
        assert_eq!(room_for_cam(4), "westhall");
        assert_eq!(room_for_cam(99), "");
        assert_eq!(g.room("stage").map(|r| r.grid), Some((1, 0)));
        assert_eq!(g.kathy_start, "stage");
        assert_eq!(g.kathy_doors, vec!["doorL".to_string(), "doorR".to_string()]);
    }

    #[test]
    fn floor_hunters_are_in_slot_order() {
        let ids: Vec<&str> = floor_hunters().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["dovelle", "ravelle", "aki", "ilyra"]);
        let d = cast().iter().find(|c| c.id == "dovelle").unwrap();
        assert_eq!(d.combo, Some(("ravelle".to_string(), 8, 1400.0)));
        assert!(cast().iter().find(|c| c.id == "aki").unwrap().only_when_cams_down);
        assert!(cast().iter().find(|c| c.id == "ilyra").unwrap().freezes_when_watched);
        assert!(cast().iter().find(|c| c.id == "nyxie").unwrap().power_out);
        assert!(cast().iter().find(|c| c.id == "kathy").unwrap().solo_night);
    }

    #[test]
    fn rules_scale_power_to_the_short_night() {
        let r = rules();
        assert!((r.power_base() - 0.1 / 0.5625).abs() < 1e-9);
        assert_eq!(r.door_grace(1), 9000.0);
        assert_eq!(r.door_grace(7), 5800.0);
        assert_eq!(r.pace(3), 1.18);
        assert_eq!(r.light_push(6), 4500.0);
        assert_eq!(r.kathy_hold_ms, vec![2500.0, 20000.0]);
        assert_eq!(r.kathy_dips.len(), 3);
    }

    #[test]
    fn modes_load_sorted_and_complete() {
        let m = ModesData::load();
        assert_eq!(m.modes.len(), 9);
        assert_eq!(m.modes[0].id, "classic");
        let c = m.get("classic").unwrap();
        assert_eq!(c.num, 1);
        assert_eq!(c.label, "1. CLASSIC");
        assert_eq!(c.levels.len(), 7);
        assert_eq!(c.scored, 7);
        assert_eq!(c.presets.len(), 4);
        assert_eq!(c.presets[0].ai[0], ("dovelle".to_string(), 8));
        assert_eq!(c.custom.as_ref().map(|v| v.len()), Some(5));
        let x = m.get("mixed").unwrap();
        assert!(x.open_all);
        assert!(x.custom.is_none());
        assert_eq!(x.levels.len(), 5);
        assert_eq!(x.scored, 4);
        assert_eq!(m.get("extra").unwrap().scored, 3);
        for e in &m.modes {
            assert!(!e.title.is_empty() && !e.label.is_empty(), "{}", e.id);
            assert!(!e.levels.is_empty() && e.levels.len() <= 20, "{}", e.id);
        }
    }
}
