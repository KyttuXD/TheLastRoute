//! A small GET-only HTTP/1.1 client that streams the body into a writer with progress, for downloads.
//!
//! Why not the game's `net/http.rs`: that one buffers whole bodies, is built around the game's `NetError` and the
//! browser build's bridge, and cannot report progress. This one is native-only, needs no game code, and the TLS
//! settings are the same (rustls with the `ring` provider and the Mozilla roots from `webpki-roots`, so it needs no
//! system certificate store: it works on an old macOS the same as on Windows).
//!
//! What it does: one request per connection (`Connection: close`, `Accept-Encoding: identity`), a connect timeout, a stall
//! timeout per read, an overall deadline, `Content-Length`, chunked and read-until-close bodies, a size cap, cancellation.
//! Redirects: a 3xx is an error, except for GitHub release downloads (`fetch::is_github_release_url`), which answer
//! 302 to `objects.githubusercontent.com`. For those, up to 3 hops are followed (301/302/303/307/308, absolute https
//! `Location` only, no userinfo, host `github.com` or `*.githubusercontent.com`, no loops). The size cap, timeouts,
//! cancel and the caller's checks apply to the final answer unchanged.
//! What it does not do: other redirects, proxies, resuming.
//! Plain `http://` is accepted here for loopback test servers; the policy for real sites (https only) is in `fetch.rs`.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The site could not be reached: no address, no connection, or no answer in time. A browser would not get through either.
    Offline(String),
    /// The server answered with a non-2xx status (the body was not read).
    Status(u16),
    /// The answer was not what HTTP allows, or TLS failed.
    Protocol(String),
    TooLarge,
    /// The writer failed (disk full, ...).
    Sink(String),
    Cancelled,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Offline(why) => write!(f, "could not reach the server ({why})"),
            HttpError::Status(code) => write!(f, "the server answered {code}"),
            HttpError::Protocol(why) => write!(f, "bad answer from the server ({why})"),
            HttpError::TooLarge => f.write_str("the download is larger than allowed"),
            HttpError::Sink(why) => write!(f, "could not write the download ({why})"),
            HttpError::Cancelled => f.write_str("cancelled"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub connect_timeout: Duration,
    /// How long one read may stall.
    pub stall_timeout: Duration,
    /// The whole request, connect to last byte.
    pub deadline: Duration,
    /// The most body bytes accepted.
    pub limit: u64,
    pub user_agent: String,
    /// Tests only: follow redirects between loopback servers (plain http allowed). Production code never sets this.
    pub follow_loopback_redirects: bool,
}

/// The most redirects followed for one request.
pub const MAX_REDIRECTS: usize = 3;

impl Options {
    pub fn new(limit: u64) -> Options {
        Options {
            connect_timeout: Duration::from_secs(5),
            stall_timeout: Duration::from_secs(20),
            deadline: Duration::from_secs(1800),
            limit,
            user_agent: format!("LastRouteLauncher/{}", env!("CARGO_PKG_VERSION")),
            follow_loopback_redirects: false,
        }
    }

    pub fn quick(limit: u64, seconds: u64) -> Options {
        Options { connect_timeout: Duration::from_secs(seconds), stall_timeout: Duration::from_secs(seconds), deadline: Duration::from_secs(seconds), ..Options::new(limit) }
    }
}

struct Url {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Result<Url, HttpError> {
    let bad = |why: &str| HttpError::Protocol(why.to_string());
    let lower = url.to_ascii_lowercase();
    let (https, rest) = if lower.starts_with("https://") {
        (true, &url[8..])
    } else if lower.starts_with("http://") {
        (false, &url[7..])
    } else {
        return Err(bad("only http and https addresses are supported"));
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || authority.contains('@') || path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(bad("invalid address"));
    }
    let default_port = if https { 443 } else { 80 };
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped.find(']').ok_or_else(|| bad("invalid address"))?;
        let port = match stripped[end + 1..].strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| bad("invalid port"))?,
            None => default_port,
        };
        (stripped[..end].to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| bad("invalid port"))?),
            None => (authority.to_string(), default_port),
        }
    };
    if host.is_empty() {
        return Err(bad("invalid address"));
    }
    Ok(Url { https, host, port, path })
}

