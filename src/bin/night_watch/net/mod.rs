//! Night Watch's network client: the ACCOUNT screen talks to the same `lr_server` account API as Last Route
//! (`/api/account/login|register|session|logout`). Saves stay local; only the account name and session token are
//! kept with them. Nothing here touches macroquad, so tests and `--check` run without a window.
//!
//! * `config`  Last Route's origin policy, included unchanged with `#[path]` (read-only).
//! * `http`    a small blocking HTTP/1.1 + rustls client (native). The browser build has no sockets and no page
//!             bridge for this executable, so accounts there stay guest only with a clear message.
//! * `account` request bodies, reply parsing, the process-wide signed-in account and the pollable `AccountClient`.
#![allow(dead_code)]

pub mod account;
#[path = "../../last_route/net/config.rs"]
pub mod config;
pub mod http;

use std::fmt;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub type Json = serde_json::Map<String, serde_json::Value>;

/// The browser build has no page bridge for Night Watch: `config` only asks for an API origin here.
#[cfg(target_arch = "wasm32")]
pub(crate) mod web_bridge {
    pub fn api_base() -> String {
        String::new()
    }
}

/// Every way an account call can fail (the subset of Last Route's `NetError` this client can produce).
#[derive(Clone, Debug, PartialEq)]
pub enum NetError {
    Offline,
    /// Refused locally before anything was sent. The text is player facing.
    Refused(String),
    Transport(String),
    Timeout,
    Unsupported(String),
    Protocol(String),
    /// The server answered with a non-2xx status: its `error` text and `code` when it sent them.
    Http { status: u16, code: String, error: String },
}

impl NetError {
    pub fn status(&self) -> Option<u16> {
        match self {
            NetError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The text a screen shows: the server's own `error` string when it sent one, else `fallback`, scrubbed of addresses.
    pub fn message_or(&self, fallback: &str) -> String {
        let text = match self {
            NetError::Http { error, .. } if !error.is_empty() => error.clone(),
            NetError::Refused(m) => m.clone(),
            NetError::Unsupported(m) => m.clone(),
            NetError::Offline => config::OFFLINE_NOTE.to_string(),
            _ => fallback.to_string(),
        };
        scrub(&text)
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Offline => write!(f, "OFFLINE MODE"),
            NetError::Refused(m) | NetError::Unsupported(m) => write!(f, "{m}"),
            NetError::Transport(m) => write!(f, "TRANSPORT FAILED: {m}"),
            NetError::Timeout => write!(f, "TIMED OUT"),
            NetError::Protocol(m) => write!(f, "BAD RESPONSE: {m}"),
            NetError::Http { status, error, .. } => write!(f, "HTTP {status}: {error}"),
        }
    }
}

impl std::error::Error for NetError {}

/// Players never see where the server is (Last Route's `ServerPrivacy`): a message naming a URL, IP, host or port
/// becomes "CAN'T REACH THE SERVER" when it reports a failure, else the address is replaced with "THE SERVER".
pub fn scrub(text: &str) -> String {
    const FRIENDLY_UNREACHABLE: &str = "CAN'T REACH THE SERVER";
    const FAILURE_WORDS: [&str; 9] = ["CONNECT", "UNREACH", "TIMEOUT", "TIMED OUT", "REFUSED", "RESOLVE", "UNAVAILABLE", "FAILED", "LOST"];
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let leaky = |t: &str| -> bool {
        let l = t.to_ascii_lowercase();
        let l = l.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';' | '"' | '\''));
        if l.contains("://") || l.starts_with("localhost") {
            return true;
        }
        let core = l.trim_end_matches('.');
        // IPv4 (not a version number like 0.0.1.0 after "VERSION": the original's lookbehind)
        let parts: Vec<&str> = core.split(':').next().unwrap_or("").split('.').collect();
        if parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit())) {
            return true;
        }
        for tld in [".org", ".com", ".net", ".io", ".dev", ".app", ".local", ".lan", ".cloud"] {
            let host = core.split(':').next().unwrap_or("");
            if host.ends_with(tld) && host.len() > tld.len() {
                return true;
            }
        }
        // a bare :port
        if let Some(p) = core.strip_prefix(':') {
            return (2..=5).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit());
        }
        false
    };
    let mut any = false;
    let mut out = Vec::with_capacity(tokens.len());
    for (i, t) in tokens.iter().enumerate() {
        let after_version = i > 0 && tokens[i - 1].eq_ignore_ascii_case("version");
        if !after_version && leaky(t) {
            any = true;
            out.push("THE SERVER".to_string());
        } else {
            out.push(t.to_string());
        }
    }
    if !any {
        return text.to_string();
    }
    let upper = text.to_uppercase();
    if FAILURE_WORDS.iter().any(|w| upper.contains(w)) {
        return FRIENDLY_UNREACHABLE.to_string();
    }
    out.join(" ")
}

