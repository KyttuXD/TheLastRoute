//! The status bars: a compact grid of thin outlined meters above the hotbar (default 3×2), short label and value under
//! each, no icons and no panel. Order and colours are data (data/ui/hud_bars.json, `lr.hud_bars/1`): BATTERY amber,
//! STAMINA light blue, FADE, EXPOSURE orange-red, NOISE, and TEMP. A readout is `BAT 97% // F`. The value to fill and
//! the zone rules are pure functions, unit-tested below. The old stamina bar over the hotbar is drawn only when this
//! grid is not (a broken data file).

use super::style::*;
use crate::app::assets::Assets;
use crate::config::{MAX_LAMP_CHARGE, MAX_STAMINA};
use crate::game::specialty::Mode;
use crate::game::Game;
use macroquad::prelude::*;
use serde_json::Value;

const FILE: &str = "ui/hud_bars.json";
const SCHEMA: &str = "lr.hud_bars/1";
/// The screen margin the HUD keeps (hud.rs SAFE).
const SAFE: f32 = 8.0;

/// The meter a bar reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Body temperature, 0 to 100 (game/meters.rs).
    Temperature,
    /// How visible the runner is to hunters, 0 to 100.
    Exposure,
    /// The noise meter, 0 to 100.
    Noise,
    /// Flashlight charge, percent.
    Lamp,
    /// Sprint stamina, percent.
    Stamina,
    /// How far the body has ducked and faded, 0 standing to 100 fully crouched.
    Fade,
}

impl Source {
    fn named(s: &str) -> Option<Source> {
        match s {
            "temperature" => Some(Source::Temperature),
            "exposure" => Some(Source::Exposure),
            "noise" => Some(Source::Noise),
            "lamp" => Some(Source::Lamp),
            "stamina" => Some(Source::Stamina),
            "fade" => Some(Source::Fade),
            _ => None,
        }
    }
}

/// The small icon in front of a bar's label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Thermometer,
    Eye,
    Sound,
    Bulb,
    Runner,
    /// Kept in the data for the loader; the row draws no icons.
    Veil,
}

impl Glyph {
    fn named(s: &str) -> Option<Glyph> {
        match s {
            "thermometer" => Some(Glyph::Thermometer),
            "eye" => Some(Glyph::Eye),
            "sound" => Some(Glyph::Sound),
            "bulb" => Some(Glyph::Bulb),
            "runner" => Some(Glyph::Runner),
            "veil" => Some(Glyph::Veil),
            _ => None,
        }
    }
}

/// A condition that replaces the zone's name and colour while it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Exhausted,
    Stim,
    SafeZone,
    HeatLocked,
}

impl When {
    fn named(s: &str) -> Option<When> {
        match s {
            "exhausted" => Some(When::Exhausted),
            "stim" => Some(When::Stim),
            "safe_zone" => Some(When::SafeZone),
            "heat_locked" => Some(When::HeatLocked),
            _ => None,
        }
    }
}

/// How much of the row is drawn, widest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub enum Tier {
    Full,
    Compact,
    Short,
    Icon,
}

