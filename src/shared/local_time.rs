//! Times shown to the player: the player's own time zone on a 12-hour AM/PM clock, for example
//! `NOV 8, 2026 3:42 PM EST`. Internal timestamps, logs and file names stay UTC; only text a player reads goes
//! through here.
//!
//! The zone comes from the computer (Windows: GetTimeZoneInformation; macOS and Linux: the `TZ` variable or
//! `/etc/localtime`, a TZif file with its POSIX footer rule). The web build reads nothing here: the game asks the
//! browser for its offset at each shown time and builds a zone with `zone_from_offset_minutes`. When the computer's
//! zone cannot be read (a bare container) the fallback zone applies: US Eastern, EST/EDT with the US rule (second
//! Sunday of March 2:00 to first Sunday of November 2:00). The words, zone mode, fallback, clock, the zone table and the
//! Windows zone-name abbreviations are `data/app/time.json` (`TimeRules`). No dependency beyond `serde_json`.
#![allow(dead_code)]

use serde_json::Value;
use std::collections::BTreeMap;

/// US Eastern as a POSIX TZ rule: the built-in last resort when even the data's fallback name is unknown.
pub const EASTERN_POSIX: &str = "EST5EDT,M3.2.0,M11.1.0";

/// Which zone the game shows times in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneMode {
    /// The player's computer.
    Local,
    /// A zone named in the data's zone table (for example `America/New_York`).
    Fixed(String),
}

/// The 12-hour or 24-hour clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    Hours12,
    Hours24,
}

/// `data/app/time.json`.
#[derive(Clone, Debug)]
pub struct TimeRules {
    pub zone: ZoneMode,
    pub fallback: String,
    pub clock: Clock,
    pub months: Vec<String>,
    pub am: String,
    pub pm: String,
    pub format: String,
    pub checked: String,
    pub never: String,
    zones: BTreeMap<String, String>,
    /// Windows zone names to abbreviations ("Eastern Standard Time" to "EST"); a name not listed shows as its offset.
    pub windows_abbr: BTreeMap<String, String>,
}

impl TimeRules {
    pub fn parse(text: &str) -> Result<TimeRules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string).ok_or(format!("time.json has no {k}"));
        let zone = match s("zone")?.as_str() {
            "local" => ZoneMode::Local,
            name => ZoneMode::Fixed(name.to_string()),
        };
        let clock = match s("clock")?.as_str() {
            "12h" => Clock::Hours12,
            "24h" => Clock::Hours24,
            other => return Err(format!("time.json clock must be 12h or 24h, not {other}")),
        };
        let months: Vec<String> = v
            .get("months")
            .and_then(|m| m.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if months.len() != 12 {
            return Err("time.json months needs 12 names".into());
        }
        let mut zones = BTreeMap::new();
        for (name, rule) in v.get("zones").and_then(|z| z.as_object()).ok_or("time.json has no zones")? {
            let rule = rule.as_str().ok_or(format!("time.json zone {name} is not text"))?;
            parse_posix(rule).ok_or(format!("time.json zone {name} has a bad rule"))?;
            zones.insert(name.clone(), rule.to_string());
        }
        let mut windows_abbr = BTreeMap::new();
        if let Some(map) = v.get("windows_abbr") {
            for (name, abbr) in map.as_object().ok_or("time.json windows_abbr is not a map")? {
                let abbr = abbr.as_str().ok_or(format!("time.json windows_abbr {name} is not text"))?;
                windows_abbr.insert(name.clone(), abbr.to_string());
            }
        }
        Ok(TimeRules {
            zone,
            fallback: s("fallback")?,
            clock,
            months,
            am: s("am")?,
            pm: s("pm")?,
            format: s("format")?,
            checked: s("checked")?,
            never: s("never")?,
            zones,
            windows_abbr,
        })
    }

    /// The fallback zone: the data's named zone, else built-in US Eastern.
    pub fn fallback_zone(&self) -> Zone {
        self.named(&self.fallback).unwrap_or_else(|| Zone::Posix(parse_posix(EASTERN_POSIX).expect("eastern rule parses")))
    }

    fn named(&self, name: &str) -> Option<Zone> {
        self.zones.get(name).and_then(|r| parse_posix(r)).map(Zone::Posix)
    }

    /// The zone to show times in, given what the computer reported (None when it could not be read).
    pub fn resolve_with(&self, local: Option<Zone>) -> Zone {
        match &self.zone {
            ZoneMode::Local => local.unwrap_or_else(|| self.fallback_zone()),
            ZoneMode::Fixed(name) => self.named(name).unwrap_or_else(|| self.fallback_zone()),
        }
    }

    /// The zone to show times in, reading the computer when the mode is local.
    pub fn resolve(&self) -> Zone {
        let local = if self.zone == ZoneMode::Local { detect_local(&self.windows_abbr) } else { None };
        self.resolve_with(local)
    }

    /// `NOV 8, 2026 3:42 PM EST` for a Unix time.
    pub fn format(&self, unix: i64, zone: &Zone) -> String {
        let (off, abbr) = zone.at(unix);
        let local = unix + i64::from(off);
        let (y, m, d) = civil_from_days(local.div_euclid(86_400));
        let secs = local.rem_euclid(86_400);
        let (h, mi) = (secs / 3_600, secs % 3_600 / 60);
        let time = match self.clock {
            Clock::Hours24 => format!("{h:02}:{mi:02}"),
            Clock::Hours12 => {
                let h12 = if h % 12 == 0 { 12 } else { h % 12 };
                format!("{h12}:{mi:02} {}", if h < 12 { &self.am } else { &self.pm })
            }
        };
        let zone_text = if abbr.is_empty() { offset_text(off) } else { abbr };
        self.format
            .replace("{month}", &self.months[(m - 1) as usize])
            .replace("{day}", &d.to_string())
            .replace("{year}", &y.to_string())
            .replace("{time}", &time)
            .replace("{zone}", &zone_text)
    }

    /// The "last checked" line for a Unix time, or the never-text for None.
    pub fn checked_line(&self, unix: Option<i64>, zone: &Zone) -> String {
        match unix {
            Some(t) => self.checked.replace("{time}", &self.format(t, zone)),
            None => self.checked.replace("{time}", &self.never),
        }
    }
}

