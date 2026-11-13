//! Quality tiers, the AUTO tier's start-up pick and the frame-time governor (data/app/quality.json, `lr.quality/1`).
//!
//! The browser build is bound by CPU and driver submission, not pixels (the 3D view is already a quarter of the window),
//! so a slow machine needs the CPU work cut: fewer lamps in the shader, a shorter cull distance, a shorter draw distance for
//! props, a smaller chunk build budget, then a smaller render target. This module owns when to do that:
//!
//! * **AUTO is picked once.** [`pick_tier`] reads the browser's report (`hardwareConcurrency`, `deviceMemory`, a
//!   mobile and a software-renderer flag; packaging/web/hw.js leaves it in localStorage) and the first seconds of play
//!   are timed ([`Measure`]): the tier is the lower of the two answers and stays for the session. A native build has no
//!   report and keeps AUTO = HIGH.
//! * **The governor** ([`Governor`], a port of Godot's quality_governor.gd) then watches a rolling frame time against the
//!   target and gives up one notch of the table when it is missed for a while, and takes one back when it has been clean for
//!   longer. It is a pure function of the frame times fed to it. It runs on AUTO only, and only while the ADAPTIVE QUALITY
//!   setting is on; LOW, MEDIUM and HIGH are fixed.
//!
//! The renderer reads the result through [`scales`] (cull, prop and stream factors), `shader::set_governor_lamp_cap` and
//! `app::targets_scaled` (the render scale); the F7 panel shows [`rows`].

use crate::content::embedded;
use serde_json::Value;
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::OnceLock;

pub const PATH: &str = "app/quality.json";
const SCHEMA: &str = "lr.quality/1";
/// The localStorage key packaging/web/hw.js writes the browser's hardware report under.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const HARDWARE_KEY: &str = "last_route.hw";

/// A graphics quality tier. Ordered, so `min` is the more cautious one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Low,
    Medium,
    High,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Low => "LOW",
            Tier::Medium => "MEDIUM",
            Tier::High => "HIGH",
        }
    }

    pub fn parse(s: &str) -> Option<Tier> {
        match s {
            "LOW" => Some(Tier::Low),
            "MEDIUM" => Some(Tier::Medium),
            "HIGH" => Some(Tier::High),
            _ => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One step of what the governor gives up: factors of the tier's own settings (see the file's `_about`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Notch {
    pub render_scale: f32,
    pub lamps: usize,
    pub cull_scale: f32,
    pub prop_scale: f32,
    pub stream_scale: f32,
}

impl Notch {
    /// Notch 0: the tier as it is.
    pub const FULL: Notch = Notch { render_scale: 1.0, lamps: 16, cull_scale: 1.0, prop_scale: 1.0, stream_scale: 1.0 };
}

/// The governor's numbers (Godot's quality_governor.gd constants, `governor` in the file).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GovernorRules {
    pub target_fps: f32,
    pub window_seconds: f64,
    pub miss_seconds: f64,
    pub miss_factor: f32,
    pub ok_factor: f32,
    pub up_seconds: f64,
    pub down_cooldown: f64,
    pub bounce_window: f64,
    pub bounce_hold: f64,
    pub bounce_max: u32,
    pub stable_forget: f64,
    pub stall_ms: f32,
}

/// One row of the hardware table: every condition it names must hold. A row without conditions never loads.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareRule {
    /// Any of these (upper case) words in the GPU name; empty means the rule does not look at the GPU.
    pub gpu_contains: Vec<String>,
    pub max_cores: Option<u32>,
    pub max_memory_gb: Option<f32>,
    pub mobile: Option<bool>,
    pub software: Option<bool>,
    pub tier: Tier,
}

/// The AUTO pick (`auto` in the file).
#[derive(Clone, Debug, PartialEq)]
pub struct AutoRules {
    pub hardware: Vec<HardwareRule>,
    pub default_tier: Tier,
    pub unknown_tier: Tier,
    pub settle_seconds: f64,
    pub measure_seconds: f64,
    pub min_frames: usize,
    pub low_above_ratio: f32,
    pub medium_above_ratio: f32,
}

/// The parsed `lr.quality/1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The render-target scale of LOW, MEDIUM and HIGH.
    pub tier_scale: [f32; 3],
    pub notches: Vec<Notch>,
    pub governor: GovernorRules,
    pub auto: AutoRules,
}

/// The render target never goes below this share of the window, whatever a table says.
pub const MIN_RENDER_SCALE: f32 = 0.25;

impl Config {
    /// What the game uses when the file is missing or does not parse: the same numbers the file ships (a test keeps them in step).
    pub fn fallback() -> Config {
        let rule = |max_cores, max_memory_gb, mobile, software, tier| HardwareRule { gpu_contains: Vec::new(), max_cores, max_memory_gb, mobile, software, tier };
        Config {
            tier_scale: [0.35, 0.40, 0.50],
            notches: vec![
                Notch::FULL,
                Notch { render_scale: 1.0, lamps: 16, cull_scale: 0.9, prop_scale: 0.85, stream_scale: 0.75 },
                Notch { render_scale: 0.84, lamps: 8, cull_scale: 0.8, prop_scale: 0.7, stream_scale: 0.6 },
                Notch { render_scale: 0.7, lamps: 4, cull_scale: 0.7, prop_scale: 0.55, stream_scale: 0.5 },
            ],
            governor: GovernorRules {
                target_fps: 60.0,
                window_seconds: 2.0,
                miss_seconds: 3.0,
                miss_factor: 1.12,
                ok_factor: 1.05,
                up_seconds: 20.0,
                down_cooldown: 4.0,
                bounce_window: 60.0,
                bounce_hold: 60.0,
                bounce_max: 4,
                stable_forget: 300.0,
                stall_ms: 250.0,
            },
            auto: AutoRules {
                hardware: vec![
                    rule(None, None, None, Some(true), Tier::Low),
                    HardwareRule {
                        gpu_contains: ["HD GRAPHICS 6000", "HD GRAPHICS 5000", "HD GRAPHICS 4000", "IRIS 5100"].map(String::from).to_vec(),
                        ..rule(None, None, None, None, Tier::Low)
                    },
                    rule(Some(2), None, None, None, Tier::Low),
                    rule(None, Some(2.0), None, None, Tier::Low),
                    rule(Some(4), None, None, None, Tier::Medium),
                    rule(None, Some(4.0), None, None, Tier::Medium),
                    rule(None, None, Some(true), None, Tier::Medium),
                ],
                default_tier: Tier::High,
                unknown_tier: Tier::High,
                settle_seconds: 1.5,
                measure_seconds: 3.0,
                min_frames: 30,
                low_above_ratio: 1.7,
                medium_above_ratio: 1.17,
            },
        }
    }

    /// The deepest notch index.
    pub fn max_notch(&self) -> usize {
        self.notches.len().saturating_sub(1)
    }

    /// The render-target scale of `tier` at `notch` (clamped to the table and to MIN_RENDER_SCALE).
    pub fn render_scale(&self, tier: Tier, notch: usize) -> f32 {
        let n = self.notches[notch.min(self.max_notch())];
        (self.tier_scale[tier.index()] * n.render_scale).max(MIN_RENDER_SCALE)
    }

