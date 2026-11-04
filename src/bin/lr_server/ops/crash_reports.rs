//! Port of `backend/crash_reports.mjs`.
//!
//! Crash and freeze reports from game clients (CrashWatch, lr.crash/1). Store and echo only: a report is clipped to
//! fixed field limits, scrubbed of anything that looks like a credential, written as one JSON file under the data
//! directory, and echoed back as stored. Nothing here acts on a report; the owner lists them (POST
//! /api/admin/crash-reports) and routes them to whoever owns the code.

use crate::util::{self, Clock, Reply};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const CRASH_SCHEMA: &str = "lr.crash/1";
pub const CRASH_MAX_BODY_BYTES: usize = 96 * 1024;

/// `CRASH_LIMITS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashLimits {
    pub log_tail_chars: usize,
    pub breadcrumbs: usize,
    pub errors: usize,
    pub line_chars: usize,
    pub field_chars: usize,
    pub stored: usize,
    pub per_client_per_hour: usize,
    pub list_max: usize,
}

pub const CRASH_LIMITS: CrashLimits = CrashLimits {
    log_tail_chars: 32 * 1024,
    breadcrumbs: 50,
    errors: 50,
    line_chars: 400,
    field_chars: 120,
    stored: 5000,
    per_client_per_hour: 6,
    list_max: 200,
};

const KINDS: [&str; 3] = ["crash", "freeze", "script_errors"];
const TEXT_FIELDS: [&str; 11] =
    ["build", "version", "os", "os_version", "device", "gpu", "renderer", "locale", "scene", "room_role", "device_id"];
const COUNT_FIELDS: [&str; 4] = ["errors", "warnings", "script_errors", "dropped"];

/// JavaScript `\s` (WhiteSpace + LineTerminator), as a regex class body.
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// A case-insensitive ASCII word, spelled out as `[xX]` classes (JS `/i` without `/u` never folds non-ASCII letters
/// onto ASCII ones, unlike a Unicode-mode `(?i)`).
fn ci(word: &str) -> String {
    word.chars()
        .map(|c| if c.is_ascii_alphabetic() { format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()) } else { c.to_string() })
        .collect()
}

struct SecretPatterns {
    auth: Regex,
    assignment: Regex,
    hex: Regex,
}

fn secret_patterns() -> &'static SecretPatterns {
    static PATTERNS: OnceLock<SecretPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let b = r"(?-u:\b)";
        // /\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}/gi
        let auth = format!(r"{b}({}|{})[{JS_SPACE}]+[A-Za-z0-9._~+/=-]{{8,}}", ci("bearer"), ci("basic"));
        // /\b(session(?:_?token)?|token|password|passwd|secret|api_?key|admin_?ticket|host_?token|authorization)\b
        //   (\s*["']?\s*[:=]\s*["']?)[^\s"',;&}]{4,}/gi
        let words = [
            format!("{}(?:_?{})?", ci("session"), ci("token")),
            ci("token"),
            ci("password"),
            ci("passwd"),
            ci("secret"),
            format!("{}_?{}", ci("api"), ci("key")),
            format!("{}_?{}", ci("admin"), ci("ticket")),
            format!("{}_?{}", ci("host"), ci("token")),
            ci("authorization"),
        ]
        .join("|");
        let assignment = format!(
            r#"{b}({words}){b}([{JS_SPACE}]*["']?[{JS_SPACE}]*[:=][{JS_SPACE}]*["']?)[^{JS_SPACE}"',;&}}]{{4,}}"#
        );
        // /\b[A-Fa-f0-9]{40,}\b/g
        let hex = format!(r"{b}[A-Fa-f0-9]{{40,}}{b}");
        SecretPatterns {
            auth: Regex::new(&auth).expect("auth pattern"),
            assignment: Regex::new(&assignment).expect("assignment pattern"),
            hex: Regex::new(&hex).expect("hex pattern"),
        }
    })
}

/// `scrubSecrets(text)`: anything credential-shaped is replaced, wherever it appears.
pub fn scrub_secrets(text: &str) -> String {
    let patterns = secret_patterns();
    let out = patterns.auth.replace_all(text, "${1} [redacted]");
    let out = patterns.assignment.replace_all(&out, "${1}${2}[redacted]");
    let out = patterns.hex.replace_all(&out, "[redacted-hex]");
    out.into_owned()
}

