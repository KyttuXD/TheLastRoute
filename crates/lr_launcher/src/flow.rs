//! The launcher's decisions, in order (docs/LAUNCHER.md has the picture):
//!
//! 1. Read `state.json` and find the build to run (`current`), checking that it is still whole on disk.
//! 2. A build is installed: start it at once and look for a newer one in the background (or first, with `--update-first`).
//!    Nothing is installed: ask the site, download, verify, install, then start it.
//! 3. No native build for this system, the OS is too old, the site refuses to verify, or the download fails: open the
//!    web version in the default browser. Only when the site cannot be reached at all and nothing is installed does the
//!    launcher stop with a message (a browser would not get through either).
//! 4. The game is started with the original arguments. A start that fails inside the startup window counts; after
//!    `fail_limit` failures in a row the build is marked bad and the launcher goes back to the previous build.

use crate::config::{Config, Mode};
use crate::log::Log;
use lr_update::fetch::{FetchError, Site};
use lr_update::install::{install_build, join_rel, verify_installed, Progress};
use lr_update::manifest::{self, Manifest};
use lr_update::osver;
use lr_update::state::{self, InstallLock, Launch, State};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The game ran (exit_code is its exit code).
    Ran,
    /// A new build was installed and nothing was started (`--no-launch`).
    Installed,
    /// The web version was opened in the browser.
    Web,
    /// The site cannot be reached and nothing is installed.
    Offline,
    /// `--check` or `--dry-run` printed its report.
    Report,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub kind: Kind,
    pub exit_code: i32,
    /// The build that ran or was installed.
    pub build: Option<u64>,
    /// What was told to the user, one line.
    pub message: String,
    pub opened: Option<String>,
}

/// Exit codes of the launcher itself (a running game's own exit code is passed through).
pub const EXIT_OFFLINE: i32 = 2;
pub const EXIT_CHECK_UPDATE_AVAILABLE: i32 = 10;

// ---- state access ------------------------------------------------------------------------------------------------

/// Reads and writes `state.json` under a process-wide mutex; every change re-reads the file first, so a second
/// launcher or the background thread never overwrites the other's change with stale data.
pub struct Store {
    dir: PathBuf,
    guard: Mutex<()>,
    log: Arc<Log>,
}

impl Store {
    pub fn new(dir: PathBuf, log: Arc<Log>) -> Store {
        Store { dir, guard: Mutex::new(()), log }
    }

    pub fn load(&self) -> State {
        let _g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        let (s, damaged) = state::load(&self.dir);
        if damaged {
            self.log.info("state.json was damaged; rebuilt it from the builds on disk");
        }
        s
    }

    pub fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let _g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        let (mut s, _) = state::load(&self.dir);
        let r = change(&mut s);
        if let Err(e) = state::save(&self.dir, &s) {
            self.log.info(&format!("could not write state.json: {e}"));
        }
        r
    }
}

// ---- the pointer decision ----------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Install,
    /// The site offers the build that is already current.
    Same,
    /// The site offers a build lower than the current one or than any installed before (a stale or replayed pointer).
    Older,
    /// The site offers a build that crashed here; it waits for a higher one.
    Bad,
}

/// Whether the build the pointer names should be installed. A build lower than anything installed before is never
/// accepted, not even with `force`.
pub fn judge(state: &State, offered: u64, force: bool) -> Verdict {
    if offered < state.floor {
        return Verdict::Older;
    }
    if state.is_bad(offered) && !force {
        return Verdict::Bad;
    }
    match state.current {
        Some(c) if offered < c => Verdict::Older,
        Some(c) if offered == c && !force => Verdict::Same,
        _ => Verdict::Install,
    }
}

// ---- checking the site and installing ----------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Checked {
    Installed(u64),
    UpToDate(u64),
    /// Nothing to install, and why (older, bad, OS too old, launcher too old, another launcher is installing).
    Skipped(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    Offline(String),
    NoNative,
    /// The signature, a hash or a rule failed.
    Refused(String),
    /// The site answered, but the check or download did not work.
    Other(String),
}

impl Fail {
    pub fn text(&self) -> String {
        match self {
            Fail::Offline(w) => format!("the site cannot be reached ({w})"),
            Fail::NoNative => "no native build is published for this system".into(),
            Fail::Refused(w) | Fail::Other(w) => w.clone(),
        }
    }
}