    /// Parses and validates the file. Every number is range-checked and the notch table must only ever give up more.
    pub fn parse(text: &str) -> Result<Config, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(format!("schema: expected {SCHEMA}"));
        }
        let tiers = v.get("tiers").and_then(Value::as_object).ok_or("tiers: expected an object")?;
        let mut tier_scale = [0.0f32; 3];
        for t in [Tier::Low, Tier::Medium, Tier::High] {
            let row = tiers.get(t.name()).ok_or_else(|| format!("tiers.{}: missing", t.name()))?;
            tier_scale[t.index()] = number(row, "render_scale", 0.1, 1.0).map_err(|e| format!("tiers.{}: {e}", t.name()))?;
        }
        if let Some(k) = tiers.keys().find(|k| Tier::parse(k).is_none()) {
            return Err(format!("tiers.{k}: expected LOW, MEDIUM or HIGH"));
        }
        if !(tier_scale[0] <= tier_scale[1] && tier_scale[1] <= tier_scale[2]) {
            return Err("tiers: render_scale must not fall from LOW to HIGH".into());
        }

        let rows = v.get("notches").and_then(Value::as_array).ok_or("notches: expected an array")?;
        if rows.is_empty() || rows.len() > 8 {
            return Err("notches: expected 1 to 8 rows".into());
        }
        let mut notches: Vec<Notch> = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let at = |e: String| format!("notches[{i}]: {e}");
            let lamps = row
                .get("lamps")
                .and_then(Value::as_u64)
                .filter(|n| (1..=16).contains(n))
                .ok_or_else(|| at("lamps: a whole number from 1 to 16".into()))? as usize;
            let n = Notch {
                render_scale: number(row, "render_scale", 0.1, 1.0).map_err(at)?,
                lamps,
                cull_scale: number(row, "cull_scale", 0.1, 1.0).map_err(at)?,
                prop_scale: number(row, "prop_scale", 0.1, 1.0).map_err(at)?,
                stream_scale: number(row, "stream_scale", 0.1, 1.0).map_err(at)?,
            };
            if let Some(prev) = notches.last() {
                let gives_up = n.render_scale <= prev.render_scale
                    && n.lamps <= prev.lamps
                    && n.cull_scale <= prev.cull_scale
                    && n.prop_scale <= prev.prop_scale
                    && n.stream_scale <= prev.stream_scale;
                if !gives_up {
                    return Err(format!("notches[{i}]: no number may be larger than in notches[{}]", i - 1));
                }
            } else if n != Notch::FULL {
                return Err("notches[0]: must be the tier as it is (every factor 1, 16 lamps)".into());
            }
            notches.push(n);
        }

        let g = v.get("governor").ok_or("governor: missing")?;
        let governor = GovernorRules {
            target_fps: number(g, "target_fps", 15.0, 240.0).map_err(|e| format!("governor: {e}"))?,
            window_seconds: f64::from(number(g, "window_seconds", 0.25, 30.0).map_err(|e| format!("governor: {e}"))?),
            miss_seconds: f64::from(number(g, "miss_seconds", 0.1, 600.0).map_err(|e| format!("governor: {e}"))?),
            miss_factor: number(g, "miss_factor", 1.0, 4.0).map_err(|e| format!("governor: {e}"))?,
            ok_factor: number(g, "ok_factor", 0.5, 4.0).map_err(|e| format!("governor: {e}"))?,
            up_seconds: f64::from(number(g, "up_seconds", 0.1, 3600.0).map_err(|e| format!("governor: {e}"))?),
            down_cooldown: f64::from(number(g, "down_cooldown", 0.0, 600.0).map_err(|e| format!("governor: {e}"))?),
            bounce_window: f64::from(number(g, "bounce_window", 0.0, 3600.0).map_err(|e| format!("governor: {e}"))?),
            bounce_hold: f64::from(number(g, "bounce_hold", 0.0, 3600.0).map_err(|e| format!("governor: {e}"))?),
            bounce_max: g
                .get("bounce_max")
                .and_then(Value::as_u64)
                .filter(|n| (1..=10).contains(n))
                .ok_or("governor: bounce_max: a whole number from 1 to 10")? as u32,
            stable_forget: f64::from(number(g, "stable_forget", 0.0, 86_400.0).map_err(|e| format!("governor: {e}"))?),
            stall_ms: number(g, "stall_ms", 50.0, 60_000.0).map_err(|e| format!("governor: {e}"))?,
        };
        if governor.ok_factor > governor.miss_factor {
            return Err("governor: ok_factor must not exceed miss_factor (the gap between them is the dead band)".into());
        }

        let a = v.get("auto").ok_or("auto: missing")?;
        let tier_of = |key: &str| -> Result<Tier, String> {
            a.get(key).and_then(Value::as_str).and_then(Tier::parse).ok_or_else(|| format!("auto.{key}: expected LOW, MEDIUM or HIGH"))
        };
        let mut hardware = Vec::new();
        for (i, row) in a.get("hardware").and_then(Value::as_array).ok_or("auto.hardware: expected an array")?.iter().enumerate() {
            let at = |e: String| format!("auto.hardware[{i}]: {e}");
            let rule = HardwareRule {
                max_cores: match row.get("max_cores") {
                    None => None,
                    Some(c) => Some(c.as_u64().filter(|c| (1..=1024).contains(c)).ok_or_else(|| at("max_cores: a whole number of 1 or more".into()))? as u32),
                },
                max_memory_gb: match row.get("max_memory_gb") {
                    None => None,
                    Some(_) => Some(number(row, "max_memory_gb", 0.1, 4096.0).map_err(at)?),
                },
                mobile: match row.get("mobile") {
                    None => None,
                    Some(b) => Some(b.as_bool().ok_or_else(|| at("mobile: true or false".into()))?),
                },
                software: match row.get("software") {
                    None => None,
                    Some(b) => Some(b.as_bool().ok_or_else(|| at("software: true or false".into()))?),
                },
                tier: row.get("tier").and_then(Value::as_str).and_then(Tier::parse).ok_or_else(|| at("tier: expected LOW, MEDIUM or HIGH".into()))?,
                gpu_contains: match row.get("gpu_contains") {
                    None => Vec::new(),
                    Some(l) => l
                        .as_array()
                        .filter(|l| !l.is_empty())
                        .and_then(|l| l.iter().map(|w| w.as_str().filter(|w| !w.trim().is_empty()).map(|w| w.trim().to_uppercase())).collect::<Option<Vec<_>>>())
                        .ok_or_else(|| at("gpu_contains: a list of words".into()))?,
                },
            };
            if rule.gpu_contains.is_empty() && rule.max_cores.is_none() && rule.max_memory_gb.is_none() && rule.mobile.is_none() && rule.software.is_none() {
                return Err(at("names no condition, so it would match everything".into()));
            }
            hardware.push(rule);
        }
        let auto = AutoRules {
            hardware,
            default_tier: tier_of("default_tier")?,
            unknown_tier: tier_of("unknown_tier")?,
            settle_seconds: f64::from(number(a, "settle_seconds", 0.0, 120.0).map_err(|e| format!("auto: {e}"))?),
            measure_seconds: f64::from(number(a, "measure_seconds", 0.5, 120.0).map_err(|e| format!("auto: {e}"))?),
            min_frames: a.get("min_frames").and_then(Value::as_u64).filter(|n| (1..=100_000).contains(n)).ok_or("auto.min_frames: a whole number of 1 or more")? as usize,
            low_above_ratio: number(a, "low_above_ratio", 1.0, 20.0).map_err(|e| format!("auto: {e}"))?,
            medium_above_ratio: number(a, "medium_above_ratio", 1.0, 20.0).map_err(|e| format!("auto: {e}"))?,
        };
        if auto.medium_above_ratio >= auto.low_above_ratio {
            return Err("auto: medium_above_ratio must be below low_above_ratio".into());
        }
        Ok(Config { tier_scale, notches, governor, auto })
    }
}