/// One request running on a worker thread, polled once per frame (`poll` never blocks). The browser build has one
/// thread, so the work runs at once and the result arrives on the next poll.
pub struct Pending<T> {
    rx: Option<Receiver<Result<T, NetError>>>,
    ready: Option<Result<T, NetError>>,
}

impl<T: Send + 'static> Pending<T> {
    pub fn spawn<F>(work: F) -> Pending<T>
    where
        F: FnOnce() -> Result<T, NetError> + Send + 'static,
    {
        #[cfg(target_arch = "wasm32")]
        {
            Pending::done(work())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("nw-net".into()).spawn(move || {
                let _ = tx.send(work());
            });
            match spawned {
                Ok(_) => Pending { rx: Some(rx), ready: None },
                Err(e) => Pending::done(Err(NetError::Transport(format!("COULD NOT START REQUEST ({e})")))),
            }
        }
    }
}

impl<T> Pending<T> {
    pub fn done(result: Result<T, NetError>) -> Pending<T> {
        Pending { rx: None, ready: Some(result) }
    }

    /// `Some(result)` exactly once, when the work has finished.
    pub fn poll(&mut self) -> Option<Result<T, NetError>> {
        if let Some(r) = self.ready.take() {
            return Some(r);
        }
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(r) => {
                self.rx = None;
                Some(r)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                Some(Err(NetError::Transport("REQUEST THREAD ENDED".into())))
            }
        }
    }

    /// Blocks until the result is in (tests).
    pub fn wait(mut self) -> Result<T, NetError> {
        if let Some(r) = self.ready.take() {
            return r;
        }
        match self.rx.take() {
            Some(rx) => rx.recv().unwrap_or_else(|_| Err(NetError::Transport("REQUEST THREAD ENDED".into()))),
            None => Err(NetError::Protocol("RESULT ALREADY TAKEN".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_hides_addresses() {
        assert_eq!(scrub("SIGNED IN AS KYTTU"), "SIGNED IN AS KYTTU");
        assert_eq!(scrub("COULD NOT CONNECT TO http://lastroute.kyttuslab.org:8105"), "CAN'T REACH THE SERVER");
        assert_eq!(scrub("HOST 10.0.0.2:1105 GONE"), "HOST THE SERVER GONE");
    }

    #[test]
    fn error_messages() {
        let e = NetError::Http { status: 401, code: "INVALID_CREDENTIALS".into(), error: "Incorrect username or password".into() };
        assert_eq!(e.message_or("ACCOUNT LOGIN FAILED (HTTP 401)"), "Incorrect username or password");
        assert_eq!(NetError::Timeout.message_or("ACCOUNT LOGIN FAILED (HTTP 0)"), "ACCOUNT LOGIN FAILED (HTTP 0)");
    }

    #[test]
    fn pending_delivers_once() {
        let mut p = Pending::spawn(|| Ok(7));
        let mut got = None;
        for _ in 0..400 {
            if let Some(r) = p.poll() {
                got = Some(r);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.unwrap().unwrap(), 7);
        assert!(p.poll().is_none());
        let d: Pending<i32> = Pending::done(Err(NetError::Offline));
        assert_eq!(d.wait(), Err(NetError::Offline));
    }
}
