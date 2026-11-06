//! A stand-in for the game in the launcher's tests. It is published as the "game" of a fake build; what it does comes
//! from `data/behavior.txt` in its own folder (the launcher runs it with that folder as the working directory):
//!   `exit N`    exit with code N (default 0)
//!   `sleep MS`  stay alive that long first
//! It appends a line to `ran.txt` so a test can see that it ran, with which arguments and which launcher variables.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let line = format!(
        "args={:?} launcher={} build={} cwd_ok={}\n",
        args,
        std::env::var("LR_LAUNCHER").unwrap_or_default(),
        std::env::var("LR_BUILD").unwrap_or_default(),
        std::path::Path::new("data").is_dir()
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("ran.txt") {
        let _ = f.write_all(line.as_bytes());
    }
    let behavior = std::fs::read_to_string("data/behavior.txt").unwrap_or_default();
    let mut code = 0;
    for l in behavior.lines() {
        let mut parts = l.split_whitespace();
        match (parts.next(), parts.next().and_then(|v| v.parse::<u64>().ok())) {
            (Some("exit"), Some(n)) => code = n as i32,
            (Some("sleep"), Some(ms)) => std::thread::sleep(std::time::Duration::from_millis(ms)),
            _ => {}
        }
    }
    std::process::exit(code);
}