fn clip_text(text: &str, max: usize) -> String {
    let scrubbed = scrub_secrets(text);
    let cleaned: String = scrubbed
        .chars()
        .filter(|&c| !matches!(c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F))
        .collect();
    util::js_slice(&cleaned, max)
}

/// `clip(value, max)`: `scrubSecrets(String(value ?? ""))`, control characters removed, sliced to `max`.
fn clip(value: Option<&Value>, max: usize) -> String {
    clip_text(&util::str_nullish(value), max)
}

/// A JSON value with integral floats written as integers, so `serde_json` output matches `JSON.stringify`.
fn js_normalize(value: &Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => util::num(n.as_f64().unwrap_or(f64::NAN)),
        Value::Array(items) => Value::Array(items.iter().map(js_normalize).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), js_normalize(v))).collect()),
        other => other.clone(),
    }
}

/// `JSON.stringify(value)` for a parsed JSON value.
fn js_stringify(value: &Value) -> String {
    util::stringify(&js_normalize(value))
}

/// `sanitizeCrashReport(body)`: the stored shape of a report, or the error code when it is not one.
pub fn sanitize_crash_report(body: &Value) -> Result<Map<String, Value>, &'static str> {
    let Value::Object(fields) = body else {
        return Err("INVALID_REPORT");
    };
    if fields.get("schema") != Some(&Value::String(CRASH_SCHEMA.into())) {
        return Err("INVALID_REPORT_SCHEMA");
    }
    let kind = util::str_or_empty(fields.get("kind"));
    if !KINDS.contains(&kind.as_str()) {
        return Err("INVALID_REPORT_KIND");
    }
    let mut report = Map::new();
    report.insert("schema".into(), json!(CRASH_SCHEMA));
    report.insert("kind".into(), json!(kind));
    for field in TEXT_FIELDS {
        match fields.get(field) {
            None | Some(Value::Null) => {}
            Some(value) => {
                report.insert(field.into(), json!(clip(Some(value), CRASH_LIMITS.field_chars)));
            }
        }
    }
    let mut counts = Map::new();
    let count_source = fields.get("counts");
    for field in COUNT_FIELDS {
        let value = util::js_number(count_source.and_then(|c| util::field(c, field)));
        if value.is_finite() && value >= 0.0 {
            counts.insert(field.into(), util::num(value.floor().min(1e9)));
        }
    }
    report.insert("counts".into(), Value::Object(counts));
    for field in ["session_started", "session_ended"] {
        let value = util::js_number(fields.get(field));
        if value.is_finite() && value > 0.0 {
            report.insert(field.into(), util::num(value.floor()));
        }
    }
    let list_of = |value: Option<&Value>, max: usize| -> Value {
        let items: Vec<Value> = match value {
            Some(Value::Array(items)) => items[items.len().saturating_sub(max)..]
                .iter()
                .map(|entry| {
                    let text = match entry {
                        Value::String(s) => s.clone(),
                        other => js_stringify(other),
                    };
                    Value::String(clip_text(&text, CRASH_LIMITS.line_chars))
                })
                .collect(),
            _ => Vec::new(),
        };
        Value::Array(items)
    };
    report.insert("breadcrumbs".into(), list_of(fields.get("breadcrumbs"), CRASH_LIMITS.breadcrumbs));
    report.insert("recent_errors".into(), list_of(fields.get("recent_errors"), CRASH_LIMITS.errors));
    report.insert("log_tail".into(), json!(clip(fields.get("log_tail"), CRASH_LIMITS.log_tail_chars)));
    Ok(report)
}

pub struct CrashReportsOptions {
    /// Required (the JS factory throws `TypeError("directory is required")` without it).
    pub directory: PathBuf,
    pub now: Clock,
}

impl Default for CrashReportsOptions {
    fn default() -> Self {
        CrashReportsOptions { directory: PathBuf::new(), now: util::system_clock() }
    }
}

/// `createCrashReports({ directory, now })`.
pub struct CrashReports {
    directory: PathBuf,
    now: Clock,
    rates: HashMap<String, Vec<i64>>,
}