/// A number under `key` in `lo..=hi`, as f32.
fn number(v: &Value, key: &str, lo: f64, hi: f64) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (lo..=hi).contains(n))
        .map(|n| n as f32)
        .ok_or_else(|| format!("{key}: a number from {lo} to {hi}"))
}

/// The table compiled into the executable. One that fails to parse is reported on stderr and `Config::fallback` stands.
pub fn config() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Config::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Config::fallback()
        })
    })
}

// ---- the AUTO tier -------------------------------------------------------------------------------------------------

/// What the browser reports about the machine (packaging/web/hw.js). Zero or false means "not reported".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    /// The GPU's name in upper case (hw.js key `gpu`, the WebGL unmasked renderer), empty when not reported.
    pub gpu: String,
    /// `navigator.hardwareConcurrency`, 0 when missing.
    pub cores: u32,
    /// `navigator.deviceMemory` in gigabytes (Chrome rounds it to a power of two and caps it at 8), 0 when missing.
    pub memory_gb: f32,
    /// A phone or tablet browser.
    pub mobile: bool,
    /// The WebGL renderer is a software one (SwiftShader, llvmpipe).
    pub software: bool,
}

impl Hardware {
    /// Parses hw.js's text: `{ "cores": n, "memory_gb": n, "mobile": bool, "software": bool }`. Missing or odd keys read as
    /// not reported; text that is not an object is `None`.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn parse(text: &str) -> Option<Hardware> {
        let v: Value = serde_json::from_str(text).ok()?;
        v.as_object()?;
        Some(Hardware {
            cores: v.get("cores").and_then(Value::as_u64).map_or(0, |c| c.min(1024) as u32),
            memory_gb: v.get("memory_gb").and_then(Value::as_f64).filter(|m| m.is_finite() && *m > 0.0).map_or(0.0, |m| m.min(4096.0) as f32),
            mobile: v.get("mobile").and_then(Value::as_bool).unwrap_or(false),
            software: v.get("software").and_then(Value::as_bool).unwrap_or(false),
            gpu: v.get("gpu").and_then(Value::as_str).map_or(String::new(), |g| g.chars().take(120).collect::<String>().to_uppercase()),
        })
    }

    /// The browser's report, or `None` where there is none (native builds). In a browser whose storage is blocked or whose
    /// page has no hw.js the report is empty (`Some` of nothing reported), which the table reads as `unknown_tier`.
    pub fn from_page() -> Option<Hardware> {
        #[cfg(target_arch = "wasm32")]
        {
            let report = super::web_storage::get(HARDWARE_KEY).and_then(|t| Hardware::parse(&t));
            Some(report.unwrap_or_default())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            None
        }
    }

    fn reported(&self) -> bool {
        self.cores > 0 || self.memory_gb > 0.0 || !self.gpu.is_empty()
    }
}

impl HardwareRule {
    fn matches(&self, hw: &Hardware) -> bool {
        (self.gpu_contains.is_empty() || self.gpu_contains.iter().any(|w| hw.gpu.contains(w.as_str())))
            && self.max_cores.map_or(true, |m| hw.cores > 0 && hw.cores <= m)
            && self.max_memory_gb.map_or(true, |m| hw.memory_gb > 0.0 && hw.memory_gb <= m)
            && self.mobile.map_or(true, |b| hw.mobile == b)
            && self.software.map_or(true, |b| hw.software == b)
    }
}

/// The tier the hardware report alone gives: the first matching row of the table, else the default; the unknown tier when
/// the browser reported no core count and no memory (a software or mobile flag alone still counts as a report).
pub fn pick_tier(auto: &AutoRules, hw: &Hardware) -> Tier {
    if let Some(r) = auto.hardware.iter().find(|r| r.matches(hw)) {
        return r.tier;
    }
    if !hw.reported() && !hw.mobile && !hw.software {
        auto.unknown_tier
    } else {
        auto.default_tier
    }
}

/// [`pick_tier`] for the browser build: a matching hardware row still decides, but when none does (or nothing was reported)
/// the web default applies (data/app/gfx.json web.default_tier, LOW as shipped) instead of the table's own default and
/// unknown tiers, which suit a desktop.
pub fn pick_tier_web(auto: &AutoRules, hw: &Hardware, web_default: Tier) -> Tier {
    auto.hardware.iter().find(|r| r.matches(hw)).map_or(web_default, |r| r.tier)
}

/// The seconds a frame should take: one over the lower of the target and the FPS LIMIT row (0, or under 15, is unlimited).
pub fn target_ms(target_fps: f32, fps_limit: f32) -> f32 {
    let fps = if fps_limit >= 15.0 { target_fps.min(fps_limit) } else { target_fps };
    1000.0 / fps.max(1.0)
}

/// Times the first seconds of play to check the hardware's guess against this machine.
#[derive(Clone, Debug, Default)]
pub struct Measure {
    played: f64,
    frames: Vec<f32>,
    timed: f64,
}

impl Measure {
    /// Feeds one play frame of `ms`. Returns the measured tier once the window is full (`None` while it is still running).
    /// A stall (`stall_ms` or more) is skipped; the settle seconds are not timed.
    pub fn feed(&mut self, ms: f32, target_ms: f32, auto: &AutoRules, stall_ms: f32) -> Option<Tier> {
        if !(ms > 0.0) || ms >= stall_ms {
            return None;
        }
        let seconds = f64::from(ms) / 1000.0;
        self.played += seconds;
        if self.played <= auto.settle_seconds {
            return None;
        }
        self.frames.push(ms);
        self.timed += seconds;
        if self.timed < auto.measure_seconds || self.frames.len() < auto.min_frames {
            return None;
        }
        self.frames.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = self.frames[self.frames.len() / 2];
        Some(if median > target_ms * auto.low_above_ratio {
            Tier::Low
        } else if median > target_ms * auto.medium_above_ratio {
            Tier::Medium
        } else {
            Tier::High
        })
    }
}

// ---- the governor --------------------------------------------------------------------------------------------------

/// Godot's quality governor: the decision core. Notch 0 is full quality and the notch grows as quality drops.
#[derive(Clone, Debug)]
pub struct Governor {
    rules: GovernorRules,
    pub notch: usize,
    pub max_notch: usize,
    pub target_ms: f32,
    window: VecDeque<f32>,
    window_ms: f64,
    miss_time: f64,
    ok_time: f64,
    hold_left: f64,
    bounces: u32,
    since_change: f64,
    since_up: f64,
    last_was_up: bool,
}

