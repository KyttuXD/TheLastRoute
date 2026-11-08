//! Mod packs imported in the browser (the MODS tab's IMPORT buttons).
//!
//! The browser has no mods folder. The player picks a pack folder or a `.zip` in the page's file chooser
//! (`shared/file_pick.rs`); this module turns the picked files into an [`ImportedPack`], checks it, and keeps it:
//!
//! * **Reading.** A folder arrives as files named `folder/sub/file`; a zip is unpacked here ([`unzip`], stored and deflate
//!   entries, bounded). A pack root is a folder holding `pack.json`; a pick may hold several packs.
//! * **Checking.** The same rules as the pack index (`lr_server`'s `validate_pack_manifest`, `docs/MODDING_SPEC.md`
//!   section 4): the manifest keys, `schema`, id, version, name, author and `overrides` forms, at most 512 files, 16 MiB a
//!   file, 128 MiB a pack. Then the pack goes through the game's own pack loader against the base content (and the packs
//!   already imported), so a hunter that cannot be parsed or an id that already exists is refused with the loader's words.
//!   Two differences from the index, both because the local loader is more lenient: models, pictures and sounds may sit in
//!   any folder (the sample pack keeps `hunters/mimic.bbmodel` beside its JSON), and a file a pack never reads (README.md,
//!   tools/) is ignored and reported instead of refused. `regions/` is refused: the region loader reads folders only.
//! * **Keeping.** An accepted pack is encoded ([`encode`]) and handed to the page, which keeps it in IndexedDB
//!   (`packaging/web/index.html`, the `lr_mods_*` plugin) and reads every stored pack back before the game starts. The game
//!   loads them with the other packs at start-up ([`load_stored_into`]), so an import or a removal takes effect when the page
//!   is reloaded. Across all imported packs at most 256 MiB is kept; the browser's own quota may be lower, in which case
//!   the save fails and the tab says so.
//!
//! Native builds keep their folder-based loading (`packs::load_all`). Nothing here runs there except in tests.

use super::packs::{self, Source};
use super::Content;
use crate::shared::file_pick::PickedFile;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The `Pack.dir` of an imported pack is `web-pack:<id>`: not a folder, a key into the imported packs in memory.
pub const VIRTUAL_PREFIX: &str = "web-pack:";

/// (The size and count limits are data: data/packs/limits.json, read by `packs::limits()`.) `MOD_LIMITS.max_text`: a name or author is cut to this.
const MAX_TEXT: usize = 120;

/// `FALLBACK_PACK_KINDS` in lr_server: the folders a pack's JSON may sit in.
const PACK_KINDS: [&str; 12] =
    ["props", "regions", "hunters", "grabs", "sounds", "ambience", "floors", "walls", "ceilings", "rooms", "deaths", "emotes"];
/// `MANIFEST_KEYS` in lr_server.
const MANIFEST_KEYS: [&str; 9] = ["schema", "id", "name", "version", "author", "description", "game_version", "overrides", "files"];
/// File types a pack may hold besides its JSON.
const ASSET_EXTENSIONS: [&str; 4] = ["bbmodel", "glb", "png", "ogg"];

/// Said when the browser cannot keep an imported pack (private window, storage blocked).
pub const NO_STORAGE: &str =
    "this browser is not allowing site storage, so an imported mod would be lost at the reload that applies it";

// ------------------------------------------------------------------------------------------- the pack

/// One imported pack: its files by pack-relative path.
#[derive(Clone, Debug, Default)]
pub struct ImportedPack {
    pub id: String,
    pub files: BTreeMap<String, Vec<u8>>,
    /// What the pack adds or replaces (hunter ids, `prop:<id>`, `loot:<id>`), found when it was checked.
    pub provides: Vec<String>,
    /// Files in the pick that a pack does not read (README.md, tools/...); not kept.
    pub ignored: Vec<String>,
}

/// The pack.json fields the tab shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub name: String,
    pub version: String,
    pub author: String,
}

impl ImportedPack {
    pub fn text(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.files.get(rel.trim_start_matches("./"))?.clone()).ok()
    }

    /// The `*.json` files directly inside `sub`, in name order.
    pub fn json_in(&self, sub: &str) -> Vec<(String, Result<String, String>)> {
        let prefix = format!("{sub}/");
        self.files
            .iter()
            .filter(|(path, _)| path.strip_prefix(&prefix).is_some_and(|rest| !rest.contains('/') && rest.ends_with(".json")))
            .map(|(path, bytes)| (path.clone(), String::from_utf8(bytes.clone()).map_err(|_| "not valid UTF-8".to_string())))
            .collect()
    }

    pub fn total_bytes(&self) -> usize {
        self.files.values().map(Vec::len).sum()
    }

    pub fn meta(&self) -> Meta {
        let m: Value = self.text("pack.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        Meta {
            name: clean(m.get("name"), MAX_TEXT),
            version: clean(m.get("version"), 32),
            author: clean(m.get("author"), MAX_TEXT),
        }
    }
}

/// `text(value, max)` of the index: control characters to spaces, trimmed, cut to `max`.
fn clean(v: Option<&Value>, max: usize) -> String {
    let raw = v.and_then(Value::as_str).unwrap_or("");
    let spaced: String = raw.chars().map(|c| if (c as u32) < 0x20 || c as u32 == 0x7f { ' ' } else { c }).collect();
    spaced.trim().chars().take(max).collect()
}

pub fn virtual_dir(id: &str) -> PathBuf {
    PathBuf::from(format!("{VIRTUAL_PREFIX}{id}"))
}

/// The pack id behind a virtual folder.
pub fn virtual_id(dir: &Path) -> Option<&str> {
    dir.to_str()?.strip_prefix(VIRTUAL_PREFIX)
}

pub fn is_virtual(dir: &Path) -> bool {
    virtual_id(dir).is_some()
}

/// A file of an imported pack: `None` when `dir` is not an imported pack's folder, `Some(None)` when the pack or the file
/// is not there.
pub fn read_virtual(dir: &Path, rel: &str) -> Option<Option<Vec<u8>>> {
    let id = virtual_id(dir)?;
    let rel = rel.trim_start_matches("./");
    let reg = lock();
    Some(reg.packs.get(id).and_then(|p| p.files.get(rel).cloned()))
}

// ------------------------------------------------------------------------------------------- checking

/// What to do with a file of a pack folder.
#[derive(Debug, PartialEq)]
enum Verdict {
    Keep,
    Ignore,
    Reject(String),
}

