//! `night_watch --shots DIR [--frames N]`: every mode's office view and every camera feed, saved in ONE process and ONE window.
//!
//! Per mode (level 1, seeded): the shift runs in a PROTECTED state (`Mode::tour_protect`, applied before every sim step: god mode,
//! no jumpscare, systems never broken, power / battery / air topped up, the clock wound back before the night can end; the AI still
//! walks the hunters normally). The sim is fast-forwarded without drawing (coarse 100 ms steps) until the hunters are out in the
//! building and Zellia's call has finished, `DIR/<mode>.png` is saved of the office view, the monitor is raised, and for every
//! camera of the mode the sim is stepped (coarse, no presenting) until at least one hunter stands in that camera's room or
//! `SEEK_LIMIT_S` seconds of sim time pass; then `--frames` settle frames (default 6) run and `DIR/<mode>-cam-<room>.png` is saved.
//! One line per file is printed (with the sim time and the hunters found), then a summary. The shift loop of `main.rs` is not
//! used, so the pause menu can never open on focus loss.
//! Hub hooks live in `cli.rs` (`--shots`) and `main.rs` (call `shots::run`).
#![allow(dead_code)]

use crate::core::{InputFrame, Mode, Outcome, ShiftCtx};
use macroquad::prelude::*;
use std::collections::BTreeMap;

/// Default settle frames run per camera after a hunter was found, before its shot.
pub const DEFAULT_FRAMES: u32 = 6;
/// Longest sim time spent per camera waiting for a hunter to stand in its room.
pub const SEEK_LIMIT_S: f64 = 150.0;
/// Sim time per seek check (the feed report is read this often).
const SEEK_STEP_MS: f64 = 500.0;
/// Longest extra sim time spent letting Zellia's call finish after the warm-up.
const CALL_LIMIT_S: f64 = 60.0;
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

/// The seek line of one camera: when the sim time was and who stands in the room.
pub fn seek_line(mode_id: &str, room: &str, found: bool, waited_s: f64, clock_s: f64, inside: &[String]) -> String {
    if found {
        format!("{mode_id} cam {room}: sim time {clock_s:.0} s (waited {waited_s:.0} s), hunters found: [{}]", inside.join(", "))
    } else {
        format!("{mode_id} cam {room}: sim time {clock_s:.0} s, NO hunter reached this room in {waited_s:.0} s")
    }
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

/// Advance the sim by `ms` in chunks of at most 100 ms, with no drawing, the shift protected before every chunk. Stops early
/// when the shift is decided. Returns the sim milliseconds really advanced.
fn advance(env: &mut crate::Env, run: &mut crate::ShiftRun, input: &InputFrame, ms: f64) -> f64 {
    let mut left = ms;
    let mut done = 0.0;
    while left > 0.0 {
        let step = left.min(WARMUP_STEP_MS);
        left -= step;
        run.mode.tour_protect();
        let mut c = sctx!(env, run, input, false);
        run.mode.advance_ms(&mut c, step);
        // a fault raised by this very step is undone at once, before any frame is drawn
        run.mode.tour_protect();
        done += step;
        if run.mode.outcome() != Outcome::Running {
            break;
        }
    }
    done
}

/// Let the warm-up call finish (it is typed by sim time), then clear any leftover line so no shot shows a half-typed call.
fn finish_call(env: &mut crate::Env, run: &mut crate::ShiftRun, clock_ms: &mut f64) {
    let idle = InputFrame::idle(FRAME_S);
    let mut waited = 0.0;
    while !run.hud_state.call.done && waited < CALL_LIMIT_S * 1000.0 && run.mode.outcome() == Outcome::Running {
        let d = advance(env, run, &idle, SEEK_STEP_MS);
        *clock_ms += d;
        waited += d;
        let mut c = sctx!(env, run, &idle, false);
        run.mode.animate(&mut c, FRAME_S);
    }
    if !run.hud_state.call.done {
        run.hud_state.call.mute();
    }
    run.hud_state.call_text.clear();
}

/// Step the sim (coarse, no presenting) until a hunter stands in the view of camera `room` or `SEEK_LIMIT_S` passes. Returns
/// (found, sim seconds waited, the hunters in view). A mode without a crowd (`feed_report` is None) is never waited for.
fn seek_hunter(env: &mut crate::Env, run: &mut crate::ShiftRun, room: &str, clock_ms: &mut f64) -> (bool, f64, Vec<String>) {
    let idle = InputFrame::idle(FRAME_S);
    let mut waited = 0.0;
    loop {
        match run.mode.feed_report(room) {
            None => return (false, waited / 1000.0, Vec::new()),
            Some((inside, _)) if !inside.is_empty() => return (true, waited / 1000.0, inside),
            Some(_) => {}
        }
        if waited >= SEEK_LIMIT_S * 1000.0 || run.mode.outcome() != Outcome::Running {
            return (false, waited / 1000.0, Vec::new());
        }
        let d = advance(env, run, &idle, SEEK_STEP_MS);
        *clock_ms += d;
        waited += d;
        // the actors follow the walkers in animate
        let mut c = sctx!(env, run, &idle, false);
        run.mode.animate(&mut c, FRAME_S);
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
        let _ = advance(env, run, &input, FRAME_S as f64 * 1000.0);
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
        let mut clock_ms = 0.0;
        run.mode.tour_protect();
        clock_ms += advance(env, &mut run, &idle, warmup_s(id) * 1000.0);
        {
            let mut c = sctx!(env, run, &idle, false);
            run.mode.animate(&mut c, FRAME_S);
        }
        finish_call(env, &mut run, &mut clock_ms);
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
            // one frame so the mode adopts the raised monitor and room (lure and others watch the room the player watches)
            sim_frame(env, &mut run);
            let (found, waited_s, seen) = seek_hunter(env, &mut run, &room, &mut clock_ms);
            println!("{}", seek_line(id, &room, found, waited_s, clock_ms / 1000.0, &seen));
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
    fn seek_line_names_the_time_and_the_hunters() {
        let l = seek_line("lure", "reception", true, 12.4, 140.2, &["xivvy".to_string()]);
        assert_eq!(l, "lure cam reception: sim time 140 s (waited 12 s), hunters found: [xivvy]");
        let n = seek_line("classic", "stage", false, 150.0, 400.0, &[]);
        assert!(n.contains("NO hunter reached") && n.contains("150 s"));
    }

    #[test]
    fn the_seek_is_bounded_and_the_settle_is_short() {
        assert!(SEEK_LIMIT_S >= 100.0 && SEEK_LIMIT_S <= 200.0);
        assert!(DEFAULT_FRAMES <= 30, "the settle must not let a found hunter walk away");
        assert!(SEEK_STEP_MS <= 1000.0 && SEEK_STEP_MS >= WARMUP_STEP_MS);
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
