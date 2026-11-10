//! A small blocking HTTP/1.1 client for the account calls: one request per connection (`Connection: close`),
//! connect, read and write timeouts under one deadline, `Content-Length`, chunked and read-until-close bodies, a size
//! cap, and https over rustls (`ring` provider, Mozilla roots from `webpki-roots`, TLS 1.2 and 1.3, the server name is
//! always checked). Redirects are never followed: a backend does not redirect, and a 3xx reaches the caller as is.
//! A cut-down copy of Last Route's `net/http.rs`. Run it on a worker thread (`super::Pending`).
//!
//! The browser build has no sockets here: `send` answers "not available" there.
#![allow(dead_code)]

use super::{Json, NetError};
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// The biggest response accepted (an account reply is a few hundred bytes).
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Request {
        Request { method, url: url.into(), headers: Vec::new(), body: None, timeout: DEFAULT_TIMEOUT }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn json_body(mut self, value: &serde_json::Value) -> Request {
        self.body = Some(value.to_string().into_bytes());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Request {
        self.timeout = timeout;
        self
    }

    pub fn send(&self) -> Result<Response, NetError> {
        send(self)
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// `{}` unless the body is a JSON object (an empty body or a proxy's error page is "no payload").
    pub fn json(&self) -> Json {
        parse_body(&self.body)
    }
}

/// `{}` unless the body is a JSON object.
pub fn parse_body(body: &[u8]) -> Json {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if !text.starts_with('{') {
        return Json::new();
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => Json::new(),
    }
}

/// `http(s)://host[:port]/path` split into its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(url: &str) -> Result<Url, NetError> {
    let invalid = || NetError::Unsupported("INVALID SERVER ADDRESS".into());
    let (https, rest) = if url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
        (true, &url[8..])
    } else if url.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://")) {
        (false, &url[7..])
    } else {
        return Err(NetError::Unsupported("ONLY HTTP AND HTTPS ADDRESSES ARE SUPPORTED".into()));
    };
    let default_port = if https { 443 } else { 80 };
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || authority.contains('@') || path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(invalid());
    }
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped.find(']').ok_or_else(invalid)?;
        let after = &stripped[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| NetError::Unsupported("INVALID SERVER PORT".into()))?,
            None if after.is_empty() => default_port,
            None => return Err(invalid()),
        };
        (stripped[..end].to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| NetError::Unsupported("INVALID SERVER PORT".into()))?),
            None => (authority.to_string(), default_port),
        }
    };
    if host.is_empty() || host.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(invalid());
    }
    Ok(Url { https, host, port, path })
}

