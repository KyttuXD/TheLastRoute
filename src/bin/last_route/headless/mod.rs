//! Agent and test modes: the run without a window (no GPU, no audio device), driven over JSON
//! lines by an agent, by suite files, or by the built-in bot. Picked from the command line
//! before any window exists (main.rs):
//!
//! - `--headless [--port N | --playtest=N] [--no-stdin]`: the agent protocol on stdin/stdout
//!   (and a TCP port on 127.0.0.1).
//! - `--test-suite <files|dirs|globs...> [--out report.json]`: runs suite files, prints a JSON
//!   report, exit 0 pass / 1 fail / 2 could not run.
//! - `--bot` / `--playtest-runner --scenario=survive ...`: the bot plays a run (playtest_runner.gd).
//! - `--agent-window [--port N]`: the same protocol with the game window drawing the run
//!   (screenshots work).
//!
//! See docs/AGENT_CLI.md.

pub mod bot;
pub mod expect;
pub mod protocol;
pub mod runner;
pub mod sim;
pub mod suite;
pub mod window;

use protocol::Session;
use serde_json::Value;
use std::io::{BufRead, Write};
use std::sync::mpsc;

/// Options as `--key value` or `--key=value` (dashes inside keys read as underscores).
pub struct Args {
    pub raw: Vec<String>,
}

impl Args {
    pub fn get() -> Args {
        Args {
            raw: std::env::args().skip(1).collect(),
        }
    }

    pub fn flag(&self, name: &str) -> bool {
        let n = norm(name);
        self.raw.iter().any(|a| {
            a.starts_with("--")
                && norm(a.trim_start_matches('-').split('=').next().unwrap_or("")) == n
        })
    }

    pub fn value(&self, name: &str) -> Option<String> {
        let n = norm(name);
        for (i, a) in self.raw.iter().enumerate() {
            if !a.starts_with("--") {
                continue;
            }
            let body = a.trim_start_matches('-');
            let (k, v) = match body.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (body, None),
            };
            if norm(k) != n {
                continue;
            }
            return v.or_else(|| {
                self.raw
                    .get(i + 1)
                    .filter(|x| !x.starts_with("--"))
                    .cloned()
            });
        }
        None
    }

    /// Every value after `name` until the next `--option`.
    pub fn values(&self, name: &str) -> Vec<String> {
        let n = norm(name);
        let mut out = vec![];
        let mut on = false;
        for a in &self.raw {
            if a.starts_with("--") {
                let body = a.trim_start_matches('-');
                let (k, v) = match body.split_once('=') {
                    Some((k, v)) => (k, Some(v)),
                    None => (body, None),
                };
                on = norm(k) == n;
                if on {
                    if let Some(v) = v {
                        out.push(v.to_string());
                    }
                }
            } else if on {
                out.push(a.clone());
            }
        }
        out
    }
}

fn norm(s: &str) -> String {
    s.replace('-', "_").to_ascii_lowercase()
}

/// True when `--agent-window` was given (the windowed agent mode; main.rs opens the window).
pub fn wants_agent_window() -> bool {
    Args::get().flag("agent-window")
}

/// Runs a windowless mode if one was asked for and returns the process exit code.
pub fn dispatch() -> Option<i32> {
    let a = Args::get();
    if a.flag("agent-window") {
        return None;
    }
    if a.flag("test-suite") || a.flag("test_suite") {
        return Some(suite::main(&a));
    }
    if a.flag("bot") || a.flag("playtest-runner") {
        return Some(runner::main(&a));
    }
    if a.flag("headless") || a.flag("playtest") {
        return Some(serve(&a));
    }
    None
}

/// Where a reply goes.
pub enum Reply {
    Stdout,
    Chan(mpsc::Sender<String>),
}

pub type Inbox = mpsc::Receiver<Option<(String, Reply)>>;

/// The TCP port from `--port N` or `--playtest=N` (the bridge's spelling; 47600 by default
/// for a bare `--playtest` without `--headless`).
pub fn port_arg(a: &Args) -> Option<u16> {
    a.value("port")
        .and_then(|p| p.parse().ok())
        .or_else(|| a.value("playtest").and_then(|p| p.parse().ok()))
}

/// Starts the readers: stdin (unless `--no-stdin`) and the TCP listener. `None` on the channel
/// means stdin closed.
pub fn spawn_inputs(a: &Args) -> Result<(Inbox, bool), String> {
    let (tx, rx) = mpsc::channel::<Option<(String, Reply)>>();
    let port = port_arg(a);
    if let Some(port) = port {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("could not listen on 127.0.0.1:{port}: {e}"))?;
        eprintln!("PLAYTEST listening on 127.0.0.1:{port}");
        let tx = tx.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let Ok(read) = stream.try_clone() else { return };
                    let mut write = stream;
                    for line in std::io::BufReader::new(read).lines() {
                        let Ok(line) = line else { break };
                        if line.trim().is_empty() {
                            continue;
                        }
                        let (rtx, rrx) = mpsc::channel();
                        if tx.send(Some((line, Reply::Chan(rtx)))).is_err() {
                            return;
                        }
                        let Ok(reply) = rrx.recv() else { return };
                        if writeln!(write, "{reply}").is_err() {
                            return;
                        }
                    }
                });
            }
        });
    }
    let stdin = !a.flag("no-stdin");
    if stdin {
        std::thread::spawn(move || {
            let si = std::io::stdin();
            for line in si.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if tx.send(Some((line, Reply::Stdout))).is_err() {
                    return;
                }
            }
            let _ = tx.send(None);
        });
    }
    Ok((rx, port.is_some()))
}

pub fn respond(to: Reply, v: &Value) {
    let text = v.to_string();
    match to {
        Reply::Stdout => {
            let out = std::io::stdout();
            let mut l = out.lock();
            let _ = writeln!(l, "{text}");
            let _ = l.flush();
        }
        Reply::Chan(tx) => {
            let _ = tx.send(text);
        }
    }
}

/// `--headless`: the protocol loop without a window.
fn serve(a: &Args) -> i32 {
    let (rx, tcp) = match spawn_inputs(a) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let mut s = Session::new(sim::Sim::new());
    // optional run at startup: --seed --difficulty --mode
    if a.value("seed").is_some() || a.flag("start") {
        let mut c = serde_json::json!({ "cmd": "start_run" });
        if let Some(v) = a.value("seed") {
            c["seed"] = Value::from(v);
        }
        if let Some(v) = a.value("difficulty") {
            c["difficulty"] = Value::from(v);
        }
        if let Some(v) = a.value("mode") {
            c["mode"] = Value::from(v);
        }
        let r = s.handle_safe(&c);
        if r["ok"] != true {
            eprintln!("start_run failed: {r}");
            return 2;
        }
    }
    eprintln!("LAST ROUTE headless agent protocol {} ready (one JSON object per line; {{\"cmd\":\"help\"}})", protocol::PROTOCOL);
    while let Ok(msg) = rx.recv() {
        match msg {
            None => {
                if !tcp {
                    break;
                }
            }
            Some((line, to)) => {
                if let Some(r) = s.handle_line(&line) {
                    respond(to, &r);
                }
                if let Some(code) = s.quit {
                    return code;
                }
            }
        }
    }
    s.quit.unwrap_or(0)
}
