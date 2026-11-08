//! server.mjs helpers: requests/responses, JSON replies, static files, update manifests, small normalisers.

use crate::util::{is_safe_integer, js_number, str_nullish, str_or_empty, stringify, truthy, Reply};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::OnceLock;

pub const CITY_MAP_TICKET_HEADER: &str = "x-city-map-ticket";
pub const SKIN_TOKEN_HEADER: &str = "x-session-token";
pub const SKIN_ARMS_HEADER: &str = "x-skin-arms";
pub const UPDATE_CHANNELS: [&str; 2] = ["debug", "release"];
pub const UPDATE_PLATFORMS: [&str; 8] = ["linux", "windows", "windows_legacy", "macos", "web", "steamvr", "quest", "phone"];

pub fn cached(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}
#[macro_export]
macro_rules! lr_regex {
    ($pattern:expr) => {{
        static CELL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        $crate::server::support::cached(&CELL, $pattern)
    }};
}
use crate::lr_regex as regex;

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// The raw path without the query string, e.g. `/api/enet/rooms/ABCDE`.
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Lower-case header names.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    /// The body was longer than the transport's hard cap (the Node server destroys such requests).
    pub body_overflow: bool,
    /// The direct peer address (`request.socket.remoteAddress`).
    pub remote: String,
    pub local_port: u16,
}

impl Request {
    #[cfg(test)]
    pub fn new(method: &str, target: &str) -> Request {
        let (path, query) = split_target(target);
        Request { method: method.to_string(), path, query, remote: "127.0.0.1".into(), ..Request::default() }
    }
    #[cfg(test)]
    pub fn json(method: &str, target: &str, body: &Value) -> Request {
        let mut request = Request::new(method, target);
        request.body = stringify(body).into_bytes();
        request.headers.insert("content-type".into(), "application/json".into());
        request
    }
    #[cfg(test)]
    pub fn with_header(mut self, name: &str, value: &str) -> Request {
        self.headers.insert(name.to_ascii_lowercase(), value.to_string());
        self
    }
    pub fn query_param(&self, name: &str) -> Option<String> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
    pub fn header(&self, name: &str) -> String {
        self.headers.get(name).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    #[cfg(test)]
    pub fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub fn response_headers() -> Vec<(String, String)> {
    vec![
        ("cache-control".into(), "no-store".into()),
        ("access-control-allow-origin".into(), "*".into()),
        ("access-control-allow-methods".into(), "GET, HEAD, POST, PUT, OPTIONS".into()),
        (
            "access-control-allow-headers".into(),
            format!("content-type, {CITY_MAP_TICKET_HEADER}, {SKIN_TOKEN_HEADER}, {SKIN_ARMS_HEADER}"),
        ),
    ]
}

pub fn merge_headers(mut base: Vec<(String, String)>, extra: &[(&str, String)]) -> Vec<(String, String)> {
    for (key, value) in extra {
        if let Some(slot) = base.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value.clone();
        } else {
            base.push((key.to_string(), value.clone()));
        }
    }
    base
}

/// `json(response, status, value, extraHeaders)`.
pub fn json_response(status: u16, value: &Value, extra: &[(&str, String)]) -> Response {
    if status == 204 {
        return Response { status, headers: merge_headers(response_headers(), extra), body: Vec::new() };
    }
    let body = stringify(value).into_bytes();
    let mut headers = response_headers();
    headers.push(("content-type".into(), "application/json; charset=utf-8".into()));
    headers.push(("content-length".into(), body.len().to_string()));
    Response { status, headers: merge_headers(headers, extra), body }
}

pub fn reply(result: Reply) -> Response {
    json_response(result.0, &result.1, &[])
}

pub fn split_target(target: &str) -> (String, Vec<(String, String)>) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |t: &str| percent_decode(&t.replace('+', " ")).unwrap_or_else(|| t.to_string());
            (decode(k), decode(v))
        })
        .collect();
    (path, query)
}

/// `decodeURIComponent`; None on malformed escapes or invalid UTF-8.
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("ico") => "image/x-icon",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("wav") => "audio/wav",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

