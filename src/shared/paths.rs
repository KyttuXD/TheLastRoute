//! The per-user folder for settings, the saved run and mod packs, on each desktop system:
//! Windows `%APPDATA%/KYTTU/TheLastRoute`, macOS `~/Library/Application Support/KYTTU/TheLastRoute`,
//! Linux `$XDG_DATA_HOME/KYTTU/TheLastRoute` (default `~/.local/share/KYTTU/TheLastRoute`).

use std::path::PathBuf;

/// False in the browser build (wasm32): it has no file system, so nothing is written to disk there. Settings are kept
/// in the page's localStorage instead (the game's `app/web_storage.rs`); saves, the account and guest files and the
/// device id are never written, and their reads find nothing.
pub const WRITES_TO_DISK: bool = !cfg!(target_arch = "wasm32");

pub fn user_dir() -> Option<PathBuf> {
    if !WRITES_TO_DISK {
        return None;
    }
    let env = |k: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(target_os = "windows") {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local").join("share")))
    };
    base.map(|b| b.join("KYTTU").join("TheLastRoute"))
}

pub fn user_file(name: &str) -> Option<PathBuf> {
    user_dir().map(|d| d.join(name))
}

/// Optional root for the offline engineering references used while maintaining the port.
///
/// `KYTTU_DOCS` wins when set, otherwise the standard local Windows checkout is used when it
/// exists.  Keeping this discovery here makes the path portable without bundling or copying the
/// documentation into the game build.
pub fn docs_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(path) = env("KYTTU_DOCS") {
        return path.is_dir().then_some(path);
    }
    if cfg!(target_os = "windows") {
        let standard = PathBuf::from(r"C:\Users\Kyttu\Documents\Projects\Docs");
        if standard.is_dir() {
            return Some(standard);
        }
    }
    None
}

/// Resolve a file inside the offline docs root without escaping it via an absolute path.
pub fn docs_file(relative: impl AsRef<std::path::Path>) -> Option<PathBuf> {
    let root = docs_dir()?;
    let relative = relative.as_ref();
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return None;
    }
    let path = root.join(relative);
    path.is_file().then_some(path)
}
