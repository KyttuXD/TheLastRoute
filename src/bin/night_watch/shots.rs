//! `night_watch --shots DIR [--frames N]`: every mode's office view and every camera feed, saved in ONE process and ONE window.
//!
//! Per mode (level 1, seeded): the sim is fast-forwarded without drawing (coarse 100 ms steps) until the hunters are out in the
//! building, `DIR/<mode>.png` is saved of the office view, the monitor is raised, and for every camera of the mode the sim runs
//! `--frames` more frames (default 240, advanced without presenting) before `DIR/<mode>-cam-<room>.png` is saved. One line per
//! file is printed, then a summary. The shift loop of `main.rs` is not used, so the pause menu can never open on focus loss.
//! Hub hooks live in `cli.rs` (`--shots`) and `main.rs` (call `shots::run`).
#![allow(dead_code)]

use crate::core::{InputFrame, Mode, Outcome, ShiftCtx};
use macroquad::prelude::*;
use std::collections::BTreeMap;

/// Default frames advanced per camera before its shot.
pub const DEFAULT_FRAMES: u32 = 240;
/// Default sim time simulated before a mode's first shot (QUIET-type modes spawn their hunters after about 50 s).
pub const DEFAULT_WARMUP_S: f64 = 90.0;
/// Fixed frame length of the shot sim (the same 1/30 s `--shot` uses).
const FRAME_S: f32 = 1.0 / 30.0;
/// The warm-up step: main clamps `advance_ms` to 100 ms per call, so coarse steps are the biggest allowed.
const WARMUP_STEP_MS: f64 = 100.0;
/// Seed of every shot shift, so the pictures are repeatable.
const SEED: u32 = 12345;

/// Warm-up sim seconds of a mode: the mode's own first-move time from its data, plus a margin so the hunters are walking in
/// front of the cameras when the feeds are shot.
/// - mixed: the slowest pace (QUIET) starts its first threat at `start_ms` 50 000 (data/mixed/tuning.json), plus walking time.
/// - lure: Xivvy's wake step is 8 s (`wakeStep`) and the others leave their rooms after a few of them.
/// - dayshift: `firstSpawnMs` 2 000 with 50 % spread, then the hunters walk in from the airlock.
/// - mask: `stepMs` 2 300 over `hallSteps` 4; classic / story / open air: first behaviour delay of 2 to 6 s, then walking.
pub fn warmup_s(mode_id: &str) -> f64 {
    match mode_id {
        "mixed" => 50.0 + 60.0,
        "lure" => 8.0 * 6.0 + 60.0,
        "dayshift" => 2.0 * 1.5 + 57.0,
        "mask" => 2.3 * 4.0 * 4.0 + 50.0,
        "classic" | "story" | "openair" => 90.0,
        _ => DEFAULT_WARMUP_S,
    }
}

/// The per-shot line: `<mode> cam <room>: hunters in room: [ids], drawn: [ids]`.
pub fn report_line(mode_id: &str, room: &str, inside: &[String], drawn: &[String]) -> String {
    format!("{mode_id} cam {room}: hunters in room: [{}], drawn: [{}]", inside.join(", "), drawn.join(", "))
}

/// Frames per camera: the `--frames` value when it was given, else the default.
pub fn frames_or_default(frames_given: bool, frames: u32) -> u32 {
    if frames_given {
        frames.max(1)
    } else {
        DEFAULT_FRAMES
    }
}

/// A room id as a file name part: letters and digits stay, everything else becomes '-'.
pub fn sanitize(room: &str) -> String {
    let s: String = room.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    if s.is_empty() {
        "cam".to_string()
    } else {
        s
    }
}

/// `<mode>.png` for the office, `<mode>-cam-<room>.png` for a camera.
pub fn file_name(mode_id: &str, room: Option<&str>) -> String {
    match room {
        None => format!("{mode_id}.png"),
        Some(r) => format!("{mode_id}-cam-{}.png", sanitize(r)),
    }
}

