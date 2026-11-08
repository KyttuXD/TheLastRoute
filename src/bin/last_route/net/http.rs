//! A minimal HTTP/1.1 client over `std::net::TcpStream`, standing in for Godot's `HTTPRequest`
//! node that `game/autoload/backend_client.gd` and `run_saves.gd` use. The Last Route backend
//! (`backend/server.mjs`) is plain HTTP on its Pterodactyl allocation, so the backend calls need no TLS;
//! downloads (player skins from an https link, from Mojang and from `skins.kyttuslab.org`) do, so
//! `https://` origins are served over rustls.
//!
//! What it does: one request per connection (`Connection: close`), connect / read / write
//! timeouts plus an overall deadline, `Content-Length`, `Transfer-Encoding: chunked` and
//! read-until-close bodies, and a response size cap.
//!
//! HTTPS (see `docs/NET_HTTPS.md` for the reasoning): rustls 0.23 with the `ring` crypto provider and the
//! Mozilla root list from `webpki-roots`. rustls is pure Rust and `ring` carries its own C and assembly, so
//! the build needs no system OpenSSL and no system certificate store on macOS (arm64 and x86_64), Windows
//! or Linux, and a download behaves the same on all of them. TLS 1.2 and 1.3 only, the server name is always
//! checked against the certificate, there is no way to switch verification off.
//!
//! Redirects are off unless a request asks for them with [`Request::redirects`] (a backend never redirects
//! and a 3xx must reach the caller unchanged). A download that asks for them gets at most the number of
//! hops it names (never more than [`MAX_REDIRECTS`]), and every hop is checked again before anything is sent
//! to it: the scheme must be https (plain http only for a loopback host, and never after an https hop: no
//! downgrade), the caller's [`HopGuard`] must accept the new address, a non-public address is refused when the
//! request is `public_only`, and the headers that were sent are cut down to `Accept` and `User-Agent` as soon
//! as the host, port or scheme changes, so a session token or any other credential is never forwarded to
//! another site. The redirect's own body is never read. The size cap, the content-type list and the timeout
//! (one deadline for the whole chain) apply to the final answer.
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
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use crate::shared::clock::Instant;
use std::time::Duration;

/// Godot's HTTPRequest has no timeout by default (`timeout = 0`), which is fine for a node that
/// can be cancelled; a background thread cannot be, so every request here gets one. PORT
/// DEVIATION: requests the original left unbounded use this.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// The biggest response accepted (the largest real one is a 64 KB cloud save plus framing).
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// The most redirect hops any request may be given, whatever the caller asks for.
pub const MAX_REDIRECTS: u8 = 3;
/// The biggest header block accepted from a server.
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// Slack past the size cap while reading (the end of the headers and chunk framing).
const FRAMING_SLACK: usize = 64 * 1024;
/// The headers that survive a hop to another origin. Everything else (a session token, Authorization, Cookie) stays behind.
const CROSS_ORIGIN_HEADERS: [&str; 2] = ["accept", "user-agent"];

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

/// A check run on every redirect target before it is followed: `Some(reason)` refuses the hop.
#[derive(Clone)]
pub struct HopGuard(pub Arc<dyn Fn(&str) -> Option<String> + Send + Sync>);

