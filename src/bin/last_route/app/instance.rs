//! Which profile this process runs on, chosen before anything reads or writes a user file (docs/PROFILES.md).
//!
//! `main` calls [`select`] first: before the log opens, before the settings and the graphics marker are read. It
//! points `paths::user_dir` at the profile's folder and takes the profile's `instance.lock`, or asks for the profile
//! screen (ui/profile_screen.rs). That screen runs as its own short process on a scratch folder and then starts the
//! game again with `--profile NAME`; SWITCH PROFILE on the title does the same with `--profiles`. A restart is used
//! instead of a reload in place because the log, the window's graphics choice, the settings, the account session,
//! the skins and the save are all read once at start.
//!
//! The rules (names, index, folders, lock) are shared/profiles.rs; this file holds the process's one choice.

use crate::shared::profiles::{self, Index, LockState};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The profile this process holds the lock of.
struct Active {
    dir: PathBuf,
    started: u64,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);
/// True when the game started on the one profile without being told which: a held SHIFT may still ask for the screen.
static IMPLICIT: AtomicBool = AtomicBool::new(false);
/// Lines for the log, kept until it is open (`select` runs before `log::init`).
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What `main` does next.
pub enum Start {
    /// Open the game on the chosen profile (the paths are set).
    Game,
    /// Show the profile screen.
    Picker(Picker),
}

/// What the profile screen is given.
pub struct Picker {
    /// The root user folder (`paths::root_user_dir`).
    pub root: PathBuf,
    /// The profile that was asked for but is open in another instance.
    pub in_use: Option<String>,
}

/// Command-line modes that never show the profile screen and take no lock: the windowless modes, the agent window,
/// the self-test, the screenshot, the stress run and the movie capture. They run on the root folder unless
/// `--profile NAME` or `LR_PROFILE_DIR` says otherwise.
const AUTOMATION: [&str; 12] = [
    "validate-pack",
    "agent-window",
    "dedicated-host",
    "test-suite",
    "bot",
    "playtest-runner",
    "headless",
    "playtest",
    "self-test",
    "screenshot",
    "stress",
    "capture",
];

/// The time now, in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn note(line: String) {
    if let Ok(mut notes) = NOTES.lock() {
        notes.push(line);
    }
}

/// Writes the lines `select` kept to the log. Call once, after `log::init`.
pub fn log_start() {
    let notes = NOTES.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default();
    for line in notes {
        crate::lr_info!("{line}");
    }
}

/// The profile `typed` names, added to the index when new. A bad name ends the process with the reason.
fn ensure_or_exit(index: &mut Index, root: &std::path::Path, typed: &str, now: u64) -> String {
    match index.ensure(root, typed, now) {
        Ok(name) => name,
        Err(e) => {
            let reason = crate::ui::profile_screen::labels().error(&e);
            eprintln!("--profile {typed:?}: {reason}");
            std::process::exit(2);
        }
    }
}

/// Chooses the profile. Call first in `main`, before the log opens.
pub fn select() -> Start {
    let args = crate::headless::Args::get();
    let automation = AUTOMATION.iter().any(|f| args.flag(f));
    let now = unix_now();
    // LR_PROFILE_DIR wins: the folder it names is the profile, with no index and no screen
    if let Some(dir) = std::env::var_os(super::profile::PROFILE_DIR_ENV)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
    {
        let _ = std::fs::create_dir_all(&dir);
        super::paths::set_profile_dir(Some(dir.clone()));
        note(format!("profile: LR_PROFILE_DIR folder {}", dir.display()));
        if !automation {
            hold(dir, now);
        }
        return Start::Game;
    }
    let Some(root) = super::paths::root_user_dir() else {
        return Start::Game;
    };
    let wanted = profiles::profile_arg(&args.raw);
    let mut index = Index::load(&root);
    if automation {
        if let Some(typed) = wanted {
            let name = ensure_or_exit(&mut index, &root, &typed, now);
            let _ = index.save(&root);
            point_at(&root, &name);
        }
        return Start::Game;
    }
    let ask = args.raw.iter().any(|a| a == "--profiles");
    let force = args.raw.iter().any(|a| a == "--profile-force");
    let target = if ask {
        None
    } else if let Some(typed) = &wanted {
        Some(ensure_or_exit(&mut index, &root, typed, now))
    } else if index.profiles.len() > 1 {
        None
    } else {
        Some(profiles::DEFAULT_NAME.to_string())
    };
    let mut in_use = None;
    if let Some(name) = target {
        let dir = profiles::profile_dir(&root, &name);
        let stale = crate::ui::profile_screen::labels().stale_after_seconds;
        let held = match profiles::lock_state(&dir, std::process::id(), now, stale) {
            LockState::InUse(lock) => Some(lock),
            _ => None,
        };
        match held {
            Some(lock) if !force => {
                note(format!("profile: {name} is in use by process {} (started {})", lock.pid, lock.started));
                in_use = Some(name);
            }
            _ => {
                if let Some(lock) = held {
                    note(format!("profile: {name} is in use by process {}; continuing anyway", lock.pid));
                }
                point_at(&root, &name);
                hold(dir, now);
                index.touch(&name, now);
                if let Err(e) = index.save(&root) {
                    note(format!("profile: the index was not saved: {e}"));
                }
                IMPLICIT.store(wanted.is_none(), Ordering::Relaxed);
                return Start::Game;
            }
        }
    }
    // the profile screen's own process: a scratch folder, so its log and graphics marker touch no profile
    let scratch = profiles::picker_dir(&root);
    let _ = std::fs::create_dir_all(&scratch);
    super::paths::set_profile_dir(Some(scratch));
    note("profile: showing the profile screen".to_string());
    Start::Picker(Picker { root, in_use })
}

