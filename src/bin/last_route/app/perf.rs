//! Frame timing and the F7 performance overlay. F7 cycles: off → small FPS readout → full panel
//! (frame-time graph, average / 1% low / worst, per-section CPU times, spike count, scene
//! counts). Sections are timed with `let _t = perf::section("hud");` (ends when dropped).
//! Spikes (frames over `spike_ms` and over 2x the average and `debug_min_ms`) are written to the debug log. Every frame
//! over `spike_ms` also goes into a rolling log of the last few (frame time and slowest section), and into the game log
//! with the sections over `spike_section_ms`, at most `spike_lines_per_sec` lines a second.
//!
//! The numbers (history length, spike threshold, log sizes, the CSV's render parts, the overlay colour bands and the
//! debug-log rule) live in data/perf.json (`lr.perf/1`), parsed once; `Rules::default()` is the Standard set.

use macroquad::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use crate::shared::clock::Instant;
use std::time::Duration;

/// The tunable numbers, from data/perf.json. The defaults are the Standard values.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Frames kept for the graph, the average and the 1% low.
    pub history: usize,
    /// Frames over this many milliseconds are logged as spikes.
    pub spike_ms: f32,
    /// Spikes in the rolling log shown on the panel.
    pub spike_log_len: usize,
    /// Spike lines written to the game log per second; the rest are counted in the next second's line.
    pub spike_lines_per_sec: u32,
    /// Sections over this many milliseconds are named on a spike's game log line.
    pub spike_section_ms: f32,
    /// Top of the frame-time graph in milliseconds, and the guide lines drawn across it.
    pub graph_max_ms: f32,
    pub guides_ms: Vec<f32>,
    /// Average frame time under `good_ms` is green, under `ok_ms` yellow, otherwise red.
    pub good_ms: f32,
    pub ok_ms: f32,
    /// A section's smoothed time keeps `section_keep` of its old value and adds `section_new` of the frame's time.
    pub section_keep: f32,
    pub section_new: f32,
    /// Debug-log spike rule: at least `debug_min_frames` frames of history, a frame over `debug_avg_ratio` times
    /// the average and over `debug_min_ms`.
    pub debug_min_frames: usize,
    pub debug_avg_ratio: f32,
    pub debug_min_ms: f32,
    /// The render parts written to the `LR_PERF_PARTS` CSV, one column each (`render 3d` is the whole pass).
    pub parts: Vec<String>,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            history: 240,
            spike_ms: 33.3,
            spike_log_len: 20,
            spike_lines_per_sec: 5,
            spike_section_ms: 2.0,
            graph_max_ms: 50.0,
            guides_ms: vec![16.7, 33.3],
            good_ms: 17.5,
            ok_ms: 34.0,
            section_keep: 0.9,
            section_new: 0.1,
            debug_min_frames: 30,
            debug_avg_ratio: 2.0,
            debug_min_ms: 25.0,
            parts: [
                "world meshes",
                "scene items",
                "doors",
                "hunters",
                "remote players",
                "viewmodel",
                "render 3d",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

fn num(v: &Value, key: &str, lo: f32, hi: f32, into: &mut f32) {
    if let Some(x) = v[key].as_f64() {
        *into = (x as f32).clamp(lo, hi);
    }
}

impl Rules {
    pub fn parse(v: &Value) -> Rules {
        let mut r = Rules::default();
        if let Some(n) = v["history"].as_u64() {
            r.history = (n as usize).clamp(1, 10_000);
        }
        num(v, "spike_ms", 0.0, 10_000.0, &mut r.spike_ms);
        if let Some(n) = v["spike_log_len"].as_u64() {
            r.spike_log_len = (n as usize).clamp(1, 1000);
        }
        if let Some(n) = v["spike_lines_per_sec"].as_u64() {
            r.spike_lines_per_sec = n.min(1000) as u32;
        }
        num(v, "spike_section_ms", 0.0, 10_000.0, &mut r.spike_section_ms);
        num(v, "graph_max_ms", 0.1, 10_000.0, &mut r.graph_max_ms);
        if let Some(a) = v["guides_ms"].as_array() {
            r.guides_ms = a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
        }
        num(v, "good_ms", 0.0, 10_000.0, &mut r.good_ms);
        num(v, "ok_ms", 0.0, 10_000.0, &mut r.ok_ms);
        num(v, "section_keep", 0.0, 1.0, &mut r.section_keep);
        num(v, "section_new", 0.0, 1.0, &mut r.section_new);
        if let Some(n) = v["debug_min_frames"].as_u64() {
            r.debug_min_frames = n as usize;
        }
        num(v, "debug_avg_ratio", 0.0, 100.0, &mut r.debug_avg_ratio);
        num(v, "debug_min_ms", 0.0, 10_000.0, &mut r.debug_min_ms);
        if let Some(a) = v["parts"].as_array() {
            let parts: Vec<String> = a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            if !parts.is_empty() {
                r.parts = parts;
            }
        }
        r
    }
}

/// The embedded rules (data/perf.json); the Standard defaults when the file is missing or broken.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("perf.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
            .map(|v| Rules::parse(&v))
            .unwrap_or_default()
    })
}