/// `tryStaticFile(request, response, directory, pathname)`: None when the file is not served.
pub fn try_static_file(method: &str, directory: Option<&Path>, pathname: &str) -> Option<Response> {
    let directory = directory?;
    if method != "GET" && method != "HEAD" {
        return None;
    }
    let decoded = percent_decode(pathname)?;
    let relative = if decoded == "/" { "/index.html".to_string() } else { decoded };
    let mut candidate = directory.to_path_buf();
    let mut depth: i32 = 0;
    for part in Path::new(relative.trim_start_matches(['/', '\\'])).components() {
        match part {
            Component::Normal(p) => {
                candidate.push(p);
                depth += 1;
            }
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                candidate.pop();
                depth -= 1;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    if depth == 0 {
        return None;
    }
    let mut file = candidate;
    if file.is_dir() {
        file = file.join("index.html");
    }
    if !file.is_file() {
        return None;
    }
    let bytes = std::fs::read(&file).ok()?;
    let headers = merge_headers(
        response_headers(),
        &[("content-type", mime_type(&file).to_string()), ("content-length", bytes.len().to_string())],
    );
    Some(Response { status: 200, headers, body: if method == "HEAD" { Vec::new() } else { bytes } })
}

/// `publicUpdateManifest(updates, channel, platform)`.
pub fn public_update_manifest(updates: &Value, channel: &str, platform: &str) -> Option<Value> {
    let entry = updates.get(channel)?.get(platform)?;
    if !entry.is_object() {
        return None;
    }
    let version = {
        let v = str_or_empty(entry.get("version"));
        let v = v.trim();
        v.strip_prefix('v').or_else(|| v.strip_prefix('V')).unwrap_or(v).to_string()
    };
    let url = str_or_empty(entry.get("url")).trim().to_string();
    let file = str_or_empty(entry.get("file")).trim().to_string();
    let sha256 = str_or_empty(entry.get("sha256")).trim().to_lowercase();
    let revision = str_or_empty(entry.get("revision")).trim().to_lowercase();
    let relative_url = if regex!(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,190}$").is_match(&file) {
        format!("/api/updates/files/{}", encode_uri_component(&file))
    } else {
        String::new()
    };
    if !regex!(r"^\d+(?:\.\d+){2,3}$").is_match(&version) || (!url.starts_with("https://") && relative_url.is_empty()) {
        return None;
    }
    if !regex!(r"^[a-f0-9]{64}$").is_match(&sha256) {
        return None;
    }
    if !revision.is_empty() && !regex!(r"^[a-f0-9]{7,40}$").is_match(&revision) {
        return None;
    }
    let positive_int = |v: &&Value| is_safe_integer(Some(v)) && js_number(Some(v)) > 0.0;
    let build = entry.get("build").filter(positive_int).cloned();
    if platform != "web" && (build.is_none() || revision.is_empty()) {
        return None;
    }
    let mut out = Map::new();
    out.insert("version".into(), json!(version));
    out.insert("revision".into(), json!(revision));
    if let Some(build) = build {
        out.insert("build".into(), build);
    }
    out.insert("url".into(), json!(if url.starts_with("https://") { url } else { relative_url }));
    out.insert("file".into(), json!(file));
    if let Some(bytes) = entry.get("bytes").filter(positive_int) {
        out.insert("bytes".into(), bytes.clone());
    }
    out.insert("sha256".into(), json!(sha256));
    out.insert("notes".into(), json!(crate::util::js_slice(&str_or_empty(entry.get("notes")), 2000)));
    out.insert("required".into(), json!(truthy(entry.get("required"))));
    Some(Value::Object(out))
}

/// `publicUpdateStatus(updates)`.
pub fn public_update_status(updates: &Value) -> Value {
    let mut status = Map::new();
    for channel in UPDATE_CHANNELS {
        let mut published: Vec<(String, String, String)> = Vec::new();
        for platform in UPDATE_PLATFORMS {
            if let Some(update) = public_update_manifest(updates, channel, platform) {
                published.push((platform.into(), str_nullish(update.get("version")), str_nullish(update.get("revision"))));
            }
        }
        if published.is_empty() {
            continue;
        }
        let (mut versions, mut revisions): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for (_, v, r) in &published {
            if !versions.contains(v) {
                versions.push(v.clone());
            }
            if !r.is_empty() && !revisions.contains(r) {
                revisions.push(r.clone());
            }
        }
        let version = if versions.len() == 1 { versions[0].clone() } else { "mixed".to_string() };
        let revision = match revisions.len() {
            1 => revisions[0].clone(),
            0 => "unreported".to_string(),
            _ => "mixed".to_string(),
        };
        let platforms: Vec<String> = published.iter().map(|(p, _, _)| p.clone()).collect();
        status.insert(channel.into(), json!({ "version": version, "revision": revision, "platforms": platforms }));
    }
    Value::Object(status)
}

/// `publishedUpdatePath(updates, encodedName)`.
pub fn published_update_path(updates: &Value, encoded: &str) -> String {
    let Some(name) = percent_decode(encoded) else { return String::new() };
    if !regex!(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,190}$").is_match(&name) {
        return String::new();
    }
    for channel in UPDATE_CHANNELS {
        for platform in UPDATE_PLATFORMS {
            let entry = updates.get(channel).and_then(|c| c.get(platform));
            if entry.and_then(|e| e.get("file")).and_then(Value::as_str) == Some(name.as_str())
                && public_update_manifest(updates, channel, platform).is_some()
            {
                return format!("/{name}");
            }
        }
    }
    String::new()
}

