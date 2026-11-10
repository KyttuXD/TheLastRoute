//! Dead Air's tables (port of lure_data.gd and the constants of lure_logic.gd). The numbers live in
//! `data/lure/tuning.json`; this file parses them by hand (no serde derive) and offers safe lookups.
//! Pure logic: no drawing, no GL.
#![allow(dead_code)]

use crate::core::{jbool, jf64, ji32, ji32s, jstr, jstrs};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const TUNING_JSON: &str = include_str!("../../data/lure/tuning.json");

/// The eight camera rooms in camera order (camera 1..8).
pub const ROOMS: [&str; 8] = ["lobby", "arcade", "party", "ballpit", "prize", "storage", "ventl", "ventr"];
/// The two vent openings in the booth, where Xivvy gets in.
pub const OFFICE_ROOMS: [&str; 2] = ["officeL", "officeR"];
/// Rooms where a hunter crawls (the vent pose).
pub const VENT_ROOMS: [&str; 4] = ["ventl", "ventr", "officeL", "officeR"];
pub const PHANTOM_IDS: [&str; 3] = ["ferryn", "briselle", "mavrine"];
pub const SYSTEM_IDS: [&str; 5] = ["audio", "cams", "vent", "heater", "fan"];

pub fn is_office(room: &str) -> bool {
    room.starts_with("office")
}

pub fn is_vent_room(room: &str) -> bool {
    VENT_ROOMS.contains(&room)
}

#[derive(Clone, Debug, Default)]
pub struct RoomInfo {
    pub id: String,
    pub label: String,
    pub short: String,
    pub pan: f32,
}

