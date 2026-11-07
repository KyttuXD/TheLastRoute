//! Update on start. Before the game opens, it looks for the newest build of this platform in two places:
//!
//! 1. The website's signed native builds (`updater_site.rs`, the same files the launcher installs, docs/LAUNCHER.md).
//!    No key is needed: the ed25519 signature and the file hashes are the trust.
//! 2. The old update server (vps3, `/api/updates/debug/<platform>`), used when the site has no build for this platform,
//!    cannot be reached, or fails a check. It downloads with the download key (the file `download-key` in the game's
//!    user folder) and checks the SHA-256.
//!
//! A newer build is put in place and started; the old program is kept beside it until the next start. A build made
//! without CI (LR_BUILD_NUMBER 0) never checks, so a development build is not replaced. A game started by the
//! launcher (`LR_LAUNCHER=1`) never checks either: the launcher keeps builds up to date, so the two cannot disagree.

use crate::net::config::PRIMARY_BASE_URL;
use crate::net::http::{self, Method, Request};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
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
    let dir = std::env::temp_dir().join(format!("lr-update-{}", crate::shared::task::process_id()));
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
    swap_bundle(bundle, &new_bundle)
}

/// Replaces the app bundle `bundle` with the finished one at `new_bundle` (renames, so both must be on one volume).
#[cfg(target_os = "macos")]
fn swap_bundle(bundle: &Path, new_bundle: &Path) -> Result<(), String> {
    let previous = bundle.with_extension("previous.app");
    let _ = std::fs::remove_dir_all(&previous);
    std::fs::rename(bundle, &previous).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(new_bundle, bundle) {
        let _ = std::fs::rename(&previous, bundle);
        return Err(e.to_string());
    }
    // No backup is kept: the old bundle is removed as soon as the new one is in place.
    let _ = std::fs::remove_dir_all(&previous);
    let _ = run("xattr", &["-cr", &bundle.to_string_lossy()]);
    Ok(())
}

/// The website build is unpacked beside the running bundle: put it in place of the running one.
#[cfg(target_os = "macos")]
fn install_staged_bundle(new_bundle: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = exe.parent().and_then(Path::parent).and_then(Path::parent).ok_or("the game is not in an app bundle")?;
    swap_bundle(bundle, new_bundle)
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
pub(super) fn should_update(current_build: u64, current_rev: &str, offer_build: u64, offer_rev: &str) -> bool {
    offer_build > current_build && !same_commit(current_rev, offer_rev)
}

/// The server lists the short commit hash, so one revision that starts the other counts as the same commit.
fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// What the update check is doing, for the boot screen.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    /// A development build (BUILD == 0): nothing is checked and nothing is shown.
    LocalBuild,
    /// Nothing is checked and nothing is shown: the launcher keeps this build up to date, or the site could not be asked
    /// and there is no download key for the old server.
    Idle,
    Checking,
    UpToDate(u64),
    /// Downloading build `build`. A total of 0 means the size is not known yet.
    Downloading { build: u64, got: u64, total: u64 },
    Installing,
    Failed(String),
    /// The previous run installed this build and started it.
    Updated(u64),
}

/// The shared status the check thread writes and the boot screen reads.
pub type UpdateHandle = Arc<Mutex<UpdateStatus>>;

/// The update screen's words and timings (data/app/updater.json).
pub struct UpdaterText {
    pub marker_file: String,
    pub marker_text: String,
    pub line_fade_ms: f32,
    pub line_hold_ms: f32,
    strings: serde_json::Value,
}

