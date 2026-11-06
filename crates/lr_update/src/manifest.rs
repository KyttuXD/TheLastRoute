//! The two small JSON files a native build is published as, and every check made on them before anything is
//! downloaded or written:
//!
//! * `native/<platform>/latest.json` is a pointer: `{"build":47,"manifest":"builds/47/manifest.json","sha256":"..."}`.
//!   It is written last when a build is published, so a client never sees a half-published build.
//! * `builds/<N>/manifest.json` (schema `lr.native/1`) lists the files of the build, each one zlib-compressed on the
//!   server (`z`). It is signed with ed25519 (`manifest.json.sig`, see `verify.rs`).
//!
//! Everything in these files is hostile until checked: file names are confined to plain relative paths below the
//! install folder, sizes are capped, hashes must be 64 hex digits, and the platform must be the one asked for.

use serde_json::Value;
use std::collections::HashSet;

pub const SCHEMA: &str = "lr.native/1";
/// The newest launcher feature level this code understands. A manifest whose `launcher_min` is higher needs a newer
/// launcher (v1 only logs that; launcher self-update is deferred).
pub const LAUNCHER_LEVEL: u64 = 1;

pub const MAX_LATEST_BYTES: u64 = 4 * 1024;
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: u64 = 1024;
pub const MAX_FILES: usize = 2000;
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 240;

/// The platform ids a build can be published for.
pub const PLATFORMS: [&str; 4] = ["windows-x86_64", "macos-universal", "linux-x86_64", "linux-aarch64"];

/// The platform id of the machine this program was compiled for.
pub fn this_platform() -> &'static str {
    if cfg!(windows) {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "macos-universal"
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    }
}