/// `[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}` (the index's path segment).
fn segment_ok(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    s.len() <= 64 && (first.is_ascii_alphanumeric() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

const SEGMENT_RULE: &str = "letters, digits, _ . - only, 64 characters at most, not starting with . or -";

/// Files the operating systems add to a folder or a zip; never part of a pack and not worth a word.
fn is_junk(path: &str) -> bool {
    path.split('/').any(|part| part == "__MACOSX" || part == ".DS_Store" || part == "Thumbs.db" || part.starts_with("._"))
}

fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?;
    name.rsplit_once('.').map(|(_, ext)| ext)
}

/// Whether a file name is worth reading from a pick at all: a zip, or a file type a pack holds. Anything else in a chosen
/// folder is never loaded into the game.
pub fn is_wanted_name(name: &str) -> bool {
    let name = normalize(name);
    if is_junk(&name) {
        return false;
    }
    match extension(&name).map(str::to_ascii_lowercase) {
        Some(ext) => ext == "zip" || ext == "json" || ASSET_EXTENSIONS.contains(&ext.as_str()),
        None => false,
    }
}

fn classify(path: &str) -> Verdict {
    if is_junk(path) {
        return Verdict::Ignore;
    }
    if path == "pack.json" {
        return Verdict::Keep;
    }
    let ext = extension(path).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext != "json" && !ASSET_EXTENSIONS.contains(&ext.as_str()) {
        return Verdict::Ignore;
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Verdict::Reject(format!("{path}: unsafe file path (INVALID_FILE_PATH)"));
    }
    if ext == "json" {
        // only `<kind>/<name>.json` is content; other JSON (package.json, tools/...) is not read by the game
        if parts.len() != 2 || !PACK_KINDS.contains(&parts[0]) {
            return Verdict::Ignore;
        }
        if parts[0] == "regions" {
            return Verdict::Reject(format!(
                "{path}: region packs cannot be imported in the browser yet; put it in the mods folder of the desktop game"
            ));
        }
        let stem = parts[1].strip_suffix(".json").unwrap_or(parts[1]);
        if !segment_ok(stem) || !parts[1].ends_with(".json") {
            return Verdict::Reject(format!("{path}: a file name may use {SEGMENT_RULE} (INVALID_FILE_PATH)"));
        }
        return Verdict::Keep;
    }
    let ext_exact = extension(path).unwrap_or("");
    let name_ok = parts.len() <= 8 && ext_exact == ext && parts[..parts.len() - 1].iter().all(|p| segment_ok(p)) && {
        let file = parts[parts.len() - 1];
        segment_ok(file.strip_suffix(&format!(".{ext_exact}")).unwrap_or(file))
    };
    if name_ok {
        Verdict::Keep
    } else {
        Verdict::Reject(format!(
            "{path}: a file path may use {SEGMENT_RULE}, up to 8 folders deep, with a lower case extension (INVALID_FILE_PATH)"
        ))
    }
}

/// `^[a-z0-9][a-z0-9_]{2,31}$`
fn id_ok(id: &str) -> bool {
    let b = id.as_bytes();
    (3..=32).contains(&b.len()) && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// `^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$`, no `..`
fn version_ok(v: &str) -> bool {
    let b = v.as_bytes();
    (1..=32).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-')) && !v.contains("..")
}

/// An `overrides` entry: `<kind>/<id>` (the index's form), `ambience/<id>` (colons allowed inside), or a bare id (the form
/// the local loader compares).
fn override_ok(entry: &str) -> bool {
    let id_chars = |s: &str, min: usize, max: usize| (min..=max).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    match entry.split_once('/') {
        None => id_chars(entry, 1, 64),
        Some(("ambience", id)) => {
            !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b':')
                && !id.starts_with(':')
                && !id.ends_with(':')
        }
        Some((kind, id)) => PACK_KINDS.contains(&kind) && id_chars(id, 1, 64),
    }
}

/// The index's manifest rules (`validate_pack_manifest`), minus the `files` list the index adds from the folder.
/// Returns the pack id.
fn check_manifest(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "pack.json: not valid UTF-8".to_string())?;
    let value: Value = serde_json::from_str(text).map_err(|e| format!("pack.json: not valid JSON ({e})"))?;
    let Some(m) = value.as_object() else {
        return Err("pack.json: must be a JSON object (INVALID_PACK_SCHEMA)".to_string());
    };
    if m.get("schema").and_then(Value::as_str) != Some("lr.pack/1") {
        return Err("pack.json: \"schema\" must be \"lr.pack/1\" (INVALID_PACK_SCHEMA)".to_string());
    }
    if let Some(key) = m.keys().find(|k| !MANIFEST_KEYS.contains(&k.as_str())) {
        return Err(format!("pack.json: \"{key}\" is not a pack field (UNKNOWN_PACK_FIELD); a pack holds content, never scripts"));
    }
    if let Some(overrides) = m.get("overrides") {
        let ok = overrides.as_array().is_some_and(|a| a.iter().all(|e| e.as_str().is_some_and(override_ok)));
        if !ok {
            return Err("pack.json: \"overrides\" must be a list of ids like \"hunters/mimic\" (INVALID_PACK_OVERRIDES)".to_string());
        }
    }
    let id = m.get("id").and_then(Value::as_str).unwrap_or("");
    if !id_ok(id) {
        return Err("pack.json: \"id\" must be 3 to 32 lower case letters, digits or _ (INVALID_PACK_ID)".to_string());
    }
    let version = m.get("version").and_then(Value::as_str).unwrap_or("");
    if !version_ok(version) {
        return Err("pack.json: \"version\" must be 1 to 32 letters, digits, . + or - with no \"..\" (INVALID_PACK_VERSION)".to_string());
    }
    if clean(m.get("name"), MAX_TEXT).is_empty() {
        return Err("pack.json: \"name\" is required (PACK_NAME_REQUIRED)".to_string());
    }
    if clean(m.get("author"), MAX_TEXT).is_empty() {
        return Err("pack.json: \"author\" is required (PACK_AUTHOR_REQUIRED)".to_string());
    }
    Ok(id.to_string())
}

/// A file's bytes must look like what its extension says, so a renamed or cut-off file is named instead of failing later.
fn check_content(path: &str, bytes: &[u8]) -> Result<(), String> {
    match extension(path).unwrap_or("") {
        "json" | "bbmodel" => serde_json::from_slice::<Value>(bytes).map(|_| ()).map_err(|e| format!("{path}: not valid JSON ({e})")),
        "png" if !bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) => Err(format!("{path}: not a PNG picture")),
        "ogg" if !bytes.starts_with(b"OggS") => Err(format!("{path}: not an Ogg sound")),
        "glb" if !bytes.starts_with(b"glTF") => Err(format!("{path}: not a glTF model")),
        _ => Ok(()),
    }
}

/// Builds a pack from the files of one pack folder (paths relative to it). A file that could not be read fails the pack
/// when the pack needs it; files the game never reads are ignored.
pub fn assemble(files: Vec<(String, Result<Vec<u8>, String>)>) -> Result<ImportedPack, String> {
    let mut kept: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut lower: BTreeSet<String> = BTreeSet::new();
    let mut ignored = Vec::new();
    for (path, bytes) in files {
        match classify(&path) {
            Verdict::Ignore => ignored.push(path),
            Verdict::Reject(why) => return Err(why),
            Verdict::Keep => {
                let bytes = bytes.map_err(|why| {
                    if why.contains("too big") {
                        format!("{path}: the file is over the 16 MB limit (FILE_TOO_LARGE)")
                    } else {
                        format!("{path}: {why}")
                    }
                })?;
                if bytes.len() > packs::limits().max_file_bytes {
                    return Err(format!("{path}: the file is over the 16 MB limit (FILE_TOO_LARGE)"));
                }
                if !lower.insert(path.to_lowercase()) {
                    return Err(format!("{path}: the pack holds this file twice (DUPLICATE_FILE_PATH)"));
                }
                kept.insert(path, bytes);
            }
        }
    }
    let Some(manifest) = kept.get("pack.json") else {
        return Err("pack.json is missing (PACK_JSON_MISSING)".to_string());
    };
    if kept.len() > packs::limits().max_files {
        return Err(format!("{} files; a pack may hold at most {} (TOO_MANY_FILES)", kept.len(), packs::limits().max_files));
    }
    let id = check_manifest(manifest)?;
    if kept.values().map(Vec::len).sum::<usize>() > packs::limits().max_pack_bytes {
        return Err("the pack is over the 128 MB limit (PACK_TOO_LARGE)".to_string());
    }
    for (path, bytes) in &kept {
        check_content(path, bytes)?;
    }
    ignored.sort();
    Ok(ImportedPack { id, files: kept, provides: vec![], ignored })
}

/// Loads `pack` the way the game will, on top of the base content and the other imported packs, and says what it adds.
/// The loader's own warnings are the reasons a pack is refused.
fn dry_run(pack: &ImportedPack, others: &[Arc<ImportedPack>]) -> Result<Vec<String>, String> {
    let mut scratch = Content::base();
    for other in others {
        packs::load_source(&mut scratch, &Source::Memory(other));
    }
    let (warned, loaded) = (scratch.warnings.len(), scratch.packs.len());
    packs::load_source(&mut scratch, &Source::Memory(pack));
    let problems = &scratch.warnings[warned..];
    if !problems.is_empty() {
        let mut why = problems.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
        if problems.len() > 3 {
            why.push_str(&format!(" (and {} more)", problems.len() - 3));
        }
        return Err(why);
    }
    if scratch.packs.len() == loaded {
        return Err("the game could not load the pack".to_string());
    }
    let provides = scratch.packs[loaded].provides.clone();
    if provides.is_empty() {
        return Err("the pack adds nothing the game reads: put hunters/<name>.json, props/<name>.json or a loot override in it".to_string());
    }
    Ok(provides)
}

