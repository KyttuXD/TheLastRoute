//! KYTTU: Night Watch, a separate executable built from this package (`cargo run --bin night_watch`).
//! The window and the hub menu; each night type's gameplay lives in this folder. Classic is playable
//! (`classic_play.rs`) and Bedtime is playable (`modes_play/bedtime.rs`); the other seven are started and ticked
//! but have no controls or picture yet.
//!
//! `--screenshot PATH` renders one scripted Classic frame to a PNG and exits. Extra flags: `--night N`, `--frames N`
//! (fixed 60 fps steps first), `--cam N` (monitor up on camera N), `--door-l`, `--door-r`, `--light-l`, `--light-r`
//! and `--call-chars N` (keep stepping until the first call line has N letters showing). With `--mode bedtime` the
//! same flag draws a scripted Bedtime frame instead: `--hold`, `--light-hall`, `--light-bed`, `--plan`. With
//! `--mode mask` it draws a scripted Masked Hours frame: `--cam N`, `--mask`, `--flashlight`, `--lamp`, `--wind`,
//! `--vent-l`, `--vent-r`. With `--mode lure` it draws a scripted Dead Air frame: `--cam N`, `--seal-l`, `--seal-r`,
//! `--heater`, `--fan`, `--night N` and `--call-chars N`.

mod blip;
mod building_map;
mod calls_view;
mod classic_play;
mod data;
mod framework;
mod mask;
mod modes;
mod modes_play;
mod save;
mod select;
mod shell;
mod sprites;
mod nw_sound;
mod typewriter;

use classic_play::{ClassicRun, Flow};
use data::CallsData;
use macroquad::prelude::*;
use modes::Session;
use modes_play::bedtime as bedtime_play;
use modes_play::dead_air as dead_play;
use modes::open_air::Watch;
use modes_play::masked_hours as masked_play;
use modes_play::mixed_shift::{self as mixed_play, MixedRun};
use modes_play::facility as facility_play;
use modes_play::descent as descent_play;
use modes_play::open_air::{self as air, Flow as AirFlow, OpenAirRun};
use modes_play::salvage_run as salvage;
use save::NightWatchSave;
use select::{Select, SelectAction};
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

/// The seed every scripted screenshot starts on, so the same flags give the same picture.
const SHOT_SEED: i64 = 20_261_103;

/// The value after `flag` on the command line.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// The `--screenshot` run: its output path and the scripted night, or None for a normal start.
fn shot_request(args: &[String]) -> Option<(String, classic_play::ShotPlan)> {
    let path = arg_value(args, "--screenshot")?;
    let number = |flag: &str| arg_value(args, flag).and_then(|v| v.parse::<u32>().ok());
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let plan = classic_play::ShotPlan {
        night: number("--night").unwrap_or(1),
        frames: number("--frames").unwrap_or(600),
        cam: number("--cam"),
        door_l: has("--door-l"),
        door_r: has("--door-r"),
        light_l: has("--light-l"),
        light_r: has("--light-r"),
        call_chars: number("--call-chars").map(|n| n as usize),
    };
    Some((path, plan))
}

/// Draws the scripted night for a few frames (the first ones can land at a different size), saves the PNG and returns.
async fn take_screenshot(run: &ClassicRun, path: &str) {
    for frame in 0..4 {
        classic_play::draw(run);
        if frame == 3 {
            get_screen_data().export_png(path);
        }
        next_frame().await;
    }
}

/// The `--mode mask` screenshot plan: the night, the held keys and the monitor camera named on the command line.
fn mask_shot_plan(args: &[String]) -> masked_play::ShotPlan {
    let number = |flag: &str| arg_value(args, flag).and_then(|v| v.parse::<u32>().ok());
    let has = |flag: &str| args.iter().any(|a| a == flag);
    masked_play::ShotPlan {
        night: number("--night").unwrap_or(1),
        frames: number("--frames").unwrap_or(600),
        cam: number("--cam"),
        mask: has("--mask"),
        flashlight: has("--flashlight"),
        lamp: has("--lamp"),
        wind: has("--wind"),
        vent_l: has("--vent-l"),
        vent_r: has("--vent-r"),
        call_chars: number("--call-chars").map(|n| n as usize),
    }
}

