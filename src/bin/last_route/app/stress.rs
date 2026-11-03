//! Stress mode: `last_route --stress SECONDS --stress-log PATH` (main.rs reads the flags; SECONDS
//! defaults to 60, PATH to `stress.csv`).
//!
//! A solo run on the fixed seed `SEED` = 20261101 (room text `route::room_text(SEED)`, the same seed
//! the screenshot runs use), the Infinite route from the top level (the start CREATE ROOM gives). It
//! skips the title and the boot card, forces vsync off (swap interval 0) and does not apply the FPS
//! LIMIT, so the frame times are the game's own. The saved settings file is never written. The
//! menus, the pause menu and the admin tools take no input in this mode, and a run that is caught
//! ends there, with the summary covering the run up to the catch.
//!
//! The run flies, as the Godot flythrough does: admin noclip is on and the run speed is `FLY_SPEED`
//! (`fly`), so walls do not stop the runner and it covers ground fast. The synthetic input is a
//! `Controls` fed to `Game::update` the way `read_controls` feeds the keyboard and mouse: forward all
//! the time, no sprint, and a turn of `TURN_DEGREES` at each `LEG_SECONDS` boundary. The route is a
//! pure function of the elapsed time, and each turn's direction comes from the fixed seed, so the
//! run is repeatable and keeps reaching new ground.
//!
//! The log is CSV: a header, one row per whole second (`second` counts from 0), then a
//! `# summary` line once SECONDS have passed. Each row is flushed as it is written, so a crash
//! leaves the rows so far. The columns are `second, fps (frames that second), ms_avg, ms_p99`
//! (the 99th percentile frame time), then `update_ms, render_ms, hud_ms`, `hunters` (active),
//! `depth` and `travelled_m` (metres the runner has flown in the run so far, 1 cell = 1 m, counted
//! from its absolute cell so it keeps counting when the window moves). The three section times
//! are the perf sections' smoothed values, sampled once a frame and averaged over the second.

use super::{perf, World};
use crate::config::Difficulty;
use crate::game::{Controls, Game};
use crate::world::route;
use macroquad::prelude::Vec2;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

/// The run's seed. The screenshot runs start on the same seed.
const SEED: u32 = 20261101;
/// The run speed (the admin RUN SPEED multiplier) while the runner flies.
const FLY_SPEED: f32 = 4.0;
/// Seconds of straight flight between turns: the route turns at each multiple of this after the start.
const LEG_SECONDS: f32 = 20.0;
/// The size of each turn, to the left or the right by the seed's sign (`turn_sign`).
const TURN_DEGREES: f32 = 110.0;
/// Seconds over which a turn swings round, from its leg boundary.
const TURN_SECONDS: f32 = 0.25;
const HEADER: &str =
    "second,fps,ms_avg,ms_p99,update_ms,render_ms,hud_ms,hunters,depth,travelled_m";

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
    travel: Travel,
}

/// The frames of one second: their times and the sum of each section's samples.
#[derive(Default)]
struct Row {
    frame_ms: Vec<f32>,
    update_ms: f32,
    render_ms: f32,
    hud_ms: f32,
}

/// The metres the runner has flown, summed from its steps between frames in absolute cells.
#[derive(Default)]
struct Travel {
    last: Option<(f32, f32)>,
    metres: f32,
}

