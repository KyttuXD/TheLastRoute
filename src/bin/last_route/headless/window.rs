//! `--agent-window [--port N] [--realtime]`: the agent protocol with the game window drawing
//! the run, so `screenshot` works. Commands are read on other threads and run between
//! frames; the run only advances on `step` / `bot` / `goto` (deterministic, as headless)
//! unless `--realtime` lets the held inputs play at the window's frame rate.

use super::protocol::{err, Session};
use super::{respond, spawn_inputs, Args, Reply};
use crate::app::{load_world, render_play_with, save_screen, targets, World};
use crate::ui::{hud, style};
use macroquad::prelude::*;

pub async fn run() {
    let a = Args::get();
    let (rx, tcp) = match spawn_inputs(&a) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let World {
        content,
        catalog,
        assets,
        sounds,
        mat,
        ..
    } = load_world().await;
    let mut s = Session::new(super::sim::Sim::with(content, catalog, sounds));
    s.windowed = true;
    s.realtime = a.flag("realtime");
    let mut tg = targets(None, "AUTO");
    let mut waiting: Vec<(String, Reply, Option<serde_json::Value>)> = vec![];
    let mut stdin_open = true;
    eprintln!(
        "LAST ROUTE agent window ready (protocol {})",
        super::protocol::PROTOCOL
    );
    loop {
        crate::app::gfx::boot_guard_tick();
        // commands queued since the last frame
        while let Ok(msg) = rx.try_recv() {
            match msg {
                None => stdin_open = false,
                Some((line, to)) => {
                    let id = serde_json::from_str::<serde_json::Value>(&line)
                        .ok()
                        .and_then(|v| v.get("id").cloned());
                    match s.handle_line(&line) {
                        Some(r) => respond(to, &r),
                        None => {
                            let path = s.pending_shot.take().unwrap_or_default();
                            waiting.push((path, to, id));
                        }
                    }
                }
            }
            if !waiting.is_empty() {
                // draw this frame before running anything queued after the screenshot
                break;
            }
            if s.quit.is_some() {
                break;
            }
        }
        if let Some(code) = s.quit {
            std::process::exit(code);
        }
        if !stdin_open && !tcp {
            std::process::exit(0);
        }
        if s.realtime {
            if s.sim.game.is_some() {
                let dt = get_frame_time().min(0.05) * s.time_scale;
                let keep = s.sim.dt;
                s.sim.dt = dt;
                s.sim.step_with(None);
                s.sim.dt = keep;
            }
        }
        tg = targets(Some(tg), "AUTO");
        clear_background(BLACK);
        if let Some(g) = s.sim.game.as_mut() {
            g.sync_world(&assets, &s.sim.content);
        }
        if let Some(g) = s.sim.game.as_ref() {
            render_play_with(g, &assets, &mat, &tg, &s.remotes);
            hud::draw_hud(g, &assets, 60, style::ui_scale());
        } else {
            set_default_camera();
            draw_text(
                "LAST ROUTE agent window: send {\"cmd\":\"start_run\"}",
                24.0,
                40.0,
                24.0,
                WHITE,
            );
        }
        for (path, to, id) in waiting.drain(..) {
            let mut r = if path.is_empty() {
                err("could not save ")
            } else {
                save_screen(&path);
                if std::path::Path::new(&path).exists() {
                    serde_json::json!({ "ok": true, "path": path, "size": [screen_width() as i64, screen_height() as i64] })
                } else {
                    err(format!("could not save {path}"))
                }
            };
            if let (Some(id), Some(o)) = (id, r.as_object_mut()) {
                o.insert("id".into(), id);
            }
            respond(to, &r);
        }
        next_frame().await;
    }
}