impl std::fmt::Debug for HopGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HopGuard")
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
    /// How many redirect hops are followed (0, the default, hands a 3xx to the caller). At most [`MAX_REDIRECTS`].
    pub redirects: u8,
    /// The check every redirect target passes first (the first address is the caller's own to check).
    pub hop_guard: Option<HopGuard>,
    /// True for downloads from an address a player typed: loopback, private, link-local and other non-public
    /// addresses are refused after the name is resolved, on every hop, so a public name cannot point at the LAN.
    pub public_only: bool,
    /// The content types a 2xx answer may have (the part before `;`, lower case). Empty: any. An answer without
    /// a Content-Type header passes; one with another type is refused before its body is read.
    pub content_types: Vec<String>,
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Request {
        Request {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: DEFAULT_TIMEOUT,
            limit: MAX_RESPONSE_BYTES,
            redirects: 0,
            hop_guard: None,
            public_only: false,
            content_types: Vec::new(),
        }
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

    /// Follow up to `hops` redirects (capped at [`MAX_REDIRECTS`]), each one re-checked.
    pub fn redirects(mut self, hops: u8) -> Request {
        self.redirects = hops.min(MAX_REDIRECTS);
        self
    }

    /// The check every redirect target passes before it is followed (`Some(reason)` refuses it).
    pub fn hop_guard(mut self, guard: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Request {
        self.hop_guard = Some(HopGuard(Arc::new(guard)));
        self
    }

    /// Refuse non-public addresses on every hop (see the field).
    pub fn public_only(mut self) -> Request {
        self.public_only = true;
        self
    }

    /// The content types a 2xx answer may have (see the field).
    pub fn content_types(mut self, types: &[&str]) -> Request {
        self.content_types = types.iter().map(|t| t.to_ascii_lowercase()).collect();
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

/// `http(s)://host[:port]/path?query` split into its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    /// True for `https://`.
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    /// True when the port is the scheme's default (80 or 443).
    fn default_port(&self) -> bool {
        self.port == if self.https { 443 } else { 80 }
    }

    /// The URL as text again (the host in brackets when it is an IPv6 literal).
    pub fn to_text(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let port = if self.default_port() { String::new() } else { format!(":{}", self.port) };
        format!("{}://{host}{port}{}", if self.https { "https" } else { "http" }, self.path)
    }

    /// The same scheme, host and port: the only case a redirect may keep the caller's headers.
    fn same_origin(&self, other: &Url) -> bool {
        self.https == other.https && self.port == other.port && self.host.eq_ignore_ascii_case(&other.host)
    }
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
    // A fragment is never sent to a server.
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
        // [ipv6]:port
        let end = stripped.find(']').ok_or_else(invalid)?;
        let host = &stripped[..end];
        let after = &stripped[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| NetError::Unsupported("INVALID SERVER PORT".into()))?,
            None if after.is_empty() => default_port,
            None => return Err(invalid()),
        };
        (host.to_string(), port)
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

/// The address a `Location` header names, resolved against the URL it came from: an absolute URL, `//host/path`,
/// `/path` or a path relative to the current one.
pub fn resolve_location(base: &Url, location: &str) -> Result<String, NetError> {
    let location = location.trim();
    if location.is_empty() || location.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(NetError::Protocol("BAD REDIRECT".into()));
    }
    let lower = location.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Ok(location.to_string());
    }
    let scheme = if base.https { "https" } else { "http" };
    if location.starts_with("//") {
        return Ok(format!("{scheme}:{location}"));
    }
    // Anything else with a scheme (ftp:, data:, javascript:) is not a path.
    if let Some(colon) = location.find(':') {
        if !location[..colon].contains(['/', '?', '#']) {
            return Err(NetError::Unsupported("REDIRECT TO AN UNSUPPORTED ADDRESS".into()));
        }
    }
    let origin = Url { path: String::new(), ..base.clone() }.to_text();
    if location.starts_with('/') {
        return Ok(format!("{origin}{location}"));
    }
    let directory = base.path.split('?').next().unwrap_or("/");
    let directory = &directory[..directory.rfind('/').map_or(0, |i| i + 1)];
    Ok(format!("{origin}{directory}{location}"))
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

/// True for an address a download may connect to: not this machine, not a private or link-local network, not a
/// reserved range. An IPv4 address inside an IPv6 one is judged as the IPv4 address, in every form that carries one:
/// v4-mapped `::ffff:a.b.c.d`, v4-compatible `::a.b.c.d`, NAT64 `64:ff9b::/96`, 6to4 `2002::/16` and Teredo
/// `2001::/32` (both its server and its client address). The NAT64 local-use prefix `64:ff9b:1::/48` is refused outright.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => is_public_v4(a),
        IpAddr::V6(a) => match a.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(a) && embedded_v4(a).into_iter().all(is_public_v4),
        },
    }
}

/// The IPv4 addresses an IPv6 address carries inside it (see [`is_public_ip`]); none for a plain IPv6 address.
fn embedded_v4(a: Ipv6Addr) -> Vec<Ipv4Addr> {
    let s = a.segments();
    let v4 = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if s[..6] == [0, 0, 0, 0, 0, 0] {
        return vec![v4(s[6], s[7])]; // v4-compatible, ::a.b.c.d
    }
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return vec![v4(s[6], s[7])]; // NAT64, 64:ff9b::/96
    }
    if s[0] == 0x2002 {
        return vec![v4(s[1], s[2])]; // 6to4
    }
    if s[0] == 0x2001 && s[1] == 0 {
        return vec![v4(s[2], s[3]), v4(!s[6], !s[7])]; // Teredo: the server, and the client (obfuscated by NOT)
    }
    Vec::new()
}

fn is_public_v4(a: Ipv4Addr) -> bool {
    let o = a.octets();
    !(a.is_loopback()
        || a.is_private()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || a.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (64..128).contains(&o[1])) // carrier-grade NAT, 100.64.0.0/10
        || o[0] >= 240) // reserved, 240.0.0.0/4
}

fn is_public_v6(a: Ipv6Addr) -> bool {
    let segments = a.segments();
    !(a.is_loopback()
        || a.is_unspecified()
        || a.is_multicast()
        || segments[0] & 0xfe00 == 0xfc00 // unique local, fc00::/7
        || segments[0] & 0xffc0 == 0xfe80 // link-local, fe80::/10
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
        || (segments[0] == 0x64 && segments[1] == 0xff9b && segments[2] == 1)) // NAT64 local use, 64:ff9b:1::/48
}

