//! Last Route, ported to Rust + macroquad from the Godot 4 project (KYTTU: The Last Route).
//!
//! Without arguments (or with `--screenshot` / `--self-test`) this opens the game window. The
//! agent and test modes in `headless` (`--headless`, `--test-suite`, `--bot`, `--agent-window`)
//! are picked before any window exists; see docs/AGENT_CLI.md.
//!
//! On Windows it is a window program (no empty terminal opens with the game); the command-line
//! modes attach to the terminal they were started from.

#![cfg_attr(windows, windows_subsystem = "windows")]

pub use last_route_core::packs;

mod audio;
mod config;
mod content;
mod game;
mod hunters;
mod render;
mod shared;
mod world;

mod app;
mod headless;
mod net;
mod ui;

use macroquad::prelude::*;

fn window_conf(stress_vsync: Option<bool>) -> macroquad::conf::Conf {
    // fitted to the screen; the screenshot / self-test captures stay 1280x720
    let fixed = std::env::args().any(|a| a == "--screenshot" || a == "--self-test");
    let (w, h) = app::platform::window_size(fixed);
    // Metal or OpenGL: miniquad reads it once, at window creation (app/gfx.rs has the rules)
    let gfx = app::gfx::chosen().gfx;
    app::gfx::boot_guard_start();
    macroquad::conf::Conf {
        miniquad_conf: Conf {
            window_title: "KYTTU: The Last Route".to_owned(),
            window_width: w,
            window_height: h,
            high_dpi: false,
            icon: app::assets::window_icon(),
            // miniquad defaults to swap interval 1 (vsync on) whatever the VSYNC row says; a stress run
            // takes --stress-vsync instead of the saved row
            platform: macroquad::miniquad::conf::Platform {
                swap_interval: Some(match stress_vsync {
                    Some(on) => i32::from(on),
                    None => app::settings::Settings::load().swap_interval(),
                }),
                apple_gfx_api: match gfx {
                    app::gfx::Gfx::Metal => macroquad::miniquad::conf::AppleGfxApi::Metal,
                    app::gfx::Gfx::Gl => macroquad::miniquad::conf::AppleGfxApi::OpenGl,
                },
                ..Default::default()
            },
            ..Default::default()
        },
        update_on: None,
        default_filter_mode: FilterMode::Linear,
        // the floor meshes are split under the 16-bit index limit; each must fit one draw call
        draw_call_vertex_capacity: render::draw_stats::DRAW_CALL_VERTEX_CAPACITY,
        draw_call_index_capacity: render::draw_stats::DRAW_CALL_INDEX_CAPACITY,
    }
}

fn arg(name: &str) -> Option<String> {
    let v: Vec<String> = std::env::args().collect();
    v.iter()
        .position(|a| a == name)
        .and_then(|i| v.get(i + 1).cloned())
}

/// `--stress [SECONDS] [--stress-log PATH] [--stress-scenario NAME] [--stress-vsync on|off] [--stress-limit FPS]`
/// (app/stress.rs): a performance run that exits when done. Exits the process on a bad value, before
/// any window exists.
fn stress_config() -> Option<app::stress::Config> {
    if !std::env::args().any(|a| a == "--stress") {
        return None;
    }
    let seconds = match arg("--stress") {
        Some(v) if !v.starts_with("--") => match v.parse::<f32>() {
            Ok(s) if s > 0.0 && s.is_finite() => s,
            _ => {
                eprintln!("--stress needs a positive number of seconds, not {v}");
                std::process::exit(2);
            }
        },
        _ => 60.0,
    };
    let log = arg("--stress-log").unwrap_or_else(|| "stress.csv".to_owned());
    let scenario = match arg("--stress-scenario") {
        None => app::stress::Scenario::Fly,
        Some(v) => app::stress::Scenario::parse(&v).unwrap_or_else(|| {
            eprintln!("--stress-scenario needs fly, npcs, hunters, interact or mixed, not {v}");
            std::process::exit(2);
        }),
    };
    let vsync = match arg("--stress-vsync").as_deref() {
        None | Some("off") => false,
        Some("on") => true,
        Some(v) => {
            eprintln!("--stress-vsync needs on or off, not {v}");
            std::process::exit(2);
        }
    };
    let fps_limit = match arg("--stress-limit").as_deref() {
        None | Some("0") => None,
        Some(v) => match v.parse::<u32>() {
            Ok(n) if n > 0 => Some(n),
            _ => {
                eprintln!("--stress-limit needs a whole number of frames per second (0 for none), not {v}");
                std::process::exit(2);
            }
        },
    };
    Some(app::stress::Config { seconds, log, scenario, vsync, fps_limit })
}

/// What `#[macroquad::main(window_conf)]` expanded to, with the windowless modes picked first
/// (they never create a window, a GPU context or an audio device).
fn main() {
    if std::env::args()
        .skip(1)
        .any(|a| a.starts_with("--") && a != "--screenshot" && a != "--self-test")
    {
        app::platform::attach_console();
    }
    app::log::init();
    if let Some(code) = headless::dispatch() {
        app::log::flush();
        std::process::exit(code);
    }
    let stress = stress_config();
    if headless::wants_agent_window() {
        macroquad::Window::from_config(window_conf(None), headless::window::run());
        return;
    }
    macroquad::Window::from_config(window_conf(stress.as_ref().map(|c| c.vsync)), amain(stress));
}

async fn amain(stress: Option<app::stress::Config>) {
    if let Some(path) = arg("--self-test") {
        app::selftest::run(&path).await;
        return;
    }
    // a stress run never looks for an update, so nothing downloads or prompts while it measures
    let mut update = None;
    if arg("--screenshot").is_none() && stress.is_none() {
        update = Some(app::updater::start_check());
    }
    app::run(arg("--screenshot"), stress, update).await;
}