pub type Rgb = [u8; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub from: f32,
    pub name: String,
    pub colour: Rgb,
    pub alert: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub at: f32,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub when: When,
    pub name: String,
    pub colour: Rgb,
    pub alert: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BarDef {
    pub id: String,
    /// Whether the grid draws this bar. A bar the data sets `"shown": false` stays loaded (its zones and tests still
    /// read) but is not drawn.
    pub shown: bool,
    pub source: Source,
    pub label: String,
    pub short: String,
    pub glyph: Glyph,
    /// The fill of the outlined bar (the zone colour still names the readout).
    pub fill_colour: Rgb,
    pub min: f32,
    pub max: f32,
    pub unit: String,
    pub tick_numbers: bool,
    pub zones: Vec<Zone>,
    pub ticks: Vec<Tick>,
    pub overrides: Vec<Override>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The widest the cluster plate grows (units).
    pub max_width: f32,
    pub gap: f32,
    /// The icon's edge (grows with the text when the text is enlarged for a small window).
    pub icon: f32,
    pub bar_height: f32,
    /// The narrowest a bar may get before the cluster steps down a tier.
    pub bar_min: f32,
    pub text_size: f32,
    pub sub_size: f32,
    /// Padding inside the plate.
    pub pad: f32,
    /// How many columns the status grid uses (3 → two rows of three for the six shipped bars).
    pub columns: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BarRules {
    pub layout: Layout,
    pub bars: Vec<BarDef>,
}

// -- loading ---------------------------------------------------------------------------------------------------

fn num(v: &Value, path: &str) -> Result<f32, String> {
    let mut at = v;
    for k in path.split('.') {
        at = &at[k];
    }
    let x = at.as_f64().ok_or_else(|| format!("{path} is not a number"))?;
    if !x.is_finite() {
        return Err(format!("{path} is not finite"));
    }
    Ok(x as f32)
}

fn ranged(v: &Value, path: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let x = num(v, path)?;
    if !(lo..=hi).contains(&x) {
        return Err(format!("{path} must be in [{lo}, {hi}]"));
    }
    Ok(x)
}

fn text(v: &Value, path: &str, max_chars: usize) -> Result<String, String> {
    let s = v.as_str().ok_or_else(|| format!("{path} is not text"))?;
    let n = s.chars().count();
    if n == 0 || n > max_chars {
        return Err(format!("{path} must be 1 to {max_chars} characters"));
    }
    if !s.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        return Err(format!("{path} must be plain ASCII"));
    }
    Ok(s.to_string())
}

/// `#rrggbb`.
pub fn parse_colour(s: &str) -> Option<Rgb> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

fn colour(v: &Value, path: &str) -> Result<Rgb, String> {
    let s = v.as_str().ok_or_else(|| format!("{path} is not a colour"))?;
    parse_colour(s).ok_or_else(|| format!("{path} must be #rrggbb"))
}

/// Reads and validates the file text. Every range is checked: nothing is silently defaulted.
pub fn parse(src: &str) -> Result<BarRules, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
    if v["schema"] != SCHEMA {
        return Err(format!("schema must be {SCHEMA}"));
    }
    let l = &v["layout"];
    let columns = match l.get("columns") {
        None => 3,
        Some(v) => {
            let n = v.as_u64().ok_or("layout.columns is not a whole number")?;
            if !(1..=4).contains(&n) {
                return Err("layout.columns must be 1 to 4".into());
            }
            n as usize
        }
    };
    let layout = Layout {
        max_width: ranged(l, "max_width", 60.0, 400.0)?,
        gap: ranged(l, "gap", 0.0, 8.0)?,
        icon: ranged(l, "icon", 3.0, 12.0)?,
        bar_height: ranged(l, "bar_height", 2.0, 8.0)?,
        bar_min: ranged(l, "bar_min", 8.0, 100.0)?,
        text_size: ranged(l, "text_size", 3.0, 12.0)?,
        sub_size: ranged(l, "sub_size", 3.0, 10.0)?,
        pad: ranged(l, "pad", 0.0, 8.0)?,
        columns,
    };
    let list = v["bars"].as_array().ok_or("bars is not a list")?;
    if list.is_empty() || list.len() > 8 {
        return Err("bars must list 1 to 8 bars".into());
    }
    let mut bars: Vec<BarDef> = Vec::new();
    for (i, b) in list.iter().enumerate() {
        let at = |k: &str| format!("bars[{i}].{k}");
        let id = text(&b["id"], &at("id"), 16)?;
        if bars.iter().any(|o| o.id == id) {
            return Err(format!("bar id {id} is used twice"));
        }
        let source = b["source"]
            .as_str()
            .and_then(Source::named)
            .ok_or_else(|| format!("{} is not temperature, exposure, noise, lamp, stamina or fade", at("source")))?;
        if bars.iter().any(|o| o.source == source) {
            return Err(format!("{} reads a meter another bar already shows", at("source")));
        }
        let glyph = b["glyph"]
            .as_str()
            .and_then(Glyph::named)
            .ok_or_else(|| format!("{} is not thermometer, eye, sound, bulb, runner or veil", at("glyph")))?;
        let fill_colour = colour(&b["colour"], &at("colour"))?;
        let min = num(b, "min").map_err(|e| format!("bars[{i}].{e}"))?;
        let max = num(b, "max").map_err(|e| format!("bars[{i}].{e}"))?;
        if min >= max {
            return Err(format!("bars[{i}] min must be under max"));
        }
        let zl = b["zones"].as_array().ok_or_else(|| format!("{} is not a list", at("zones")))?;
        if zl.is_empty() || zl.len() > 8 {
            return Err(format!("{} must list 1 to 8 zones", at("zones")));
        }
        let mut zones: Vec<Zone> = Vec::new();
        for (j, z) in zl.iter().enumerate() {
            let zp = format!("bars[{i}].zones[{j}]");
            let from = num(z, "from").map_err(|e| format!("{zp}.{e}"))?;
            if j == 0 && from != min {
                return Err(format!("{zp}.from must equal the bar's min (the first zone starts the bar)"));
            }
            if let Some(prev) = zones.last() {
                if from <= prev.from {
                    return Err(format!("{zp}.from must be above the zone before it"));
                }
            }
            if from > max {
                return Err(format!("{zp}.from is past the bar's max"));
            }
            zones.push(Zone {
                from,
                name: text(&z["name"], &format!("{zp}.name"), 8)?,
                colour: colour(&z["colour"], &format!("{zp}.colour"))?,
                alert: z["alert"].as_bool().ok_or_else(|| format!("{zp}.alert is not true or false"))?,
            });
        }
        let mut ticks: Vec<Tick> = Vec::new();
        for (j, t) in b["ticks"].as_array().ok_or_else(|| format!("{} is not a list", at("ticks")))?.iter().enumerate() {
            let tp = format!("bars[{i}].ticks[{j}]");
            let tick_at = num(t, "at").map_err(|e| format!("{tp}.{e}"))?;
            if !(tick_at > min && tick_at < max) {
                return Err(format!("{tp}.at must be inside the bar (between min and max)"));
            }
            if ticks.last().is_some_and(|p| tick_at <= p.at) {
                return Err(format!("{tp}.at must be above the tick before it"));
            }
            ticks.push(Tick { at: tick_at, label: text(&t["label"], &format!("{tp}.label"), 4)? });
        }
        let mut overrides: Vec<Override> = Vec::new();
        for (j, o) in b["overrides"].as_array().ok_or_else(|| format!("{} is not a list", at("overrides")))?.iter().enumerate() {
            let op = format!("bars[{i}].overrides[{j}]");
            let when = o["when"]
                .as_str()
                .and_then(When::named)
                .ok_or_else(|| format!("{op}.when is not exhausted, stim, safe_zone or heat_locked"))?;
            overrides.push(Override {
                when,
                name: text(&o["name"], &format!("{op}.name"), 8)?,
                colour: colour(&o["colour"], &format!("{op}.colour"))?,
                alert: o["alert"].as_bool().ok_or_else(|| format!("{op}.alert is not true or false"))?,
            });
        }
        bars.push(BarDef {
            shown: b["shown"].as_bool().unwrap_or(true),
            id,
            source,
            label: text(&b["label"], &at("label"), 8)?,
            short: text(&b["short"], &at("short"), 4)?,
            glyph,
            fill_colour,
            min,
            max,
            unit: b["unit"].as_str().filter(|u| u.chars().count() <= 2).ok_or_else(|| format!("{} must be text of up to 2 characters", at("unit")))?.to_string(),
            tick_numbers: b["tick_numbers"].as_bool().unwrap_or(false),
            zones,
            ticks,
            overrides,
        });
    }
    Ok(BarRules { layout, bars })
}

