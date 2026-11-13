//! Flickering lamps (data/render/lamps.json, `lr.lamps/1`, and the `flicker_hz` / `flicker_depth` keys of a prop's
//! light): the per-lamp wobble of a lamp's energy with time, using the term the Inferno's lava glow uses
//! (`inferno_deco::flicker`: one plus a sine of `t * hz + phase`), the table of how many lit props a floor keeps, the
//! tint of a room's lights by region and room type, which level 0 rooms start dark, and the lamp cap by quality tier.

use super::shader::{Flicker, Lamp};
use crate::content::embedded;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const PATH: &str = "render/lamps.json";
const SCHEMA: &str = "lr.lamps/1";

/// The parsed `lr.lamps/1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lamps {
    budget_default: usize,
    budget_levels: HashMap<i32, usize>,
    glow_budget: usize,
    reselect_distance: f32,
    rooms: HashMap<String, (f32, f32)>,
    tint_regions: HashMap<String, Tint>,
    tint_types: HashMap<String, Tint>,
    dark_fraction: f32,
    cap_default: usize,
    caps: HashMap<String, usize>,
}

/// A room's light look: `color` multiplies the lamp's colour, `energy` its energy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub color: [f32; 3],
    pub energy: f32,
}

impl Tint {
    pub const NONE: Tint = Tint { color: [1.0; 3], energy: 1.0 };
}

/// What the file says when it is missing or does not parse: 10 lit props, no flicker, no tint, no dark rooms.
fn fallback() -> Lamps {
    Lamps {
        budget_default: 10,
        budget_levels: HashMap::new(),
        glow_budget: 4,
        reselect_distance: 8.0,
        rooms: HashMap::new(),
        tint_regions: HashMap::new(),
        tint_types: HashMap::new(),
        dark_fraction: 0.0,
        cap_default: super::shader::MAX_LAMPS,
        caps: HashMap::new(),
    }
}

impl Lamps {
    /// How many lit props a floor on `level` keeps.
    pub fn prop_budget(&self, level: i32) -> usize {
        self.budget_levels.get(&level).copied().unwrap_or(self.budget_default)
    }

    /// How many of a floor's lit props may be screen glows (a prop light marked `glow`): the rest of the budget is the
    /// real lamps'.
    pub fn prop_glow_budget(&self) -> usize {
        self.glow_budget
    }

    /// How far (metres) the runner walks before the lit props are chosen again around it.
    pub fn prop_reselect_distance(&self) -> f32 {
        self.reselect_distance
    }

    /// The (hz, depth) of the switchable room lights in `region`, if they flicker.
    pub fn room_flicker(&self, region: &str) -> Option<(f32, f32)> {
        self.rooms.get(region).copied()
    }

    /// The tint of a room's lights: its room type's row, else its region's, else none.
    pub fn room_tint(&self, region: &str, rtype: &str) -> Tint {
        self.tint_types.get(rtype).or_else(|| self.tint_regions.get(region)).copied().unwrap_or(Tint::NONE)
    }

    /// Whether the room with this id starts with its lamps off (about `dark_rooms.fraction` of rooms). The id carries the
    /// run's seed (`ROOM-<code>-<hex>`), so a run has the same dark rooms every time.
    pub fn room_starts_dark(&self, room_id: &str) -> bool {
        if self.dark_fraction <= 0.0 {
            return false;
        }
        let mut h: u64 = 0xcbf29ce484222325;
        for b in room_id.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        // splitmix64 finish, so the last bytes of an id spread over the whole range
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
        h ^= h >> 31;
        ((h >> 11) as f64 / (1u64 << 53) as f64) < self.dark_fraction as f64
    }

    /// The most lamps the shader lights with on a graphics quality tier ("LOW", "MEDIUM", "HIGH", "AUTO").
    pub fn lamp_cap(&self, quality: &str) -> usize {
        self.caps.get(quality).copied().unwrap_or(self.cap_default).clamp(1, super::shader::MAX_LAMPS)
    }