impl UpdaterText {
    pub fn parse(text: &str) -> Result<UpdaterText, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string).ok_or(format!("updater.json has no {k}"));
        let n = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32).ok_or(format!("updater.json has no {k}"));
        Ok(UpdaterText {
            marker_file: s("marker_file")?,
            marker_text: s("marker_text")?,
            line_fade_ms: n("line_fade_ms")?,
            line_hold_ms: n("line_hold_ms")?,
            strings: v.get("strings").cloned().ok_or("updater.json has no strings")?,
        })
    }

    /// The built-in data, parsed once.
    pub fn standard() -> &'static UpdaterText {
        static TEXT: OnceLock<UpdaterText> = OnceLock::new();
        TEXT.get_or_init(|| UpdaterText::parse(include_str!("../data/app/updater.json")).expect("updater.json parses"))
    }

    fn string(&self, key: &str) -> &str {
        self.strings.get(key).and_then(|v| v.as_str()).unwrap_or("")
    }

    /// The line the boot screen shows for a status, or None when it shows nothing.
    pub fn line(&self, status: &UpdateStatus) -> Option<String> {
        let fill = |key: &str, build: u64, percent: u32, reason: &str| {
            self.string(key)
                .replace("{build}", &build.to_string())
                .replace("{percent}", &percent.to_string())
                .replace("{reason}", reason)
        };
        let text = match status {
            UpdateStatus::LocalBuild | UpdateStatus::Idle => return None,
            UpdateStatus::Checking => fill("checking", 0, 0, ""),
            UpdateStatus::UpToDate(b) => fill("up_to_date", *b, 0, ""),
            UpdateStatus::Downloading { build, got, total } => fill("downloading", *build, percent(*got, *total), ""),
            UpdateStatus::Installing => fill("installing", 0, 0, ""),
            UpdateStatus::Failed(why) => fill("failed", 0, 0, why),
            UpdateStatus::Updated(b) => fill("updated", *b, 0, ""),
        };
        (!text.is_empty()).then_some(text)
    }

    /// Whether the status is a final one, so the line should fade out.
    pub fn is_final(status: &UpdateStatus) -> bool {
        matches!(status, UpdateStatus::UpToDate(_) | UpdateStatus::Failed(_) | UpdateStatus::Updated(_))
    }
}

/// Download progress as a whole percent, clamped to 0..=100; a total of 0 (unknown) reads 0.
pub fn percent(got: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    ((got.min(total) as u128 * 100) / total as u128) as u32
}

fn marker_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(&UpdaterText::standard().marker_file))
}

/// Writes the marker the next launch reads: the build that was just installed.
fn write_marker(path: &Path, text: &UpdaterText, build: u64) -> std::io::Result<()> {
    std::fs::write(path, text.marker_text.replace("{build}", &build.to_string()))
}

/// Reads the marker and deletes it, so "Updated to build N" shows once. None when there is no marker.
fn take_marker(path: &Path, text: &UpdaterText) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    let (head, tail) = text.marker_text.split_once("{build}")?;
    content.trim().strip_prefix(head)?.strip_suffix(tail)?.trim().parse().ok()
}

/// The status a check starts in, before any thread runs. The browser build never updates itself this way (its page
/// does, see web_update.rs).
fn initial_status(build: u64, launcher: bool) -> UpdateStatus {
    if build == 0 || cfg!(target_arch = "wasm32") {
        UpdateStatus::LocalBuild
    } else if launcher {
        UpdateStatus::Idle
    } else {
        UpdateStatus::Checking
    }
}

/// True when the launcher started this game (it sets `LR_LAUNCHER=1`).
fn started_by_launcher() -> bool {
    std::env::var("LR_LAUNCHER").is_ok_and(|v| v == "1")
}

fn set(handle: &Mutex<UpdateStatus>, status: UpdateStatus) {
    if let Ok(mut s) = handle.lock() {
        *s = status;
    }
}

/// Records the server's offer: Downloading when it replaces this build (true), otherwise UpToDate (false).
/// An "Updated" line from the last run stays rather than being replaced by "Up to date".
fn apply_offer(handle: &Mutex<UpdateStatus>, build: u64, revision: &str, offer: &Offer) -> bool {
    if should_update(build, revision, offer.build, &offer.revision) {
        set(handle, UpdateStatus::Downloading { build: offer.build, got: 0, total: 0 });
        return true;
    }
    if let Ok(mut s) = handle.lock() {
        if !matches!(*s, UpdateStatus::Updated(_)) {
            *s = UpdateStatus::UpToDate(build);
        }
    }
    false
}