/// Where the window settings live under data/ (the embedded file's name).
const WINDOW_PATH: &str = "app/perf.json";

/// The length of the counting window of the game log's spike line and the CSV's rows (data/app/perf.json
/// `window_seconds`, Standard 1 second); a missing, broken or non-positive value reads as Standard.
fn window_len() -> Duration {
    static LEN: OnceLock<Duration> = OnceLock::new();
    *LEN.get_or_init(|| {
        crate::content::embedded::data_under(WINDOW_PATH)
            .next()
            .and_then(|(_, text)| parse_window(text))
            .unwrap_or(STANDARD_WINDOW)
    })
}

const STANDARD_WINDOW: Duration = Duration::from_secs(1);

/// Parses `{ "window_seconds" }`: a finite number above 0.
fn parse_window(text: &str) -> Option<Duration> {
    let v: Value = serde_json::from_str(text).ok()?;
    let s = v["window_seconds"].as_f64().filter(|s| s.is_finite() && *s > 0.0)?;
    Some(Duration::from_secs_f64(s))
}

/// Whether the window that began at `start` has run its length by `now`.
fn window_elapsed(start: Instant, now: Instant, window: Duration) -> bool {
    now.saturating_duration_since(start) >= window
}

/// Whether a frame of `dt` seconds is a spike (logged on the game log and the panel's spike log).
fn is_spike(r: &Rules, dt: f32) -> bool {
    dt * 1000.0 > r.spike_ms
}

/// Whether a frame is a debug-log spike: enough history, over `debug_avg_ratio` times the average and over `debug_min_ms`.
fn is_debug_spike(r: &Rules, dt: f32, avg: f32, frames: usize) -> bool {
    frames >= r.debug_min_frames && dt > avg * r.debug_avg_ratio && dt > r.debug_min_ms / 1000.0
}

/// One logged spike: the frame time and the section that took longest in that frame.
#[derive(Clone, Copy)]
struct Spike {
    ms: f32,
    section: &'static str,
    section_ms: f32,
}

/// Rolling log of the last `cap` spikes, oldest overwritten first. `total` counts every spike seen.
/// Pushing reuses the existing slots, so it does not allocate per frame.
struct SpikeLog {
    cap: usize,
    entries: Vec<Spike>,
    head: usize,
    total: u32,
    window: SpikeWindow,
}

/// The current one-second window of the game log: spikes seen, spike lines written, and the draw
/// calls over the frames of the window.
#[derive(Default)]
struct SpikeWindow {
    start: Option<Instant>,
    spikes: u32,
    logged: u32,
    frames: u32,
    draws: u64,
    draws_max: u32,
}

impl SpikeLog {
    fn new(cap: usize) -> SpikeLog {
        SpikeLog {
            cap: cap.max(1),
            entries: Vec::new(),
            head: 0,
            total: 0,
            window: SpikeWindow::default(),
        }
    }

    fn push(&mut self, spike: Spike) {
        self.total += 1;
        if self.entries.len() < self.cap {
            self.entries.push(spike);
        } else {
            self.entries[self.head] = spike;
        }
        self.head = (self.head + 1) % self.cap;
    }

