//! The native build's WebSocket room transport: the same relay the browser build joins (`/api/ws` on the backend;
//! `backend/ws_relay.mjs` and `game/network/ws_room_peer.gd` of the Godot project are the reference), over a plain
//! `std::net::TcpStream`. It implements [`RoomLink`], so [`super::session::CoopSession`] runs over it unchanged, as host
//! (the dedicated room process, `headless/dedicated.rs`) and as guest (a native client joining a hosted room).
//!
//! The relay protocol is the one [`super::ws_link`] documents and implements for the browser: a JSON `hello` first
//! (`role` host with the host token, or guest with an optional ticket), `welcome` / `peer` / `host` / `error` control
//! messages, and binary frames `[int32 LE peer][mode][channel][payload]`. [`super::ws_link::WsCore`] is the state
//! machine; this file only moves its bytes over a socket.
//!
//! What is here, no new dependency:
//! * RFC 6455 client side: the HTTP upgrade (random `Sec-WebSocket-Key`, the `Sec-WebSocket-Accept` check with a small
//!   SHA-1), masked client frames, an incremental frame parser (fragments, control frames between fragments, size cap),
//!   ping answered with pong, close frames in both directions with their code and reason.
//! * [`WsConn`]: a non-blocking connection after the handshake. `pump` reads what arrived and flushes what is queued.
//! * [`NativeWsLink`]: the [`RoomLink`] over a [`WsConn`] and a [`WsCore`], with the browser link's keepalive (a
//!   `{"t":"ping"}` text frame every 20 s) and its unreliable-drop rule (no unreliable frame is queued behind 256 KB).
//!
//! `wss://` (the website's relay) runs the TLS handshake inside [`WsConn::connect`] with the rustls client settings of
//! `net/http.rs` (Mozilla roots, TLS 1.2 and 1.3, the server name always checked); the link and everything above it are
//! the same over both. The connect and the handshake block (up to [`CONNECT_TIMEOUT`]), so a caller builds the link on a
//! worker thread, the way `Pending::spawn` builds a session. Name resolution is the blocking system lookup and has no
//! timeout of its own.

use super::relay::{RelayEvent, Role};
use super::room_link::RoomLink;
use super::session::HOST_PEER_ID;
use super::web_bridge::WsItem;
use super::ws_link::{
    encode_frame, hello_text, kick_text, transfer_mode, WsCore, FRAME_HEADER_BYTES, KEEPALIVE, MAX_FRAME_PAYLOAD, MODE_UNRELIABLE,
    RELAY_PATH, UNRELIABLE_BUFFER_LIMIT,
};
use crate::shared::clock::Instant;
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// The longest the TCP connect plus the HTTP upgrade may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// A link that has not been welcomed this long after its hello is closed (ws_room_peer.gd HELLO_TIMEOUT_MSEC).
pub const WELCOME_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest message (all fragments together) the parser accepts: the relay's own cap (ws_relay.mjs MAX_MESSAGE_BYTES).
pub const MAX_MESSAGE_BYTES: usize = MAX_FRAME_PAYLOAD + FRAME_HEADER_BYTES + 64;
/// More than this queued unsent means the peer is not reading: the link is closed instead of growing without bound.
pub const MAX_QUEUED_BYTES: usize = 4 * 1024 * 1024;
/// The upgrade response may not be longer than this.
const MAX_HANDSHAKE_BYTES: usize = 8 * 1024;
/// How much one `pump` reads at most, so one chatty peer cannot hold the frame loop.
const MAX_READ_PER_PUMP: usize = 1024 * 1024;
/// The RFC 6455 key suffix.
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// ---- SHA-1 and the accept key -------------------------------------------------------------------------------------

/// SHA-1 of `data` (RFC 3174). Used only for the `Sec-WebSocket-Accept` check, never for security.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// The `Sec-WebSocket-Accept` value the server must answer to `key` (RFC 6455 section 4.2.2).
pub fn accept_key(key: &str) -> String {
    super::base64_encode(&sha1(format!("{key}{GUID}").as_bytes()))
}

// ---- the address ---------------------------------------------------------------------------------------------------

/// A parsed `ws://host[:port]/path` address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsUrl {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, always starting with `/`.
    pub path: String,
}

/// Reads a `ws://` or `wss://` address (an `http://` or `https://` one is read as its WebSocket twin). The port
/// defaults to 80 / 443. An IPv6 host is written `[::1]`.
pub fn parse_ws_url(url: &str) -> Result<WsUrl, String> {
    let url = url.trim();
    let (secure, rest) = if let Some(r) = url.strip_prefix("ws://").or_else(|| url.strip_prefix("http://")) {
        (false, r)
    } else if let Some(r) = url.strip_prefix("wss://").or_else(|| url.strip_prefix("https://")) {
        (true, r)
    } else {
        return Err("NOT A WEBSOCKET ADDRESS".into());
    };
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return Err("BAD WEBSOCKET ADDRESS".into());
    }
    let default_port = if secure { 443 } else { 80 };
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, tail) = v6.split_once(']').ok_or("BAD WEBSOCKET ADDRESS")?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| "BAD WEBSOCKET PORT")?,
            None if tail.is_empty() => default_port,
            None => return Err("BAD WEBSOCKET ADDRESS".into()),
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse::<u16>().map_err(|_| "BAD WEBSOCKET PORT")?),
            None => (authority.to_string(), default_port),
        }
    };
    if host.is_empty() || port == 0 {
        return Err("BAD WEBSOCKET ADDRESS".into());
    }
    Ok(WsUrl { secure, host, port, path })
}

/// The relay address for what a caller was given: an origin (`http://host:8105`, `https://host`), a WebSocket origin
/// (`ws://host:8105`), or a full relay address (one that already ends in `/api/ws`).
pub fn relay_address(input: &str) -> String {
    let base = input.trim().trim_end_matches('/');
    if base.ends_with(RELAY_PATH) {
        return ws_scheme(base);
    }
    super::ws_link::relay_url(base)
}

/// `http(s)://x` -> `ws(s)://x`; anything else unchanged.
fn ws_scheme(address: &str) -> String {
    if let Some(r) = address.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = address.strip_prefix("http://") {
        format!("ws://{r}")
    } else {
        address.to_string()
    }
}

/// The HTTP upgrade request for `url` with the client key `key`.
pub fn handshake_request(url: &WsUrl, key: &str) -> String {
    let default_port = if url.secure { 443 } else { 80 };
    let host = if url.host.contains(':') { format!("[{}]", url.host) } else { url.host.clone() };
    let authority = if url.port == default_port { host } else { format!("{host}:{}", url.port) };
    format!(
        "GET {} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        url.path
    )
}

/// Checks the server's answer to the upgrade: status 101, `Upgrade: websocket` and the accept value for `key`.
/// `head` is the response up to (and not including) the blank line.
pub fn check_handshake_response(head: &str, key: &str) -> Result<(), String> {
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let code = status.split_whitespace().nth(1).unwrap_or_default();
    if code != "101" {
        return Err(format!("THE SERVER DID NOT UPGRADE THE CONNECTION ({})", if code.is_empty() { "NO STATUS" } else { code }));
    }
    let (mut accept, mut upgrade) = (None, false);
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            if name == "sec-websocket-accept" {
                accept = Some(value.to_string());
            } else if name == "upgrade" && value.eq_ignore_ascii_case("websocket") {
                upgrade = true;
            }
        }
    }
    if !upgrade {
        return Err("THE SERVER ANSWERED WITHOUT AN UPGRADE".into());
    }
    if accept.as_deref() != Some(accept_key(key).as_str()) {
        return Err("THE SERVER'S WEBSOCKET KEY DID NOT MATCH".into());
    }
    Ok(())
}

