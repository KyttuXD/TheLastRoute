//! The in-game updater's first source: the signed native builds on the GitHub nightly release (`native-<platform>-latest.json`, the
//! same files the launcher installs, see docs/LAUNCHER.md). The old vps3 update API (`updater.rs`) stays as the
//! fallback when the site has no build for this platform, cannot be reached, or fails a check.
//!
//! Everything that decides trust lives in the `lr_update` crate (the launcher uses the same code): the pointer, the
//! manifest hash, the ed25519 signature, file names, sizes and every file's sha256. This module only glues it to the
//! running game: which build is "newer", which files are needed, and where they are staged.
//!
//! Windows and Linux replace just the program file (`manifest.run`); macOS replaces the whole `.app` bundle, staged
//! beside the running bundle so the final rename stays on one volume.

use lr_update::fetch::{FetchError, Site};
use lr_update::install::{install_build, join_rel, Progress};
use lr_update::manifest::{self, Manifest};
use lr_update::osver;
use std::path::{Path, PathBuf};

/// The production source: the GitHub release tagged "nightly".
pub const SITE_URL: &str = lr_update::fetch::NIGHTLY_URL;

/// The site to ask. `LR_UPDATE_SITE_URL` may point at a local test site (http on loopback only); anything else is
/// ignored, so the variable cannot send a player's game to another server.
pub fn site_url(env_value: Option<&str>) -> String {
    match env_value {
        Some(url) if lr_update::fetch::base_url_allowed(url).is_ok() && is_loopback(url) => url.trim_end_matches('/').to_string(),
        _ => SITE_URL.to_string(),
    }
}

fn is_loopback(url: &str) -> bool {
    lr_update::http::host_of(url).is_some_and(|h| lr_update::http::is_loopback_host(&h))
}

/// The keys that may sign a build: those compiled into `lr_update`, plus `LR_NATIVE_PUBKEY` for a loopback test site only.
pub fn trusted_keys(base: &str, env_key: Option<&str>) -> Vec<[u8; 32]> {
    let mut keys = lr_update::verify::trusted_keys();
    if let (Some(hex), true) = (env_key, is_loopback(base)) {
        if let Some(key) = lr_update::verify::from_hex(hex).and_then(|k| <[u8; 32]>::try_from(k).ok()) {
            keys.push(key);
        }
    }
    keys
}

/// What asking the site found.
#[derive(Debug, PartialEq, Eq)]
pub enum SiteCheck {
    /// The site offers nothing newer than this build.
    UpToDate,
    /// A newer, verified build.
    Newer(Manifest),
}

/// Why the site cannot be used; the old server is tried next.
#[derive(Debug, PartialEq, Eq)]
pub struct Unavailable(pub String);

impl From<FetchError> for Unavailable {
    fn from(e: FetchError) -> Unavailable {
        Unavailable(e.to_string())
    }
}

/// Asks `site` whether it has a build that replaces this one. The decision rule is the old updater's: the build number
/// must go up and the commit must differ (`newer`), and the build must run on this system (`min_os`).
pub fn check(site: &Site, newer: impl Fn(u64, &str) -> bool, os_version: Option<&str>) -> Result<SiteCheck, Unavailable> {
    let latest = site.fetch_latest(None)?;
    // The pointer alone says "not newer" for most checks, so the manifest (and its signature) is fetched only when needed.
    // With no revision named, only the build number counts (an empty revision never equals a commit).
    if !newer(latest.build, "") {
        return Ok(SiteCheck::UpToDate);
    }
    let (m, _, _) = site.fetch_manifest(&latest, None)?;
    if !newer(m.build, &m.revision) {
        return Ok(SiteCheck::UpToDate);
    }
    if m.launcher_min > manifest::LAUNCHER_LEVEL {
        // The in-game updater is not the launcher; a manifest that needs a newer launcher is for launcher installs only.
        return Err(Unavailable(format!("build {} needs a newer launcher", m.build)));
    }
    if let (Some(need), Some(have)) = (&m.min_os, os_version) {
        if !osver::at_least(have, need) {
            return Err(Unavailable(format!("build {} needs system version {need}, this is {have}", m.build)));
        }
    }
    Ok(SiteCheck::Newer(m))
}