    pub fn parse(text: &str) -> Result<Lamps, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(format!("schema: expected {SCHEMA}"));
        }
        let budget = v.get("prop_budget").and_then(Value::as_object).ok_or("prop_budget: expected an object")?;
        let mut out = Lamps { budget_default: 0, ..fallback() };
        let mut have_default = false;
        for (key, n) in budget {
            let n = n.as_u64().ok_or_else(|| format!("prop_budget.{key}: expected a whole number"))? as usize;
            if key == "default" {
                out.budget_default = n;
                have_default = true;
            } else if let Some(l) = key.strip_prefix("level.").and_then(|l| l.parse::<i32>().ok()) {
                out.budget_levels.insert(l, n);
            } else {
                return Err(format!("prop_budget.{key}: expected `default` or `level.<n>`"));
            }
        }
        if !have_default {
            return Err("prop_budget.default: missing".into());
        }
        if let Some(n) = v.get("prop_glow_budget") {
            out.glow_budget = n.as_u64().ok_or("prop_glow_budget: expected a whole number")? as usize;
        }
        if let Some(d) = v.get("prop_reselect_distance") {
            out.reselect_distance = d.as_f64().filter(|d| *d > 0.0).ok_or("prop_reselect_distance: a number above 0")? as f32;
        }
        for (region, row) in v.get("room_flicker").and_then(Value::as_object).into_iter().flatten() {
            let num = |k: &str| row.get(k).and_then(Value::as_f64).filter(|n| *n >= 0.0).map(|n| n as f32);
            let (Some(hz), Some(depth)) = (num("hz"), num("depth")) else {
                return Err(format!("room_flicker.{region}: hz and depth are numbers of 0 or more"));
            };
            if depth > 1.0 {
                return Err(format!("room_flicker.{region}: depth is 0..1"));
            }
            out.rooms.insert(region.clone(), (hz, depth));
        }
        for (key, row) in v.get("room_tint").and_then(Value::as_object).into_iter().flatten() {
            let tint = parse_tint(row).map_err(|e| format!("room_tint.{key}: {e}"))?;
            if let Some(id) = key.strip_prefix("region.").filter(|id| !id.is_empty()) {
                out.tint_regions.insert(id.to_string(), tint);
            } else if let Some(id) = key.strip_prefix("type.").filter(|id| !id.is_empty()) {
                out.tint_types.insert(id.to_string(), tint);
            } else {
                return Err(format!("room_tint.{key}: expected `region.<id>` or `type.<room type>`"));
            }
        }
        if let Some(d) = v.get("dark_rooms") {
            let f = d.get("fraction").and_then(Value::as_f64).filter(|f| (0.0..=1.0).contains(f));
            out.dark_fraction = f.ok_or("dark_rooms.fraction: a number in 0..1")? as f32;
        }
        for (key, n) in v.get("lamp_cap").and_then(Value::as_object).into_iter().flatten() {
            let n = n.as_u64().filter(|n| *n >= 1).ok_or_else(|| format!("lamp_cap.{key}: expected a whole number of 1 or more"))? as usize;
            if key == "default" {
                out.cap_default = n;
            } else {
                out.caps.insert(key.clone(), n);
            }
        }
        Ok(out)
    }
}

fn parse_tint(v: &Value) -> Result<Tint, String> {
    let colour = v.get("color").and_then(Value::as_str).and_then(|s| {
        let h = s.strip_prefix('#').filter(|h| h.len() == 6 && h.is_ascii())?;
        let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|b| b as f32 / 255.0);
        Some([ch(0)?, ch(2)?, ch(4)?])
    });
    let energy = match v.get("energy") {
        None => 1.0,
        Some(e) => e.as_f64().filter(|e| *e >= 0.0).ok_or("energy: a number of 0 or more")? as f32,
    };
    Ok(Tint { color: colour.ok_or("color: expected \"#rrggbb\"")?, energy })
}

/// The table compiled into the executable. One that fails to parse is reported on stderr and the old rules stand
/// (10 lit props, no room flicker).
pub fn table() -> &'static Lamps {
    static T: OnceLock<Lamps> = OnceLock::new();
    T.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Lamps::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            fallback()
        })
    })
}

/// The start phase (0..1) for a lamp at a world place: FNV-1a over its position in centimetres, so it is the same on
/// every run and when the window moves (callers pass world cells, not window cells).
pub fn phase_at(x: f32, y: f32, z: f32) -> f32 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in [x, y, z] {
        for b in ((v * 100.0).round() as i32).to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    (h >> 11) as f32 / (1u64 << 53) as f32
}