/// Starts the check on a thread and returns the status the boot screen reads. When a newer build is
/// installed, the new one is started and this process ends; otherwise the game starts as normal. A failure
/// is logged, shown, and never stops the game from opening.
pub fn start_check() -> UpdateHandle {
    remove_leftover();
    let launcher = started_by_launcher();
    let handle: UpdateHandle = Arc::new(Mutex::new(initial_status(BUILD, launcher)));
    let marker = marker_path().and_then(|p| take_marker(&p, UpdaterText::standard()));
    if BUILD == 0 || cfg!(target_arch = "wasm32") {
        crate::lr_info!("update: this is a local or browser build (no CI build number), so no update check");
        return handle;
    }
    if let Some(built) = marker {
        set(&handle, UpdateStatus::Updated(built));
    }
    if launcher {
        crate::lr_info!("update: the launcher started this game and keeps builds up to date, so no update check here");
        return handle;
    }
    let key = download_key();
    let shared = handle.clone();
    crate::shared::task::detach(move || check_thread(&shared, key.as_deref()));
    handle
}

/// The website's signed builds first, then the old server (only with a download key) when the site cannot be used.
fn check_thread(handle: &Mutex<UpdateStatus>, key: Option<&str>) {
    #[cfg(not(target_arch = "wasm32"))]
    match site_update(handle) {
        SiteOutcome::Installed(build) => finish_install(build),
        SiteOutcome::UpToDate => return,
        SiteOutcome::Failed(e) => {
            crate::lr_error!("update: {e}");
            set(handle, UpdateStatus::Failed(e));
            return;
        }
        SiteOutcome::Unavailable(why) => {
            #[cfg(target_os = "macos")]
            {
                crate::lr_info!("update: the website has nothing usable ({why}); checking the nightly release on GitHub");
                match github_update(handle) {
                    Ok(true) => start_installed(),
                    Ok(false) => return,
                    Err(e) => {
                        crate::lr_error!("update: {e}");
                        set(handle, UpdateStatus::Failed(e));
                        return;
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            crate::lr_info!("update: the website has nothing usable ({why}); trying the old update server");
        }
    }
    let Some(key) = key else {
        crate::lr_info!("update: no download key in the game folder, so the old update server is not asked");
        set(handle, UpdateStatus::Idle);
        return;
    };
    legacy_check(handle, key);
}

fn finish_install(build: u64) -> ! {
    crate::lr_info!("update: installed build {build}; starting it");
    if let Some(path) = marker_path() {
        let _ = write_marker(&path, UpdaterText::standard(), build);
    }
    start_installed()
}

/// The build as the admin STATUS tab shows it: the CI build number and commit, or LOCAL for a development build.
pub fn build_label() -> String {
    let rev: String = REVISION.chars().take(7).collect();
    if BUILD == 0 {
        format!("LOCAL ({rev})")
    } else {
        format!("{BUILD} ({rev})")
    }
}

/// macOS: the nightly release on GitHub, which CI replaces on every push to main. The installed app is up to date when
/// its executable's SHA-256 equals the release file's digest. Returns Ok(true) after a new bundle is installed,
/// Ok(false) when nothing needs doing.
#[cfg(target_os = "macos")]
fn github_update(handle: &Mutex<UpdateStatus>) -> Result<bool, String> {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    let file = format!("last_route-macos-{arch}.zip");
    let url = "https://api.github.com/repos/KyttuXD/TheLastRoute/releases/tags/nightly";
    let reply = Request::new(Method::Get, url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "last-route-updater")
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| format!("GitHub did not answer ({e:?})"))?;
    if !reply.ok() {
        return Err(format!("GitHub refused the release check ({})", reply.status));
    }
    let body = http::parse_body(&reply.body);
    let asset = body
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|list| list.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(file.as_str())))
        .ok_or("the nightly release has no build for this Mac")?;
    let want = asset
        .get("digest")
        .and_then(|d| d.as_str())
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or("the release file has no checksum")?
        .to_string();
    let link = asset.get("browser_download_url").and_then(|u| u.as_str()).ok_or("the release file has no download link")?.to_string();

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mine: String = Sha256::digest(std::fs::read(&exe).map_err(|e| e.to_string())?).iter().map(|b| format!("{b:02x}")).collect();
    if mine == want {
        crate::lr_info!("update: the nightly release is the build this app already is");
        if let Ok(mut s) = handle.lock() {
            if !matches!(*s, UpdateStatus::Updated(_)) {
                *s = UpdateStatus::UpToDate(BUILD);
            }
        }
        return Ok(false);
    }

    crate::lr_info!("update: the nightly release differs from this app; downloading {file}");
    set(handle, UpdateStatus::Downloading { build: 0, got: 0, total: 0 });
    // GitHub answers a release file with a redirect to its file host, so the download follows up to 3 hops, each one
    // limited to GitHub's own hosts. The SHA-256 check below still decides whether the bytes are installed.
    let reply = Request::new(Method::Get, link)
        .redirects(3)
        .hop_guard(|next| if release_host_allowed(next) { None } else { Some(format!("the download moved to {next}")) })
        .timeout(Duration::from_secs(900))
        .limit(256 * 1024 * 1024)
        .send()
        .map_err(|e| format!("download failed: {e:?}"))?;
    if !reply.ok() {
        return Err(format!("the download was refused ({})", reply.status));
    }
    let got: String = Sha256::digest(&reply.body).iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        return Err("checksum mismatch: the update was not installed".into());
    }

    let stage = site_stage_root(true)?;
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let zip = stage.join(&file);
    std::fs::write(&zip, &reply.body).map_err(|e| e.to_string())?;
    set(handle, UpdateStatus::Installing);
    run("ditto", &["-x", "-k", zip.to_str().unwrap_or(""), stage.to_str().unwrap_or("")])?;
    let bundle = stage.join("KYTTU The Last Route.app");
    if !bundle.is_dir() {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("the release file does not hold the app".into());
    }
    let result = install_staged_bundle(&bundle);
    let _ = std::fs::remove_dir_all(&stage);
    result.map(|()| true)
}