/// True for `localhost` and the loopback addresses: the only place plain http may be used for a redirect.
/// Whether a plain `http://` request to `url` may be sent: loopback hosts always (a developer's own server, the tests);
/// the TEST instance in `config::TEST_BASE_URLS` only in a debug build (the documented exception, until it moves to
/// https); every other host needs https. Release builds refuse the rest.
pub fn plain_http_allowed(url: &Url, debug_build: bool) -> bool {
    if is_loopback_host(&url.host) {
        return true;
    }
    debug_build
        && super::config::TEST_BASE_URLS
            .iter()
            .any(|t| parse_url(t).is_ok_and(|u| !u.https && u.host.eq_ignore_ascii_case(&url.host) && u.port == url.port))
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Resolves `host` within the request deadline. The system resolver has no timeout of its own and can hang, so it runs on a
/// helper thread and the caller waits only until the deadline (the thread is left to finish on its own). An IP literal needs
/// no lookup.
fn resolve(host: &str, port: u16, deadline: Instant) -> Result<Vec<SocketAddr>, NetError> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let host = host.to_string();
    resolve_with(move || (host.as_str(), port).to_socket_addrs().map(|a| a.collect()), deadline)
}

fn resolve_with(lookup: impl FnOnce() -> std::io::Result<Vec<SocketAddr>> + Send + 'static, deadline: Instant) -> Result<Vec<SocketAddr>, NetError> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(lookup());
    });
    match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(result) => result.map_err(transport),
        Err(_) => Err(NetError::Timeout),
    }
}

fn connect(url: &Url, deadline: Instant, public_only: bool) -> Result<TcpStream, NetError> {
    let mut addrs: Vec<SocketAddr> = resolve(&url.host, url.port, deadline)?;
    if public_only {
        let before = addrs.len();
        addrs.retain(|a| is_public_ip(a.ip()));
        if addrs.is_empty() && before > 0 {
            return Err(NetError::Refused("THAT ADDRESS IS NOT PUBLIC".into()));
        }
    }
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

// ---- TLS ----------------------------------------------------------------------------------------------------------

/// The TLS settings a request carries: the rustls client config on native. The browser build (wasm32) has no TLS
/// and no sockets; [`send`] refuses every request there before any of this runs.
#[cfg(not(target_arch = "wasm32"))]
type Tls = rustls::ClientConfig;
#[cfg(target_arch = "wasm32")]
type Tls = ();

/// The rustls client settings every https request uses: the `ring` provider, TLS 1.2 and 1.3, the Mozilla roots.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn tls_config() -> Arc<Tls> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(client_config(roots))
        })
        .clone()
}

#[cfg(target_arch = "wasm32")]
fn tls_config() -> Arc<Tls> {
    Arc::new(())
}

#[cfg(not(target_arch = "wasm32"))]
fn client_config(roots: rustls::RootCertStore) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// A connection, plain or inside TLS.
enum Stream {
    Plain(TcpStream),
    #[cfg(not(target_arch = "wasm32"))]
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn open(tcp: TcpStream, url: &Url, tls: &Arc<Tls>) -> Result<Stream, NetError> {
        if !url.https {
            // TODO 1075: a release build speaks plain http only to a loopback host (a debug build also to the TEST instance)
            if !plain_http_allowed(url, cfg!(debug_assertions)) {
                return Err(NetError::Unsupported(super::api::error_texts().text("http_insecure")));
            }
            return Ok(Stream::Plain(tcp));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (tcp, tls);
            return Err(NetError::Unsupported("HTTPS IS NOT AVAILABLE IN THE BROWSER BUILD".into()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let name = rustls::pki_types::ServerName::try_from(url.host.clone()).map_err(|_| NetError::Unsupported("INVALID SERVER ADDRESS".into()))?;
            let connection = rustls::ClientConnection::new(tls.clone(), name).map_err(|e| NetError::Transport(format!("TLS: {e}")))?;
            Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp))))
        }
    }

    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            #[cfg(not(target_arch = "wasm32"))]
            Stream::Tls(s) => &s.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            // A server that closes the socket without a TLS close_notify is common; the body framing (length or
            // chunks) is checked by the caller, so a cut-off body is still caught.
            #[cfg(not(target_arch = "wasm32"))]
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
            #[cfg(not(target_arch = "wasm32"))]
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            #[cfg(not(target_arch = "wasm32"))]
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// Sends one request and reads the whole response (blocking; run it on a worker thread from
/// the frame loop, see [`super::Pending`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn send(req: &Request) -> Result<Response, NetError> {
    send_with(req, &tls_config())
}

