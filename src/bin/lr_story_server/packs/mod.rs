//! The story packs the server knows: one folder per pack id under `LR_STORY_PACKS_DIR`.
//!
//! The server reads each pack itself, so a client cannot claim a pack is multiplayer or pick its fingerprint. The
//! fingerprint is the one `lr_story pack <dir>` prints (a sha256 over every file's path, a NUL, its bytes and a NUL, the
//! files sorted by path). A pack is multiplayer only when its `pack.json` says `"story": {"multiplayer": true}`. A
//! pack allows a room of one player unless `"story": {"allow_solo": false}`.


use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct PackInfo {
    pub id: String,
    pub version: String,
    /// Lower-case hex sha256 of the folder, as `lr_story pack` prints it.
    pub fingerprint: String,
    pub multiplayer: bool,
    pub allow_solo: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PackError {
    /// The id is not a valid pack id (so it cannot name a folder).
    Invalid(String),
    /// No such pack folder here.
    Unknown,
    /// The folder exists but its manifest cannot be used.
    Unreadable(String),
}

/// lr_story's pack id rule: lower-case letters, digits and `_`, starting with a letter, 3 to 32 characters.
pub fn is_pack_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (3..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

pub struct PackRegistry {
    dir: PathBuf,
}

impl PackRegistry {
    pub fn new(dir: PathBuf) -> PackRegistry {
        PackRegistry { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Reads pack `id` from its folder: the manifest, the multiplayer flags and the fingerprint.
    pub fn load(&self, id: &str) -> Result<PackInfo, PackError> {
        if !is_pack_id(id) {
            return Err(PackError::Invalid(format!("'{id}' is not a valid pack id")));
        }
        let folder = self.dir.join(id);
        let manifest_path = folder.join("pack.json");
        if !manifest_path.is_file() {
            return Err(PackError::Unknown);
        }
        let manifest = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .ok_or_else(|| PackError::Unreadable(format!("{} is not readable JSON", manifest_path.display())))?;
        if manifest.get("id").and_then(Value::as_str) != Some(id) {
            return Err(PackError::Unreadable(format!("{} names another pack id", manifest_path.display())));
        }
        let story = manifest.get("story");
        let multiplayer = story.and_then(|s| s.get("multiplayer")) == Some(&Value::Bool(true));
        let allow_solo = story.and_then(|s| s.get("allow_solo")) != Some(&Value::Bool(false));
        let (fingerprint, _, _) = fingerprint(&folder).map_err(PackError::Unreadable)?;
        Ok(PackInfo {
            id: id.to_string(),
            version: manifest.get("version").and_then(Value::as_str).unwrap_or("0.0.0").to_string(),
            fingerprint,
            multiplayer,
            allow_solo,
        })
    }
}

/// The fingerprint of a pack folder: (sha256 hex, file count, total bytes). Same algorithm as `lr_story pack`.
pub fn fingerprint(dir: &Path) -> Result<(String, usize, u64), String> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    for (rel, path) in &files {
        let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        bytes += data.len() as u64;
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update(&data);
        hasher.update([0u8]);
    }
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok((hex, files.len(), bytes))
}

/// Every file under `dir` as (its path relative to `root` with `/` separators, its path).
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let read = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in read {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.push((parts.join("/"), path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_story_packs_{name}_{}", util::hex(&util::random_bytes(6))));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pack_ids_follow_lr_story_rules() {
        assert!(is_pack_id("party_pack"));
        assert!(is_pack_id("abc"));
        for bad in ["ab", "Party", "1party", "../etc", "a-b", "has space", &"a".repeat(33)] {
            assert!(!is_pack_id(bad), "{bad}");
        }
    }

    #[test]
    fn fingerprint_is_sha256_over_sorted_paths_and_bytes_with_nul_separators() {
        let dir = temp_dir("hash");
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::write(dir.join("pack.json"), b"{}").unwrap();
        fs::write(dir.join("scenes/main.json"), b"abc").unwrap();
        // sorted by path: "pack.json" < "scenes/main.json"
        let mut hasher = Sha256::new();
        for (rel, data) in [("pack.json", &b"{}"[..]), ("scenes/main.json", &b"abc"[..])] {
            hasher.update(rel.as_bytes());
            hasher.update([0u8]);
            hasher.update(data);
            hasher.update([0u8]);
        }
        let expected: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let (got, files, bytes) = fingerprint(&dir).unwrap();
        assert_eq!((got.as_str(), files, bytes), (expected.as_str(), 2, 5));
        fs::write(dir.join("scenes/main.json"), b"abd").unwrap();
        assert_ne!(fingerprint(&dir).unwrap().0, expected, "an edited file changes the fingerprint");
    }

    #[test]
    fn a_pack_reads_its_multiplayer_and_solo_flags() {
        let root = temp_dir("flags");
        let folder = root.join("party_pack");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("pack.json"), r#"{"id":"party_pack","version":"1.2.0","story":{"multiplayer":true}}"#).unwrap();
        let registry = PackRegistry::new(root.clone());
        let info = registry.load("party_pack").unwrap();
        assert!(info.multiplayer && info.allow_solo);
        assert_eq!(info.version, "1.2.0");
        fs::write(folder.join("pack.json"), r#"{"id":"party_pack","story":{"multiplayer":true,"allow_solo":false}}"#).unwrap();
        assert!(!registry.load("party_pack").unwrap().allow_solo);
        fs::write(folder.join("pack.json"), r#"{"id":"party_pack","story":{"multiplayer":"yes"}}"#).unwrap();
        assert!(!registry.load("party_pack").unwrap().multiplayer, "only a true boolean enables multiplayer");
        assert_eq!(registry.load("nope_pack"), Err(PackError::Unknown));
        assert!(matches!(registry.load("../x"), Err(PackError::Invalid(_))));
    }
}
