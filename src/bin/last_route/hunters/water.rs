//! Water for hunters: the `WaterField` the sewer water layer and the pipes are read into each frame, and the
//! per-hunter `WaterRules` (data/hunters/water.json) that turn a depth into a path cost and a speed.
//!
//! A cell is dry, shallow (the shore), wading depth or deep. A flooded cell (a pipe burst, an open sluice) counts as
//! deep. Deep water is only entered by a hunter that can swim and is chasing (or one that does not avoid it when idle);
//! a hunter that cannot wade treats wading depth like deep. Every number is data.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.hunters.water/1";
/// The embedded path; content/mod.rs skips it when it reads hunter definitions.
pub const WATER_PATH: &str = "hunters/water.json";

/// Metres of water over the bed per cell, and the cells being flooded now. Cheap to share through an `Arc`.
#[derive(Clone, Debug, Default)]
pub struct WaterField {
    depths: HashMap<(i32, i32), f32>,
    flooded: HashSet<(i32, i32)>,
    version: u64,
}

fn mix(a: i32, b: i32, c: i64, tag: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ tag;
    for v in [a as i64 as u64, b as i64 as u64, c as u64] {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    h
}

impl WaterField {
    /// Dry cells (depth <= 0) are not stored. The version is a hash of the contents, so an identical field built on
    /// the next frame keeps its version and a changed depth or flood gives a new one.
    pub fn from_depths(depths: &[((i32, i32), f32)], flooded: &[(i32, i32)]) -> WaterField {
        let mut f = WaterField::default();
        let mut version = 0u64;
        for (c, d) in depths {
            if *d > 0.0 && d.is_finite() {
                f.depths.insert(*c, *d);
            }
        }
        for c in flooded {
            f.flooded.insert(*c);
        }
        for (c, d) in &f.depths {
            version ^= mix(c.0, c.1, (*d * 1000.0).round() as i64, 1);
        }
        for c in &f.flooded {
            version ^= mix(c.0, c.1, 0, 2);
        }
        f.version = version;
        f
    }

    pub fn depth_at(&self, cell: (i32, i32)) -> f32 {
        self.depths.get(&cell).copied().unwrap_or(0.0)
    }

    pub fn flooded(&self, cell: (i32, i32)) -> bool {
        self.flooded.contains(&cell)
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn is_dry(&self) -> bool {
        self.depths.is_empty() && self.flooded.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Dry,
    Shallow,
    Wade,
    Deep,
}

/// One hunter's water rules, resolved (no nulls left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HunterWater {
    pub can_wade: bool,
    pub can_swim: bool,
    pub wade_speed_scale: f32,
    pub swim_speed_scale: f32,
    pub shallow_below_m: f32,
    pub deep_from_m: f32,
    pub path_cost_shallow: f32,
    pub path_cost_wade: f32,
    pub path_cost_deep: f32,
    pub avoids_when_idle: bool,
    pub splash_noise_per_step: f32,
}

impl HunterWater {
    pub fn zone(&self, field: &WaterField, cell: (i32, i32)) -> Zone {
        if field.flooded(cell) {
            return Zone::Deep;
        }
        let d = field.depth_at(cell);
        if d <= 0.0 {
            Zone::Dry
        } else if d < self.shallow_below_m {
            Zone::Shallow
        } else if d < self.deep_from_m {
            Zone::Wade
        } else {
            Zone::Deep
        }
    }

    fn may_swim(&self, chasing: bool) -> bool {
        self.can_swim && (chasing || !self.avoids_when_idle)
    }

    fn deep_cost(&self) -> f32 {
        if self.path_cost_deep > 0.0 {
            self.path_cost_deep
        } else {
            self.path_cost_wade
        }
    }

    /// The path cost multiplier of entering `cell`, or None when this hunter will not plan through it.
    pub fn step_factor(&self, field: &WaterField, cell: (i32, i32), chasing: bool) -> Option<f32> {
        match self.zone(field, cell) {
            Zone::Dry => Some(1.0),
            Zone::Shallow => Some(self.path_cost_shallow),
            Zone::Wade if self.can_wade => Some(self.path_cost_wade),
            Zone::Wade | Zone::Deep => self.may_swim(chasing).then(|| self.deep_cost()),
        }
    }

    /// The speed multiplier while standing in `cell`.
    pub fn speed_scale(&self, field: &WaterField, cell: (i32, i32)) -> f32 {
        match self.zone(field, cell) {
            Zone::Dry | Zone::Shallow => 1.0,
            Zone::Wade if self.can_wade => self.wade_speed_scale,
            Zone::Wade | Zone::Deep => self.swim_speed_scale,
        }
    }

    /// Splash noise for one step taken at `cell` (0 on dry ground and on the shore).
    pub fn splash_noise(&self, field: &WaterField, cell: (i32, i32)) -> f32 {
        match self.zone(field, cell) {
            Zone::Dry | Zone::Shallow => 0.0,
            _ => self.splash_noise_per_step,
        }
    }
}

impl Default for HunterWater {
    /// The embedded default row.
    fn default() -> Self {
        WaterRules::get().default
    }
}

/// What the planner reads: the field, the hunter's rules and whether it is chasing.
pub struct WaterPlan<'a> {
    pub field: &'a WaterField,
    pub rules: &'a HunterWater,
    pub chasing: bool,
}

impl WaterPlan<'_> {
    pub fn step_factor(&self, cell: (i32, i32)) -> Option<f32> {
        if self.field.is_dry() {
            return Some(1.0);
        }
        self.rules.step_factor(self.field, cell, self.chasing)
    }
}

