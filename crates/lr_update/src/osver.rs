//! The version of the operating system, for `min_os` in a manifest. Only Windows and macOS have a minimum worth
//! checking (the Metal and OpenGL paths of an old macOS); Linux reports nothing and always passes.

use std::process::Command;

/// Compares dotted numbers (`12.7.6` against `11.0`): true when `have` is at least `need`. Missing parts count as 0;
/// a part that is not a number makes the answer true (an unknown system is not blocked by a guess).
pub fn at_least(have: &str, need: &str) -> bool {
    let parts = |s: &str| -> Option<Vec<u64>> { s.trim().split('.').map(|p| p.trim().parse::<u64>().ok()).collect() };
    let (Some(h), Some(n)) = (parts(have), parts(need)) else {
        return true;
    };
    for i in 0..h.len().max(n.len()) {
        let (a, b) = (h.get(i).copied().unwrap_or(0), n.get(i).copied().unwrap_or(0));
        if a != b {
            return a > b;
        }
    }
    true
}

/// `Microsoft Windows [Version 10.0.26100.1234]` -> `10.0.26100.1234`.
pub fn parse_windows_ver(text: &str) -> Option<String> {
    let after = text.split("Version").nth(1)?;
    let v: String = after.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    (!v.is_empty()).then_some(v)
}

/// The running system's version, or None when it cannot be read.
pub fn detect() -> Option<String> {
    if cfg!(windows) {
        let out = Command::new("cmd").args(["/C", "ver"]).output().ok()?;
        parse_windows_ver(&String::from_utf8_lossy(&out.stdout))
    } else if cfg!(target_os = "macos") {
        let out = Command::new("sw_vers").arg("-productVersion").output().ok()?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!v.is_empty()).then_some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(at_least("12.7.6", "11.0"));
        assert!(at_least("11.0", "11"));
        assert!(at_least("11", "11.0.0"));
        assert!(!at_least("10.15.7", "11.0"));
        assert!(at_least("10.0.26100", "10.0.19041"));
        assert!(!at_least("10.9", "10.10"), "10.9 is older than 10.10");
        assert!(at_least("weird", "11.0"), "an unreadable version is not blocked");
    }

    #[test]
    fn the_windows_ver_line_parses() {
        assert_eq!(parse_windows_ver("\r\nMicrosoft Windows [Version 10.0.26100.1234]\r\n").as_deref(), Some("10.0.26100.1234"));
        assert_eq!(parse_windows_ver("nothing"), None);
    }
}
