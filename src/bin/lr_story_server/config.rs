//! The settings of `lr_story_server`, read from the environment.
//!
//! A variable that is unset takes its default. A variable that is set but not valid (an empty value, port 0, a port
//! above 65535, a word that is not a boolean) is an error: the server does not start, and nothing falls back silently.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_HTTP_PORT: u16 = 8307;
pub const DEFAULT_RELAY_PORT: u16 = 1307;
pub const DEFAULT_HOST: &str = "0.0.0.0";
pub const DEFAULT_DATA_DIR: &str = ".data/lr_story_server";
pub const DEFAULT_PACKS_DIR: &str = "mods";
pub const DEFAULT_RATE_WINDOW_SECONDS: u64 = 60;
pub const DEFAULT_API_RATE_LIMIT: u64 = 600;
pub const DEFAULT_ACCOUNT_RATE_LIMIT: u64 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct StoryConfig {
    /// The address both the HTTP API and the UDP relay bind (`LR_STORY_HOST`).
    pub host: String,
    pub http_port: u16,
    pub relay_port: u16,
    pub relay_enabled: bool,
    /// Accounts, guest names and the party saves (`accounts.json`, `guest-names.json`, `parties/<CODE>.json`).
    pub data_dir: PathBuf,
    /// The story pack folders, one folder per pack id (`LR_STORY_PACKS_DIR`).
    pub packs_dir: PathBuf,
    /// Whether `X-Forwarded-For` names the client (behind a reverse proxy only).
    pub trust_proxy: bool,
    pub rate_limit_window_ms: i64,
    pub api_rate_limit_max: u32,
    pub account_rate_limit_max: u32,
    /// The account website (`LASTROUTE_ACCOUNT_UPSTREAM`, the same setting as lr_server): when set, sign-in, sessions and
    /// registration are answered by it, so a story player uses the same account as in Last Route. Plain http only
    /// (loopback on VPS 2: http://127.0.0.1:8205). Unset keeps the story server's own accounts.json.
    pub account_upstream: Option<crate::upstream::Upstream>,
}

/// A TCP or UDP port: digits only, from 1 to 65535.
pub fn parse_port(name: &str, value: &str) -> Result<u16, String> {
    let text = value.trim();
    let bad = || format!("{name} must be a port number from 1 to 65535, not '{value}'");
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    match text.parse::<u32>() {
        Ok(number) if (1..=65535).contains(&number) => Ok(number as u16),
        _ => Err(bad()),
    }
}

/// A boolean word: 1/true/yes/on or 0/false/no/off (any case).
pub fn parse_bool(name: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{name} must be true or false, not '{value}'")),
    }
}

/// A whole number from `min` to `max`.
pub fn parse_count(name: &str, value: &str, min: u64, max: u64) -> Result<u64, String> {
    let text = value.trim();
    match text.parse::<u64>() {
        Ok(number) if text.bytes().all(|b| b.is_ascii_digit()) && (min..=max).contains(&number) => Ok(number),
        _ => Err(format!("{name} must be a whole number from {min} to {max}, not '{value}'")),
    }
}

/// A folder setting: relative paths hang off the working directory.
fn folder_setting(name: &str, value: Option<&str>, cwd: &Path, default: &str) -> Result<PathBuf, String> {
    match value {
        None => Ok(cwd.join(default)),
        Some(text) if text.trim().is_empty() => Err(format!("{name} is set but empty")),
        Some(text) => {
            let path = PathBuf::from(text.trim());
            Ok(if path.is_absolute() { path } else { cwd.join(path) })
        }
    }
}