/// `UTC+05:30` for an offset in seconds.
fn offset_text(off: i32) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.abs();
    format!("UTC{sign}{:02}:{:02}", a / 3_600, a % 3_600 / 60)
}

/// A time zone: how far from UTC and what it is called at any moment.
#[derive(Clone, Debug)]
pub enum Zone {
    Posix(Posix),
    Tzif(Tzif),
}

/// A fixed offset (minutes east of UTC) with no name, so it prints as `UTC+hh:mm`: the web build's zone for one
/// moment, from the browser's offset at that time.
pub fn zone_from_offset_minutes(east_min: i32) -> Zone {
    Zone::Posix(Posix { std_abbr: String::new(), std_off: east_min * 60, dst: None })
}

impl Zone {
    /// (seconds east of UTC, abbreviation) at a Unix time.
    pub fn at(&self, unix: i64) -> (i32, String) {
        match self {
            Zone::Posix(p) => p.at(unix),
            Zone::Tzif(t) => t.at(unix),
        }
    }
}

// ---------------------------------------------------------------- civil dates

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's algorithm).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (year, month, day) to days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Unix seconds of an ISO UTC time such as `2026-01-02T03:04:05Z` (GitHub's format); the seconds, the `Z` and
/// fractions are optional.
pub fn parse_iso_utc(text: &str) -> Option<i64> {
    let b = text.trim().as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    if b.len() < 16 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' {
        return None;
    }
    let (y, mo, d, h, mi) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?);
    let s = if b.get(16) == Some(&b':') { num(17, 2)? } else { 0 };
    if !(1..=12).contains(&mo) || d < 1 || d > i64::from(days_in_month(y, mo as u32)) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, mo as u32, d as u32) * 86_400 + h * 3_600 + mi * 60 + s)
}

// ---------------------------------------------------------------- POSIX rules

/// One "month, week, weekday, time" switch day: `M3.2.0/2` is the second Sunday of March at 2:00.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub month: u32,
    /// 1 to 5, 5 meaning the last one of the month.
    pub week: u32,
    /// 0 is Sunday.
    pub wday: u32,
    /// Seconds after local midnight, on the clock in force just before the switch.
    pub secs: i32,
}