fn from_fetch(e: FetchError) -> Fail {
    match e {
        FetchError::Offline(w) => Fail::Offline(w),
        FetchError::NoNativeBuild => Fail::NoNative,
        FetchError::Untrusted(w) => Fail::Refused(w),
        other => Fail::Other(other.to_string()),
    }
}

fn site_of(cfg: &Config) -> Result<Site, Fail> {
    let mut site = Site::new(&cfg.base_url, &cfg.channel, &cfg.platform, cfg.trusted.clone()).map_err(Fail::Other)?;
    site.quick_seconds = cfg.quick_seconds;
    Ok(site)
}

/// What the site offers compared with the state, without changing anything (used by check/dry-run and the install).
pub enum Offer {
    Nothing(Checked),
    Available { manifest: Manifest, bytes: Vec<u8>, sig: String },
}

pub fn look(cfg: &Config, state: &State, log: &Log, cancel: &AtomicBool) -> Result<Offer, Fail> {
    let site = site_of(cfg)?;
    let latest = site.fetch_latest(Some(cancel)).map_err(from_fetch)?;
    match judge(state, latest.build, cfg.force) {
        Verdict::Same => return Ok(Offer::Nothing(Checked::UpToDate(latest.build))),
        Verdict::Older => {
            let newest = state.floor.max(state.current.unwrap_or(0));
            return Ok(Offer::Nothing(Checked::Skipped(format!("the site offers build {}, older than build {newest} that was installed before", latest.build))));
        }
        Verdict::Bad => return Ok(Offer::Nothing(Checked::Skipped(format!("build {} crashed on this computer; waiting for a newer build", latest.build)))),
        Verdict::Install => {}
    }
    let (m, bytes, sig) = site.fetch_manifest(&latest, Some(cancel)).map_err(from_fetch)?;
    if let (Some(need), Some(have)) = (&m.min_os, &cfg.os_version) {
        if !osver::at_least(have, need) {
            return Ok(Offer::Nothing(Checked::Skipped(format!("build {} needs system version {need}, this is {have}", m.build))));
        }
    }
    if m.launcher_min > manifest::LAUNCHER_LEVEL {
        log.info(&format!("build {} needs a newer launcher (level {}, this is {}); not installed", m.build, m.launcher_min, manifest::LAUNCHER_LEVEL));
        return Ok(Offer::Nothing(Checked::Skipped(format!("build {} needs a newer launcher", m.build))));
    }
    Ok(Offer::Available { manifest: m, bytes, sig })
}

fn size_text(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// Checks the site and installs a newer build when there is one. `show` prints a progress line to the console.
pub fn check_and_install(cfg: &Config, store: &Store, log: &Log, show: bool, cancel: &AtomicBool) -> Result<Checked, Fail> {
    let state = store.load();
    let (m, bytes, sig) = match look(cfg, &state, log, cancel)? {
        Offer::Nothing(c) => return Ok(c),
        Offer::Available { manifest, bytes, sig } => (manifest, bytes, sig),
    };
    let Some(_lock) = InstallLock::acquire(&cfg.dir, 1800) else {
        return Ok(Checked::Skipped("another launcher is installing".into()));
    };
    log.info(&format!("installing build {} ({} files, {} to download)", m.build, m.files.len(), size_text(m.total_download())));
    let site = site_of(cfg)?;
    let tmp = state::tmp_dir(&cfg.dir);
    let stage = tmp.join(format!("{}.partial", m.build));
    let scratch = tmp.join(format!("{}.dl", m.build));
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_dir_all(&scratch);
    let mut last_line = String::new();
    let mut report = |p: &Progress| {
        if show && cfg.echo {
            let line = format!("Downloading build {}: {}% ({} of {} files)", p.build, p.percent(), p.file.min(p.files), p.files);
            if line != last_line {
                eprint!("\r{line}   ");
                let _ = std::io::stderr().flush();
                last_line = line;
            }
        }
    };
    let installed = install_build(&site, &m, &stage, &scratch, None, Some(cancel), &mut report);
    if show && cfg.echo {
        eprintln!();
    }
    installed.map_err(from_fetch)?;
    // From here on a failure removes the stage again: the old build stays current and runnable.
    let finish = (|| -> Result<(), String> {
        std::fs::write(stage.join(state::MANIFEST_COPY), &bytes).map_err(|e| e.to_string())?;
        std::fs::write(stage.join(state::SIGNATURE_COPY), &sig).map_err(|e| e.to_string())?;
        verify_installed(&stage, &m, false)?;
        let dest = state::build_dir(&cfg.dir, m.build);
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| format!("could not replace build {}: {e}", m.build))?;
        }
        std::fs::create_dir_all(state::builds_dir(&cfg.dir)).map_err(|e| e.to_string())?;
        rename_folder(&stage, &dest)
    })();
    if let Err(e) = finish {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(Fail::Other(e));
    }
    let forced = cfg.force;
    let keep = store.update(|s| {
        if forced {
            s.bad.retain(|b| *b != m.build);
        }
        s.adopt(m.build);
        [s.current, s.previous]
    });
    let keep: Vec<u64> = keep.into_iter().flatten().collect();
    state::collect_garbage(&cfg.dir, &keep);
    log.info(&format!("installed build {} (current), previous {:?}", m.build, keep.get(1)));
    Ok(Checked::Installed(m.build))
}