/// The browser build has no sockets of its own: a request goes through the page (`XMLHttpRequest`, see
/// [`super::web_bridge`] and dist/web/index.html), which applies the browser's TLS and CORS rules. The call blocks the
/// page until the answer comes back (the browser build runs requests inline, see [`crate::shared::task::detach`]).
/// Redirects are followed by the browser; the redirect guards and the 2xx content-type check are native-only.
#[cfg(target_arch = "wasm32")]
pub fn send(req: &Request) -> Result<Response, NetError> {
    let reply = super::web_bridge::http_request(req.method.as_str(), &req.url, &req.headers, req.body.as_deref());
    if reply.status == 0 {
        return Err(NetError::Transport("THE REQUEST DID NOT REACH THE SERVER".into()));
    }
    if reply.body.len() > req.limit {
        return Err(NetError::Protocol("THE SERVER SENT TOO MUCH DATA".into()));
    }
    let headers = reply
        .headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    Ok(Response { status: reply.status, headers, body: reply.body })
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// [`send`] with the TLS settings given (tests trust their own certificate this way).
#[cfg(not(target_arch = "wasm32"))]
fn send_with(req: &Request, tls: &Arc<Tls>) -> Result<Response, NetError> {
    let deadline = Instant::now() + req.timeout;
    let mut url = parse_url(&req.url)?;
    let (mut method, mut body, mut headers) = (req.method, req.body.clone(), req.headers.clone());
    let follow = req.redirects.min(MAX_REDIRECTS);
    let mut hops = 0u8;
    loop {
        let response = exchange(req, &url, method, &headers, body.as_deref(), deadline, follow > 0, tls)?;
        if follow == 0 || !is_redirect(response.status) {
            return Ok(response);
        }
        if hops >= follow {
            return Err(NetError::Protocol("TOO MANY REDIRECTS".into()));
        }
        hops += 1;
        let location = response.header("location").ok_or_else(|| NetError::Protocol("REDIRECT WITHOUT A LOCATION".into()))?;
        let next_text = resolve_location(&url, location)?;
        if let Some(guard) = &req.hop_guard {
            if let Some(why) = (guard.0)(&next_text) {
                return Err(NetError::Refused(why));
            }
        }
        let next = parse_url(&next_text)?;
        if !next.https && (url.https || !is_loopback_host(&next.host)) {
            return Err(NetError::Refused("A REDIRECT TO PLAIN HTTP IS REFUSED".into()));
        }
        // 303 always becomes a GET; 301 and 302 do too (what every client does); 307 and 308 keep the method.
        let keeps_method = matches!(response.status, 307 | 308);
        let origin_changed = !url.same_origin(&next);
        if !keeps_method {
            method = Method::Get;
            body = None;
        } else if origin_changed && body.is_some() {
            return Err(NetError::Refused("A REDIRECT WITH A BODY TO ANOTHER SITE IS REFUSED".into()));
        }
        if origin_changed {
            headers.retain(|(k, _)| CROSS_ORIGIN_HEADERS.iter().any(|keep| k.eq_ignore_ascii_case(keep)));
        }
        url = next;
    }
}

/// One request on one connection. With `stop_at_redirect` a 3xx answer comes back with an empty body: it is never read.
#[allow(clippy::too_many_arguments)]
fn exchange(
    req: &Request,
    url: &Url,
    method: Method,
    request_headers: &[(String, String)],
    request_body: Option<&[u8]>,
    deadline: Instant,
    stop_at_redirect: bool,
    tls: &Arc<Tls>,
) -> Result<Response, NetError> {
    let tcp = connect(url, deadline, req.public_only)?;
    let _ = tcp.set_nodelay(true);
    let mut stream = Stream::open(tcp, url, tls)?;
    let host_header = if url.host.contains(':') { format!("[{}]", url.host) } else { url.host.clone() };
    let host_header = if url.default_port() { host_header } else { format!("{host_header}:{}", url.port) };
    let mut head = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", method.as_str(), url.path, host_header);
    let has = |name: &str| request_headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if !has("user-agent") {
        head.push_str(&format!("User-Agent: {}\r\n", user_agent()));
    }
    if !has("accept") {
        head.push_str("Accept: */*\r\n");
    }
    for (k, v) in request_headers {
        if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
            return Err(NetError::Protocol("INVALID HEADER".into()));
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(body) = request_body {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    } else if matches!(method, Method::Post | Method::Put) {
        head.push_str("Content-Length: 0\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
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
    stream.write_all(head.as_bytes()).map_err(transport)?;
    if let Some(body) = request_body {
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
        if buf.len() > MAX_HEADER_BYTES {
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
    let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    if stop_at_redirect && is_redirect(status) {
        return Ok(Response { status, headers, body: Vec::new() });
    }
    // The type is judged before a single body byte is read.
    if (200..300).contains(&status) && !req.content_types.is_empty() {
        if let Some(kind) = header("content-type") {
            let kind = kind.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            if !req.content_types.contains(&kind) {
                return Err(NetError::Protocol("UNEXPECTED CONTENT TYPE".into()));
            }
        }
    }
    let mut rest = buf[header_end + 4..].to_vec();
    let no_body = status == 204 || status == 304 || (100..200).contains(&status);
    let mut read_more = |rest: &mut Vec<u8>| -> Result<bool, NetError> {
        set_timeouts(&stream)?;
        let n = stream.read(&mut chunk).map_err(transport)?;
        if n == 0 {
            return Ok(false);
        }
        rest.extend_from_slice(&chunk[..n]);
        if rest.len() > req.limit + FRAMING_SLACK {
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
            if out.len().saturating_add(size) > req.limit {
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
        assert_eq!(u, Url { https: false, host: "lastroute.kyttuslab.org".into(), port: 8105, path: "/api/health".into() });
        let u = parse_url("http://127.0.0.1:18105").unwrap();
        assert_eq!(u.path, "/");
        assert_eq!(parse_url("http://example.org/x").unwrap().port, 80);
        assert!(parse_url("https://kyttuslab.org/godot2-api").unwrap().https);
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

    // ---- https, redirects and the download limits ----------------------------------------------------------------

    #[test]
    fn parses_https_urls_with_their_default_port() {
        let u = parse_url("https://skins.kyttuslab.org/abc.png").unwrap();
        assert_eq!(u, Url { https: true, host: "skins.kyttuslab.org".into(), port: 443, path: "/abc.png".into() });
        assert_eq!(parse_url("HTTPS://x.org:8443/a?b=1#frag").unwrap(), Url { https: true, host: "x.org".into(), port: 8443, path: "/a?b=1".into() });
        assert_eq!(parse_url("https://x.org?q=1").unwrap().path, "/?q=1");
        assert_eq!(parse_url("https://x.org/a").unwrap().to_text(), "https://x.org/a");
        assert_eq!(parse_url("http://x.org:81/a").unwrap().to_text(), "http://x.org:81/a");
        assert!(parse_url("https://user@x.org/").is_err());
        assert!(parse_url("https://x.org/a b").is_err());
        assert!(parse_url("https:///x").is_err());
    }

    #[test]
    fn locations_resolve_against_the_url_they_came_from() {
        let base = parse_url("https://a.example/dir/file.png?x=1").unwrap();
        assert_eq!(resolve_location(&base, "https://b.example/z").unwrap(), "https://b.example/z");
        assert_eq!(resolve_location(&base, "//c.example/z").unwrap(), "https://c.example/z");
        assert_eq!(resolve_location(&base, "/root.png").unwrap(), "https://a.example/root.png");
        assert_eq!(resolve_location(&base, "other.png").unwrap(), "https://a.example/dir/other.png");
        assert_eq!(resolve_location(&base, "?y=2").unwrap(), "https://a.example/dir/?y=2");
        assert!(resolve_location(&base, "ftp://a.example/x").is_err());
        assert!(resolve_location(&base, "data:image/png;base64,AAAA").is_err());
        assert!(resolve_location(&base, "").is_err());
        assert!(resolve_location(&base, "/a b").is_err());
    }

    #[test]
    fn only_public_addresses_are_public() {
        let public = |s: &str| is_public_ip(s.parse().unwrap());
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(public(ip), "{ip}");
        }
        for ip in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.1.1", "0.0.0.0", "100.64.0.1", "224.0.0.1",
            "255.255.255.255", "240.0.0.1", "192.0.2.1", "::1", "::", "fe80::1", "fc00::1", "fd12:3456::1", "ff02::1", "::ffff:127.0.0.1",
            "::ffff:10.0.0.1", "2001:db8::1",
        ] {
            assert!(!public(ip), "{ip}");
        }
    }

    #[test]
    fn ipv6_forms_that_carry_an_ipv4_address_are_judged_by_it() {
        let public = |s: &str| is_public_ip(s.parse().unwrap());
        // NAT64 64:ff9b::/96 of loopback, private, link-local and carrier-grade addresses
        for ip in ["64:ff9b::7f00:1", "64:ff9b::a00:1", "64:ff9b::c0a8:101", "64:ff9b::a9fe:1", "64:ff9b::6440:1", "64:ff9b::127.0.0.1"] {
            assert!(!public(ip), "NAT64 {ip}");
        }
        assert!(public("64:ff9b::808:808"), "NAT64 of 8.8.8.8 is public");
        assert!(!public("64:ff9b:1::1"), "the NAT64 local-use prefix is refused outright");
        // 6to4 2002:AABB:CCDD::/48 carries AA.BB.CC.DD
        for ip in ["2002:7f00:1::1", "2002:a00:1::", "2002:c0a8:101::5", "2002:a9fe:1::1"] {
            assert!(!public(ip), "6to4 {ip}");
        }
        assert!(public("2002:808:808::1"), "6to4 of 8.8.8.8 is public");
        // IPv4-compatible ::a.b.c.d
        for ip in ["::7f00:1", "::a00:1", "::c0a8:101", "::127.0.0.1", "::10.1.2.3"] {
            assert!(!public(ip), "v4-compatible {ip}");
        }
        // Teredo 2001:0:SSSS:SSSS:flags:port:CCCC:CCCC, the client address is the last 32 bits inverted
        assert!(!public("2001:0:7f00:1::1"), "Teredo server 127.0.0.1");
        assert!(!public("2001:0:808:808:0:0:80ff:fffe"), "Teredo client 127.0.0.1 (inverted) behind a public server");
        assert!(public("2001:0:808:808:0:0:f7f7:f7f7"), "Teredo with a public server and a public client (8.8.8.8)");
        // a v4-mapped address is still judged as its v4
        assert!(!public("::ffff:7f00:1") && public("::ffff:808:808"));
    }

    #[test]
    fn a_slow_dns_lookup_cannot_outlast_the_request_deadline() {
        let started = Instant::now();
        let result = resolve_with(
            || {
                std::thread::sleep(Duration::from_secs(3));
                Ok(Vec::new())
            },
            Instant::now() + Duration::from_millis(150),
        );
        assert_eq!(result, Err(NetError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(2), "the caller did not wait for the resolver");
        // a quick lookup is returned, and an IP literal is not looked up at all
        let quick = resolve_with(|| Ok(vec!["1.2.3.4:80".parse().unwrap()]), Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(quick.len(), 1);
        assert_eq!(resolve("[::1]", 9, Instant::now()).unwrap()[0].port(), 9);
        assert_eq!(resolve("127.0.0.1", 9, Instant::now()).unwrap()[0].ip(), "127.0.0.1".parse::<IpAddr>().unwrap());
    }

    /// Serves one canned reply per connection, in order; returns the port and the request heads it saw.
    fn serve_script(replies: Vec<Vec<u8>>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let Ok((mut s, _)) = listener.accept() else { break };
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut got = Vec::new();
                let mut b = [0u8; 4096];
                while find(&got, b"\r\n\r\n").is_none() {
                    let n = s.read(&mut b).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&b[..n]);
                }
                seen.push(String::from_utf8_lossy(&got).to_string());
                let _ = s.write_all(&reply);
            }
            seen
        });
        (port, handle)
    }

    fn redirect(status: u16, location: &str) -> Vec<u8> {
        format!("HTTP/1.1 {status} Found\r\nLocation: {location}\r\nContent-Length: 4\r\n\r\nmove").into_bytes()
    }

    fn ok(body: &str, content_type: &str) -> Vec<u8> {
        format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn a_redirect_is_handed_back_unless_the_request_asks_to_follow_it() {
        let (port, _h) = serve_script(vec![redirect(302, "/elsewhere")]);
        let r = Request::new(Method::Get, format!("http://127.0.0.1:{port}/a")).send().unwrap();
        assert_eq!((r.status, r.header("location")), (302, Some("/elsewhere")));
    }

    #[test]
    fn redirects_are_followed_each_hop_is_sent_and_three_hops_are_the_most() {
        let (port, h) = serve_script(vec![redirect(302, "/b"), redirect(301, "c"), redirect(307, "/d"), ok("done", "text/plain")]);
        let r = Request::new(Method::Get, format!("http://127.0.0.1:{port}/a")).redirects(3).send().unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, b"done".as_slice()));
        let seen = h.join().unwrap();
        let paths: Vec<&str> = seen.iter().map(|s| s.lines().next().unwrap()).collect();
        assert_eq!(paths, ["GET /a HTTP/1.1", "GET /b HTTP/1.1", "GET /c HTTP/1.1", "GET /d HTTP/1.1"]);

        let (port, _h) = serve_script(vec![redirect(302, "/1"), redirect(302, "/2"), redirect(302, "/3"), redirect(302, "/4")]);
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/a")).redirects(3).send().unwrap_err();
        assert_eq!(e, NetError::Protocol("TOO MANY REDIRECTS".into()));
        // A caller cannot ask for more than the cap.
        assert_eq!(Request::new(Method::Get, "http://x").redirects(50).redirects, MAX_REDIRECTS);
    }

    #[test]
    fn a_redirect_to_plain_http_elsewhere_or_to_another_scheme_is_refused_and_never_connected_to() {
        for location in ["http://example.org/x", "ftp://example.org/x", "file:///etc/passwd", "javascript:alert(1)"] {
            let (port, _h) = serve_script(vec![redirect(302, location)]);
            let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/a")).redirects(3).send().unwrap_err();
            assert!(matches!(e, NetError::Refused(_) | NetError::Unsupported(_)), "{location}: {e:?}");
        }
    }

    #[test]
    fn the_hop_guard_can_refuse_a_target() {
        let (port, _h) = serve_script(vec![redirect(302, "/forbidden")]);
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/a"))
            .redirects(3)
            .hop_guard(|url| url.ends_with("/forbidden").then(|| "that host is not allowed here".to_string()))
            .send()
            .unwrap_err();
        assert_eq!(e, NetError::Refused("that host is not allowed here".into()));
    }

    #[test]
    fn credentials_stay_with_their_origin_across_a_redirect() {
        // Same origin keeps the header, another origin (localhost is not the same host as 127.0.0.1) loses it.
        let (port_b, hb) = serve_script(vec![ok("b", "text/plain")]);
        let (port_a, ha) = serve_script(vec![redirect(302, "/same"), redirect(302, &format!("http://localhost:{port_b}/other"))]);
        let r = Request::new(Method::Get, format!("http://127.0.0.1:{port_a}/start"))
            .header("X-Session-Token", "secret-token")
            .header("Accept", "image/png")
            .redirects(3)
            .send()
            .unwrap();
        assert_eq!(r.body, b"b");
        let a = ha.join().unwrap();
        assert!(a[0].contains("X-Session-Token: secret-token"));
        assert!(a[1].contains("X-Session-Token: secret-token"), "same origin keeps it");
        let b = &hb.join().unwrap()[0];
        assert!(!b.to_ascii_lowercase().contains("secret-token") && !b.to_ascii_lowercase().contains("x-session-token"), "{b}");
        assert!(b.contains("Accept: image/png") && b.contains("User-Agent: LastRouteRust/"));
    }

    #[test]
    fn a_redirect_turns_a_post_into_a_get_and_drops_the_body() {
        let (port, h) = serve_script(vec![redirect(303, "/result"), ok("fine", "text/plain")]);
        let r = Request::new(Method::Post, format!("http://127.0.0.1:{port}/form"))
            .json_body(&serde_json::json!({"a": 1}))
            .redirects(1)
            .send()
            .unwrap();
        assert_eq!(r.status, 200);
        let seen = h.join().unwrap();
        assert!(seen[0].starts_with("POST /form") && seen[1].starts_with("GET /result"));
        assert!(!seen[1].to_ascii_lowercase().contains("content-length"));
    }

    #[test]
    fn the_content_type_the_length_and_the_size_cap_are_enforced_before_a_body_is_kept() {
        let (port, _h) = serve_script(vec![ok("<html>nope</html>", "text/html; charset=utf-8")]);
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).content_types(&["image/png", "application/octet-stream"]).send().unwrap_err();
        assert_eq!(e, NetError::Protocol("UNEXPECTED CONTENT TYPE".into()));
        let (port, _h) = serve_script(vec![ok("PNGDATA", "Image/PNG; x=y")]);
        let r = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).content_types(&["image/png"]).send().unwrap();
        assert_eq!(r.body, b"PNGDATA");
        // No Content-Type at all passes (a server may omit it); the caller validates the bytes.
        let (port, _h) = serve_script(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi".to_vec()]);
        assert_eq!(Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).content_types(&["image/png"]).send().unwrap().body, b"hi");
        // A body announced above the cap is refused without reading it.
        let (port, _h) = serve_script(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\nhi".to_vec()]);
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).limit(100).send().unwrap_err();
        assert_eq!(e, NetError::Protocol("RESPONSE TOO LARGE".into()));
        // A close-delimited body that runs past the cap is cut off.
        let (port, _h) = serve_script(vec![[b"HTTP/1.0 200 OK\r\n\r\n".to_vec(), vec![b'x'; 300_000]].concat()]);
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).limit(100).send().unwrap_err();
        assert_eq!(e, NetError::Protocol("RESPONSE TOO LARGE".into()));
    }

    #[test]
    fn a_public_only_request_never_connects_to_this_machine() {
        let (port, _h) = serve_script(vec![ok("secret", "text/plain")]);
        let e = Request::new(Method::Get, format!("http://127.0.0.1:{port}/")).public_only().send().unwrap_err();
        assert_eq!(e, NetError::Refused("THAT ADDRESS IS NOT PUBLIC".into()));
        let e = Request::new(Method::Get, format!("http://localhost:{port}/")).public_only().send().unwrap_err();
        assert!(matches!(e, NetError::Refused(_)), "{e:?}");
    }

    // ---- real TLS on loopback: a throwaway self-signed certificate (testdata/, for 127.0.0.1 and localhost only) ----

    const TEST_CERT: &[u8] = include_bytes!("testdata/test_cert.der");
    const TEST_KEY: &[u8] = include_bytes!("testdata/test_key.der");

    fn trusting_test_cert() -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(rustls::pki_types::CertificateDer::from(TEST_CERT.to_vec())).unwrap();
        Arc::new(client_config(roots))
    }

    /// An https server on loopback answering each connection with the next reply. Returns the port.
    fn serve_tls(replies: Vec<Vec<u8>>) -> (u16, std::thread::JoinHandle<()>) {
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for reply in replies {
                let Ok((tcp, _)) = listener.accept() else { break };
                tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                let mut tls = rustls::StreamOwned::new(connection, tcp);
                let mut got = Vec::new();
                let mut b = [0u8; 4096];
                while find(&got, b"\r\n\r\n").is_none() {
                    match tls.read(&mut b) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&b[..n]),
                    }
                }
                if find(&got, b"\r\n\r\n").is_some() {
                    let _ = tls.write_all(&reply);
                    let _ = tls.flush();
                    tls.conn.send_close_notify();
                    let _ = tls.flush();
                }
            }
        });
        (port, handle)
    }

    #[test]
    fn a_skin_png_comes_over_https_through_the_client_and_verifies_while_a_hostile_one_does_not() {
        use crate::shared::skins;
        // Loopback stands in for the skins host: real TLS, the skin download's own limit and content types, then the shared check.
        let mut px: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 7 % 251) as u8).collect();
        for pixel in px.chunks_mut(4) {
            pixel[3] = 255;
        }
        let good = skins::encode_png(&px, 64, 64);
        let id = skins::sha256_hex(&good);
        let mut png_reply = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", good.len()).into_bytes();
        png_reply.extend_from_slice(&good);
        // a header-only body served as image/png with the hash of its own bytes
        let mut head = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        head.extend_from_slice(b"IHDR");
        head.extend_from_slice(&[0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0, 0, 0, 0, 0]);
        let mut hostile_reply = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", head.len()).into_bytes();
        hostile_reply.extend_from_slice(&head);
        let (port, _h) = serve_tls(vec![png_reply, hostile_reply, ok("<html>", "text/html")]);
        let get = |path: &str| {
            let req = Request::new(Method::Get, format!("https://127.0.0.1:{port}/{path}"))
                .timeout(Duration::from_secs(5))
                .limit(skins::read_limit())
                .content_types(&["image/png", "image/x-png", "application/octet-stream", "binary/octet-stream"]);
            send_with(&req, &trusting_test_cert())
        };
        let body = get(&format!("{id}.png")).unwrap().body;
        assert_eq!(body, good);
        assert!(skins::verify_skin(&id, &body), "a skin over https passes the one shared check");
        let body = get("hostile.png").unwrap().body;
        assert_eq!(body, head);
        assert!(!skins::verify_skin(&skins::sha256_hex(&body), &body), "the right hash is not enough: the full check refuses it");
        assert!(get("page.png").is_err(), "an html page is not a picture");
    }

    #[test]
    fn https_works_end_to_end_when_the_certificate_is_trusted() {
        let (port, _h) = serve_tls(vec![ok("hello over tls", "text/plain")]);
        let req = Request::new(Method::Get, format!("https://127.0.0.1:{port}/skin.png"));
        let r = send_with(&req, &trusting_test_cert()).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, b"hello over tls".as_slice()));
    }

    #[test]
    fn https_refuses_a_certificate_the_built_in_roots_do_not_vouch_for() {
        let (port, _h) = serve_tls(vec![ok("never read", "text/plain")]);
        // `send` uses the Mozilla roots only: the test certificate is not among them.
        let e = Request::new(Method::Get, format!("https://127.0.0.1:{port}/")).timeout(Duration::from_secs(5)).send().unwrap_err();
        match e {
            NetError::Transport(m) => assert!(m.to_ascii_lowercase().contains("certificate") || m.to_ascii_lowercase().contains("unknownissuer") || m.contains("invalid"), "{m}"),
            other => panic!("expected a transport error, got {other:?}"),
        }
    }

    #[test]
    fn https_follows_a_redirect_over_tls_but_never_down_to_plain_http() {
        let (plain_port, _p) = serve_script(vec![ok("downgraded", "text/plain")]);
        let (port, _h) = serve_tls(vec![redirect(302, &format!("http://127.0.0.1:{plain_port}/x"))]);
        let req = Request::new(Method::Get, format!("https://127.0.0.1:{port}/a")).redirects(3);
        let e = send_with(&req, &trusting_test_cert()).unwrap_err();
        assert_eq!(e, NetError::Refused("A REDIRECT TO PLAIN HTTP IS REFUSED".into()));

        let (port, _h) = serve_tls(vec![redirect(302, "/b"), ok("second", "text/plain")]);
        // Both hops need their own connection; the second one is accepted by the same listener.
        let req = Request::new(Method::Get, format!("https://127.0.0.1:{port}/a")).redirects(3);
        assert_eq!(send_with(&req, &trusting_test_cert()).unwrap().body, b"second");
    }

    /// Needs the internet: `cargo test --bin last_route live_ -- --ignored`. Proves the built-in roots reach the real hosts.
    #[test]
    #[ignore]
    fn live_https_reaches_the_real_skin_hosts() {
        for url in ["https://api.mojang.com/users/profiles/minecraft/Notch", "https://sessionserver.mojang.com/session/minecraft/profile/069a79f444e94726a5befca90e38aaf5", "https://skins.kyttuslab.org/"] {
            let r = Request::new(Method::Get, url).timeout(Duration::from_secs(10)).public_only().redirects(3).send();
            eprintln!("{url} -> {:?}", r.as_ref().map(|r| (r.status, r.body.len())));
            assert!(r.is_ok(), "{url}: {r:?}");
        }
    }
}

#[cfg(test)]
mod plain_http_policy_tests {
    use super::*;

    #[test]
    fn release_refuses_plain_http_off_loopback() {
        let url = |s: &str| parse_url(s).unwrap();
        assert!(plain_http_allowed(&url("http://127.0.0.1:81/x"), false));
        assert!(plain_http_allowed(&url("http://localhost:81/x"), false));
        assert!(!plain_http_allowed(&url("http://example.org/x"), false));
        assert!(!plain_http_allowed(&url("http://example.org/x"), true), "a debug build still refuses arbitrary hosts");
        let test = url(super::super::config::TEST_BASE_URLS[0]);
        assert!(!plain_http_allowed(&test, false), "release refuses the TEST instance over http");
        assert!(plain_http_allowed(&test, true), "debug allows the TEST instance");
    }
}