/// The shipped rules, or None when the embedded file is broken (the cluster then stays off and the old stamina bar
/// remains; the unit tests fail on a broken shipped file, so this only guards a hand-edited mod build).
pub fn rules() -> Option<&'static BarRules> {
    static RULES: std::sync::OnceLock<Option<BarRules>> = std::sync::OnceLock::new();
    RULES
        .get_or_init(|| {
            let src = crate::content::embedded::data_under(FILE).next().map(|x| x.1)?;
            match parse(src) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("data/{FILE}: {e}; the system bars stay off");
                    None
                }
            }
        })
        .as_ref()
}

// -- pure rules: value to fill, zone and text --------------------------------------------------------------------

/// How full a bar is for a value: 0 to 1, a value under the minimum or a broken number is 0.
pub fn fill_fraction(value: f32, min: f32, max: f32) -> f32 {
    if !value.is_finite() || !(max > min) {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

/// The index of the zone a value is in: the last zone that starts at or below it (the first for a value under all).
pub fn zone_index(zones: &[Zone], value: f32) -> usize {
    if !value.is_finite() {
        return 0;
    }
    zones.iter().rposition(|z| z.from <= value).unwrap_or(0)
}

/// The flags the overrides test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub exhausted: bool,
    pub stim: bool,
    pub safe_zone: bool,
    pub heat_locked: bool,
}

impl Flags {
    fn holds(&self, w: When) -> bool {
        match w {
            When::Exhausted => self.exhausted,
            When::Stim => self.stim,
            When::SafeZone => self.safe_zone,
            When::HeatLocked => self.heat_locked,
        }
    }
}

/// A bar's value turned into what is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved<'a> {
    pub value: f32,
    pub fill: f32,
    pub zone: usize,
    pub name: &'a str,
    pub colour: Rgb,
    pub alert: bool,
}

/// Value to fill, zone, name and colour for a bar. The first override whose condition holds replaces the zone's name and
/// colour.
pub fn resolve<'a>(bar: &'a BarDef, value: f32, flags: &Flags) -> Resolved<'a> {
    let value = if value.is_finite() { value.clamp(bar.min, bar.max) } else { bar.min };
    let zone = zone_index(&bar.zones, value);
    let z = &bar.zones[zone];
    let (name, colour, alert) = match bar.overrides.iter().find(|o| flags.holds(o.when)) {
        Some(o) => (o.name.as_str(), o.colour, o.alert),
        None => (z.name.as_str(), z.colour, z.alert),
    };
    Resolved { value, fill: fill_fraction(value, bar.min, bar.max), zone, name, colour, alert }
}

/// The line under a bar: `BATTERY 97% // F`.
pub fn readout(label: &str, value: f32, unit: &str, zone: &str) -> String {
    format!("{label} {:.0}{unit} // {zone}", value)
}

/// The value text for a tier: the rounded number and unit, with the zone name at the full tier; nothing below compact.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn value_text(bar: &BarDef, r: &Resolved, tier: Tier) -> String {
    let n = format!("{}{}", r.value.round() as i32, bar.unit);
    match tier {
        Tier::Full => format!("{n} {}", r.name),
        Tier::Compact => n,
        Tier::Short | Tier::Icon => String::new(),
    }
}

/// The longest value text a bar can show at a tier (its widest number with its longest zone or override name), so the
/// value column never changes width from frame to frame.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn widest_value_text(bar: &BarDef, tier: Tier) -> String {
    let num = format!("{}{}", bar.max.round() as i32, bar.unit);
    match tier {
        Tier::Full => {
            let name = bar
                .zones
                .iter()
                .map(|z| z.name.as_str())
                .chain(bar.overrides.iter().map(|o| o.name.as_str()))
                .max_by_key(|n| n.chars().count())
                .unwrap_or("");
            format!("{num} {name}")
        }
        Tier::Compact => num,
        Tier::Short | Tier::Icon => String::new(),
    }
}

/// The text widths a tier decision needs, each the widest over the bars.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub struct Widths {
    pub label: f32,
    pub short: f32,
    pub value_full: f32,
    pub value_compact: f32,
}

/// The widest tier whose row leaves a bar at least `layout.bar_min` units inside `avail` units.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn choose_tier(avail: f32, l: &Layout, icon: f32, w: &Widths) -> Tier {
    let need = |label: f32, value: f32| {
        let mut n = icon + l.gap + l.bar_min;
        if label > 0.0 {
            n += label + l.gap;
        }
        if value > 0.0 {
            n += value + l.gap;
        }
        n
    };
    if need(w.label, w.value_full) <= avail {
        Tier::Full
    } else if need(w.label, w.value_compact) <= avail {
        Tier::Compact
    } else if need(w.short, 0.0) <= avail {
        Tier::Short
    } else {
        Tier::Icon
    }
}