// ---- the installed build -----------------------------------------------------------------------------------------

/// Renames the finished folder into place. Windows refuses to rename a folder while a virus scanner still has one of the
/// freshly written files open, so a few tries are made before giving up.
fn rename_folder(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..8 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(150 * (attempt + 1)));
    }
    Err(format!("could not move the finished build into place: {last}"))
}

/// The manifest of an installed build when the build is whole: its saved manifest still verifies against a trusted key,
/// every file is there with the right size (and hash, with `--verify`).
pub fn usable(cfg: &Config, build: u64, log: &Log) -> Option<Manifest> {
    let dir = state::build_dir(&cfg.dir, build);
    let bytes = std::fs::read(dir.join(state::MANIFEST_COPY)).ok()?;
    let sig = std::fs::read_to_string(dir.join(state::SIGNATURE_COPY)).ok()?;
    if let Err(e) = lr_update::verify::verify_signature(&cfg.trusted, &bytes, &sig) {
        log.info(&format!("installed build {build} cannot be trusted: {e}"));
        return None;
    }
    let m = Manifest::parse(&bytes, &cfg.platform).map_err(|e| log.info(&format!("installed build {build}: {e}"))).ok()?;
    if m.build != build {
        return None;
    }
    if let Err(e) = verify_installed(&dir, &m, cfg.verify_deep) {
        log.info(&format!("installed build {build} is damaged: {e}"));
        return None;
    }
    Some(m)
}

struct GameRun {
    /// Some(code) when the process ended; None when it could not be started.
    code: Option<i32>,
    ok: bool,
    /// The failure count reached the limit and the launcher rolled back.
    rolled_back: bool,
}

fn run_game(cfg: &Config, store: &Store, log: &Log, build: u64, m: &Manifest) -> GameRun {
    let dir = state::build_dir(&cfg.dir, build);
    let exe = join_rel(&dir, &m.run);
    store.update(|s| s.launch = Some(Launch { build, started: state::now_unix(), result: "pending".into() }));
    log.info(&format!("starting build {build}: {}", exe.display()));
    let mut command = Command::new(&exe);
    command
        .args(&cfg.game_args)
        .current_dir(&dir)
        .env("LR_LAUNCHER", "1")
        .env("LR_BUILD", build.to_string())
        .env("LR_LAUNCHER_VERSION", LAUNCHER_VERSION);
    let started = Instant::now();
    let child = command.spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            log.info(&format!("could not start build {build}: {e}"));
            let rolled_back = record_failure(cfg, store, log, build);
            return GameRun { code: None, ok: false, rolled_back };
        }
    };
    if let Some(started_hook) = &cfg.on_game_started {
        started_hook();
    }
    let mut marked_ok = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(e),
        }
        if !marked_ok && started.elapsed() >= cfg.startup_window {
            marked_ok = true;
            store.update(|s| {
                if s.current == Some(build) {
                    s.failures = 0;
                }
                s.launch = Some(Launch { build, started: state::now_unix(), result: "ok".into() });
            });
            log.info(&format!("build {build} is still running after {} s: counted as a good start", cfg.startup_window.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            log.info(&format!("lost track of build {build}: {e}"));
            return GameRun { code: None, ok: true, rolled_back: false };
        }
    };
    let code = status.code();
    log.info(&format!("build {build} ended after {} ms, exit code {:?}", started.elapsed().as_millis(), code));
    if marked_ok || code == Some(0) {
        if !marked_ok {
            store.update(|s| {
                if s.current == Some(build) {
                    s.failures = 0;
                }
                s.launch = Some(Launch { build, started: state::now_unix(), result: "ok".into() });
            });
        }
        return GameRun { code, ok: true, rolled_back: false };
    }
    // Non-zero or killed by a signal, inside the startup window: a failed start.
    let rolled_back = record_failure(cfg, store, log, build);
    GameRun { code, ok: false, rolled_back }
}

