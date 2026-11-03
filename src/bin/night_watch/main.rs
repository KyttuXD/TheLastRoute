//! KYTTU: Night Watch, a separate executable built from this package (`cargo run --bin night_watch`).
//! The window and the hub menu; each night type's gameplay lives in this folder.


mod data;
mod framework;
mod mask;
mod modes;
mod save;
mod shell;

use data::CallsData;
use macroquad::prelude::*;
use modes::Session;
use shell::{Shell, ShellAction, ShellInput, HUB_ENTRIES};

fn window_conf() -> Conf {
    Conf {
        window_title: "KYTTU: Night Watch".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

fn new_seed() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(1)
}

fn hub_label(mode_id: &str) -> &'static str {
    HUB_ENTRIES.iter().find(|entry| entry.id == mode_id).map_or("", |entry| entry.label)
}

#[macroquad::main(window_conf)]
async fn main() {
    let calls = CallsData::load();
    let mut hub = Shell::new();
    let mut session: Option<Session> = None;
    let mut ended_announced = false;
    loop {
        let dt = get_frame_time();
        hub.tick(dt);
        if let Some(night) = session.as_mut() {
            night.tick(f64::from(dt) * 1000.0);
            if night.finished() && !ended_announced {
                ended_announced = true;
                hub.set_toast("The night is over. ESC returns to the hub.");
            }
        }
        if let Some(input) = shell::poll_input(&hub) {
            if session.is_some() {
                if input == ShellInput::Back {
                    session = None;
                    ended_announced = false;
                    hub.set_toast("Back at the hub.");
                }
            } else {
                match hub.handle(input) {
                    ShellAction::Quit => break,
                    ShellAction::Start(mode_id) => match modes::start(mode_id, new_seed(), &calls) {
                        Some(night) => {
                            hub.set_toast(format!("{} started. ESC returns to the hub.", hub_label(mode_id)));
                            session = Some(night);
                        }
                        None => hub.set_toast("This night type is not ported yet."),
                    },
                    ShellAction::None => {}
                }
            }
        }
        shell::draw(&hub);
        next_frame().await;
    }
}
