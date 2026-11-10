//! The secret scrub the logs and the crash reports use (copied from lr_server ops/crash_reports.rs).

use regex::Regex;
use std::sync::OnceLock;


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
