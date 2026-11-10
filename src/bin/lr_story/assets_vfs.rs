//! One asset source for lr_story: every pack file, prop, skin and animation the game reads goes through `read`,
//! `read_to_string`, `read_dir` and `is_file` here, so the same code runs on the desktop and in the browser.
//!
//! Native (desktop): the store is empty, so each call is the plain `std::fs` call it replaces. Nothing changes there.
//!
//! Browser (wasm32): there are no folders to read. `preload` runs once, before the packs load. It fetches the one bundle
//! the manifest names with macroquad's `load_file` (a single request) and keeps each listed file's bytes in the store
//! under its key. Folder listings come from the same manifest: the store keeps a sorted list of the entries of every
//! folder.
//!
//! A key is a file's path relative to the crate folder (the folder that holds Cargo.toml), with `/` separators, for
//! example `src/bin/lr_story/assets/player/anim/player_mannequin.json`. The game builds its paths from
//! `env!("CARGO_MANIFEST_DIR")`, so `key_of` strips that prefix and the keys match what the game asks for.
//!
//! The manifest and the bundle are written at web-build time by `packaging/story-web/assemble.mjs` (node, no
//! dependencies). The listed files are:
//!   - every file under the core pack, `src/bin/last_route/data/lr_story`,
//!   - every file under each mod pack in `mods/<pack id>/` (a folder with a `pack.json`; only when present),
//!   - every file under lr_story's assets, `src/bin/lr_story/assets`.
//! They are packed back to back into `vfs.bin` (its gzip copy `vfs.bin.gz` is what the server sends; the browser decodes
//! it). The manifest, `manifest.json`, is
//! `{"schema": "lr.story.manifest/2", "bundle": "vfs.bin", "files": [{"key": "...", "offset": 0, "length": 0}, ...]}`.
//! Nothing is embedded in the wasm.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub const MANIFEST_SCHEMA: &str = "lr.story.manifest/2";
/// The manifest, beside the page.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const MANIFEST_URL: &str = "manifest.json";

/// What the browser has fetched: the bytes of each file by key, and for each folder the sorted names of its entries.
/// Empty on the desktop, where the disk is read instead.
struct Store {
    files: BTreeMap<String, Vec<u8>>,
    /// folder key ("" is the crate folder) -> the names of the files and folders directly in it, sorted
    dirs: BTreeMap<String, BTreeSet<String>>,
}

static STORE: Mutex<Store> = Mutex::new(Store { files: BTreeMap::new(), dirs: BTreeMap::new() });

fn store() -> MutexGuard<'static, Store> {
    // A poisoned lock still holds whole entries (every write is one insert), so the data is used as it is.
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

impl Store {
    /// Keeps `bytes` under `key` and records the key in every folder above it.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn add(&mut self, key: &str, bytes: Vec<u8>) {
        if key.is_empty() {
            return;
        }
        // "a/b/c.json" puts "a" in the crate folder, "b" in "a" and "c.json" in "a/b"
        let mut dir = String::new();
        let mut parts = key.split('/').peekable();
        while let Some(part) = parts.next() {
            self.dirs.entry(dir.clone()).or_default().insert(part.to_string());
            if parts.peek().is_some() {
                dir = if dir.is_empty() { part.to_string() } else { format!("{dir}/{part}") };
            }
        }
        self.files.insert(key.to_string(), bytes);
    }

    /// The names of the entries of folder `dir`, sorted, or None when the store has no such folder.
    fn list(&self, dir: &str) -> Option<Vec<String>> {
        self.dirs.get(dir).map(|names| names.iter().cloned().collect())
    }
}

/// The crate folder with `/` separators and no trailing slash.
fn crate_root() -> String {
    env!("CARGO_MANIFEST_DIR").replace('\\', "/").trim_end_matches('/').to_string()
}