macro_rules! sctx {
    ($env:expr, $run:expr, $input:expr, $paused:expr) => {
        ShiftCtx {
            session: &$env.session,
            modes: &$env.modes,
            input: $input,
            bindings: &$env.bindings,
            save: &mut $env.save,
            snd: &mut $env.snd,
            hud: &mut $run.hud_state,
            gfx: &mut $env.gfx,
            hunters: &mut $env.hunters,
            player: &mut $run.player,
            manual_clock: true,
            paused: $paused,
        }
    };
}

/// Advance the sim by `ms` in chunks of at most 100 ms, with no drawing. Stops early when the shift is decided.
fn advance(env: &mut crate::Env, run: &mut crate::ShiftRun, input: &InputFrame, ms: f64) {
    let mut left = ms;
    while left > 0.0 {
        let step = left.min(WARMUP_STEP_MS);
        left -= step;
        let mut c = sctx!(env, run, input, false);
        run.mode.advance_ms(&mut c, step);
        if run.mode.outcome() != Outcome::Running {
            break;
        }
    }
}

/// One sim frame without presenting: input, sim, animation.
fn sim_frame(env: &mut crate::Env, run: &mut crate::ShiftRun) {
    let input = InputFrame::idle(FRAME_S);
    {
        let mut c = sctx!(env, run, &input, false);
        run.mode.handle_input(&mut c);
    }
    if run.mode.outcome() == Outcome::Running {
        advance(env, run, &input, FRAME_S as f64 * 1000.0);
        run.hud_state.tick_note(FRAME_S);
    }
    let mut c = sctx!(env, run, &input, false);
    run.mode.animate(&mut c, FRAME_S);
}

/// Draw one presented frame the way `shift_frame` does (feed, world, base HUD, mode overlay), without the admin overlay or cards.
fn draw_frame(env: &mut crate::Env, run: &mut crate::ShiftRun) {
    let input = InputFrame::idle(FRAME_S);
    clear_background(BLACK);
    if run.hud_state.monitor_up {
        let room = run.hud_state.monitor_room.clone();
        let mut c = sctx!(env, run, &input, false);
        run.mode.draw_feed(&mut c, &room);
    }
    {
        let mut c = sctx!(env, run, &input, false);
        run.mode.draw_world(&mut c);
    }
    set_default_camera();
    env.hud.draw_base(&run.hud_state, &env.bindings, &env.save, &env.gfx);
    let mut c = sctx!(env, run, &input, false);
    run.mode.draw_overlay(&mut c);
}

/// Present a settle frame, then draw again and save the picture to `path`. True when the file exists afterwards.
#[cfg(not(target_arch = "wasm32"))]
async fn capture(env: &mut crate::Env, run: &mut crate::ShiftRun, path: &std::path::Path) -> bool {
    draw_frame(env, run);
    next_frame().await;
    sim_frame(env, run);
    draw_frame(env, run);
    get_screen_data().export_png(&path.to_string_lossy());
    let ok = std::fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false);
    next_frame().await;
    ok
}