impl Rule {
    /// The switch as local clock seconds since 1970 (the local clock being the one in force before the switch).
    fn local_seconds(&self, year: i64) -> i64 {
        let first = days_from_civil(year, self.month, 1);
        let first_wday = (first + 4).rem_euclid(7) as u32;
        let mut day = 1 + (self.wday + 7 - first_wday) % 7 + (self.week - 1) * 7;
        while day > days_in_month(year, self.month) {
            day -= 7;
        }
        days_from_civil(year, self.month, day) * 86_400 + i64::from(self.secs)
    }
}

/// A POSIX TZ rule: a standard name and offset, and optionally daylight time with its two switch days.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posix {
    pub std_abbr: String,
    /// Seconds east of UTC.
    pub std_off: i32,
    pub dst: Option<PosixDst>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixDst {
    pub abbr: String,
    pub off: i32,
    /// When daylight time starts (local standard clock).
    pub start: Rule,
    /// When daylight time ends (local daylight clock).
    pub end: Rule,
}

impl Posix {
    pub fn at(&self, unix: i64) -> (i32, String) {
        let Some(d) = &self.dst else { return (self.std_off, self.std_abbr.clone()) };
        let year = civil_from_days((unix + i64::from(self.std_off)).div_euclid(86_400)).0;
        let start = d.start.local_seconds(year) - i64::from(self.std_off);
        let end = d.end.local_seconds(year) - i64::from(d.off);
        let in_dst = if start < end { unix >= start && unix < end } else { !(unix >= end && unix < start) };
        if in_dst {
            (d.off, d.abbr.clone())
        } else {
            (self.std_off, self.std_abbr.clone())
        }
    }
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn name(&mut self) -> Option<String> {
        let start = self.i;
        if self.eat(b'<') {
            let from = self.i;
            while self.peek().is_some_and(|c| c != b'>') {
                self.i += 1;
            }
            let n = String::from_utf8_lossy(&self.s[from..self.i]).into_owned();
            return if self.eat(b'>') && !n.is_empty() { Some(n) } else { None };
        }
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i - start >= 3 {
            Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
        } else {
            None
        }
    }
    fn number(&mut self) -> Option<i32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
    /// `[+-]hh[:mm[:ss]]` in seconds, signed as written.
    fn clock(&mut self) -> Option<i32> {
        let sign = if self.eat(b'-') {
            -1
        } else {
            self.eat(b'+');
            1
        };
        let mut total = self.number()? * 3_600;
        if self.eat(b':') {
            total += self.number()? * 60;
            if self.eat(b':') {
                total += self.number()?;
            }
        }
        Some(sign * total)
    }
    fn rule(&mut self) -> Option<Rule> {
        if !self.eat(b'M') {
            return None;
        }
        let month = self.number()? as u32;
        if !self.eat(b'.') {
            return None;
        }
        let week = self.number()? as u32;
        if !self.eat(b'.') {
            return None;
        }
        let wday = self.number()? as u32;
        let secs = if self.eat(b'/') { self.clock()? } else { 7_200 };
        if !(1..=12).contains(&month) || !(1..=5).contains(&week) || wday > 6 {
            return None;
        }
        Some(Rule { month, week, wday, secs })
    }
}

/// Parses a POSIX TZ string such as `EST5EDT,M3.2.0,M11.1.0` or `<+03>-3`. Only the `Mm.w.d` switch form is
/// understood; a daylight name with no rules uses the US rule.
pub fn parse_posix(text: &str) -> Option<Posix> {
    let mut c = Cursor { s: text.trim().as_bytes(), i: 0 };
    let std_abbr = c.name()?;
    let std_off = -c.clock()?;
    if c.peek().is_none() {
        return Some(Posix { std_abbr, std_off, dst: None });
    }
    let abbr = c.name()?;
    let off = if matches!(c.peek(), Some(b',') | None) { std_off + 3_600 } else { -c.clock()? };
    let (start, end) = if c.eat(b',') {
        let start = c.rule()?;
        if !c.eat(b',') {
            return None;
        }
        (start, c.rule()?)
    } else {
        (Rule { month: 3, week: 2, wday: 0, secs: 7_200 }, Rule { month: 11, week: 1, wday: 0, secs: 7_200 })
    };
    if c.peek().is_some() {
        return None;
    }
    Some(Posix { std_abbr, std_off, dst: Some(PosixDst { abbr, off, start, end }) })
}

// ---------------------------------------------------------------- TZif

