//! Reading the website: `latest.json`, the signed manifest, and the compressed files. Each step is checked
//! before the next one starts (see `manifest.rs` and `verify.rs`).

use crate::http::{self, HttpError, Options};
use crate::manifest::{self, FileEntry, Latest, Manifest};
use crate::verify;
use std::io::Write;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The site cannot be reached at all (a browser would not get through either).
    Offline(String),
    /// There is no latest.json for this platform and channel: no native build is published for it.
    NoNativeBuild,
    /// The site answered, but something on it is wrong or missing (HTTP error, bad JSON, size).
    Site(String),
    /// A hash, a signature or a rule of the manifest failed: never retried with the same files.
    Untrusted(String),
    Io(String),
    Cancelled,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Offline(w) => write!(f, "offline: {w}"),
            FetchError::NoNativeBuild => f.write_str("no native build is published for this system"),
            FetchError::Site(w) => write!(f, "the site: {w}"),
            FetchError::Untrusted(w) => write!(f, "refused: {w}"),
            FetchError::Io(w) => write!(f, "disk: {w}"),
            FetchError::Cancelled => f.write_str("cancelled"),
        }
    }
}

fn from_http(e: HttpError) -> FetchError {
    match e {
        HttpError::Offline(w) => FetchError::Offline(w),
        HttpError::Cancelled => FetchError::Cancelled,
        HttpError::Sink(w) => FetchError::Io(w),
        other => FetchError::Site(other.to_string()),
    }
}

/// Whether `url` may be a download base: https for any host, plain http only for this machine (tests).
pub fn base_url_allowed(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if lower.starts_with("http://") && http::host_of(url).is_some_and(|h| http::is_loopback_host(&h)) {
        return Ok(());
    }
    Err("the site address must be https (plain http only for localhost)".into())
}

/// One platform's builds on the website.
#[derive(Clone, Debug)]
pub struct Site {
    base: String,
    root: String,
    pub platform: String,
    pub trusted: Vec<[u8; 32]>,
    /// The timeout for the small files (latest.json, manifest, signature).
    pub quick_seconds: u64,
}

impl Site {
    pub fn new(base_url: &str, channel: &str, platform: &str, trusted: Vec<[u8; 32]>) -> Result<Site, String> {
        base_url_allowed(base_url)?;
        if !manifest::valid_channel(channel) {
            return Err("the channel must be lower case letters, digits and dashes".into());
        }
        Ok(Site { base: base_url.trim_end_matches('/').to_string(), root: manifest::site_root(channel), platform: platform.to_string(), trusted, quick_seconds: 4 })
    }

    pub fn url(&self, rel: &str) -> String {
        format!("{}/{}/{}/{rel}", self.base, self.root, self.platform)
    }

    fn small(&self, rel: &str, limit: u64, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, HttpError> {
        http::get_bytes(&self.url(rel), &Options::quick(limit, self.quick_seconds), cancel)
    }

    /// Reads the pointer. A 404 is "no native build for this platform".
    pub fn fetch_latest(&self, cancel: Option<&AtomicBool>) -> Result<Latest, FetchError> {
        let bytes = match self.small("latest.json", manifest::MAX_LATEST_BYTES, cancel) {
            Ok(b) => b,
            Err(HttpError::Status(404)) | Err(HttpError::Status(403)) => return Err(FetchError::NoNativeBuild),
            Err(e) => return Err(from_http(e)),
        };
        Latest::parse(&bytes).map_err(FetchError::Site)
    }

    /// Reads the manifest the pointer names and checks it end to end: the hash the pointer lists, the signature, the
    /// schema, the platform, and that it is the build the pointer says.
    pub fn fetch_manifest(&self, latest: &Latest, cancel: Option<&AtomicBool>) -> Result<(Manifest, Vec<u8>, String), FetchError> {
        let bytes = self.small(&latest.manifest, manifest::MAX_MANIFEST_BYTES, cancel).map_err(from_http)?;
        if verify::sha256_hex(&bytes) != latest.sha256 {
            return Err(FetchError::Untrusted("the manifest does not match the hash in latest.json".into()));
        }
        let sig_bytes = self.small(&format!("{}.sig", latest.manifest), manifest::MAX_SIGNATURE_BYTES, cancel).map_err(from_http)?;
        let sig = String::from_utf8(sig_bytes).map_err(|_| FetchError::Untrusted("the signature file is not text".into()))?;
        verify::verify_signature(&self.trusted, &bytes, &sig).map_err(|e| FetchError::Untrusted(e.to_string()))?;
        let m = Manifest::parse(&bytes, &self.platform).map_err(FetchError::Untrusted)?;
        if m.build != latest.build {
            return Err(FetchError::Untrusted("the manifest is for another build than latest.json names".into()));
        }
        Ok((m, bytes, sig))
    }

    /// Streams one compressed file into `sink`. The caller checks size and hash afterwards (the writer hashes as it goes).
    pub fn download_z(
        &self,
        build: u64,
        entry: &FileEntry,
        cancel: Option<&AtomicBool>,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, FetchError> {
        let url = self.url(&format!("builds/{build}/{}", entry.z));
        // The listed size plus one: a server that sends more is cut off, one that sends less is caught by the size check.
        let options = Options::new(entry.z_size.saturating_add(1));
        http::get_to(&url, &options, cancel, sink, progress).map_err(from_http)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_local_http_may_be_a_base() {
        assert!(base_url_allowed("https://lastrouterust.games.kyttuslab.org").is_ok());
        assert!(base_url_allowed("http://127.0.0.1:8741").is_ok());
        assert!(base_url_allowed("http://localhost:81").is_ok());
        assert!(base_url_allowed("http://[::1]:81").is_ok());
        assert!(base_url_allowed("http://lastrouterust.games.kyttuslab.org").is_err());
        assert!(base_url_allowed("http://localhost.evil.example").is_err());
        assert!(base_url_allowed("ftp://x").is_err());
        assert!(base_url_allowed("file:///etc/passwd").is_err());
    }

    #[test]
    fn urls_follow_the_site_layout() {
        let site = Site::new("https://example.org/", "", "windows-x86_64", vec![]).unwrap();
        assert_eq!(site.url("latest.json"), "https://example.org/native/windows-x86_64/latest.json");
        let beta = Site::new("https://example.org", "beta", "macos-universal", vec![]).unwrap();
        assert_eq!(beta.url("builds/3/manifest.json"), "https://example.org/native-beta/macos-universal/builds/3/manifest.json");
        assert!(Site::new("https://example.org", "../x", "p", vec![]).is_err());
    }
}
