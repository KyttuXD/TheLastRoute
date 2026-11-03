//! Shared helpers for the Rust port of the Last Route Node backend.
//!
//! The Node modules lean on JavaScript coercions (`String(x || "")`, `Number(x)`, truthiness) and on
//! `JSON.stringify` output. These helpers reproduce those semantics so each ported module can stay a line-by-line
//! translation of its `.mjs` original.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicI64, Ordering};

/// A module result, like the `[status, body]` pairs the Node modules return.
pub type Reply = (u16, Value);

/// Milliseconds since the Unix epoch (the Node modules' injectable `now = Date.now`).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    })
}

/// A settable clock for tests: `let (clock, handle) = manual_clock(1_000); handle.set(2_000);`
#[cfg(test)]
#[derive(Clone)]
pub struct ManualClock(pub Arc<AtomicI64>);

#[cfg(test)]
impl ManualClock {
    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
    pub fn get(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
pub fn manual_clock(start_ms: i64) -> (Clock, ManualClock) {
    let cell = Arc::new(AtomicI64::new(start_ms));
    let reader = cell.clone();
    (Arc::new(move || reader.load(Ordering::SeqCst)), ManualClock(cell))
}

/// Read and parse a JSON file; None when missing or unreadable.
#[cfg(test)]
pub fn read_json_file(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// Anything that can turn a session token into a canonical username (the account service, or a test fake).
/// Mirrors `accountService.sessionUsername(token)`, which returns "" / null for unknown or expired tokens.
pub trait SessionLookup {
    fn session_username(&self, token: &str) -> Option<String>;
}

// ---------------------------------------------------------------------------------------------------------------
// Logging (the Node `logger = console`).

pub fn log_info(message: &str) {
    println!("{message}");
}
pub fn log_warn(message: &str) {
    eprintln!("{message}");
}
pub fn log_error(message: &str) {
    eprintln!("{message}");
}

// ---------------------------------------------------------------------------------------------------------------
// JavaScript coercions.

/// JavaScript truthiness of an optional JSON value (`undefined` = None).
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(0.0);
            f != 0.0 && !f.is_nan()
        }
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

/// JavaScript `Number(value)` -> string conversion (`String(1)` is "1", `String(1.5)` is "1.5").
pub fn js_number_string(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return "0".into();
    }
    if f.fract() == 0.0 && f.abs() < 1e21 {
        return format!("{}", f as i128);
    }
    // Rust's shortest round-trip formatting matches JS for the ordinary range.
    let s = format!("{f}");
    s
}

/// JavaScript `String(value)` for a present JSON value (null -> "null").
pub fn js_string_of(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                js_number_string(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string_of(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `String(value || "")`: falsy values (undefined, null, false, 0, NaN, "") become "".
pub fn str_or_empty(value: Option<&Value>) -> String {
    if truthy(value) {
        js_string_of(value.unwrap())
    } else {
        String::new()
    }
}

/// `String(value ?? "")`: only undefined/null become "".
pub fn str_nullish(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string_of(v),
    }
}

/// JavaScript `Number(text)` for a string.
pub fn js_number_from_str(text: &str) -> f64 {
    let t = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if t.is_empty() {
        return 0.0;
    }
    let (neg, body) = if let Some(rest) = t.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = t.strip_prefix('+') {
        (false, rest)
    } else {
        (false, t)
    };
    let lower = body.to_ascii_lowercase();
    let radix = if lower.starts_with("0x") {
        Some(16)
    } else if lower.starts_with("0o") {
        Some(8)
    } else if lower.starts_with("0b") {
        Some(2)
    } else {
        None
    };
    if let Some(radix) = radix {
        // JS rejects a sign before a radix prefix.
        if t.starts_with('-') || t.starts_with('+') {
            return f64::NAN;
        }
        let digits = &body[2..];
        if digits.is_empty() {
            return f64::NAN;
        }
        let mut acc = 0f64;
        for c in digits.chars() {
            match c.to_digit(radix) {
                Some(d) => acc = acc * radix as f64 + d as f64,
                None => return f64::NAN,
            }
        }
        return acc;
    }
    if body == "Infinity" {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // JS numeric literal grammar: digits, optional fraction, optional exponent. No "inf"/"nan" words, no "_".
    let valid = {
        let bytes = body.as_bytes();
        let mut i = 0;
        let mut digits = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            false
        } else {
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return f64::NAN;
                }
            }
            i == bytes.len()
        }
    };
    if !valid {
        return f64::NAN;
    }
    let parsed: f64 = body.parse().unwrap_or(f64::NAN);
    if neg {
        -parsed
    } else {
        parsed
    }
}

/// JavaScript `Number(value)` for an optional JSON value (`undefined` -> NaN, null -> 0).
pub fn js_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js_number_from_str(s),
        Some(Value::Array(items)) => match items.len() {
            0 => 0.0,
            1 => js_number(Some(&Value::String(js_string_of(&items[0])))),
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `Number(value) || fallback`.
pub fn number_or(value: Option<&Value>, fallback: f64) -> f64 {
    let n = js_number(value);
    if n == 0.0 || n.is_nan() {
        fallback
    } else {
        n
    }
}

/// `Number.isSafeInteger(value)` (no coercion: only real JSON numbers count).
pub fn is_safe_integer(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            f.is_finite() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0
        }
        _ => false,
    }
}