const NEVER: f64 = 1e9;

impl Governor {
    pub fn new(rules: GovernorRules, max_notch: usize, target_ms: f32) -> Governor {
        Governor {
            rules,
            notch: 0,
            max_notch,
            target_ms,
            window: VecDeque::new(),
            window_ms: 0.0,
            miss_time: 0.0,
            ok_time: 0.0,
            hold_left: 0.0,
            bounces: 0,
            since_change: NEVER,
            since_up: NEVER,
            last_was_up: false,
        }
    }

    /// Back to notch 0 with no history (a settings change: the old measurements describe different settings).
    pub fn reset(&mut self) {
        self.notch = 0;
        self.window.clear();
        self.window_ms = 0.0;
        self.miss_time = 0.0;
        self.ok_time = 0.0;
        self.hold_left = 0.0;
        self.bounces = 0;
        self.last_was_up = false;
        self.since_change = NEVER;
        self.since_up = NEVER;
    }

    /// The rolling average frame time, 0 before the first frame.
    pub fn average_ms(&self) -> f32 {
        if self.window.is_empty() {
            0.0
        } else {
            (self.window_ms / self.window.len() as f64) as f32
        }
    }

    /// Seconds the target has been missed in a row (for the F7 panel).
    pub fn miss_seconds(&self) -> f32 {
        self.miss_time as f32
    }

    /// Seconds a step up is still held for (for the F7 panel).
    pub fn hold_seconds(&self) -> f32 {
        self.hold_left as f32
    }

    /// One frame of `ms`; returns +1 / -1 when the notch moved down / up in quality (the notch grows as quality drops).
    pub fn feed(&mut self, ms: f32) -> i32 {
        let r = self.rules;
        let ms = f64::from(ms);
        self.window.push_back(ms as f32);
        self.window_ms += ms;
        let window_ms = r.window_seconds * 1000.0;
        while self.window.len() > 1 && self.window_ms - f64::from(*self.window.front().unwrap_or(&0.0)) > window_ms {
            self.window_ms -= f64::from(self.window.pop_front().unwrap_or(0.0));
        }
        let seconds = ms / 1000.0;
        self.since_change += seconds;
        self.since_up += seconds;
        self.hold_left = (self.hold_left - seconds).max(0.0);
        if self.last_was_up && self.since_up >= r.stable_forget {
            self.bounces = 0; // the last step up survived: forget the escalation
        }
        if self.window_ms < window_ms * 0.5 {
            return 0; // not a full enough window yet
        }
        let avg = (self.window_ms / self.window.len() as f64) as f32;
        if avg > self.target_ms * r.miss_factor {
            self.miss_time += seconds;
            self.ok_time = 0.0;
        } else {
            self.miss_time = (self.miss_time - seconds).max(0.0);
            if avg <= self.target_ms * r.ok_factor {
                self.ok_time += seconds;
            } else {
                self.ok_time = 0.0; // the dead band: neither missing nor clean
            }
        }
        if self.miss_time >= r.miss_seconds && self.notch < self.max_notch && self.since_change >= r.down_cooldown {
            if self.last_was_up && self.since_up < r.bounce_window {
                self.bounces = (self.bounces + 1).min(r.bounce_max);
                self.hold_left = r.bounce_hold * f64::from(2u32.pow(self.bounces - 1));
            }
            self.last_was_up = false; // a cascaded step down is not a bounce
            self.notch += 1;
            self.after_step();
            return 1;
        }
        if self.ok_time >= r.up_seconds && self.notch > 0 && self.hold_left <= 0.0 {
            self.last_was_up = true;
            self.since_up = 0.0;
            self.notch -= 1;
            self.after_step();
            return -1;
        }
        0
    }

    fn after_step(&mut self) {
        self.miss_time = 0.0;
        self.ok_time = 0.0;
        self.since_change = 0.0;
        // the new settings need a fresh window before they are judged
        self.window.clear();
        self.window_ms = 0.0;
    }
}

// ---- the live state ------------------------------------------------------------------------------------------------

/// What the renderer reads this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Live {
    /// The tier in force (for AUTO, the one picked).
    pub tier: Tier,
    pub notch: usize,
    pub render_scale: f32,
    pub lamps: usize,
    pub scales: Scales,
}

/// The factors the scene and the chunk builder apply (1 everywhere at notch 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scales {
    /// Of the fog cull distance (render/scene.rs `world_cull_range`).
    pub cull: f32,
    /// Of the draw distance of props drawn one by one (render/scene.rs `draw_static`).
    pub prop: f32,
    /// Of the per-frame chunk build budget (render/world_mesh.rs `StreamBudget::allows_scaled`).
    pub stream: f32,
}

impl Scales {
    pub const FULL: Scales = Scales { cull: 1.0, prop: 1.0, stream: 1.0 };
}

thread_local! {
    static SCALES: Cell<Scales> = const { Cell::new(Scales::FULL) };
    static AUTO_TIER: Cell<Tier> = const { Cell::new(Tier::High) };
    static ROWS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The cull, prop and stream factors in force (1 each until a [`Manager`] publishes something else).
pub fn scales() -> Scales {
    SCALES.with(Cell::get)
}

/// The name of the tier AUTO is using, for the QUALITY row (HIGH until a [`Manager`] publishes another).
pub fn auto_tier_name() -> &'static str {
    AUTO_TIER.with(Cell::get).name()
}

/// The panel rows for the F7 overlay: the tier, the notch and what it set, and the governor's view of the frame time.
pub fn rows() -> Vec<(String, String)> {
    ROWS.with(|r| r.borrow().clone())
}

/// Where the AUTO pick stands.
#[derive(Clone, Debug)]
enum Auto {
    /// The hardware's answer, until the first play seconds have been timed.
    Measuring(Tier, Measure),
    Resolved(Tier),
}

/// The quality state of one run of the game: the AUTO pick and the governor. Call [`Manager::update`] once a frame.
pub struct Manager {
    cfg: &'static Config,
    auto: Auto,
    gov: Governor,
    last_setting: String,
}

impl Manager {
    /// `hw` is the browser's report; `None` (a native build) keeps AUTO = HIGH.
    pub fn new(cfg: &'static Config, hw: Option<Hardware>) -> Manager {
        let auto = match hw {
            Some(hw) if crate::shared::task::IN_BROWSER => {
                let web = Tier::parse(&crate::app::gfx::web_rules().default_tier).unwrap_or(Tier::Low);
                Auto::Measuring(pick_tier_web(&cfg.auto, &hw, web), Measure::default())
            }
            Some(hw) => Auto::Measuring(pick_tier(&cfg.auto, &hw), Measure::default()),
            None => Auto::Resolved(Tier::High),
        };
        let gov = Governor::new(cfg.governor, cfg.max_notch(), target_ms(cfg.governor.target_fps, 0.0));
        Manager { cfg, auto, gov, last_setting: String::new() }
    }

