//! Version numbers: release tags (`v0.2.0`, `0.2.0`, `v0.3.0-test.1`) read leniently as semver.

use semver::Version;

/// The running program's version (Cargo.toml `version`). `LR_UPDATE_CURRENT_VERSION` overrides
/// it for tests (pretend to be an older build).
pub fn current() -> Version {
    std::env::var("LR_UPDATE_CURRENT_VERSION")
        .ok()
        .and_then(|v| parse(&v))
        .unwrap_or_else(|| parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|| Version::new(0, 0, 0)))
}

/// `v1.2.3`, `1.2.3`, `V1.2`, `1.2.3-test.1`, `release-1.2.3` -> semver; `None` if no x.y[.z] is found.
/// A missing patch (`1.2`) reads as `.0`. Build metadata (`+abc`) is kept but never compared.
pub fn parse(tag: &str) -> Option<Version> {
    let t = tag.trim();
    // drop a leading word such as "v", "V", "release-", "last_route-v"
    let start = t.find(|c: char| c.is_ascii_digit())?;
    let t = &t[start..];
    if let Ok(v) = Version::parse(t) {
        return Some(v);
    }
    // pad "1.2" / "1.2-test.1" to three parts
    let (core, rest) = match t.find(['-', '+']) {
        Some(i) => (&t[..i], &t[i..]),
        None => (t, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut padded = parts.join(".");
    for _ in parts.len()..3 {
        padded.push_str(".0");
    }
    Version::parse(&format!("{padded}{rest}")).ok()
}

/// Strictly newer, ignoring build metadata (semver precedence: 0.2.0-test.1 < 0.2.0).
pub fn newer(candidate: &Version, current: &Version) -> bool {
    candidate.cmp_precedence(current) == std::cmp::Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags() {
        assert_eq!(parse("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse("1.2").unwrap(), Version::new(1, 2, 0));
        assert_eq!(parse("release-0.4.1").unwrap(), Version::new(0, 4, 1));
        assert_eq!(parse("v0.3.0-test.1").unwrap().pre.as_str(), "test.1");
        assert_eq!(parse("v0.3-test.2").unwrap().to_string(), "0.3.0-test.2");
        assert!(parse("nightly").is_none());
        assert!(parse("v0.0.1.0").is_none());
    }

    #[test]
    fn compares() {
        let v = |s| parse(s).unwrap();
        assert!(newer(&v("0.2.0"), &v("0.1.0")));
        assert!(!newer(&v("0.1.0"), &v("0.1.0")));
        assert!(!newer(&v("0.1.0+abc"), &v("0.1.0")));
        assert!(newer(&v("0.2.0"), &v("0.2.0-test.1")));
        assert!(newer(&v("0.2.0-test.2"), &v("0.2.0-test.1")));
        assert!(!newer(&v("0.1.9"), &v("0.2.0-test.1")));
    }
}