/// Run the whole tour. Returns the process exit code: 0 when every shot was written, 1 otherwise.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn run(env: &mut crate::Env, dir: &str, frames: u32) -> i32 {
    let dir_path = std::path::PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir_path) {
        eprintln!("night_watch --shots: cannot create '{dir}': {e}");
        return 1;
    }
    env.manual = true;
    let (mut written, mut failed) = (0usize, 0usize);
    let (mut files_wanted, mut modes_done) = (0usize, 0usize);
    for id in crate::registry::MODE_IDS {
        if !crate::registry::playable(id) {
            println!("{id}: not built");
            continue;
        }
        env.session.start_mode(id, 1, BTreeMap::new(), Some(SEED));
        let Some(mut run) = crate::begin_shift(env) else {
            println!("{id}: FAILED to start");
            failed += 1;
            files_wanted += 1;
            continue;
        };
        modes_done += 1;
        run.hud_state.monitor_up = false;

        // fast-forward without drawing until the hunters are out
        let idle = InputFrame::idle(FRAME_S);
        {
            let mut c = sctx!(env, run, &idle, false);
            run.mode.handle_input(&mut c);
        }
        advance(env, &mut run, &idle, warmup_s(id) * 1000.0);
        {
            let mut c = sctx!(env, run, &idle, false);
            run.mode.animate(&mut c, FRAME_S);
        }
        if run.mode.outcome() != Outcome::Running {
            println!("{id}: note: the shift was decided during the warm-up ({:?})", run.mode.outcome());
        }

        // the office view
        files_wanted += 1;
        let path = dir_path.join(file_name(id, None));
        if capture(env, &mut run, &path).await {
            println!("{}", path.display());
            written += 1;
        } else {
            println!("{}: FAILED", path.display());
            failed += 1;
        }

        // the cameras: raise the monitor and visit every camera of the mode
        let cams: Vec<String> = run.hud_state.cam_map.as_ref().map(|l| l.cams.clone()).unwrap_or_default();
        if cams.is_empty() {
            println!("{id}: no cameras");
            continue;
        }
        run.hud_state.monitor_up = true;
        for room in cams {
            // raise the monitor the way the player's request reaches every mode: the HUD flag and room, which the mode adopts
            // in its next handle_input (lure keeps its own monitor state and now honours this too)
            run.hud_state.monitor_up = true;
            run.hud_state.monitor_room = room.clone();
            for _ in 0..frames {
                sim_frame(env, &mut run);
            }
            files_wanted += 1;
            // the sim's answer to "who is in this camera's view, and who does the feed draw"
            match run.mode.feed_report(&room) {
                Some((inside, drawn)) => println!("{}", report_line(id, &room, &inside, &drawn)),
                None => println!("{id} cam {room}: hunters in room: [?], drawn: [?] (this mode reports none)"),
            }
            let path = dir_path.join(file_name(id, Some(&room)));
            if capture(env, &mut run, &path).await {
                println!("{}", path.display());
                written += 1;
            } else {
                println!("{}: FAILED", path.display());
                failed += 1;
            }
        }
    }
    println!("night_watch --shots: {written} of {files_wanted} files written, {modes_done} modes, {failed} failed");
    if failed == 0 && written > 0 {
        0
    } else {
        1
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn run(_env: &mut crate::Env, _dir: &str, _frames: u32) -> i32 {
    eprintln!("night_watch --shots: not available on the web build");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(file_name("classic", None), "classic.png");
        assert_eq!(file_name("lure", Some("Dining Hall")), "lure-cam-dining-hall.png");
        assert_eq!(file_name("mask", Some("")), "mask-cam-cam.png");
    }

    #[test]
    fn sanitize_keeps_ids_readable() {
        assert_eq!(sanitize("west_hall"), "west-hall");
        assert_eq!(sanitize("Vent2"), "vent2");
    }

    #[test]
    fn warmup_covers_each_modes_first_move() {
        assert!(warmup_s("mixed") >= 50.0 + 30.0, "QUIET starts its first threat at 50 s");
        assert!(warmup_s("lure") >= 8.0 * 4.0);
    }

    #[test]
    fn report_line_format() {
        let l = report_line("classic", "hall", &["aki".to_string(), "dovelle".to_string()], &["aki".to_string()]);
        assert_eq!(l, "classic cam hall: hunters in room: [aki, dovelle], drawn: [aki]");
        assert_eq!(report_line("lure", "x", &[], &[]), "lure cam x: hunters in room: [], drawn: []");
    }

    #[test]
    fn frames_default_and_override() {
        assert_eq!(frames_or_default(false, 30), DEFAULT_FRAMES);
        assert_eq!(frames_or_default(true, 30), 30);
        assert_eq!(frames_or_default(true, 0), 1);
    }

    #[test]
    fn warmup_reaches_the_spawn_time_for_every_mode() {
        for id in crate::registry::MODE_IDS {
            assert!(warmup_s(id) >= 60.0, "{id} warm-up too short for QUIET-type spawns");
        }
    }
}
