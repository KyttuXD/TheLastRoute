//! Frame timing and the F7 performance overlay. F7 cycles: off → small FPS readout → full panel
//! (frame-time graph, average / 1% low / worst, per-section CPU times, spike count, scene
//! counts). Sections are timed with `let _t = perf::section("hud");` (ends when dropped).
//! Spikes (frames over 2x the average and over 25 ms) are written to the debug log. Every frame
//! over 33.3 ms also goes into a rolling log of the last 20 (frame time and slowest section), and
//! into the game log with the sections over 2 ms, at most 5 lines a second.

use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const HISTORY: usize = 240;
/// Frames over this many milliseconds are logged as spikes.
const SPIKE_MS: f32 = 33.3;
const SPIKE_LOG_LEN: usize = 20;
/// Spike lines written to the game log per second; the rest are counted in the next second's line.
const SPIKE_LINES_PER_SEC: u32 = 5;
/// Sections over this many milliseconds are named on a spike's game log line.
const SPIKE_SECTION_MS: f32 = 2.0;

/// One logged spike: the frame time and the section that took longest in that frame.
#[derive(Clone, Copy)]
struct Spike {
    ms: f32,
    section: &'static str,
    section_ms: f32,
}

/// Rolling log of the last `SPIKE_LOG_LEN` spikes, oldest overwritten first. `total` counts every
/// spike seen. Pushing reuses the existing slots, so it does not allocate per frame.
#[derive(Default)]
struct SpikeLog {
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
    fn push(&mut self, spike: Spike) {
        self.total += 1;
        if self.entries.len() < SPIKE_LOG_LEN {
            self.entries.push(spike);
        } else {
            self.entries[self.head] = spike;
        }
        self.head = (self.head + 1) % SPIKE_LOG_LEN;
    }

    /// The most recent spike, if any.
    fn latest(&self) -> Option<&Spike> {
        self.entries.get((self.head + SPIKE_LOG_LEN - 1) % SPIKE_LOG_LEN)
    }