/// Counts a failed start of `build`. At the limit the build is marked bad and `current` goes back to `previous`.
fn record_failure(cfg: &Config, store: &Store, log: &Log, build: u64) -> bool {
    enum Effect {
        /// A newer build became current while this one ran: the failure is not counted against it.
        Ignored,
        Counted(u32),
        RolledBack(Option<u64>),
    }
    let limit = cfg.fail_limit;
    let effect = store.update(|s| {
        s.launch = Some(Launch { build, started: state::now_unix(), result: "failed".into() });
        if s.current != Some(build) {
            return Effect::Ignored;
        }
        s.failures += 1;
        if s.failures >= limit {
            Effect::RolledBack(s.roll_back())
        } else {
            Effect::Counted(s.failures)
        }
    });
    match effect {
        Effect::Ignored => {
            log.info(&format!("build {build} failed to start, but a newer build was installed meanwhile; not counted"));
            false
        }
        Effect::Counted(n) => {
            log.info(&format!("build {build} failed to start ({n} of {limit} before it is rolled back)"));
            false
        }
        Effect::RolledBack(Some(prev)) => {
            log.info(&format!("build {build} failed to start {limit} times in a row: marked bad, rolled back to build {prev}"));
            true
        }
        Effect::RolledBack(None) => {
            log.info(&format!("build {build} failed to start {limit} times in a row: marked bad, and there is no earlier build to go back to"));
            true
        }
    }
}

// ---- the whole run -----------------------------------------------------------------------------------------------

fn web_outcome(cfg: &Config, log: &Log, why: &str) -> Outcome {
    let message = format!("Opening the web version at {} ({why})", cfg.web_url);
    log.info(&message);
    match (cfg.opener)(&cfg.web_url) {
        Ok(()) => Outcome { kind: Kind::Web, exit_code: 0, build: None, message, opened: Some(cfg.web_url.clone()) },
        Err(e) => {
            let message = format!("Could not open the browser ({e}). Open {} yourself to play the web version.", cfg.web_url);
            log.info(&message);
            Outcome { kind: Kind::Web, exit_code: 1, build: None, message, opened: None }
        }
    }
}

fn offline_outcome(log: &Log, cfg: &Config, why: &str) -> Outcome {
    let message = format!("Cannot reach {} and no game is installed yet ({why}). Connect to the internet and start again.", cfg.base_url);
    log.info(&message);
    Outcome { kind: Kind::Offline, exit_code: EXIT_OFFLINE, build: None, message, opened: None }
}