/// The old vps3 update API: the offer, a keyed download, the SHA-256, the replacement.
fn legacy_check(handle: &Mutex<UpdateStatus>, key: &str) {
    let offer = match offer() {
        Ok(offer) => offer,
        Err(why) => {
            crate::lr_info!("update: could not read the offer from the server: {why} (this is build {BUILD})");
            set(handle, UpdateStatus::Failed(why));
            return;
        }
    };
    if !apply_offer(handle, BUILD, REVISION, &offer) {
        crate::lr_info!("{}", current_line(BUILD, REVISION, offer.build, &offer.revision));
        return;
    }
    crate::lr_info!("update: build {} is available (this is {}); downloading", offer.build, BUILD);
    let result = download(&offer, key).and_then(|bytes| {
        set(handle, UpdateStatus::Installing);
        install(&bytes, &offer.file)
    });
    match result {
        Ok(()) => finish_install(offer.build),
        Err(e) => {
            crate::lr_error!("update: {e}");
            set(handle, UpdateStatus::Failed(e));
        }
    }
}

/// What the website source did.
#[cfg(not(target_arch = "wasm32"))]
enum SiteOutcome {
    UpToDate,
    Installed(u64),
    /// No usable offer (no native build here, offline, a failed signature): the old server is next.
    Unavailable(String),
    /// A verified offer was found but could not be installed: shown to the player, no fallback.
    Failed(String),
}