#[derive(Clone, Debug)]
pub struct WaterRules {
    pub default: HunterWater,
    pub hunters: HashMap<String, HunterWater>,
}

const KEYS: [&str; 11] = [
    "can_wade",
    "can_swim",
    "wade_speed_scale",
    "swim_speed_scale",
    "shallow_below_m",
    "deep_from_m",
    "path_cost_shallow",
    "path_cost_wade",
    "path_cost_deep",
    "avoids_when_idle",
    "splash_noise_per_step",
];

type Row = serde_json::Map<String, serde_json::Value>;

fn merged(base: &Row, over: Option<&serde_json::Value>, who: &str) -> Result<Row, String> {
    let mut m = base.clone();
    if let Some(o) = over {
        let o = o.as_object().ok_or_else(|| format!("hunter water: \"{who}\" must be an object"))?;
        for (k, v) in o {
            m.insert(k.clone(), v.clone());
        }
    }
    for k in m.keys() {
        if !KEYS.contains(&k.as_str()) {
            return Err(format!("hunter water: \"{who}\" has an unknown key \"{k}\""));
        }
    }
    Ok(m)
}

fn resolve(m: &Row, who: &str, swim_scale: f32) -> Result<HunterWater, String> {
    let flag = |k: &str| {
        m.get(k).and_then(|v| v.as_bool()).ok_or_else(|| format!("hunter water: \"{who}\".{k} must be true or false"))
    };
    let num = |k: &str, lo: f32, hi: f32| -> Result<f32, String> {
        match m.get(k).and_then(|v| v.as_f64()) {
            Some(x) if x.is_finite() && (x as f32) >= lo && (x as f32) <= hi => Ok(x as f32),
            _ => Err(format!("hunter water: \"{who}\".{k} must be a number from {lo} to {hi}")),
        }
    };
    let scale = |k: &str| -> Result<f32, String> {
        match m.get(k) {
            Some(serde_json::Value::Null) | None => Ok(swim_scale),
            Some(_) => num(k, 0.05, 1.5),
        }
    };
    let r = HunterWater {
        can_wade: flag("can_wade")?,
        can_swim: flag("can_swim")?,
        wade_speed_scale: scale("wade_speed_scale")?,
        swim_speed_scale: scale("swim_speed_scale")?,
        shallow_below_m: num("shallow_below_m", 0.0, 20.0)?,
        deep_from_m: num("deep_from_m", 0.0, 20.0)?,
        path_cost_shallow: num("path_cost_shallow", 1.0, 100.0)?,
        path_cost_wade: num("path_cost_wade", 1.0, 100.0)?,
        path_cost_deep: num("path_cost_deep", 0.0, 100.0)?,
        avoids_when_idle: flag("avoids_when_idle")?,
        splash_noise_per_step: num("splash_noise_per_step", 0.0, 100.0)?,
    };
    if r.deep_from_m <= r.shallow_below_m {
        return Err(format!("hunter water: \"{who}\" deep_from_m must be above shallow_below_m"));
    }
    if r.path_cost_deep > 0.0 && r.path_cost_deep < 1.0 {
        return Err(format!("hunter water: \"{who}\".path_cost_deep must be 0 or at least 1"));
    }
    Ok(r)
}

