//! Last Route, ported to Rust + macroquad from the Godot 4 project (KYTTU: The Last Route).
//!
//! Without arguments (or with `--screenshot` / `--self-test`) this opens the game window. The
//! agent and test modes in `headless` (`--headless`, `--test-suite`, `--bot`, `--agent-window`)
//! are picked before any window exists; see docs/AGENT_CLI.md.
//!
//! On Windows it is a window program (no empty terminal opens with the game); the command-line
//! modes attach to the terminal they were started from.

#![cfg_attr(windows, windows_subsystem = "windows")]

pub use last_route_core::{audio, config, content, game, hunters, render, shared, world};

mod app;
mod headless;
mod net;
mod ui;

use macroquad::prelude::*;

fn window_conf() -> macroquad::conf::Conf {
    // fitted to the screen; the screenshot / self-test captures stay 1280x720
    let fixed = std::env::args().any(|a| a == "--screenshot" || a == "--self-test");
    let (w, h) = app::platform::window_size(fixed);
    macroquad::conf::Conf {
        miniquad_conf: Conf {
            window_title: "KYTTU: The Last Route".to_owned(),
            window_width: w,
            window_height: h,
            high_dpi: false,
            icon: app::assets::window_icon(),
            // miniquad defaults to swap interval 1 (vsync on) whatever the VSYNC row says
            platform: macroquad::miniquad::conf::Platform {
                swap_interval: Some(app::settings::Settings::load().swap_interval()),
                ..Default::default()
            },
            ..Default::default()
        },
        update_on: None,
        default_filter_mode: FilterMode::Linear,
        // the floor meshes are split under the 16-bit index limit; each must fit one draw call
        draw_call_vertex_capacity: 65536,
        draw_call_index_capacity: 65536 * 3 / 2,
    }
}

fn arg(name: &str) -> Option<String> {
    let v: Vec<String> = std::env::args().collect();
    v.iter()
        .position(|a| a == name)
        .and_then(|i| v.get(i + 1).cloned())
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
    if headless::wants_agent_window() {
        macroquad::Window::from_config(window_conf(), headless::window::run());
        return;
    }
    macroquad::Window::from_config(window_conf(), amain());
}

async fn amain() {
    if let Some(path) = arg("--self-test") {
        app::selftest::run(&path).await;
        return;
    }
    if arg("--screenshot").is_none() {
        app::updater::check_on_start();
    }
    app::run(arg("--screenshot")).await;
}