#[derive(Clone, Debug, Default)]
pub struct SystemInfo {
    pub id: String,
    pub label: String,
    pub key: String,
    pub climate: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Variety {
    pub xivvy_tempo: f64,
    pub phantom_tempo: f64,
    pub phantom_rest_spread: f64,
    pub same_side_again: f64,
    pub wake_step: f64,
    pub start_temp_spread: f64,
    pub drift_spread: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Climate {
    pub start: f64,
    pub cold: f64,
    pub hot: f64,
    pub drift_per_sec: f64,
    pub drift_per_hour: f64,
    pub heater_per_sec: f64,
    pub fan_per_sec: f64,
    pub appliance_stress: f64,
    pub cold_per_deg: f64,
    pub heat_per_deg: f64,
    pub recover_per_sec: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub phantom_pace: f64,
    pub phantom_first_ms: f64,
    pub phantom_cool_ms: f64,
    pub lure_strain_every: i32,
    pub lure_pace: f64,
    pub office_wait_ms: f64,
    pub retreat_ms: f64,
    pub lure_ms: f64,
    pub lure_cool_ms: f64,
    pub reboot_ms: f64,
    pub phantom_show_ms: f64,
    pub stress_per_seal_sec: f64,
    pub stress_broken_sec: f64,
    pub stress_decay_sec: f64,
}

#[derive(Clone, Debug, Default)]
pub struct CustomAir {
    pub seal_sec: f64,
    pub broken_sec: f64,
    pub broken_per_level: f64,
    pub appliance_stress: f64,
    pub decay_sec: f64,
}

/// Everything in `tuning.json`.
#[derive(Clone, Debug, Default)]
pub struct Tuning {
    pub cast: Vec<String>,
    pub names: Vec<(String, String)>,
    pub rooms: Vec<RoomInfo>,
    pub graph: Vec<(String, Vec<String>)>,
    pub system_of: Vec<(String, String)>,
    pub systems: Vec<SystemInfo>,
    pub variety: Variety,
    pub levels: Vec<i32>,
    pub max_active: i32,
    pub solyth_levels: Vec<i32>,
    pub wynelle_levels: Vec<i32>,
    pub climate: Climate,
    /// id -> levels per night
    pub phantom_levels: Vec<(String, Vec<i32>)>,
    /// id -> per night -> (system, chance)
    pub phantom_extra: Vec<(String, Vec<Vec<(String, f64)>>)>,
    pub t: Timing,
    pub custom_air: CustomAir,
}

fn pairs_of(v: &Value, key: &str) -> Vec<(String, Vec<String>)> {
    match v.get(key).and_then(|g| g.as_object()) {
        Some(o) => o.iter().map(|(k, a)| (k.clone(), a.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect()).unwrap_or_default())).collect(),
        None => Vec::new(),
    }
}

impl Tuning {
    pub fn parse(text: &str) -> Result<Tuning, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("lure tuning: {e}"))?;
        let mut t = Tuning::default();
        t.cast = jstrs(&v, "cast");
        if let Some(o) = v.get("names").and_then(|n| n.as_object()) {
            t.names = o.iter().map(|(k, n)| (k.clone(), n.as_str().unwrap_or("").to_string())).collect();
        }
        if let Some(a) = v.get("rooms").and_then(|r| r.as_array()) {
            for r in a {
                t.rooms.push(RoomInfo { id: jstr(r, "id"), label: jstr(r, "label"), short: jstr(r, "short"), pan: jf64(r, "pan", 0.0) as f32 });
            }
        }
        t.graph = pairs_of(&v, "graph");
        if let Some(o) = v.get("system_of").and_then(|n| n.as_object()) {
            t.system_of = o.iter().map(|(k, n)| (k.clone(), n.as_str().unwrap_or("").to_string())).collect();
        }
        if let Some(a) = v.get("systems").and_then(|r| r.as_array()) {
            for r in a {
                t.systems.push(SystemInfo { id: jstr(r, "id"), label: jstr(r, "label"), key: jstr(r, "key"), climate: jbool(r, "climate", false) });
            }
        }
        let vy = v.get("variety").cloned().unwrap_or(Value::Null);
        t.variety = Variety {
            xivvy_tempo: jf64(&vy, "xivvyTempo", 0.0),
            phantom_tempo: jf64(&vy, "phantomTempo", 0.0),
            phantom_rest_spread: jf64(&vy, "phantomRestSpread", 0.0),
            same_side_again: jf64(&vy, "sameSideAgain", 0.0),
            wake_step: jf64(&vy, "wakeStep", 0.0),
            start_temp_spread: jf64(&vy, "startTempSpread", 0.0),
            drift_spread: jf64(&vy, "driftSpread", 0.0),
        };
        t.levels = ji32s(&v, "levels");
        t.max_active = ji32(&v, "max_active", 5);
        let cl = v.get("climate_levels").cloned().unwrap_or(Value::Null);
        t.solyth_levels = ji32s(&cl, "solyth");
        t.wynelle_levels = ji32s(&cl, "wynelle");
        let c = v.get("climate").cloned().unwrap_or(Value::Null);
        t.climate = Climate {
            start: jf64(&c, "start", 70.0),
            cold: jf64(&c, "cold", 60.0),
            hot: jf64(&c, "hot", 100.0),
            drift_per_sec: jf64(&c, "driftPerSec", 0.0),
            drift_per_hour: jf64(&c, "driftPerHour", 0.0),
            heater_per_sec: jf64(&c, "heaterPerSec", 0.0),
            fan_per_sec: jf64(&c, "fanPerSec", 0.0),
            appliance_stress: jf64(&c, "applianceStress", 0.0),
            cold_per_deg: jf64(&c, "coldPerDeg", 0.0),
            heat_per_deg: jf64(&c, "heatPerDeg", 0.0),
            recover_per_sec: jf64(&c, "recoverPerSec", 0.0),
        };
        if let Some(o) = v.get("phantom_levels").and_then(|n| n.as_object()) {
            t.phantom_levels = o
                .iter()
                .map(|(k, a)| (k.clone(), a.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).map(|f| f as i32).collect()).unwrap_or_default()))
                .collect();
        }
        if let Some(o) = v.get("phantom_extra").and_then(|n| n.as_object()) {
            for (id, nights) in o {
                let mut per_night: Vec<Vec<(String, f64)>> = Vec::new();
                for n in nights.as_array().cloned().unwrap_or_default() {
                    let mut list = Vec::new();
                    for e in n.as_array().cloned().unwrap_or_default() {
                        let a = e.as_array().cloned().unwrap_or_default();
                        if a.len() >= 2 {
                            list.push((a[0].as_str().unwrap_or("").to_string(), a[1].as_f64().unwrap_or(0.0)));
                        }
                    }
                    per_night.push(list);
                }
                t.phantom_extra.push((id.clone(), per_night));
            }
        }
        let tm = v.get("t").cloned().unwrap_or(Value::Null);
        t.t = Timing {
            phantom_pace: jf64(&tm, "phantomPace", 0.3),
            phantom_first_ms: jf64(&tm, "phantomFirstMs", 4000.0),
            phantom_cool_ms: jf64(&tm, "phantomCoolMs", 10000.0),
            lure_strain_every: ji32(&tm, "lureStrainEvery", 2),
            lure_pace: jf64(&tm, "lurePace", 1.8),
            office_wait_ms: jf64(&tm, "officeWaitMs", 5200.0),
            retreat_ms: jf64(&tm, "retreatMs", 2000.0),
            lure_ms: jf64(&tm, "lureMs", 12000.0),
            lure_cool_ms: jf64(&tm, "lureCoolMs", 7000.0),
            reboot_ms: jf64(&tm, "rebootMs", 6000.0),
            phantom_show_ms: jf64(&tm, "phantomShowMs", 4000.0),
            stress_per_seal_sec: jf64(&tm, "stressPerSealSec", 1.2),
            stress_broken_sec: jf64(&tm, "stressBrokenSec", 3.0),
            stress_decay_sec: jf64(&tm, "stressDecaySec", 2.0),
        };
        let ca = v.get("custom_air").cloned().unwrap_or(Value::Null);
        t.custom_air = CustomAir {
            seal_sec: jf64(&ca, "sealSec", 1.2),
            broken_sec: jf64(&ca, "brokenSec", 2.8),
            broken_per_level: jf64(&ca, "brokenPerLevel", 0.02),
            appliance_stress: jf64(&ca, "applianceStress", 0.8),
            decay_sec: jf64(&ca, "decaySec", 2.1),
        };
        t.validate().map(|_| t)
    }

    /// Problems in the tables (the first one as an error).
    pub fn validate(&self) -> Result<(), String> {
        for id in ROOMS.iter().chain(OFFICE_ROOMS.iter()) {
            if self.room(id).is_none() {
                return Err(format!("lure tuning: room {id} has no info"));
            }
            if self.neighbours(id).is_empty() {
                return Err(format!("lure tuning: room {id} has no neighbours"));
            }
        }
        for (a, list) in &self.graph {
            for b in list {
                if !self.neighbours(b).iter().any(|x| x == a) {
                    return Err(format!("lure tuning: {a} -> {b} is not mirrored"));
                }
            }
        }
        for id in SYSTEM_IDS {
            if !self.systems.iter().any(|s| s.id == id) {
                return Err(format!("lure tuning: system {id} missing"));
            }
        }
        if self.levels.len() != 6 || self.solyth_levels.len() != 6 || self.wynelle_levels.len() != 6 {
            return Err("lure tuning: level tables need six nights".to_string());
        }
        for id in PHANTOM_IDS {
            if self.phantom_table(id).len() != 6 {
                return Err(format!("lure tuning: phantom {id} needs six nights"));
            }
            if self.system_of(id).is_none() {
                return Err(format!("lure tuning: phantom {id} breaks nothing"));
            }
        }
        if self.cast.len() != 6 {
            return Err("lure tuning: the cast is six hunters".to_string());
        }
        Ok(())
    }

    pub fn room(&self, id: &str) -> Option<&RoomInfo> {
        self.rooms.iter().find(|r| r.id == id)
    }

    pub fn label(&self, room: &str) -> String {
        self.room(room).map(|r| r.label.clone()).unwrap_or_default()
    }

    pub fn pan(&self, room: &str) -> f32 {
        self.room(room).map(|r| r.pan).unwrap_or(0.0)
    }

    pub fn neighbours(&self, room: &str) -> Vec<String> {
        self.graph.iter().find(|(k, _)| k == room).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    pub fn system(&self, id: &str) -> Option<&SystemInfo> {
        self.systems.iter().find(|s| s.id == id)
    }

    pub fn system_of(&self, hunter: &str) -> Option<&str> {
        self.system_of.iter().find(|(k, _)| k == hunter).map(|(_, v)| v.as_str())
    }

    /// The display name of a hunter ("mavrine" is "The air").
    pub fn name(&self, id: &str) -> String {
        match self.names.iter().find(|(k, _)| k == id) {
            Some((_, n)) => n.clone(),
            None => crate::core::display_name(id),
        }
    }

    /// Xivvy's level on a 0-based night.
    pub fn xivvy_level(&self, night_idx: usize) -> i32 {
        self.levels.get(night_idx.min(5)).copied().unwrap_or(0)
    }

    pub fn climate_level(&self, id: &str, night_idx: usize) -> i32 {
        let table = if id == "solyth" { &self.solyth_levels } else { &self.wynelle_levels };
        table.get(night_idx.min(5)).copied().unwrap_or(0)
    }

    pub fn phantom_table(&self, id: &str) -> &[i32] {
        self.phantom_levels.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    pub fn phantom_level(&self, id: &str, night_idx: usize) -> i32 {
        self.phantom_table(id).get(night_idx.min(5)).copied().unwrap_or(0)
    }

    /// What else a phantom's visit may take on a night: (system, chance).
    pub fn extras(&self, id: &str, night_idx: usize) -> Vec<(String, f64)> {
        self.phantom_extra
            .iter()
            .find(|(k, _)| k == id)
            .and_then(|(_, nights)| nights.get(night_idx.min(5)).cloned())
            .unwrap_or_default()
    }

    /// Who is on the floor on a level (0-based): the lure.gd `active_hunters`.
    pub fn active_hunters(&self, level_idx: usize) -> Vec<String> {
        let i = level_idx.min(5);
        let mut out = Vec::new();
        for id in &self.cast {
            let lv = if id == "xivvy" {
                self.xivvy_level(i)
            } else if id == "solyth" || id == "wynelle" {
                self.climate_level(id, i)
            } else {
                self.phantom_level(id, i)
            };
            if lv > 0 {
                out.push(id.clone());
            }
        }
        out
    }
}

/// The parsed tuning (embedded). A broken file gives empty tables, which `check_data` and the tests report.
pub fn tuning() -> &'static Tuning {
    static T: OnceLock<Tuning> = OnceLock::new();
    T.get_or_init(|| Tuning::parse(TUNING_JSON).unwrap_or_default())
}