#[cfg(not(target_arch = "wasm32"))]
fn site_update(handle: &Mutex<UpdateStatus>) -> SiteOutcome {
    use super::updater_site as source;
    let base = source::site_url(std::env::var("LR_UPDATE_SITE_URL").ok().as_deref());
    let keys = source::trusted_keys(&base, std::env::var("LR_NATIVE_PUBKEY").ok().as_deref());
    let site = match lr_update::fetch::Site::new(&base, "", lr_update::manifest::this_platform(), keys) {
        Ok(site) => site,
        Err(e) => return SiteOutcome::Unavailable(e),
    };
    let found = source::check(&site, |build, revision| should_update(BUILD, REVISION, build, revision), lr_update::osver::detect().as_deref());
    let m = match found {
        Err(source::Unavailable(why)) => return SiteOutcome::Unavailable(why),
        Ok(source::SiteCheck::UpToDate) => {
            crate::lr_info!("update: the website offers nothing newer than build {BUILD}");
            if let Ok(mut s) = handle.lock() {
                if !matches!(*s, UpdateStatus::Updated(_)) {
                    *s = UpdateStatus::UpToDate(BUILD);
                }
            }
            return SiteOutcome::UpToDate;
        }
        Ok(source::SiteCheck::Newer(m)) => m,
    };
    crate::lr_info!("update: the website offers build {} (this is {}); downloading {} files", m.build, BUILD, m.files.len());
    set(handle, UpdateStatus::Downloading { build: m.build, got: 0, total: 0 });
    let mac = cfg!(target_os = "macos");
    let stage_root = match site_stage_root(mac) {
        Ok(p) => p,
        Err(e) => return SiteOutcome::Failed(e),
    };
    let build = m.build;
    let staged = source::stage(&site, &m, &stage_root, mac, &mut |p| {
        set(handle, UpdateStatus::Downloading { build, got: p.got, total: p.total });
    });
    let staged = match staged {
        Ok(s) => s,
        Err(e) => return SiteOutcome::Failed(format!("download failed: {e}")),
    };
    set(handle, UpdateStatus::Installing);
    let installed = match staged {
        source::Staged::Program(bytes) => install(&bytes, &m.run),
        source::Staged::Bundle { bundle, stage_root } => {
            #[cfg(target_os = "macos")]
            let result = install_staged_bundle(&bundle);
            #[cfg(not(target_os = "macos"))]
            let result: Result<(), String> = Err(format!("{} is an app bundle, which only macOS installs", bundle.display()));
            let _ = std::fs::remove_dir_all(&stage_root);
            result
        }
    };
    match installed {
        Ok(()) => SiteOutcome::Installed(build),
        Err(e) => SiteOutcome::Failed(e),
    }
}

/// Where the website build is unpacked first. On macOS it sits beside the running bundle (so the final rename stays on one
/// volume); elsewhere in the temporary folder.
#[cfg(not(target_arch = "wasm32"))]
fn site_stage_root(mac: bool) -> Result<PathBuf, String> {
    if mac {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let bundle = exe.parent().and_then(Path::parent).and_then(Path::parent).ok_or("the game is not in an app bundle")?;
        let root = bundle.with_file_name(".lr-update-stage");
        let _ = std::fs::remove_dir_all(&root);
        Ok(root)
    } else {
        Ok(scratch_dir()?.join("site-stage"))
    }
}