    /// The most recent spike, if any.
    fn latest(&self) -> Option<&Spike> {
        self.entries.get((self.head + self.cap - 1) % self.cap)
    }

    /// Counts a frame and its draw calls in the window. When the window ends, the spikes and draw
    /// calls of the second that just ended go to the game log.
    fn tick(&mut self, now: Instant, draws: u32) {
        self.tick_in(window_len(), now, draws);
    }

    /// `tick` with the window length given (the tests use short ones).
    fn tick_in(&mut self, window: Duration, now: Instant, draws: u32) {
        let w = &mut self.window;
        let start = *w.start.get_or_insert(now);
        if window_elapsed(start, now, window) {
            if w.spikes > 0 {
                crate::lr_info!(
                    "frame spikes last second: {} total, {} suppressed; draw calls per frame avg {:.0}, max {}",
                    w.spikes,
                    w.spikes - w.logged,
                    w.draws as f64 / w.frames.max(1) as f64,
                    w.draws_max
                );
            }
            *w = SpikeWindow {
                start: Some(now),
                ..SpikeWindow::default()
            };
        }
        w.frames += 1;
        w.draws += u64::from(draws);
        w.draws_max = w.draws_max.max(draws);
    }

    /// Writes a spike to the game log with its sections over `spike_section_ms`, or only counts it
    /// once this second has `spike_lines_per_sec` lines.
    fn log_spike(&mut self, r: &Rules, ms: f32, sections: &[(&'static str, f32)]) {
        let w = &mut self.window;
        w.spikes += 1;
        if w.logged >= r.spike_lines_per_sec {
            return;
        }
        w.logged += 1;
        let slow: Vec<String> = sections
            .iter()
            .filter(|(_, t)| *t > r.spike_section_ms)
            .map(|(name, t)| format!("{name} {t:.1} ms"))
            .collect();
        let cause = if slow.is_empty() {
            format!("no section over {} ms", r.spike_section_ms)
        } else {
            slow.join(", ")
        };
        crate::lr_info!("frame spike {:.1} ms; {}", ms, cause);
    }
}

/// Per-second means (ms per frame) of the render parts, from the raw section times of each frame. Opened only when the
/// `LR_PERF_PARTS` environment variable names a file; a measurement aid that changes no drawing.
struct PartsLog {
    file: std::fs::File,
    start: Option<Instant>,
    second: u32,
    frames: u32,
    parts: Vec<String>,
    sums: Vec<f32>,
}

impl PartsLog {
    /// The CSV header: `second,frames` then one `<part>_ms` column per part, spaces as underscores.
    fn header(parts: &[String]) -> String {
        let mut header = String::from("second,frames");
        for p in parts {
            header.push_str(&format!(",{}_ms", p.replace(' ', "_")));
        }
        header
    }

    fn open(parts: &[String]) -> Option<PartsLog> {
        use std::io::Write;
        let path = std::env::var("LR_PERF_PARTS").ok()?;
        let mut file = std::fs::File::create(path).ok()?;
        writeln!(file, "{}", PartsLog::header(parts)).ok()?;
        Some(PartsLog {
            file,
            start: None,
            second: 0,
            frames: 0,
            parts: parts.to_vec(),
            sums: vec![0.0; parts.len()],
        })
    }

    /// Counts one frame's section times; when a second has passed, writes that second's row.
    fn tick(&mut self, now: Instant, secs: &[(&'static str, f32)]) {
        use std::io::Write;
        let start = *self.start.get_or_insert(now);
        if window_elapsed(start, now, window_len()) {
            let n = self.frames.max(1) as f32;
            let mut row = format!("{},{}", self.second, self.frames);
            for s in &self.sums {
                row.push_str(&format!(",{:.3}", s / n));
            }
            let _ = writeln!(self.file, "{row}");
            for s in self.sums.iter_mut() {
                *s = 0.0;
            }
            self.frames = 0;
            self.second += 1;
            self.start = Some(now);
        }
        self.frames += 1;
        for (i, name) in self.parts.iter().enumerate() {
            if let Some(e) = secs.iter().find(|e| e.0 == name.as_str()) {
                self.sums[i] += e.1;
            }
        }
    }
}

struct State {
    parts: Option<PartsLog>,
    parts_checked: bool,
    frames: Vec<f32>,
    head: usize,
    sections: BTreeMap<&'static str, f32>,
    // raw section times of the frame in progress; cleared each frame so a spike names its own
    // slowest section rather than a smoothed average
    frame_secs: Vec<(&'static str, f32)>,
    open: BTreeMap<&'static str, Instant>,
    spikes: SpikeLog,
    mode: u8,
    counters: BTreeMap<&'static str, f64>,
}

impl State {
    fn new() -> State {
        State {
            parts: None,
            parts_checked: false,
            frames: Vec::new(),
            head: 0,
            sections: BTreeMap::new(),
            frame_secs: Vec::new(),
            open: BTreeMap::new(),
            spikes: SpikeLog::new(rules().spike_log_len),
            mode: 0,
            counters: BTreeMap::new(),
        }
    }
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::new());
}

/// Records one frame's duration in seconds and the draw calls it made.
pub fn frame(dt: f32) {
    let draws = crate::render::builder::take_draw_count();
    let r = rules();
    S.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        let avg = if s.frames.is_empty() {
            dt
        } else {
            s.frames.iter().sum::<f32>() / s.frames.len() as f32
        };
        s.spikes.tick(Instant::now(), draws);
        if is_spike(r, dt) {
            let (section, section_ms) = s
                .frame_secs
                .iter()
                .copied()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or(("none", 0.0));
            s.spikes.push(Spike {
                ms: dt * 1000.0,
                section,
                section_ms,
            });
            s.spikes.log_spike(r, dt * 1000.0, &s.frame_secs);
        }
        if is_debug_spike(r, dt, avg, s.frames.len()) {
            let worst = s
                .sections
                .iter()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(k, v)| format!("{k} {v:.1} ms"))
                .unwrap_or_default();
            crate::lr_debug!(
                "frame spike {:.1} ms (avg {:.1} ms); slowest section {worst}",
                dt * 1000.0,
                avg * 1000.0
            );
        }
        if !s.parts_checked {
            s.parts_checked = true;
            s.parts = PartsLog::open(&r.parts);
        }
        if let Some(p) = s.parts.as_mut() {
            p.tick(Instant::now(), &s.frame_secs);
        }
        s.frame_secs.clear();
        if s.frames.len() < r.history {
            s.frames.push(dt);
        } else {
            let h = s.head;
            s.frames[h] = dt;
        }
        s.head = (s.head + 1) % r.history;
    });
    counter("draws", f64::from(draws));
}

pub struct Section(&'static str);

/// Times a named section of this frame until the returned guard is dropped.
pub fn section(name: &'static str) -> Section {
    S.with(|s| s.borrow_mut().open.insert(name, Instant::now()));
    Section(name)
}

impl Drop for Section {
    fn drop(&mut self) {
        let r = rules();
        S.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(t) = s.open.remove(self.0) {
                let ms = t.elapsed().as_secs_f32() * 1000.0;
                let e = s.sections.entry(self.0).or_insert(ms);
                *e = *e * r.section_keep + ms * r.section_new;
                match s.frame_secs.iter_mut().find(|e| e.0 == self.0) {
                    Some(e) => e.1 += ms,
                    None => s.frame_secs.push((self.0, ms)),
                }
            }
        });
    }
}