// ------------------------------------------------------------------------------------------- reading a pick

/// One pack found in a pick, or the reason it is not one.
#[derive(Debug)]
pub struct Candidate {
    pub label: String,
    pub result: Result<ImportedPack, String>,
}

fn normalize(name: &str) -> String {
    let mut n = name.replace('\\', "/");
    loop {
        if let Some(rest) = n.strip_prefix("./") {
            n = rest.to_string();
        } else if let Some(rest) = n.strip_prefix('/') {
            n = rest.to_string();
        } else {
            return n;
        }
    }
}

/// The picked files as packs: each `.zip` is unpacked and searched, the rest (a folder, or loose files) is searched as one
/// group. Every folder that holds a `pack.json` is one pack.
pub fn candidates(files: Vec<PickedFile>) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut loose: Vec<(String, Result<Vec<u8>, String>)> = Vec::new();
    for file in files {
        let name = normalize(&file.name);
        if name.to_ascii_lowercase().ends_with(".zip") {
            let label = name.rsplit('/').next().unwrap_or(&name).to_string();
            let stem = label[..label.len() - 4].to_string();
            match file.bytes {
                Err(why) => out.push(Candidate { label, result: Err(format!("could not be read: {why}")) }),
                Ok(bytes) => match unzip(&bytes, &|n| is_wanted_name(n)) {
                    Err(why) => out.push(Candidate { label, result: Err(why) }),
                    Ok(entries) => out.extend(group(&stem, entries)),
                },
            }
        } else {
            loose.push((name, file.bytes));
        }
    }
    if !loose.is_empty() {
        let first = loose[0].0.split_once('/').map(|(top, _)| top.to_string());
        let shared = first.filter(|top| loose.iter().all(|(n, _)| n.split_once('/').is_some_and(|(t, _)| t == top)));
        let label = shared.unwrap_or_else(|| "chosen files".to_string());
        out.extend(group(&label, loose));
    }
    out
}

/// Packs inside one group of files: every folder with a `pack.json`, and what lies under it.
fn group(label: &str, entries: Vec<(String, Result<Vec<u8>, String>)>) -> Vec<Candidate> {
    let entries: Vec<(String, Result<Vec<u8>, String>)> = entries
        .into_iter()
        .map(|(n, b)| (normalize(&n), b))
        .filter(|(n, _)| !n.is_empty() && !n.ends_with('/') && !is_junk(n))
        .collect();
    let roots: BTreeSet<String> = entries
        .iter()
        .filter_map(|(n, _)| match n.strip_suffix("pack.json") {
            Some(prefix) if prefix.is_empty() || prefix.ends_with('/') => Some(prefix.to_string()),
            _ => None,
        })
        .collect();
    if roots.is_empty() {
        let bare = entries.iter().any(|(n, _)| !n.contains('/'));
        let hint = if bare && entries.len() > 1 {
            " Files chosen one by one lose their folders: choose the pack folder, or a .zip of it."
        } else {
            " Choose the pack folder, or a .zip of it."
        };
        return vec![Candidate { label: label.to_string(), result: Err(format!("no pack.json found in the chosen files.{hint}")) }];
    }
    let mut out = Vec::new();
    for root in &roots {
        // a file belongs to the deepest root above it, so a pack inside another pack's folder keeps its own files
        let own: Vec<(String, Result<Vec<u8>, String>)> = entries
            .iter()
            .filter(|(n, _)| n.starts_with(root.as_str()))
            .filter(|(n, _)| !roots.iter().any(|r| r.len() > root.len() && r.starts_with(root.as_str()) && n.starts_with(r.as_str())))
            .map(|(n, b)| (n[root.len()..].to_string(), b.clone()))
            .collect();
        let name = if root.is_empty() { label.to_string() } else { root.trim_end_matches('/').rsplit('/').next().unwrap_or(label).to_string() };
        out.push(Candidate { label: name, result: assemble(own) });
    }
    out
}

// ------------------------------------------------------------------------------------------- zip

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

fn le16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?) as usize)
}

fn le32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize)
}