/// Points the user files at profile `name` (DEFAULT is the root folder itself).
fn point_at(root: &std::path::Path, name: &str) {
    if name == profiles::DEFAULT_NAME {
        super::paths::set_profile_dir(None);
    } else {
        super::paths::set_profile_dir(Some(profiles::profile_dir(root, name)));
    }
    note(format!("profile: {name} in {}", profiles::profile_dir(root, name).display()));
}

/// Takes the lock of the profile folder `dir` and keeps it fresh from a small thread until [`release`].
fn hold(dir: PathBuf, now: u64) {
    let pid = std::process::id();
    if let Err(e) = profiles::write_lock(&dir, pid, now, now) {
        note(format!("profile: the lock file was not written: {e}"));
    }
    if let Ok(mut active) = ACTIVE.lock() {
        *active = Some(Active { dir, started: now });
    }
    let beat = crate::ui::profile_screen::labels().heartbeat_seconds.max(1);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(beat));
        let Ok(active) = ACTIVE.lock() else { break };
        let Some(a) = active.as_ref() else { break };
        let _ = profiles::write_lock(&a.dir, pid, a.started, unix_now());
    });
}

/// Gives the profile's lock back. Call when the game ends; a process that dies without it leaves a lock that goes
/// stale by its age.
pub fn release() {
    let active = ACTIVE.lock().ok().and_then(|mut a| a.take());
    if let Some(a) = active {
        profiles::release_lock(&a.dir, std::process::id());
    }
}

/// Starts the game again with the same command line, the profile options replaced by `extra`. True when the new
/// process started: the caller then ends this one. The lock is given back first, so the new process finds it free.
pub fn restart(extra: &[&str]) -> bool {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let extra: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
    let args = profiles::restart_args(&raw, &extra);
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            crate::lr_warn!("profile: no path to restart: {e}");
            return false;
        }
    };
    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(_) => {
            crate::lr_info!("profile: restarting with {args:?}");
            release();
            super::log::flush();
            true
        }
        Err(e) => {
            crate::lr_warn!("profile: the restart failed: {e}");
            false
        }
    }
}

/// SWITCH PROFILE: starts the game again on the profile screen. False when it could not (the caller says so), and
/// when `LR_PROFILE_DIR` fixes the folder, which no screen can change.
pub fn restart_to_picker() -> bool {
    if std::env::var_os(super::profile::PROFILE_DIR_ENV).map_or(false, |d| !d.is_empty()) {
        crate::lr_warn!("profile: LR_PROFILE_DIR fixes the folder; there is no profile to switch to");
        return false;
    }
    restart(&["--profiles"])
}

/// True while SHIFT is held in the first moments of the title, for a game that started on its one profile without
/// being told which: the player asks for the profile screen. Call once per frame.
pub fn shift_wants_picker() -> bool {
    use macroquad::prelude::{is_key_down, KeyCode};
    if !IMPLICIT.load(Ordering::Relaxed) {
        return false;
    }
    static FIRST: Mutex<Option<f64>> = Mutex::new(None);
    let now = macroquad::time::get_time();
    let first = match FIRST.lock() {
        Ok(mut f) => *f.get_or_insert(now),
        Err(_) => now,
    };
    if now - first > f64::from(crate::ui::profile_screen::labels().shift_hold_seconds) {
        IMPLICIT.store(false, Ordering::Relaxed);
        return false;
    }
    is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_modes_that_skip_the_profile_screen_are_spelled_as_their_options() {
        // Args::flag compares names with - and _ as one, so `test-suite` also covers `--test_suite`
        for name in AUTOMATION {
            assert!(!name.starts_with('-') && !name.is_empty(), "{name}");
            let args = crate::headless::Args { raw: vec![format!("--{name}")] };
            assert!(AUTOMATION.iter().any(|f| args.flag(f)), "{name}");
        }
        let plain = crate::headless::Args { raw: vec!["--profile".into(), "TWO".into(), "--offline".into()] };
        assert!(!AUTOMATION.iter().any(|f| plain.flag(f)));
    }

    #[test]
    fn the_time_is_after_2020() {
        assert!(unix_now() > 1_577_836_800);
    }
}