const REPORT_ID_PATTERN: &str = r"^[0-9]{8}T[0-9]{6}-[a-f0-9]{8}$";

fn report_id_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(REPORT_ID_PATTERN).expect("report id pattern"))
}

impl CrashReports {
    pub fn new(options: CrashReportsOptions) -> CrashReports {
        if options.directory.as_os_str().is_empty() {
            panic!("directory is required");
        }
        if let Err(error) = std::fs::create_dir_all(&options.directory) {
            util::log_error(&format!("Crash reports: could not create {}: {error}", options.directory.display()));
        }
        CrashReports { directory: options.directory, now: options.now, rates: HashMap::new() }
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| name.ends_with(".json"))
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    fn allow(&mut self, client: &str) -> bool {
        let hour = 3600 * 1000;
        let timestamp = (self.now)();
        let mut recent: Vec<i64> = self
            .rates
            .get(client)
            .map(|times| times.iter().copied().filter(|&at| timestamp - at < hour).collect())
            .unwrap_or_default();
        if recent.len() >= CRASH_LIMITS.per_client_per_hour {
            self.rates.insert(client.to_string(), recent);
            return false;
        }
        recent.push(timestamp);
        self.rates.insert(client.to_string(), recent);
        if self.rates.len() > 10000 {
            self.rates.retain(|_, times| times.iter().any(|&at| timestamp - at < hour));
        }
        true
    }

    /// `submit(body, client)`. `client` is the requester address plus the report's device id (rate limit only;
    /// never stored). An empty `client` counts as "unknown" (`String(client || "unknown")`).
    pub fn submit(&mut self, body: &Value, client: &str) -> Reply {
        let report = match sanitize_crash_report(body) {
            Ok(report) => report,
            Err(code) => return (400, json!({ "ok": false, "code": code, "error": "Not a crash report" })),
        };
        let client = if client.is_empty() { "unknown" } else { client };
        if !self.allow(client) {
            return (429, json!({ "ok": false, "code": "CRASH_RATE_LIMITED", "error": "Too many crash reports from this client" }));
        }
        let stamp: String = util::iso_time((self.now)()).replace(['-', ':'], "").chars().take(15).collect();
        let id = format!("{stamp}-{}", util::hex(&util::random_bytes(4)));
        let mut stored = Map::new();
        stored.insert("id".into(), json!(id));
        stored.insert("received_at".into(), json!(util::iso_time((self.now)())));
        for (key, value) in report {
            stored.insert(key, value);
        }
        let stored = Value::Object(stored);
        let path = self.directory.join(format!("{id}.json"));
        if let Err(error) = util::write_atomic(&path, util::stringify(&stored).as_bytes()) {
            // Node would throw here (an unhandled rejection); answer instead of crashing.
            util::log_error(&format!("Crash report write failed: {error}"));
            return (500, json!({ "ok": false, "error": error.to_string() }));
        }
        let names = self.files();
        let excess = names.len().saturating_sub(CRASH_LIMITS.stored);
        for name in &names[..excess] {
            let _ = std::fs::remove_file(self.directory.join(name));
        }
        (201, json!({ "ok": true, "id": id, "report": stored }))
    }

