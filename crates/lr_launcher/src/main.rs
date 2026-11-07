//! `lr_launcher`: see the library crate and docs/LAUNCHER.md.

use lr_launcher::args::{self, Parsed};
use lr_launcher::{run, Kind};
use std::sync::Arc;

/// On Windows the launcher is a console program (so first-run progress can print). When the console window belongs to
/// the launcher alone (it was started by a double click), the window is closed once the game is running, so it does not
/// stay open for the whole play session. When it was started from a terminal, the terminal is left alone.
#[cfg(windows)]
mod console {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        fn FreeConsole() -> i32;
    }

    /// True when this process is the only one attached to its console window (started by a double click).
    pub fn is_ours() -> bool {
        let mut ids = [0u32; 4];
        // SAFETY: a plain Win32 call with a valid buffer.
        unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 }
    }

    pub fn release() {
        if is_ours() {
            // SAFETY: a plain Win32 call.
            unsafe {
                FreeConsole();
            }
        }
    }
}

#[cfg(not(windows))]
mod console {
    pub fn is_ours() -> bool {
        false
    }

    pub fn release() {}
}

/// A message in a console window that is about to close (a double click) would flash by unread: wait for Enter first.
fn hold_own_console() {
    if console::is_ours() {
        eprintln!("Press Enter to close this window.");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
}

fn main() {
    let parsed = args::parse(std::env::args_os().skip(1).collect(), std::env::var("LR_NATIVE_PUBKEY").ok(), lr_update::paths::install_dir);
    let mut cfg = match parsed {
        Parsed::Run(cfg) => *cfg,
        Parsed::Info(text) => {
            println!("{text}");
            return;
        }
        Parsed::Error(e) => {
            eprintln!("lr_launcher: {e}");
            std::process::exit(64);
        }
    };
    if !cfg.test_mode && std::env::var_os("LR_LAUNCHER_KEEP_CONSOLE").is_none() {
        cfg.on_game_started = Some(Arc::new(console::release));
    }
    let outcome = run(&cfg);
    match outcome.kind {
        Kind::Report => println!("{}", outcome.message),
        Kind::Ran => {}
        // The message was already written to the console by the log; only keep a closing window readable.
        _ if cfg.echo && outcome.exit_code != 0 => hold_own_console(),
        _ => {}
    }
    if cfg.test_mode {
        let result = serde_json::json!({
            "kind": format!("{:?}", outcome.kind),
            "exit": outcome.exit_code,
            "build": outcome.build,
            "message": outcome.message,
            "opened": outcome.opened,
        });
        println!("LAUNCHER_RESULT {result}");
    }
    std::process::exit(outcome.exit_code);
}