    /// The tier AUTO is using: the hardware's guess until the first seconds are timed, then the pick.
    pub fn auto_tier(&self) -> Tier {
        match &self.auto {
            Auto::Measuring(t, _) | Auto::Resolved(t) => *t,
        }
    }

    /// Whether the AUTO pick is final.
    pub fn auto_resolved(&self) -> bool {
        matches!(self.auto, Auto::Resolved(_))
    }

    /// One frame. `setting` is the QUALITY row (AUTO, LOW, MEDIUM or HIGH), `adaptive` the ADAPTIVE QUALITY row,
    /// `fps_limit` the FPS LIMIT row, `frame_ms` the last frame's wall time and `in_play` whether that frame drew the 3D
    /// world (menus, loading and the pause screen do not count towards either the pick or the governor).
    pub fn update(&mut self, setting: &str, adaptive: bool, fps_limit: f32, frame_ms: f32, in_play: bool) -> Live {
        let auto_on = setting == "AUTO";
        if self.last_setting != setting {
            self.gov.reset();
            self.last_setting = setting.to_string();
        }
        let target = target_ms(self.cfg.governor.target_fps, fps_limit);
        self.gov.target_ms = target;
        let governing = auto_on && adaptive && self.auto_resolved();
        if !governing {
            if self.gov.notch != 0 || self.gov.average_ms() > 0.0 {
                self.gov.reset();
            }
        }
        if in_play && frame_ms.is_finite() && frame_ms > 0.0 {
            if auto_on {
                if let Auto::Measuring(hw_tier, m) = &mut self.auto {
                    if let Some(measured) = m.feed(frame_ms, target, &self.cfg.auto, self.cfg.governor.stall_ms) {
                        let pick = (*hw_tier).min(measured);
                        crate::lr_info!("quality: AUTO picked {} (hardware {}, measured {})", pick.name(), hw_tier.name(), measured.name());
                        self.auto = Auto::Resolved(pick);
                    }
                } else if governing && frame_ms < self.cfg.governor.stall_ms {
                    let before = self.gov.average_ms();
                    let moved = self.gov.feed(frame_ms);
                    if moved != 0 {
                        crate::lr_info!("quality: notch {} ({:.1} ms average against {:.1})", self.gov.notch, before, target);
                    }
                }
            }
        }
        let tier = if auto_on { self.auto_tier() } else { Tier::parse(setting).unwrap_or(Tier::High) };
        let notch = if governing { self.gov.notch } else { 0 };
        self.live(tier, notch)
    }

    fn live(&self, tier: Tier, notch: usize) -> Live {
        let n = self.cfg.notches[notch.min(self.cfg.max_notch())];
        Live {
            tier,
            notch,
            render_scale: self.cfg.render_scale(tier, notch),
            lamps: n.lamps,
            scales: Scales { cull: n.cull_scale, prop: n.prop_scale, stream: n.stream_scale },
        }
    }

