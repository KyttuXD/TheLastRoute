//! Blocking HTTP/1.1 client for the account website, set with `LASTROUTE_ACCOUNT_UPSTREAM` (for example
//! `http://127.0.0.1:80`).
//!
//! When the upstream is set, the account routes, the admin roster, the session lookups and the game saves are answered
//! by the website, and the local accounts.json is neither read nor written, so accounts live in one store. Plain `http`
//! only (std has no TLS), one request per connection (`Connection: close`), a 5 second timeout and a 1 MB reply cap.

use crate::util::{log_warn, stringify, Reply};
use serde_json::{json, Value};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
pub const UPSTREAM_REPLY_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// `host:port`: the TCP target and the Host header.
    authority: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpstreamError {
    /// No connection, a timeout, or the connection closed early.
    Unavailable(String),
    /// Not one HTTP/1.x reply with a JSON body, or over the size cap.
    BadReply(String),
}

impl Upstream {
    /// Reads `http://host[:port]` (an optional trailing slash). `https` is refused: this client speaks plain HTTP only.
    pub fn parse(url: &str) -> Result<Upstream, String> {
        let rest = url.trim().strip_prefix("http://").ok_or("must start with http:// (https is not supported)")?;
        let authority = rest.trim_end_matches('/');
        if authority.is_empty() || authority.contains(|c: char| matches!(c, '/' | '@' | '?' | '#') || c.is_whitespace()) {
            return Err("must be http://host[:port] with no path".into());
        }
        let has_port = match authority.rfind(']') {
            Some(bracket) => authority[bracket..].contains(':'),
            None => authority.contains(':'),
        };
        let authority = if has_port { authority.to_string() } else { format!("{authority}:80") };
        Ok(Upstream { authority })
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// POSTs `body` as JSON to `path` and answers with the upstream's status and JSON body, unchanged. A connection
    /// failure answers 503 UPSTREAM_UNAVAILABLE and a bad reply 502 UPSTREAM_BAD_REPLY; the caller never falls back to a
    /// local store.
    pub fn call(&self, path: &str, body: &Value) -> Reply {
        match self.post_json(path, body) {
            Ok(reply) => reply,
            Err(UpstreamError::Unavailable(detail)) => {
                log_warn(&format!("account upstream {} unavailable for {path}: {detail}", self.authority));
                (503, json!({ "ok": false, "code": "UPSTREAM_UNAVAILABLE", "error": "Accounts are unavailable right now; try again shortly" }))
            }
            Err(UpstreamError::BadReply(detail)) => {
                log_warn(&format!("account upstream {} sent a bad reply for {path}: {detail}", self.authority));
                (502, json!({ "ok": false, "code": "UPSTREAM_BAD_REPLY", "error": "The account service sent an unusable reply" }))
            }
        }
    }

    pub fn post_json(&self, path: &str, body: &Value) -> Result<(u16, Value), UpstreamError> {
        let payload = stringify(body);
        let request = request_bytes(&self.authority, path, &payload);
        let raw = self.exchange(&request)?;
        parse_reply(&raw)
    }

    /// GETs `path` and answers with the status and the body bytes as they came (a PNG, for a skin). Connection
    /// failures are `UpstreamError::Unavailable`; the caller decides the reply.
    pub fn get_raw(&self, path: &str) -> Result<(u16, Vec<u8>), UpstreamError> {
        let request = format!("GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", self.authority).into_bytes();
        let raw = self.exchange(&request)?;
        parse_raw_reply(&raw)
    }

    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, UpstreamError> {
        let unavailable = |error: std::io::Error| UpstreamError::Unavailable(error.to_string());
        let mut stream = self.connect().map_err(unavailable)?;
        stream.set_read_timeout(Some(UPSTREAM_TIMEOUT)).map_err(unavailable)?;
        stream.set_write_timeout(Some(UPSTREAM_TIMEOUT)).map_err(unavailable)?;
        stream.write_all(request).map_err(unavailable)?;
        let mut raw = Vec::new();
        (&mut stream).take(UPSTREAM_REPLY_MAX_BYTES as u64 + 1).read_to_end(&mut raw).map_err(unavailable)?;
        Ok(raw)
    }

    fn connect(&self) -> std::io::Result<TcpStream> {
        let mut last = std::io::Error::new(ErrorKind::NotFound, "no address for the account upstream");
        for address in self.authority.as_str().to_socket_addrs()? {
            match TcpStream::connect_timeout(&address, UPSTREAM_TIMEOUT) {
                Ok(stream) => return Ok(stream),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}

fn request_bytes(authority: &str, path: &str, payload: &str) -> Vec<u8> {
    format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )
    .into_bytes()
}

/// Parses one HTTP/1.x reply into its status and JSON body. The body is read by Content-Length, by chunked
/// transfer-coding, or to the end of the reply.
pub fn parse_reply(raw: &[u8]) -> Result<(u16, Value), UpstreamError> {
    let (status, payload) = parse_raw_reply(raw)?;
    let value: Value = serde_json::from_slice(&payload).map_err(|_| UpstreamError::BadReply("the body is not JSON".into()))?;
    Ok((status, value))
}

/// [`parse_reply`] without the JSON step: the status and the body bytes.
pub fn parse_raw_reply(raw: &[u8]) -> Result<(u16, Vec<u8>), UpstreamError> {
    let bad = |detail: &str| UpstreamError::BadReply(detail.to_string());
    if raw.len() > UPSTREAM_REPLY_MAX_BYTES {
        return Err(bad("the reply is over 1 MB"));
    }
    let split = find(raw, b"\r\n\r\n").ok_or_else(|| bad("no end of headers"))?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| bad("the headers are not text"))?;
    let mut lines = head.split("\r\n");
    let status = parse_status(lines.next().unwrap_or(""))?;
    let (mut length, mut chunked) = (None, false);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = Some(value.parse::<usize>().map_err(|_| bad("bad Content-Length"))?),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    let body = &raw[split + 4..];
    let payload = if chunked {
        dechunk(body)?
    } else {
        match length {
            Some(n) if n <= body.len() => body[..n].to_vec(),
            Some(_) => return Err(bad("the body is shorter than Content-Length")),
            None => body.to_vec(),
        }
    };
    Ok((status, payload))
}

fn parse_status(line: &str) -> Result<u16, UpstreamError> {
    let mut parts = line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let code = parts.next().and_then(|code| code.parse::<u16>().ok()).filter(|code| (100..=599).contains(code));
    match code {
        Some(code) if version.starts_with("HTTP/1.") => Ok(code),
        _ => Err(UpstreamError::BadReply("the status line is not HTTP/1.x".into())),
    }
}

fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, UpstreamError> {
    let bad = || UpstreamError::BadReply("the chunked body is malformed".into());
    let mut out = Vec::new();
    loop {
        let line_end = find(rest, b"\r\n").ok_or_else(bad)?;
        let size_text = std::str::from_utf8(&rest[..line_end]).map_err(|_| bad())?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad())?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size + 2 {
            return Err(bad());
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// A stand-in website for tests: serves one connection per entry of `replies` (in order), each answering 200 with that
/// JSON text. The join handle returns `(path, parsed body)` for every request it got, in order.
#[cfg(test)]
pub(crate) mod test_website {
    use super::*;
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    pub(crate) fn fake_website(replies: Vec<&'static str>) -> (Upstream, JoinHandle<Vec<(String, Value)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let upstream = Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(UPSTREAM_TIMEOUT)).unwrap();
                let raw = read_request(&mut stream);
                let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
                let path = head.lines().next().unwrap_or("").split(' ').nth(1).unwrap_or("").to_string();
                seen.push((path, serde_json::from_str(body).unwrap_or(Value::Null)));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
            seen
        });
        (upstream, handle)
    }

    /// A stand-in website that answers one connection with `reply` (raw bytes, so a PNG can be served). The join handle
    /// returns the request line's path.
    pub(crate) fn fake_website_bytes(reply: Vec<u8>) -> (Upstream, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let upstream = Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(UPSTREAM_TIMEOUT)).unwrap();
            let raw = read_request(&mut stream);
            let path = raw.lines().next().unwrap_or("").split(' ').nth(1).unwrap_or("").to_string();
            let _ = stream.write_all(&reply);
            path
        });
        (upstream, handle)
    }

    /// Reads one request: the headers, then as many bytes as Content-Length names.
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            if let Some(split) = find(&raw, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..split]).to_string();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&raw).to_string()
    }
}