// ---- frames ----------------------------------------------------------------------------------------------------------

pub mod opcode {
    pub const CONT: u8 = 0;
    pub const TEXT: u8 = 1;
    pub const BINARY: u8 = 2;
    pub const CLOSE: u8 = 8;
    pub const PING: u8 = 9;
    pub const PONG: u8 = 10;
}

/// One WebSocket frame, FIN set (this client never fragments). `mask` is the 4-byte key of a client frame (every
/// client frame must be masked, RFC 6455 section 5.3), `None` for a server frame.
pub fn encode_ws_frame(op: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | (op & 0x0f));
    let flag = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(flag | n as u8),
        n if n <= 0xffff => {
            out.push(flag | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(flag | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i & 3]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// The payload of a close frame: the status code, then the reason (cut to 100 bytes, as the relay cuts its own).
pub fn close_payload(code: u16, reason: &str) -> Vec<u8> {
    let mut out = code.to_be_bytes().to_vec();
    let mut reason = reason;
    while reason.len() > 100 {
        let mut cut = 100;
        while !reason.is_char_boundary(cut) {
            cut -= 1;
        }
        reason = &reason[..cut];
    }
    out.extend_from_slice(reason.as_bytes());
    out
}

/// What the parser hands out: whole messages (fragments joined) and control frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsMessage {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// The status code (1005 when the frame carried none) and reason.
    Close { code: u16, reason: String },
}

/// A frame the peer must not have sent. The connection answers it with close code 1002 (1009 for a message over the cap).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameError {
    pub code: u16,
    pub why: &'static str,
}

/// An incremental RFC 6455 frame reader. Feed it bytes with [`FrameParser::push`], take messages with
/// [`FrameParser::next_message`] until it returns `Ok(None)`.
pub struct FrameParser {
    buf: Vec<u8>,
    start: usize,
    /// The data message being assembled: its opcode and the bytes so far.
    partial: Option<(u8, Vec<u8>)>,
    /// A server requires masked client frames (RFC 6455); a client accepts either (a masked server frame is out of
    /// spec but harmless).
    require_mask: bool,
    max_message: usize,
}

impl FrameParser {
    pub fn new(require_mask: bool, max_message: usize) -> FrameParser {
        FrameParser { buf: Vec::new(), start: 0, partial: None, require_mask, max_message }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > 64 * 1024 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Bytes held that do not make a whole frame yet.
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }

    pub fn next_message(&mut self) -> Result<Option<WsMessage>, FrameError> {
        loop {
            let data = &self.buf[self.start..];
            if data.len() < 2 {
                return Ok(None);
            }
            let (first, second) = (data[0], data[1]);
            if first & 0x70 != 0 {
                return Err(FrameError { code: 1002, why: "reserved bits set" });
            }
            let fin = first & 0x80 != 0;
            let op = first & 0x0f;
            let masked = second & 0x80 != 0;
            let mut len = (second & 0x7f) as u64;
            let mut offset = 2usize;
            if len == 126 {
                if data.len() < 4 {
                    return Ok(None);
                }
                len = u16::from_be_bytes([data[2], data[3]]) as u64;
                offset = 4;
            } else if len == 127 {
                if data.len() < 10 {
                    return Ok(None);
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&data[2..10]);
                len = u64::from_be_bytes(b);
                offset = 10;
            }
            if self.require_mask && !masked {
                return Err(FrameError { code: 1002, why: "client frames must be masked" });
            }
            if len > self.max_message as u64 {
                return Err(FrameError { code: 1009, why: "frame too large" });
            }
            let len = len as usize;
            let key_len = if masked { 4 } else { 0 };
            if data.len() < offset + key_len + len {
                return Ok(None);
            }
            let mut payload = data[offset + key_len..offset + key_len + len].to_vec();
            if masked {
                let key = [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]];
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= key[i & 3];
                }
            }
            self.start += offset + key_len + len;
            if op >= 8 {
                if !fin || len > 125 {
                    return Err(FrameError { code: 1002, why: "bad control frame" });
                }
                return match op {
                    opcode::CLOSE => match payload.len() {
                        0 => Ok(Some(WsMessage::Close { code: 1005, reason: String::new() })),
                        1 => Err(FrameError { code: 1002, why: "bad close frame" }),
                        _ => Ok(Some(WsMessage::Close {
                            code: u16::from_be_bytes([payload[0], payload[1]]),
                            reason: String::from_utf8_lossy(&payload[2..]).into_owned(),
                        })),
                    },
                    opcode::PING => Ok(Some(WsMessage::Ping(payload))),
                    opcode::PONG => Ok(Some(WsMessage::Pong(payload))),
                    _ => Err(FrameError { code: 1002, why: "unknown control opcode" }),
                };
            }
            let (kind, mut bytes) = match op {
                opcode::TEXT | opcode::BINARY => {
                    if self.partial.is_some() {
                        return Err(FrameError { code: 1002, why: "new message inside a fragmented one" });
                    }
                    (op, Vec::new())
                }
                opcode::CONT => match self.partial.take() {
                    Some(p) => p,
                    None => return Err(FrameError { code: 1002, why: "unexpected continuation" }),
                },
                _ => return Err(FrameError { code: 1002, why: "unknown opcode" }),
            };
            if bytes.len() + payload.len() > self.max_message {
                return Err(FrameError { code: 1009, why: "message too large" });
            }
            bytes.extend_from_slice(&payload);
            if fin {
                return Ok(Some(if kind == opcode::TEXT { WsMessage::Text(bytes) } else { WsMessage::Binary(bytes) }));
            }
            self.partial = Some((kind, bytes));
        }
    }
}

// ---- the connection --------------------------------------------------------------------------------------------------

/// The socket under a connection: plain TCP for `ws://`, TLS over TCP for `wss://`.
enum WsStream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl WsStream {
    fn tcp(&self) -> &TcpStream {
        match self {
            WsStream::Plain(s) => s,
            WsStream::Tls(s) => &s.sock,
        }
    }
}

impl Read for WsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            WsStream::Plain(s) => s.read(buf),
            // a server that drops the socket without a TLS close_notify is a closed peer, as in net/http.rs
            WsStream::Tls(s) => match s.read(buf) {
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => Ok(0),
                other => other,
            },
        }
    }
}

impl Write for WsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            WsStream::Plain(s) => s.write(buf),
            // rustls takes plaintext into its buffer and reports 0 only when that buffer is full: that is "not now",
            // not a closed socket, so it is reported as WouldBlock (the caller keeps its bytes and retries)
            WsStream::Tls(s) => match s.write(buf) {
                Ok(0) if !buf.is_empty() => Err(ErrorKind::WouldBlock.into()),
                other => other,
            },
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            WsStream::Plain(s) => s.flush(),
            WsStream::Tls(s) => s.flush(),
        }
    }
}