/// Reads the configuration from `env` (the process environment in production). `cwd` anchors relative folders.
pub fn from_env(env: &HashMap<String, String>, cwd: &Path) -> Result<StoryConfig, String> {
    let get = |name: &str| env.get(name).map(String::as_str);
    let http_port = match get("LR_STORY_HTTP_PORT") {
        Some(value) => parse_port("LR_STORY_HTTP_PORT", value)?,
        None => DEFAULT_HTTP_PORT,
    };
    let relay_port = match get("LR_STORY_RELAY_PORT") {
        Some(value) => parse_port("LR_STORY_RELAY_PORT", value)?,
        None => DEFAULT_RELAY_PORT,
    };
    let relay_enabled = match get("LR_STORY_RELAY_ENABLED") {
        Some(value) => parse_bool("LR_STORY_RELAY_ENABLED", value)?,
        None => true,
    };
    let host = match get("LR_STORY_HOST") {
        Some(value) if value.trim().is_empty() => return Err("LR_STORY_HOST is set but empty".into()),
        Some(value) => value.trim().to_string(),
        None => DEFAULT_HOST.to_string(),
    };
    let data_dir = folder_setting("LR_STORY_DATA_DIR", get("LR_STORY_DATA_DIR"), cwd, DEFAULT_DATA_DIR)?;
    let packs_dir = folder_setting("LR_STORY_PACKS_DIR", get("LR_STORY_PACKS_DIR"), cwd, DEFAULT_PACKS_DIR)?;
    let trust_proxy = match get("LR_STORY_TRUST_PROXY") {
        Some(value) => parse_bool("LR_STORY_TRUST_PROXY", value)?,
        None => false,
    };
    let window_seconds = match get("LR_STORY_RATE_LIMIT_WINDOW_SECONDS") {
        Some(value) => parse_count("LR_STORY_RATE_LIMIT_WINDOW_SECONDS", value, 1, 86_400)?,
        None => DEFAULT_RATE_WINDOW_SECONDS,
    };
    let api_rate_limit_max = match get("LR_STORY_API_RATE_LIMIT") {
        Some(value) => parse_count("LR_STORY_API_RATE_LIMIT", value, 1, 1_000_000)?,
        None => DEFAULT_API_RATE_LIMIT,
    };
    let account_rate_limit_max = match get("LR_STORY_ACCOUNT_RATE_LIMIT") {
        Some(value) => parse_count("LR_STORY_ACCOUNT_RATE_LIMIT", value, 1, 1_000_000)?,
        None => DEFAULT_ACCOUNT_RATE_LIMIT,
    };
    let account_upstream = match get("LASTROUTE_ACCOUNT_UPSTREAM") {
        Some(value) if !value.trim().is_empty() => {
            Some(crate::upstream::Upstream::parse(value).map_err(|e| format!("LASTROUTE_ACCOUNT_UPSTREAM {e}"))?)
        }
        _ => None,
    };
    Ok(StoryConfig {
        host,
        http_port,
        relay_port,
        relay_enabled,
        data_dir,
        packs_dir,
        trust_proxy,
        rate_limit_window_ms: window_seconds as i64 * 1000,
        api_rate_limit_max: api_rate_limit_max as u32,
        account_rate_limit_max: account_rate_limit_max as u32,
        account_upstream,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// The account website is the same setting as lr_server's: unset keeps local accounts; http is read; https and a
    /// path are refused (the upstream client speaks plain http to one host).
    #[test]
    fn the_account_upstream_is_optional_and_plain_http() {
        let cwd = Path::new("/srv");
        assert_eq!(from_env(&env(&[]), cwd).unwrap().account_upstream, None);
        assert_eq!(from_env(&env(&[("LASTROUTE_ACCOUNT_UPSTREAM", "  ")]), cwd).unwrap().account_upstream, None);
        let up = from_env(&env(&[("LASTROUTE_ACCOUNT_UPSTREAM", "http://127.0.0.1:8205")]), cwd).unwrap().account_upstream.unwrap();
        assert_eq!(up.authority(), "127.0.0.1:8205");
        assert!(from_env(&env(&[("LASTROUTE_ACCOUNT_UPSTREAM", "https://example.org")]), cwd).is_err());
        assert!(from_env(&env(&[("LASTROUTE_ACCOUNT_UPSTREAM", "http://127.0.0.1:8205/api")]), cwd).is_err());
    }

    #[test]
    fn unset_ports_take_the_story_defaults_8307_and_1307() {
        let config = from_env(&HashMap::new(), Path::new("C:/work")).unwrap();
        assert_eq!(config.http_port, 8307);
        assert_eq!(config.relay_port, 1307);
        assert!(config.relay_enabled);
        assert_eq!(config.data_dir, Path::new("C:/work").join(".data/lr_story_server"));
    }

    #[test]
    fn port_zero_and_bad_port_values_are_errors_never_a_fallback() {
        for bad in ["0", "65536", "-1", "abc", "", "  ", "80.5", "1e3", "99999999999999999999"] {
            assert!(parse_port("LR_STORY_HTTP_PORT", bad).is_err(), "{bad:?} should be refused");
            let config = env(&[("LR_STORY_HTTP_PORT", bad)]);
            assert!(from_env(&config, Path::new(".")).is_err(), "{bad:?} should stop the server");
            let config = env(&[("LR_STORY_RELAY_PORT", bad)]);
            assert!(from_env(&config, Path::new(".")).is_err(), "{bad:?} should stop the relay");
        }
        assert_eq!(parse_port("LR_STORY_HTTP_PORT", "1").unwrap(), 1);
        assert_eq!(parse_port("LR_STORY_HTTP_PORT", " 65535 ").unwrap(), 65535);
    }

    #[test]
    fn explicit_ports_and_flags_are_read() {
        // An absolute folder on every OS ("D:/..." is relative on Linux, where the server runs).
        let data = std::env::temp_dir().join("story-data");
        let data_text = data.to_string_lossy().to_string();
        let config = env(&[
            ("LR_STORY_HTTP_PORT", "9001"),
            ("LR_STORY_RELAY_PORT", "9002"),
            ("LR_STORY_RELAY_ENABLED", "off"),
            ("LR_STORY_DATA_DIR", data_text.as_str()),
        ]);
        let config = from_env(&config, Path::new(".")).unwrap();
        assert_eq!((config.http_port, config.relay_port, config.relay_enabled), (9001, 9002, false));
        assert_eq!(config.data_dir, data);
    }

    #[test]
    fn a_relay_flag_must_be_a_boolean_word() {
        assert_eq!(parse_bool("X", "TRUE"), Ok(true));
        assert_eq!(parse_bool("X", "0"), Ok(false));
        assert!(parse_bool("X", "maybe").is_err());
        assert!(parse_bool("X", "").is_err());
        let config = env(&[("LR_STORY_RELAY_ENABLED", "maybe")]);
        assert!(from_env(&config, Path::new(".")).is_err());
    }

    #[test]
    fn an_empty_folder_setting_is_an_error() {
        let config = env(&[("LR_STORY_DATA_DIR", "  ")]);
        assert!(from_env(&config, Path::new(".")).is_err());
    }
}