/// A named value shown on the full panel (e.g. hunters, props, meshes).
pub fn counter(name: &'static str, v: f64) {
    S.with(|s| s.borrow_mut().counters.insert(name, v));
}

/// A section's smoothed time in ms, read by the stress run (app/stress.rs); 0 until it has run.
pub fn section_ms(name: &str) -> f32 {
    S.with(|s| s.borrow().sections.get(name).copied().unwrap_or(0.0))
}

/// F7 cycles the overlay mode.
pub fn handle_keys() {
    if is_key_pressed(KeyCode::F7) {
        S.with(|s| {
            let mut s = s.borrow_mut();
            s.mode = (s.mode + 1) % 3;
        });
    }
}

pub fn mode() -> u8 {
    S.with(|s| s.borrow().mode)
}

/// (fps from the average, average ms, 1% low fps, worst ms)
pub fn stats() -> (f32, f32, f32, f32) {
    S.with(|s| {
        let s = s.borrow();
        if s.frames.is_empty() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let avg = s.frames.iter().sum::<f32>() / s.frames.len() as f32;
        let mut sorted = s.frames.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let k = (sorted.len() / 100).max(1);
        let low = sorted[..k].iter().sum::<f32>() / k as f32;
        (
            1.0 / avg.max(1e-4),
            avg * 1000.0,
            1.0 / low.max(1e-4),
            sorted[0] * 1000.0,
        )
    })
}

