//! The game's network client, ported from the Godot client's `game/autoload/backend_client.gd`
//! (accounts, guest names, the ENet room directory, reserved rooms, admin room proofs),
//! `game/autoload/run_saves.gd` (the cloud copy of the one-run autosave), the room-code rules of
//! `game/autoload/room_state.gd`, and the gameplay relay tunnel `game/network/enet_relay_tunnel.gd`.
//!
//! Server contract: `backend/server.mjs`, `account_service.mjs`, `guest_names.mjs`,
//! `game_saves.mjs`, `reserved_rooms.mjs`, `enet_relay.mjs` of the Godot project.
//!
//! Layout:
//! * [`config`]   origin policy (primary origin, loopback dev exception, TEST instance), timeouts.
//! * [`http`]     an HTTP/1.1 client (TcpStream, timeouts, Content-Length / chunked, https over rustls, checked redirects; see docs/NET_HTTPS.md).
//! * [`pending`]  [`Pending<T>`]: requests on worker threads, polled from the frame loop.
//! * [`api`]      one blocking function per server route, typed request/response shapes.
//! * [`backend`]  [`Backend`]: the stateful `Backend` autoload (session, heartbeat, events).
//! * [`store`]    the session / guest-name files in Godot's ConfigFile format.
//! * [`saves`]    run_saves.gd's cloud sync (owed-marker, newest-wins, 409 STALE).
//! * [`relay`]    the LRR1 relay wrapper: host register / HOST_ALIVE keep-alive / guest join.
//! * [`protocol`] typed versions of every message the Godot co-op code syncs, and their encoding.
//! * [`session`]  a Rust-to-Rust co-op transport (reliable + unreliable channels) over the relay.
//!
//! INTEROP NOTE: Godot clients exchange ENet datagrams carrying Godot SceneMultiplayer RPC
//! packets (Variant-encoded arguments, node-path caches, RPC ids by sorted method name). Talking
//! to a Godot client would need a full ENet implementation plus SceneMultiplayer's wire format;
//! that is not done here. Instead [`session`] runs its own small datagram protocol (documented
//! there) through the SAME relay and the SAME directory, carrying the messages of [`protocol`].
//! A Rust host advertises its own protocol fingerprint ([`protocol::fingerprint`]), so a Godot
//! client resolving its room is refused with the original's "BUILD MISMATCH" message before it
//! connects, and vice versa.

// The menus use a subset of this API; the rest (admin, reserved-room owner tools, co-op) stays available.
#![allow(dead_code)]

pub mod api;
pub mod backend;
/// Co-op chat lines (team / nearby, spoken aloud in the original) and their host-side moderation.
pub mod chat;
pub mod hide_sync;
pub mod config;
pub mod http;
pub mod pending;
pub mod hunter_events;
pub mod protocol;
pub mod public_route;
pub mod relay;
pub mod runner_figure;
pub mod saves;
pub mod session;
pub mod store;
pub mod valve_events;
pub mod wiring;
pub mod world_sync;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use backend::{Backend, BackendEvent};
pub use config::BackendConfig;
pub use pending::Pending;

use std::fmt;

/// Every way a network call can fail.
#[derive(Clone, Debug, PartialEq)]
pub enum NetError {
    /// Offline mode (`--offline`, Create Room -> Offline) or requests switched off.
    Offline,
    /// Refused locally before anything was sent (origin policy, not signed in, a busy slot...).
    /// The text is the original's player-facing message.
    Refused(String),
    /// Could not connect / the connection broke.
    Transport(String),
    /// No answer in time.
    Timeout,
    /// Something this client cannot do (an address that is not http or https, a malformed address).
    Unsupported(String),
    /// Not a usable HTTP/JSON answer.
    Protocol(String),
    /// The server answered with a non-2xx status: its `error` text and machine `code` when it sent
    /// them (`{ok:false, code, error}` is the server's error shape).
    Http { status: u16, code: String, error: String, body: serde_json::Value },
}

impl NetError {
    /// The HTTP status, if the server answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            NetError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// True when the server was reached (any HTTP status).
    pub fn reached(&self) -> bool {
        matches!(self, NetError::Http { .. })
    }