/// The key of a path: its path relative to the crate folder, with `/` separators and no leading or trailing slash.
/// A path outside the crate folder keeps its whole path (it will not match a key, so the fallback reads it).
pub fn key_of(path: &Path) -> String {
    let root = crate_root();
    let s = path.to_string_lossy().replace('\\', "/");
    let rel = if s == root {
        ""
    } else {
        s.strip_prefix(&format!("{root}/")).unwrap_or(&s)
    };
    rel.trim_matches('/').to_string()
}

/// The bytes of a file. The store first; on the desktop, the disk.
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    let key = key_of(path);
    let found = store().files.get(&key).cloned();
    match found {
        Some(bytes) => Ok(bytes),
        None => read_fallback(path),
    }
}

/// The text of a file, as `std::fs::read_to_string` gives it (an error when it is not UTF-8).
pub fn read_to_string(path: &Path) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The entries of a folder, sorted, as full paths (`dir.join(name)`). An error when the folder is unknown, as
/// `std::fs::read_dir` gives it.
pub fn read_dir(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let key = key_of(dir);
    let listed = store().list(&key);
    match listed {
        Some(names) => Ok(names.iter().map(|name| dir.join(name)).collect()),
        None => read_dir_fallback(dir),
    }
}

/// True when the store holds a file at `path`, or (on the desktop) the disk does.
pub fn is_file(path: &Path) -> bool {
    let key = key_of(path);
    let stored = store().files.contains_key(&key);
    stored || is_file_fallback(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_fallback(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

#[cfg(target_arch = "wasm32")]
fn read_fallback(path: &Path) -> io::Result<Vec<u8>> {
    Err(not_fetched(path))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_dir_fallback(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    out.sort();
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
fn read_dir_fallback(dir: &Path) -> io::Result<Vec<PathBuf>> {
    Err(not_fetched(dir))
}

#[cfg(not(target_arch = "wasm32"))]
fn is_file_fallback(path: &Path) -> bool {
    path.is_file()
}

#[cfg(target_arch = "wasm32")]
fn is_file_fallback(_path: &Path) -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
fn not_fetched(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} was not in the browser's game files (see manifest.json)", key_of(path)),
    )
}

/// One file in the bundle: its key and where its bytes sit in `vfs.bin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub offset: usize,
    pub length: usize,
}

/// The manifest: the bundle's name beside the page, and the files it holds, in the order the manifest lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub bundle: String,
    pub entries: Vec<Entry>,
}

/// Reads a manifest. Checks the schema, that the bundle is a plain file name, and that every key is a relative path.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let doc: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("{MANIFEST_URL}: {e}"))?;
    match doc.get("schema").and_then(|s| s.as_str()) {
        Some(MANIFEST_SCHEMA) => {}
        other => return Err(format!("{MANIFEST_URL}: schema is {other:?}, expected '{MANIFEST_SCHEMA}'")),
    }
    let bundle = doc
        .get("bundle")
        .and_then(|b| b.as_str())
        .filter(|b| !b.is_empty() && !b.contains('/') && !b.contains(".."))
        .ok_or_else(|| format!("{MANIFEST_URL}: no plain \"bundle\" file name"))?
        .to_string();
    let files = doc.get("files").and_then(|f| f.as_array()).ok_or_else(|| format!("{MANIFEST_URL}: no \"files\" list"))?;
    let entries = files
        .iter()
        .map(|f| {
            let key = f
                .get("key")
                .and_then(|k| k.as_str())
                .filter(|k| !k.is_empty() && !k.starts_with('/') && !k.split('/').any(|p| p == ".."))
                .ok_or_else(|| format!("{MANIFEST_URL}: a file entry has no relative \"key\": {f}"))?;
            let offset = f.get("offset").and_then(|o| o.as_u64()).ok_or_else(|| format!("{MANIFEST_URL}: {key} has no \"offset\""))?;
            let length = f.get("length").and_then(|l| l.as_u64()).ok_or_else(|| format!("{MANIFEST_URL}: {key} has no \"length\""))?;
            Ok(Entry { key: key.to_string(), offset: offset as usize, length: length as usize })
        })
        .collect::<Result<Vec<Entry>, String>>()?;
    Ok(Manifest { bundle, entries })
}