/// Which tick numbers fit under a bar: each is centred on its tick (kept inside the bar) and is dropped when it would
/// come within `min_gap` of the one before it. `ticks` is `(position as a fraction of the bar, label width)` left to right.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn fit_tick_labels(bar_w: f32, ticks: &[(f32, f32)], min_gap: f32) -> Vec<bool> {
    let mut last_end = f32::NEG_INFINITY;
    ticks
        .iter()
        .map(|&(frac, w)| {
            let left = (frac * bar_w - w * 0.5).clamp(0.0, (bar_w - w).max(0.0));
            if left >= last_end + min_gap {
                last_end = left + w;
                true
            } else {
                false
            }
        })
        .collect()
}

/// The status grid above the hotbar: `layout.columns` across, as many rows as the bar count needs, centred on the
/// hotbar and above the specialty bar when that bar has moved up over a narrow window. `line_h` is one caption line.
pub fn row_bounds(v: Vec2, c: Vec2, hb: Rect, n: usize, l: &Layout, line_h: f32) -> Rect {
    let n = n.max(1);
    let cols = l.columns.clamp(1, n);
    let rows = (n + cols - 1) / cols;
    let col_gap = l.gap.max(2.0);
    let row_gap = 2.0;
    let avail = (v.x - 2.0 * SAFE - col_gap * (cols as f32 - 1.0)).max(cols as f32 * 14.0);
    // stay near the hotbar width so six bars do not stretch across a wide desktop
    let target = hb.w.min(l.max_width.max(120.0));
    let col_w = ((avail / cols as f32).min(target / cols as f32)).clamp(28.0, 64.0);
    let total = col_w * cols as f32 + col_gap * (cols as f32 - 1.0);
    let x = (hb.x + hb.w * 0.5 - total * 0.5).clamp(SAFE, (v.x - SAFE - total).max(SAFE));
    let (_, sy, _, _, _) = super::hud::specialty_bar_layout(v, c, hb, crate::game::specialty::SLOT_COUNT);
    let limit = if sy + 0.5 < hb.y { sy } else { hb.y };
    let cell_h = l.bar_height + line_h + 2.0;
    let h = cell_h * rows as f32 + row_gap * (rows as f32 - 1.0);
    Rect::new(x, (limit - 3.0 - h).max(0.0), total, h)
}

// -- what the game says ----------------------------------------------------------------------------------------

/// The meter readings the bars show, taken from the game once a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub temperature: f32,
    /// The temperature the body is heading for (ambient and room, with the temperature item's source on top).
    pub target: f32,
    /// The temperature item's type while the runner carries it.
    pub item: Option<Mode>,
    pub exposure: f32,
    pub noise: f32,
    pub lamp: f32,
    pub stamina: f32,
    pub fade: f32,
    pub flags: Flags,
}

impl Snapshot {
    pub fn of(g: &Game) -> Snapshot {
        let ambient = g.meters.ambient;
        Snapshot {
            temperature: g.meters.temperature,
            target: (ambient + g.specialty.felt_local(ambient, g.local_climate())).clamp(0.0, 100.0),
            item: g.specialty.temp_mode(),
            exposure: g.meters.exposure,
            noise: g.meters.noise_meter,
            lamp: g.p.lamp / MAX_LAMP_CHARGE * 100.0,
            stamina: g.p.stamina / MAX_STAMINA * 100.0,
            fade: g.p.crouch_t.clamp(0.0, 1.0) * 100.0,
            flags: Flags {
                exhausted: g.p.exhausted,
                stim: g.p.stim_boost > 0.0,
                safe_zone: g.in_safe_zone(),
                heat_locked: g.meters.heat_locked,
            },
        }
    }

    pub fn value(&self, s: Source) -> f32 {
        match s {
            Source::Temperature => self.temperature,
            Source::Exposure => self.exposure,
            Source::Noise => self.noise,
            Source::Lamp => self.lamp,
            Source::Stamina => self.stamina,
            Source::Fade => self.fade,
        }
    }
}

/// The temperature item's source line under the TEMP bar: its type and the temperature it pushes the body toward.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn source_line(mode: Mode, target: f32) -> String {
    format!("{} ITEM > {}", mode.label(), target.round() as i32)
}

// -- drawing ---------------------------------------------------------------------------------------------------

fn col(c: Rgb) -> Color {
    Color::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0)
}

#[allow(dead_code)] // the row draws no icons; the glyph shapes stay for the data
fn mode_colour(m: Mode) -> Color {
    match m {
        Mode::Heat => rgba(1.0, 0.52, 0.2, 1.0),
        Mode::Cold => rgba(0.4, 0.72, 1.0, 1.0),
    }
}