impl WaterRules {
    /// Parses a table; `swim_scale` is what a null speed scale takes (the runner's wade speed scale).
    pub fn parse(text: &str, swim_scale: f32) -> Result<WaterRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("hunter water: {e}"))?;
        if v["schema"].as_str() != Some(SCHEMA) {
            return Err(format!("hunter water: schema must be \"{SCHEMA}\""));
        }
        let base = v["default"].as_object().ok_or("hunter water: the \"default\" row is missing")?;
        let default_map = merged(base, None, "default")?;
        let default = resolve(&default_map, "default", swim_scale)?;
        let mut hunters = HashMap::new();
        if let Some(h) = v.get("hunters") {
            let h = h.as_object().ok_or("hunter water: \"hunters\" must be an object")?;
            for (id, row) in h {
                let m = merged(&default_map, Some(row), id)?;
                hunters.insert(id.clone(), resolve(&m, id, swim_scale)?);
            }
        }
        Ok(WaterRules { default, hunters })
    }

    pub fn for_hunter(&self, id: &str) -> HunterWater {
        self.hunters.get(id).copied().unwrap_or(self.default)
    }

    pub fn get() -> &'static WaterRules {
        static RULES: OnceLock<WaterRules> = OnceLock::new();
        RULES.get_or_init(|| {
            let scale = crate::game::swim::rules().wade.speed_scale;
            WaterRules::parse(include_str!("../data/hunters/water.json"), scale)
                .unwrap_or_else(|e| panic!("data/hunters/water.json: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> HunterWater {
        WaterRules::get().default
    }

    #[test]
    fn the_embedded_table_loads_and_takes_the_runner_speed() {
        let r = rules();
        assert!(r.can_wade && !r.can_swim, "Standard: hunters wade and do not swim");
        assert_eq!(r.wade_speed_scale, crate::game::swim::rules().wade.speed_scale);
        assert!(r.shallow_below_m < r.deep_from_m);
        assert_eq!(WaterRules::get().for_hunter("no-such-hunter"), r);
    }

    #[test]
    fn depth_makes_zones_and_a_flood_is_deep() {
        let f = WaterField::from_depths(&[((0, 0), 0.1), ((1, 0), 0.5), ((2, 0), 1.5)], &[(3, 0)]);
        let r = rules();
        assert_eq!(r.zone(&f, (9, 9)), Zone::Dry);
        assert_eq!(r.zone(&f, (0, 0)), Zone::Shallow);
        assert_eq!(r.zone(&f, (1, 0)), Zone::Wade);
        assert_eq!(r.zone(&f, (2, 0)), Zone::Deep);
        assert_eq!(r.zone(&f, (3, 0)), Zone::Deep, "a flooded cell counts as deep");
        assert_eq!(f.depth_at((1, 0)), 0.5);
        assert!(f.flooded((3, 0)) && !f.flooded((2, 0)));
    }

    #[test]
    fn the_version_follows_the_contents() {
        let a = WaterField::from_depths(&[((0, 0), 0.5), ((1, 1), 0.2)], &[]);
        let same = WaterField::from_depths(&[((1, 1), 0.2), ((0, 0), 0.5)], &[]);
        let deeper = WaterField::from_depths(&[((0, 0), 0.6), ((1, 1), 0.2)], &[]);
        let flood = WaterField::from_depths(&[((0, 0), 0.5), ((1, 1), 0.2)], &[(4, 4)]);
        assert_eq!(a.version(), same.version());
        assert_ne!(a.version(), deeper.version());
        assert_ne!(a.version(), flood.version());
        assert_eq!(WaterField::default().version(), 0);
        assert!(WaterField::default().is_dry());
    }

    #[test]
    fn idle_hunters_avoid_deep_and_chasing_swimmers_cross_it() {
        let f = WaterField::from_depths(&[((0, 0), 0.5), ((1, 0), 2.0), ((2, 0), 0.1)], &[]);
        let mut r = rules();
        assert!(r.step_factor(&f, (1, 0), false).is_none());
        assert!(r.step_factor(&f, (1, 0), true).is_none(), "a non-swimmer never crosses deep water");
        assert!(r.step_factor(&f, (0, 0), false).unwrap() > r.step_factor(&f, (2, 0), false).unwrap());
        assert!(r.step_factor(&f, (2, 0), false).unwrap() > 1.0);
        r.can_swim = true;
        assert!(r.step_factor(&f, (1, 0), false).is_none(), "idle swimmers still avoid it");
        assert!(r.step_factor(&f, (1, 0), true).is_some());
        r.avoids_when_idle = false;
        assert!(r.step_factor(&f, (1, 0), false).is_some());
    }

    #[test]
    fn speed_scale_applies_in_water_only() {
        let f = WaterField::from_depths(&[((0, 0), 0.5)], &[]);
        let r = rules();
        assert_eq!(r.speed_scale(&f, (5, 5)), 1.0);
        assert!(r.speed_scale(&f, (0, 0)) < 1.0);
        assert!(r.splash_noise(&f, (0, 0)) > 0.0);
        assert_eq!(r.splash_noise(&f, (5, 5)), 0.0);
    }

    fn table(edit: impl FnOnce(&mut serde_json::Value)) -> Result<WaterRules, String> {
        let mut v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/water.json")).unwrap();
        edit(&mut v);
        WaterRules::parse(&v.to_string(), 0.45)
    }

    #[test]
    fn a_hunter_row_overrides_single_keys() {
        let t = table(|v| v["hunters"]["aki"] = serde_json::json!({"can_swim": true, "swim_speed_scale": 0.8})).unwrap();
        let aki = t.for_hunter("aki");
        assert!(aki.can_swim && aki.can_wade);
        assert_eq!(aki.swim_speed_scale, 0.8);
        assert_eq!(t.for_hunter("elise"), t.default);
    }

    #[test]
    fn changing_the_data_changes_the_cost() {
        let f = WaterField::from_depths(&[((0, 0), 0.5)], &[]);
        let cheap = table(|v| v["default"]["path_cost_wade"] = serde_json::json!(1.5)).unwrap();
        let dear = table(|v| v["default"]["path_cost_wade"] = serde_json::json!(9.0)).unwrap();
        assert!(cheap.default.step_factor(&f, (0, 0), false).unwrap() < dear.default.step_factor(&f, (0, 0), false).unwrap());
    }

    #[test]
    fn a_bad_table_is_refused_with_a_clear_message() {
        assert!(table(|v| v["schema"] = serde_json::json!("x")).unwrap_err().contains("schema"));
        assert!(table(|v| v["default"]["deep_from_m"] = serde_json::json!(0.1)).unwrap_err().contains("deep_from_m"));
        assert!(table(|v| v["default"]["path_cost_wade"] = serde_json::json!(0.5)).unwrap_err().contains("path_cost_wade"));
        assert!(table(|v| v["default"]["can_swim"] = serde_json::json!("yes")).unwrap_err().contains("can_swim"));
        assert!(table(|v| v["hunters"]["aki"] = serde_json::json!({"fly": true})).unwrap_err().contains("fly"));
        assert!(table(|v| {
            v.as_object_mut().unwrap().remove("default");
        })
        .unwrap_err()
        .contains("default"));
        assert!(WaterRules::parse("not json", 0.45).is_err());
    }
}