impl Travel {
    /// Adds the step from the last frame's cell to `cell`. A frame with no game clears the last cell,
    /// so the first cell of the next game starts a new path and adds nothing.
    fn add(&mut self, cell: Option<(f32, f32)>) {
        if let (Some(last), Some(cell)) = (self.last, cell) {
            self.metres += (cell.0 - last.0).hypot(cell.1 - last.1);
        }
        self.last = cell;
    }
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
            travel: Travel::default(),
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
        let cell = g.map(|g| {
            let (ox, oy) = g.window_origin();
            (g.p.pos.0 + ox as f32, g.p.pos.1 + oy as f32)
        });
        self.travel.add(cell);
        t >= self.seconds
    }

    /// The synthetic input for this frame; `dt` is the frame's game time.
    pub fn controls(&self, dt: f32) -> Controls {
        let t = self.start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        Controls {
            fwd: 1.0,
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
            "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{:.2}",
            self.second,
            frames,
            ms_avg,
            percentile(&row.frame_ms, 99.0),
            mean(row.update_ms),
            mean(row.render_ms),
            mean(row.hud_ms),
            self.hunters,
            self.depth,
            self.travel.metres,
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

/// Turns the flight on for `g`: admin noclip and the `FLY_SPEED` run speed, set on the game's own
/// admin switches (the playtest bridge's `noclip` and `speed` helpers), which `admin_flags` merges
/// with the admin menu's. The saved admin preferences are never written. Called each frame before the
/// update, so a game that starts after a level change keeps the flight.
pub fn fly(g: &mut Game) {
    g.admin.noclip = true;
    g.admin.run_speed = FLY_SPEED;
}

/// The sign of the turn at leg boundary `k` (the first boundary is 1): a fixed multiplicative hash, so
/// the direction sequence is the same on every run.
fn turn_sign(k: u32) -> f32 {
    let h = k.wrapping_mul(2_654_435_761).wrapping_add(SEED);
    if h >> 31 == 0 {
        1.0
    } else {
        -1.0
    }
}

/// The route's heading in radians at `t` seconds from the start. There is no turn before the first
/// leg boundary; each boundary after it turns the heading by `TURN_DEGREES` in its `turn_sign`
/// direction, swung over `TURN_SECONDS`. Positive turns right (`Player::look` subtracts it from yaw).
fn heading(t: f32) -> f32 {
    let t = t.max(0.0);
    let leg = (t / LEG_SECONDS) as u32;
    if leg == 0 {
        return 0.0;
    }
    let done: f32 = (1..leg).map(turn_sign).sum();
    let swing = ((t - leg as f32 * LEG_SECONDS) / TURN_SECONDS).clamp(0.0, 1.0);
    (done + turn_sign(leg) * swing) * TURN_DEGREES.to_radians()
}

/// The look for a frame at `t` seconds from the start: the heading turned over the frame's `dt`.
fn turn_look(t: f32, dt: f32) -> f32 {
    heading(t) - heading(t - dt)
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

    #[test]
    fn route_turns_at_each_leg_boundary_and_holds_between() {
        let turn = TURN_DEGREES.to_radians();
        assert_eq!(heading(0.0), 0.0);
        assert_eq!(heading(LEG_SECONDS - 0.5), 0.0);
        for k in 1..=12u32 {
            let boundary = k as f32 * LEG_SECONDS;
            let before = heading(boundary - 0.001);
            let after = heading(boundary + TURN_SECONDS + 0.001);
            assert!((after - before - turn_sign(k) * turn).abs() < 1e-3, "boundary {k}");
            let next_boundary = heading(boundary + LEG_SECONDS - 0.001);
            assert!((next_boundary - after).abs() < 1e-3, "leg after boundary {k}");
        }
    }

    #[test]
    fn heading_is_a_pure_function_of_time_with_both_turn_directions() {
        let t = 137.3;
        assert_eq!(heading(t), heading(t));
        assert_eq!(turn_look(t, 1.0 / 60.0), turn_look(t, 1.0 / 60.0));
        let signs: Vec<f32> = (1..=16u32).map(turn_sign).collect();
        assert!(signs.contains(&1.0) && signs.contains(&-1.0));
    }

    #[test]
    fn travelled_metres_carry_across_a_window_move() {
        // 3 m east and 4 m north in the first window (5 m), then the window moves 10 cells east and
        // the runner goes 4 m north: its window x goes from 3 to -7 and its absolute x stays 3
        let mut travel = Travel::default();
        let abs = |origin: (f32, f32), local: (f32, f32)| (origin.0 + local.0, origin.1 + local.1);
        travel.add(Some(abs((0.0, 0.0), (0.0, 0.0))));
        travel.add(Some(abs((0.0, 0.0), (3.0, 4.0))));
        travel.add(Some(abs((10.0, 0.0), (-7.0, 8.0))));
        assert!((travel.metres - 9.0).abs() < 1e-4);
        // a frame with no game, then a new game far away: nothing is added
        travel.add(None);
        travel.add(Some((50.0, 50.0)));
        assert!((travel.metres - 9.0).abs() < 1e-4);
    }
}