/// Steps from every room to `target` (the original's distancesTo), breadth first over the walkable graph.
pub fn distances_to(target: &str) -> HashMap<String, i32> {
    let t = tuning();
    let mut dist: HashMap<String, i32> = HashMap::new();
    dist.insert(target.to_string(), 0);
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(target.to_string());
    while let Some(room) = queue.pop_front() {
        let d = dist.get(&room).copied().unwrap_or(0);
        for nxt in t.neighbours(&room) {
            if !dist.contains_key(&nxt) {
                dist.insert(nxt.clone(), d + 1);
                queue.push_back(nxt);
            }
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_tuning_parses_and_validates() {
        let t = Tuning::parse(TUNING_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.cast.len(), 6);
        assert_eq!(t.rooms.len(), 10);
        assert_eq!(t.t.lure_cool_ms, 7000.0);
        assert_eq!(t.t.office_wait_ms, 5200.0);
        assert_eq!(t.t.retreat_ms, 2000.0);
        assert_eq!(t.t.reboot_ms, 6000.0);
        assert_eq!(t.levels, vec![0, 3, 5, 7, 9, 11]);
        assert_eq!(tuning().max_active, 5);
    }

    #[test]
    fn distances_match_the_graph() {
        let d = distances_to("officeL");
        assert_eq!(d["officeL"], 0);
        assert_eq!(d["ventl"], 1);
        assert_eq!(d["storage"], 2);
        assert_eq!(d["ballpit"], 3);
        assert_eq!(d["lobby"], 5); // lobby - arcade - ballpit - storage - ventl - officeL
        let r = distances_to("officeR");
        assert_eq!(r["lobby"], 5);
        assert_eq!(r["ventl"], 3);
    }

    #[test]
    fn the_cast_ramps_one_hunter_a_night() {
        let t = tuning();
        // night 1: the tutorial, only Ferryn takes the cameras
        assert_eq!(t.active_hunters(0), vec!["ferryn".to_string()]);
        // night 2: Xivvy wakes
        assert_eq!(t.active_hunters(1), vec!["xivvy".to_string(), "ferryn".to_string()]);
        // night 4 adds Wynelle, night 6 the whole floor but Ferryn rests
        assert!(t.active_hunters(3).contains(&"wynelle".to_string()));
        let six = t.active_hunters(5);
        assert!(six.contains(&"solyth".to_string()) && !six.contains(&"ferryn".to_string()));
        assert!(six.len() as i32 <= t.max_active);
    }

    #[test]
    fn names_and_pans() {
        let t = tuning();
        assert_eq!(t.name("mavrine"), "The air");
        assert_eq!(t.name("xivvy"), "Xivvy");
        assert_eq!(t.pan("ventl"), -0.9);
        assert_eq!(t.pan("officeR"), 1.0);
        assert_eq!(t.label("storage"), "TRANSMITTER ROOM");
    }
}
