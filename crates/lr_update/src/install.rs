//! Downloads a build's files into a fresh folder, verifying each one, then unpacks them (zlib) to their final names.
//!
//! The caller (launcher or game) gives an empty `stage` folder and a `scratch` folder for the compressed downloads; on any
//! error both are removed, so a failed or interrupted install leaves nothing half-written behind. Nothing here touches a
//! running program or the state file: the launcher renames the finished stage into `builds/<N>` afterwards.

use crate::fetch::{FetchError, Site};
use crate::manifest::{FileEntry, Manifest};
use crate::verify::{self, HashingWriter};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// What the progress callback is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub build: u64,
    /// Files finished, files to do.
    pub file: usize,
    pub files: usize,
    /// Compressed bytes downloaded so far, and in all.
    pub got: u64,
    pub total: u64,
}

impl Progress {
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((self.got.min(self.total) as u128 * 100) / self.total as u128) as u32
    }
}

fn io(e: std::io::Error) -> FetchError {
    FetchError::Io(e.to_string())
}

/// The path of a manifest file below `root`. The manifest was checked by `bad_path`, so this only joins plain names.
pub fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// Fetches, verifies and unpacks one file into `stage`.
fn install_file(
    site: &Site,
    build: u64,
    entry: &FileEntry,
    stage: &Path,
    scratch: &Path,
    cancel: Option<&AtomicBool>,
    progress: &mut dyn FnMut(u64),
) -> Result<(), FetchError> {
    let z_path = scratch.join(&entry.z);
    let file = File::create(&z_path).map_err(io)?;
    let mut writer = HashingWriter::new(std::io::BufWriter::new(file));
    let mut last = 0u64;
    site.download_z(build, entry, cancel, &mut writer, &mut |got, _| {
        progress(got - last);
        last = got;
    })?;
    let (buffered, z_hash, z_len) = writer.finish();
    let file = buffered.into_inner().map_err(|e| FetchError::Io(e.to_string()))?;
    drop(file);
    if z_len != entry.z_size {
        return Err(FetchError::Untrusted(format!("{}: the download is {z_len} bytes, the manifest says {}", entry.path, entry.z_size)));
    }
    if z_hash != entry.z_sha256 {
        return Err(FetchError::Untrusted(format!("{}: the download does not match its sha256", entry.path)));
    }
    let compressed = std::fs::read(&z_path).map_err(io)?;
    // The output may not be longer than the manifest says: a zip bomb stops at the limit.
    let data = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&compressed, entry.size as usize)
        .map_err(|e| FetchError::Untrusted(format!("{}: cannot unpack ({:?})", entry.path, e.status)))?;
    drop(compressed);
    if data.len() as u64 != entry.size {
        return Err(FetchError::Untrusted(format!("{}: unpacked to {} bytes, the manifest says {}", entry.path, data.len(), entry.size)));
    }
    if verify::sha256_hex(&data) != entry.sha256 {
        return Err(FetchError::Untrusted(format!("{}: the unpacked file does not match its sha256", entry.path)));
    }
    let dest = join_rel(stage, &entry.path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    std::fs::write(&dest, &data).map_err(io)?;
    set_mode(&dest, entry.mode).map_err(io)?;
    let _ = std::fs::remove_file(&z_path);
    Ok(())
}

/// Installs the files of `manifest` into `stage` (created here; it must not exist yet). `only` limits the files (the
/// in-game updater on Windows and Linux needs just the program). On error `stage` and `scratch` are removed.
pub fn install_build(
    site: &Site,
    manifest: &Manifest,
    stage: &Path,
    scratch: &Path,
    only: Option<&dyn Fn(&str) -> bool>,
    cancel: Option<&AtomicBool>,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), FetchError> {
    let chosen: Vec<&FileEntry> = manifest.files.iter().filter(|f| only.map_or(true, |keep| keep(&f.path))).collect();
    let total: u64 = chosen.iter().map(|f| f.z_size).sum();
    let result = (|| {
        if stage.exists() {
            return Err(FetchError::Io(format!("{} already exists", stage.display())));
        }
        std::fs::create_dir_all(stage).map_err(io)?;
        std::fs::create_dir_all(scratch).map_err(io)?;
        let mut got = 0u64;
        for (i, entry) in chosen.iter().enumerate() {
            let before = got;
            let mut report = Progress { build: manifest.build, file: i, files: chosen.len(), got, total };
            progress(&report);
            install_file(site, manifest.build, entry, stage, scratch, cancel, &mut |n| {
                got += n;
                report.got = got;
                progress(&report);
            })?;
            // Whatever the callback saw, the file counts as fully downloaded now.
            got = before + entry.z_size;
        }
        progress(&Progress { build: manifest.build, file: chosen.len(), files: chosen.len(), got: total, total });
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(scratch);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}

/// Checks an installed build folder against its manifest: every file is there with the right size, and with `deep` the
/// right sha256 as well. The launcher does the quick check at every start.
pub fn verify_installed(dir: &Path, manifest: &Manifest, deep: bool) -> Result<(), String> {
    for entry in &manifest.files {
        let path = join_rel(dir, &entry.path);
        let meta = std::fs::metadata(&path).map_err(|_| format!("{} is missing", entry.path))?;
        if !meta.is_file() || meta.len() != entry.size {
            return Err(format!("{} has the wrong size", entry.path));
        }
        if deep {
            let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", entry.path))?;
            if verify::sha256_hex(&data) != entry.sha256 {
                return Err(format!("{} does not match its sha256", entry.path));
            }
        }
    }
    Ok(())
}
