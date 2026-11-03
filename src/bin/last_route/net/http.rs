//! A minimal HTTP/1.1 client over `std::net::TcpStream`, standing in for Godot's `HTTPRequest`
//! node that `game/autoload/backend_client.gd` and `run_saves.gd` use. The Last Route backend
//! (`backend/server.mjs`) is plain HTTP on its Pterodactyl allocation, so no TLS is needed; an
//! `https://` origin is refused with a clear error (the original only uses https on its web build).
//!
//! What it does: one request per connection (`Connection: close`), connect / read / write
//! timeouts plus an overall deadline, `Content-Length`, `Transfer-Encoding: chunked` and
//! read-until-close bodies, and a response size cap.
//!
//! Headers: the original sends `Accept: application/json` and `Content-Type: application/json`
//! (`_headers()`), plus whatever Godot's HTTPClient adds itself (`Host`, `User-Agent:
//! GodotEngine/<version> (<OS>)`, `Accept-Encoding: gzip, deflate` from HTTPRequest's
//! `accept_gzip`, `Content-Length`). PORT DEVIATIONS: the User-Agent here is
//! `LastRouteRust/<crate version> (<os>)` (the server never reads it), and no `Accept-Encoding`
//! is sent, so the server never compresses (it does not anyway: `json()` in server.mjs writes
//! plain bodies).

use super::NetError;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// Godot's HTTPRequest has no timeout by default (`timeout = 0`), which is fine for a node that
/// can be cancelled; a background thread cannot be, so every request here gets one. PORT
/// DEVIATION: requests the original left unbounded use this.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// The biggest response accepted (the largest real one is a 64 KB cloud save plus framing).
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
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
    /// The biggest response this request accepts (the default is MAX_RESPONSE_BYTES; downloads raise it).
    pub limit: usize,
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Request {
        Request { method, url: url.into(), headers: Vec::new(), body: None, timeout: DEFAULT_TIMEOUT, limit: MAX_RESPONSE_BYTES }
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

    pub fn limit(mut self, bytes: usize) -> Request {
        self.limit = bytes;
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

    /// backend_client.gd `_parse_body`: an empty or non-JSON body (a proxy error page) is just
    /// "no payload", and only an object counts.
    pub fn json(&self) -> serde_json::Map<String, serde_json::Value> {
        parse_body(&self.body)
    }
}

/// `_parse_body`: `{}` unless the body is a JSON object.
pub fn parse_body(body: &[u8]) -> serde_json::Map<String, serde_json::Value> {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() || !(text.starts_with('{') || text.starts_with('[')) {
        return serde_json::Map::new();
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// `http://host[:port]/path?query` split into its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(url: &str) -> Result<Url, NetError> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Err(NetError::Unsupported("HTTPS IS NOT SUPPORTED BY THIS BUILD".into()));
    }
    let rest = url
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("http://"))
        .map(|_| &url[7..])
        .ok_or_else(|| NetError::Unsupported("ONLY HTTP ORIGINS ARE SUPPORTED".into()))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(NetError::Unsupported("INVALID SERVER ADDRESS".into()));
    }
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        // [ipv6]:port
        let end = stripped.find(']').ok_or_else(|| NetError::Unsupported("INVALID SERVER ADDRESS".into()))?;
        let host = &stripped[..end];
        let after = &stripped[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| NetError::Unsupported("INVALID SERVER PORT".into()))?,
            None => 80,
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| NetError::Unsupported("INVALID SERVER PORT".into()))?),
            None => (authority.to_string(), 80),
        }
    };
    Ok(Url { host, port, path: path.to_string() })
}

fn user_agent() -> String {
    format!("LastRouteRust/{} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS)
}

fn transport(error: std::io::Error) -> NetError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => NetError::Timeout,
        _ => NetError::Transport(error.to_string()),
    }
}