/// True for `localhost` and loopback addresses.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The host part of an http(s) URL, for policy checks.
pub fn host_of(url: &str) -> Option<String> {
    parse_url(url).ok().map(|u| u.host)
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("ring supports the default TLS versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
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
            // Many servers close without a TLS close_notify. The body framing (length or chunks) is checked by the caller.
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

fn io_error(e: std::io::Error) -> HttpError {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => HttpError::Offline("timed out".into()),
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::NotConnected => {
            HttpError::Offline(e.to_string())
        }
        _ => HttpError::Protocol(e.to_string()),
    }
}

fn resolve(host: &str, port: u16, deadline: Instant) -> Result<Vec<SocketAddr>, HttpError> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    // The system resolver has no timeout and can hang: it runs on a helper thread that is abandoned at the deadline.
    let host = host.to_string();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send((host.as_str(), port).to_socket_addrs().map(|a| a.collect::<Vec<_>>()));
    });
    match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(addrs)) if !addrs.is_empty() => Ok(addrs),
        Ok(Ok(_)) => Err(HttpError::Offline("no address".into())),
        Ok(Err(e)) => Err(HttpError::Offline(format!("name lookup failed: {e}"))),
        Err(_) => Err(HttpError::Offline("name lookup timed out".into())),
    }
}

/// Which redirect targets a request may follow.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Follow {
    No,
    /// GitHub release downloads: https to github.com or *.githubusercontent.com.
    Github,
    /// Tests: loopback hosts only.
    Loopback,
}

fn redirect_target_allowed(mode: Follow, url: &str) -> bool {
    let Ok(t) = parse_url(url) else { return false };
    let host = t.host.to_ascii_lowercase();
    match mode {
        Follow::No => false,
        Follow::Github => t.https && t.port == 443 && (host == "github.com" || host.ends_with(".githubusercontent.com")),
        Follow::Loopback => is_loopback_host(&host),
    }
}

