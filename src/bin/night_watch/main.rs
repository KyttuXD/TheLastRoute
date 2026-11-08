//! Night Watch 3D: the hub. Parses arguments, runs `--check` without a window, otherwise opens the macroquad window and
//! runs the shell menu -> shift loop. Hub file (integrator).
//!
//! Frame order (contract 3.11): read input; shell or shift; for a shift `handle_input` -> `advance_ms` (chunks of at most
//! 100 ms) -> `animate`; 3D (`draw_feed` if the monitor is up, then `draw_world`); screen space (`Hud::draw_base`, the
//! mode's `draw_overlay`, the dev overlay, `Hud::draw_cards`).
//!
//! Note: this crate has a module named `core`. Inside the crate always write `crate::core::...`; the std crate is
//! `::core` at the root.
#![allow(dead_code)]

mod assets;
mod audio;
mod cli;
mod core;
mod gfx;
mod hud;
mod hunters;
mod modes;
mod net;
mod player;
mod registry;
mod shell;
mod shots;
mod ui;

use crate::audio::{cue_names, Sounds};
use crate::core::{
    camera_action, Bindings, EndCard, HudState, InputFrame, Mode, ModesData, Outcome, Save, Session, ShiftCtx, MONITOR, NIGHTS, PAUSE,
};
use crate::gfx::office::SPAWN;
use crate::gfx::Gfx;
use crate::hud::{BaseClick, Hud, HudAction};
use crate::hunters::HunterLib;
use crate::player::Player;
use crate::shell::{Backdrop, Screen, Shell, ShellAction};
use macroquad::prelude::*;
use std::collections::BTreeMap;

/// The seed `--shot` uses when no `--seed` is given, so a screenshot is repeatable.
/// The game's display title (window, title screen, credits). Ids, the bin name and save keys stay `night_watch`.
pub const GAME_TITLE: &str = "KYTTU - Night Watch";
const SHOT_SEED: u32 = 12345;

fn window_conf() -> macroquad::conf::Conf {
    macroquad::conf::Conf {
        miniquad_conf: macroquad::miniquad::conf::Conf {
            window_title: GAME_TITLE.to_owned(),
            window_width: 1280,
            window_height: 720,
            sample_count: 4,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn main() {
    let list: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::Args::parse(&list) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("night_watch: {e}\nusage: {}", cli::USAGE);
            std::process::exit(2);
        }
    };
    if args.help {
        println!("usage: {}", cli::USAGE);
        return;
    }
    if args.check {
        std::process::exit(cli::run_check());
    }
    macroquad::Window::from_config(window_conf(), amain(args));
}

/// Everything shared between the menu and a shift.
struct Env {
    args: cli::Args,
    modes: ModesData,
    save: Save,
    bindings: Bindings,
    session: Session,
    gfx: Gfx,
    hunters: HunterLib,
    snd: Sounds,
    hud: Hud,
    backdrop: Backdrop,
    /// --shot: fixed 1/30 s frames, animations snap.
    manual: bool,
    notice: Option<(String, f32)>,
    last_volume: f32,
    cursor_grabbed: bool,
    /// The admin panels of every mode (the overlay opens only for an admin account, see `hud::AdminGate`).
    admin_reg: crate::hud::AdminRegistry,
}

/// One running shift.
struct ShiftRun {
    mode: Box<dyn Mode>,
    hud_state: HudState,
    player: Player,
    paused: bool,
    /// The outcome was seen (win saved or death moment started).
    resolved: bool,
    card: Option<EndCard>,
}

enum Scene {
    Menu(Shell),
    Shift(ShiftRun),
}

enum Flow {
    Stay,
    Board,
    Title,
    Retry,
    Next,
}

macro_rules! ctx {
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
            manual_clock: $env.manual,
            paused: $paused,
        }
    };
}

/// Build the mode of `env.session` and start it. None when the mode is not built.
fn begin_shift(env: &mut Env) -> Option<ShiftRun> {
    if let Some(e) = crate::registry::load_error(&env.session.mode_id) {
        env.notice = Some((format!("CANNOT START: {e}"), 8.0));
        return None;
    }
    let mode = crate::registry::make_mode(&env.session)?;
    let mut run = ShiftRun { mode, hud_state: HudState::default(), player: Player::new(SPAWN), paused: false, resolved: false, card: None };
    let input = InputFrame::idle(0.0);
    {
        let mut c = ctx!(env, run, &input, false);
        run.mode.start(&mut c);
    }
    if let Some(room) = env.args.monitor.clone() {
        run.hud_state.monitor_up = true;
        run.hud_state.monitor_room = room;
    }
    env.hud = Hud::new();
    Some(run)
}

