//! Stress mode: `last_route --stress SECONDS --stress-log PATH` (main.rs reads the flags; SECONDS
//! defaults to 60, PATH to `stress.csv`).
//!
//! A solo run on the fixed seed `SEED` = 20261101 (room text `route::room_text(SEED)`, the same seed
//! the screenshot runs use), the Infinite route from the top level (the start CREATE ROOM gives). It skips the title and the boot card, forces vsync off (swap interval 0) and does
//! not apply the FPS LIMIT, so the frame times are the game's own. The saved settings file is
//! never written. The menus, the pause menu and the admin tools take no input in this mode, and a
//! run that is caught starts again on the same seed.
//!
//! The synthetic input is a `Controls` fed to `Game::update` the way `read_controls` feeds the
//! keyboard and mouse, so it walks the same movement code: forward all the time, sprint on the odd
//! seconds, and a quarter turn at each 4 s mark. The turn direction comes from a fixed-seed
//! sequence, so the route covers new ground and repeats only by chance.
//!
//! The log is CSV: a header, one row per whole second (`second` counts from 0), then a
//! `# summary` line once SECONDS have passed. Each row is flushed as it is written, so a crash
//! leaves the rows so far. The columns are `second, fps (frames that second), ms_avg, ms_p99`
//! (the 99th percentile frame time), then `update_ms, render_ms, hud_ms`, `hunters` (active) and
//! `depth`. The three section times are the perf sections' smoothed values, sampled once a frame
//! and averaged over the second.

use super::{perf, World};
use crate::config::Difficulty;
use crate::game::{Controls, Game};
use crate::world::route;
use macroquad::prelude::Vec2;
use std::f32::consts::FRAC_PI_2;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

/// The run's seed. The screenshot runs start on the same seed.
const SEED: u32 = 20261101;
/// A turn starts at each mark this many seconds apart and swings round over `TURN_SECONDS`.
const TURN_EVERY: f32 = 4.0;
const TURN_SECONDS: f32 = 0.25;
const HEADER: &str = "second,fps,ms_avg,ms_p99,update_ms,render_ms,hud_ms,hunters,depth";

/// What `--stress` asked for.
pub struct Config {
    pub seconds: f32,
    pub log: String,
}

/// The stress run: its clock, the open log, the second being counted and every frame time.
pub struct Stress {
    seconds: f32,
    log: BufWriter<File>,
    /// Set on the first frame of the run, so the load before it is not counted.
    start: Option<Instant>,
    /// The whole second being counted; its row is written when the clock passes it.
    second: u32,
    row: Row,
    /// Every frame of the run in ms, for the summary.
    frame_ms: Vec<f32>,
    /// The hunters and depth at the last frame, written into each row.
    hunters: usize,
    depth: i32,
}

/// The frames of one second: their times and the sum of each section's samples.
#[derive(Default)]
struct Row {
    frame_ms: Vec<f32>,
    update_ms: f32,
    render_ms: f32,
    hud_ms: f32,
}

impl Stress {
    /// Creates the log and writes its header. The clock starts on the first frame.
    pub fn open(cfg: Config) -> Result<Stress, String> {
        if let Some(dir) = std::path::Path::new(&cfg.log).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = File::create(&cfg.log).map_err(|e| format!("cannot create {}: {e}", cfg.log))?;
        let mut log = BufWriter::new(file);
        writeln!(log, "{}", HEADER)
            .and_then(|()| log.flush())
            .map_err(|e| format!("cannot write {}: {e}", cfg.log))?;
        crate::lr_info!("stress run: {} s on seed {}, log {}", cfg.seconds, SEED, cfg.log);
        Ok(Stress {
            seconds: cfg.seconds,
            log,
            start: None,
            second: 0,
            row: Row::default(),
            frame_ms: Vec::new(),
            hunters: 0,
            depth: 0,
        })
    }

    /// Counts one frame of `dt` seconds, with the run `g` (none while a run is being built).
    /// Returns true once SECONDS have passed.
    pub fn frame(&mut self, dt: f32, g: Option<&Game>) -> bool {
        let Some(start) = self.start else {
            self.start = Some(Instant::now());
            return false;
        };
        let t = start.elapsed().as_secs_f32();
        let whole = t as u32;
        while self.second < whole {
            self.write_row();
        }
        let ms = dt * 1000.0;
        self.row.frame_ms.push(ms);
        self.frame_ms.push(ms);
        self.row.update_ms += perf::section_ms("update");
        self.row.render_ms += perf::section_ms("render 3d");
        self.row.hud_ms += perf::section_ms("hud");
        let (hunters, depth) = g.map_or((0, 0), |g| {
            (g.hunters.iter().filter(|h| h.active).count(), g.depth)
        });
        self.hunters = hunters;
        self.depth = depth;
        t >= self.seconds
    }