#[allow(dead_code)] // the row draws no icons; the glyph shapes stay for the data
fn draw_glyph(g: Glyph, r: Rect, c: Color, fill_frac: f32, badge: Option<Color>) {
    let dim = alpha(c, 0.45);
    let mid = vec2(r.x + r.w * 0.5, r.y + r.h * 0.5);
    match g {
        Glyph::Thermometer => {
            let stem = Rect::new(mid.x - r.w * 0.14, r.y + r.h * 0.04, r.w * 0.28, r.h * 0.66);
            fill(stem, dim);
            let hg = stem.h * fill_frac.clamp(0.0, 1.0);
            fill(Rect::new(stem.x, stem.y + stem.h - hg, stem.w, hg), c);
            circle(vec2(mid.x, r.y + r.h * 0.82), r.h * 0.2, c);
        }
        Glyph::Eye => {
            let (l, rr) = (vec2(r.x, mid.y), vec2(r.x + r.w, mid.y));
            tri(l, vec2(mid.x, r.y + r.h * 0.18), rr, alpha(c, 0.8));
            tri(l, vec2(mid.x, r.y + r.h * 0.82), rr, alpha(c, 0.8));
            circle(mid, r.h * 0.2, rgba(0.04, 0.02, 0.05, 0.95));
            circle(mid, r.h * 0.09, c);
        }
        Glyph::Sound => {
            let w = r.w * 0.2;
            for (i, hf) in [0.4_f32, 0.7, 1.0].iter().enumerate() {
                let h = r.h * hf;
                fill(Rect::new(r.x + r.w * 0.1 + i as f32 * (w + r.w * 0.1), r.y + r.h - h, w, h), if i == 2 { c } else { alpha(c, 0.75) });
            }
        }
        Glyph::Bulb => {
            circle(vec2(mid.x, r.y + r.h * 0.38), r.h * 0.34, c);
            fill(Rect::new(mid.x - r.w * 0.2, r.y + r.h * 0.68, r.w * 0.4, r.h * 0.12), dim);
            fill(Rect::new(mid.x - r.w * 0.13, r.y + r.h * 0.84, r.w * 0.26, r.h * 0.12), dim);
        }
        Glyph::Runner => {
            for k in 0..2 {
                let ox = r.x + r.w * (0.1 + 0.38 * k as f32);
                let (a, b, d) = (vec2(ox, r.y + r.h * 0.1), vec2(ox + r.w * 0.34, mid.y), vec2(ox, r.y + r.h * 0.9));
                line(a, b, 1.0, c);
                line(b, d, 1.0, c);
            }
        }
        Glyph::Veil => {
            fill(Rect::new(r.x + r.w * 0.12, r.y + r.h * 0.2, r.w * 0.76, r.h * 0.6), alpha(c, 0.35));
            line(vec2(r.x + r.w * 0.2, mid.y), vec2(r.x + r.w * 0.8, mid.y), 1.0, c);
        }
    }
    if let Some(b) = badge {
        circle(vec2(r.x + r.w - 0.6, r.y + 0.9), r.w * 0.2, rgba(0.0, 0.0, 0.0, 0.8));
        circle(vec2(r.x + r.w - 0.6, r.y + 0.9), r.w * 0.15, b);
    }
}

fn draw_bar(r: Rect, bar: &BarDef, res: &Resolved) {
    // a darker track and a bright fill keep the thin meters readable on the game view
    fill(r, rgba(0.02, 0.02, 0.03, 0.82));
    if res.fill > 0.0 {
        let mut fill_c = col(bar.fill_colour);
        fill_c.a = 1.0;
        fill(Rect::new(r.x, r.y, r.w * res.fill, r.h), fill_c);
    }
    border(r, 1.0, hexa(0xf0ebe2ff));
}