/// The hosts a GitHub release download may move through: github.com, then its file hosts. Anything else refuses the hop.
#[cfg(target_os = "macos")]
fn release_host_allowed(url: &str) -> bool {
    const HOSTS: [&str; 4] = [
        "https://github.com/",
        "https://objects.githubusercontent.com/",
        "https://release-assets.githubusercontent.com/",
        "https://github-releases.githubusercontent.com/",
    ];
    HOSTS.iter().any(|h| url.starts_with(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn a_release_download_may_only_move_through_github_hosts() {
        assert!(release_host_allowed("https://github.com/KyttuXD/TheLastRoute/releases/download/nightly/x.zip"));
        assert!(release_host_allowed("https://release-assets.githubusercontent.com/github-production-release-asset/1/x?sig=a"));
        assert!(!release_host_allowed("http://github.com/x.zip"));
        assert!(!release_host_allowed("https://evil.example.com/x.zip"));
        assert!(!release_host_allowed("https://github.com.evil.example/x.zip"));
    }

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

    fn offer_of(build: u64, rev: &str) -> Offer {
        Offer { build, revision: rev.into(), file: "f".into(), sha256: "s".into() }
    }

    #[test]
    fn a_local_build_reports_local_build_and_shows_nothing() {
        assert_eq!(initial_status(0, false), UpdateStatus::LocalBuild);
        assert_eq!(initial_status(0, true), UpdateStatus::LocalBuild);
        assert_eq!(initial_status(5, false), UpdateStatus::Checking);
        assert_eq!(initial_status(5, true), UpdateStatus::Idle, "a game started by the launcher does not check");
        let text = UpdaterText::standard();
        assert!(text.line(&UpdateStatus::LocalBuild).is_none() && text.line(&UpdateStatus::Idle).is_none());
    }

    #[test]
    fn status_moves_from_checking_to_up_to_date_or_downloading() {
        let status = Mutex::new(UpdateStatus::Checking);
        assert!(!apply_offer(&status, 6, "bbbb2222", &offer_of(6, "aaaa1111")));
        assert_eq!(*status.lock().unwrap(), UpdateStatus::UpToDate(6));
        *status.lock().unwrap() = UpdateStatus::Checking;
        assert!(apply_offer(&status, 6, "bbbb2222", &offer_of(7, "aaaa1111")));
        assert_eq!(*status.lock().unwrap(), UpdateStatus::Downloading { build: 7, got: 0, total: 0 });
        set(&status, UpdateStatus::Installing);
        assert_eq!(*status.lock().unwrap(), UpdateStatus::Installing);
    }

    #[test]
    fn an_updated_line_survives_an_up_to_date_check() {
        let status = Mutex::new(UpdateStatus::Updated(7));
        assert!(!apply_offer(&status, 7, "bbbb2222", &offer_of(7, "bbbb2222")));
        assert_eq!(*status.lock().unwrap(), UpdateStatus::Updated(7));
    }

    #[test]
    fn the_percent_is_clamped_and_safe_with_no_total() {
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(0, 100), 0);
        assert_eq!(percent(42, 100), 42);
        assert_eq!(percent(500, 100), 100, "more than the total clamps to 100");
        assert_eq!(percent(u64::MAX, u64::MAX), 100);
    }

    #[test]
    fn the_marker_is_written_then_read_once_and_cleared() {
        let text = UpdaterText::standard();
        let path = std::env::temp_dir().join(format!("lr-marker-test-{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_file(&path);
        assert_eq!(take_marker(&path, text), None);
        write_marker(&path, text, 42).unwrap();
        assert!(path.exists());
        assert_eq!(take_marker(&path, text), Some(42));
        assert!(!path.exists(), "reading the marker deletes it");
        assert_eq!(take_marker(&path, text), None);
    }

    #[test]
    fn the_strings_and_timings_come_from_the_json() {
        let custom = r#"{"marker_file":"m","marker_text":"b{build}","line_fade_ms":10,"line_hold_ms":20,
            "strings":{"checking":"C?","downloading":"D {build} {percent}","installing":"I","up_to_date":"U{build}","updated":"N{build}","failed":"F {reason}"}}"#;
        let t = UpdaterText::parse(custom).unwrap();
        assert_eq!(t.line(&UpdateStatus::Checking).as_deref(), Some("C?"));
        assert_eq!(t.line(&UpdateStatus::UpToDate(3)).as_deref(), Some("U3"));
        assert_eq!(t.line(&UpdateStatus::Downloading { build: 8, got: 1, total: 4 }).as_deref(), Some("D 8 25"));
        assert_eq!(t.line(&UpdateStatus::Failed("x".into())).as_deref(), Some("F x"));
        assert_eq!(t.line_fade_ms, 10.0);
        let std = UpdaterText::standard();
        assert_eq!(std.line(&UpdateStatus::Checking).as_deref(), Some("Checking for updates"));
        assert_eq!(std.line(&UpdateStatus::Updated(9)).as_deref(), Some("Updated to build 9"));
        assert_eq!(
            std.line(&UpdateStatus::Downloading { build: 9, got: 42, total: 100 }).as_deref(),
            Some("Downloading build 9: 42%")
        );
    }
}