/// The TLS handshake over `tcp` for `host`, within `deadline`. Blocking (the socket has its timeouts set here).
fn open_tls(url: &WsUrl, tcp: TcpStream, deadline: Instant, tls: &Arc<rustls::ClientConfig>) -> Result<Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>, String> {
    let name = rustls::pki_types::ServerName::try_from(url.host.clone()).map_err(|_| "BAD WEBSOCKET ADDRESS".to_string())?;
    let connection = rustls::ClientConnection::new(tls.clone(), name).map_err(|_| "SECURE CONNECTION FAILED".to_string())?;
    let mut stream = rustls::StreamOwned::new(connection, tcp);
    while stream.conn.is_handshaking() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("TIMED OUT".into());
        }
        let _ = stream.sock.set_read_timeout(Some(left));
        let _ = stream.sock.set_write_timeout(Some(left));
        if let Err(e) = stream.conn.complete_io(&mut stream.sock) {
            return Err(match e.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => "TIMED OUT".to_string(),
                // a certificate the roots do not vouch for, a name that does not match, a server that does not speak TLS
                _ => "SECURE CONNECTION FAILED".to_string(),
            });
        }
    }
    Ok(Box::new(stream))
}

/// The player-facing reason a TCP connect failed.
fn reach_message(kind: ErrorKind) -> String {
    let why = match kind {
        ErrorKind::ConnectionRefused => "NOTHING IS LISTENING THERE",
        ErrorKind::TimedOut | ErrorKind::WouldBlock => "TIMED OUT",
        _ => "NETWORK ERROR",
    };
    format!("COULD NOT REACH THE SERVER ({why})")
}

/// A WebSocket client connection after its handshake: non-blocking, driven by [`WsConn::pump`].
pub struct WsConn {
    stream: WsStream,
    parser: FrameParser,
    /// Frames queued and not yet written, from `out_pos` on.
    out: Vec<u8>,
    out_pos: usize,
    /// The close was reported (or we closed): nothing more is read or written.
    closed: bool,
    close_sent: bool,
}

/// The share of the remaining connect time one address may use, so a blackholed first address cannot starve the rest.
fn address_slice(left: Duration, addresses_left: usize) -> Duration {
    let share = left / addresses_left.max(1) as u32;
    share.max(Duration::from_millis(250).min(left))
}

impl WsConn {
    /// Opens `url`: `ws://` as plain TCP, `wss://` over TLS (the TCP connect, the TLS handshake and the HTTP upgrade, all
    /// within `timeout`). Blocking.
    pub fn connect(url: &str, timeout: Duration) -> Result<WsConn, String> {
        WsConn::connect_with(url, timeout, &super::http::tls_config())
    }