    /// Hands `live` to the renderer: the shader's lamp limit, the scene's and the chunk builder's factors, and the F7 rows.
    pub fn publish(&self, live: &Live, setting: &str) {
        crate::render::shader::set_lamp_cap_for(live.tier.name());
        crate::render::shader::set_governor_lamp_cap(live.lamps);
        SCALES.with(|s| s.set(live.scales));
        AUTO_TIER.with(|t| t.set(self.auto_tier()));
        if crate::app::perf::mode() == 2 {
            let picked = match (&self.auto, setting == "AUTO") {
                (Auto::Measuring(..), true) => format!("AUTO ({} for now, timing)", live.tier.name()),
                (_, true) => format!("AUTO ({})", live.tier.name()),
                _ => live.tier.name().to_string(),
            };
            let rows = vec![
                ("quality".to_string(), picked),
                (
                    "notch".to_string(),
                    format!("{}/{} scale {:.2} lamps {} cull {:.2} props {:.2}", live.notch, self.cfg.max_notch(), live.render_scale, crate::render::shader::lamp_cap(), live.scales.cull, live.scales.prop),
                ),
                (
                    "governor".to_string(),
                    format!("avg {:.1} / {:.1} ms miss {:.1}s hold {:.0}s", self.gov.average_ms(), self.gov.target_ms, self.gov.miss_seconds(), self.gov.hold_seconds()),
                ),
            ];
            ROWS.with(|r| *r.borrow_mut() = rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = include_str!("../data/app/quality.json");

    fn rules() -> GovernorRules {
        Config::fallback().governor
    }

    fn gov(max: usize) -> Governor {
        Governor::new(rules(), max, 1000.0 / 60.0)
    }

    /// Feeds `seconds` of frames of `ms` each; returns every notch change as (second, +1 or -1).
    fn run(g: &mut Governor, ms: f32, seconds: f64) -> Vec<(f64, i32)> {
        let mut t = 0.0;
        let mut out = Vec::new();
        while t < seconds {
            let moved = g.feed(ms);
            t += f64::from(ms) / 1000.0;
            if moved != 0 {
                out.push((t, moved));
            }
        }
        out
    }

    #[test]
    fn the_shipped_file_parses_and_matches_the_built_in_fallback() {
        let c = Config::parse(FILE).unwrap();
        assert_eq!(&c, config());
        assert_eq!(c, Config::fallback(), "data/app/quality.json and Config::fallback drifted apart");
        assert_eq!(c.tier_scale, [0.35, 0.40, 0.50], "the shipped tier scales: LOW 0.35, MEDIUM 0.40, HIGH 0.50");
        assert_eq!(c.max_notch(), 3);
        let by_notch: Vec<f32> = (0..=3).map(|n| (c.render_scale(Tier::High, n) * 100.0).round() / 100.0).collect();
        assert_eq!(by_notch, vec![0.50, 0.50, 0.42, 0.35]);
        assert_eq!(c.notches.iter().map(|n| n.lamps).collect::<Vec<_>>(), vec![16, 16, 8, 4]);
    }

    #[test]
    fn a_broken_table_is_refused_with_the_field_named() {
        // every edit must find its text in the shipped file, or the case would pass by changing nothing
        let bad = |from: &str, to: &str| {
            assert!(FILE.contains(from), "the test's pattern is not in the shipped file: {from}");
            Config::parse(&FILE.replacen(from, to, 1)).unwrap_err()
        };
        assert!(bad("lr.quality/1", "lr.quality/2").contains("schema"));
        assert!(Config::parse("{").is_err());
        assert!(Config::parse("{}").unwrap_err().contains("schema"));
        // a tier scale out of range, a missing tier, an unknown tier
        assert!(bad("\"render_scale\": 0.35 }", "\"render_scale\": 3.0 }").contains("tiers.LOW"));
        assert!(bad("\"LOW\": { \"render_scale\": 0.35 },", "").contains("tiers.LOW"));
        assert!(bad("\"HIGH\": { \"render_scale\": 0.50 }", "\"HIGH\": { \"render_scale\": 0.50 }, \"ULTRA\": { \"render_scale\": 0.5 }").contains("ULTRA"));
        assert!(bad("\"LOW\": { \"render_scale\": 0.35 }", "\"LOW\": { \"render_scale\": 0.45 }").contains("must not fall"));
        // notch 0 is the tier as it is, and a later row never asks for more than the one before
        assert!(bad("{ \"render_scale\": 1.0, \"lamps\": 16, \"cull_scale\": 1.0, \"prop_scale\": 1.0, \"stream_scale\": 1.0 },", "{ \"render_scale\": 0.9, \"lamps\": 16, \"cull_scale\": 1.0, \"prop_scale\": 1.0, \"stream_scale\": 1.0 },").contains("notches[0]"));
        assert!(bad("\"lamps\": 8,", "\"lamps\": 17,").contains("lamps"));
        assert!(bad("\"lamps\": 8,", "\"lamps\": 0,").contains("lamps"));
        assert!(bad("\"lamps\": 4,", "\"lamps\": 12,").contains("notches[3]"));
        assert!(bad("\"cull_scale\": 0.7,", "\"cull_scale\": 0.95,").contains("notches[3]"));
        assert!(bad("\"stream_scale\": 0.5 }", "\"stream_scale\": 0.0 }").contains("stream_scale"));
        // the governor
        assert!(bad("\"ok_factor\": 1.05", "\"ok_factor\": 1.5").contains("ok_factor"));
        assert!(bad("\"bounce_max\": 4", "\"bounce_max\": 0").contains("bounce_max"));
        assert!(bad("\"target_fps\": 60", "\"target_fps\": 5").contains("target_fps"));
        assert!(bad("\"miss_seconds\": 3.0", "\"miss_seconds\": -1").contains("miss_seconds"));
        // the AUTO pick
        assert!(bad("{ \"max_cores\": 4, \"tier\": \"MEDIUM\" }", "{ \"max_cores\": 4, \"tier\": \"TURBO\" }").contains("auto.hardware"));
        assert!(bad("{ \"software\": true, \"tier\": \"LOW\" }", "{ \"tier\": \"LOW\" }").contains("no condition"));
        assert!(bad("\"default_tier\": \"HIGH\"", "\"default_tier\": \"AUTO\"").contains("default_tier"));
        assert!(bad("\"medium_above_ratio\": 1.17", "\"medium_above_ratio\": 1.9").contains("medium_above_ratio"));
        assert!(bad("\"min_frames\": 30", "\"min_frames\": 0").contains("min_frames"));
    }

    // ---- the governor ----

    #[test]
    fn it_gives_up_a_notch_at_a_time_when_frames_run_slow_and_stops_at_the_bottom() {
        let mut g = gov(3);
        let moves = run(&mut g, 30.0, 60.0);
        assert_eq!(moves.iter().map(|m| m.1).collect::<Vec<_>>(), vec![1, 1, 1], "one notch per miss, three notches, then nothing");
        assert_eq!(g.notch, 3);
        // the first step needs the window to fill (1 s) and the miss to last 3 s; the next two wait out the 4 s cooldown
        assert!(moves[0].0 > 3.0 && moves[0].0 < 5.5, "first step at {}", moves[0].0);
        assert!(moves[1].0 - moves[0].0 >= 4.0 - 0.1, "cooldown between steps: {:?}", moves);
        assert!(moves[2].0 - moves[1].0 >= 4.0 - 0.1);
    }

    #[test]
    fn it_takes_notches_back_slowly_once_frames_are_clean() {
        let mut g = gov(3);
        run(&mut g, 30.0, 40.0);
        assert_eq!(g.notch, 3);
        // a clean 8 ms frame: one notch back per 20 clean seconds, not faster
        let back = run(&mut g, 8.0, 100.0);
        assert_eq!(back.iter().map(|m| m.1).collect::<Vec<_>>(), vec![-1, -1, -1]);
        assert_eq!(g.notch, 0);
        assert!(back[0].0 >= 20.0 && back[1].0 - back[0].0 >= 20.0 - 0.1 && back[2].0 - back[1].0 >= 20.0 - 0.1, "{back:?}");
    }

    #[test]
    fn frames_between_the_ok_and_miss_lines_move_nothing_either_way() {
        let (target, r) = (1000.0 / 60.0, rules());
        let dead_band = target * (r.ok_factor + r.miss_factor) / 2.0;
        assert!(dead_band > target * r.ok_factor && dead_band < target * r.miss_factor);
        // at notch 0 it never goes down, and at notch 2 it never comes back up
        let mut g = gov(3);
        assert!(run(&mut g, dead_band, 120.0).is_empty());
        assert_eq!(g.notch, 0);
        let mut g = gov(3);
        run(&mut g, 30.0, 10.0);
        let at = g.notch;
        assert!(at >= 1);
        assert!(run(&mut g, dead_band, 120.0).is_empty(), "the known dead band of the Godot governor");
        assert_eq!(g.notch, at);
    }

    #[test]
    fn a_short_spike_is_not_a_miss() {
        let mut g = gov(3);
        for i in 0..2000 {
            // one 120 ms hitch in every 100 frames of 16 ms
            let ms = if i % 100 == 99 { 120.0 } else { 16.0 };
            assert_eq!(g.feed(ms), 0, "frame {i}");
        }
        assert_eq!(g.notch, 0);
    }

    #[test]
    fn a_step_down_soon_after_a_step_up_holds_the_next_step_up_and_the_hold_doubles() {
        let mut g = gov(3);
        run(&mut g, 30.0, 6.0);
        assert_eq!(g.notch, 1);
        // clean for 20 s: one notch back
        let up = run(&mut g, 8.0, 21.0);
        assert_eq!((up.len(), g.notch), (1, 0));
        // slow again inside the bounce window: down, and the next step up is held for 60 s
        let down = run(&mut g, 30.0, 8.0);
        assert_eq!(down.iter().map(|m| m.1).collect::<Vec<_>>(), vec![1]);
        assert!(g.hold_seconds() > 50.0, "hold {}", g.hold_seconds());
        let held = run(&mut g, 8.0, 50.0);
        assert!(held.is_empty() && g.notch == 1, "no step up during the hold");
        let freed = run(&mut g, 8.0, 40.0);
        assert_eq!(freed.iter().map(|m| m.1).collect::<Vec<_>>(), vec![-1]);
        // a second bounce doubles the hold
        run(&mut g, 30.0, 8.0);
        assert!(g.hold_seconds() > 110.0, "second bounce holds 120 s: {}", g.hold_seconds());
    }

    #[test]
    fn it_is_a_pure_function_of_the_frame_times_and_never_leaves_its_bounds() {
        let sequence = || {
            let mut seed = 0x2545f491u32;
            (0..40_000).map(move |i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                // phases of slow, clean and noisy running
                let base = match (i / 3000) % 4 {
                    0 => 30.0,
                    1 => 9.0,
                    2 => 17.8,
                    _ => 24.0,
                };
                base + (seed >> 24) as f32 / 255.0 * 12.0
            })
        };
        let trace = |max: usize| -> Vec<usize> {
            let mut g = gov(max);
            sequence()
                .map(|ms| {
                    g.feed(ms);
                    assert!(g.notch <= max, "left the table: {}", g.notch);
                    g.notch
                })
                .collect()
        };
        assert_eq!(trace(3), trace(3), "same frames, same notches");
        assert!(trace(3).iter().any(|n| *n > 0) && trace(3).iter().any(|n| *n == 0), "the run does move");
        assert!(trace(0).iter().all(|n| *n == 0), "a table with one row never moves");
        assert!(trace(1).iter().all(|n| *n <= 1));
    }

    #[test]
    fn a_reset_forgets_everything_and_the_target_follows_the_fps_limit() {
        let mut g = gov(3);
        run(&mut g, 30.0, 10.0);
        assert!(g.notch > 0 && g.average_ms() > 0.0);
        g.reset();
        assert_eq!((g.notch, g.average_ms(), g.miss_seconds(), g.hold_seconds()), (0, 0.0, 0.0, 0.0));
        assert!((target_ms(60.0, 0.0) - 16.667).abs() < 0.01);
        assert!((target_ms(60.0, 30.0) - 33.333).abs() < 0.01, "a 30 FPS cap is not a miss");
        assert!((target_ms(60.0, 144.0) - 16.667).abs() < 0.01, "a higher cap does not raise the target");
        assert!((target_ms(60.0, 10.0) - 16.667).abs() < 0.01, "under 15 is unlimited");
    }

    // ---- the AUTO pick ----

    fn hw(cores: u32, memory_gb: f32) -> Hardware {
        Hardware { cores, memory_gb, ..Default::default() }
    }

    #[test]
    fn an_intel_mac_gpu_name_picks_low_before_any_frame_is_timed() {
        let a = &Config::fallback().auto;
        let air = Hardware::parse(r#"{"cores":8,"memory_gb":8,"gpu":"ANGLE (Intel Inc., Intel(R) HD Graphics 6000, OpenGL 4.1)"}"#).unwrap();
        assert_eq!(pick_tier(a, &air), Tier::Low);
        assert_eq!(pick_tier(a, &hw(8, 8.0)), Tier::High, "no GPU name, no GPU rule");
        let other = Hardware::parse(r#"{"cores":8,"memory_gb":8,"gpu":"NVIDIA GeForce RTX 3060"}"#).unwrap();
        assert_eq!(pick_tier(a, &other), Tier::High);
        // a GPU name alone is a report, so it is not the unknown tier
        let mut u = a.clone();
        u.unknown_tier = Tier::Medium;
        assert_eq!(pick_tier(&u, &Hardware { gpu: "X".into(), ..Default::default() }), Tier::High);
    }

    #[test]
    fn the_hardware_table_picks_a_tier() {
        let auto = &Config::fallback().auto;
        let cases: Vec<(&str, Hardware, Tier)> = vec![
            ("two cores", hw(2, 8.0), Tier::Low),
            ("one core", hw(1, 0.0), Tier::Low),
            ("2 GB", hw(8, 2.0), Tier::Low),
            ("half a GB", hw(8, 0.5), Tier::Low),
            ("four cores", hw(4, 8.0), Tier::Medium),
            ("3 cores", hw(3, 0.0), Tier::Medium),
            ("4 GB", hw(8, 4.0), Tier::Medium),
            ("eight cores, 8 GB", hw(8, 8.0), Tier::High),
            ("twelve cores, memory not reported (Firefox, Safari)", hw(12, 0.0), Tier::High),
            ("cores not reported, 8 GB", hw(0, 8.0), Tier::High),
            ("a phone", Hardware { cores: 8, memory_gb: 8.0, mobile: true, ..Default::default() }, Tier::Medium),
            ("a software renderer", Hardware { cores: 16, memory_gb: 8.0, software: true, ..Default::default() }, Tier::Low),
        ];
        for (name, h, want) in cases {
            assert_eq!(pick_tier(auto, &h), want, "{name}");
        }
        // nothing reported at all is the unknown tier; a phone flag alone is still a report
        assert_eq!(pick_tier(auto, &Hardware::default()), auto.unknown_tier);
        assert_eq!(pick_tier_web(auto, &Hardware::default(), Tier::Low), Tier::Low, "the web default replaces the desktop default");
        assert_eq!(pick_tier_web(auto, &hw(1, 8.0), Tier::High), Tier::Low, "a matching row still decides");
        let mut stricter = auto.clone();
        stricter.unknown_tier = Tier::Medium;
        assert_eq!(pick_tier(&stricter, &Hardware::default()), Tier::Medium);
        assert_eq!(pick_tier(&stricter, &hw(8, 8.0)), Tier::High);
    }

    #[test]
    fn the_browsers_report_is_read_leniently() {
        assert_eq!(Hardware::parse(r#"{"cores":8,"memory_gb":8,"mobile":false,"software":false}"#), Some(hw(8, 8.0)));
        assert_eq!(Hardware::parse(r#"{"cores":4}"#), Some(hw(4, 0.0)));
        assert_eq!(Hardware::parse(r#"{"cores":-3,"memory_gb":"lots","mobile":1}"#), Some(Hardware::default()));
        assert_eq!(Hardware::parse(r#"{"cores":2,"memory_gb":0.5}"#), Some(hw(2, 0.5)));
        assert_eq!(Hardware::parse("[1,2]"), None);
        assert_eq!(Hardware::parse("nonsense"), None);
        assert_eq!(Hardware::parse(r#"{"cores":99999999,"memory_gb":1e99}"#).map(|h| (h.cores, h.memory_gb)), Some((1024, 4096.0)));
    }

    #[test]
    fn the_first_seconds_of_play_can_only_lower_the_pick() {
        let cfg = Config::fallback();
        let target = 1000.0 / 60.0;
        let measure = |ms: f32| -> Tier {
            let mut m = Measure::default();
            for i in 0..2000 {
                if let Some(t) = m.feed(ms, target, &cfg.auto, 250.0) {
                    assert!(i as f32 * ms / 1000.0 > 4.4, "settle 1.5 s then 3 s are timed, not less");
                    return t;
                }
            }
            panic!("never finished at {ms} ms");
        };
        assert_eq!(measure(8.0), Tier::High);
        assert_eq!(measure(16.7), Tier::High);
        assert_eq!(measure(21.0), Tier::Medium);
        assert_eq!(measure(40.0), Tier::Low);
        // stalls are skipped, and a frame that never arrives finishes nothing
        let mut m = Measure::default();
        assert!((0..500).all(|_| m.feed(900.0, target, &cfg.auto, 250.0).is_none()));
        assert!(m.feed(f32::NAN, target, &cfg.auto, 250.0).is_none());
        assert!(m.feed(0.0, target, &cfg.auto, 250.0).is_none());
    }

    fn manager(h: Option<Hardware>) -> Manager {
        Manager::new(config(), h)
    }

    /// Plays `seconds` at `ms` a frame under the setting; returns the last Live.
    fn play(m: &mut Manager, setting: &str, adaptive: bool, ms: f32, seconds: f64) -> Live {
        let mut live = m.update(setting, adaptive, 0.0, ms, true);
        let mut t = 0.0;
        while t < seconds {
            live = m.update(setting, adaptive, 0.0, ms, true);
            t += f64::from(ms) / 1000.0;
        }
        live
    }

    #[test]
    fn on_the_web_auto_is_no_longer_always_high() {
        // a weak machine: the hardware says MEDIUM at once, before any frame is timed
        let mut weak = manager(Some(hw(4, 8.0)));
        let first = weak.update("AUTO", true, 0.0, 16.7, false);
        assert_eq!(first.tier, Tier::Medium);
        assert_eq!(first.render_scale, 0.40);
        assert!(!weak.auto_resolved());
        // a strong machine that runs slowly is lowered by the timing, once
        let mut strong = manager(Some(hw(16, 8.0)));
        assert_eq!(strong.update("AUTO", true, 0.0, 16.7, false).tier, Tier::High);
        let live = play(&mut strong, "AUTO", false, 40.0, 8.0);
        assert!(strong.auto_resolved());
        assert_eq!(live.tier, Tier::Low);
        // and the pick holds: fast frames afterwards do not raise it
        assert_eq!(play(&mut strong, "AUTO", false, 5.0, 60.0).tier, Tier::Low);
        // a strong machine that runs well keeps HIGH
        let mut good = manager(Some(hw(16, 8.0)));
        assert_eq!(play(&mut good, "AUTO", true, 12.0, 8.0).tier, Tier::High);
        // the weak machine is not raised by fast frames either (the pick is the lower of the two)
        assert_eq!(play(&mut weak, "AUTO", true, 10.0, 8.0).tier, Tier::Medium);
    }

    #[test]
    fn a_native_build_keeps_auto_at_high_with_the_old_scale_and_no_governor() {
        let mut m = manager(None);
        let live = m.update("AUTO", false, 0.0, 16.7, true);
        assert_eq!((live.tier, live.render_scale, live.notch, live.lamps), (Tier::High, 0.50, 0, 16));
        assert!(m.auto_resolved());
        // slow frames with ADAPTIVE QUALITY off never move a notch; on, they do
        let off = play(&mut m, "AUTO", false, 40.0, 60.0);
        assert_eq!((off.notch, off.render_scale, off.scales), (0, 0.50, Scales::FULL));
        let on = play(&mut m, "AUTO", true, 40.0, 60.0);
        assert!(on.notch > 0 && on.render_scale < 0.50 && on.lamps < 16 && on.scales.cull < 1.0);
    }

    #[test]
    fn the_named_tiers_are_fixed_at_their_shipped_scales() {
        let mut m = manager(Some(hw(2, 1.0)));
        for (name, scale, tier) in [("LOW", 0.35, Tier::Low), ("MEDIUM", 0.40, Tier::Medium), ("HIGH", 0.50, Tier::High)] {
            let live = play(&mut m, name, true, 60.0, 30.0);
            assert_eq!((live.tier, live.render_scale, live.notch, live.scales), (tier, scale, 0, Scales::FULL), "{name} does not adapt");
        }
        // an unknown setting reads as HIGH, as `targets` always did
        assert_eq!(m.update("SOMETHING", true, 0.0, 16.7, true).tier, Tier::High);
    }

    #[test]
    fn the_governor_ignores_frames_outside_play_and_resets_when_the_setting_changes() {
        let mut m = manager(None);
        for _ in 0..4000 {
            let live = m.update("AUTO", true, 0.0, 50.0, false);
            assert_eq!(live.notch, 0, "menu frames are not timed");
        }
        let slow = play(&mut m, "AUTO", true, 50.0, 30.0);
        assert!(slow.notch > 0);
        assert_eq!(m.update("HIGH", true, 0.0, 50.0, true).notch, 0);
        assert_eq!(m.update("AUTO", true, 0.0, 16.7, true).notch, 0, "back on AUTO it starts again from the top");
        // a 30 FPS cap is judged against 33 ms, so 33 ms frames are fine
        let mut capped = manager(None);
        let mut live = capped.update("AUTO", true, 30.0, 33.3, true);
        for _ in 0..3000 {
            live = capped.update("AUTO", true, 30.0, 33.3, true);
        }
        assert_eq!(live.notch, 0);
    }

    #[test]
    fn the_notch_table_only_ever_reduces_work() {
        let c = config();
        let mut prev = c.notches[0];
        for (i, n) in c.notches.iter().enumerate() {
            assert!(n.render_scale <= prev.render_scale && n.lamps <= prev.lamps && n.cull_scale <= prev.cull_scale, "notch {i}");
            assert!(n.prop_scale <= prev.prop_scale && n.stream_scale <= prev.stream_scale, "notch {i}");
            prev = *n;
        }
        for tier in [Tier::Low, Tier::Medium, Tier::High] {
            for n in 0..=c.max_notch() + 2 {
                let s = c.render_scale(tier, n);
                assert!((MIN_RENDER_SCALE..=1.0).contains(&s), "{tier:?} notch {n}: {s}");
            }
        }
        // an overlong notch index is the last row
        assert_eq!(c.render_scale(Tier::High, 99), c.render_scale(Tier::High, c.max_notch()));
    }
}

#[cfg(test)]
mod governor_boundary_tests {
    use super::*;

    fn feed_for(g: &mut Governor, ms: f32, seconds: f64) -> i32 {
        let mut t = 0.0;
        let mut moves = 0;
        while t < seconds {
            moves += g.feed(ms).abs();
            t += f64::from(ms) / 1000.0;
        }
        moves
    }

    #[test]
    fn a_steady_frame_time_on_either_boundary_never_moves_the_notch() {
        let r = config().governor;
        let target = 1000.0 / r.target_fps;
        for factor in [r.miss_factor, r.ok_factor, (r.miss_factor + r.ok_factor) / 2.0] {
            let mut g = Governor::new(r, config().max_notch(), target);
            assert_eq!(feed_for(&mut g, target * factor * 0.999, 600.0), 0, "factor {factor}");
            assert_eq!(g.notch, 0);
        }
    }

    #[test]
    fn it_does_not_oscillate_when_a_notch_leaves_frames_just_under_the_miss_line() {
        let r = config().governor;
        let target = 1000.0 / r.target_fps;
        let mut g = Governor::new(r, config().max_notch(), target);
        let mut steps = 0;
        let mut t = 0.0;
        while t < 1200.0 {
            let ms = if g.notch == 0 { target * 1.3 } else { target * r.miss_factor * 0.999 };
            steps += g.feed(ms).abs();
            t += f64::from(ms) / 1000.0;
        }
        assert_eq!(steps, 1, "one step down and then it stays");
        assert_eq!(g.notch, 1);
    }

    #[test]
    fn the_targets_follow_the_fps_limit_at_40_50_and_60() {
        let r = config().governor;
        assert_eq!(target_ms(r.target_fps, 0.0), 1000.0 / r.target_fps);
        for limit in [40.0f32, 50.0, 60.0] {
            assert_eq!(target_ms(r.target_fps, limit), 1000.0 / limit.min(r.target_fps));
        }
    }

    #[test]
    fn the_tiers_and_notches_come_from_the_data_file() {
        let c = config();
        let v: serde_json::Value = serde_json::from_str(include_str!("../data/app/quality.json")).unwrap();
        for tier in [Tier::Low, Tier::Medium, Tier::High] {
            let want = v["tiers"][tier.name()]["render_scale"].as_f64().unwrap() as f32;
            assert!((c.render_scale(tier, 0) - want).abs() < 1e-6);
        }
        assert_eq!(c.max_notch() + 1, v["notches"].as_array().unwrap().len());
    }
}