enum Step {
    Done(u64),
    Redirect(String),
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Downloads `url` into `sink`, calling `progress(bytes so far, total if known)` as data arrives. Returns the
/// number of body bytes written. A non-2xx status is an error before any body byte is read.
pub fn get_to(
    url: &str,
    options: &Options,
    cancel: Option<&AtomicBool>,
    sink: &mut dyn Write,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<u64, HttpError> {
    let first = parse_url(url)?;
    let mode = if crate::fetch::is_github_release_url(url) {
        Follow::Github
    } else if options.follow_loopback_redirects && is_loopback_host(&first.host) {
        Follow::Loopback
    } else {
        Follow::No
    };
    let mut current = url.to_string();
    let mut seen = vec![current.clone()];
    loop {
        // each hop gets the full time: a GitHub download is always a redirect plus a second TLS handshake
        let deadline = Instant::now() + options.deadline;
        match get_once(&current, options, cancel, sink, progress, deadline, mode)? {
            Step::Done(n) => return Ok(n),
            Step::Redirect(next) => {
                if seen.len() > MAX_REDIRECTS {
                    return Err(HttpError::Protocol("too many redirects".into()));
                }
                if !redirect_target_allowed(mode, &next) {
                    return Err(HttpError::Protocol("redirect to an address that is not allowed".into()));
                }
                if seen.contains(&next) {
                    return Err(HttpError::Protocol("redirect loop".into()));
                }
                seen.push(next.clone());
                current = next;
            }
        }
    }
}

fn get_once(
    url: &str,
    options: &Options,
    cancel: Option<&AtomicBool>,
    sink: &mut dyn Write,
    progress: &mut dyn FnMut(u64, Option<u64>),
    deadline: Instant,
    mode: Follow,
) -> Result<Step, HttpError> {
    let target = parse_url(url)?;
    let started = Instant::now();
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    if cancelled() {
        return Err(HttpError::Cancelled);
    }

    let addrs = resolve(&target.host, target.port, started + options.connect_timeout.max(Duration::from_millis(1)).min(options.deadline))?;
    let mut tcp = None;
    let mut last = HttpError::Offline("no address".into());
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, options.connect_timeout) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = io_error(e),
        }
    }
    let tcp = tcp.ok_or(last)?;
    let _ = tcp.set_nodelay(true);
    let mut stream = if target.https {
        let name = rustls::pki_types::ServerName::try_from(target.host.clone()).map_err(|_| HttpError::Protocol("invalid host name".into()))?;
        let connection = rustls::ClientConnection::new(tls_config(), name).map_err(|e| HttpError::Protocol(format!("TLS: {e}")))?;
        Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp)))
    } else {
        Stream::Plain(tcp)
    };

    let set_timeouts = |s: &Stream| -> Result<(), HttpError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(HttpError::Offline("timed out".into()));
        }
        let t = left.min(options.stall_timeout);
        s.tcp().set_read_timeout(Some(t)).map_err(io_error)?;
        s.tcp().set_write_timeout(Some(t)).map_err(io_error)?;
        Ok(())
    };

    let host_header = if target.host.contains(':') { format!("[{}]", target.host) } else { target.host.clone() };
    let default_port = if target.https { 443 } else { 80 };
    let host_header = if target.port == default_port { host_header } else { format!("{host_header}:{}", target.port) };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: {}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        target.path, options.user_agent
    );
    set_timeouts(&stream)?;
    stream.write_all(request.as_bytes()).map_err(io_error)?;
    stream.flush().map_err(io_error)?;

    // Headers.
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = vec![0u8; 64 * 1024];
    let header_end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err(HttpError::Protocol("response headers too large".into()));
        }
        set_timeouts(&stream)?;
        let n = stream.read(&mut chunk).map_err(io_error)?;
        if n == 0 {
            return Err(HttpError::Protocol("connection closed before a response".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut status_parts = lines.next().unwrap_or("").splitn(3, ' ');
    if !status_parts.next().unwrap_or("").starts_with("HTTP/1.") {
        return Err(HttpError::Protocol("not an HTTP response".into()));
    }
    let status: u16 = status_parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| HttpError::Protocol("bad status line".into()))?;
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    if mode != Follow::No && matches!(status, 301 | 302 | 303 | 307 | 308) {
        // Only an absolute address is taken: no relative Location, no scheme-less one.
        return match header("location") {
            Some(l) if !l.is_empty() => Ok(Step::Redirect(l.to_string())),
            _ => Err(HttpError::Protocol("redirect without a location".into())),
        };
    }
    if !(200..300).contains(&status) {
        return Err(HttpError::Status(status));
    }
    if status == 204 {
        return Ok(Step::Done(0));
    }
    if header("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity")) {
        return Err(HttpError::Protocol("the server compressed the body although identity was asked".into()));
    }
    let chunked = header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length: Option<u64> = if chunked {
        None
    } else {
        header("content-length").map(|v| v.parse::<u64>().map_err(|_| HttpError::Protocol("bad content-length".into()))).transpose()?
    };
    if length.is_some_and(|n| n > options.limit) {
        return Err(HttpError::TooLarge);
    }

    let mut written: u64 = 0;
    let mut emit = |data: &[u8], written: &mut u64, total: Option<u64>| -> Result<(), HttpError> {
        if *written + data.len() as u64 > options.limit {
            return Err(HttpError::TooLarge);
        }
        sink.write_all(data).map_err(|e| HttpError::Sink(e.to_string()))?;
        *written += data.len() as u64;
        progress(*written, total);
        Ok(())
    };
    let mut pending: Vec<u8> = buf[header_end + 4..].to_vec();
    drop(buf);

    if chunked {
        // size line, data, CRLF, ..., 0-size chunk. `pending` holds bytes read but not consumed yet.
        loop {
            let line_end = loop {
                if let Some(i) = find(&pending, b"\r\n") {
                    break i;
                }
                if pending.len() > 1024 {
                    return Err(HttpError::Protocol("bad chunk header".into()));
                }
                if cancelled() {
                    return Err(HttpError::Cancelled);
                }
                set_timeouts(&stream)?;
                let n = stream.read(&mut chunk).map_err(io_error)?;
                if n == 0 {
                    return Err(HttpError::Protocol("truncated chunked body".into()));
                }
                pending.extend_from_slice(&chunk[..n]);
            };
            let size_text = String::from_utf8_lossy(&pending[..line_end]).to_string();
            let size = u64::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| HttpError::Protocol("bad chunk size".into()))?;
            pending.drain(..line_end + 2);
            if size == 0 {
                return Ok(Step::Done(written));
            }
            let mut left = size;
            while left > 0 {
                if pending.is_empty() {
                    if cancelled() {
                        return Err(HttpError::Cancelled);
                    }
                    set_timeouts(&stream)?;
                    let n = stream.read(&mut chunk).map_err(io_error)?;
                    if n == 0 {
                        return Err(HttpError::Protocol("truncated chunked body".into()));
                    }
                    pending.extend_from_slice(&chunk[..n]);
                }
                let take = (pending.len() as u64).min(left) as usize;
                emit(&pending[..take], &mut written, None)?;
                pending.drain(..take);
                left -= take as u64;
            }
            // the CRLF after the chunk data
            while pending.len() < 2 {
                set_timeouts(&stream)?;
                let n = stream.read(&mut chunk).map_err(io_error)?;
                if n == 0 {
                    return Err(HttpError::Protocol("truncated chunked body".into()));
                }
                pending.extend_from_slice(&chunk[..n]);
            }
            if &pending[..2] != b"\r\n" {
                return Err(HttpError::Protocol("bad chunk framing".into()));
            }
            pending.drain(..2);
        }
    }

    // Content-Length, or until the server closes.
    let mut take_first = pending.len();
    if let Some(total) = length {
        take_first = take_first.min(total as usize);
    }
    if take_first > 0 {
        emit(&pending[..take_first], &mut written, length)?;
    }
    drop(pending);
    loop {
        if length.is_some_and(|total| written >= total) {
            return Ok(Step::Done(written));
        }
        if cancelled() {
            return Err(HttpError::Cancelled);
        }
        set_timeouts(&stream)?;
        let n = stream.read(&mut chunk).map_err(io_error)?;
        if n == 0 {
            return match length {
                Some(_) => Err(HttpError::Protocol("the body ended early".into())),
                None => Ok(Step::Done(written)),
            };
        }
        let mut data = &chunk[..n];
        if let Some(total) = length {
            data = &data[..data.len().min((total - written) as usize)];
        }
        emit(data, &mut written, length)?;
    }
}