/// Cuts the bundle into the files the manifest lists and keeps each in the store. Refuses an entry that runs past the
/// end of the bundle (the bundle and the manifest do not match).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn unpack(bundle: &[u8], entries: &[Entry]) -> Result<usize, String> {
    let mut keep = Vec::with_capacity(entries.len());
    for entry in entries {
        let end = entry.offset.checked_add(entry.length).filter(|end| *end <= bundle.len()).ok_or_else(|| {
            format!("{MANIFEST_URL}: {} runs past the end of the game bundle ({} bytes)", entry.key, bundle.len())
        })?;
        keep.push((entry.key.clone(), bundle[entry.offset..end].to_vec()));
    }
    let count = keep.len();
    let mut s = store();
    for (key, bytes) in keep {
        s.add(&key, bytes);
    }
    Ok(count)
}

/// What the preload is doing now, for the loading screen (main.rs draws it each frame until the files are in).
#[cfg(target_arch = "wasm32")]
static STATUS: Mutex<String> = Mutex::new(String::new());

#[cfg(target_arch = "wasm32")]
fn set_status(text: String) {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = text;
}

/// The preload's current status line, e.g. "downloading 408 game files (12345678 bytes)".
#[cfg(target_arch = "wasm32")]
pub fn status() -> String {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Fetches the manifest and the one bundle it names, and keeps every listed file in the store. Call once, before the
/// packs load. Returns how many files it holds. A failure stops the load with its message; the status line shows it.
#[cfg(target_arch = "wasm32")]
pub async fn preload() -> Result<usize, String> {
    set_status(format!("fetching {MANIFEST_URL}"));
    let text = macroquad::file::load_string(MANIFEST_URL).await.map_err(|e| {
        let msg = format!("{MANIFEST_URL}: {e:?}");
        set_status(format!("failed: {msg}"));
        msg
    })?;
    let manifest = parse_manifest(&text).map_err(|e| {
        set_status(format!("failed: {e}"));
        e
    })?;
    set_status(format!("downloading {} game files", manifest.entries.len()));
    let bundle = macroquad::file::load_file(&manifest.bundle).await.map_err(|e| {
        let msg = format!("{}: {e:?}", manifest.bundle);
        set_status(format!("failed: {msg}"));
        msg
    })?;
    set_status(format!("unpacking {} bytes", bundle.len()));
    let count = unpack(&bundle, &manifest.entries).map_err(|e| {
        set_status(format!("failed: {e}"));
        e
    })?;
    set_status(format!("{count} game files ready"));
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Store {
        Store { files: BTreeMap::new(), dirs: BTreeMap::new() }
    }

    /// A file in the shipped assets folder, the same path the game asks for.
    fn shipped(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets").join(rel)
    }

    /// On the desktop the store is empty, so read gives exactly the bytes of the disk file.
    #[test]
    fn read_falls_back_to_the_disk_on_native() {
        let path = shipped("data/player/anim.json");
        let from_disk = std::fs::read(&path).expect("the shipped anim.json is on disk");
        assert_eq!(read(&path).expect("read falls back to the disk"), from_disk);
        assert_eq!(read_to_string(&path).unwrap(), String::from_utf8(from_disk).unwrap());
        assert!(is_file(&path));
        assert!(!is_file(&shipped("no/such/file.json")));
        assert!(read(&shipped("no/such/file.json")).is_err(), "a missing file is an error");
    }

    /// The listing of a folder comes out sorted, whatever order the files were added in, and a folder lists its own
    /// entries only.
    #[test]
    fn listing_from_the_manifest_is_sorted() {
        let mut s = fresh();
        for key in ["b/z.json", "a/x.png", "b/a.json", "a/c/d.json", "a/c/a.json", "top.json", "a/B.png"] {
            s.add(key, key.as_bytes().to_vec());
        }
        assert_eq!(s.list("").unwrap(), vec!["a", "b", "top.json"]);
        assert_eq!(s.list("a").unwrap(), vec!["B.png", "c", "x.png"], "byte order, so capitals first");
        assert_eq!(s.list("b").unwrap(), vec!["a.json", "z.json"]);
        assert_eq!(s.list("a/c").unwrap(), vec!["a.json", "d.json"]);
        assert!(s.list("missing").is_none(), "an unknown folder has no listing");
        assert_eq!(s.files["a/c/d.json"], b"a/c/d.json".to_vec());
    }

    /// The desktop listing of a shipped folder is sorted too, so callers see one order on both platforms.
    #[test]
    fn native_read_dir_is_sorted() {
        let entries = read_dir(&shipped("")).expect("the assets folder is on disk");
        assert!(!entries.is_empty());
        assert!(entries.windows(2).all(|w| w[0] <= w[1]), "entries are sorted");
    }

    /// Keys are relative to the crate folder with `/` separators, whichever separator the path was built with.
    #[test]
    fn keys_are_relative_to_the_crate_folder() {
        assert_eq!(key_of(&shipped("props/blocky/labStool_0.bbmodel")), "src/bin/lr_story/assets/props/blocky/labStool_0.bbmodel");
        let windows_style = PathBuf::from(format!("{}\\src\\bin\\lr_story\\assets\\a.json", env!("CARGO_MANIFEST_DIR")));
        assert_eq!(key_of(&windows_style), "src/bin/lr_story/assets/a.json");
        assert_eq!(key_of(Path::new(env!("CARGO_MANIFEST_DIR"))), "");
        assert_eq!(key_of(Path::new("C:/elsewhere/x.json")), "C:/elsewhere/x.json");
    }

    #[test]
    fn manifest_lists_the_bundle_and_its_files_in_order_and_refuses_bad_ones() {
        let good = r#"{"schema": "lr.story.manifest/2", "bundle": "vfs.bin",
            "files": [{"key": "src/a.json", "offset": 0, "length": 3}, {"key": "mods/x/pack.json", "offset": 3, "length": 2}]}"#;
        let m = parse_manifest(good).unwrap();
        assert_eq!(m.bundle, "vfs.bin");
        assert_eq!(m.entries[1], Entry { key: "mods/x/pack.json".into(), offset: 3, length: 2 });
        assert!(parse_manifest(r#"{"schema": "lr.story.manifest/1", "files": []}"#).unwrap_err().contains("schema"));
        assert!(parse_manifest(r#"{"schema": "lr.story.manifest/2", "bundle": "../vfs.bin", "files": []}"#).is_err());
        assert!(parse_manifest(r#"{"schema": "lr.story.manifest/2", "bundle": "vfs.bin", "files": [{"key": "../etc", "offset": 0, "length": 1}]}"#).is_err());
        assert!(parse_manifest(r#"{"schema": "lr.story.manifest/2", "bundle": "vfs.bin", "files": [{"key": "/abs", "offset": 0, "length": 1}]}"#).is_err());
        assert!(parse_manifest(r#"{"schema": "lr.story.manifest/2", "bundle": "vfs.bin", "files": [{"key": "a", "length": 1}]}"#).is_err());
        assert!(parse_manifest("not json").is_err());
    }

    #[test]
    fn unpack_cuts_each_file_out_of_the_bundle_and_refuses_a_short_bundle() {
        let bundle = b"abcdefgh".to_vec();
        let entries = vec![
            Entry { key: "src/one.txt".into(), offset: 0, length: 3 },
            Entry { key: "src/two.txt".into(), offset: 3, length: 5 },
        ];
        assert_eq!(unpack(&bundle, &entries).unwrap(), 2);
        assert_eq!(read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/one.txt")).unwrap(), b"abc".to_vec());
        assert_eq!(read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/two.txt")).unwrap(), b"defgh".to_vec());
        let short = vec![Entry { key: "src/three.txt".into(), offset: 6, length: 9 }];
        assert!(unpack(&bundle, &short).unwrap_err().contains("past the end"));
    }
}
