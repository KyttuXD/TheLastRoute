//! Update on start. Before the game opens, it asks the server for the newest build of this platform. A newer
//! build is downloaded with the download key (the file `download-key` in the game's user folder), checked
//! against its SHA-256, put in place, and started; the old program is kept beside it. A build made without CI
//! (LR_BUILD_NUMBER 0) never checks, so a development build is not replaced.

use crate::net::config::PRIMARY_BASE_URL;
use crate::net::http::{self, Method, Request};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The CI run number this build was made in (0 for a local build).
const BUILD: u64 = parse_build(env!("LR_BUILD_NUMBER"));

/// The commit this build was made from ("unknown" when neither CI nor git was available to the build).
const REVISION: &str = env!("LR_REVISION");

const fn parse_build(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut value = 0u64;
    let mut i = 0;
    while i < bytes.len() {
        value = value * 10 + (bytes[i] - b'0') as u64;
        i += 1;
    }
    value
}

/// The platform name the server's update manifest uses.
pub const PLATFORM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

/// A newer build the server offers for this platform.
struct Offer {
    build: u64,
    revision: String,
    file: String,
    sha256: String,
}

/// Asks the server for this platform's newest build. The Err text says why there is no offer, for the log.
fn offer() -> Result<Offer, String> {
    let url = format!("{PRIMARY_BASE_URL}/api/updates/debug/{PLATFORM}");
    let reply = Request::new(Method::Get, url).timeout(Duration::from_secs(5)).send().map_err(|e| format!("the request failed ({e:?})"))?;
    if !reply.ok() {
        return Err("the server did not answer OK".into());
    }
    let body = http::parse_body(&reply.body);
    let update = body.get("update").and_then(|u| u.as_object()).ok_or("the server reply has no update entry")?;
    let text = |key: &str| update.get(key).and_then(|v| v.as_str()).map(str::to_string).ok_or(format!("the update entry has no {key}"));
    Ok(Offer {
        build: update.get("build").and_then(|v| v.as_u64()).ok_or("the update entry has no build number")?,
        revision: text("revision")?,
        file: text("file")?,
        sha256: text("sha256")?,
    })
}

/// The log line for a check that finds nothing to install: which build this is, and what the server offers.
fn current_line(build: u64, revision: &str, offered_build: u64, offered_revision: &str) -> String {
    let short = |r: &str| r.chars().take(7).collect::<String>();
    format!(
        "update: no update to install (this is build {build} of {}; the server offers build {offered_build} of {})",
        short(revision),
        short(offered_revision)
    )
}

fn download_key() -> Option<String> {
    let text = std::fs::read_to_string(crate::shared::paths::user_file("download-key")?).ok()?;
    let key = text.trim().to_string();
    (!key.is_empty()).then_some(key)
}

fn download(offer: &Offer, key: &str) -> Result<Vec<u8>, String> {
    let url = format!("{PRIMARY_BASE_URL}/api/updates/files/{}", offer.file);
    let reply = Request::new(Method::Get, url)
        .header("X-Download-Key", key)
        .timeout(Duration::from_secs(900))
        .limit(256 * 1024 * 1024)
        .send()
        .map_err(|e| format!("download failed: {e:?}"))?;
    if !reply.ok() {
        return Err(format!("the server refused the download ({})", reply.status));
    }
    let digest: String = Sha256::digest(&reply.body).iter().map(|b| format!("{b:02x}")).collect();
    if digest != offer.sha256 {
        return Err("checksum mismatch: the update was not installed".into());
    }
    Ok(reply.body)
}

fn scratch_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("lr-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program).args(args).status().map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed"))
    }
}

