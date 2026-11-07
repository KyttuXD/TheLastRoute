//! Where the launcher keeps downloaded builds: a per-user folder that is NOT the game's roaming data folder
//! (`shared::paths::user_dir` is the Roaming APPDATA on Windows; a 60 MB program must not roam).
//!
//! * Windows: `%LOCALAPPDATA%\KYTTU\TheLastRoute\native`
//! * macOS: `~/Library/Application Support/KYTTU/TheLastRoute/native`
//! * Linux: `$XDG_DATA_HOME` or `~/.local/share`, then `KYTTU/TheLastRoute/native`
//!
//! A channel other than the default adds `-<channel>` to the last name. `LR_NATIVE_DIR` overrides the whole path.

use std::path::PathBuf;

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The base folder for `native` builds, or None when the system gives no home folder.
pub fn install_dir(channel: &str) -> Option<PathBuf> {
    if let Some(dir) = env_path("LR_NATIVE_DIR") {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env_path("LOCALAPPDATA").or_else(|| env_path("APPDATA"))
    } else if cfg!(target_os = "macos") {
        env_path("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env_path("XDG_DATA_HOME").or_else(|| env_path("HOME").map(|h| h.join(".local").join("share")))
    }?;
    let name = if channel.is_empty() { "native".to_string() } else { format!("native-{channel}") };
    Some(base.join("KYTTU").join("TheLastRoute").join(name))
}