/// A parsed TZif file (RFC 8536): the transition list and, past the last one, the footer's POSIX rule.
#[derive(Clone, Debug)]
pub struct Tzif {
    times: Vec<i64>,
    idx: Vec<u8>,
    /// (seconds east of UTC, abbreviation)
    types: Vec<(i32, String)>,
    footer: Option<Posix>,
}

impl Tzif {
    pub fn at(&self, unix: i64) -> (i32, String) {
        let ty = |i: usize| self.types.get(i).cloned().unwrap_or((0, String::new()));
        if self.times.is_empty() || unix < self.times[0] {
            if self.times.is_empty() {
                if let Some(f) = &self.footer {
                    return f.at(unix);
                }
            }
            return ty(0);
        }
        let n = self.times.partition_point(|&t| t <= unix);
        if n == self.times.len() {
            if let Some(f) = &self.footer {
                return f.at(unix);
            }
        }
        ty(self.idx[n - 1] as usize)
    }
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// One data block starting at the header at `off`; returns the zone data and where the block ends.
fn tzif_block(d: &[u8], off: usize, time_size: usize) -> Option<(Tzif, usize)> {
    if d.get(off..off + 4)? != b"TZif" {
        return None;
    }
    let count = |n: usize| be32(d, off + 20 + 4 * n).map(|v| v as usize);
    let (isut, isstd, leap, timecnt, typecnt, charcnt) = (count(0)?, count(1)?, count(2)?, count(3)?, count(4)?, count(5)?);
    let mut p = off + 44;
    let mut times = Vec::with_capacity(timecnt);
    for i in 0..timecnt {
        let at = p + i * time_size;
        times.push(if time_size == 8 {
            i64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?)
        } else {
            i64::from(be32(d, at)? as i32)
        });
    }
    p += timecnt * time_size;
    let idx = d.get(p..p + timecnt)?.to_vec();
    p += timecnt;
    let type_bytes = d.get(p..p + typecnt * 6)?;
    p += typecnt * 6;
    let chars = d.get(p..p + charcnt)?;
    p += charcnt;
    let mut types = Vec::with_capacity(typecnt);
    for t in type_bytes.chunks(6) {
        let utoff = i32::from_be_bytes(t[0..4].try_into().ok()?);
        let start = t[5] as usize;
        let end = chars.get(start..)?.iter().position(|&c| c == 0).map(|n| start + n)?;
        types.push((utoff, String::from_utf8_lossy(&chars[start..end]).into_owned()));
    }
    if idx.iter().any(|&i| i as usize >= types.len()) {
        return None;
    }
    p += leap * (time_size + 4) + isstd + isut;
    Some((Tzif { times, idx, types, footer: None }, p))
}

/// Parses a TZif file, preferring the version 2+ block (64-bit times and the POSIX footer).
pub fn parse_tzif(d: &[u8]) -> Option<Tzif> {
    let (v1, end1) = tzif_block(d, 0, 4)?;
    if *d.get(4)? < b'2' {
        return Some(v1);
    }
    let (mut v2, end2) = tzif_block(d, end1, 8)?;
    if d.get(end2) == Some(&b'\n') {
        let rest = &d[end2 + 1..];
        if let Some(n) = rest.iter().position(|&c| c == b'\n') {
            v2.footer = std::str::from_utf8(&rest[..n]).ok().and_then(parse_posix);
        }
    }
    Some(v2)
}

// ---------------------------------------------------------------- the computer's zone

/// The computer's zone, or None when it cannot be read. `abbrs` names Windows zones (`TimeRules::windows_abbr`).
pub fn detect_local(abbrs: &BTreeMap<String, String>) -> Option<Zone> {
    detect_os(abbrs)
}

/// The web build reads no zone here (the game asks the browser per time, app/updater.rs `zone_at`).
#[cfg(target_arch = "wasm32")]
fn detect_os(_abbrs: &BTreeMap<String, String>) -> Option<Zone> {
    None
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn detect_os(_abbrs: &BTreeMap<String, String>) -> Option<Zone> {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim().trim_start_matches(':').to_string();
        if !tz.is_empty() {
            if let Some(z) = zone_from_tz_var(&tz) {
                return Some(z);
            }
        }
    }
    let bytes = std::fs::read("/etc/localtime").ok()?;
    parse_tzif(&bytes).map(Zone::Tzif)
}