/// Draws the grid above the hotbar; returns how many bars it drew (0 when the data is broken).
pub fn draw(g: &Game, a: &Assets, v: Vec2, hb: Rect) -> usize {
    let Some(rules) = rules() else { return 0 };
    let shown: Vec<&BarDef> = rules.bars.iter().filter(|b| b.shown).collect();
    if shown.is_empty() {
        return 0;
    }
    let snap = Snapshot::of(g);
    let c = (v * 0.5).round();
    let l = &rules.layout;
    let size0 = l.text_size;
    let line_h = lh(F::Body, size0);
    let band = row_bounds(v, c, hb, shown.len(), l, line_h);
    let cols = l.columns.clamp(1, shown.len());
    let rows = (shown.len() + cols - 1) / cols;
    let col_gap = l.gap.max(2.0);
    let row_gap = 2.0;
    let col_w = (band.w - col_gap * (cols as f32 - 1.0)) / cols as f32;
    let cell_h = (band.h - row_gap * (rows as f32 - 1.0)) / rows as f32;
    let ink = rgba(0.96, 0.94, 0.88, 1.0);
    for (i, bar) in shown.iter().copied().enumerate() {
        let res = resolve(bar, snap.value(bar.source), &snap.flags);
        let col = i % cols;
        let row = i / cols;
        let x = band.x + col as f32 * (col_w + col_gap);
        let y = band.y + row as f32 * (cell_h + row_gap);
        let br = Rect::new(x, y, col_w, l.bar_height);
        draw_bar(br, bar, &res);
        // short labels keep the 3×2 cells readable; the long form stays in the data for the loader tests
        let text = readout(&bar.short, res.value, &bar.unit, res.name);
        let mut size = size0;
        let mut twid = tw(a, F::Body, &text, size);
        while twid > col_w - 1.0 && size > 2.5 {
            size -= 0.5;
            twid = tw(a, F::Body, &text, size);
        }
        label(a, F::Body, &text, x + (col_w - twid).max(0.0) * 0.5, br.y + br.h + 0.5, size, ink);
    }
    shown.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> BarRules {
        let (_, src) = crate::content::embedded::data_under(FILE).next().expect("data/ui/hud_bars.json is embedded");
        parse(src).unwrap_or_else(|e| panic!("data/{FILE}: {e}"))
    }

    fn bar<'a>(r: &'a BarRules, id: &str) -> &'a BarDef {
        r.bars.iter().find(|b| b.id == id).unwrap_or_else(|| panic!("no bar {id}"))
    }

    #[test]
    fn the_shipped_file_loads_with_the_five_bars_in_order() {
        let r = shipped();
        let ids: Vec<&str> = r.bars.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["lamp", "stamina", "fade", "exposure", "noise", "temperature"]);
        let src: Vec<Source> = r.bars.iter().map(|b| b.source).collect();
        assert_eq!(src, [Source::Lamp, Source::Stamina, Source::Fade, Source::Exposure, Source::Noise, Source::Temperature]);
        assert_eq!(bar(&r, "lamp").fill_colour, [0xff, 0xc8, 0x57]);
        assert_eq!(readout("BAT", 97.0, "%", "F"), "BAT 97% // F");
        assert_eq!(readout("EXP", 56.4, "%", "LIT"), "EXP 56% // LIT");
        assert!(rules().is_some(), "the embedded loader accepts the shipped file");
        assert_eq!(r.layout.columns, 3);
        assert_eq!((r.layout.max_width, r.layout.icon, r.layout.sub_size, r.layout.pad, r.layout.bar_height, r.layout.text_size), (168.0, 6.0, 4.0, 1.0, 3.0, 4.5));
    }

    #[test]
    fn only_stamina_fade_and_temperature_are_shown_in_the_grid() {
        let r = shipped();
        let shown: Vec<&str> = r.bars.iter().filter(|b| b.shown).map(|b| b.id.as_str()).collect();
        assert_eq!(shown, ["stamina", "fade", "temperature"]);
        // three bars fill one row of the three columns
        assert_eq!(r.layout.columns, 3);
    }

    #[test]
    fn the_stamina_bar_marks_the_speed_threshold_of_the_stamina_rules() {
        // the FIT zone and the 45 tick sit exactly where the run speed starts to weaken (data/player/stamina.json)
        let r = shipped();
        let s = bar(&r, "stamina");
        let weak = crate::game::stamina::rules().weak_below * 100.0;
        assert!(s.ticks.iter().any(|t| (t.at - weak).abs() < 1e-4), "no tick at {weak}");
        let fit = s.zones.iter().find(|z| z.name == "FIT").expect("a FIT zone");
        assert!((fit.from - weak).abs() < 1e-4, "FIT starts at {} not {weak}", fit.from);
    }

    #[test]
    fn temperature_ticks_and_zones_are_the_real_thresholds() {
        let r = shipped();
        let t = bar(&r, "temperature");
        let ticks: Vec<f32> = t.ticks.iter().map(|x| x.at).collect();
        assert_eq!(ticks, [10.0, 60.0, 90.0]);
        let names: Vec<&str> = t.zones.iter().map(|z| z.name.as_str()).collect();
        assert_eq!(names, ["FROZEN", "COLD", "STABLE", "WARM", "HOT", "LATCH"]);
        assert_eq!(t.zones.last().map(|z| z.from), Some(100.0));
        assert!(t.tick_numbers);
        // the zone lines agree with the meters' own thresholds: freezing at 10 and under, overheating from 90
        let m = crate::game::meters::Meters::default();
        for v in [0.0, 5.0, 9.9, 10.0, 10.1, 30.0, 59.9, 60.0, 75.9, 76.0, 89.9, 90.0, 99.9] {
            let mut mm = crate::game::meters::Meters::default();
            mm.temperature = v;
            let name = resolve(t, v, &Flags::default()).name;
            assert_eq!(mm.freezing(), name == "FROZEN", "freezing line at {v}");
            assert_eq!(mm.overheating(), name == "HOT", "overheating line at {v}");
            // Wynelle's cold: she gathers strictly below 60
            assert_eq!(v < 60.0, name == "FROZEN" || name == "COLD", "wynelle's line at {v}");
        }
        assert!(!m.overheating());
        assert_eq!(resolve(t, 100.0, &Flags::default()).name, "LATCH");
        assert_eq!(resolve(t, 95.0, &Flags { heat_locked: true, ..Flags::default() }).name, "LATCHED");
    }

    #[test]
    fn fill_fraction_maps_the_value_to_a_clamped_fraction() {
        assert_eq!(fill_fraction(0.0, 0.0, 100.0), 0.0);
        assert_eq!(fill_fraction(50.0, 0.0, 100.0), 0.5);
        assert_eq!(fill_fraction(100.0, 0.0, 100.0), 1.0);
        assert_eq!(fill_fraction(-20.0, 0.0, 100.0), 0.0);
        assert_eq!(fill_fraction(180.0, 0.0, 100.0), 1.0);
        assert_eq!(fill_fraction(f32::NAN, 0.0, 100.0), 0.0);
        assert_eq!(fill_fraction(5.0, 10.0, 10.0), 0.0);
        assert!((fill_fraction(15.0, 10.0, 20.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zone_index_picks_the_last_zone_at_or_below_the_value() {
        let r = shipped();
        let z = &bar(&r, "stamina").zones;
        assert_eq!(zone_index(z, 0.0), 0);
        assert_eq!(zone_index(z, 19.9), 0);
        assert_eq!(zone_index(z, 20.0), 1);
        assert_eq!(zone_index(z, 44.9), 1);
        assert_eq!(zone_index(z, 45.0), 2);
        assert_eq!(zone_index(z, 100.0), 2);
        assert_eq!(zone_index(z, -5.0), 0);
        assert_eq!(zone_index(z, f32::NAN), 0);
    }

    #[test]
    fn overrides_replace_the_zone_name_and_colour_in_order() {
        let r = shipped();
        let s = bar(&r, "stamina");
        let plain = resolve(s, 80.0, &Flags::default());
        assert_eq!((plain.name, plain.colour, plain.alert), ("FIT", [0x6f, 0xe8, 0xff], false));
        let spent = resolve(s, 80.0, &Flags { exhausted: true, ..Flags::default() });
        assert_eq!((spent.name, spent.alert), ("SPENT", true));
        // exhausted is listed before stim, so it wins when both hold
        let both = resolve(s, 80.0, &Flags { exhausted: true, stim: true, ..Flags::default() });
        assert_eq!(both.name, "SPENT");
        let stim = resolve(s, 10.0, &Flags { stim: true, ..Flags::default() });
        assert_eq!((stim.name, stim.alert), ("STIM", false));
        let lamp = bar(&r, "lamp");
        assert_eq!(resolve(lamp, 0.0, &Flags::default()).name, "DEAD");
        assert_eq!(resolve(lamp, 10.0, &Flags::default()).name, "LOW");
        assert_eq!(resolve(lamp, 60.0, &Flags::default()).name, "F");
        assert_eq!(resolve(lamp, 60.0, &Flags { safe_zone: true, ..Flags::default() }).name, "CHARGE");
    }

    #[test]
    fn exposure_and_noise_zones_follow_the_web_hud() {
        let r = shipped();
        let e = bar(&r, "exposure");
        assert_eq!(resolve(e, 5.0, &Flags::default()).name, "LOW");
        assert_eq!(resolve(e, 40.0, &Flags::default()).name, "LIT");
        let hi = resolve(e, 75.0, &Flags::default());
        assert_eq!((hi.name, hi.alert), ("EXPOSED", true));
        let n = bar(&r, "noise");
        assert_eq!(resolve(n, 10.0, &Flags::default()).name, "QUIET");
        assert_eq!(resolve(n, 75.0, &Flags::default()).alert, true);
    }

    #[test]
    fn value_text_shrinks_with_the_tier() {
        let r = shipped();
        let t = bar(&r, "temperature");
        let res = resolve(t, 57.6, &Flags::default());
        assert_eq!(value_text(t, &res, Tier::Full), "58 COLD");
        assert_eq!(value_text(t, &res, Tier::Compact), "58");
        assert_eq!(value_text(t, &res, Tier::Short), "");
        assert_eq!(value_text(t, &res, Tier::Icon), "");
        let s = bar(&r, "stamina");
        let res = resolve(s, 100.0, &Flags::default());
        assert_eq!(value_text(s, &res, Tier::Full), "100% FIT");
        assert_eq!(value_text(s, &res, Tier::Compact), "100%");
        assert_eq!(widest_value_text(t, Tier::Full), "100 LATCHED");
        assert_eq!(widest_value_text(s, Tier::Compact), "100%");
    }

    #[test]
    fn the_source_line_names_the_item_and_its_pull() {
        assert_eq!(source_line(Mode::Heat, 84.0), "HEAT ITEM > 84");
        assert_eq!(source_line(Mode::Cold, 31.6), "COLD ITEM > 32");
    }

    #[test]
    fn narrow_layouts_step_down_through_the_tiers() {
        let l = shipped().layout;
        let w = Widths { label: 24.0, short: 10.0, value_full: 40.0, value_compact: 16.0 };
        let icon = 6.0;
        // base = icon + gap + bar_min; each label/value part adds its width and another gap
        let base = icon + l.gap + l.bar_min;
        let full = base + (w.label + l.gap) + (w.value_full + l.gap);
        let compact = base + (w.label + l.gap) + (w.value_compact + l.gap);
        let short = base + (w.short + l.gap);
        assert_eq!(choose_tier(full + 10.0, &l, icon, &w), Tier::Full);
        assert_eq!(choose_tier(full, &l, icon, &w), Tier::Full);
        assert_eq!(choose_tier(full - 0.5, &l, icon, &w), Tier::Compact);
        assert_eq!(choose_tier(compact, &l, icon, &w), Tier::Compact);
        assert_eq!(choose_tier(compact - 0.5, &l, icon, &w), Tier::Short);
        assert_eq!(choose_tier(short, &l, icon, &w), Tier::Short);
        assert_eq!(choose_tier(short - 0.5, &l, icon, &w), Tier::Icon);
        assert_eq!(choose_tier(0.0, &l, icon, &w), Tier::Icon);
    }

    #[test]
    fn tick_numbers_that_would_collide_are_dropped() {
        // a roomy bar keeps every number
        assert_eq!(fit_tick_labels(60.0, &[(0.1, 6.0), (0.6, 6.0), (0.9, 6.0)], 1.0), [true, true, true]);
        // a tight bar keeps the first of two that touch
        assert_eq!(fit_tick_labels(24.0, &[(0.1, 6.0), (0.6, 6.0), (0.9, 6.0)], 1.0), [true, true, false]);
        // the numbers stay inside the bar at its ends
        assert_eq!(fit_tick_labels(20.0, &[(0.02, 6.0), (0.98, 6.0)], 1.0), [true, true]);
        assert!(fit_tick_labels(20.0, &[], 1.0).is_empty());
    }

    #[test]
    fn the_cluster_never_meets_the_hotbar_or_the_specialty_bar() {
        let l = shipped().layout;
        let n = shipped().bars.len();
        assert_eq!((n + l.columns - 1) / l.columns, 2, "six bars in three columns are two rows");
        for (vx, vy) in [(390.0, 270.0), (480.0, 270.0), (480.0, 360.0), (640.0, 270.0), (844.0, 390.0), (1000.0, 270.0)] {
            let v = vec2(vx, vy);
            let c = (v * 0.5).round();
            let hb = Rect::new(c.x - 94.0, v.y - 42.0, 188.0, 34.0);
            let row = row_bounds(v, c, hb, n, &l, 6.0);
            assert!(row.x >= SAFE - 1e-3, "{vx}x{vy}: keeps the screen margin");
            assert!(row.x + row.w <= v.x - SAFE + 1e-3, "{vx}x{vy}: stays on screen");
            assert!(row.y >= 0.0, "{vx}x{vy}: on screen");
            assert!(row.y + row.h <= hb.y + 1e-3, "{vx}x{vy}: sits above the hotbar");
            assert!(row.w < hb.w + 8.0, "{vx}x{vy}: stays near the hotbar width ({})", row.w);
            let (sx, sy, sw, sh, sg) = super::super::hud::specialty_bar_layout(v, c, hb, crate::game::specialty::SLOT_COUNT);
            let spec = Rect::new(sx, sy, sw * 4.0 + sg * 3.0, sh);
            if sy + 0.5 < hb.y {
                assert!(row.y + row.h <= sy + 1e-3, "{vx}x{vy}: sits above the specialty bar");
            }
            assert!(!row.overlaps(&spec), "{vx}x{vy}: clear of the specialty bar");
            assert!(!row.overlaps(&hb), "{vx}x{vy}: clear of the hotbar");
        }
    }

    #[test]
    fn the_loader_rejects_a_broken_column_count() {
        assert!(with(|v| v["layout"]["columns"] = 0.into()).unwrap_err().contains("columns"));
        assert!(with(|v| v["layout"]["columns"] = 5.into()).unwrap_err().contains("columns"));
    }

    fn with(edit: impl Fn(&mut Value)) -> Result<BarRules, String> {
        let (_, src) = crate::content::embedded::data_under(FILE).next().unwrap();
        let mut v: Value = serde_json::from_str(src).unwrap();
        edit(&mut v);
        parse(&v.to_string())
    }

    #[test]
    fn the_loader_rejects_broken_files() {
        assert!(with(|_| {}).is_ok());
        assert!(with(|v| v["schema"] = "lr.hud_bars/2".into()).unwrap_err().contains("schema"));
        assert!(with(|v| v["bars"] = serde_json::json!([])).is_err());
        assert!(with(|v| v["layout"]["bar_min"] = 1.0.into()).unwrap_err().contains("bar_min"));
        assert!(with(|v| v["bars"][1]["source"] = "temperature".into()).unwrap_err().contains("another bar"));
        assert!(with(|v| v["bars"][1]["id"] = "temperature".into()).unwrap_err().contains("twice"));
        assert!(with(|v| v["bars"][0]["glyph"] = "skull".into()).unwrap_err().contains("glyph"));
        assert!(with(|v| v["bars"][0]["zones"][0]["colour"] = "red".into()).unwrap_err().contains("#rrggbb"));
        assert!(with(|v| v["bars"][0]["zones"][0]["from"] = 3.0.into()).unwrap_err().contains("min"));
        assert!(with(|v| {
            let i = v["bars"].as_array().unwrap().iter().position(|b| b["id"] == "temperature").unwrap();
            v["bars"][i]["zones"][2]["from"] = 5.0.into();
        })
        .unwrap_err()
        .contains("above the zone before"));
        assert!(with(|v| {
            let i = v["bars"].as_array().unwrap().iter().position(|b| b["id"] == "temperature").unwrap();
            v["bars"][i]["zones"][5]["from"] = 150.0.into();
        })
        .unwrap_err()
        .contains("past"));
        assert!(with(|v| {
            let i = v["bars"].as_array().unwrap().iter().position(|b| b["id"] == "temperature").unwrap();
            v["bars"][i]["ticks"][0]["at"] = 0.0.into();
        })
        .unwrap_err()
        .contains("inside the bar"));
        assert!(with(|v| {
            let i = v["bars"].as_array().unwrap().iter().position(|b| b["id"] == "temperature").unwrap();
            v["bars"][i]["ticks"][1]["at"] = 5.0.into();
        })
        .unwrap_err()
        .contains("above the tick before"));
        assert!(with(|v| v["bars"][0]["overrides"][0]["when"] = "sometimes".into()).unwrap_err().contains("when"));
        assert!(with(|v| v["bars"][0]["label"] = "WAY TOO LONG".into()).unwrap_err().contains("label"));
        assert!(with(|v| v["bars"][0]["short"] = "".into()).unwrap_err().contains("short"));
        assert!(with(|v| v["bars"][0]["max"] = (-1.0).into()).unwrap_err().contains("min must be under max"));
        assert!(parse("not json").is_err());
    }

    #[test]
    fn colours_parse_only_as_six_digit_hex() {
        assert_eq!(parse_colour("#75c492"), Some([0x75, 0xc4, 0x92]));
        assert_eq!(parse_colour("#FFFFFF"), Some([255, 255, 255]));
        assert_eq!(parse_colour("75c492"), None);
        assert_eq!(parse_colour("#75c49"), None);
        assert_eq!(parse_colour("#zzzzzz"), None);
        assert_eq!(parse_colour("#é12345"), None);
    }
}
