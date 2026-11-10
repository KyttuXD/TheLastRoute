//! The HTTP the start menu and the room client use: one request, one reply, no streaming.
//!
//! Native: each request runs on its own thread, so a server that does not answer never freezes a frame. Plain `http`
//! and `https` (rustls with the Mozilla roots, the same settings as last_route's net/http.rs). The server is
//! `LR_STORY_SERVER` when set, else the public site.
//!
//! Browser (wasm32): the page's synchronous XHR through the `lr_http_*` plugin in packaging/story-web/index.html. The
//! answer is ready when the request returns, so `Pending::poll` gives it on the first call. A path such as `/api/health`
//! names the page's own origin, which is why the browser's base is empty. A synchronous request cannot time out in the
//! page: a server that accepts the connection and never answers holds the tab until the browser gives up. The native
//! client has a 10 second limit per read and the menu's 20 second reply deadline; the browser has neither.
//!
//! Only the story server's API uses this module (paths start with `/api/`). Nothing here reads a game file.

use serde_json::Value;

/// The public story server, used by the native client unless `LR_STORY_SERVER` names another (for example
/// `http://127.0.0.1:8307` for a server on this machine).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const DEFAULT_SERVER: &str = "https://lastroutestory.games.kyttuslab.org";

/// The largest reply the client keeps. The story server's replies are small JSON; this only stops a runaway answer.
const MAX_REPLY_BYTES: usize = 1 << 20;

/// One answer from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    /// The body as JSON; `Null` when it is not JSON.
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// True for a 2xx answer whose JSON says `"ok": true`.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status) && self.json().get("ok") == Some(&Value::Bool(true))
    }

    /// The server's own message for a refused call (its `error` text), else the status.
    pub fn failure(&self) -> String {
        match self.json().get("error").and_then(Value::as_str) {
            Some(text) if !text.trim().is_empty() => text.to_string(),
            _ => format!("the server answered {}", self.status),
        }
    }
}

/// The base address of the server for this build: the native setting, or "" in the browser (relative paths).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn server_base() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("LR_STORY_SERVER")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_SERVER.to_string())
    }
}

/// A request that may still be on its way.
pub struct Pending {
    #[cfg(not(target_arch = "wasm32"))]
    rx: Option<std::sync::mpsc::Receiver<Result<Reply, String>>>,
    #[cfg(target_arch = "wasm32")]
    ready: Option<Result<Reply, String>>,
}

impl Pending {
    /// The answer, once it is in. Each answer is returned once; None while the request is still waiting.
    pub fn poll(&mut self) -> Option<Result<Reply, String>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let attempt = self.rx.as_ref()?.try_recv();
            match attempt {
                Ok(result) => {
                    self.rx = None;
                    Some(result)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    Some(Err("the request thread stopped".to_string()))
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.ready.take()
        }
    }
}