    /// The synthetic input for this frame; `dt` is the frame's game time.
    pub fn controls(&self, dt: f32) -> Controls {
        let t = self.start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        Controls {
            fwd: 1.0,
            sprint: (t as u32) % 2 == 1,
            look: Vec2::new(turn_look(t, dt), 0.0),
            ..Default::default()
        }
    }

    /// Writes the summary to the log and to stdout. The caller ends the process.
    pub fn finish(&mut self) {
        let secs = self.start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        let total = self.frame_ms.len();
        let avg_fps = if secs > 0.0 { total as f32 / secs } else { 0.0 };
        let worst = self.frame_ms.iter().copied().fold(0.0, f32::max);
        let line = format!(
            "# summary total_frames={} avg_fps={:.2} low_1pct_fps={:.2} worst_frame_ms={:.2}",
            total,
            avg_fps,
            one_percent_low_fps(&self.frame_ms),
            worst
        );
        self.emit(&line);
        println!("{}", line);
    }

    /// Writes the row for `self.second` and moves on to the next second.
    fn write_row(&mut self) {
        let row = std::mem::take(&mut self.row);
        let frames = row.frame_ms.len();
        let mean = |sum: f32| if frames == 0 { 0.0 } else { sum / frames as f32 };
        let ms_avg = mean(row.frame_ms.iter().sum::<f32>());
        let line = format!(
            "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{},{}",
            self.second,
            frames,
            ms_avg,
            percentile(&row.frame_ms, 99.0),
            mean(row.update_ms),
            mean(row.render_ms),
            mean(row.hud_ms),
            self.hunters,
            self.depth,
        );
        self.emit(&line);
        self.second += 1;
    }

    /// Appends one line to the log and flushes it.
    fn emit(&mut self, line: &str) {
        if let Err(e) = writeln!(self.log, "{}", line).and_then(|()| self.log.flush()) {
            eprintln!("stress: cannot write the log: {e}");
        }
    }
}

/// The game a stress run plays: the Infinite route from the top level, as CREATE ROOM starts one.
pub fn start_game(w: &World) -> Game {
    Game::new_infinite(
        Difficulty::Standard,
        SEED,
        &route::room_text(SEED),
        0,
        &w.content,
        &w.catalog,
        &w.assets,
    )
}

/// The sign of the turn at mark `turn` (the first mark is 0): a fixed multiplicative hash, so the
/// direction sequence is the same on every run.
fn turn_sign(turn: u32) -> f32 {
    let h = turn.wrapping_mul(2_654_435_761).wrapping_add(SEED);
    if h >> 31 == 0 {
        1.0
    } else {
        -1.0
    }
}

/// The look for a frame at `t` seconds from the start: a quarter turn spread over the first
/// `TURN_SECONDS` after each mark. Positive look turns right (`Player::look` subtracts it from yaw).
fn turn_look(t: f32, dt: f32) -> f32 {
    if t % TURN_EVERY >= TURN_SECONDS {
        return 0.0;
    }
    let mark = (t / TURN_EVERY) as u32;
    turn_sign(mark) * FRAC_PI_2 / TURN_SECONDS * dt
}

/// The `p`th percentile (0 to 100) of frame times in ms, by nearest rank; 0 when there are none.
fn percentile(ms: &[f32], p: f32) -> f32 {
    if ms.is_empty() {
        return 0.0;
    }
    let mut sorted = ms.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (p * sorted.len() as f32 / 100.0).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The 1% low in fps, as the HUD computes it: the mean of the slowest 1% of frames (at least one),
/// as an fps.
fn one_percent_low_fps(frame_ms: &[f32]) -> f32 {
    if frame_ms.is_empty() {
        return 0.0;
    }
    let mut sorted = frame_ms.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let k = (sorted.len() / 100).max(1);
    let low_ms = sorted[..k].iter().sum::<f32>() / k as f32;
    1000.0 / low_ms.max(1e-4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_and_one_percent_low_from_frame_times() {
        let ms: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        assert_eq!(percentile(&ms, 99.0), 99.0);
        assert_eq!(percentile(&ms, 100.0), 100.0);
        let none: Vec<f32> = Vec::new();
        assert_eq!(percentile(&none, 99.0), 0.0);
        // 200 frames at 10 ms with two 50 ms hitches: the slowest 1% (two frames) averages 50 ms
        let mut frames = vec![10.0f32; 200];
        frames[3] = 50.0;
        frames[90] = 50.0;
        assert!((one_percent_low_fps(&frames) - 20.0).abs() < 1e-3);
        assert!((one_percent_low_fps(&[10.0f32; 50]) - 100.0).abs() < 1e-3);
    }
}