    /// `list({ since, limit })`: owner view, newest first, reports received after `since` (an id), at most `limit`.
    pub fn list(&self, since: Option<&Value>, limit: Option<&Value>) -> Reply {
        let max = util::number_or(limit, 50.0).max(1.0).min(CRASH_LIMITS.list_max as f64);
        let since = since.map(util::js_string_of).unwrap_or_default();
        let after = if report_id_pattern().is_match(&since) { since } else { String::new() };
        let mut reports = Vec::new();
        let mut names = self.files();
        names.reverse();
        for name in names {
            let id = &name[..name.len() - 5];
            if !after.is_empty() && id <= after.as_str() {
                break;
            }
            let parsed = std::fs::read_to_string(self.directory.join(&name))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok());
            match parsed {
                Some(report) => reports.push(report),
                None => continue,
            }
            if reports.len() as f64 >= max {
                break;
            }
        }
        (200, json!({ "ok": true, "reports": reports, "total": self.files().len() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::manual_clock;

    fn report(extra: Value) -> Value {
        let mut base = json!({
            "schema": "lr.crash/1", "kind": "crash", "build": "abc1234", "os": "macOS", "os_version": "15.1",
            "device": "Mac15,3", "gpu": "Apple M3", "scene": "res://game/infinite_route/infinite_route_test.tscn",
            "room_role": "guest", "device_id": "dev-1",
            "session_started": 1_800_000_000i64, "session_ended": 1_800_000_100i64,
            "counts": { "errors": 3, "warnings": 1, "script_errors": 2 },
            "breadcrumbs": ["scene infinite_route_test", "interact stream_door d1"], "recent_errors": ["E 12 x"],
            "log_tail": "line 1\nline 2\n",
        });
        if let (Value::Object(map), Value::Object(extra)) = (&mut base, extra) {
            for (k, v) in extra {
                map.insert(k, v);
            }
        }
        base
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr-crash-{name}-{}", util::hex(&util::random_bytes(6))));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_report_is_clipped_scrubbed_and_refused_when_not_lr_crash_1() {
        assert_eq!(sanitize_crash_report(&json!({ "schema": "lr.crash/2", "kind": "crash" })).unwrap_err(), "INVALID_REPORT_SCHEMA");
        assert_eq!(sanitize_crash_report(&json!({ "schema": "lr.crash/1", "kind": "boom" })).unwrap_err(), "INVALID_REPORT_KIND");
        assert_eq!(sanitize_crash_report(&json!([])).unwrap_err(), "INVALID_REPORT");
        let crumbs: Vec<Value> = (0..200).map(|i| json!(format!("crumb {i} {}", "y".repeat(1000)))).collect();
        let big = sanitize_crash_report(&report(json!({
            "log_tail": "x".repeat(CRASH_LIMITS.log_tail_chars * 2),
            "breadcrumbs": crumbs,
            "build": "q".repeat(500),
            "unknown_field": "dropped",
            "counts": { "errors": -1, "warnings": "7", "script_errors": 1e12 },
        })))
        .unwrap();
        assert_eq!(big["log_tail"].as_str().unwrap().len(), CRASH_LIMITS.log_tail_chars);
        let breadcrumbs = big["breadcrumbs"].as_array().unwrap();
        assert_eq!(breadcrumbs.len(), CRASH_LIMITS.breadcrumbs);
        assert!(breadcrumbs[0].as_str().unwrap().starts_with("crumb 150"), "the newest breadcrumbs are kept");
        assert!(breadcrumbs.iter().all(|line| util::js_len(line.as_str().unwrap()) <= CRASH_LIMITS.line_chars));
        assert_eq!(big["build"].as_str().unwrap().len(), CRASH_LIMITS.field_chars);
        assert!(big.get("unknown_field").is_none());
        assert_eq!(util::stringify(&big["counts"]), r#"{"warnings":7,"script_errors":1000000000}"#);

        let leaky = format!(
            "auth: Bearer abcdefghijklmnop sessionToken=s3cr3tvalue password: hunter22 admin_ticket=\"tkt-123456\" {}",
            "f".repeat(64)
        );
        let scrubbed = scrub_secrets(&leaky);
        for secret in ["abcdefghijklmnop", "s3cr3tvalue", "hunter22", "tkt-123456", &"f".repeat(64)] {
            assert!(!scrubbed.contains(secret), "{secret} survived: {scrubbed}");
        }
        let stored = sanitize_crash_report(&report(json!({
            "log_tail": leaky, "breadcrumbs": [{ "what": "join", "token": "abcd1234zz" }],
        })))
        .unwrap();
        assert!(!stored["log_tail"].as_str().unwrap().contains("hunter22"));
        assert!(!stored["breadcrumbs"][0].as_str().unwrap().contains("abcd1234zz"));
    }

    #[test]
    fn store_and_echo_rate_limit_prune_and_list_newest_first_after_an_id() {
        let root = temp_dir("store");
        // Date.UTC(2026, 9, 28, 20, 0, 0)
        let (clock, handle) = manual_clock(util::parse_iso_time("2026-10-28T20:00:00.000Z").unwrap());
        let mut store = CrashReports::new(CrashReportsOptions { directory: root.clone(), now: clock });
        let mut ids = Vec::new();
        for i in 0..CRASH_LIMITS.per_client_per_hour {
            handle.advance(1000);
            let (status, body) = store.submit(&report(json!({ "build": format!("b{i}") })), "1.2.3.4|dev-1");
            assert_eq!(status, 201);
            assert_eq!(body["report"]["build"], json!(format!("b{i}")), "the stored report is echoed");
            ids.push(body["id"].as_str().unwrap().to_string());
        }
        assert_eq!(store.submit(&report(json!({})), "1.2.3.4|dev-1").0, 429, "a seventh report in an hour is refused");
        assert_eq!(store.submit(&report(json!({})), "5.6.7.8|dev-2").0, 201, "another client is not limited by the first");
        handle.advance(3600 * 1000);
        assert_eq!(store.submit(&report(json!({})), "1.2.3.4|dev-1").0, 201, "the limit resets after an hour");
        let (_, listed) = store.list(None, Some(&json!(3)));
        let reports = listed["reports"].as_array().unwrap();
        assert_eq!(reports.len(), 3);
        assert!(reports[0]["id"].as_str().unwrap() > reports[1]["id"].as_str().unwrap(), "newest first");
        let (_, after) = store.list(Some(&json!(ids[3])), None);
        let after = after["reports"].as_array().unwrap();
        assert!(after.iter().all(|entry| entry["id"].as_str().unwrap() > ids[3].as_str()) && after.len() == 4);
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(format!("{}.json", ids[0]))).unwrap()).unwrap();
        assert_eq!(on_disk["device_id"], json!("dev-1"));
        assert!(!util::stringify(&on_disk).contains("1.2.3.4"), "the client address is never stored");
        assert!(
            std::fs::read_dir(&root).unwrap().all(|e| e.unwrap().file_name().to_string_lossy().ends_with(".json")),
            "no temp files are left"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // Reference values produced by the Node module (crash_reports.mjs) for the same inputs.
    #[test]
    fn matches_node_reference_output() {
        let sanitized = sanitize_crash_report(&json!({
            "schema": "lr.crash/1", "kind": "freeze", "build": 12.5, "os": null, "gpu": { "a": 1 },
            "counts": { "errors": "4.9", "dropped": 2.2, "warnings": null },
            "session_started": "17.8", "session_ended": -1,
            "breadcrumbs": [1.0, { "b": [1, "x"] }, null, "tab\there\u{0001}"], "recent_errors": "nope", "log_tail": 42,
        }))
        .unwrap();
        assert_eq!(
            util::stringify(&Value::Object(sanitized)),
            r#"{"schema":"lr.crash/1","kind":"freeze","build":"12.5","gpu":"[object Object]","counts":{"errors":4,"warnings":0,"dropped":2},"session_started":17,"breadcrumbs":["1","{\"b\":[1,\"x\"]}","null","tab\there"],"recent_errors":[],"log_tail":"42"}"#
        );
        let cases: Vec<String> = serde_json::from_str(r##"["auth: Bearer abcdefghijklmnop sessionToken=s3cr3tvalue password: hunter22 admin_ticket=\"tkt-123456\" ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","BASIC QWxhZGRpbjpvcGVuIHNlc2FtZQ== then TOKEN = 'abcd' and api_key:\"zz\" secret=abc","host_token: 0123456789abcdef0123456789abcdef01234567 xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeg","mytoken=abcdef session_token = qqqqqq; passwd=1234;x authorization: \"Basic zzzzzzzzzz\"","Session-Token=abcdefgh session token=abcdef apikey=abcdefg"]"##).unwrap();
        let expected: Vec<String> = serde_json::from_str(r##"["auth: Bearer [redacted] sessionToken=[redacted] password: [redacted] admin_ticket=\"[redacted]\" [redacted-hex]","BASIC [redacted] then TOKEN = '[redacted]' and api_key:\"zz\" secret=abc","host_token: [redacted] xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeg","mytoken=abcdef session_token = [redacted]; passwd=[redacted];x authorization: \"[redacted] [redacted]\"","Session-Token=[redacted] session token=[redacted] apikey=[redacted]"]"##).unwrap();
        for (case, want) in cases.iter().zip(&expected) {
            assert_eq!(&scrub_secrets(case), want);
        }
    }
}
