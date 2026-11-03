//! KYTTU: Night Watch, a separate executable built from this package (`cargo run --bin night_watch`).
//! Placeholder: the mode is not ported yet. The Godot original lives in `game/night_watch/` of the
//! read-only reference project; this window only reserves the executable and its title.

use macroquad::prelude::*;

fn window_conf() -> Conf {
    Conf {
        window_title: "KYTTU: Night Watch".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    loop {
        if is_key_pressed(KeyCode::Escape) {
            break;
        }
        clear_background(BLACK);
        draw_text("KYTTU: NIGHT WATCH", 48.0, 96.0, 56.0, WHITE);
        draw_text("Not ported to Rust yet.", 48.0, 150.0, 28.0, GRAY);
        draw_text("Press Esc to close.", 48.0, 190.0, 24.0, DARKGRAY);
        next_frame().await;
    }
}