    /// Counts a frame and its draw calls in the window. When the window ends, the spikes and draw
    /// calls of the second that just ended go to the game log.
    fn tick(&mut self, now: Instant, draws: u32) {
        let w = &mut self.window;
        let start = *w.start.get_or_insert(now);
        if now.duration_since(start) >= Duration::from_secs(1) {
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

    /// Writes a spike to the game log with its sections over `SPIKE_SECTION_MS`, or only counts it
    /// once this second has `SPIKE_LINES_PER_SEC` lines.
    fn log_spike(&mut self, ms: f32, sections: &[(&'static str, f32)]) {
        let w = &mut self.window;
        w.spikes += 1;
        if w.logged >= SPIKE_LINES_PER_SEC {
            return;
        }
        w.logged += 1;
        let slow: Vec<String> = sections
            .iter()
            .filter(|(_, t)| *t > SPIKE_SECTION_MS)
            .map(|(name, t)| format!("{name} {t:.1} ms"))
            .collect();
        let cause = if slow.is_empty() {
            format!("no section over {SPIKE_SECTION_MS} ms")
        } else {
            slow.join(", ")
        };
        crate::lr_info!("frame spike {:.1} ms; {}", ms, cause);
    }
}

/// The render parts written to the `LR_PERF_PARTS` CSV, one row a second. Each is a section name; `render 3d` is the whole pass.
const PARTS: [&str; 7] = [
    "world meshes",
    "scene items",
    "doors",
    "hunters",
    "remote players",
    "viewmodel",
    "render 3d",
];

/// Per-second means (ms per frame) of the render parts, from the raw section times of each frame. Opened only when the
/// `LR_PERF_PARTS` environment variable names a file; a measurement aid that changes no drawing.
struct PartsLog {
    file: std::fs::File,
    start: Option<Instant>,
    second: u32,
    frames: u32,
    sums: [f32; PARTS.len()],
}

impl PartsLog {
    fn open() -> Option<PartsLog> {
        use std::io::Write;
        let path = std::env::var("LR_PERF_PARTS").ok()?;
        let mut file = std::fs::File::create(path).ok()?;
        let mut header = String::from("second,frames");
        for p in PARTS {
            header.push_str(&format!(",{}_ms", p.replace(' ', "_")));
        }
        writeln!(file, "{header}").ok()?;
        Some(PartsLog {
            file,
            start: None,
            second: 0,
            frames: 0,
            sums: [0.0; PARTS.len()],
        })
    }

    /// Counts one frame's section times; when a second has passed, writes that second's row.
    fn tick(&mut self, now: Instant, secs: &[(&'static str, f32)]) {
        use std::io::Write;
        let start = *self.start.get_or_insert(now);
        if now.duration_since(start) >= Duration::from_secs(1) {
            let n = self.frames.max(1) as f32;
            let mut row = format!("{},{}", self.second, self.frames);
            for s in self.sums {
                row.push_str(&format!(",{:.3}", s / n));
            }
            let _ = writeln!(self.file, "{row}");
            self.sums = [0.0; PARTS.len()];
            self.frames = 0;
            self.second += 1;
            self.start = Some(now);
        }
        self.frames += 1;
        for (i, name) in PARTS.iter().enumerate() {
            if let Some(e) = secs.iter().find(|e| e.0 == *name) {
                self.sums[i] += e.1;
            }
        }
    }
}

#[derive(Default)]
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

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

/// Records one frame's duration in seconds and the draw calls it made.
pub fn frame(dt: f32) {
    let draws = crate::render::builder::take_draw_count();
    S.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        let avg = if s.frames.is_empty() {
            dt
        } else {
            s.frames.iter().sum::<f32>() / s.frames.len() as f32
        };
        s.spikes.tick(Instant::now(), draws);
        if dt * 1000.0 > SPIKE_MS {
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
            s.spikes.log_spike(dt * 1000.0, &s.frame_secs);
        }
        if s.frames.len() >= 30 && dt > avg * 2.0 && dt > 0.025 {
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
            s.parts = PartsLog::open();
        }
        if let Some(p) = s.parts.as_mut() {
            p.tick(Instant::now(), &s.frame_secs);
        }
        s.frame_secs.clear();
        if s.frames.len() < HISTORY {
            s.frames.push(dt);
        } else {
            let h = s.head;
            s.frames[h] = dt;
        }
        s.head = (s.head + 1) % HISTORY;
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
        S.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(t) = s.open.remove(self.0) {
                let ms = t.elapsed().as_secs_f32() * 1000.0;
                let e = s.sections.entry(self.0).or_insert(ms);
                *e = *e * 0.9 + ms * 0.1;
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
    let ok = if avg < 17.5 {
        GREEN
    } else if avg < 34.0 {
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
    // the frame-time graph: last 240 frames, 16.7 and 33.3 ms guides
    let gh = 40.0;
    let gy = y;
    draw_rectangle(x, gy, w - 12.0, gh, Color::new(1.0, 1.0, 1.0, 0.04));
    for (ms, c) in [
        (16.7, Color::new(0.3, 0.9, 0.4, 0.4)),
        (33.3, Color::new(0.9, 0.4, 0.3, 0.4)),
    ] {
        let ly = gy + gh - (ms / 50.0 * gh).min(gh);
        draw_line(x, ly, x + w - 12.0, ly, 1.0, c);
    }
    S.with(|s| {
        let s = s.borrow();
        let n = s.frames.len();
        for i in 0..n {
            let f = s.frames[(s.head + HISTORY - n + i) % HISTORY.max(n)] * 1000.0;
            let bh = (f / 50.0 * gh).min(gh);
            let bx = x + i as f32 * (w - 12.0) / HISTORY as f32;
            let c = if f < 17.5 {
                Color::new(0.4, 0.9, 0.5, 0.9)
            } else if f < 34.0 {
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
    row("spikes >33 ms", S.with(|s| s.borrow().spikes.total.to_string()));
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

    #[test]
    fn spike_log_keeps_only_the_last_twenty() {
        let mut log = SpikeLog::default();
        assert!(log.latest().is_none());
        for i in 1..=25 {
            log.push(spike(i as f32));
        }
        assert_eq!(log.total, 25);
        assert_eq!(log.entries.len(), SPIKE_LOG_LEN);
        let mut kept: Vec<f32> = log.entries.iter().map(|s| s.ms).collect();
        kept.sort_by(|a, b| a.total_cmp(b));
        let expected: Vec<f32> = (6..=25).map(|i| i as f32).collect();
        assert_eq!(kept, expected);
        assert_eq!(log.latest().map(|s| s.ms), Some(25.0));
    }

    #[test]
    fn spike_log_latest_tracks_pushes_before_it_is_full() {
        let mut log = SpikeLog::default();
        log.push(spike(34.0));
        assert_eq!(log.latest().map(|s| s.ms), Some(34.0));
        log.push(spike(50.0));
        assert_eq!(log.latest().map(|s| s.ms), Some(50.0));
        assert_eq!(log.total, 2);
    }
}