/// The entries of a zip whose names `wanted` accepts: stored and deflate entries, no zip64, no passwords. Sizes are read
/// from the central directory and checked against the 16 MB file limit before anything is inflated; what an entry inflates
/// to must match what the directory says (so a lying header cannot make a bomb). A damaged entry is an `Err` of its own;
/// a damaged zip is an `Err` for the whole file.
pub fn unzip(bytes: &[u8], wanted: &dyn Fn(&str) -> bool) -> Result<Vec<(String, Result<Vec<u8>, String>)>, String> {
    let damaged = || "the .zip is damaged or is not a zip file".to_string();
    if bytes.len() < 22 {
        return Err(damaged());
    }
    let last = bytes.len() - 22;
    let first = last.saturating_sub(65535);
    let eocd = (first..=last).rev().find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(damaged)?;
    let total = le16(bytes, eocd + 10).ok_or_else(damaged)?;
    let cd_offset = le32(bytes, eocd + 16).ok_or_else(damaged)?;
    if total == 0xFFFF || cd_offset == 0xFFFF_FFFF {
        return Err("zip64 archives are not supported; make a normal .zip".to_string());
    }
    if total > packs::limits().max_listed {
        return Err(format!("the .zip holds {total} entries; at most {} are read", packs::limits().max_listed));
    }
    let mut pos = cd_offset;
    let mut budget = packs::limits().max_total_bytes;
    let mut out = Vec::new();
    for _ in 0..total {
        if bytes.get(pos..pos + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(damaged());
        }
        let flags = le16(bytes, pos + 8).ok_or_else(damaged)?;
        let method = le16(bytes, pos + 10).ok_or_else(damaged)?;
        let crc = le32(bytes, pos + 16).ok_or_else(damaged)? as u32;
        let csize = le32(bytes, pos + 20).ok_or_else(damaged)?;
        let usize_ = le32(bytes, pos + 24).ok_or_else(damaged)?;
        let nlen = le16(bytes, pos + 28).ok_or_else(damaged)?;
        let elen = le16(bytes, pos + 30).ok_or_else(damaged)?;
        let clen = le16(bytes, pos + 32).ok_or_else(damaged)?;
        let local = le32(bytes, pos + 42).ok_or_else(damaged)?;
        let raw_name = bytes.get(pos + 46..pos + 46 + nlen).ok_or_else(damaged)?;
        pos += 46 + nlen + elen + clen;
        let name = normalize(&String::from_utf8_lossy(raw_name));
        if name.is_empty() || name.ends_with('/') || !wanted(&name) {
            continue;
        }
        let entry = (|| -> Result<Vec<u8>, String> {
            if flags & 1 != 0 {
                return Err("the file is password protected".to_string());
            }
            if usize_ > packs::limits().max_file_bytes {
                return Err("the file is too big".to_string());
            }
            if bytes.get(local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
                return Err("the file's entry is damaged".to_string());
            }
            let start = local + 30 + le16(bytes, local + 26).ok_or("the file's entry is damaged")? + le16(bytes, local + 28).ok_or("the file's entry is damaged")?;
            let data = bytes.get(start..start.checked_add(csize).ok_or("the file's entry is damaged")?).ok_or("the file's entry is damaged")?;
            let inflated = match method {
                0 => data.to_vec(),
                8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, usize_).map_err(|e| format!("the file could not be unpacked ({:?})", e.status))?,
                other => return Err(format!("the file uses zip compression method {other}, which is not supported")),
            };
            if inflated.len() != usize_ || crc32(&inflated) != crc {
                return Err("the file is damaged (its checksum does not match)".to_string());
            }
            Ok(inflated)
        })();
        if entry.is_ok() {
            if usize_ > budget {
                return Err(format!("the .zip unpacks to more than {} MB", packs::limits().max_total_bytes / (1024 * 1024)));
            }
            budget -= usize_;
        }
        out.push((name, entry));
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------------- keeping

/// Where an accepted pack is kept between page loads. The page's IndexedDB in the browser; a fake in tests.
pub trait Persist {
    /// False when nothing can be kept (storage blocked or missing).
    fn available(&self) -> bool;
    /// Hands the encoded pack over to be stored under `id`. False when it was refused outright.
    fn put(&mut self, id: &str, data: &[u8]) -> bool;
    fn remove(&mut self, id: &str) -> bool;
    /// True when the last save the browser accepted later failed (storage full).
    fn save_failed(&self) -> bool;
}

const MAGIC: &[u8; 6] = b"LRMOD1";

/// A pack as bytes for storage: the magic, a file count, then each path and its bytes (lengths are u32, little endian).
pub fn encode(pack: &ImportedPack) -> Vec<u8> {
    let mut out = Vec::with_capacity(pack.total_bytes() + 64 * pack.files.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(pack.files.len() as u32).to_le_bytes());
    for (path, bytes) in &pack.files {
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// Reads [`encode`]'s bytes back. The files go through the same checks as a fresh import, so damaged or tampered storage
/// cannot put anything into the game that an import would refuse.
pub fn decode(bytes: &[u8]) -> Result<ImportedPack, String> {
    let bad = || "the stored copy is damaged".to_string();
    if bytes.get(..MAGIC.len()) != Some(&MAGIC[..]) {
        return Err(bad());
    }
    let mut pos = MAGIC.len();
    let count = le32(bytes, pos).ok_or_else(bad)?;
    pos += 4;
    if count > packs::limits().max_files {
        return Err(bad());
    }
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let plen = le32(bytes, pos).ok_or_else(bad)?;
        let path = std::str::from_utf8(bytes.get(pos + 4..pos + 4 + plen).ok_or_else(bad)?).map_err(|_| bad())?.to_string();
        pos += 4 + plen;
        let blen = le32(bytes, pos).ok_or_else(bad)?;
        let data = bytes.get(pos + 4..pos + 4 + blen).ok_or_else(bad)?.to_vec();
        pos += 4 + blen;
        files.push((path, Ok(data)));
    }
    assemble(files)
}

/// What the tab says about an import: the packs kept and the picks refused, each with its reason.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub imported: Vec<Imported>,
    pub rejected: Vec<(String, String)>,
}

#[derive(Debug, PartialEq)]
pub struct Imported {
    pub id: String,
    pub name: String,
    pub version: String,
    pub provides: Vec<String>,
    /// An earlier import of the same id was replaced.
    pub replaced: bool,
    pub ignored: Vec<String>,
}

/// One line per outcome: whether it went well, and the words.
impl Report {
    pub fn lines(&self) -> Vec<(bool, String)> {
        let mut out = Vec::new();
        for p in &self.imported {
            let verb = if p.replaced { "Updated" } else { "Imported" };
            out.push((true, format!("{verb} {} {} (adds {}). Reload the page to use it.", p.name, p.version, p.provides.len())));
            if !p.ignored.is_empty() {
                let shown: Vec<&str> = p.ignored.iter().take(3).map(String::as_str).collect();
                let more = if p.ignored.len() > 3 { format!(" and {} more", p.ignored.len() - 3) } else { String::new() };
                out.push((true, format!("{}: left out {} file(s) a pack does not use: {}{more}.", p.name, p.ignored.len(), shown.join(", "))));
            }
        }
        for (label, why) in &self.rejected {
            out.push((false, format!("Not imported, {label}: {why}")));
        }
        out
    }
}

/// The packs imported in this browser, by id.
#[derive(Default)]
pub struct Registry {
    packs: BTreeMap<String, Arc<ImportedPack>>,
}

impl Registry {
    pub fn ids(&self) -> Vec<String> {
        self.packs.keys().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<ImportedPack>> {
        self.packs.get(id).cloned()
    }

    pub fn len(&self) -> usize {
        self.packs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }

    pub fn all(&self) -> Vec<Arc<ImportedPack>> {
        self.packs.values().cloned().collect()
    }

    pub fn insert(&mut self, pack: ImportedPack) {
        self.packs.insert(pack.id.clone(), Arc::new(pack));
    }

    /// Checks each candidate (storage, the 256 MB total, a trial load) and keeps the ones that pass.
    pub fn accept(&mut self, found: Vec<Candidate>, store: &mut dyn Persist) -> Report {
        let mut report = Report::default();
        for cand in found {
            let mut pack = match cand.result {
                Ok(p) => p,
                Err(why) => {
                    report.rejected.push((cand.label, why));
                    continue;
                }
            };
            if !store.available() {
                report.rejected.push((cand.label, NO_STORAGE.to_string()));
                continue;
            }
            let others: Vec<Arc<ImportedPack>> = self.packs.values().filter(|p| p.id != pack.id).cloned().collect();
            let used: usize = others.iter().map(|p| p.total_bytes()).sum();
            if used + pack.total_bytes() > packs::limits().max_total_bytes {
                report.rejected.push((
                    cand.label,
                    format!("the browser keeps at most {} MB of imported mods; remove one first", packs::limits().max_total_bytes / (1024 * 1024)),
                ));
                continue;
            }
            match dry_run(&pack, &others) {
                Ok(provides) => pack.provides = provides,
                Err(why) => {
                    report.rejected.push((cand.label, why));
                    continue;
                }
            }
            if !store.put(&pack.id, &encode(&pack)) {
                report.rejected.push((cand.label, "the browser would not store it".to_string()));
                continue;
            }
            let meta = pack.meta();
            report.imported.push(Imported {
                id: pack.id.clone(),
                name: meta.name,
                version: meta.version,
                provides: pack.provides.clone(),
                replaced: self.packs.contains_key(&pack.id),
                ignored: std::mem::take(&mut pack.ignored),
            });
            self.insert(pack);
        }
        report
    }

    /// Forgets an imported pack (and its stored copy). Returns its name.
    pub fn remove(&mut self, id: &str, store: &mut dyn Persist) -> Option<String> {
        let pack = self.packs.remove(id)?;
        store.remove(id);
        Some(pack.meta().name)
    }
}

fn global() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn lock() -> std::sync::MutexGuard<'static, Registry> {
    global().lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs `f` on the packs imported in this browser.
pub fn with_registry<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    f(&mut lock())
}

/// The ids of the imported packs the game has loaded (they are `web-pack:` folders in `content.packs`).
pub fn loaded_ids(content: &Content) -> Vec<String> {
    content.packs.iter().filter_map(|p| virtual_id(&p.dir).map(str::to_string)).collect()
}

/// The page-reported byte length of a stored record as an allocation size; None when missing or above `packs::limits().max_pack_bytes`.
fn checked_stored_size(size: i32) -> Option<usize> {
    usize::try_from(size).ok().filter(|&n| n <= packs::limits().max_pack_bytes)
}

/// Start-up: reads the stored packs back from the page, remembers them, and loads them into `c` like folder packs.
pub fn load_stored_into(c: &mut Content) {
    load_stored_from(c, browser::stored());
}

/// [`load_stored_into`] with the stored packs given (key and bytes). A stored pack that no longer passes the checks is
/// skipped with a warning on the MODS tab.
pub fn load_stored_from(c: &mut Content, stored: Vec<(String, Vec<u8>)>) {
    let mut reg = lock();
    for (key, bytes) in stored {
        match decode(&bytes) {
            Ok(mut pack) if pack.id == key => {
                // the same trial load an import gets: a pack the loader now warns about is skipped, not half-loaded
                let others: Vec<Arc<ImportedPack>> = reg.all().into_iter().filter(|p| p.id != pack.id).collect();
                match dry_run(&pack, &others) {
                    Ok(provides) => {
                        pack.provides = provides;
                        reg.insert(pack);
                    }
                    Err(why) => c.warnings.push(format!("imported mod {key}: {why}; skipped")),
                }
            }
            Ok(pack) => c.warnings.push(format!("imported mod {key}: stored under the wrong name ({}); skipped", pack.id)),
            Err(why) => c.warnings.push(format!("imported mod {key}: {why}; skipped")),
        }
    }
    for pack in reg.all() {
        packs::load_source(c, &Source::Memory(&pack));
    }
}

// ------------------------------------------------------------------------------------------- the page

/// The page's storage bridge (`lr_mods_*`, registered by packaging/web/index.html). The page reads IndexedDB into memory
/// before the game starts, so reading is a plain call; saving is queued and finishes on its own.
#[cfg(target_arch = "wasm32")]
pub mod browser {
    use super::Persist;

    extern "C" {
        /// How many packs were stored when the page started.
        fn lr_mods_count() -> i32;
        /// Copies stored pack `i`'s id into `dst` (at most `cap` bytes); returns its length.
        fn lr_mods_id(i: u32, dst: *mut u8, cap: u32) -> i32;
        /// Stored pack `i`'s byte length, -1 when it is not there.
        fn lr_mods_size(i: u32) -> i32;
        /// Copies stored pack `i`'s bytes into `dst` (at most `cap`); returns their length.
        fn lr_mods_read(i: u32, dst: *mut u8, cap: u32) -> i32;
        /// Queues a save under `id`. 1 queued, 0 refused (no storage).
        fn lr_mods_put(id: *const u8, id_len: u32, data: *const u8, data_len: u32) -> i32;
        fn lr_mods_remove(id: *const u8, id_len: u32) -> i32;
        /// 0 no storage (nothing is kept after a reload), 1 storage works, 2 the last save failed.
        fn lr_mods_state() -> i32;
    }

    pub struct Browser;

    impl Persist for Browser {
        fn available(&self) -> bool {
            // SAFETY: no arguments, an integer out.
            unsafe { lr_mods_state() != 0 }
        }
        fn put(&mut self, id: &str, data: &[u8]) -> bool {
            // SAFETY: both slices are live for the whole call and their lengths go with them; the page copies the bytes before returning.
            unsafe { lr_mods_put(id.as_ptr(), id.len() as u32, data.as_ptr(), data.len() as u32) == 1 }
        }
        fn remove(&mut self, id: &str) -> bool {
            // SAFETY: the slice is live for the whole call.
            unsafe { lr_mods_remove(id.as_ptr(), id.len() as u32) == 1 }
        }
        fn save_failed(&self) -> bool {
            // SAFETY: no arguments, an integer out.
            unsafe { lr_mods_state() == 2 }
        }
    }

    /// Every pack the page had stored, as (id, bytes).
    pub fn stored() -> Vec<(String, Vec<u8>)> {
        // SAFETY (all calls below): plain integers in; each buffer is as long as the length passed with it, and the page copies at most that many bytes.
        let count = unsafe { lr_mods_count() }.max(0) as u32;
        let mut out = Vec::new();
        for i in 0..count {
            let mut id = vec![0u8; 256];
            let n = unsafe { lr_mods_id(i, id.as_mut_ptr(), id.len() as u32) };
            if n < 0 || n as usize > id.len() {
                continue;
            }
            id.truncate(n as usize);
            let size = unsafe { lr_mods_size(i) };
            let Some(size) = super::checked_stored_size(size) else { continue };
            let mut data = vec![0u8; size];
            let got = unsafe { lr_mods_read(i, data.as_mut_ptr(), data.len() as u32) };
            if got < 0 || got as usize != data.len() {
                continue;
            }
            if let Ok(id) = String::from_utf8(id) {
                out.push((id, data));
            }
        }
        out
    }
}

/// Native builds have no page: nothing is stored and nothing is kept.
#[cfg(not(target_arch = "wasm32"))]
pub mod browser {
    use super::Persist;

    pub struct Browser;

    impl Persist for Browser {
        fn available(&self) -> bool {
            false
        }
        fn put(&mut self, _id: &str, _data: &[u8]) -> bool {
            false
        }
        fn remove(&mut self, _id: &str) -> bool {
            false
        }
        fn save_failed(&self) -> bool {
            false
        }
    }

    pub fn stored() -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
}

/// A store that keeps packs in memory, for tests of this module and of the tab.
#[cfg(test)]
pub mod testing {
    use super::Persist;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct Memory {
        pub blocked: bool,
        pub failing: bool,
        pub kept: BTreeMap<String, Vec<u8>>,
    }

    impl Persist for Memory {
        fn available(&self) -> bool {
            !self.blocked
        }
        fn put(&mut self, id: &str, data: &[u8]) -> bool {
            self.kept.insert(id.to_string(), data.to_vec());
            true
        }
        fn remove(&mut self, id: &str) -> bool {
            self.kept.remove(id).is_some()
        }
        fn save_failed(&self) -> bool {
            self.failing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Memory;
    use super::*;

    const MIMIC_JSON: &[u8] = include_bytes!("../../../../docs/examples/mod-hunter/hunters/mimic.json");
    const MIMIC_MODEL: &[u8] = include_bytes!("../../../../docs/examples/mod-hunter/hunters/mimic.bbmodel");
    const MIMIC_ATLAS: &[u8] = include_bytes!("../../../../docs/examples/mod-hunter/hunters/mimic_atlas.png");
    const MIMIC_PACK: &[u8] = include_bytes!("../../../../docs/examples/mod-hunter/pack.json");
    const SERVER_SOURCE: &str = include_str!("../../lr_server/packs/mod_packs.rs");

    fn pack_json(id: &str) -> String {
        format!(r#"{{"schema":"lr.pack/1","id":"{id}","name":"Neon Pack","version":"1.0.0","author":"KT"}}"#)
    }

    fn hunter_json(id: &str) -> String {
        format!(
            r#"{{"schema":"lr.hunter/1","id":"{id}","name":"{id}","tier":"intermediate","category":"az","speed":0.9,"chase":3.7,"prog":0.2,"sight":7.0,"hearing":1.3,"spawn":14,"chase_style":"pump","taser_immune":false,"melee_immune":false}}"#
        )
    }

    fn picked(files: &[(&str, Vec<u8>)]) -> Vec<PickedFile> {
        files.iter().map(|(n, b)| PickedFile { name: n.to_string(), bytes: Ok(b.clone()) }).collect()
    }

    /// A folder pick of a good pack: names start with the folder's name, like the page's webkitRelativePath.
    fn good_folder(folder: &str, id: &str, hunter: &str) -> Vec<PickedFile> {
        picked(&[
            (&format!("{folder}/pack.json"), pack_json(id).into_bytes()),
            (&format!("{folder}/hunters/{hunter}.json"), hunter_json(hunter).into_bytes()),
        ])
    }

    fn import(files: Vec<PickedFile>, reg: &mut Registry, store: &mut Memory) -> Report {
        reg.accept(candidates(files), store)
    }

    fn only_reason(report: &Report) -> String {
        assert!(report.imported.is_empty(), "the pack should have been refused: {report:?}");
        assert_eq!(report.rejected.len(), 1, "{report:?}");
        report.rejected[0].1.clone()
    }

    // ---- zip fixtures: a writer for stored and deflate entries

    fn zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in entries {
            let packed = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            let crc = crc32(data);
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    // ---- a good pack

    #[test]
    fn a_good_pack_folder_is_kept_and_loads_into_the_game() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let report = import(good_folder("neon_pack", "neon_pack", "neonling"), &mut reg, &mut store);
        assert!(report.rejected.is_empty(), "{report:?}");
        assert_eq!(report.imported.len(), 1);
        let kept = &report.imported[0];
        assert_eq!((kept.id.as_str(), kept.name.as_str(), kept.version.as_str()), ("neon_pack", "Neon Pack", "1.0.0"));
        assert_eq!(kept.provides, ["neonling"]);
        assert!(!kept.replaced);
        assert_eq!(reg.ids(), ["neon_pack"]);
        assert!(store.kept.contains_key("neon_pack"), "the page was asked to keep it");
        assert!(report.lines()[0].1.contains("Reload the page"), "{:?}", report.lines());

        // the game's loader takes it from memory, like a folder pack
        let pack = reg.get("neon_pack").unwrap();
        let mut c = Content::base();
        let (base_hunters, base_warnings) = (c.hunters.len(), c.warnings.len());
        packs::load_source(&mut c, &Source::Memory(&pack));
        assert!(c.warnings[base_warnings..].is_empty(), "{:?}", &c.warnings[base_warnings..]);
        assert_eq!(c.hunters.len(), base_hunters + 1);
        assert!(c.hunter("neonling").is_some());
        assert_eq!(c.packs.last().unwrap().dir, virtual_dir("neon_pack"));
        assert_eq!(loaded_ids(&c), ["neon_pack"]);
    }

    #[test]
    fn the_sample_pack_in_the_docs_imports_and_its_extra_files_are_left_out() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let files = picked(&[
            ("mod-hunter/pack.json", MIMIC_PACK.to_vec()),
            ("mod-hunter/hunters/mimic.json", MIMIC_JSON.to_vec()),
            ("mod-hunter/hunters/mimic.bbmodel", MIMIC_MODEL.to_vec()),
            ("mod-hunter/hunters/mimic_atlas.png", MIMIC_ATLAS.to_vec()),
            ("mod-hunter/README.md", b"# sample".to_vec()),
            ("mod-hunter/tools/make_skin.mjs", b"//".to_vec()),
            ("mod-hunter/.DS_Store", vec![0, 1]),
            ("__MACOSX/mod-hunter/._pack.json", vec![0, 1]),
        ]);
        let report = import(files, &mut reg, &mut store);
        assert!(report.rejected.is_empty(), "{report:?}");
        let kept = &report.imported[0];
        assert_eq!(kept.id, "mimic_pack");
        assert_eq!(kept.provides, ["mimic"]);
        assert_eq!(kept.ignored, ["README.md", "tools/make_skin.mjs"], "OS junk is dropped without a word");
        let pack = reg.get("mimic_pack").unwrap();
        assert!(pack.files.contains_key("hunters/mimic.bbmodel") && pack.files.contains_key("hunters/mimic_atlas.png"));
    }

    #[test]
    fn an_imported_packs_files_are_served_from_memory_through_read_pack_file() {
        let pack = assemble(vec![
            ("pack.json".into(), Ok(pack_json("mem_serve").into_bytes())),
            ("hunters/a.json".into(), Ok(hunter_json("memserve").into_bytes())),
            ("hunters/a.bbmodel".into(), Ok(b"{}".to_vec())),
        ])
        .unwrap();
        with_registry(|r| r.insert(pack));
        let dir = virtual_dir("mem_serve");
        assert_eq!(packs::read_pack_file(&dir, "hunters/a.bbmodel"), Some(b"{}".to_vec()));
        assert_eq!(packs::read_pack_file(&dir, "./hunters/a.bbmodel"), Some(b"{}".to_vec()));
        assert_eq!(packs::read_pack_file(&dir, "hunters/missing.bbmodel"), None);
        assert_eq!(packs::read_pack_file(&dir, "../outside.json"), None, "no path climbs out of an imported pack");
        assert_eq!(packs::read_pack_file(&virtual_dir("not_imported"), "pack.json"), None);
        assert_eq!(read_virtual(Path::new("mods/other"), "pack.json"), None, "a folder pack is not an imported one");
    }

    // ---- bad packs are refused with the real reason

    #[test]
    fn a_pack_with_a_bad_manifest_is_refused_with_the_key_and_code() {
        let cases: [(&str, &str); 8] = [
            (r#"{"id":"neon_pack","name":"N","version":"1","author":"A"}"#, "INVALID_PACK_SCHEMA"),
            (r#"{"schema":"lr.pack/1","id":"neon_pack","name":"N","version":"1","author":"A","scripts":["x.gd"]}"#, "UNKNOWN_PACK_FIELD"),
            (r#"{"schema":"lr.pack/1","id":"Neon-Pack","name":"N","version":"1","author":"A"}"#, "INVALID_PACK_ID"),
            (r#"{"schema":"lr.pack/1","id":"neon_pack","name":"N","version":"1..2","author":"A"}"#, "INVALID_PACK_VERSION"),
            (r#"{"schema":"lr.pack/1","id":"neon_pack","name":"  ","version":"1","author":"A"}"#, "PACK_NAME_REQUIRED"),
            (r#"{"schema":"lr.pack/1","id":"neon_pack","name":"N","version":"1"}"#, "PACK_AUTHOR_REQUIRED"),
            (r#"{"schema":"lr.pack/1","id":"neon_pack","name":"N","version":"1","author":"A","overrides":["hunters/../x"]}"#, "INVALID_PACK_OVERRIDES"),
            (r#"["not","an","object"]"#, "INVALID_PACK_SCHEMA"),
        ];
        for (manifest, code) in cases {
            let (mut reg, mut store) = (Registry::default(), Memory::default());
            let files = picked(&[("p/pack.json", manifest.as_bytes().to_vec()), ("p/hunters/a.json", hunter_json("aa").into_bytes())]);
            let why = only_reason(&import(files, &mut reg, &mut store));
            assert!(why.starts_with("pack.json:") && why.contains(code), "{manifest} gave {why}");
            assert!(reg.is_empty() && store.kept.is_empty(), "a refused pack is not kept");
        }
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let why = only_reason(&import(picked(&[("p/pack.json", b"{nope".to_vec())]), &mut reg, &mut store));
        assert!(why.starts_with("pack.json: not valid JSON"), "{why}");
        let why = only_reason(&import(picked(&[("p/hunters/a.json", hunter_json("aa").into_bytes())]), &mut reg, &mut store));
        assert!(why.starts_with("no pack.json found"), "{why}");
    }

    #[test]
    fn files_chosen_one_by_one_are_told_to_pick_the_folder() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let files = picked(&[("a.json", b"{}".to_vec()), ("b.png", b"x".to_vec())]);
        let why = only_reason(&import(files, &mut reg, &mut store));
        assert!(why.contains("lose their folders") && why.contains("or a .zip"), "{why}");
    }

    #[test]
    fn a_pack_the_game_cannot_load_is_refused_with_the_loaders_words() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        // an unreadable hunter
        let files = picked(&[("p/pack.json", pack_json("neon_pack").into_bytes()), ("p/hunters/bad.json", br#"{"schema":"lr.hunter/1","id":""}"#.to_vec())]);
        let why = only_reason(&import(files, &mut reg, &mut store));
        assert!(why.starts_with("neon_pack:"), "{why}");
        // a hunter id the base game already has, not listed in overrides
        let base_id = Content::base().hunters[0].id.clone();
        let files = picked(&[("p/pack.json", pack_json("neon_pack").into_bytes()), ("p/hunters/dupe.json", hunter_json(&base_id).into_bytes())]);
        let why = only_reason(&import(files, &mut reg, &mut store));
        assert!(why.contains("already exists") && why.contains("overrides"), "{why}");
        // a pack that adds nothing
        let files = picked(&[("p/pack.json", pack_json("neon_pack").into_bytes())]);
        let why = only_reason(&import(files, &mut reg, &mut store));
        assert!(why.contains("adds nothing"), "{why}");
        assert!(reg.is_empty());
    }

    #[test]
    fn a_pack_file_that_breaks_a_rule_is_named() {
        let manifest = pack_json("neon_pack").into_bytes();
        let hunter = hunter_json("neonling").into_bytes();
        let cases: Vec<(Vec<(&str, Vec<u8>)>, &str)> = vec![
            (vec![("p/regions/a.json", b"{}".to_vec())], "regions/a.json: region packs cannot be imported"),
            (vec![("p/assets/My Skin.png", b"x".to_vec())], "My Skin.png: a file path may use"),
            (vec![("p/assets/skin.PNG", b"x".to_vec())], "lower case extension"),
            (vec![("p/assets/skin.png", b"not a png".to_vec())], "assets/skin.png: not a PNG picture"),
            (vec![("p/assets/hit.ogg", b"nope".to_vec())], "assets/hit.ogg: not an Ogg sound"),
            (vec![("p/assets/m.glb", b"nope".to_vec())], "assets/m.glb: not a glTF model"),
            (vec![("p/assets/m.bbmodel", b"{broken".to_vec())], "assets/m.bbmodel: not valid JSON"),
            (vec![("p/hunters/Bad Name.json", b"{}".to_vec())], "hunters/Bad Name.json: a file name may use"),
            (vec![("p/hunters/a.json", b"{}".to_vec()), ("p/hunters/A.json", b"{}".to_vec())], "DUPLICATE_FILE_PATH"),
        ];
        for (extra, expect) in cases {
            let mut files = picked(&[("p/pack.json", manifest.clone()), ("p/hunters/neonling.json", hunter.clone())]);
            files.extend(picked(&extra));
            let (mut reg, mut store) = (Registry::default(), Memory::default());
            let why = only_reason(&import(files, &mut reg, &mut store));
            assert!(why.contains(expect), "expected {expect:?} in {why:?}");
        }
    }

    #[test]
    fn the_size_and_count_limits_are_enforced() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        // a file the chooser refused for its size (16 MB cap)
        let mut files = good_folder("p", "neon_pack", "neonling");
        files.push(PickedFile { name: "p/assets/big.png".into(), bytes: Err("the file is too big".into()) });
        let why = only_reason(&import(files, &mut reg, &mut store));
        assert!(why.contains("assets/big.png") && why.contains("16 MB") && why.contains("FILE_TOO_LARGE"), "{why}");
        // a file over the cap that did arrive
        let mut files = good_folder("p", "neon_pack", "neonling");
        files.push(PickedFile { name: "p/assets/big.png".into(), bytes: Ok(vec![0; packs::limits().max_file_bytes + 1]) });
        assert!(only_reason(&import(files, &mut reg, &mut store)).contains("FILE_TOO_LARGE"));
        // too many files
        let mut files = good_folder("p", "neon_pack", "neonling");
        files.extend((0..packs::limits().max_files).map(|i| PickedFile { name: format!("p/assets/f{i}.png"), bytes: Ok(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) }));
        let why = only_reason(&import(files, &mut reg, &mut store));
        assert!(why.contains("TOO_MANY_FILES"), "{why}");
        // all imported mods together
        let mut full = ImportedPack { id: "huge_pack".into(), ..Default::default() };
        full.files.insert("pack.json".into(), vec![0; packs::limits().max_total_bytes - 10]);
        reg.insert(full);
        let why = only_reason(&import(good_folder("p", "neon_pack", "neonling"), &mut reg, &mut store));
        assert!(why.contains("256 MB") && why.contains("remove one first"), "{why}");
    }

    #[test]
    fn storage_that_cannot_keep_a_pack_refuses_it() {
        let (mut reg, mut store) = (Registry::default(), Memory { blocked: true, ..Default::default() });
        let why = only_reason(&import(good_folder("p", "neon_pack", "neonling"), &mut reg, &mut store));
        assert_eq!(why, NO_STORAGE);
        assert!(reg.is_empty());
    }

    // ---- several packs, replacing, removing

    #[test]
    fn a_folder_of_packs_gives_each_pack_and_the_bad_one_is_named_alone() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let files = picked(&[
            ("mods/one/pack.json", pack_json("pack_one").into_bytes()),
            ("mods/one/hunters/a.json", hunter_json("hunter_one").into_bytes()),
            ("mods/two/pack.json", pack_json("pack_two").into_bytes()),
            ("mods/two/hunters/a.json", hunter_json("hunter_two").into_bytes()),
            ("mods/broken/pack.json", b"{".to_vec()),
            ("mods/notes.txt", b"hi".to_vec()),
        ]);
        let report = import(files, &mut reg, &mut store);
        assert_eq!(reg.ids(), ["pack_one", "pack_two"]);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(report.rejected[0].0, "broken");
        assert!(report.rejected[0].1.starts_with("pack.json: not valid JSON"));
        assert_eq!(report.lines().len(), 3);
        assert!(report.lines().iter().filter(|(good, _)| !good).count() == 1);
    }

    #[test]
    fn a_second_pack_cannot_add_a_hunter_an_imported_one_already_added() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        assert!(import(good_folder("one", "pack_one", "shared_hunter"), &mut reg, &mut store).rejected.is_empty());
        let why = only_reason(&import(good_folder("two", "pack_two", "shared_hunter"), &mut reg, &mut store));
        assert!(why.contains("already exists"), "{why}");
        assert_eq!(reg.ids(), ["pack_one"]);
    }

    #[test]
    fn importing_a_pack_again_replaces_it_and_removal_forgets_it() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        import(good_folder("a", "neon_pack", "neonling"), &mut reg, &mut store);
        let first = store.kept["neon_pack"].clone();
        let report = import(good_folder("b", "neon_pack", "neonling"), &mut reg, &mut store);
        assert!(report.rejected.is_empty(), "the same hunter again is the pack itself, not a clash: {report:?}");
        assert!(report.imported[0].replaced);
        assert!(report.lines()[0].1.starts_with("Updated"));
        assert_eq!(reg.len(), 1);
        assert_eq!(store.kept["neon_pack"], first, "the same files encode the same");

        assert_eq!(reg.remove("neon_pack", &mut store), Some("Neon Pack".to_string()));
        assert!(reg.is_empty());
        assert!(store.kept.is_empty(), "the stored copy is removed with it");
        assert_eq!(reg.remove("neon_pack", &mut store), None, "nothing left to remove");
        // and it can be imported again
        assert!(import(good_folder("a", "neon_pack", "neonling"), &mut reg, &mut store).rejected.is_empty());
    }

    // ---- keeping across page loads

    #[test]
    fn a_stored_pack_comes_back_and_loads_at_start_up() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        import(good_folder("p", "reload_pack", "reloadling"), &mut reg, &mut store);
        let stored: Vec<(String, Vec<u8>)> = store.kept.clone().into_iter().collect();
        let back = decode(&stored[0].1).unwrap();
        assert_eq!(back.files, reg.get("reload_pack").unwrap().files);
        let mut c = Content::base();
        let base_warnings = c.warnings.len();
        load_stored_from(&mut c, stored);
        assert!(c.warnings[base_warnings..].is_empty(), "{:?}", &c.warnings[base_warnings..]);
        assert!(c.hunter("reloadling").is_some(), "the stored hunter is in the game");
        assert!(loaded_ids(&c).contains(&"reload_pack".to_string()));
        assert_eq!(with_registry(|r| r.get("reload_pack").is_some()), true, "its files are served from memory");
    }

    #[test]
    fn damaged_or_tampered_storage_is_skipped_with_a_warning() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        import(good_folder("p", "tamper_pack", "tamperling"), &mut reg, &mut store);
        let good = store.kept["tamper_pack"].clone();
        assert!(decode(b"garbage").is_err());
        assert!(decode(&good[..good.len() / 2]).is_err(), "cut off");
        // a stored pack with a script field in its manifest fails the import checks again
        let evil = encode(&ImportedPack {
            id: "evil_pack".into(),
            files: [("pack.json".to_string(), br#"{"schema":"lr.pack/1","id":"evil_pack","name":"E","version":"1","author":"A","scripts":["x"]}"#.to_vec())].into(),
            ..Default::default()
        });
        assert!(decode(&evil).unwrap_err().contains("UNKNOWN_PACK_FIELD"));
        let mut c = Content::base();
        load_stored_from(&mut c, vec![("evil_pack".into(), evil), ("wrong_key".into(), good)]);
        assert!(c.warnings.iter().any(|w| w.contains("evil_pack") && w.contains("skipped")), "{:?}", c.warnings);
        assert!(c.warnings.iter().any(|w| w.contains("wrong_key") && w.contains("wrong name")), "{:?}", c.warnings);
        assert!(!loaded_ids(&c).contains(&"evil_pack".to_string()));
    }

    #[test]
    fn a_stored_size_above_the_pack_cap_is_refused_before_allocating() {
        assert_eq!(checked_stored_size(-1), None);
        assert_eq!(checked_stored_size(i32::MAX), None);
        assert_eq!(checked_stored_size(1024), Some(1024));
        assert_eq!(checked_stored_size(packs::limits().max_pack_bytes as i32), Some(packs::limits().max_pack_bytes));
    }

    #[test]
    fn a_stored_pack_the_loader_now_warns_about_is_skipped() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        import(good_folder("p", "stale_pack", "staleling"), &mut reg, &mut store);
        let mut pack = decode(&store.kept["stale_pack"]).unwrap();
        pack.files.insert("hunters/broken.json".to_string(), b"{ not json".to_vec());
        let mut c = Content::base();
        load_stored_from(&mut c, vec![("stale_pack".into(), encode(&pack))]);
        assert!(c.warnings.iter().any(|w| w.contains("stale_pack") && w.contains("skipped")), "{:?}", c.warnings);
        assert!(!loaded_ids(&c).contains(&"stale_pack".to_string()));
    }

    // ---- zip files

    #[test]
    fn a_zip_with_a_folder_inside_imports_and_so_does_one_with_the_pack_at_its_root() {
        let hunter = hunter_json("zipling").into_bytes();
        let pack = pack_json("zip_pack").into_bytes();
        let nested = zip(&[
            ("zip_pack/pack.json", &pack, true),
            ("zip_pack/hunters/zipling.json", &hunter, true),
            ("zip_pack/hunters/", b"", false),
            ("zip_pack/README.md", b"read me", false),
            ("__MACOSX/zip_pack/._pack.json", b"x", false),
        ]);
        let flat = zip(&[("pack.json", &pack, false), ("hunters/zipling.json", &hunter, true)]);
        for (name, bytes) in [("zip_pack.zip", nested), ("Downloads/flat.ZIP", flat)] {
            let (mut reg, mut store) = (Registry::default(), Memory::default());
            let report = import(picked(&[(name, bytes)]), &mut reg, &mut store);
            assert!(report.rejected.is_empty(), "{name}: {report:?}");
            assert_eq!(reg.ids(), ["zip_pack"], "{name}");
            let pack = reg.get("zip_pack").unwrap();
            assert_eq!(pack.files["hunters/zipling.json"], hunter, "the deflated entry came out whole");
        }
    }

    #[test]
    fn several_zips_give_several_packs() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let z = |id: &str, h: &str| zip(&[("pack.json", pack_json(id).as_bytes(), true), ("hunters/h.json", hunter_json(h).as_bytes(), true)]);
        let report = import(picked(&[("a.zip", z("pack_aa", "hunter_aa")), ("b.zip", z("pack_bb", "hunter_bb")), ("c.zip", b"not a zip at all, just text".to_vec())]), &mut reg, &mut store);
        assert_eq!(reg.ids(), ["pack_aa", "pack_bb"]);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(report.rejected[0], ("c.zip".to_string(), "the .zip is damaged or is not a zip file".to_string()));
    }

    #[test]
    fn a_damaged_zip_entry_and_a_lying_header_are_caught() {
        let data = b"hello world, hello world, hello world".to_vec();
        let all = |_: &str| true;
        // a flipped byte in a stored entry breaks the checksum
        let mut stored = zip(&[("a.json", &data, false)]);
        stored[30 + 6 + 3] ^= 0xff;
        let entries = unzip(&stored, &all).unwrap();
        assert!(entries[0].1.as_ref().unwrap_err().contains("checksum"));
        // a header that claims a small size for a big deflate stream cannot inflate past it
        let big = vec![7u8; 100_000];
        let mut bomb = zip(&[("a.json", &big, true)]);
        let claimed = 10u32.to_le_bytes();
        // the central directory's uncompressed size (at 24 into the entry) and the local one (at 22)
        let cd = bomb.windows(4).position(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        bomb[cd + 24..cd + 28].copy_from_slice(&claimed);
        let entries = unzip(&bomb, &all).unwrap();
        assert!(entries[0].1.is_err(), "inflating past the declared size is refused");
        // a declared size over the file limit is refused without inflating
        let mut huge = zip(&[("a.json", &data, false)]);
        let cd = huge.windows(4).position(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        huge[cd + 24..cd + 28].copy_from_slice(&((packs::limits().max_file_bytes as u32) + 1).to_le_bytes());
        assert_eq!(unzip(&huge, &all).unwrap()[0].1, Err("the file is too big".to_string()));
        // not a zip, and a zip cut short
        assert!(unzip(b"PK nothing", &all).is_err());
        let cut = zip(&[("a.json", &data, false)]);
        assert!(unzip(&cut[..cut.len() - 30], &all).is_err());
        // unwanted names are not even read
        let entries = unzip(&zip(&[("movie.mp4", b"vid", false), ("a.json", &data, false)]), &|n| is_wanted_name(n)).unwrap();
        assert_eq!(entries.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), ["a.json"]);
    }

    #[test]
    fn a_zip_path_cannot_climb_out_of_the_pack() {
        let (mut reg, mut store) = (Registry::default(), Memory::default());
        let evil = zip(&[("pack.json", pack_json("slip_pack").as_bytes(), false), ("hunters/../../evil.png", b"x", false)]);
        let why = only_reason(&import(picked(&[("slip.zip", evil)]), &mut reg, &mut store));
        assert!(why.contains("unsafe file path"), "{why}");
    }

    // ---- the rules match the pack index

    #[test]
    fn the_limits_and_kinds_match_the_pack_index_in_lr_server() {
        let kinds = PACK_KINDS.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", ");
        assert!(SERVER_SOURCE.contains(&format!("&[{kinds}]")), "PACK_KINDS drifted from FALLBACK_PACK_KINDS in lr_server");
        assert!(SERVER_SOURCE.contains(&format!("max_file_bytes: {} * 1024 * 1024,", packs::limits().max_file_bytes / (1024 * 1024))));
        assert!(SERVER_SOURCE.contains(&format!("max_pack_bytes: {} * 1024 * 1024,", packs::limits().max_pack_bytes / (1024 * 1024))));
        assert!(SERVER_SOURCE.contains(&format!("max_files: {},", packs::limits().max_files)));
        assert!(SERVER_SOURCE.contains(&format!("max_text: {MAX_TEXT},")));
        let keys = MANIFEST_KEYS.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", ");
        assert!(SERVER_SOURCE.contains(&format!("[&str; 9] = [{keys}]")), "MANIFEST_KEYS drifted");
    }

    #[test]
    fn id_version_and_override_forms_follow_the_index() {
        for ok in ["abc", "a1_b", "neon_halls", "0day"] {
            assert!(id_ok(ok), "{ok}");
        }
        for bad in ["ab", "Abc", "_abc", "a-b", &"a".repeat(33)] {
            assert!(!id_ok(bad), "{bad}");
        }
        for ok in ["1", "1.0.0", "v2+beta-1"] {
            assert!(version_ok(ok), "{ok}");
        }
        for bad in ["", ".1", "1..2", "1 0", &"1".repeat(33)] {
            assert!(!version_ok(bad), "{bad}");
        }
        for ok in ["hunters/mimic", "props/officePrinter", "ambience/room:chapel", "mimic", "regions/mall"] {
            assert!(override_ok(ok), "{ok}");
        }
        for bad in ["", "hunters/", "nokind/x", "ambience/:x", "ambience/x:", "hunters/a/b", "a b"] {
            assert!(!override_ok(bad), "{bad}");
        }
    }

    #[test]
    fn only_files_a_pack_uses_are_read_from_a_pick() {
        for yes in ["p/pack.json", "p/hunters/a.json", "p/assets/x.PNG", "p/m.bbmodel", "p/s.ogg", "p/m.glb", "p.zip", "P.ZIP"] {
            assert!(is_wanted_name(yes), "{yes}");
        }
        for no in ["p/README.md", "p/movie.mp4", "p/.DS_Store", "__MACOSX/p/._pack.json", "p/Thumbs.db", "p/noext", "p/._skin.png"] {
            assert!(!is_wanted_name(no), "{no}");
        }
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }
}