fn menu_scene(env: &Env, open_on: Screen) -> Scene {
    Scene::Menu(Shell::new(open_on, &env.session, &env.modes))
}

fn set_cursor(env: &mut Env, grab: bool) {
    if env.cursor_grabbed != grab {
        env.cursor_grabbed = grab;
        set_cursor_grab(grab);
        show_mouse(!grab);
    }
}

fn capitalize(id: &str) -> String {
    let mut c = id.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// One frame of a shift. Returns what the player chose on a card, if anything.
fn shift_frame(env: &mut Env, run: &mut ShiftRun, raw: &InputFrame) -> Flow {
    let mut input = raw.clone();
    if env.manual {
        input.dt = 1.0 / 30.0;
    }
    let dt = input.dt;

    // the dev overlay (key 0) steals the digit and the mode's input while it is open
    let admin_on = crate::hud::admin::enabled();
    if !admin_on {
        env.hud.admin.open = false;
    }
    if admin_on && input.digit == Some(0) {
        env.hud.admin.toggle();
        input.digit = None;
    }
    let admin_open = admin_on && env.hud.admin.open;
    let death_active = env.hud.death_moment_active();
    let running = run.mode.outcome() == Outcome::Running;

    // clicks on the always-on buttons become the key presses the modes already understand
    if !run.paused && running && !admin_open && !death_active {
        match env.hud.base_click(&run.hud_state, &env.bindings, &input) {
            Some(BaseClick::Pause) => {
                if !env.manual {
                    run.paused = true;
                }
            }
            Some(BaseClick::Cameras) | Some(BaseClick::CloseCameras) => {
                input.pressed.insert(MONITOR);
                input.down.insert(MONITOR);
            }
            Some(BaseClick::Camera(room)) => {
                let n = run.hud_state.cam_map.as_ref().map(|l| crate::hud::cam_map::number_for(l, &room)).unwrap_or(0);
                if n >= 1 {
                    let a = camera_action(n as i32);
                    input.pressed.insert(a);
                    input.down.insert(a);
                }
            }
            Some(BaseClick::Mute) => {
                if run.hud_state.call.mute() {
                    run.hud_state.zellia_muted = true;
                }
            }
            None => {}
        }
        if !run.paused && !env.manual && (is_key_pressed(KeyCode::Escape) || input.just(PAUSE)) {
            run.paused = true;
        }
    }

    // the mode sees no keys while the overlay is open
    let mut mode_input = input.clone();
    if admin_open {
        mode_input.down.clear();
        mode_input.pressed.clear();
        mode_input.released.clear();
        mode_input.digit = None;
        mode_input.click = false;
        mode_input.click_down = false;
        mode_input.click_right = false;
    }

    let playing = !run.paused && running && !admin_open;
    if playing {
        {
            let mut c = ctx!(env, run, &mode_input, false);
            run.mode.handle_input(&mut c);
        }
        let mut left = dt as f64 * 1000.0;
        while left > 0.0 {
            let step = left.min(100.0);
            left -= step;
            {
                let mut c = ctx!(env, run, &mode_input, false);
                run.mode.advance_ms(&mut c, step);
            }
            if run.mode.outcome() != Outcome::Running {
                break;
            }
        }
        run.hud_state.tick_note(dt);
    }
    {
        let mut c = ctx!(env, run, &mode_input, run.paused);
        run.mode.animate(&mut c, dt);
    }

    // a decided shift: save a win, start the death moment of a loss
    let outcome = run.mode.outcome();
    if outcome != Outcome::Running && !run.resolved {
        run.resolved = true;
        match outcome {
            Outcome::Won => {
                // a Custom Night of Open Air is recorded on its last watch only
                if !crate::modes::openair::defers_record(&env.session) {
                    env.save.record_win(&env.session, &env.modes);
                    env.save.store();
                }
            }
            Outcome::Lost => {
                if let Some((hid, cause)) = run.mode.killer() {
                    // Scare Off shows only the end card
                    if !env.save.scare_off() {
                        let short = env.save.short_death_scenes();
                        env.hud.start_death_moment(&capitalize(&hid), &hid, &cause, env.session.seed_value, env.session.night, short);
                    }
                }
            }
            Outcome::Running => {}
        }
    }
    if env.hud.death_moment_active() {
        env.hud.update_death_moment(dt, &input);
        for _ in 0..env.hud.take_death_blips() {
            env.snd.cues(&cue_names::menu("move"));
        }
    }
    if run.resolved && run.card.is_none() && !env.hud.death_moment_active() {
        let c = ctx!(env, run, &mode_input, run.paused);
        run.card = run.mode.end_card(&c);
    }

    // cursor: grabbed only while playing with the mouse not needed
    let want_free = run.mode.wants_mouse_free() || run.paused || !running || admin_open || env.hud.death_moment_active();
    set_cursor(env, !want_free);

    // 3D
    clear_background(BLACK);
    if run.hud_state.monitor_up {
        let room = run.hud_state.monitor_room.clone();
        let mut c = ctx!(env, run, &mode_input, run.paused);
        run.mode.draw_feed(&mut c, &room);
    }
    {
        let mut c = ctx!(env, run, &mode_input, run.paused);
        run.mode.draw_world(&mut c);
    }

    // screen space
    set_default_camera();
    env.hud.draw_base(&run.hud_state, &env.bindings, &env.save, &env.gfx);
    {
        let mut c = ctx!(env, run, &mode_input, run.paused);
        run.mode.draw_overlay(&mut c);
    }
    if admin_on {
        let entries = if env.hud.admin.open { env.admin_reg.entries(run.mode.as_mut()) } else { Vec::new() };
        let status = if env.save.debug_run { "DEBUG RUN: NOTHING IS SAVED" } else { "" };
        if let Some(id) = env.hud.admin.update_and_draw(&input, &entries, status) {
            env.admin_reg.run(&id, run.mode.as_mut());
            env.save.debug_run = true;
        }
    }
    let action = env.hud.draw_cards(run.paused, run.card.as_ref(), &input);
    match action {
        HudAction::None | HudAction::ToggleMute => Flow::Stay,
        HudAction::Resume => {
            run.paused = false;
            Flow::Stay
        }
        HudAction::QuitToBoard | HudAction::Menu => Flow::Board,
        HudAction::Retry => Flow::Retry,
        HudAction::Next => Flow::Next,
    }
}

/// One admin panel per mode.
fn build_admin_registry() -> crate::hud::AdminRegistry {
    let mut reg = crate::hud::AdminRegistry::new();
    reg.register(Box::new(crate::hud::ClassicAdmin));
    reg.register(Box::new(crate::modes::mask::MaskAdmin));
    reg.register(Box::new(crate::modes::lure::LureAdmin));
    reg.register(Box::new(crate::modes::bedroom::BedroomAdmin));
    reg.register(Box::new(crate::modes::story::StoryAdmin));
    reg.register(Box::new(crate::modes::descent::DescentAdmin));
    reg.register(Box::new(crate::modes::dayshift::DayshiftAdmin));
    reg.register(Box::new(crate::modes::openair::OpenAirAdmin));
    reg.register(Box::new(crate::modes::mixed::MixedAdmin));
    reg
}

async fn amain(args: cli::Args) {
    let modes = ModesData::load();
    let save = Save::load();
    let bindings = Bindings::from_save(&save);
    let gfx = Gfx::new();
    let backdrop = Backdrop::new(&gfx.tex);
    let mut snd = Sounds::new();
    snd.set_master(save.master_volume);
    let last_volume = save.master_volume;
    let manual = args.shot.is_some();
    let mut env = Env {
        args: args.clone(),
        modes,
        save,
        bindings,
        session: Session::new(),
        gfx,
        hunters: HunterLib::new(),
        snd,
        hud: Hud::new(),
        backdrop,
        manual,
        notice: None,
        last_volume,
        cursor_grabbed: false,
        admin_reg: build_admin_registry(),
    };

    if let Some(dir) = args.shots.clone() {
        let frames = shots::frames_or_default(args.frames_given, args.frames);
        let code = shots::run(&mut env, &dir, frames).await;
        #[cfg(not(target_arch = "wasm32"))]
        std::process::exit(code);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = code;
            return;
        }
    }

    let mut scene = menu_scene(&env, Screen::Title);
    if let Some(mode) = args.mode.clone() {
        let seed = args.seed.or(if manual { Some(SHOT_SEED) } else { None });
        env.session.start_mode(&mode, args.level, BTreeMap::new(), seed);
        match begin_shift(&mut env) {
            Some(run) => scene = Scene::Shift(run),
            None => {
                if env.notice.is_none() {
                    env.notice = Some((format!("{} IS NOT BUILT YET", mode.to_uppercase()), 3.0));
                }
            }
        }
    }

    let mut frame: u32 = 0;
    loop {
        // the admin gate follows the signed-in account (sign-in, sign-out and the session restore all change it)
        crate::hud::AdminGate { enabled: crate::net::account::is_admin() }.publish();
        let input = InputFrame::from_macroquad(&env.bindings);

        // the shell edits the volume
        if (env.save.master_volume - env.last_volume).abs() > 0.001 {
            env.last_volume = env.save.master_volume;
            env.snd.set_master(env.last_volume);
        }

        let mut next: Option<Scene> = None;
        match &mut scene {
            Scene::Menu(shell) => {
                set_cursor(&mut env, false);
                let act = shell.update_and_draw(&input, &mut env.save, &mut env.bindings, &env.modes, &mut env.backdrop, &env.gfx, &mut env.hunters);
                match act {
                    ShellAction::None => {}
                    ShellAction::PlayCue(list) => {
                        for c in list {
                            env.snd.cue(&c);
                        }
                    }
                    ShellAction::Quit => {
                        quit_game();
                        return;
                    }
                    ShellAction::StartShift { mode, level, custom } => {
                        env.session.start_mode(&mode, level, custom, None);
                        match begin_shift(&mut env) {
                            Some(run) => next = Some(Scene::Shift(run)),
                            None => {
                                if env.notice.is_none() {
                                    env.notice = Some((format!("{} IS NOT BUILT YET", mode.to_uppercase()), 3.0));
                                }
                            }
                        }
                    }
                }
            }
            Scene::Shift(run) => match shift_frame(&mut env, run, &input) {
                Flow::Stay => {}
                Flow::Board => next = Some(menu_scene(&env, Screen::Board)),
                Flow::Title => next = Some(menu_scene(&env, Screen::Title)),
                Flow::Retry => {
                    env.session.retry();
                    match begin_shift(&mut env) {
                        Some(r) => next = Some(Scene::Shift(r)),
                        None => next = Some(menu_scene(&env, Screen::Board)),
                    }
                }
                Flow::Next => {
                    let moved = if env.session.is_custom && env.session.mode_id == "extra" {
                        crate::modes::openair::chain_advance(&env.session)
                    } else {
                        if env.session.mode_id == "dayshift" {
                            crate::modes::dayshift::next_pressed();
                        }
                        env.session.next_level(&env.modes, NIGHTS)
                    };
                    let run = if moved { begin_shift(&mut env) } else { None };
                    match run {
                        Some(r) => next = Some(Scene::Shift(r)),
                        None => next = Some(menu_scene(&env, Screen::Board)),
                    }
                }
            },
        }
        if let Some(s) = next {
            scene = s;
        }

        // a short-lived line over everything (mode not built)
        if let Some((msg, left)) = &mut env.notice {
            crate::ui::text::text_center(msg, 240.0, 228.0, 9.0, crate::ui::theme::AMBER);
            *left -= get_frame_time();
            if *left <= 0.0 {
                env.notice = None;
            }
        }

        frame += 1;
        if let Some(path) = &args.shot {
            if frame >= args.frames {
                save_shot(path);
                quit_game();
                return;
            }
        }
        next_frame().await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn save_shot(path: &str) {
    get_screen_data().export_png(path);
}

#[cfg(target_arch = "wasm32")]
fn save_shot(_path: &str) {}

/// Leave the process on native builds; on the web the loop just ends.
fn quit_game() {
    #[cfg(not(target_arch = "wasm32"))]
    std::process::exit(0);
}