fn connect(url: &Url, deadline: Instant) -> Result<TcpStream, NetError> {
    let addrs: Vec<SocketAddr> = (url.host.as_str(), url.port).to_socket_addrs().map_err(transport)?.collect();
    if addrs.is_empty() {
        return Err(NetError::Transport("NO ADDRESS".into()));
    }
    let mut last = NetError::Transport("NO ADDRESS".into());
    for addr in addrs {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(NetError::Timeout);
        }
        match TcpStream::connect_timeout(&addr, left) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = transport(e),
        }
    }
    Err(last)
}

/// Sends one request and reads the whole response (blocking; run it on a worker thread from
/// the frame loop, see [`super::Pending`]).
pub fn send(req: &Request) -> Result<Response, NetError> {
    let url = parse_url(&req.url)?;
    let deadline = Instant::now() + req.timeout;
    let mut stream = connect(&url, deadline)?;
    let _ = stream.set_nodelay(true);
    let host_header = if url.host.contains(':') { format!("[{}]", url.host) } else { url.host.clone() };
    let host_header = if url.port == 80 { host_header } else { format!("{host_header}:{}", url.port) };
    let mut head = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", req.method.as_str(), url.path, host_header);
    let has = |name: &str| req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if !has("user-agent") {
        head.push_str(&format!("User-Agent: {}\r\n", user_agent()));
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
    if let Some(body) = &req.body {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    } else if matches!(req.method, Method::Post | Method::Put) {
        head.push_str("Content-Length: 0\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    let set_timeouts = |s: &TcpStream| -> Result<(), NetError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(NetError::Timeout);
        }
        s.set_read_timeout(Some(left)).map_err(transport)?;
        s.set_write_timeout(Some(left)).map_err(transport)?;
        Ok(())
    };
    set_timeouts(&stream)?;
    stream.write_all(head.as_bytes()).map_err(transport)?;
    if let Some(body) = &req.body {
        stream.write_all(body).map_err(transport)?;
    }
    stream.flush().map_err(transport)?;

    // read until the headers end
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return Err(NetError::Protocol("RESPONSE HEADERS TOO LARGE".into()));
        }
        set_timeouts(&stream)?;
        let n = stream.read(&mut chunk).map_err(transport)?;
        if n == 0 {
            return Err(NetError::Transport("CONNECTION CLOSED BEFORE A RESPONSE".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head_text = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head_text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(NetError::Protocol("NOT AN HTTP RESPONSE".into()));
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| NetError::Protocol("BAD STATUS LINE".into()))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    let mut rest = buf[header_end + 4..].to_vec();
    let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let no_body = status == 204 || status == 304 || (100..200).contains(&status);
    let mut read_more = |rest: &mut Vec<u8>| -> Result<bool, NetError> {
        set_timeouts(&stream)?;
        let n = stream.read(&mut chunk).map_err(transport)?;
        if n == 0 {
            return Ok(false);
        }
        rest.extend_from_slice(&chunk[..n]);
        if rest.len() > req.limit + 64 * 1024 {
            return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
        }
        Ok(true)
    };
    let body = if no_body {
        Vec::new()
    } else if header("transfer-encoding").map(|v| v.to_ascii_lowercase().contains("chunked")).unwrap_or(false) {
        let mut out = Vec::new();
        let mut pos = 0usize;
        loop {
            // chunk size line
            let line_end = loop {
                if let Some(i) = find(&rest[pos..], b"\r\n") {
                    break pos + i;
                }
                if !read_more(&mut rest)? {
                    return Err(NetError::Protocol("TRUNCATED CHUNKED BODY".into()));
                }
            };
            let size_text = String::from_utf8_lossy(&rest[pos..line_end]).to_string();
            let size_text = size_text.split(';').next().unwrap_or("").trim().to_string();
            let size = usize::from_str_radix(&size_text, 16).map_err(|_| NetError::Protocol("BAD CHUNK SIZE".into()))?;
            pos = line_end + 2;
            if size == 0 {
                break;
            }
            if out.len() + size > req.limit {
                return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
            }
            while rest.len() < pos + size + 2 {
                if !read_more(&mut rest)? {
                    return Err(NetError::Protocol("TRUNCATED CHUNKED BODY".into()));
                }
            }
            out.extend_from_slice(&rest[pos..pos + size]);
            pos += size + 2;
        }
        out
    } else if let Some(len) = header("content-length") {
        let len: usize = len.parse().map_err(|_| NetError::Protocol("BAD CONTENT-LENGTH".into()))?;
        if len > req.limit {
            return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
        }
        while rest.len() < len {
            if !read_more(&mut rest)? {
                return Err(NetError::Protocol("TRUNCATED BODY".into()));
            }
        }
        rest.truncate(len);
        rest
    } else {
        while read_more(&mut rest)? {}
        if rest.len() > req.limit {
            return Err(NetError::Protocol("RESPONSE TOO LARGE".into()));
        }
        rest
    };
    Ok(Response { status, headers, body })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn parses_urls() {
        let u = parse_url("http://lastroute.kyttuslab.org:8105/api/health").unwrap();
        assert_eq!(u, Url { host: "lastroute.kyttuslab.org".into(), port: 8105, path: "/api/health".into() });
        let u = parse_url("http://127.0.0.1:18105").unwrap();
        assert_eq!(u.path, "/");
        assert_eq!(parse_url("http://example.org/x").unwrap().port, 80);
        assert!(matches!(parse_url("https://kyttuslab.org/godot2-api"), Err(NetError::Unsupported(_))));
        assert!(parse_url("ftp://x").is_err());
    }

    #[test]
    fn parse_body_is_lenient() {
        assert!(parse_body(b"").is_empty());
        assert!(parse_body(b"<html>bad gateway</html>").is_empty());
        assert!(parse_body(b"[1,2]").is_empty());
        assert_eq!(parse_body(br#" {"ok":true} "#)["ok"], serde_json::Value::Bool(true));
    }

    fn serve_once(reply: &'static [u8]) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut got = Vec::new();
            let mut b = [0u8; 4096];
            // read the head and a body of the announced length
            loop {
                let n = s.read(&mut b).unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
                if let Some(i) = find(&got, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&got[..i]).to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if got.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            s.write_all(reply).unwrap();
            got
        });
        (port, handle)
    }

    #[test]
    fn content_length_round_trip() {
        let (port, h) = serve_once(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}");
        let r = Request::new(Method::Post, format!("http://127.0.0.1:{port}/api/x"))
            .header("Content-Type", "application/json")
            .json_body(&serde_json::json!({"a": 1}))
            .send()
            .unwrap();
        assert_eq!(r.status, 201);
        assert_eq!(r.json()["ok"], true);
        let sent = String::from_utf8(h.join().unwrap()).unwrap();
        assert!(sent.starts_with("POST /api/x HTTP/1.1\r\n"));
        assert!(sent.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
        assert!(sent.contains("Content-Length: 7\r\n"));
        assert!(sent.ends_with("{\"a\":1}"));
    }

    #[test]
    fn chunked_body() {
        let (port, _h) = serve_once(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"ok\r\n7;ext=1\r\n\":true}\r\n0\r\n\r\n");
        let r = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).send().unwrap();
        assert_eq!(r.body, b"{\"ok\":true}");
    }

    #[test]
    fn close_delimited_body() {
        let (port, _h) = serve_once(b"HTTP/1.0 404 Not Found\r\n\r\nnope");
        let r = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).send().unwrap();
        assert_eq!(r.status, 404);
        assert_eq!(r.body, b"nope");
    }

    #[test]
    fn refused_and_timeout() {
        // a port nobody listens on
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).timeout(Duration::from_secs(2)).send().unwrap_err();
        assert!(matches!(e, NetError::Transport(_) | NetError::Timeout), "{e:?}");
        // a server that accepts and never answers
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _keep = std::thread::spawn(move || {
            let c = listener.accept();
            std::thread::sleep(Duration::from_secs(3));
            drop(c);
        });
        let start = Instant::now();
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).timeout(Duration::from_millis(400)).send().unwrap_err();
        assert!(matches!(e, NetError::Timeout), "{e:?}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