/// The `TZ` variable: a zoneinfo name or path, or a POSIX rule.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn zone_from_tz_var(tz: &str) -> Option<Zone> {
    if !tz.contains("..") {
        let candidates = [format!("/usr/share/zoneinfo/{tz}"), format!("/usr/lib/zoneinfo/{tz}"), tz.to_string()];
        for path in candidates.iter().filter(|p| p.starts_with('/')) {
            if let Some(z) = std::fs::read(path).ok().and_then(|b| parse_tzif(&b)) {
                return Some(Zone::Tzif(z));
            }
        }
    }
    parse_posix(tz).map(Zone::Posix)
}

#[cfg(windows)]
fn detect_os(abbrs: &BTreeMap<String, String>) -> Option<Zone> {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SysTime {
        year: u16,
        month: u16,
        wday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        ms: u16,
    }
    #[repr(C)]
    struct Tzi {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SysTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SysTime,
        daylight_bias: i32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(info: *mut Tzi) -> u32;
    }
    let mut t = Tzi {
        bias: 0,
        standard_name: [0; 32],
        standard_date: SysTime::default(),
        standard_bias: 0,
        daylight_name: [0; 32],
        daylight_date: SysTime::default(),
        daylight_bias: 0,
    };
    // TIME_ZONE_ID_INVALID
    if unsafe { GetTimeZoneInformation(&mut t) } == 0xFFFF_FFFF {
        return None;
    }
    let name = |n: &[u16; 32]| String::from_utf16_lossy(&n[..n.iter().position(|&c| c == 0).unwrap_or(32)]);
    let switch = |d: &SysTime| (d.month as u32, d.day as u32, d.wday as u32, d.hour as i32 * 3_600 + d.minute as i32 * 60 + d.second as i32);
    Some(Zone::Posix(posix_from_windows(
        t.bias,
        t.standard_bias,
        t.daylight_bias,
        &name(&t.standard_name),
        &name(&t.daylight_name),
        switch(&t.daylight_date),
        switch(&t.standard_date),
        abbrs,
    )))
}

#[cfg(not(any(windows, unix, target_arch = "wasm32")))]
fn detect_os(_abbrs: &BTreeMap<String, String>) -> Option<Zone> {
    None
}

/// A Windows zone as a POSIX rule. Biases are minutes west of UTC as Windows reports them; each switch is
/// (month, week 1-5, weekday 0 = Sunday, seconds into the day), month 0 meaning no daylight time. `dst_start` is
/// the Windows "daylight date" and `dst_end` its "standard date". `abbrs` maps zone names to abbreviations.
pub fn posix_from_windows(
    bias: i32,
    std_bias: i32,
    dst_bias: i32,
    std_name: &str,
    dst_name: &str,
    dst_start: (u32, u32, u32, i32),
    dst_end: (u32, u32, u32, i32),
    abbrs: &BTreeMap<String, String>,
) -> Posix {
    let std_off = -(bias + std_bias) * 60;
    let std_abbr = windows_abbr(std_name, std_off, abbrs);
    if dst_start.0 == 0 || dst_end.0 == 0 {
        return Posix { std_abbr, std_off, dst: None };
    }
    let off = -(bias + dst_bias) * 60;
    let rule = |(month, week, wday, secs): (u32, u32, u32, i32)| Rule { month, week: week.clamp(1, 5), wday: wday.min(6), secs };
    Posix { std_abbr, std_off, dst: Some(PosixDst { abbr: windows_abbr(dst_name, off, abbrs), off, start: rule(dst_start), end: rule(dst_end) }) }
}