/// Runs the launcher once. Never panics on bad input; every failure ends in a message and an exit code.
pub fn run(cfg: &Config) -> Outcome {
    if cfg.mode == Mode::Rollback {
        return roll_back_by_hand(cfg);
    }
    if cfg.mode != Mode::Run {
        return crate::report::report(cfg);
    }
    let _ = std::fs::create_dir_all(&cfg.dir);
    let log = Arc::new(Log::open(Some(&cfg.dir), cfg.echo));
    log.info(&format!(
        "lr_launcher {LAUNCHER_VERSION}: platform {}, channel {:?}, site {}, folder {}, args {:?}",
        cfg.platform, cfg.channel, cfg.base_url, cfg.dir.display(), cfg.game_args
    ));
    let store = Arc::new(Store::new(cfg.dir.clone(), log.clone()));
    // Leftovers of a download that was cut off (the launcher killed, the power gone) are removed at the start,
    // unless another launcher is installing right now.
    if let Some(_lock) = InstallLock::acquire(&cfg.dir, 1800) {
        state::remove_tmp(&cfg.dir);
    }
    if let Some(l) = store.load().launch.filter(|l| l.result == "pending") {
        log.info(&format!("the last start of build {} never reported an end (the launcher was closed?); not counted", l.build));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let mut tried_foreground = false;
    for _ in 0..6 {
        let state = store.load();
        let current = state.current.and_then(|b| usable(cfg, b, &log).map(|m| (b, m)));
        if state.current.is_some() && current.is_none() {
            // Whole build missing or damaged: forget it (without calling it bad) and use the previous one if there is one.
            let gone = state.current.unwrap_or(0);
            store.update(|s| {
                s.current = s.previous.take();
                s.failures = 0;
            });
            log.info(&format!("build {gone} is not usable; dropped it"));
            continue;
        }
        let Some((build, manifest)) = current else {
            // Nothing installed.
            if tried_foreground {
                return web_outcome(cfg, &log, "no native build could be installed");
            }
            tried_foreground = true;
            if cfg.echo {
                eprintln!("Setting up The Last Route (first start)...");
            }
            match check_and_install(cfg, &store, &log, true, &cancel) {
                Ok(Checked::Installed(b)) => {
                    if cfg.no_launch {
                        return Outcome { kind: Kind::Installed, exit_code: 0, build: Some(b), message: format!("installed build {b}"), opened: None };
                    }
                    continue;
                }
                Ok(Checked::UpToDate(_)) => continue,
                Ok(Checked::Skipped(why)) => return web_outcome(cfg, &log, &why),
                Err(Fail::Offline(w)) => return offline_outcome(&log, cfg, &w),
                Err(f) => return web_outcome(cfg, &log, &f.text()),
            }
        };
        // A build is installed.
        let mut manifest = manifest;
        let mut build = build;
        if cfg.update_first || cfg.force {
            match check_and_install(cfg, &store, &log, true, &cancel) {
                Ok(Checked::Installed(b)) => {
                    if let Some(m) = usable(cfg, b, &log) {
                        build = b;
                        manifest = m;
                    }
                }
                Ok(other) => log.info(&format!("update check: {other:?}")),
                Err(f) => log.info(&format!("update check failed: {}", f.text())),
            }
        }
        if cfg.no_launch {
            return Outcome { kind: Kind::Installed, exit_code: 0, build: Some(build), message: format!("build {build} is installed"), opened: None };
        }
        // Check for a newer build while the game runs (not when it was just done above).
        let background = if cfg.update_first || cfg.force {
            None
        } else {
            let (cfg2, store2, log2, cancel2) = (cfg.clone(), store.clone(), log.clone(), cancel.clone());
            Some(std::thread::spawn(move || match check_and_install(&cfg2, &store2, &log2, false, &cancel2) {
                Ok(Checked::Installed(b)) => log2.info(&format!("build {b} is ready for the next start")),
                Ok(other) => log2.info(&format!("update check: {other:?}")),
                Err(f) => log2.info(&format!("update check failed: {}", f.text())),
            }))
        };
        let result = run_game(cfg, &store, &log, build, &manifest);
        if let Some(handle) = background {
            wait_for_background(cfg, &log, &cancel, handle);
        }
        let keep: Vec<u64> = {
            let s = store.load();
            [s.current, s.previous].into_iter().flatten().collect()
        };
        state::collect_garbage(&cfg.dir, &keep);
        if result.rolled_back {
            tried_foreground = false;
            continue;
        }
        let exit_code = result.code.unwrap_or(1);
        let message = if result.ok { format!("build {build} ran") } else { format!("build {build} failed to start") };
        return Outcome { kind: Kind::Ran, exit_code, build: Some(build), message, opened: None };
    }
    web_outcome(cfg, &log, "no native build could be started")
}

/// `--rollback`: the current build is marked bad and the previous one becomes current.
fn roll_back_by_hand(cfg: &Config) -> Outcome {
    let _ = std::fs::create_dir_all(&cfg.dir);
    let log = Arc::new(Log::open(Some(&cfg.dir), cfg.echo));
    let store = Store::new(cfg.dir.clone(), log.clone());
    let (was, to) = store.update(|s| {
        let was = s.current;
        (was, s.roll_back())
    });
    let keep: Vec<u64> = {
        let s = store.load();
        [s.current, s.previous].into_iter().flatten().collect()
    };
    state::collect_garbage(&cfg.dir, &keep);
    let (message, exit_code) = match (was, to) {
        (Some(w), Some(p)) => (format!("rolled back: build {w} is marked bad, build {p} is current"), 0),
        (Some(w), None) => (format!("build {w} is marked bad; there is no earlier build, so the next start installs or opens the web version"), 1),
        (None, _) => ("nothing is installed".to_string(), 1),
    };
    log.info(&message);
    Outcome { kind: Kind::Report, exit_code, build: to, message, opened: None }
}

fn wait_for_background(cfg: &Config, log: &Log, cancel: &AtomicBool, handle: std::thread::JoinHandle<()>) {
    let start = Instant::now();
    while !handle.is_finished() {
        if start.elapsed() > cfg.background_wait {
            log.info("the background update is still running; giving up on it for this start");
            cancel.store(true, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(300));
            return;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let _ = handle.join();
}