async fn take_mask_screenshot(run: &masked_play::MaskRun, path: &str) {
    for frame in 0..4 {
        masked_play::draw(run);
        if frame == 3 {
            get_screen_data().export_png(path);
        }
        next_frame().await;
    }
}

/// The `--mode bedtime` screenshot plan: the night and the held controls named on the command line.
fn bedtime_shot_plan(args: &[String]) -> bedtime_play::ShotPlan {
    let number = |flag: &str| arg_value(args, flag).and_then(|v| v.parse::<u32>().ok());
    let has = |flag: &str| args.iter().any(|a| a == flag);
    bedtime_play::ShotPlan {
        night: number("--night").unwrap_or(1),
        frames: number("--frames").unwrap_or(600),
        hold: has("--hold"),
        light_hall: has("--light-hall"),
        light_bed: has("--light-bed"),
        plan: has("--plan"),
        call_chars: number("--call-chars").map(|n| n as usize),
    }
}

async fn take_bedtime_screenshot(run: &bedtime_play::BedtimeRun, path: &str) {
    for frame in 0..4 {
        bedtime_play::draw(run);
        if frame == 3 {
            get_screen_data().export_png(path);
        }
        next_frame().await;
    }
}

async fn take_dead_air_screenshot(run: &dead_play::DeadAirRun, art: &dead_play::Art, path: &str) {
    for frame in 0..4 {
        dead_play::draw(run, art);
        if frame == 3 {
            get_screen_data().export_png(path);
        }
        next_frame().await;
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let calls = CallsData::load();
    nw_sound::init();
    let args: Vec<String> = std::env::args().collect();
    if let Some(path) = arg_value(&args, "--select-shot") {
        // The Night Select screen for one mode, on the demo save; `--custom` opens its Custom Night.
        let mode = arg_value(&args, "--select-mode").unwrap_or_else(|| "classic".to_owned());
        let save = select::demo_save();
        if let Some(mut screen) = Select::open(&mode, &save) {
            if args.iter().any(|a| a == "--custom") {
                screen.handle(select::SelectInput::CustomKey, &save);
            }
            for frame in 0..4 {
                select::draw(&screen, &save);
                if frame == 3 {
                    get_screen_data().export_png(&path);
                }
                next_frame().await;
            }
        }
        return;
    }
    if arg_value(&args, "--mode").as_deref() == Some("facility") {
        if let Some(path) = arg_value(&args, "--screenshot") {
            let number = |flag: &str| arg_value(&args, flag).and_then(|v| v.parse::<u32>().ok());
            let run = facility_play::run_script(&calls, SHOT_SEED, number("--night").unwrap_or(1), number("--frames").unwrap_or(900));
            let art = facility_play::Art::load();
            for frame in 0..4 {
                facility_play::draw(&run, &art);
                if frame == 3 {
                    get_screen_data().export_png(&path);
                }
                next_frame().await;
            }
            return;
        }
    }
    if arg_value(&args, "--mode").as_deref() == Some("mask") {
        if let Some(path) = arg_value(&args, "--screenshot") {
            let run = masked_play::run_script(&calls, SHOT_SEED, &mask_shot_plan(&args));
            take_mask_screenshot(&run, &path).await;
            return;
        }
    }
    if arg_value(&args, "--mode").as_deref() == Some("bedtime") {
        if let Some(path) = arg_value(&args, "--screenshot") {
            let run = bedtime_play::run_script(&calls, SHOT_SEED, &bedtime_shot_plan(&args));
            take_bedtime_screenshot(&run, &path).await;
            return;
        }
    }
    if let Some((path, plan)) = salvage::shot_request(&args) {
        let art = salvage::Art::load();
        let run = salvage::run_script(&calls, SHOT_SEED, &plan);
        for frame in 0..4 {
            salvage::draw(&run, &art);
            if frame == 3 {
                get_screen_data().export_png(&path);
            }
            next_frame().await;
        }
        return;
    }
    if arg_value(&args, "--mode").as_deref() == Some("lure") {
        if let Some(path) = arg_value(&args, "--screenshot") {
            let run = dead_play::run_script(&calls, SHOT_SEED, &dead_play::ShotPlan::from_args(&args));
            let dead_art = dead_play::Art::load();
            take_dead_air_screenshot(&run, &dead_art, &path).await;
            return;
        }
    }
    if let Some(path) = arg_value(&args, "--mixed-screenshot") {
        let number = |flag: &str| arg_value(&args, flag).and_then(|v| v.parse::<u32>().ok());
        let plan = mixed_play::ShotPlan { shift: number("--shift").unwrap_or(0) as usize, frames: number("--frames").unwrap_or(600), cam: number("--cam") };
        let art = mixed_play::Art::load();
        let run = mixed_play::run_script(SHOT_SEED, &plan);
        for frame in 0..4 {
            mixed_play::draw(&run, &art);
            if frame == 3 {
                get_screen_data().export_png(&path);
            }
            next_frame().await;
        }
        return;
    }
    if let Some((path, plan)) = descent_play::shot_request(&args) {
        let run = descent_play::run_script(&calls, SHOT_SEED, &plan);
        for frame in 0..4 {
            descent_play::draw(&run);
            if frame == 3 {
                get_screen_data().export_png(&path);
            }
            next_frame().await;
        }
        return;
    }
    if let Some((path, plan)) = shot_request(&args) {
        let run = classic_play::run_script(&calls, SHOT_SEED, &plan);
        take_screenshot(&run, &path).await;
        return;
    }

    if let Some(path) = arg_value(&args, "--open-air-screenshot") {
        let watch = arg_value(&args, "--watch").and_then(|v| v.parse::<i32>().ok()).map(Watch::from_level);
        let custom = args.iter().any(|a| a == "--custom");
        let frames = arg_value(&args, "--frames").and_then(|v| v.parse::<u32>().ok()).unwrap_or(600);
        let run = air::run_script(&calls, SHOT_SEED, watch, custom, frames);
        air::take_screenshot(&run, &path).await;
        return;
    }

    let mut hub = Shell::new();
    let mut session: Option<Session> = None;
    let mut classic: Option<ClassicRun> = None;
    let mut open_air: Option<OpenAirRun> = None;
    let mut bedtime: Option<bedtime_play::BedtimeRun> = None;
    let mut salvage_run: Option<salvage::SalvageRunRun> = None;
    let art = salvage::Art::load();
    let mut masked: Option<masked_play::MaskRun> = None;
    let mut dead: Option<dead_play::DeadAirRun> = None;
    let mut dead_art: Option<dead_play::Art> = None;
    let mut mixed: Option<MixedRun> = None;
    let mixed_art = mixed_play::Art::load();
    let mut facility: Option<facility_play::FacilityRun> = None;
    let mut descent: Option<descent_play::DescentRun> = None;
    let facility_art = facility_play::Art::load();
    let mut ended_announced = false;
    let mut select: Option<(Select, NightWatchSave)> = None;
    loop {
        let dt = get_frame_time();
        hub.tick(dt);
        let mut chosen = SelectAction::None;
        if let Some((screen, save)) = select.as_mut() {
            if let Some(input) = select::poll_input(screen) {
                chosen = screen.handle(input, save);
            }
        }
        match chosen {
            SelectAction::None => {}
            SelectAction::Back => select = None,
            SelectAction::Start { mode, night } => {
                select = None;
                match mode {
                    "classic" => classic = Some(ClassicRun::new(&calls, night, new_seed(), true)),
                    "mask" => masked = Some(masked_play::MaskRun::new(&calls, night, new_seed(), true)),
                    "lure" => dead = Some(dead_play::DeadAirRun::new(&calls, night as i32, new_seed(), true)),
                    "bedroom" => bedtime = Some(bedtime_play::BedtimeRun::new(&calls, night, new_seed(), true)),
                    "story" => facility = Some(facility_play::FacilityRun::new(&calls, night, new_seed(), true)),
                    "descent" => descent = Some(descent_play::DescentRun::with_choice(&calls, descent_play::Choice::Campaign(night as usize), new_seed(), true)),
                    "dayshift" => salvage_run = Some(salvage::SalvageRunRun::with_week(night, new_seed(), true)),
                    "mixed" => mixed = Some(MixedRun::with_shift(night as usize - 1, new_seed(), true)),
                    _ => {}
                }
            }
            SelectAction::StartCustom { mode, values } => {
                if let Some((screen, mut save)) = select.take() {
                    screen.store_values(&mut save);
                    if save.save().is_err() {
                        hub.set_toast("Could not save your progress.");
                    }
                }
                let night = select::custom_night(mode);
                let seed = new_seed();
                let levels: Vec<(&'static str, i32)> = values.iter().map(|(id, level)| (*id, *level as i32)).collect();
                match mode {
                    "classic" => classic = Some(ClassicRun::new_custom(&calls, night, &levels, seed)),
                    "mask" => masked = Some(masked_play::MaskRun::new_custom(&calls, night, &levels, seed)),
                    "lure" => dead = Some(dead_play::DeadAirRun::new_custom(&calls, night as i32, &levels, seed)),
                    "bedroom" => bedtime = Some(bedtime_play::BedtimeRun::new_custom(&calls, night, [levels[0].1, levels[1].1, levels[2].1], seed)),
                    "story" => facility = Some(facility_play::FacilityRun::new_custom(&calls, [Some(levels[0].1), Some(levels[1].1), Some(levels[2].1)], seed)),
                    "dayshift" => salvage_run = Some(salvage::SalvageRunRun::new_custom(&calls, &levels, seed)),
                    _ => {}
                }
            }
        }
        if let Some((screen, save)) = select.as_ref() {
            select::draw(screen, save);
            next_frame().await;
            continue;
        }
        if let Some(run) = facility.as_mut() {
            let controls = facility_play::poll_controls(run);
            let flow = run.update(f64::from(dt) * 1000.0, &controls);
            for id in run.take_sound_events() {
                nw_sound::event("facility", id, 0.0);
            }
            match flow {
                facility_play::Flow::Stay => {}
                facility_play::Flow::Hub => {
                    facility = None;
                    hub.set_toast("Back at the hub.");
                }
                facility_play::Flow::Night(night) => *run = facility_play::FacilityRun::new(&calls, night, new_seed(), true),
            }
            match facility.as_ref() {
                Some(run) => facility_play::draw(run, &facility_art),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = descent.as_mut() {
            let controls = descent_play::poll_controls(run);
            let flow = run.update(f64::from(dt) * 1000.0, &controls, &calls);
            for cue in run.take_cues() {
                nw_sound::event("descent", cue, 0.0);
            }
            match flow {
                descent_play::Flow::Stay => {}
                descent_play::Flow::Hub => {
                    descent = None;
                    hub.set_toast("Back at the hub.");
                }
            }
            match descent.as_ref() {
                Some(run) => descent_play::draw(run),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = mixed.as_mut() {
            let controls = mixed_play::poll_controls(run);
            let flow = run.update(f64::from(dt) * 1000.0, &controls);
            for (id, pan) in run.take_sound_events() {
                nw_sound::event("mixed", id, pan);
            }
            if flow == mixed_play::Flow::Hub {
                mixed = None;
                hub.set_toast("Back at the hub.");
            }
            match mixed.as_ref() {
                Some(run) => mixed_play::draw(run, &mixed_art),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = dead.as_mut() {
            let controls = dead_play::poll_controls(run);
            match run.update(f64::from(dt) * 1000.0, &controls) {
                dead_play::Flow::Stay => {}
                dead_play::Flow::Hub => {
                    dead = None;
                    hub.set_toast("Back at the hub.");
                }
                dead_play::Flow::Night(night) => *run = dead_play::DeadAirRun::new(&calls, night, new_seed(), true),
            }
            let art = dead_art.get_or_insert_with(dead_play::Art::load);
            match dead.as_ref() {
                Some(run) => dead_play::draw(run, art),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = bedtime.as_mut() {
            let controls = bedtime_play::poll_controls(run);
            let flow = run.update(f64::from(dt) * 1000.0, &controls);
            for c in run.take_sounds() {
                nw_sound::cue(c.name, c.pan, c.k);
            }
            match flow {
                bedtime_play::Flow::Stay => {}
                bedtime_play::Flow::Hub => {
                    bedtime = None;
                    hub.set_toast("Back at the hub.");
                }
                bedtime_play::Flow::Night(night) => *run = bedtime_play::BedtimeRun::new(&calls, night, new_seed(), true),
            }
            match bedtime.as_ref() {
                Some(run) => bedtime_play::draw(run),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = masked.as_mut() {
            let controls = masked_play::poll_controls(run);
            match run.update(f64::from(dt) * 1000.0, &controls) {
                masked_play::MaskFlow::Stay => {}
                masked_play::MaskFlow::Hub => {
                    masked = None;
                    hub.set_toast("Back at the hub.");
                }
                masked_play::MaskFlow::Night(night) => *run = masked_play::MaskRun::new(&calls, night, new_seed(), true),
            }
            match masked.as_ref() {
                Some(run) => masked_play::draw(run),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = salvage_run.as_mut() {
            let controls = salvage::poll_controls(run);
            let flow = run.update(f64::from(dt) * 1000.0, &controls, &calls);
            for cue in run.take_cues() {
                nw_sound::event("salvage_run", cue, 0.0);
            }
            match flow {
                salvage::Flow::Stay => {}
                salvage::Flow::Hub => {
                    salvage_run = None;
                    hub.set_toast("Back at the hub.");
                }
            }
            match salvage_run.as_ref() {
                Some(run) => salvage::draw(run, &art),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = classic.as_mut() {
            let controls = classic_play::poll_controls(run);
            match run.update(f64::from(dt) * 1000.0, &controls) {
                Flow::Stay => {}
                Flow::Hub => {
                    classic = None;
                    hub.set_toast("Back at the hub.");
                }
                Flow::Night(night) => *run = ClassicRun::new(&calls, night, new_seed(), true),
            }
            match classic.as_ref() {
                Some(run) => classic_play::draw(run),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
        if let Some(run) = open_air.as_mut() {
            let controls = air::poll_controls(run);
            if run.update(&calls, f64::from(dt) * 1000.0, &controls) == AirFlow::Hub {
                open_air = None;
                hub.set_toast("Back at the hub.");
            }
            match open_air.as_ref() {
                Some(run) => air::draw(run),
                None => shell::draw(&hub),
            }
            next_frame().await;
            continue;
        }
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
                    ShellAction::Start(mode_id) if select::spec(mode_id).is_some() => {
                        let save = NightWatchSave::load();
                        select = Select::open(mode_id, &save).map(|screen| (screen, save));
                    }
                    ShellAction::Start("extra") => open_air = Some(OpenAirRun::new(new_seed(), true)),
                    // a playable id has its own arm above; this arm only starts the ids still ticked with no input
                    ShellAction::Start(mode_id) if !shell::PLAYABLE.contains(&mode_id) => match modes::start(mode_id, new_seed(), &calls) {
                        Some(night) => {
                            hub.set_toast(format!("{} has no controls yet. ESC returns to the hub.", hub_label(mode_id)));
                            session = Some(night);
                        }
                        None => hub.set_toast("This night type is not ported yet."),
                    },
                    ShellAction::Start(_) | ShellAction::None => {}
                }
            }
        }
        shell::draw(&hub);
        next_frame().await;
    }
}