/// A Windows zone name as the player reads it: the data's abbreviation ("Eastern Standard Time" is "EST"), else the
/// offset, `UTC+01:00` (no made-up initials for zones the data does not list).
fn windows_abbr(name: &str, off: i32, abbrs: &BTreeMap<String, String>) -> String {
    abbrs.get(name).cloned().unwrap_or_else(|| offset_text(off))
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> TimeRules {
        TimeRules::parse(include_str!("../bin/last_route/data/app/time.json")).expect("time.json parses")
    }

    fn at(iso: &str) -> i64 {
        parse_iso_utc(iso).expect(iso)
    }

    fn eastern() -> Zone {
        Zone::Posix(parse_posix(EASTERN_POSIX).unwrap())
    }

    #[test]
    fn iso_times_become_unix_seconds() {
        assert_eq!(at("1970-01-01T00:00:00Z"), 0);
        assert_eq!(at("2001-09-09T01:46:40Z"), 1_000_000_000);
        assert_eq!(at("2024-02-29 00:00"), 1_709_164_800);
        assert_eq!(parse_iso_utc("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso_utc("nope"), None);
    }

    #[test]
    fn utc_becomes_eastern_across_both_dst_edges() {
        let z = eastern();
        // 2026: DST starts Sunday March 8 at 2:00 EST (07:00 UTC), ends Sunday November 1 at 2:00 EDT (06:00 UTC)
        assert_eq!(z.at(at("2026-03-08T06:59:59Z")), (-5 * 3_600, "EST".into()));
        assert_eq!(z.at(at("2026-03-08T07:00:00Z")), (-4 * 3_600, "EDT".into()));
        assert_eq!(z.at(at("2026-11-01T05:59:59Z")), (-4 * 3_600, "EDT".into()));
        assert_eq!(z.at(at("2026-11-01T06:00:00Z")), (-5 * 3_600, "EST".into()));
        assert_eq!(z.at(at("2026-07-04T12:00:00Z")).1, "EDT");
        assert_eq!(z.at(at("2027-01-15T12:00:00Z")).1, "EST");
        // the year after: second Sunday of March 2027 is the 14th
        assert_eq!(z.at(at("2027-03-14T06:59:59Z")).1, "EST");
        assert_eq!(z.at(at("2027-03-14T07:00:00Z")).1, "EDT");
    }

    #[test]
    fn the_player_text_is_twelve_hour_with_the_zone() {
        let (r, z) = (rules(), eastern());
        assert_eq!(r.format(at("2026-11-08T20:42:00Z"), &z), "NOV 8, 2026 3:42 PM EST");
        assert_eq!(r.format(at("2026-11-08T05:00:00Z"), &z), "NOV 8, 2026 12:00 AM EST");
        assert_eq!(r.format(at("2026-11-08T17:00:00Z"), &z), "NOV 8, 2026 12:00 PM EST");
        assert_eq!(r.format(at("2026-11-08T17:05:00Z"), &z), "NOV 8, 2026 12:05 PM EST");
        assert_eq!(r.format(at("2026-07-01T03:30:00Z"), &z), "JUN 30, 2026 11:30 PM EDT");
    }

    #[test]
    fn the_shipped_defaults_are_local_zone_twelve_hour_eastern_fallback() {
        let r = rules();
        assert_eq!(r.zone, ZoneMode::Local);
        assert_eq!(r.clock, Clock::Hours12);
        assert_eq!(r.fallback, "America/New_York");
        assert_eq!(r.checked_line(None, &eastern()), "LAST CHECKED: NEVER");
        assert_eq!(r.checked_line(Some(at("2026-11-08T20:42:00Z")), &eastern()), "LAST CHECKED: NOV 8, 2026 3:42 PM EST");
    }

    #[test]
    fn the_fallback_is_used_when_the_computer_zone_is_unavailable() {
        let r = rules();
        let z = r.resolve_with(None);
        assert_eq!(z.at(at("2026-01-10T12:00:00Z")), (-5 * 3_600, "EST".into()));
        assert_eq!(z.at(at("2026-07-10T12:00:00Z")).1, "EDT");
        // a local zone, when there is one, wins
        let local = Zone::Posix(parse_posix("PST8PDT,M3.2.0,M11.1.0").unwrap());
        assert_eq!(r.resolve_with(Some(local)).at(at("2026-01-10T12:00:00Z")).1, "PST");
    }

    #[test]
    fn changed_data_changes_the_behaviour() {
        let text = include_str!("../bin/last_route/data/app/time.json")
            .replace("\"zone\": \"local\"", "\"zone\": \"UTC\"")
            .replace("\"clock\": \"12h\"", "\"clock\": \"24h\"");
        let r = TimeRules::parse(&text).unwrap();
        let z = r.resolve_with(Some(eastern()));
        assert_eq!(r.format(at("2026-11-08T20:42:00Z"), &z), "NOV 8, 2026 20:42 UTC");
        let bad = include_str!("../bin/last_route/data/app/time.json").replace("\"12h\"", "\"13h\"");
        assert!(TimeRules::parse(&bad).is_err());
    }

    #[test]
    fn posix_rules_parse_southern_and_numeric_names() {
        let p = parse_posix("<+03>-3").unwrap();
        assert_eq!((p.std_off, p.std_abbr.as_str(), p.dst.is_none()), (3 * 3_600, "+03", true));
        let s = Zone::Posix(parse_posix("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap());
        assert_eq!(s.at(at("2026-01-15T00:00:00Z")), (11 * 3_600, "AEDT".into()));
        assert_eq!(s.at(at("2026-07-15T00:00:00Z")), (10 * 3_600, "AEST".into()));
        assert!(parse_posix("EST5EDT,J60,J300").is_none());
        assert!(parse_posix("").is_none());
    }

    /// A minimal v2 TZif file: v1 block with one UTC type, then the v2 block with the given data and footer.
    fn tzif(times: &[i64], idx: &[u8], types: &[(i32, u8)], chars: &[u8], footer: &str) -> Vec<u8> {
        let header = |timecnt: usize, typecnt: usize, charcnt: usize| {
            let mut h = b"TZif2".to_vec();
            h.extend([0u8; 15]);
            for n in [0, 0, 0, timecnt, typecnt, charcnt] {
                h.extend((n as u32).to_be_bytes());
            }
            h
        };
        let mut out = header(0, 1, 4);
        out.extend([0, 0, 0, 0, 0, 0]);
        out.extend(b"UTC\0");
        out.extend(header(times.len(), types.len(), chars.len()));
        for t in times {
            out.extend(t.to_be_bytes());
        }
        out.extend(idx);
        for (off, abbr_at) in types {
            out.extend(off.to_be_bytes());
            out.extend([0, *abbr_at]);
        }
        out.extend(chars);
        out.extend(format!("\n{footer}\n").bytes());
        out
    }

    #[test]
    fn a_tzif_file_gives_transitions_then_the_footer_rule() {
        // type 0 = EST (-5h), type 1 = EDT (-4h); one switch into EDT at 2026-03-08 07:00 UTC
        let bytes = tzif(&[at("2026-03-08T07:00:00Z")], &[1], &[(-18_000, 0), (-14_400, 4)], b"EST\0EDT\0", EASTERN_POSIX);
        let z = Zone::Tzif(parse_tzif(&bytes).expect("parses"));
        assert_eq!(z.at(at("2026-01-01T00:00:00Z")), (-18_000, "EST".into()));
        assert_eq!(z.at(at("2026-03-08T07:00:00Z")), (-14_400, "EDT".into()));
        // past the last listed switch the footer decides: November is EST again, July is EDT
        assert_eq!(z.at(at("2026-12-01T00:00:00Z")).1, "EST");
        assert_eq!(z.at(at("2027-07-01T00:00:00Z")).1, "EDT");
        assert!(parse_tzif(b"not a zone file").is_none());
        assert!(parse_tzif(&bytes[..30]).is_none());
    }

    #[test]
    fn a_windows_zone_becomes_the_same_rule_as_eastern() {
        // Eastern Standard Time as Windows reports it: bias 300, daylight bias -60,
        // daylight date = second Sunday of March 2:00, standard date = first Sunday of November 2:00
        let abbrs = rules().windows_abbr;
        let p = posix_from_windows(300, 0, -60, "Eastern Standard Time", "Eastern Daylight Time", (3, 2, 0, 7_200), (11, 1, 0, 7_200), &abbrs);
        assert_eq!(p, parse_posix(EASTERN_POSIX).unwrap());
        let none = posix_from_windows(0, 0, -60, "GMT Standard Time", "GMT Daylight Time", (0, 0, 0, 0), (0, 0, 0, 0), &abbrs);
        assert_eq!((none.std_abbr.as_str(), none.dst.is_none()), ("UTC+00:00", true));
        assert_eq!(windows_abbr("Pacific Daylight Time", -25_200, &abbrs), "PDT");
        assert_eq!(windows_abbr("Mitteleuropäische Zeit", 3_600, &abbrs), "UTC+01:00");
    }

    #[test]
    fn windows_names_outside_the_data_show_their_offset_not_made_up_initials() {
        let abbrs = rules().windows_abbr;
        assert_eq!(windows_abbr("W. Europe Standard Time", 3_600, &abbrs), "UTC+01:00");
        assert_eq!(windows_abbr("W. Europe Daylight Time", 7_200, &abbrs), "UTC+02:00");
        assert_eq!(windows_abbr("China Standard Time", 8 * 3_600, &abbrs), "UTC+08:00");
        assert_eq!(windows_abbr("Tokyo Standard Time", 9 * 3_600, &abbrs), "UTC+09:00");
        assert_eq!(windows_abbr("India Standard Time", 19_800, &abbrs), "UTC+05:30");
        assert_eq!(windows_abbr("Pacific Daylight Time", -25_200, &abbrs), "PDT");
        assert_eq!(windows_abbr("Alaskan Standard Time", -32_400, &abbrs), "AKST");
        assert_eq!(windows_abbr("Hawaiian Standard Time", -36_000, &abbrs), "HST");
        assert_eq!(windows_abbr("Atlantic Daylight Time", -10_800, &abbrs), "ADT");
        // the whole US/Canada table is in the data
        for name in ["Eastern", "Central", "Mountain", "Pacific", "Alaskan", "Atlantic"] {
            for kind in ["Standard", "Daylight"] {
                assert!(abbrs.contains_key(&format!("{name} {kind} Time")), "{name} {kind} Time");
            }
        }
        assert!(abbrs.contains_key("Hawaiian Standard Time"));
        // a Windows zone with no listed name prints as its offset through the whole format, daylight time included
        let p = posix_from_windows(-60, 0, -60, "W. Europe Standard Time", "W. Europe Daylight Time", (3, 5, 0, 7_200), (10, 5, 0, 10_800), &abbrs);
        let r = rules();
        assert_eq!(r.format(at("2026-01-15T12:00:00Z"), &Zone::Posix(p.clone())), "JAN 15, 2026 1:00 PM UTC+01:00");
        assert_eq!(r.format(at("2026-07-15T12:00:00Z"), &Zone::Posix(p)), "JUL 15, 2026 2:00 PM UTC+02:00");
        // with no table at all, offsets, never initials
        assert_eq!(windows_abbr("Eastern Standard Time", -18_000, &BTreeMap::new()), "UTC-05:00");
    }

    #[test]
    fn the_windows_abbreviations_are_read_from_the_data() {
        let src = include_str!("../bin/last_route/data/app/time.json");
        assert!(src.contains(r#""Eastern Standard Time": "EST""#));
        let r = TimeRules::parse(&src.replace(r#""Eastern Standard Time": "EST""#, r#""Eastern Standard Time": "ET""#)).unwrap();
        assert_eq!(r.windows_abbr.get("Eastern Standard Time").map(String::as_str), Some("ET"));
        let bad = src.replace(r#""Eastern Standard Time": "EST""#, r#""Eastern Standard Time": 5"#);
        assert!(TimeRules::parse(&bad).is_err(), "a non-text abbreviation is an error");
    }

    #[test]
    fn a_browser_offset_becomes_a_fixed_unnamed_zone() {
        let r = rules();
        let t = at("2026-11-08T20:42:00Z");
        let z = zone_from_offset_minutes(60);
        assert_eq!(z.at(t), (3_600, String::new()));
        assert_eq!(r.format(t, &z), "NOV 8, 2026 9:42 PM UTC+01:00");
        let z = zone_from_offset_minutes(-300);
        assert_eq!(z.at(t), (-18_000, String::new()));
        assert_eq!(r.format(t, &z), "NOV 8, 2026 3:42 PM UTC-05:00");
        let z = zone_from_offset_minutes(330);
        assert_eq!(z.at(t), (19_800, String::new()));
        assert_eq!(r.format(t, &z), "NOV 9, 2026 2:12 AM UTC+05:30");
        let z = zone_from_offset_minutes(0);
        assert_eq!(z.at(t), (0, String::new()));
        assert_eq!(r.format(t, &z), "NOV 8, 2026 8:42 PM UTC+00:00");
        match zone_from_offset_minutes(60) {
            Zone::Posix(p) => assert!(p.std_abbr.is_empty() && p.dst.is_none()),
            Zone::Tzif(_) => panic!("a fixed offset is a POSIX zone"),
        }
    }

    #[test]
    fn the_zone_the_computer_reports_resolves_without_panicking() {
        // whatever the machine says (or nothing), a zone comes back and formats
        let z = rules().resolve();
        assert!(!rules().format(at("2026-11-08T20:42:00Z"), &z).is_empty());
    }
}