/// Draws the overlay for the current mode in the top-left corner (under the HUD's own FPS).
pub fn draw_overlay(font: Option<&Font>, extra: &[(String, String)]) {
    let mode = mode();
    if mode == 0 {
        return;
    }
    let r = rules();
    let (fps, avg, low, worst) = stats();
    let tp = |size: u16, color: Color| TextParams {
        font,
        font_size: size,
        color,
        ..Default::default()
    };
    let (x, mut y) = (10.0, screen_height() * 0.12);
    if mode == 1 {
        let t = format!("{fps:.0} FPS  {avg:.1} MS");
        draw_rectangle(
            x - 4.0,
            y - 14.0,
            150.0,
            20.0,
            Color::new(0.0, 0.0, 0.0, 0.55),
        );
        draw_text_ex(&t, x, y, tp(14, WHITE));
        return;
    }
    let w = 300.0;
    let lines = 8 + S.with(|s| s.borrow().sections.len() + s.borrow().counters.len()) + extra.len();
    let h = 62.0 + lines as f32 * 15.0;
    draw_rectangle(x - 6.0, y - 16.0, w, h, Color::new(0.02, 0.01, 0.03, 0.78));
    draw_rectangle_lines(
        x - 6.0,
        y - 16.0,
        w,
        h,
        1.0,
        Color::new(0.45, 0.3, 0.5, 1.0),
    );
    let ok = if avg < r.good_ms {
        GREEN
    } else if avg < r.ok_ms {
        YELLOW
    } else {
        RED
    };
    draw_text_ex(
        &format!("PERFORMANCE // F7  {fps:.0} FPS"),
        x,
        y,
        tp(15, ok),
    );
    y += 16.0;
    // the frame-time graph: last `history` frames, with the guide lines from data/perf.json
    let gh = 40.0;
    let gy = y;
    draw_rectangle(x, gy, w - 12.0, gh, Color::new(1.0, 1.0, 1.0, 0.04));
    for (i, ms) in r.guides_ms.iter().enumerate() {
        let c = if i == 0 {
            Color::new(0.3, 0.9, 0.4, 0.4)
        } else {
            Color::new(0.9, 0.4, 0.3, 0.4)
        };
        let ly = gy + gh - (ms / r.graph_max_ms * gh).min(gh);
        draw_line(x, ly, x + w - 12.0, ly, 1.0, c);
    }
    S.with(|s| {
        let s = s.borrow();
        let n = s.frames.len();
        for i in 0..n {
            let f = s.frames[(s.head + r.history - n + i) % r.history.max(n)] * 1000.0;
            let bh = (f / r.graph_max_ms * gh).min(gh);
            let bx = x + i as f32 * (w - 12.0) / r.history as f32;
            let c = if f < r.good_ms {
                Color::new(0.4, 0.9, 0.5, 0.9)
            } else if f < r.ok_ms {
                YELLOW
            } else {
                RED
            };
            draw_line(bx, gy + gh, bx, gy + gh - bh, 1.0, c);
        }
    });
    y = gy + gh + 16.0;
    let mut row = |k: &str, v: String| {
        draw_text_ex(k, x, y, tp(13, Color::new(0.75, 0.7, 0.8, 1.0)));
        draw_text_ex(&v, x + 150.0, y, tp(13, WHITE));
        y += 15.0;
    };
    row("frame avg", format!("{avg:.2} ms"));
    row("1% low", format!("{low:.0} fps"));
    row("worst (4 s)", format!("{worst:.1} ms"));
    row(
        &format!("spikes >{:.0} ms", r.spike_ms),
        S.with(|s| s.borrow().spikes.total.to_string()),
    );
    row(
        "last spike",
        S.with(|s| {
            let s = s.borrow();
            match s.spikes.latest() {
                Some(sp) => format!("{:.1} ms, {} {:.1} ms", sp.ms, sp.section, sp.section_ms),
                None => "none".to_string(),
            }
        }),
    );
    row(
        "window",
        format!("{:.0} x {:.0}", screen_width(), screen_height()),
    );
    let (secs, ctrs) = S.with(|s| (s.borrow().sections.clone(), s.borrow().counters.clone()));
    for (k, v) in secs {
        row(k, format!("{v:.2} ms"));
    }
    for (k, v) in ctrs {
        row(k, format!("{v:.0}"));
    }
    for (k, v) in extra {
        row(k, v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spike(ms: f32) -> Spike {
        Spike {
            ms,
            section: "hud",
            section_ms: 1.0,
        }
    }

    fn parsed(text: &str) -> Rules {
        Rules::parse(&serde_json::from_str::<Value>(text).expect("valid json"))
    }

    #[test]
    fn data_file_matches_standard_defaults() {
        let v: Value = serde_json::from_str(include_str!("../data/perf.json")).expect("perf.json parses");
        assert_eq!(v["schema"].as_str(), Some("lr.perf/1"));
        assert_eq!(Rules::parse(&v), Rules::default());
        assert_eq!(Rules::default().history, 240);
        assert_eq!(Rules::default().spike_ms, 33.3);
        assert_eq!(Rules::default().spike_log_len, 20);
        assert_eq!(Rules::default().spike_lines_per_sec, 5);
        assert_eq!(Rules::default().parts.len(), 7);
    }

    #[test]
    fn changing_spike_threshold_in_data_changes_spike_detection() {
        let std_rules = Rules::default();
        let tight = parsed(r#"{ "spike_ms": 20.0 }"#);
        assert!(!is_spike(&std_rules, 0.025));
        assert!(is_spike(&tight, 0.025));
        assert!(!is_spike(&tight, 0.019));
    }

    #[test]
    fn debug_spike_rule_reads_frames_ratio_and_ms_from_data() {
        let std_rules = Rules::default();
        assert!(!is_debug_spike(&std_rules, 0.1, 0.01, 29));
        assert!(is_debug_spike(&std_rules, 0.1, 0.01, 30));
        assert!(!is_debug_spike(&std_rules, 0.02, 0.01, 100));
        let loose = parsed(r#"{ "debug_min_frames": 5, "debug_avg_ratio": 1.5, "debug_min_ms": 10.0 }"#);
        assert!(is_debug_spike(&loose, 0.02, 0.01, 5));
        assert!(!is_debug_spike(&loose, 0.009, 0.001, 5));
    }

    #[test]
    fn spike_log_length_comes_from_data() {
        let mut log = SpikeLog::new(parsed(r#"{ "spike_log_len": 5 }"#).spike_log_len);
        for i in 1..=8 {
            log.push(spike(i as f32));
        }
        assert_eq!(log.total, 8);
        assert_eq!(log.entries.len(), 5);
        let mut kept: Vec<f32> = log.entries.iter().map(|s| s.ms).collect();
        kept.sort_by(|a, b| a.total_cmp(b));
        assert_eq!(kept, vec![4.0, 5.0, 6.0, 7.0, 8.0]);
        assert_eq!(log.latest().map(|s| s.ms), Some(8.0));
    }

    #[test]
    fn spike_log_keeps_only_the_last_twenty() {
        let mut log = SpikeLog::new(Rules::default().spike_log_len);
        assert!(log.latest().is_none());
        for i in 1..=25 {
            log.push(spike(i as f32));
        }
        assert_eq!(log.total, 25);
        assert_eq!(log.entries.len(), 20);
        let mut kept: Vec<f32> = log.entries.iter().map(|s| s.ms).collect();
        kept.sort_by(|a, b| a.total_cmp(b));
        let expected: Vec<f32> = (6..=25).map(|i| i as f32).collect();
        assert_eq!(kept, expected);
        assert_eq!(log.latest().map(|s| s.ms), Some(25.0));
    }

    #[test]
    fn spike_log_latest_tracks_pushes_before_it_is_full() {
        let mut log = SpikeLog::new(20);
        log.push(spike(34.0));
        assert_eq!(log.latest().map(|s| s.ms), Some(34.0));
        log.push(spike(50.0));
        assert_eq!(log.latest().map(|s| s.ms), Some(50.0));
        assert_eq!(log.total, 2);
    }

    #[test]
    fn csv_header_is_unchanged_by_default_and_follows_data_parts() {
        assert_eq!(
            PartsLog::header(&Rules::default().parts),
            "second,frames,world_meshes_ms,scene_items_ms,doors_ms,hunters_ms,remote_players_ms,viewmodel_ms,render_3d_ms"
        );
        let custom = parsed(r#"{ "parts": ["hunters"] }"#);
        assert_eq!(PartsLog::header(&custom.parts), "second,frames,hunters_ms");
        let empty = parsed(r#"{ "parts": [] }"#);
        assert_eq!(empty.parts, Rules::default().parts);
    }

    #[test]
    fn history_and_colour_bands_come_from_data() {
        let r = parsed(r#"{ "history": 60, "good_ms": 10.0, "ok_ms": 20.0, "guides_ms": [8.0] }"#);
        assert_eq!(r.history, 60);
        assert_eq!(r.good_ms, 10.0);
        assert_eq!(r.ok_ms, 20.0);
        assert_eq!(r.guides_ms, vec![8.0]);
        assert_eq!(parsed(r#"{ "history": 0 }"#).history, 1);
        assert_eq!(Rules::default().good_ms, 17.5);
        assert_eq!(Rules::default().ok_ms, 34.0);
    }

    #[test]
    fn section_smoothing_weights_come_from_data() {
        let r = Rules::default();
        assert!((r.section_keep + r.section_new - 1.0).abs() < 1e-6);
        let custom = parsed(r#"{ "section_keep": 0.5, "section_new": 0.5 }"#);
        assert_eq!((custom.section_keep, custom.section_new), (0.5, 0.5));
    }

    #[test]
    fn window_data_matches_standard_and_parses_strictly() {
        let text = include_str!("../data/app/perf.json");
        assert_eq!(parse_window(text), Some(STANDARD_WINDOW));
        assert_eq!(window_len(), STANDARD_WINDOW);
        assert_eq!(parse_window(r#"{ "window_seconds": 0.5 }"#), Some(Duration::from_millis(500)));
        assert_eq!(parse_window(r#"{ "window_seconds": 0 }"#), None);
        assert_eq!(parse_window("{}"), None);
    }

    #[test]
    fn window_length_decides_when_a_window_ends_on_an_injected_clock() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert!(!window_elapsed(t0, at(999), STANDARD_WINDOW));
        assert!(window_elapsed(t0, at(1000), STANDARD_WINDOW));
        assert!(window_elapsed(t0, at(250), Duration::from_millis(250)));
        // a clock reading before the start never ends the window
        assert!(!window_elapsed(at(500), t0, STANDARD_WINDOW));
    }

    #[test]
    fn spike_window_restarts_after_the_data_length() {
        let t0 = Instant::now();
        let mut log = SpikeLog::new(5);
        let short = Duration::from_millis(100);
        log.tick_in(short, t0, 10);
        log.tick_in(short, t0 + Duration::from_millis(50), 30);
        assert_eq!((log.window.frames, log.window.draws_max), (2, 30));
        // past the window: it restarts, this frame counted in the new one
        log.tick_in(short, t0 + Duration::from_millis(120), 7);
        assert_eq!((log.window.frames, log.window.draws, log.window.draws_max), (1, 7, 7));
        // the same gap under a one second window does not restart it
        let mut std_log = SpikeLog::new(5);
        std_log.tick_in(STANDARD_WINDOW, t0, 10);
        std_log.tick_in(STANDARD_WINDOW, t0 + Duration::from_millis(120), 7);
        assert_eq!(std_log.window.frames, 2);
    }
}