/// A JSON number the way `JSON.stringify` writes it: integral values have no ".0"; NaN/Infinity become null.
pub fn num(f: f64) -> Value {
    if !f.is_finite() {
        return Value::Null;
    }
    if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 {
        return Value::from(f as i64);
    }
    serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
}

/// Field access that tolerates non-objects (like `body?.field`).
pub fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key),
        _ => None,
    }
}

/// `Math.floor`, `Math.trunc`, `Math.ceil` on f64 returning f64 (keep NaN semantics explicit at call sites).
pub fn js_round(f: f64) -> f64 {
    // Math.round rounds .5 toward +Infinity.
    (f + 0.5).floor()
}

/// `JSON.stringify(value)`.
pub fn stringify(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// Length in UTF-16 code units (JavaScript `string.length`).
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `string.slice(0, n)` in UTF-16 code units (never splits a surrogate pair into invalid UTF-8; a split pair is
/// dropped).
pub fn js_slice(text: &str, end: usize) -> String {
    let mut count = 0usize;
    let mut out = String::new();
    for c in text.chars() {
        let units = c.len_utf16();
        if count + units > end {
            break;
        }
        count += units;
        out.push(c);
    }
    out
}

/// `string.trim()` (JS whitespace incl. BOM).
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

// ---------------------------------------------------------------------------------------------------------------
// Crypto.

pub fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    getrandom::getrandom(&mut bytes).expect("operating system random source");
    bytes
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

/// `Buffer.from(text, "hex")`: Node stops at the first invalid pair.
pub fn from_hex(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16);
        let lo = (bytes[i + 1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => break,
        }
        i += 2;
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (`buffer.toString("base64")`).
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// URL-safe base64 without padding (`buffer.toString("base64url")`).
pub fn base64url(bytes: &[u8]) -> String {
    base64(bytes).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

/// Lenient base64 / base64url decode (Node's `Buffer.from(text, "base64")` ignores invalid characters).
pub fn from_base64(text: &str) -> Vec<u8> {
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::new();
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// `createHash("sha256").update(text).digest("hex")`.
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex(&sha256(bytes.as_ref()))
}

/// `createHmac("sha256", key).update(data).digest()`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Node's `crypto.scrypt(password, salt, keylen)` with its defaults (N=16384, r=8, p=1).
pub fn scrypt_derive(password: &str, salt: &str, key_len: usize) -> Vec<u8> {
    let params = scrypt::Params::new(14, 8, 1, key_len).expect("valid scrypt parameters");
    let mut out = vec![0u8; key_len];
    scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, &mut out).expect("scrypt output length");
    out
}

/// Constant-time comparison of two byte strings of equal length (`timingSafeEqual`).
pub fn timing_safe_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

/// server.mjs `safeEqual(left, right)`: padded constant-time compare of `String(x || "")`.
pub fn safe_equal(left: &str, right: &str) -> bool {
    let length = left.len().max(right.len()).max(1);
    let mut a = vec![0u8; length];
    let mut b = vec![0u8; length];
    a[..left.len()].copy_from_slice(left.as_bytes());
    b[..right.len()].copy_from_slice(right.as_bytes());
    timing_safe_equal(&a, &b) && left.len() == right.len()
}

/// `safeEqual` applied to JSON body values (`String(value || "")` on both sides).
pub fn safe_equal_values(left: Option<&Value>, right: &str) -> bool {
    safe_equal(&str_or_empty(left), right)
}

// ---------------------------------------------------------------------------------------------------------------
// Files.

/// `writeFileSync(path + ".tmp", text); renameSync(tmp, path)` (creating the parent directory first).
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, path)
}

// ---------------------------------------------------------------------------------------------------------------
// Time.

/// `new Date(ms).toISOString()`.
pub fn iso_time(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

/// `Date.parse(iso)` for the `YYYY-MM-DDTHH:MM:SS(.mmm)?Z` strings this backend writes; None otherwise.
#[cfg(test)]
pub fn parse_iso_time(text: &str) -> Option<i64> {
    let t = text.trim();
    let bytes = t.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> { t.get(a..b)?.parse::<i64>().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if bytes[4] != b'-' || bytes[7] != b'-' || (bytes[10] != b'T' && bytes[10] != b't') || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let mut ms = 0i64;
    let mut rest = &t[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let padded = format!("{:0<3}", &digits[..digits.len().min(3)]);
        ms = padded.parse().ok()?;
        rest = &frac[digits.len()..];
    }
    if rest != "Z" && rest != "z" {
        return None;
    }
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400) + h * 3600 + mi * 60 + s) * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coercions_match_javascript() {
        assert_eq!(str_or_empty(Some(&json!(0))), "");
        assert_eq!(str_or_empty(Some(&json!(12))), "12");
        assert_eq!(str_or_empty(Some(&json!(1.5))), "1.5");
        assert_eq!(str_nullish(Some(&json!(false))), "false");
        assert_eq!(js_number(Some(&json!(" 42 "))), 42.0);
        assert!(js_number(Some(&json!("4x"))).is_nan());
        assert_eq!(js_number(Some(&json!(""))), 0.0);
        assert_eq!(js_number(Some(&json!("0x10"))), 16.0);
        assert!(js_number(None).is_nan());
        assert_eq!(stringify(&num(3.0)), "3");
        assert_eq!(stringify(&json!({"b": 1, "a": 2})), r#"{"b":1,"a":2}"#);
    }

    #[test]
    fn encodings_and_time() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(from_base64("aGVsbG8"), b"hello");
        assert_eq!(hex(&[0, 255]), "00ff");
        assert_eq!(from_hex("00ffzz"), vec![0, 255]);
        assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(parse_iso_time("2023-11-14T22:13:20.123Z"), Some(1_700_000_000_123));
        assert!(safe_equal("abc", "abc"));
        assert!(!safe_equal("abc", "abcd"));
        assert!(!safe_equal("", "x"));
    }

    #[test]
    fn scrypt_matches_node_defaults() {
        // node -e 'console.log(require("crypto").scryptSync("password","salt",16).toString("hex"))'
        assert_eq!(hex(&scrypt_derive("password", "salt", 16)), "745731af4484f323968969eda289aeee");
    }
}
