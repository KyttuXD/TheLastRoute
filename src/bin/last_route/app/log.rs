//! Debug log: `logs/last_route.log` in the per-user folder (the previous run's log kept as
//! `last_route.prev.log`), mirrored to stderr when there is a console. The level comes from
//! `LR_LOG` (error, warn, info, debug, trace; default info) or `--log-level <level>`.
//! Panics are written to the log before the program exits.
//!
//!   crate::lr_info!("run started seed={seed}");
//!   crate::lr_debug!("window recentred to {:?}", center);

use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
#[repr(u8)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
static START: Mutex<Option<Instant>> = Mutex::new(None);

fn parse(s: &str) -> Option<Level> {
    Some(match s.to_ascii_lowercase().as_str() {
        "error" => Level::Error,
        "warn" | "warning" => Level::Warn,
        "info" => Level::Info,
        "debug" => Level::Debug,
        "trace" => Level::Trace,
        _ => return None,
    })
}

/// Opens the log file and installs the panic hook. Call once at start.
pub fn init() {
    let args: Vec<String> = std::env::args().collect();
    let from_arg = args
        .iter()
        .position(|a| a == "--log-level")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| parse(s));
    if let Some(l) = from_arg.or_else(|| std::env::var("LR_LOG").ok().and_then(|s| parse(&s))) {
        LEVEL.store(l as u8, Ordering::Relaxed);
    }
    *START.lock().unwrap() = Some(Instant::now());
    if let Some(dir) = super::paths::user_dir().map(|d| d.join("logs")) {
        let _ = std::fs::create_dir_all(&dir);
        let cur = dir.join("last_route.log");
        let _ = std::fs::rename(&cur, dir.join("last_route.prev.log"));
        *FILE.lock().unwrap() = std::fs::File::create(&cur).ok();
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write(Level::Error, format_args!("PANIC: {info}"));
        prev(info);
    }));
    write(
        Level::Info,
        format_args!(
            "Last Route port {} on {} {} // args {:?}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            &args[1..]
        ),
    );
    if let Some(root) = super::paths::docs_dir() {
        let project_notes = super::paths::docs_file(std::path::Path::new("kyttu-docs/README.md"))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".to_owned());
        write(
            Level::Info,
            format_args!(
                "offline docs root={} project_notes={project_notes}",
                root.display()
            ),
        );
    }
}

pub fn enabled(l: Level) -> bool {
    l as u8 <= LEVEL.load(Ordering::Relaxed)
}

pub fn write(l: Level, msg: std::fmt::Arguments) {
    if !enabled(l) {
        return;
    }
    let t = START
        .lock()
        .unwrap()
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let line = format!("[{t:10.3}] {:<5} {msg}\n", format!("{l:?}").to_uppercase());
    if let Some(f) = FILE.lock().unwrap().as_mut() {
        let _ = f.write_all(line.as_bytes());
        if l <= Level::Warn {
            let _ = f.flush();
        }
    }
    // stderr is invisible (or absent) for the windowed game; harmless to write
    let _ = std::io::stderr().write_all(line.as_bytes());
}

pub fn flush() {
    if let Some(f) = FILE.lock().unwrap().as_mut() {
        let _ = f.flush();
    }
}

#[macro_export]
macro_rules! lr_error { ($($t:tt)*) => { $crate::app::log::write($crate::app::log::Level::Error, format_args!($($t)*)) } }
#[macro_export]
macro_rules! lr_warn { ($($t:tt)*) => { $crate::app::log::write($crate::app::log::Level::Warn, format_args!($($t)*)) } }
#[macro_export]
macro_rules! lr_info { ($($t:tt)*) => { $crate::app::log::write($crate::app::log::Level::Info, format_args!($($t)*)) } }
#[macro_export]
macro_rules! lr_debug { ($($t:tt)*) => { $crate::app::log::write($crate::app::log::Level::Debug, format_args!($($t)*)) } }
#[macro_export]
macro_rules! lr_trace { ($($t:tt)*) => { $crate::app::log::write($crate::app::log::Level::Trace, format_args!($($t)*)) } }