/// The files this system needs from `m`: on macOS everything (the bundle is replaced whole), elsewhere the program only.
pub fn wanted<'a>(m: &'a Manifest, mac: bool) -> impl Fn(&str) -> bool + 'a {
    move |path| mac || path == m.run
}

/// A downloaded build, ready to put in place.
#[derive(Debug)]
pub enum Staged {
    /// The program's bytes (Windows, Linux).
    Program(Vec<u8>),
    /// The unpacked `.app` bundle (macOS), inside `stage_root`.
    Bundle { bundle: PathBuf, stage_root: PathBuf },
}

/// Downloads and verifies the files of `m` into `stage_root` (created here, removed on failure), reporting progress.
pub fn stage(
    site: &Site,
    m: &Manifest,
    stage_root: &Path,
    mac: bool,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Staged, String> {
    let scratch = stage_root.with_extension("dl");
    let filter = wanted(m, mac);
    install_build(site, m, stage_root, &scratch, Some(&filter), None, progress).map_err(|e| e.to_string())?;
    if mac {
        let bundle_name = m.bundle().ok_or("the build is not an app bundle")?;
        let bundle = stage_root.join(bundle_name);
        if !bundle.join("Contents/MacOS").is_dir() {
            let _ = std::fs::remove_dir_all(stage_root);
            return Err("the update has no game program".into());
        }
        Ok(Staged::Bundle { bundle, stage_root: stage_root.to_path_buf() })
    } else {
        let bytes = std::fs::read(join_rel(stage_root, &m.run)).map_err(|e| e.to_string());
        let _ = std::fs::remove_dir_all(stage_root);
        Ok(Staged::Program(bytes?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_update::testsite::{BuildSpec, Fault, TestSite};
    use lr_update::verify::test_keys;

    fn site(web: &TestSite, key_seed: u8) -> Site {
        let platform = manifest::this_platform();
        Site::new(&web.url, "", platform, vec![test_keys::public(&test_keys::pair(key_seed))]).unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lr-game-update-{name}-{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn publish(web: &TestSite, build: u64, key_seed: u8) {
        let key = test_keys::pair(key_seed);
        let mut spec = BuildSpec::simple(manifest::this_platform(), build, vec![9; 4000], &key);
        spec.files = vec![("last_route.exe", vec![build as u8; 4000], 0o755), ("data/other.txt", b"x".to_vec(), 0o644)];
        spec.run = "last_route.exe";
        web.publish(&spec);
    }

    fn newer_than(build: u64, rev: &str) -> impl Fn(u64, &str) -> bool {
        let rev = rev.to_string();
        move |b, r| super::super::updater::should_update(build, &rev, b, r)
    }

    #[test]
    fn the_site_url_can_only_be_changed_to_localhost() {
        assert_eq!(site_url(None), SITE_URL);
        assert_eq!(site_url(Some("https://evil.example")), SITE_URL, "another server is ignored");
        assert_eq!(site_url(Some("http://evil.example")), SITE_URL);
        assert_eq!(site_url(Some("http://127.0.0.1:8741/")), "http://127.0.0.1:8741");
    }

    #[test]
    fn a_test_key_is_trusted_only_for_a_local_site() {
        let key = lr_update::verify::to_hex(&[3u8; 32]);
        assert!(trusted_keys("http://127.0.0.1:1", Some(&key)).contains(&[3u8; 32]));
        assert!(!trusted_keys(SITE_URL, Some(&key)).contains(&[3u8; 32]));
        assert!(!trusted_keys("http://127.0.0.1:1", Some("zz")).contains(&[3u8; 32]));
    }

    #[test]
    fn a_newer_signed_build_is_found_and_staged_as_the_program() {
        let web = TestSite::start();
        publish(&web, 8, 1);
        let s = site(&web, 1);
        let found = check(&s, newer_than(5, "aaaa"), None).unwrap();
        let SiteCheck::Newer(m) = found else { panic!("{found:?}") };
        assert_eq!(m.build, 8);
        let root = scratch("stage").join("stage");
        web.clear_hits();
        let mut last = None;
        let staged = stage(&s, &m, &root, false, &mut |p| last = Some(p.clone())).unwrap();
        let Staged::Program(bytes) = staged else { panic!() };
        assert_eq!(bytes, vec![8u8; 4000]);
        assert!(!web.hits().iter().any(|h| h.ends_with("f1.z")), "only the program is downloaded: {:?}", web.hits());
        assert_eq!(last.unwrap().percent(), 100);
        assert!(!root.exists(), "the staging folder is removed");
    }

    #[test]
    fn an_equal_or_older_build_is_up_to_date() {
        let web = TestSite::start();
        publish(&web, 5, 1);
        let s = site(&web, 1);
        assert_eq!(check(&s, newer_than(5, "bbbb"), None).unwrap(), SiteCheck::UpToDate);
        assert_eq!(check(&s, newer_than(9, "bbbb"), None).unwrap(), SiteCheck::UpToDate, "an older build never replaces a newer one");
    }

    #[test]
    fn the_same_commit_is_not_an_update() {
        let web = TestSite::start();
        publish(&web, 6, 1);
        let s = site(&web, 1);
        // the test manifest's revision is the build number in hex, 40 digits
        let same = format!("{:040x}", 6);
        assert_eq!(check(&s, newer_than(5, &same), None).unwrap(), SiteCheck::UpToDate);
    }

    #[test]
    fn no_build_a_bad_signature_or_a_dead_site_means_unavailable_so_the_old_server_is_used() {
        let web = TestSite::start();
        let s = site(&web, 1);
        assert!(check(&s, newer_than(1, "a"), None).unwrap_err().0.contains("no native build"), "nothing published");
        publish(&web, 9, 2); // signed by another key
        assert!(check(&s, newer_than(1, "a"), None).unwrap_err().0.contains("refused"));
        let mut gone = TestSite::start();
        let dead = site(&gone, 1);
        gone.stop();
        assert!(check(&dead, newer_than(1, "a"), None).unwrap_err().0.contains("offline"));
    }

    #[test]
    fn a_system_older_than_min_os_does_not_get_the_build() {
        let web = TestSite::start();
        let key = test_keys::pair(1);
        let mut spec = BuildSpec::simple(manifest::this_platform(), 7, vec![1; 100], &key);
        spec.min_os = Some("13.0");
        spec.files = vec![("last_route.exe", vec![1; 100], 0o755)];
        spec.run = "last_route.exe";
        web.publish(&spec);
        let s = site(&web, 1);
        assert!(check(&s, newer_than(1, "a"), Some("12.7.6")).unwrap_err().0.contains("system version 13.0"));
        assert!(matches!(check(&s, newer_than(1, "a"), Some("14.0")).unwrap(), SiteCheck::Newer(_)));
    }

    #[test]
    fn a_damaged_download_fails_staging_and_leaves_no_files() {
        let web = TestSite::start();
        publish(&web, 8, 1);
        let s = site(&web, 1);
        let SiteCheck::Newer(m) = check(&s, newer_than(5, "a"), None).unwrap() else { panic!() };
        web.fault(&format!("native/{}/builds/8/f0.z", manifest::this_platform()), Fault::FlipByte);
        let root = scratch("damaged").join("stage");
        assert!(stage(&s, &m, &root, false, &mut |_| {}).is_err());
        assert!(!root.exists() && !root.with_extension("dl").exists());
    }

    #[test]
    fn the_wanted_files_are_the_program_alone_except_on_a_mac() {
        let web = TestSite::start();
        publish(&web, 8, 1);
        let s = site(&web, 1);
        let SiteCheck::Newer(m) = check(&s, newer_than(5, "a"), None).unwrap() else { panic!() };
        assert!(wanted(&m, false)("last_route.exe") && !wanted(&m, false)("data/other.txt"));
        assert!(wanted(&m, true)("data/other.txt"));
    }
}