/// `normalizeEnetCode(value)`.
pub fn normalize_enet_code(value: Option<&Value>) -> String {
    let code = str_or_empty(value).trim().to_uppercase();
    if regex!(r"^[A-Z0-9]{5}$").is_match(&code) {
        code
    } else {
        String::new()
    }
}

pub fn code_of(text: &str) -> String {
    normalize_enet_code(Some(&Value::String(text.to_string())))
}

pub fn clean_address(value: &str) -> String {
    let address = value.trim();
    if let Some(rest) = address.strip_prefix("::ffff:") {
        return rest.to_string();
    }
    if address == "::1" {
        return "127.0.0.1".into();
    }
    address.to_string()
}

/// `requesterAddress(request, trustProxy)`: loopback peers (a same-machine proxy) are trusted for X-Forwarded-For.
pub fn requester_address(request: &Request, trust_proxy: bool) -> String {
    let direct = clean_address(&request.remote);
    if trust_proxy || direct == "127.0.0.1" {
        let forwarded = request.header("x-forwarded-for");
        let first = forwarded.split(',').next().unwrap_or("").trim().to_string();
        if !first.is_empty() {
            return clean_address(&first);
        }
    }
    direct
}

/// `cleanGame(value)`.
pub fn clean_game(value: Option<&Value>) -> String {
    let text = str_nullish(value).trim().to_lowercase();
    if regex!(r"^[a-z][a-z0-9_]{0,15}$").is_match(&text) {
        text
    } else {
        "last_route".into()
    }
}

/// `finitePlayers(value, fallback)`.
pub fn finite_players(value: Option<&Value>, fallback: f64) -> f64 {
    let number = js_number(value);
    if number.is_finite() {
        number.min(16.0).max(1.0)
    } else {
        fallback
    }
}

/// `deploymentPorts(environment)` -> (httpPort, relayPort).
pub fn deployment_ports(env: &HashMap<String, String>) -> (u16, u16) {
    let valid = |name: &str| -> u16 {
        let n = crate::util::js_number_from_str(env.get(name).map(String::as_str).unwrap_or("x"));
        if n.is_finite() && n.fract() == 0.0 && (1.0..=65_535.0).contains(&n) {
            n as u16
        } else {
            0
        }
    };
    let injected = if valid("PORT") != 0 { valid("PORT") } else { valid("SERVER_PORT") };
    let mut http = valid("LASTROUTE_HTTP_PORT");
    if http == 0 {
        http = if injected == 1105 { 8105 } else { injected };
    }
    if http == 0 {
        http = 8105;
    }
    let relay = if valid("LASTROUTE_RELAY_PORT") != 0 { valid("LASTROUTE_RELAY_PORT") } else { 1105 };
    (http, relay)
}

/// A log line with every credential-shaped value replaced (passwords, session tokens, dev tokens, bearer headers, long
/// hex keys). Every line that carries request data goes through this before it is written; nothing logs a body.
pub fn redact_log_line(line: &str) -> String {
    let scrubbed = crate::crash_reports::scrub_secrets(line);
    // the server's own request fields, which the crash-report patterns do not name
    let fields = regex!(r#"(?i)(dev_?token|admin_?proof|confirm_?token|current_?password|new_?password|x-session-token|x-download-key)([[:space:]]*["']?[[:space:]]*[:=][[:space:]]*["']?)[^[:space:]"',;&}]{3,}"#);
    fields.replace_all(&scrubbed, "$1$2[redacted]").into_owned()
}