/// Puts the new program in place. Windows and Linux replace the executable (a running one can be renamed);
/// macOS replaces the whole app bundle, unpacked from the zip.
fn install(bytes: &[u8], file: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let work = scratch_dir()?;
    let result = install_in(&exe, bytes, file, &work);
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(target_os = "macos")]
fn install_in(exe: &Path, bytes: &[u8], file: &str, work: &Path) -> Result<(), String> {
    let bundle = exe
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or("the game is not in an app bundle")?;
    let archive = work.join(file);
    std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
    let out = work.join("out");
    run("unzip", &["-q", &archive.to_string_lossy(), "-d", &out.to_string_lossy()])?;
    let new_bundle = out.join("KYTTU The Last Route.app");
    if !new_bundle.join("Contents/MacOS/last_route").is_file() {
        return Err("the update has no game program".into());
    }
    let previous = bundle.with_extension("previous.app");
    let _ = std::fs::remove_dir_all(&previous);
    std::fs::rename(bundle, &previous).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new_bundle, bundle) {
        let _ = std::fs::rename(&previous, bundle);
        return Err(e.to_string());
    }
    // No backup is kept: the old bundle is removed as soon as the new one is in place.
    let _ = std::fs::remove_dir_all(&previous);
    let _ = run("xattr", &["-cr", &bundle.to_string_lossy()]);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn install_in(exe: &Path, bytes: &[u8], file: &str, work: &Path) -> Result<(), String> {
    let program = if file.ends_with(".tar.gz") {
        let archive = work.join(file);
        std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        run("tar", &["-xzf", &archive.to_string_lossy(), "-C", &work.to_string_lossy()])?;
        let name = if cfg!(windows) { "last_route.exe" } else { "last_route" };
        let found = std::fs::read_dir(work)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path().join("bin").join(name))
            .find(|p| p.is_file())
            .ok_or("the update has no game program")?;
        std::fs::read(found).map_err(|e| e.to_string())?
    } else {
        bytes.to_vec()
    };
    let dir = exe.parent().ok_or("the game has no folder")?;
    let staged = dir.join("last_route.update");
    std::fs::write(&staged, &program).map_err(|e| e.to_string())?;
    let previous = dir.join("last_route.previous");
    let _ = std::fs::remove_file(&previous);
    std::fs::rename(exe, &previous).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::rename(&previous, exe);
        return Err(e.to_string());
    }
    // Windows cannot delete the running program yet, so this may fail; remove_leftover clears it on the next start.
    let _ = std::fs::remove_file(&previous);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Starts the installed build and ends this process, so the new program takes over.
fn start_installed() -> ! {
    let exe = std::env::current_exe().ok();
    #[cfg(target_os = "macos")]
    {
        if let Some(bundle) = exe.as_deref().and_then(Path::parent).and_then(Path::parent).and_then(Path::parent) {
            let _ = Command::new("open").arg(bundle).spawn();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(exe) = exe {
            let _ = Command::new(exe).args(std::env::args().skip(1)).spawn();
        }
    }
    std::process::exit(0)
}

/// Removes the old program left by an earlier update, which Windows could not delete while it was running.
fn remove_leftover() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe.parent().and_then(Path::parent).and_then(Path::parent) {
        let _ = std::fs::remove_dir_all(bundle.with_extension("previous.app"));
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(dir) = exe.parent() {
        let _ = std::fs::remove_file(dir.join("last_route.previous"));
    }
}

/// Whether an offered build replaces this one. The build number must go up, so an older published build never
/// replaces a newer one, and the commit must differ, so a rebuild of the same commit does not replace itself.
fn should_update(current_build: u64, current_rev: &str, offer_build: u64, offer_rev: &str) -> bool {
    offer_build > current_build && !same_commit(current_rev, offer_rev)
}

/// The server lists the short commit hash, so one revision that starts the other counts as the same commit.
fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// Runs the check once, before the game opens. When a newer build is installed, the new one is started and
/// this process ends; otherwise it returns and the game starts as normal. A failure is logged and never stops
/// the game from opening.
pub fn check_on_start() {
    remove_leftover();
    if BUILD == 0 {
        crate::lr_info!("update: this is a local build (no CI build number), so no update check");
        return;
    }
    let Some(key) = download_key() else {
        crate::lr_info!("update: no download key in the game folder, so no update check");
        return;
    };
    let offer = match offer() {
        Ok(offer) => offer,
        Err(why) => {
            crate::lr_info!("update: could not read the offer from the server: {why} (this is build {BUILD})");
            return;
        }
    };
    if !should_update(BUILD, REVISION, offer.build, &offer.revision) {
        crate::lr_info!("{}", current_line(BUILD, REVISION, offer.build, &offer.revision));
        return;
    }
    crate::lr_info!("update: build {} is available (this is {}); downloading", offer.build, BUILD);
    match download(&offer, &key).and_then(|bytes| install(&bytes, &offer.file)) {
        Ok(()) => {
            crate::lr_info!("update: installed build {}; starting it", offer.build);
            start_installed();
        }
        Err(e) => crate::lr_error!("update: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_number_parses_from_the_build_environment() {
        assert_eq!(parse_build("0"), 0);
        assert_eq!(parse_build("9001"), 9001);
    }

    #[test]
    fn a_check_with_nothing_to_install_says_which_builds_it_compared() {
        let line = current_line(27, "1b5e3c2e23d40e1ba1cc69b917dd10efc81b3e1a", 27, "1b5e3c2e23d40e1b");
        assert!(line.contains("build 27 of 1b5e3c2") && line.contains("offers build 27"), "{line}");
    }

    #[test]
    fn an_update_needs_a_newer_build_of_another_commit() {
        let commit = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        assert!(should_update(5, "bbbb2222", 6, commit));
        assert!(!should_update(6, "bbbb2222", 5, commit), "an older build never replaces a newer one");
        assert!(!should_update(5, "bbbb2222", 5, commit), "the same build number is not an update");
        assert!(!should_update(5, commit, 6, commit), "a rebuild of the same commit is not an update");
        assert!(!should_update(5, commit, 6, "aaaa1111"), "a short hash of the same commit is not an update");
    }
}