/// Starts a request. `path` is the API path (for example `/api/rooms/join`); `body`, when given, is sent as JSON.
pub fn send(method: &str, path: &str, body: Option<&Value>) -> Pending {
    let payload = body.map(|b| b.to_string().into_bytes());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let url = format!("{}{path}", server_base());
        let (tx, rx) = std::sync::mpsc::channel();
        let method = method.to_string();
        std::thread::spawn(move || {
            let result = native::exchange(&method, &url, payload.as_deref());
            let _ = tx.send(result);
        });
        Pending { rx: Some(rx) }
    }
    #[cfg(target_arch = "wasm32")]
    {
        Pending { ready: Some(web::exchange(method, path, payload.as_deref())) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Reply, MAX_REPLY_BYTES};
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(10);

    /// Sends one request and reads the whole reply (the request asks for `Connection: close`).
    pub fn exchange(method: &str, url: &str, body: Option<&[u8]>) -> Result<Reply, String> {
        let (https, host, port, path) = split_url(url)?;
        let addr = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| format!("could not look up {host}: {e}"))?
            .next()
            .ok_or_else(|| format!("no address for {host}"))?;
        let tcp = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| format!("could not reach {host}: {e}"))?;
        tcp.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
        tcp.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;

        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: lr_story\r\nAccept: application/json\r\nConnection: close\r\n"
        );
        if let Some(bytes) = body {
            request.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", bytes.len()));
        }
        request.push_str("\r\n");
        let mut wire = request.into_bytes();
        if let Some(bytes) = body {
            wire.extend_from_slice(bytes);
        }

        let raw = if https {
            let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| format!("bad server name {host}"))?;
            let connection = rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("TLS: {e}"))?;
            let mut stream = rustls::StreamOwned::new(connection, tcp);
            stream.write_all(&wire).map_err(|e| format!("could not send: {e}"))?;
            read_all(&mut stream)?
        } else {
            let mut stream = tcp;
            stream.write_all(&wire).map_err(|e| format!("could not send: {e}"))?;
            read_all(&mut stream)?
        };
        parse_reply(&raw)
    }

    /// Reads to the end of the reply. A TLS peer that closes without close_notify ends the reply too; the framing
    /// checks in `parse_reply` still catch a cut-off body.
    fn read_all(stream: &mut impl Read) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    // A Content-Length reply is whole once its body is in; a server that keeps the socket open must not hold it.
                    if is_complete(&out) {
                        break;
                    }
                    if out.len() > MAX_REPLY_BYTES {
                        return Err("the server's reply is too large".to_string());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(format!("the server did not finish answering: {e}")),
            }
        }
        Ok(out)
    }

    /// The scheme, host, port and path of a URL (`https://host[:port]/path`). IPv6 literals are not supported.
    pub fn split_url(url: &str) -> Result<(bool, String, u16, String), String> {
        let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(format!("not an http(s) address: {url}"));
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, "/".to_string()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| format!("bad port in {url}"))?),
            None => (authority.to_string(), if https { 443 } else { 80 }),
        };
        if host.is_empty() {
            return Err(format!("no host in {url}"));
        }
        Ok((https, host, port, path))
    }

    /// The rustls settings: the ring provider, TLS 1.2 and 1.3, and the Mozilla roots.
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

    /// True when a Content-Length reply has its whole body. A chunked reply is read to the end of the connection (the
    /// request asks for `Connection: close`), so it is never complete early.
    fn is_complete(raw: &[u8]) -> bool {
        let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
        let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
        if head.lines().any(|line| line.starts_with("transfer-encoding:") && line.contains("chunked")) {
            return false;
        }
        let length = head.lines().find_map(|line| line.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
        match length {
            Some(n) => raw.len() - (split + 4) >= n,
            None => false,
        }
    }

    /// Splits a raw HTTP/1.1 reply into its status and body, decoding chunked framing and checking Content-Length.
    pub fn parse_reply(raw: &[u8]) -> Result<Reply, String> {
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("the server's reply was cut short")?;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let body = &raw[split + 4..];
        let mut lines = head.split("\r\n");
        let status_line = lines.next().unwrap_or_default();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| format!("the server's reply has no status: {status_line}"))?;
        let mut chunked = false;
        let mut length: Option<usize> = None;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else { continue };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
                "content-length" => length = value.parse().ok(),
                _ => {}
            }
        }
        let body = if chunked {
            decode_chunked(body)?
        } else if let Some(n) = length {
            if body.len() < n {
                return Err("the server's reply was cut short".to_string());
            }
            body[..n].to_vec()
        } else {
            body.to_vec()
        };
        Ok(Reply { status, body })
    }

    /// The bytes of a chunked body.
    pub fn decode_chunked(mut data: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            let eol = data.windows(2).position(|w| w == b"\r\n").ok_or("a chunk has no size line")?;
            let size_text = String::from_utf8_lossy(&data[..eol]).to_string();
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| format!("bad chunk size {size_text:?}"))?;
            data = &data[eol + 2..];
            if size == 0 {
                return Ok(out);
            }
            let end = size.checked_add(2).ok_or("a chunk size is too large")?;
            if data.len() < end {
                return Err("a chunk was cut short".to_string());
            }
            if &data[size..end] != b"\r\n" {
                return Err("a chunk is not followed by CRLF".to_string());
            }
            out.extend_from_slice(&data[..size]);
            data = &data[end..];
            if out.len() > MAX_REPLY_BYTES {
                return Err("the server's reply is too large".to_string());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn urls_split_into_scheme_host_port_and_path() {
            assert_eq!(split_url("https://example.org/api/health").unwrap(), (true, "example.org".into(), 443, "/api/health".into()));
            assert_eq!(split_url("http://127.0.0.1:8307").unwrap(), (false, "127.0.0.1".into(), 8307, "/".into()));
            assert!(split_url("ftp://x").is_err());
            assert!(split_url("https://:99/x").is_err());
        }

        #[test]
        fn a_content_length_reply_is_read_whole() {
            let reply = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}").unwrap();
            assert_eq!(reply.status, 200);
            assert!(reply.ok());
            assert!(parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 40\r\n\r\n{\"ok\":true}").is_err(), "a cut-off body is an error");
        }

        #[test]
        fn a_chunked_reply_is_decoded() {
            let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"ok\r\n7;ext=1\r\n\":true}\r\n0\r\n\r\n";
            assert_eq!(parse_reply(raw).unwrap().body, b"{\"ok\":true}".to_vec());
        }

        #[test]
        fn a_bad_chunk_is_refused_and_a_content_length_reply_is_complete_early() {
            assert!(decode_chunked(b"3\r\nabcXY0\r\n\r\n").is_err(), "a chunk must end in CRLF");
            assert!(decode_chunked(b"ffffffffffffffff\r\nab\r\n").is_err(), "a huge chunk size must not overflow");
            assert!(is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"));
            assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}"));
            assert!(!is_complete(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"), "chunked replies end at EOF");
        }

        #[test]
        fn an_error_answer_keeps_its_message() {
            let reply = parse_reply(b"HTTP/1.1 404 Not Found\r\nContent-Length: 45\r\n\r\n{\"ok\":false,\"error\":\"No room with that code\"}").unwrap();
            assert!(!reply.ok());
            assert_eq!(reply.failure(), "No room with that code");
        }

        #[test]
        fn a_reply_without_a_status_is_refused() {
            assert!(parse_reply(b"nonsense\r\n\r\nbody").is_err());
        }

        #[test]
        fn an_empty_reply_is_an_error_not_a_panic() {
            assert!(parse_reply(b"").is_err());
            assert!(parse_reply(b"HTTP/1.1 200 OK\r\n").is_err(), "no blank line means the head was cut short");
        }

        #[test]
        fn a_reply_with_bad_json_is_not_ok_and_reads_as_null() {
            let reply = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n{not json").unwrap();
            assert_eq!(reply.json(), serde_json::Value::Null);
            assert!(!reply.ok(), "a 200 with an unreadable body is not ok");
            assert_eq!(reply.failure(), "the server answered 200");
            let empty = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").unwrap();
            assert!(!empty.ok());
        }

        #[test]
        fn a_4xx_without_json_names_its_status() {
            let reply = parse_reply(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 9\r\n\r\nForbidden").unwrap();
            assert_eq!(reply.status, 403);
            assert!(!reply.ok());
            assert_eq!(reply.failure(), "the server answered 403");
            let blank = parse_reply(b"HTTP/1.1 500 Oops\r\nContent-Length: 25\r\n\r\n{\"ok\":false,\"error\":\"  \"}").unwrap();
            assert_eq!(blank.failure(), "the server answered 500", "a blank error text falls back to the status");
        }

        #[test]
        fn a_status_that_is_not_a_number_is_refused() {
            assert!(parse_reply(b"HTTP/1.1 abc OK\r\n\r\n").is_err());
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Reply, MAX_REPLY_BYTES};

    extern "C" {
        fn lr_http_send(
            method: *const u8,
            method_len: u32,
            url: *const u8,
            url_len: u32,
            headers: *const u8,
            headers_len: u32,
            body: *const u8,
            body_len: u32,
            has_body: i32,
        ) -> u32;
        fn lr_http_status(id: u32) -> i32;
        fn lr_http_body(id: u32, dst: *mut u8, cap: u32) -> i32;
        fn lr_http_free(id: u32);
    }

    /// One synchronous request through the page. Status 0 (or below) means the page could not reach the server.
    pub fn exchange(method: &str, path: &str, body: Option<&[u8]>) -> Result<Reply, String> {
        let headers = if body.is_some() { "Accept: application/json\nContent-Type: application/json" } else { "Accept: application/json" };
        let (body_ptr, body_len) = match body {
            Some(bytes) => (bytes.as_ptr(), bytes.len() as u32),
            None => (std::ptr::null(), 0),
        };
        // SAFETY: every pointer and length names a live slice for the duration of the call; the page copies what it reads.
        let id = unsafe {
            lr_http_send(
                method.as_ptr(),
                method.len() as u32,
                path.as_ptr(),
                path.len() as u32,
                headers.as_ptr(),
                headers.len() as u32,
                body_ptr,
                body_len,
                body.is_some() as i32,
            )
        };
        // SAFETY: `id` is the result the call above returned; it is freed below, once its body has been read.
        let status = unsafe { lr_http_status(id) };
        let bytes = read_blob(|dst, cap| unsafe { lr_http_body(id, dst, cap) });
        unsafe { lr_http_free(id) };
        if status <= 0 {
            return Err("could not reach the story server".to_string());
        }
        if bytes.len() > MAX_REPLY_BYTES {
            return Err("the server's reply is too large".to_string());
        }
        Ok(Reply { status: status as u16, body: bytes })
    }

    /// Reads a page blob: the first try uses a 64 KB buffer; a longer blob is read again at its own length.
    fn read_blob(get: impl Fn(*mut u8, u32) -> i32) -> Vec<u8> {
        let mut buf = vec![0u8; 65536];
        let n = get(buf.as_mut_ptr(), buf.len() as u32);
        if n < 0 {
            return Vec::new();
        }
        let n = n as usize;
        if n <= buf.len() {
            buf.truncate(n);
            return buf;
        }
        let mut big = vec![0u8; n];
        let m = get(big.as_mut_ptr(), n as u32);
        big.truncate(m.max(0) as usize);
        big
    }
}