/// A lamp's flicker from its data keys, at world place `at`. No rate or no depth is steady; depth caps at 1.
pub fn make(hz: f32, depth: f32, at: [f32; 3]) -> Flicker {
    if !(hz > 0.0 && depth > 0.0) {
        return Flicker::NONE;
    }
    Flicker { hz, depth: depth.min(1.0), phase: phase_at(at[0], at[1], at[2]) }
}

/// The energy multiplier at time `t_s`: one plus `depth` times a sine, so it stays within 1 - depth ..= 1 + depth.
pub fn factor(t_s: f32, f: Flicker) -> f32 {
    if f.hz <= 0.0 || f.depth <= 0.0 {
        return 1.0;
    }
    (1.0 + f.depth * ((t_s * f.hz + f.phase) * std::f32::consts::TAU).sin()).max(0.0)
}

/// `lamps` with each flickering lamp's energy at time `t_s`; steady while `reduced_motion` is on.
pub fn applied(lamps: &[Lamp], t_s: f32, reduced_motion: bool) -> Vec<Lamp> {
    lamps
        .iter()
        .map(|l| if reduced_motion { *l } else { Lamp { energy: l.energy * factor(t_s, l.flicker), ..*l } })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::*;

    const FILE: &str = include_str!("../data/render/lamps.json");

    fn lamp(f: Flicker) -> Lamp {
        Lamp { pos: vec3(1.0, 2.0, 3.0), range: 5.0, color: WHITE, energy: 2.0, flicker: f }
    }

    #[test]
    fn the_shipped_file_parses_and_raises_the_sewer_budget() {
        let t = Lamps::parse(FILE).unwrap();
        assert_eq!(&t, table());
        assert_eq!(t.prop_budget(0), 10);
        assert!(t.prop_budget(-1) > 10 && t.prop_budget(-2) > 10);
        assert_eq!(t.prop_budget(-3), 10);
        assert!(t.prop_glow_budget() > 0 && t.prop_glow_budget() < t.prop_budget(0), "screen glows leave the real lamps most of the budget");
        assert!(t.prop_reselect_distance() > 0.0);
        assert!(t.room_flicker("maintenance").is_some());
        assert_eq!(t.room_flicker("office"), None);
    }

    #[test]
    fn a_room_type_row_beats_its_region_row_and_an_unlisted_room_is_untinted() {
        let t = table();
        let hospital = t.room_tint("hospital", "hospitalWard");
        assert_ne!(hospital, Tint::NONE);
        assert!(hospital.color[2] > hospital.color[0], "the hospital's lights run cool");
        let theatre = t.room_tint("hospital", "operatingTheatre");
        assert!(theatre.energy > hospital.energy, "the theatre row wins over the region's");
        let (mansion, mall) = (t.room_tint("mansion", "bedroom"), t.room_tint("mall", "laundromat"));
        assert!(mansion.color[0] > mansion.color[2], "the mansion's lights run warm");
        assert_ne!(mansion, mall);
        assert_eq!(t.room_tint("nowhere", "nothing"), Tint::NONE);
        assert_eq!(t.room_tint("nowhere", "diningRoom"), t.room_tint("mansion", "diningRoom"), "a type row needs no region");
    }

    #[test]
    fn dark_rooms_are_a_stable_quarter_per_seed_and_differ_between_seeds() {
        let quarter = Lamps::parse(&FILE.replacen("\"fraction\": 0 }", "\"fraction\": 0.25 }", 1)).unwrap();
        assert!(table().dark_fraction == 0.0, "the shipped file starts no room dark");
        let t = &quarter;
        let ids = |seed: &str| (0..4000).map(|i| format!("{seed}:ROOM-BRANCH:{i}")).collect::<Vec<_>>();
        for seed in ["ROOM-ABC-0123456789abcdef0123", "ROOM-XYZ-fedcba98765432100123"] {
            let a: Vec<bool> = ids(seed).iter().map(|id| t.room_starts_dark(id)).collect();
            let b: Vec<bool> = ids(seed).iter().map(|id| t.room_starts_dark(id)).collect();
            assert_eq!(a, b, "the same run has the same dark rooms");
            let share = a.iter().filter(|d| **d).count() as f32 / a.len() as f32;
            assert!((0.21..=0.29).contains(&share), "{seed}: {share} of rooms dark");
        }
        let a: Vec<bool> = ids("ROOM-A-1").iter().map(|id| t.room_starts_dark(id)).collect();
        let b: Vec<bool> = ids("ROOM-B-2").iter().map(|id| t.room_starts_dark(id)).collect();
        assert_ne!(a, b);
        assert!(ids("ROOM-A-1").iter().all(|id| !table().room_starts_dark(id)));
    }

    #[test]
    fn the_lamp_cap_is_eight_on_the_slowest_tier_and_sixteen_otherwise() {
        let t = table();
        assert_eq!(t.lamp_cap("LOW"), 8);
        for q in ["MEDIUM", "HIGH", "AUTO", "anything"] {
            assert_eq!(t.lamp_cap(q), crate::render::shader::MAX_LAMPS, "{q}");
        }
        let big = Lamps::parse(&FILE.replace("\"LOW\": 8", "\"LOW\": 99")).unwrap();
        assert_eq!(big.lamp_cap("LOW"), crate::render::shader::MAX_LAMPS, "never above the shader's array");
        assert!(Lamps::parse(&FILE.replace("\"LOW\": 8", "\"LOW\": 0")).is_err());
    }

    #[test]
    fn bad_files_are_refused() {
        assert!(Lamps::parse(&FILE.replace("region.hospital", "floor.hospital")).is_err());
        assert!(Lamps::parse(&FILE.replace("#d4eef2", "d4eef2")).is_err());
        assert!(Lamps::parse(&FILE.replace("\"fraction\": 0 }", "\"fraction\": 2 }")).is_err());
        assert!(Lamps::parse("{}").is_err());
        assert!(Lamps::parse(&FILE.replace("lr.lamps/1", "lr.lamps/2")).is_err());
        assert!(Lamps::parse(&FILE.replace("\"default\": 10,", "")).is_err());
        assert!(Lamps::parse(&FILE.replace("\"depth\": 0.1", "\"depth\": 1.5")).is_err());
        assert!(Lamps::parse(&FILE.replace("\"level.-1\"", "\"floor.1\"")).is_err());
    }

    #[test]
    fn factor_stays_within_depth_and_matches_the_inferno_term() {
        for depth in [0.1, 0.5, 1.0] {
            let f = make(1.7, depth, [3.0, 2.0, 9.0]);
            for i in 0..2000 {
                let k = factor(i as f32 * 0.013, f);
                assert!(k >= 1.0 - depth - 1e-5 && k <= 1.0 + depth + 1e-5, "{k} outside 1 +- {depth}");
            }
        }
        let f = Flicker { hz: 1.3, depth: 0.2, phase: 0.3 };
        for i in 0..50 {
            let t = i as f32 * 0.37;
            assert!((factor(t, f) - crate::render::inferno_deco::flicker(t, 0.3, 1.3)).abs() < 1e-6);
        }
    }

    #[test]
    fn steady_lamps_never_change_and_reduced_motion_holds_all() {
        assert_eq!(make(0.0, 0.5, [0.0; 3]), Flicker::NONE);
        assert_eq!(make(2.0, 0.0, [0.0; 3]), Flicker::NONE);
        assert_eq!(factor(5.0, Flicker::NONE), 1.0);
        let f = make(2.0, 0.6, [4.0, 2.0, 4.0]);
        let lamps = [lamp(f), lamp(Flicker::NONE)];
        let moving: Vec<f32> = (0..20).map(|i| applied(&lamps, i as f32 * 0.1, false)[0].energy).collect();
        assert!(moving.windows(2).any(|w| w[0] != w[1]), "a flickering lamp moves with time");
        for i in 0..20 {
            assert_eq!(applied(&lamps, i as f32 * 0.1, false)[1].energy, 2.0);
            assert_eq!(applied(&lamps, i as f32 * 0.1, true)[0].energy, 2.0, "reduced motion holds still");
        }
    }

    #[test]
    fn phase_is_deterministic_in_unit_range_and_differs_per_place() {
        assert_eq!(phase_at(1.0, 2.0, 3.0), phase_at(1.0, 2.0, 3.0));
        let ps: Vec<f32> = (0..40).map(|i| phase_at(i as f32 * 7.0, 2.35, 11.0)).collect();
        assert!(ps.iter().all(|p| (0.0..1.0).contains(p)));
        let mut sorted = ps.clone();
        sorted.sort_by(f32::total_cmp);
        sorted.dedup();
        assert!(sorted.len() > 30, "phases spread out");
    }
}