    /// The text a menu shows: the server's own `error` string when it sent one, else `fallback`
    /// (the original's `String(payload.get("error", fallback))`), scrubbed of addresses.
    pub fn message_or(&self, fallback: &str) -> String {
        let text = match self {
            NetError::Http { error, .. } if !error.is_empty() => error.clone(),
            NetError::Refused(m) => m.clone(),
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
            NetError::Refused(m) => write!(f, "{m}"),
            NetError::Transport(m) => write!(f, "TRANSPORT FAILED: {m}"),
            NetError::Timeout => write!(f, "TIMED OUT"),
            NetError::Unsupported(m) => write!(f, "{m}"),
            NetError::Protocol(m) => write!(f, "BAD RESPONSE: {m}"),
            NetError::Http { status, error, .. } => write!(f, "HTTP {status}: {error}"),
        }
    }
}

impl std::error::Error for NetError {}

/// `ServerPrivacy` (game/config/server_privacy.gd): players never see where the server is. A
/// message that names a URL, IP address, host name or port becomes "CAN'T REACH THE SERVER" when
/// it reports a failure, else the address is replaced with "THE SERVER". (The original uses
/// regexes; this is a hand-written scan with the same targets.)
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

/// Random bytes from the OS-seeded SipHash keys of `RandomState` plus a counter and the clock.
/// Not a CSPRNG (std has none); used where the original uses `Crypto.generate_random_bytes` for a
/// relay join nonce and `randi()` for room codes. PORT NOTE: if a crypto-grade source is wanted,
/// add `getrandom = "0.2"` and swap this function.
pub fn random_bytes(n: usize) -> Vec<u8> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = Vec::with_capacity(n + 8);
    while out.len() < n {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        h.write_u128(t);
        out.extend_from_slice(&h.finish().to_le_bytes());
    }
    out.truncate(n);
    out
}

/// A uniformly random index below `n`.
pub fn random_below(n: usize) -> usize {
    let b = random_bytes(8);
    (u64::from_le_bytes(b.try_into().unwrap()) % n as u64) as usize
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decodes standard or url-safe base64, padded or not (`Marshalls.base64_to_raw` after the
/// tunnel's `-`/`_` replacement and re-padding). `None` on any invalid character.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Seconds since the Unix epoch (`Time.get_unix_time_from_system()`).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for n in 0..40 {
            let b: Vec<u8> = (0..n as u8).map(|x| x.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64_encode(&b)).unwrap(), b);
        }
        // a base64url ticket like reserved_rooms.mjs issues (18 bytes, no padding)
        assert_eq!(base64_decode("-_-_").unwrap(), vec![0xfb, 0xff, 0xbf]);
        assert!(base64_decode("a b").is_none());
    }

    #[test]
    fn random_is_varied() {
        let a = random_bytes(12);
        let b = random_bytes(12);
        assert_eq!(a.len(), 12);
        assert_ne!(a, b);
        assert!(random_below(32) < 32);
    }

    #[test]
    fn scrub_hides_addresses() {
        assert_eq!(scrub("SIGNED IN AS KYTTU"), "SIGNED IN AS KYTTU");
        assert_eq!(scrub("COULD NOT CONNECT TO http://lastroute.kyttuslab.org:8105"), "CAN'T REACH THE SERVER");
        assert_eq!(scrub("RELAY lastroute.kyttuslab.org READY"), "RELAY THE SERVER READY");
        assert_eq!(scrub("VERSION 0.0.1.0"), "VERSION 0.0.1.0");
        assert_eq!(scrub("HOST 10.0.0.2:1105 GONE"), "HOST THE SERVER GONE");
    }

    #[test]
    fn error_messages() {
        let e = NetError::Http { status: 401, code: "INVALID_CREDENTIALS".into(), error: "Incorrect username or password".into(), body: serde_json::Value::Null };
        assert_eq!(e.message_or("ACCOUNT LOGIN FAILED (HTTP 401)"), "Incorrect username or password");
        assert_eq!(NetError::Timeout.message_or("ACCOUNT LOGIN FAILED (HTTP 0)"), "ACCOUNT LOGIN FAILED (HTTP 0)");
    }
}