/// A channel name: lower case letters, digits and dashes. The default channel is the empty string.
pub fn valid_channel(channel: &str) -> bool {
    channel.is_empty()
        || (channel.len() <= 24
            && !channel.starts_with('-')
            && channel.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
}

/// The folder on the website that holds a channel's builds: `native`, or `native-<channel>`.
pub fn site_root(channel: &str) -> String {
    if channel.is_empty() {
        "native".to_string()
    } else {
        format!("native-{channel}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Latest {
    pub build: u64,
    /// Always `builds/<build>/manifest.json`.
    pub manifest: String,
    /// sha256 of the manifest file, hex.
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Where the file goes, relative to the build folder, with `/` separators.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// The compressed copy on the server, a plain file name inside `builds/<N>/`.
    pub z: String,
    pub z_size: u64,
    pub z_sha256: String,
    /// 0o755 or 0o644 (applied on macOS and Linux only).
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub platform: String,
    pub build: u64,
    pub version: String,
    pub revision: String,
    pub built: String,
    pub min_os: Option<String>,
    pub launcher_min: u64,
    /// The sha256 of the web build this native build was made together with (informational).
    pub web_build: Option<String>,
    pub notes: String,
    /// The program to start, one of `files`.
    pub run: String,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    /// The bytes of all files once installed.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The bytes that have to be downloaded.
    pub fn total_download(&self) -> u64 {
        self.files.iter().map(|f| f.z_size).sum()
    }

    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }

    /// The `.app` folder the run file sits in, when it is a macOS bundle (`Name.app/Contents/MacOS/x`).
    pub fn bundle(&self) -> Option<&str> {
        let first = self.run.split('/').next()?;
        (first.ends_with(".app") && self.run.contains('/')).then_some(first)
    }
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn get_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| format!("no text field \"{key}\""))
}

fn get_u64(v: &Value, key: &str) -> Result<u64, String> {
    v.get(key).and_then(Value::as_u64).ok_or_else(|| format!("no whole-number field \"{key}\""))
}

fn parse_json(bytes: &[u8], limit: u64, what: &str) -> Result<Value, String> {
    if bytes.len() as u64 > limit {
        return Err(format!("the {what} is too large"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| format!("the {what} is not UTF-8 text"))?;
    serde_json::from_str(text).map_err(|e| format!("the {what} is not valid JSON ({e})"))
}

impl Latest {
    pub fn parse(bytes: &[u8]) -> Result<Latest, String> {
        let v = parse_json(bytes, MAX_LATEST_BYTES, "latest.json")?;
        let build = get_u64(&v, "build")?;
        let manifest = get_str(&v, "manifest")?.to_string();
        let sha256 = get_str(&v, "sha256")?.to_string();
        if build == 0 {
            return Err("build 0 is not a published build".into());
        }
        // The pointer may only name the manifest of its own build: no other path, no traversal.
        if manifest != format!("builds/{build}/manifest.json") {
            return Err("latest.json names a manifest outside builds/<build>/".into());
        }
        if !is_hex(&sha256, 64) {
            return Err("latest.json has a bad sha256".into());
        }
        Ok(Latest { build, manifest, sha256 })
    }
}

/// Why a file name is not allowed, or None when it is a safe relative path. The rules are the strictest of the
/// three systems: no absolute path, no drive letter or stream (`:`), no backslash, no `..`, no hidden names, no
/// names Windows reserves, no trailing dot or space.
pub fn bad_path(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("empty path");
    }
    if path.len() > MAX_PATH_BYTES {
        return Some("path too long");
    }
    if path.starts_with('/') || path.ends_with('/') {
        return Some("absolute path or trailing slash");
    }
    if path.bytes().any(|b| b < 0x20 || b == 0x7f || matches!(b, b'\\' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*')) {
        return Some("forbidden character");
    }
    for part in path.split('/') {
        if part.is_empty() {
            return Some("empty path part");
        }
        if part == "." || part == ".." {
            return Some("dot path part");
        }
        if part.starts_with('.') {
            return Some("hidden path part");
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Some("path part ends with a dot or space");
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
        if reserved {
            return Some("reserved Windows name");
        }
    }
    None
}

/// A plain file name for the compressed copy: letters, digits, dot, dash, underscore.
fn bad_z_name(name: &str) -> bool {
    name.is_empty()
        || name.len() > 64
        || name.starts_with('.')
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

impl Manifest {
    /// Parses and checks a manifest for `platform`. Nothing is trusted: see the module comment.
    pub fn parse(bytes: &[u8], platform: &str) -> Result<Manifest, String> {
        let v = parse_json(bytes, MAX_MANIFEST_BYTES, "manifest")?;
        if get_str(&v, "schema")? != SCHEMA {
            return Err(format!("the manifest schema is not {SCHEMA}"));
        }
        let got = get_str(&v, "platform")?;
        if got != platform {
            return Err(format!("the manifest is for {got}, not {platform}"));
        }
        let build = get_u64(&v, "build")?;
        if build == 0 {
            return Err("build 0 is not a published build".into());
        }
        let short_text = |key: &str, max: usize| -> Result<String, String> {
            let t = get_str(&v, key)?;
            if t.len() > max || t.chars().any(|c| c.is_control() && c != '\n') {
                return Err(format!("\"{key}\" is too long or has control characters"));
            }
            Ok(t.to_string())
        };
        let version = short_text("version", 40)?;
        let revision = short_text("revision", 64)?;
        let built = short_text("built", 40)?;
        let notes = v.get("notes").map(|_| short_text("notes", 500)).transpose()?.unwrap_or_default();
        let min_os = match v.get("min_os") {
            None | Some(Value::Null) => None,
            Some(_) => {
                let t = short_text("min_os", 20)?;
                if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
                    return Err("\"min_os\" is not a dotted number".into());
                }
                Some(t)
            }
        };
        let launcher_min = v.get("launcher_min").map(|_| get_u64(&v, "launcher_min")).transpose()?.unwrap_or(1);
        let web_build = match v.get("web_build") {
            None | Some(Value::Null) => None,
            Some(_) => Some(short_text("web_build", 64)?),
        };
        let run = get_str(&v, "run")?.to_string();
        let list = v.get("files").and_then(Value::as_array).ok_or("the manifest has no files list")?;
        if list.is_empty() || list.len() > MAX_FILES {
            return Err("the manifest has no files or too many".into());
        }
        let mut files = Vec::with_capacity(list.len());
        let mut seen: HashSet<String> = HashSet::new();
        let mut seen_z: HashSet<String> = HashSet::new();
        let mut total: u64 = 0;
        for entry in list {
            let path = get_str(entry, "path")?.to_string();
            if let Some(why) = bad_path(&path) {
                return Err(format!("unsafe file name {path:?}: {why}"));
            }
            let size = get_u64(entry, "size")?;
            let z_size = get_u64(entry, "z_size")?;
            if size > MAX_FILE_BYTES || z_size > MAX_FILE_BYTES {
                return Err(format!("{path}: file too large"));
            }
            total = total.saturating_add(size);
            if total > MAX_TOTAL_BYTES {
                return Err("the build is larger than the allowed total".into());
            }
            let sha256 = get_str(entry, "sha256")?.to_string();
            let z_sha256 = get_str(entry, "z_sha256")?.to_string();
            if !is_hex(&sha256, 64) || !is_hex(&z_sha256, 64) {
                return Err(format!("{path}: bad sha256"));
            }
            let z = get_str(entry, "z")?.to_string();
            if bad_z_name(&z) {
                return Err(format!("{path}: bad compressed file name {z:?}"));
            }
            let mode = match entry.get("mode").and_then(Value::as_str) {
                Some("755") => 0o755,
                Some("644") | None => 0o644,
                Some(_) => return Err(format!("{path}: mode must be 755 or 644")),
            };
            if !seen.insert(path.to_ascii_lowercase()) {
                return Err(format!("{path}: listed twice (names compare without case)"));
            }
            if !seen_z.insert(z.to_ascii_lowercase()) {
                return Err(format!("{z}: used for two files"));
            }
            files.push(FileEntry { path, size, sha256, z, z_size, z_sha256, mode });
        }
        // A file and a folder cannot share a name ("a" and "a/b").
        let names: HashSet<String> = files.iter().map(|f| f.path.to_ascii_lowercase()).collect();
        for f in &files {
            let lower = f.path.to_ascii_lowercase();
            let mut at = 0;
            while let Some(i) = lower[at..].find('/') {
                at += i;
                if names.contains(&lower[..at]) {
                    return Err(format!("{}: sits inside the file {}", f.path, &f.path[..at]));
                }
                at += 1;
            }
        }
        if bad_path(&run).is_some() || !files.iter().any(|f| f.path == run) {
            return Err("\"run\" is not one of the files".into());
        }
        Ok(Manifest { platform: platform.to_string(), build, version, revision, built, min_os, launcher_min, web_build, notes, run, files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn hex(c: char) -> String {
        std::iter::repeat(c).take(64).collect()
    }

    pub(crate) fn sample(platform: &str, build: u64) -> String {
        format!(
            r#"{{"schema":"lr.native/1","platform":"{platform}","build":{build},"version":"0.1.0","revision":"df640905804616743cd31aa0dd611fc9d534bcbd",
            "built":"2026-11-07T10:00:00Z","min_os":"11.0","launcher_min":1,"web_build":"{w}","notes":"short",
            "run":"KYTTU The Last Route.app/Contents/MacOS/last_route",
            "files":[{{"path":"KYTTU The Last Route.app/Contents/MacOS/last_route","size":100,"sha256":"{a}","z":"f0.z","z_size":50,"z_sha256":"{b}","mode":"755"}},
                     {{"path":"KYTTU The Last Route.app/Contents/Info.plist","size":10,"sha256":"{a}","z":"f1.z","z_size":9,"z_sha256":"{b}","mode":"644"}}]}}"#,
            w = hex('c'),
            a = hex('a'),
            b = hex('b')
        )
    }

    #[test]
    fn a_good_manifest_parses() {
        let m = Manifest::parse(sample("macos-universal", 47).as_bytes(), "macos-universal").unwrap();
        assert_eq!(m.build, 47);
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.files[0].mode, 0o755);
        assert_eq!(m.min_os.as_deref(), Some("11.0"));
        assert_eq!(m.bundle(), Some("KYTTU The Last Route.app"));
        assert_eq!(m.total_size(), 110);
        assert_eq!(m.total_download(), 59);
    }

    #[test]
    fn the_wrong_platform_is_refused() {
        let err = Manifest::parse(sample("linux-x86_64", 1).as_bytes(), "windows-x86_64").unwrap_err();
        assert!(err.contains("not windows-x86_64"), "{err}");
    }

    #[test]
    fn hostile_file_names_are_refused() {
        for bad in [
            "../evil.exe", "a/../../evil", "/etc/passwd", "C:/x.exe", "a\\\\b", "a//b", "a/./b", ".hidden", "a/.git/x", "con", "NUL.txt", "dir/COM1",
            "lpt9.log", "trail.", "trail ", "x:stream", "", "a/", "wild*card", "q?x", "a|b",
        ] {
            assert!(bad_path(bad).is_some(), "{bad:?} must be refused");
            let text = sample("macos-universal", 1).replace("Contents/Info.plist", bad);
            assert!(Manifest::parse(text.as_bytes(), "macos-universal").is_err(), "{bad:?} in a manifest must be refused");
        }
        for good in ["last_route.exe", "a/b/c.txt", "KYTTU The Last Route.app/Contents/MacOS/last_route", "bin/last_route"] {
            assert!(bad_path(good).is_none(), "{good:?}");
        }
    }

    #[test]
    fn sizes_hashes_modes_and_names_are_bounded() {
        let ok = sample("macos-universal", 1);
        let try_with = |from: &str, to: &str| Manifest::parse(ok.replace(from, to).as_bytes(), "macos-universal");
        assert!(try_with("\"size\":100", "\"size\":99999999999").is_err(), "a huge file");
        assert!(try_with("\"z_size\":50", "\"z_size\":99999999999").is_err(), "a huge download");
        assert!(try_with(&hex('a'), "xyz").is_err(), "a bad hash");
        assert!(try_with("\"mode\":\"755\"", "\"mode\":\"4755\"").is_err(), "a setuid mode");
        assert!(try_with("f0.z", "../f0.z").is_err(), "a compressed name with a path");
        assert!(try_with("\"f1.z\"", "\"f0.z\"").is_err(), "two files share a compressed copy");
        assert!(try_with("\"build\":1", "\"build\":0").is_err(), "build 0");
        assert!(try_with("lr.native/1", "lr.native/2").is_err(), "another schema");
        assert!(try_with("\"run\":\"KYTTU", "\"run\":\"Other").is_err(), "run must be a listed file");
        assert!(try_with("\"min_os\":\"11.0\"", "\"min_os\":\"eleven\"").is_err(), "min_os must be numeric");
        // many files: more than the cap
        let one = r#"{"path":"p0","size":1,"sha256":"A","z":"z0","z_size":1,"z_sha256":"A"}"#;
        let many = std::iter::repeat(one).take(MAX_FILES + 1).collect::<Vec<_>>().join(",");
        let text = format!(r#"{{"schema":"lr.native/1","platform":"p","build":1,"version":"v","revision":"r","built":"b","run":"p0","files":[{many}]}}"#);
        assert!(Manifest::parse(text.as_bytes(), "p").is_err());
        // a total over the cap from files that are each under it
        let big = (0..5)
            .map(|i| format!(r#"{{"path":"p{i}","size":300000000,"sha256":"{a}","z":"z{i}","z_size":1,"z_sha256":"{a}"}}"#, a = hex('a')))
            .collect::<Vec<_>>()
            .join(",");
        let text = format!(r#"{{"schema":"lr.native/1","platform":"p","build":1,"version":"v","revision":"r","built":"b","run":"p0","files":[{big}]}}"#);
        assert!(Manifest::parse(text.as_bytes(), "p").unwrap_err().contains("total"));
    }

    #[test]
    fn a_file_cannot_sit_inside_another_file_or_repeat_with_other_case() {
        let ok = sample("macos-universal", 1);
        let nested = ok.replace("Contents/Info.plist", "Contents/MacOS/last_route/inner");
        assert!(Manifest::parse(nested.as_bytes(), "macos-universal").unwrap_err().contains("inside the file"));
        let dup = ok.replace("Contents/Info.plist", "Contents/MacOS/LAST_ROUTE");
        assert!(Manifest::parse(dup.as_bytes(), "macos-universal").unwrap_err().contains("twice"));
    }

    #[test]
    fn oversized_or_non_json_input_is_refused() {
        assert!(Manifest::parse(&vec![b' '; (MAX_MANIFEST_BYTES + 1) as usize], "x").is_err());
        assert!(Manifest::parse(b"<html>", "x").is_err());
        assert!(Manifest::parse(&[0xff, 0xfe], "x").is_err());
        assert!(Latest::parse(&vec![b' '; 5000]).is_err());
    }

    #[test]
    fn latest_may_only_point_at_its_own_manifest() {
        let good = format!(r#"{{"build":47,"manifest":"builds/47/manifest.json","sha256":"{}"}}"#, hex('d'));
        let l = Latest::parse(good.as_bytes()).unwrap();
        assert_eq!((l.build, l.manifest.as_str()), (47, "builds/47/manifest.json"));
        for bad in ["builds/46/manifest.json", "../x/manifest.json", "/etc/passwd", "builds/47/../../x", "http://evil/m.json"] {
            let text = good.replace("builds/47/manifest.json", bad);
            assert!(Latest::parse(text.as_bytes()).is_err(), "{bad}");
        }
        assert!(Latest::parse(good.replace("\"build\":47", "\"build\":0").as_bytes()).is_err());
        assert!(Latest::parse(good.replace(&hex('d'), "nothex").as_bytes()).is_err());
    }

    #[test]
    fn channels_are_plain_words() {
        assert!(valid_channel("") && valid_channel("beta") && valid_channel("kt-test-2"));
        assert!(!valid_channel("../x") && !valid_channel("Beta") && !valid_channel("-x") && !valid_channel(&"a".repeat(25)));
        assert_eq!(site_root(""), "native");
        assert_eq!(site_root("beta"), "native-beta");
    }
}