    /// [`WsConn::connect`] with the TLS client settings given (the tests trust their own certificate this way).
    fn connect_with(url: &str, timeout: Duration, tls: &Arc<rustls::ClientConfig>) -> Result<WsConn, String> {
        let url = parse_ws_url(url)?;
        let deadline = Instant::now() + timeout;
        let addrs: Vec<_> = (url.host.as_str(), url.port).to_socket_addrs().map_err(|_| "COULD NOT FIND THE SERVER".to_string())?.collect();
        let mut tcp = None;
        let mut last = "COULD NOT REACH THE SERVER".to_string();
        let total = addrs.len();
        for (tried, addr) in addrs.into_iter().enumerate() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("TIMED OUT".into());
            }
            match TcpStream::connect_timeout(&addr, address_slice(left, total - tried)) {
                Ok(s) => {
                    tcp = Some(s);
                    break;
                }
                Err(e) => last = reach_message(e.kind()),
            }
        }
        let tcp = tcp.ok_or(last)?;
        let _ = tcp.set_nodelay(true);
        let mut stream = if url.secure { WsStream::Tls(open_tls(&url, tcp, deadline, tls)?) } else { WsStream::Plain(tcp) };
        let key = super::base64_encode(&super::random_bytes(16));
        let io = |e: std::io::Error| match e.kind() {
            ErrorKind::TimedOut | ErrorKind::WouldBlock => "TIMED OUT".to_string(),
            _ => "THE CONNECTION BROKE DURING THE HANDSHAKE".to_string(),
        };
        stream.tcp().set_write_timeout(Some(deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)))).map_err(io)?;
        stream.write_all(handshake_request(&url, &key).as_bytes()).map_err(io)?;
        stream.flush().map_err(io)?;
        // the response head, then whatever the server already sent behind it
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        let split = loop {
            if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            if head.len() > MAX_HANDSHAKE_BYTES {
                return Err("THE SERVER'S ANSWER WAS TOO LONG".into());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("TIMED OUT".into());
            }
            stream.tcp().set_read_timeout(Some(left)).map_err(io)?;
            match stream.read(&mut chunk) {
                Ok(0) => return Err("THE SERVER CLOSED THE CONNECTION".into()),
                Ok(n) => head.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(io(e)),
            }
        };
        check_handshake_response(&String::from_utf8_lossy(&head[..split]), &key)?;
        let mut parser = FrameParser::new(false, MAX_MESSAGE_BYTES);
        parser.push(&head[split + 4..]);
        stream.tcp().set_read_timeout(None).map_err(io)?;
        stream.tcp().set_write_timeout(None).map_err(io)?;
        stream.tcp().set_nonblocking(true).map_err(io)?;
        Ok(WsConn { stream, parser, out: Vec::new(), out_pos: 0, closed: false, close_sent: false })
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Bytes queued and not written yet (a browser socket's `bufferedAmount`).
    pub fn buffered(&self) -> usize {
        self.out.len() - self.out_pos
    }

    fn mask() -> [u8; 4] {
        let b = super::random_bytes(4);
        [b[0], b[1], b[2], b[3]]
    }

    fn queue(&mut self, op: u8, payload: &[u8]) -> bool {
        if self.closed || (self.close_sent && op != opcode::CLOSE) || self.buffered() > MAX_QUEUED_BYTES {
            return false;
        }
        if self.out_pos > 0 && self.out_pos == self.out.len() {
            self.out.clear();
            self.out_pos = 0;
        }
        self.out.extend_from_slice(&encode_ws_frame(op, payload, Some(WsConn::mask())));
        self.flush();
        true
    }

    /// A text message. False when the connection is closed or its queue is full.
    pub fn send_text(&mut self, text: &str) -> bool {
        self.queue(opcode::TEXT, text.as_bytes())
    }

    /// A binary message.
    pub fn send_binary(&mut self, bytes: &[u8]) -> bool {
        self.queue(opcode::BINARY, bytes)
    }

    /// Writes as much of the queue as the socket takes now.
    fn flush(&mut self) {
        while self.out_pos < self.out.len() && !self.closed {
            match self.stream.write(&self.out[self.out_pos..]) {
                Ok(0) => self.closed = true,
                Ok(n) => self.out_pos += n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => self.closed = true,
            }
        }
        if self.out_pos == self.out.len() {
            self.out.clear();
            self.out_pos = 0;
        }
        // TLS keeps records of its own until they reach the socket; a blocked socket here is not an error
        if !self.closed {
            let _ = self.stream.flush();
        }
    }

    /// Reads what arrived, answers pings, flushes the queue and returns the messages. After a close (the peer's, ours
    /// answered, a protocol error or a broken socket) it returns one `Closed` item, then nothing.
    pub fn pump(&mut self) -> Vec<WsItem> {
        let mut items = Vec::new();
        if self.closed {
            return items;
        }
        let mut eof = false;
        let mut read = 0usize;
        let mut chunk = [0u8; 16 * 1024];
        while read < MAX_READ_PER_PUMP {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => {
                    read += n;
                    self.parser.push(&chunk[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    eof = true;
                    break;
                }
            }
        }
        loop {
            match self.parser.next_message() {
                Ok(Some(WsMessage::Text(bytes))) => items.push(WsItem::Text(bytes)),
                Ok(Some(WsMessage::Binary(bytes))) => items.push(WsItem::Binary(bytes)),
                Ok(Some(WsMessage::Ping(payload))) => {
                    self.queue(opcode::PONG, &payload);
                }
                Ok(Some(WsMessage::Pong(_))) => {}
                Ok(Some(WsMessage::Close { code, reason })) => {
                    // answer a close with a close (RFC 6455 section 5.5.1), then the connection is over
                    if !self.close_sent {
                        self.queue(opcode::CLOSE, &close_payload(if code == 1005 { 1000 } else { code }, ""));
                        self.close_sent = true;
                    }
                    self.finish();
                    items.push(WsItem::Closed { code, reason });
                    return items;
                }
                Ok(None) => break,
                Err(e) => {
                    self.queue(opcode::CLOSE, &close_payload(e.code, e.why));
                    self.close_sent = true;
                    self.finish();
                    items.push(WsItem::Closed { code: e.code, reason: e.why.to_string() });
                    return items;
                }
            }
        }
        if eof {
            self.closed = true;
            items.push(WsItem::Closed { code: 1006, reason: String::new() });
            return items;
        }
        self.flush();
        if self.closed {
            items.push(WsItem::Closed { code: 1006, reason: String::new() });
        }
        items
    }

    /// Writes out what is queued (a close frame included) and shuts the socket.
    fn finish(&mut self) {
        self.flush();
        if let WsStream::Tls(tls) = &mut self.stream {
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
        self.closed = true;
        let _ = self.stream.tcp().shutdown(std::net::Shutdown::Both);
    }

    /// Sends a close frame with `code` and `reason` and shuts the connection. Nothing is read after it.
    pub fn close(&mut self, code: u16, reason: &str) {
        if self.closed {
            return;
        }
        if !self.close_sent {
            self.queue(opcode::CLOSE, &close_payload(code, reason));
            self.close_sent = true;
        }
        self.finish();
    }
}

impl Drop for WsConn {
    fn drop(&mut self) {
        self.close(1000, "");
    }
}

// ---- the room link ---------------------------------------------------------------------------------------------------

/// One relay connection (host or guest) for the native build: the [`RoomLink`] the co-op layer runs over.
pub struct NativeWsLink {
    core: WsCore,
    /// `RoomLink::send` takes `&self`, a socket needs `&mut`: the link is used from one thread at a time.
    conn: RefCell<Option<WsConn>>,
    /// The host token (sent in the hello; kept for a token refresh).
    token: String,
    opened: Instant,
    last_out: RefCell<Instant>,
}

impl NativeWsLink {
    /// A host connection to room `code` with the token the directory issued for it. `address` is the backend's origin
    /// (`http://host:port`), a `ws://` origin or the full `ws://host:port/api/ws` address.
    pub fn host(code: &str, token: &str, address: &str) -> std::io::Result<NativeWsLink> {
        NativeWsLink::open(Role::Host, code, token, address)
    }

    /// A guest connection; `ticket` is a reserved room's join ticket (base64url), empty for an open room.
    pub fn guest(code: &str, ticket: &str, address: &str) -> std::io::Result<NativeWsLink> {
        NativeWsLink::open(Role::Guest, code, ticket, address)
    }

    fn open(role: Role, code: &str, credential: &str, address: &str) -> std::io::Result<NativeWsLink> {
        let code = code.trim().to_uppercase();
        let mut conn = WsConn::connect(&relay_address(address), CONNECT_TIMEOUT)
            .map_err(|e| std::io::Error::new(ErrorKind::ConnectionRefused, e))?;
        if !conn.send_text(&hello_text(&code, role, credential)) {
            return Err(std::io::Error::new(ErrorKind::BrokenPipe, "THE CONNECTION BROKE BEFORE THE HELLO"));
        }
        let now = Instant::now();
        Ok(NativeWsLink {
            core: WsCore::new(role, &code),
            conn: RefCell::new(Some(conn)),
            token: if role == Role::Host { credential.to_string() } else { String::new() },
            opened: now,
            last_out: RefCell::new(now),
        })
    }

    /// The host token the link was opened with (the last one set by [`RoomLink::update_host_token`]).
    pub fn token(&self) -> &str {
        &self.token
    }

    fn drop_conn(&mut self) {
        if let Some(mut conn) = self.conn.borrow_mut().take() {
            conn.close(1000, "");
        }
    }
}

impl RoomLink for NativeWsLink {
    fn role(&self) -> Role {
        self.core.role()
    }

    fn ready(&self) -> bool {
        self.core.ready()
    }

    fn alive(&self) -> bool {
        self.conn.borrow().is_some() && !self.core.terminal()
    }

    fn max_payload(&self) -> usize {
        MAX_FRAME_PAYLOAD
    }

    fn session(&self) -> u32 {
        self.core.session()
    }

    fn code(&self) -> &str {
        self.core.code()
    }

    /// Host: to guest `session` (the frame's target). Guest: to the host (target 1); `session` is ignored.
    fn send(&self, session: u32, payload: &[u8]) -> bool {
        if !self.core.ready() || payload.is_empty() {
            return false;
        }
        let target = match self.core.role() {
            Role::Host => session as i32,
            Role::Guest => HOST_PEER_ID as i32,
        };
        let channel = payload[0];
        let mode = transfer_mode(channel);
        let mut guard = self.conn.borrow_mut();
        let Some(conn) = guard.as_mut() else { return false };
        // unreliable traffic is dropped rather than queued behind a slow link (ws_room_peer.gd)
        if mode == MODE_UNRELIABLE && conn.buffered() > UNRELIABLE_BUFFER_LIMIT as usize {
            return false;
        }
        let Some(frame) = encode_frame(target, mode, channel, payload) else { return false };
        *self.last_out.borrow_mut() = Instant::now();
        conn.send_binary(&frame)
    }

    /// Host: the relay closes guest `session` with 4005.
    fn kick(&self, session: u32) -> bool {
        if self.core.role() != Role::Host || !self.core.ready() {
            return false;
        }
        match self.conn.borrow_mut().as_mut() {
            Some(conn) => conn.send_text(&kick_text(session)),
            None => false,
        }
    }

    fn poll(&mut self) -> Vec<RelayEvent> {
        let items = match self.conn.borrow_mut().as_mut() {
            Some(conn) => conn.pump(),
            None => Vec::new(),
        };
        for item in items {
            match item {
                WsItem::Text(bytes) => self.core.on_text(&String::from_utf8_lossy(&bytes)),
                WsItem::Binary(bytes) => self.core.on_binary(&bytes),
                WsItem::Closed { code, reason } => {
                    self.core.on_close(code, &reason);
                    *self.conn.borrow_mut() = None;
                }
            }
        }
        let now = Instant::now();
        if self.conn.borrow().is_some() {
            if !self.core.ready() && now.duration_since(self.opened) >= WELCOME_TIMEOUT {
                // the relay never welcomed us (ws_room_peer.gd closes with 4003 too)
                self.core.on_close(4003, "hello timeout");
                self.drop_conn();
            } else if now.duration_since(*self.last_out.borrow()) >= KEEPALIVE {
                *self.last_out.borrow_mut() = now;
                if let Some(conn) = self.conn.borrow_mut().as_mut() {
                    conn.send_text(r#"{"t":"ping"}"#);
                }
            }
        }
        self.core.take_events()
    }

    fn update_host_token(&mut self, token: &str) {
        self.token = token.to_string();
    }

    fn close(&mut self) {
        self.drop_conn();
    }
}

impl Drop for NativeWsLink {
    fn drop(&mut self) {
        self.drop_conn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn address_slice_leaves_time_for_the_next_address() {
        use std::time::Duration;
        let left = Duration::from_secs(8);
        assert!(super::address_slice(left, 2) <= Duration::from_secs(4));
        assert_eq!(super::address_slice(left, 1), left);
        assert!(super::address_slice(Duration::from_millis(100), 3) <= Duration::from_millis(100));
    }

    use super::super::protocol;
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
        // the million-a vector of RFC 3174, and the padding boundary (55 and 56 bytes need one and two blocks)
        assert_eq!(hex(&sha1(&vec![b'a'; 1_000_000])), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        assert_eq!(sha1(&[7u8; 55]).len(), 20);
        assert_ne!(sha1(&[7u8; 55]), sha1(&[7u8; 56]));
    }

    #[test]
    fn the_accept_key_is_the_rfc_6455_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn addresses_parse_with_their_defaults() {
        let u = parse_ws_url("ws://127.0.0.1:8205/api/ws").unwrap();
        assert_eq!(u, WsUrl { secure: false, host: "127.0.0.1".into(), port: 8205, path: "/api/ws".into() });
        let u = parse_ws_url("wss://lastroutegodot.games.kyttuslab.org/api/ws").unwrap();
        assert_eq!((u.secure, u.port, u.host.as_str()), (true, 443, "lastroutegodot.games.kyttuslab.org"));
        assert_eq!(parse_ws_url("http://localhost").unwrap().path, "/");
        assert_eq!(parse_ws_url("ws://h?x=1").unwrap().path, "/?x=1");
        assert_eq!(parse_ws_url("ws://[::1]:9/api/ws").unwrap().host, "::1");
        for bad in ["", "ftp://x", "ws://", "ws://h:0/", "ws://h:99999/", "ws://u@h/", "ws://a b/"] {
            assert!(parse_ws_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_relay_address_comes_from_an_origin_or_a_full_address() {
        assert_eq!(relay_address("http://127.0.0.1:8205"), "ws://127.0.0.1:8205/api/ws");
        assert_eq!(relay_address("https://x.org/"), "wss://x.org/api/ws");
        assert_eq!(relay_address("ws://127.0.0.1:8205"), "ws://127.0.0.1:8205/api/ws");
        assert_eq!(relay_address("ws://127.0.0.1:8205/api/ws"), "ws://127.0.0.1:8205/api/ws");
        assert_eq!(relay_address("http://h:1/api/ws/"), "ws://h:1/api/ws");
    }

    #[test]
    fn the_upgrade_request_names_the_host_path_and_key() {
        let url = parse_ws_url("ws://127.0.0.1:8205/api/ws").unwrap();
        let req = handshake_request(&url, "KEY=");
        assert!(req.starts_with("GET /api/ws HTTP/1.1\r\nHost: 127.0.0.1:8205\r\n"));
        assert!(req.contains("Upgrade: websocket\r\n") && req.contains("Connection: Upgrade\r\n"));
        assert!(req.contains("Sec-WebSocket-Key: KEY=\r\n") && req.contains("Sec-WebSocket-Version: 13\r\n"));
        assert!(req.ends_with("\r\n\r\n"));
        let default = handshake_request(&parse_ws_url("ws://example.org/x").unwrap(), "K");
        assert!(default.contains("Host: example.org\r\n"), "the default port is left out of Host");
    }

    #[test]
    fn the_upgrade_response_is_checked() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let ok = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
        assert!(check_handshake_response(ok, key).is_ok());
        assert!(check_handshake_response(&ok.replace("Upgrade: websocket", "upgrade: WebSocket"), key).is_ok(), "case is free");
        assert!(check_handshake_response(&ok.replace("101", "400"), key).unwrap_err().contains("400"));
        assert!(check_handshake_response(&ok.replace("s3pPLM", "xxxxxx"), key).unwrap_err().contains("KEY"));
        assert!(check_handshake_response(&ok.replace("Upgrade: websocket\r\n", ""), key).is_err());
        assert!(check_handshake_response("", key).is_err());
    }

    #[test]
    fn a_client_frame_is_masked_and_reads_back_through_a_server_parser() {
        let mask = [0x12, 0x34, 0x56, 0x78];
        for len in [0usize, 5, 125, 126, 300, 65_535, 65_536] {
            let payload: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
            let bytes = encode_ws_frame(opcode::BINARY, &payload, Some(mask));
            assert_eq!(bytes[1] & 0x80, 0x80, "masked");
            if len > 0 {
                assert_ne!(&bytes[bytes.len() - len..], payload.as_slice(), "the payload is not sent in the clear");
            }
            let mut server = FrameParser::new(true, 1 << 20);
            server.push(&bytes);
            assert_eq!(server.next_message(), Ok(Some(WsMessage::Binary(payload))), "len {len}");
            assert_eq!(server.next_message(), Ok(None));
        }
    }

    #[test]
    fn length_forms_are_the_rfc_ones() {
        assert_eq!(encode_ws_frame(opcode::TEXT, &[0u8; 125], None)[1], 125);
        assert_eq!(&encode_ws_frame(opcode::TEXT, &[0u8; 126], None)[1..4], &[126, 0, 126]);
        let big = encode_ws_frame(opcode::TEXT, &vec![0u8; 70_000], None);
        assert_eq!(big[1], 127);
        assert_eq!(&big[2..10], &70_000u64.to_be_bytes());
        assert_eq!(encode_ws_frame(opcode::BINARY, b"x", None)[0], 0x82);
    }

    #[test]
    fn a_server_parser_refuses_an_unmasked_client_frame() {
        let mut server = FrameParser::new(true, 1 << 20);
        server.push(&encode_ws_frame(opcode::TEXT, b"hi", None));
        assert_eq!(server.next_message().unwrap_err().code, 1002);
    }

    #[test]
    fn the_parser_waits_for_a_split_frame_and_reads_back_to_back_frames() {
        let a = encode_ws_frame(opcode::TEXT, b"hello", None);
        let b = encode_ws_frame(opcode::BINARY, &[1, 2, 3], None);
        let mut p = FrameParser::new(false, 1 << 20);
        let all: Vec<u8> = a.iter().chain(b.iter()).copied().collect();
        for byte in &all[..all.len() - 1] {
            p.push(&[*byte]);
            let _ = p.next_message();
        }
        // one byte short of the second frame: the first came out, the second waits
        assert_eq!(p.pending(), b.len() - 1);
        p.push(&all[all.len() - 1..]);
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Binary(vec![1, 2, 3]))));
        // both in one push
        p.push(&all);
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Text(b"hello".to_vec()))));
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Binary(vec![1, 2, 3]))));
    }

    /// A frame with FIN clear.
    fn fragment(op: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
        let mut f = encode_ws_frame(op, payload, None);
        if !fin {
            f[0] &= 0x7f;
        }
        f
    }

    #[test]
    fn fragments_join_and_a_ping_may_sit_between_them() {
        let mut p = FrameParser::new(false, 1 << 20);
        p.push(&fragment(opcode::TEXT, false, b"he"));
        p.push(&encode_ws_frame(opcode::PING, b"p", None));
        p.push(&fragment(opcode::CONT, false, b"ll"));
        p.push(&fragment(opcode::CONT, true, b"o"));
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Ping(b"p".to_vec()))));
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Text(b"hello".to_vec()))));
        assert_eq!(p.next_message(), Ok(None));
    }

    #[test]
    fn broken_framing_is_refused() {
        let mut p = FrameParser::new(false, 1000);
        p.push(&fragment(opcode::CONT, true, b"x"));
        assert_eq!(p.next_message().unwrap_err().code, 1002, "a continuation with nothing to continue");

        let mut p = FrameParser::new(false, 1000);
        p.push(&fragment(opcode::TEXT, false, b"a"));
        p.push(&fragment(opcode::TEXT, true, b"b"));
        assert_eq!(p.next_message().unwrap_err().code, 1002, "a new message inside a fragmented one");

        let mut p = FrameParser::new(false, 1000);
        p.push(&fragment(opcode::PING, false, b"a"));
        assert_eq!(p.next_message().unwrap_err().code, 1002, "a fragmented control frame");

        let mut p = FrameParser::new(false, 1000);
        p.push(&encode_ws_frame(opcode::PING, &[0u8; 126], None));
        assert_eq!(p.next_message().unwrap_err().code, 1002, "an over-long control frame");

        let mut p = FrameParser::new(false, 1000);
        let mut f = encode_ws_frame(opcode::TEXT, b"a", None);
        f[0] |= 0x40;
        p.push(&f);
        assert_eq!(p.next_message().unwrap_err().code, 1002, "reserved bits");

        let mut p = FrameParser::new(false, 1000);
        p.push(&[0x83, 0x00]);
        assert_eq!(p.next_message().unwrap_err().code, 1002, "an unknown opcode");

        let mut p = FrameParser::new(false, 10);
        p.push(&encode_ws_frame(opcode::BINARY, &[0u8; 11], None));
        assert_eq!(p.next_message().unwrap_err().code, 1009, "over the cap");

        let mut p = FrameParser::new(false, 10);
        p.push(&fragment(opcode::BINARY, false, &[0u8; 6]));
        p.push(&fragment(opcode::CONT, true, &[0u8; 6]));
        assert_eq!(p.next_message().unwrap_err().code, 1009, "the fragments together are over the cap");
    }

    #[test]
    fn close_frames_carry_a_code_and_a_reason() {
        let mut p = FrameParser::new(false, 1000);
        p.push(&encode_ws_frame(opcode::CLOSE, &close_payload(4001, "host left"), None));
        p.push(&encode_ws_frame(opcode::CLOSE, &[], None));
        p.push(&encode_ws_frame(opcode::CLOSE, &[3], None));
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Close { code: 4001, reason: "host left".into() })));
        assert_eq!(p.next_message(), Ok(Some(WsMessage::Close { code: 1005, reason: String::new() })));
        assert_eq!(p.next_message().unwrap_err().code, 1002, "a one-byte close payload");
        assert_eq!(close_payload(1000, &"x".repeat(300)).len(), 102, "the reason is cut to 100 bytes");
    }

    /// The test certificate (`testdata/`, for 127.0.0.1 and localhost only) as the one root a client trusts.
    const TEST_CERT: &[u8] = include_bytes!("testdata/test_cert.der");
    const TEST_KEY: &[u8] = include_bytes!("testdata/test_key.der");

    fn trusting_test_config() -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(rustls::pki_types::CertificateDer::from(TEST_CERT.to_vec())).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    #[test]
    fn a_wss_address_builds_a_tls_connection_attempt() {
        assert!(parse_ws_url("wss://127.0.0.1:1/api/ws").unwrap().secure);
        // a port nobody listens on: a clear error, not a panic and not the old "not supported" refusal
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = WsConn::connect(&format!("wss://127.0.0.1:{port}/api/ws"), Duration::from_secs(3)).err().unwrap();
        assert!(err.contains("COULD NOT REACH THE SERVER"), "{err}");
        assert!(!err.contains("NOT SUPPORTED"), "{err}");
    }

    /// A loopback wss server: TLS with the test certificate, then the upgrade answer for the key it reads. Returns its port.
    fn serve_wss_upgrade() -> (u16, std::thread::JoinHandle<()>) {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(TEST_CERT.to_vec())],
                rustls::pki_types::PrivatePkcs8KeyDer::from(TEST_KEY.to_vec()).into(),
            )
            .unwrap();
        let config = Arc::new(config);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut tls = rustls::StreamOwned::new(rustls::ServerConnection::new(config).unwrap(), tcp);
            let mut got = Vec::new();
            let mut b = [0u8; 1024];
            while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got.extend_from_slice(&b[..n]),
                }
            }
            let head = String::from_utf8_lossy(&got).to_string();
            let key = head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).unwrap_or("").trim().to_string();
            let answer = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key));
            tls.write_all(answer.as_bytes()).unwrap();
            tls.flush().unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        (port, handle)
    }

    #[test]
    fn a_wss_connection_runs_the_tls_handshake_and_then_the_upgrade() {
        let (port, server) = serve_wss_upgrade();
        let conn = WsConn::connect_with(&format!("wss://127.0.0.1:{port}/api/ws"), Duration::from_secs(5), &trusting_test_config())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!conn.is_closed());
        server.join().unwrap();
    }

    #[test]
    fn a_wss_client_refuses_a_server_that_does_not_speak_tls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let err = WsConn::connect_with(&format!("wss://127.0.0.1:{port}/api/ws"), Duration::from_secs(3), &trusting_test_config()).err().unwrap();
        assert!(err.contains("SECURE CONNECTION FAILED"), "{err}");
        t.join().unwrap();
    }

    #[test]
    fn a_wss_client_refuses_a_certificate_the_roots_do_not_vouch_for() {
        // the built-in roots do not include the test certificate, so the handshake must fail on it
        let (port, _server) = serve_wss_upgrade();
        let err = WsConn::connect(&format!("wss://127.0.0.1:{port}/api/ws"), Duration::from_secs(5)).err().unwrap();
        assert!(err.contains("SECURE CONNECTION FAILED"), "{err}");
    }

    // ---- over a real loopback socket, against the in-process hub ----------------------------------------------------

    use super::super::protocol::Message;
    use super::super::session::{CoopSession, SessionEvent};
    use super::super::ws_test_hub::TestHub;

    /// Polls `step` until it returns true (5 s at most).
    fn until(what: &str, mut step: impl FnMut() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < end {
            if step() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("timed out waiting for {what}");
    }

    /// Polls a link until it reports an event that `pick` accepts, and returns everything it saw.
    fn wait_event(link: &mut NativeWsLink, what: &str, pick: impl Fn(&RelayEvent) -> bool) -> Vec<RelayEvent> {
        let mut seen = Vec::new();
        until(what, || {
            seen.extend(link.poll());
            seen.iter().any(&pick)
        });
        seen
    }

    fn ready_session(seen: &[RelayEvent]) -> u32 {
        seen.iter().find_map(|e| if let RelayEvent::Ready { session } = e { Some(*session) } else { None }).expect("a Ready event")
    }

    fn host_and_guest(hub: &TestHub) -> (NativeWsLink, NativeWsLink, u32) {
        hub.add_room("ABCDE", "tok");
        let mut host = NativeWsLink::host("abcde", "tok", &hub.url()).unwrap();
        wait_event(&mut host, "the host welcome", |e| matches!(e, RelayEvent::Ready { .. }));
        assert!(host.ready() && host.session() == 0 && host.role() == Role::Host && host.code() == "ABCDE");
        let mut guest = NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap();
        let seen = wait_event(&mut guest, "the guest welcome", |e| matches!(e, RelayEvent::Ready { .. }));
        let id = ready_session(&seen);
        assert!(id >= 2 && guest.session() == id);
        let seen = wait_event(&mut host, "the peer join", |e| matches!(e, RelayEvent::PeerJoined(_)));
        assert!(seen.contains(&RelayEvent::PeerJoined(id)));
        (host, guest, id)
    }

    #[test]
    fn a_host_and_a_guest_exchange_frames_through_the_hub() {
        let hub = TestHub::start();
        let (mut host, mut guest, id) = host_and_guest(&hub);
        assert_eq!(hub.guest_ids("ABCDE"), vec![id]);
        // a guest to the host: the hub names the sender
        assert!(guest.send(0, &[protocol::channel::EVENTS, 1, 2, 3]));
        let seen = wait_event(&mut host, "the guest data", |e| matches!(e, RelayEvent::Data { .. }));
        assert!(seen.contains(&RelayEvent::Data { session: id, payload: vec![protocol::channel::EVENTS, 1, 2, 3] }));
        // the host to the guest: the sender is the host, peer 1
        assert!(host.send(id, &[protocol::channel::STATE, 9, 9]));
        let seen = wait_event(&mut guest, "the host data", |e| matches!(e, RelayEvent::Data { .. }));
        assert!(seen.contains(&RelayEvent::Data { session: 1, payload: vec![protocol::channel::STATE, 9, 9] }));
        // the biggest frame the relay carries
        let big: Vec<u8> = (0..MAX_FRAME_PAYLOAD).map(|i| i as u8).collect();
        assert!(guest.send(0, &big));
        let seen = wait_event(&mut host, "the big frame", |e| matches!(e, RelayEvent::Data { payload, .. } if payload.len() == big.len()));
        assert!(seen.contains(&RelayEvent::Data { session: id, payload: big }));
        assert!(!guest.send(0, &vec![0u8; MAX_FRAME_PAYLOAD + 1]), "over the relay cap is refused here, not by the hub");
        assert!(!guest.send(0, &[]), "an empty payload has no channel");
        assert!(hub.routed.load(std::sync::atomic::Ordering::Relaxed) >= 3);
    }

    #[test]
    fn a_guest_leaving_is_reported_to_the_host_and_the_host_leaving_to_the_guest() {
        let hub = TestHub::start();
        let (mut host, mut guest, id) = host_and_guest(&hub);
        guest.close();
        let seen = wait_event(&mut host, "the peer leave", |e| matches!(e, RelayEvent::PeerLeft(_)));
        assert!(seen.contains(&RelayEvent::PeerLeft(id)));
        assert!(!guest.send(0, &[0, 1]), "a closed link sends nothing");

        let mut second = NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap();
        wait_event(&mut second, "the second welcome", |e| matches!(e, RelayEvent::Ready { .. }));
        host.close();
        let seen = wait_event(&mut second, "the host leaving", |e| matches!(e, RelayEvent::HostLeft(_)));
        assert_eq!(seen.iter().filter(|e| matches!(e, RelayEvent::HostLeft(_))).count(), 1, "reported once");
        assert!(seen.contains(&RelayEvent::HostLeft("THE HOST LEFT THE ROOM".into())), "the host-off control message comes before the 4001 close");
    }

    #[test]
    fn the_relay_refusals_reach_the_link_as_errors() {
        let hub = TestHub::start();
        hub.add_room("ABCDE", "tok");
        let mut wrong = NativeWsLink::host("ABCDE", "nope", &hub.url()).unwrap();
        let seen = wait_event(&mut wrong, "the refusal", |e| matches!(e, RelayEvent::Error(_)));
        assert!(seen.contains(&RelayEvent::Error("INVALID HOST TOKEN".into())), "{seen:?}");
        assert!(!wrong.ready());

        let mut no_room = NativeWsLink::guest("ZZZZZ", "", &hub.url()).unwrap();
        let seen = wait_event(&mut no_room, "no room", |e| matches!(e, RelayEvent::Error(_)));
        assert!(seen.contains(&RelayEvent::Error("ROOM NOT FOUND".into())));

        let mut early = NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap();
        let seen = wait_event(&mut early, "no host yet", |e| matches!(e, RelayEvent::Error(_)));
        assert!(seen.contains(&RelayEvent::Error("HOST IS NOT CONNECTED YET".into())));
    }

    #[test]
    fn a_kick_and_a_closed_room_reach_the_guest_with_their_reasons() {
        let hub = TestHub::start();
        let (mut host, mut guest, id) = host_and_guest(&hub);
        // the host asks the relay to remove the guest (a text control frame, ws_relay.mjs)
        host.conn.borrow_mut().as_mut().unwrap().send_text(&format!(r#"{{"t":"kick","id":{id}}}"#));
        let seen = wait_event(&mut guest, "the kick", |e| matches!(e, RelayEvent::HostLeft(_)));
        assert!(seen.contains(&RelayEvent::HostLeft("YOU WERE REMOVED FROM THE ROOM".into())));
        wait_event(&mut host, "the guest leaving", |e| matches!(e, RelayEvent::PeerLeft(_)));

        let mut again = NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap();
        wait_event(&mut again, "welcome", |e| matches!(e, RelayEvent::Ready { .. }));
        hub.close_room("ABCDE");
        let seen = wait_event(&mut again, "the room closing", |e| matches!(e, RelayEvent::HostLeft(_)));
        assert!(seen.contains(&RelayEvent::HostLeft("room closed".into())));
    }

    #[test]
    fn a_ping_from_the_hub_is_answered_with_a_pong() {
        let hub = TestHub::start();
        let (mut host, mut guest, _) = host_and_guest(&hub);
        hub.ping_all("ABCDE");
        until("two pongs", || {
            host.poll();
            guest.poll();
            hub.pongs.load(std::sync::atomic::Ordering::Relaxed) >= 2
        });
    }

    #[test]
    fn a_link_to_nothing_fails_to_open() {
        // a port nobody listens on
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = NativeWsLink::host("ABCDE", "tok", &format!("ws://127.0.0.1:{port}")).err().expect("no server");
        assert!(err.to_string().contains("COULD NOT REACH"), "{err}");
        assert!(NativeWsLink::guest("ABCDE", "", "not a url").is_err());
    }

    #[test]
    fn a_server_that_is_not_a_websocket_server_is_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let err = WsConn::connect(&format!("ws://127.0.0.1:{port}/api/ws"), Duration::from_secs(3)).err().unwrap();
        assert!(err.contains("200"), "{err}");
        t.join().unwrap();
    }

    /// The co-op layer over the native link: a registered guest, a reliable stream of messages in order, a guest leaving.
    #[test]
    fn a_co_op_session_runs_over_native_websocket_links() {
        let hub = TestHub::start();
        hub.add_room("ABCDE", "tok");
        let mut host = CoopSession::new(NativeWsLink::host("ABCDE", "tok", &hub.url()).unwrap());
        until("the host link", || {
            host.poll();
            host.link.ready()
        });
        let mut guest = CoopSession::new(NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap());
        guest.send(HOST_PEER_ID, &Message::RegisterPeer { requested_name: "R-7".into() });
        let (mut host_heard, mut guest_heard, mut sent, mut guest_id) = (Vec::new(), Vec::new(), false, 0);
        let mut host_events = Vec::new();
        until("the exchange", || {
            for event in host.poll() {
                if let SessionEvent::Message { from, message: Message::RegisterPeer { requested_name } } = &event {
                    guest_id = *from;
                    host_heard.push(requested_name.clone());
                    if !sent {
                        sent = true;
                        for i in 0..5 {
                            host.send(*from, &Message::Chat { peer_id: 1, display_name: "HOST".into(), message: format!("m{i}") });
                        }
                    }
                }
                host_events.push(event);
            }
            for event in guest.poll() {
                if let SessionEvent::Message { from, message: Message::Chat { message, .. } } = event {
                    assert_eq!(from, HOST_PEER_ID);
                    guest_heard.push(message);
                }
            }
            guest_heard.len() == 5
        });
        assert_eq!(host_heard, vec!["R-7".to_string()]);
        assert_eq!(guest_heard, (0..5).map(|i| format!("m{i}")).collect::<Vec<_>>());
        assert!(guest_id >= 2 && host_events.contains(&SessionEvent::PeerSeen(guest_id)));
        guest.close();
        until("the guest leaving", || host.poll().contains(&SessionEvent::Relay(RelayEvent::PeerLeft(guest_id))));
    }

    /// The UDP relay holds 1450 bytes a message; a WebSocket frame holds far more, so a big hunter frame (a dedicated room
    /// with hunters on several levels) crosses whole.
    #[test]
    fn a_message_bigger_than_a_udp_datagram_crosses_a_websocket_link() {
        let hub = TestHub::start();
        hub.add_room("ABCDE", "tok");
        let mut host = CoopSession::new(NativeWsLink::host("ABCDE", "tok", &hub.url()).unwrap());
        until("the host link", || {
            host.poll();
            host.link.ready()
        });
        assert_eq!(host.link.max_payload(), MAX_FRAME_PAYLOAD);
        let mut guest = CoopSession::new(NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap());
        guest.send(HOST_PEER_ID, &Message::RegisterPeer { requested_name: "G".into() });
        let mut guest_id = 0;
        until("the registration", || {
            guest.poll();
            for event in host.poll() {
                if let SessionEvent::Message { from, .. } = event {
                    guest_id = from;
                }
            }
            guest_id != 0
        });
        let entries: Vec<serde_json::Value> = (0..40).map(|i| serde_json::json!({"index": i, "id": format!("hunter_{i}"), "position": [1.5, 2.5], "pad": "x".repeat(60)})).collect();
        let message = Message::HuntersLarge { data: entries.clone() };
        assert!(message.encode().len() > super::super::session::MAX_MESSAGE_BYTES, "bigger than a UDP datagram holds");
        assert!(host.send(guest_id, &message), "the link takes it");
        let mut got = None;
        until("the big message", || {
            host.poll();
            for event in guest.poll() {
                if let SessionEvent::Message { message: Message::HuntersLarge { data }, .. } = event {
                    got = Some(data);
                }
            }
            got.is_some()
        });
        assert_eq!(got.unwrap(), entries);
    }

    /// Against the real Node relay of the Godot project: set `LR_WS_RELAY_MJS` to its `backend/ws_relay.mjs` (and have
    /// `node` on the path). Skipped otherwise. It starts a throw-away http server on a free port that hosts the relay with
    /// one fake room, joins a native host and guest through it and checks the frames both ways.
    #[test]
    fn node_hub_host_and_guest_exchange_frames() {
        let Ok(relay) = std::env::var("LR_WS_RELAY_MJS") else {
            eprintln!("skipped: LR_WS_RELAY_MJS is not set");
            return;
        };
        let script = format!(
            r#"import {{ createServer }} from "node:http";
import {{ createWsRelay }} from "file:///{}";
const relay = createWsRelay({{ roomByCode: (code) => code === "ABCDE" ? {{ code, token: "tok", transport: "ws" }} : null, logger: {{ log() {{}} }} }});
const server = createServer((req, res) => {{ res.statusCode = 404; res.end(); }});
server.on("upgrade", (req, socket, head) => {{ if (!relay.handleUpgrade(req, socket, head)) socket.destroy(); }});
server.listen(0, "127.0.0.1", () => console.log("PORT " + server.address().port));
setTimeout(() => process.exit(0), 60000).unref();
"#,
            relay.replace('\\', "/")
        );
        let dir = std::env::temp_dir().join(format!("lr_node_hub_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hub.mjs");
        std::fs::write(&file, script).unwrap();
        let mut child = std::process::Command::new("node").arg(&file).stdout(std::process::Stdio::piped()).spawn().expect("node");
        let mut line = String::new();
        {
            use std::io::BufRead;
            let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
            out.read_line(&mut line).unwrap();
        }
        let port: u16 = line.trim().strip_prefix("PORT ").expect("the hub prints its port").parse().unwrap();
        let origin = format!("http://127.0.0.1:{port}");
        let mut host = NativeWsLink::host("ABCDE", "tok", &origin).unwrap();
        wait_event(&mut host, "the Node host welcome", |e| matches!(e, RelayEvent::Ready { .. }));
        let mut guest = NativeWsLink::guest("ABCDE", "", &origin).unwrap();
        let seen = wait_event(&mut guest, "the Node guest welcome", |e| matches!(e, RelayEvent::Ready { .. }));
        let id = ready_session(&seen);
        wait_event(&mut host, "the Node peer join", |e| matches!(e, RelayEvent::PeerJoined(p) if *p == id));
        assert!(guest.send(0, &[protocol::channel::EVENTS, 4, 5, 6]));
        let seen = wait_event(&mut host, "the guest data", |e| matches!(e, RelayEvent::Data { .. }));
        assert!(seen.contains(&RelayEvent::Data { session: id, payload: vec![protocol::channel::EVENTS, 4, 5, 6] }));
        assert!(host.send(id, &[protocol::channel::STATE, 7]));
        let seen = wait_event(&mut guest, "the host data", |e| matches!(e, RelayEvent::Data { .. }));
        assert!(seen.contains(&RelayEvent::Data { session: 1, payload: vec![protocol::channel::STATE, 7] }));
        let big: Vec<u8> = (0..MAX_FRAME_PAYLOAD).map(|i| (i * 3) as u8).collect();
        assert!(host.send(id, &big));
        wait_event(&mut guest, "the big frame", |e| matches!(e, RelayEvent::Data { payload, .. } if payload.len() == big.len()));
        host.close();
        let seen = wait_event(&mut guest, "the Node host leaving", |e| matches!(e, RelayEvent::HostLeft(_)));
        assert!(seen.iter().any(|e| matches!(e, RelayEvent::HostLeft(_))));
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