/// The request head and body as bytes (`Host`, `User-Agent`, `Accept`, the caller's headers, `Content-Length`).
pub fn build_request_bytes(req: &Request, url: &Url) -> Result<Vec<u8>, NetError> {
    let host = if url.host.contains(':') { format!("[{}]", url.host) } else { url.host.clone() };
    let default_port = url.port == if url.https { 443 } else { 80 };
    let host = if default_port { host } else { format!("{host}:{}", url.port) };
    let mut head = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", req.method.as_str(), url.path, host);
    let has = |name: &str| req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if !has("user-agent") {
        head.push_str(&format!("User-Agent: NightWatchRust/{} ({})\r\n", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
    }
    if !has("accept") {
        head.push_str("Accept: */*\r\n");
    }
    for (k, v) in &req.headers {
        if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
            return Err(NetError::Protocol("INVALID HEADER".into()));
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    match &req.body {
        Some(b) => head.push_str(&format!("Content-Length: {}\r\n", b.len())),
        None if req.method == Method::Post => head.push_str("Content-Length: 0\r\n"),
        None => {}
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut out = head.into_bytes();
    if let Some(b) = &req.body {
        out.extend_from_slice(b);
    }
    Ok(out)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Splits a raw response (everything the server sent before closing) into status, headers and body.
pub fn parse_response(raw: &[u8], limit: usize) -> Result<Response, NetError> {
    let header_end = find(raw, b"\r\n\r\n").ok_or_else(|| NetError::Transport("CONNECTION CLOSED BEFORE A RESPONSE".into()))?;
    let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut parts = lines.next().unwrap_or("").splitn(3, ' ');
    if !parts.next().unwrap_or("").starts_with("HTTP/1.") {
        return Err(NetError::Protocol("NOT AN HTTP RESPONSE".into()));
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| NetError::Protocol("BAD STATUS LINE".into()))?;
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
    let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let rest = &raw[header_end + 4..];
    let no_body = status == 204 || status == 304 || (100..200).contains(&status);
    let body = if no_body {
        Vec::new()
    } else if header("transfer-encoding").map(|v| v.to_ascii_lowercase().contains("chunked")).unwrap_or(false) {
        let mut out = Vec::new();
        let mut pos = 0usize;
        loop {
            let line_end = find(&rest[pos..], b"\r\n").map(|i| pos + i).ok_or_else(|| NetError::Protocol("TRUNCATED CHUNKED BODY".into()))?;
            let size_text = String::from_utf8_lossy(&rest[pos..line_end]).to_string();
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| NetError::Protocol("BAD CHUNK SIZE".into()))?;
            pos = line_end + 2;
            if size == 0 {
                break;
            }
            if out.len().saturating_add(size) > limit {
                return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
            }
            if rest.len() < pos + size + 2 {
                return Err(NetError::Protocol("TRUNCATED CHUNKED BODY".into()));
            }
            out.extend_from_slice(&rest[pos..pos + size]);
            pos += size + 2;
        }
        out
    } else if let Some(len) = header("content-length") {
        let len: usize = len.parse().map_err(|_| NetError::Protocol("BAD CONTENT-LENGTH".into()))?;
        if len > limit {
            return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
        }
        if rest.len() < len {
            return Err(NetError::Protocol("TRUNCATED BODY".into()));
        }
        rest[..len].to_vec()
    } else {
        if rest.len() > limit {
            return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
        }
        rest.to_vec()
    };
    Ok(Response { status, headers, body })
}

#[cfg(target_arch = "wasm32")]
pub fn send(_req: &Request) -> Result<Response, NetError> {
    Err(NetError::Unsupported("ACCOUNTS NEED THE DESKTOP BUILD".into()))
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::send;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
    use std::sync::{Arc, OnceLock};
    use std::time::Instant;

    fn transport(error: std::io::Error) -> NetError {
        match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => NetError::Timeout,
            _ => NetError::Transport(error.to_string()),
        }
    }

    fn tls_config() -> Arc<rustls::ClientConfig> {
        static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
        CONFIG
            .get_or_init(|| {
                let mut roots = rustls::RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let provider = Arc::new(rustls::crypto::ring::default_provider());
                let config = rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("ring supports the default TLS versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                Arc::new(config)
            })
            .clone()
    }

    enum Stream {
        Plain(TcpStream),
        Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
    }

    impl Stream {
        fn tcp(&self) -> &TcpStream {
            match self {
                Stream::Plain(s) => s,
                Stream::Tls(s) => &s.sock,
            }
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self {
                Stream::Plain(s) => s.read(buf),
                // A server that closes without a TLS close_notify is common; the body framing is checked by the parser.
                Stream::Tls(s) => match s.read(buf) {
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
                    other => other,
                },
            }
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            match self {
                Stream::Plain(s) => s.write(buf),
                Stream::Tls(s) => s.write(buf),
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            match self {
                Stream::Plain(s) => s.flush(),
                Stream::Tls(s) => s.flush(),
            }
        }
    }

    /// The system resolver has no timeout, so it runs on a helper thread and the caller waits until the deadline.
    fn resolve(host: &str, port: u16, deadline: Instant) -> Result<Vec<SocketAddr>, NetError> {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let host = host.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send((host.as_str(), port).to_socket_addrs().map(|a| a.collect::<Vec<_>>()));
        });
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result.map_err(transport),
            Err(_) => Err(NetError::Timeout),
        }
    }

    pub fn send(req: &Request) -> Result<Response, NetError> {
        let deadline = Instant::now() + req.timeout;
        let url = parse_url(&req.url)?;
        let bytes = build_request_bytes(req, &url)?;
        let addrs = resolve(&url.host, url.port, deadline)?;
        let mut last = NetError::Transport("NO ADDRESS".into());
        let mut tcp = None;
        for addr in addrs {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(NetError::Timeout);
            }
            match TcpStream::connect_timeout(&addr, left) {
                Ok(s) => {
                    tcp = Some(s);
                    break;
                }
                Err(e) => last = transport(e),
            }
        }
        let tcp = tcp.ok_or(last)?;
        let _ = tcp.set_nodelay(true);
        let mut stream = if url.https {
            let name = rustls::pki_types::ServerName::try_from(url.host.clone())
                .map_err(|_| NetError::Unsupported("INVALID SERVER ADDRESS".into()))?;
            let connection = rustls::ClientConnection::new(tls_config(), name).map_err(|e| NetError::Transport(format!("TLS: {e}")))?;
            Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp)))
        } else {
            Stream::Plain(tcp)
        };
        let set_timeouts = |s: &Stream| -> Result<(), NetError> {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(NetError::Timeout);
            }
            s.tcp().set_read_timeout(Some(left)).map_err(transport)?;
            s.tcp().set_write_timeout(Some(left)).map_err(transport)?;
            Ok(())
        };
        set_timeouts(&stream)?;
        stream.write_all(&bytes).map_err(transport)?;
        stream.flush().map_err(transport)?;
        // `Connection: close`: read until the server closes (the parser frames the body).
        let mut raw: Vec<u8> = Vec::with_capacity(4096);
        let mut chunk = [0u8; 8192];
        loop {
            set_timeouts(&stream)?;
            let n = stream.read(&mut chunk).map_err(transport)?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..n]);
            if raw.len() > MAX_RESPONSE_BYTES + MAX_HEADER_BYTES {
                return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
            }
        }
        parse_response(&raw, MAX_RESPONSE_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_parse() {
        let u = parse_url("https://lastrouterust.games.kyttuslab.org").unwrap();
        assert!(u.https && u.port == 443 && u.path == "/");
        let u = parse_url("http://127.0.0.1:8105/api/x?y=1").unwrap();
        assert!(!u.https && u.port == 8105 && u.path == "/api/x?y=1");
        assert!(parse_url("ftp://x").is_err());
        assert!(parse_url("http://a@b/").is_err());
    }

    #[test]
    fn request_bytes_carry_the_json_headers_and_length() {
        let req = Request::new(Method::Post, "http://127.0.0.1:81/api/account/login")
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json_body(&serde_json::json!({"a": 1}));
        let url = parse_url(&req.url).unwrap();
        let text = String::from_utf8(build_request_bytes(&req, &url).unwrap()).unwrap();
        assert!(text.starts_with("POST /api/account/login HTTP/1.1\r\nHost: 127.0.0.1:81\r\n"));
        assert!(text.contains("Content-Type: application/json\r\n"));
        assert!(text.contains("Content-Length: 7\r\n"));
        assert!(text.ends_with("\r\n\r\n{\"a\":1}"));
        let bad = Request::new(Method::Get, "http://h/").header("X", "a\r\nb");
        assert!(build_request_bytes(&bad, &parse_url("http://h/").unwrap()).is_err());
    }

    #[test]
    fn responses_parse_length_chunked_and_close() {
        let r = parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}", 100).unwrap();
        assert!(r.ok() && r.body == b"{}");
        let r = parse_response(b"HTTP/1.1 401 No\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n", 100).unwrap();
        assert_eq!(r.status, 401);
        assert_eq!(r.json().get("a").and_then(|v| v.as_i64()), Some(1));
        let r = parse_response(b"HTTP/1.1 200 OK\r\n\r\n{\"ok\":true}", 100).unwrap();
        assert_eq!(r.json().get("ok").and_then(|v| v.as_bool()), Some(true));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}", 100).is_err());
        assert!(parse_response(b"garbage", 100).is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n", 100).is_err());
    }

    #[test]
    fn non_object_bodies_are_no_payload() {
        assert!(parse_body(b"<html>").is_empty());
        assert!(parse_body(b"[1]").is_empty());
        assert!(parse_body(b"").is_empty());
    }
}