/// Convenience: the whole body in memory (for latest.json, manifest and signature).
pub fn get_bytes(url: &str, options: &Options, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    get_to(url, options, cancel, &mut out, &mut |_, _| {})?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsite::TestSite;

    fn opts(limit: u64) -> Options {
        Options { follow_loopback_redirects: true, ..Options::quick(limit, 5) }
    }

    #[test]
    fn follows_one_redirect() {
        let site = TestSite::start();
        site.put("real.bin", b"hello".to_vec());
        site.redirect("start", &format!("{}/real.bin", site.url));
        assert_eq!(get_bytes(&format!("{}/start", site.url), &opts(100), None).unwrap(), b"hello");
        assert_eq!(site.hits(), vec!["start", "real.bin"]);
    }

    #[test]
    fn production_options_never_follow() {
        let site = TestSite::start();
        site.put("real.bin", b"hello".to_vec());
        site.redirect("start", &format!("{}/real.bin", site.url));
        let plain = Options::quick(100, 5);
        assert_eq!(get_bytes(&format!("{}/start", site.url), &plain, None), Err(HttpError::Status(302)));
        assert_eq!(site.hits(), vec!["start"]);
    }

    #[test]
    fn follows_up_to_three_hops_not_four() {
        let site = TestSite::start();
        site.put("end", b"ok".to_vec());
        site.redirect("a", &format!("{}/b", site.url));
        site.redirect("b", &format!("{}/c", site.url));
        site.redirect("c", &format!("{}/end", site.url));
        assert_eq!(get_bytes(&format!("{}/a", site.url), &opts(100), None).unwrap(), b"ok");
        site.redirect("c", &format!("{}/d", site.url));
        site.redirect("d", &format!("{}/end", site.url));
        assert!(matches!(get_bytes(&format!("{}/a", site.url), &opts(100), None), Err(HttpError::Protocol(_))));
    }

    #[test]
    fn refuses_loops() {
        let site = TestSite::start();
        site.redirect("a", &format!("{}/b", site.url));
        site.redirect("b", &format!("{}/a", site.url));
        let e = get_bytes(&format!("{}/a", site.url), &opts(100), None).unwrap_err();
        assert!(matches!(&e, HttpError::Protocol(w) if w.contains("loop")), "{e:?}");
        site.redirect("self", &format!("{}/self", site.url));
        assert!(get_bytes(&format!("{}/self", site.url), &opts(100), None).is_err());
    }

    #[test]
    fn refuses_other_hosts_userinfo_and_relative() {
        let site = TestSite::start();
        for (i, target) in [
            "http://example.com/x".to_string(),
            "https://github.com/x".to_string(),
            format!("http://user:pw@127.0.0.1:{}/real", site.url.rsplit(':').next().unwrap()),
            "/real".to_string(),
            "real".to_string(),
            "ftp://127.0.0.1/x".to_string(),
        ]
        .iter()
        .enumerate()
        {
            let path = format!("r{i}");
            site.redirect(&path, target);
            let e = get_bytes(&format!("{}/{path}", site.url), &opts(100), None);
            assert!(matches!(e, Err(HttpError::Protocol(_))), "{target}: {e:?}");
        }
    }

    #[test]
    fn github_mode_allows_only_github_https_hosts() {
        let ok = |u: &str| redirect_target_allowed(Follow::Github, u);
        assert!(ok("https://github.com/a/b"));
        assert!(ok("https://objects.githubusercontent.com/x?y=1"));
        assert!(ok("https://release-assets.githubusercontent.com/x"));
        assert!(!ok("http://github.com/a"));
        assert!(!ok("http://objects.githubusercontent.com/x"));
        assert!(!ok("https://evilgithubusercontent.com/x"));
        assert!(!ok("https://githubusercontent.com.evil.org/x"));
        assert!(!ok("https://github.com.evil.org/x"));
        assert!(!ok("https://user@github.com/x"));
        assert!(!ok("https://github.com:8443/x"));
        assert!(!ok("https://127.0.0.1/x"));
        assert!(!ok("/relative"));
        assert!(!redirect_target_allowed(Follow::No, "https://github.com/a"));
        assert!(!redirect_target_allowed(Follow::Loopback, "https://github.com/a"));
        assert!(redirect_target_allowed(Follow::Loopback, "http://127.0.0.1:9/a"));
    }

    #[test]
    fn size_cap_applies_after_a_redirect() {
        let site = TestSite::start();
        site.put("big.bin", vec![7u8; 500]);
        site.redirect("start", &format!("{}/big.bin", site.url));
        assert_eq!(get_bytes(&format!("{}/start", site.url), &opts(100), None), Err(HttpError::TooLarge));
        site.fault("big.bin", crate::testsite::Fault::Chunked);
        assert_eq!(get_bytes(&format!("{}/start", site.url), &opts(100), None), Err(HttpError::TooLarge));
        assert_eq!(get_bytes(&format!("{}/start", site.url), &opts(500), None).unwrap().len(), 500);
    }

    #[test]
    fn error_status_after_a_redirect_is_reported() {
        let site = TestSite::start();
        site.redirect("start", &format!("{}/missing", site.url));
        assert_eq!(get_bytes(&format!("{}/start", site.url), &opts(100), None), Err(HttpError::Status(404)));
    }
}
