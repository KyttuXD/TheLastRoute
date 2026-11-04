//! Port of `backend/mod_packs.mjs`: the mod pack index (VPS 3). Lists the mod packs the game may download, with a
//! sha256 and size for every file, and lets an owner block a pack. The files themselves are never served from here:
//! they live on VPS 2 under `<LASTROUTE_MOD_DOWNLOAD_BASE>/<pack_id>/<version>/<file path>` and the client checks
//! every downloaded file against the sha256 listed here. Follows the DATA lane's loader contract (`lr.pack/1`): a
//! pack the client would refuse is refused here at deploy time.
//!
//! The store (`{"version":1,"packs":[...]}`) is byte-compatible with the Node server's `mod_packs.json`.

use crate::util::{self, Clock, Reply};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::OnceLock;

/// `MOD_PACK_ID_PATTERN` (no hyphen: content ids inside a pack are `<pack_id>_<name>` and may only use [A-Za-z0-9_]).
pub const MOD_PACK_ID_PATTERN: &str = r"^[a-z0-9][a-z0-9_]{2,31}$";
pub const MOD_PACK_SCHEMA: &str = "lr.pack/1";
pub const MOD_VERSION_PATTERN: &str = r"^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$";
const MANIFEST_KEYS: [&str; 9] = ["schema", "id", "name", "version", "author", "description", "game_version", "overrides", "files"];
/// What the game accepts when neither `pack_kinds.json` nor a repo checkout is found (see `pack_kinds`).
pub const FALLBACK_PACK_KINDS: &[&str] =
    &["props", "regions", "hunters", "grabs", "sounds", "ambience", "floors", "walls", "ceilings", "rooms", "deaths", "emotes"];
const SEGMENT: &str = r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}";

/// `MOD_LIMITS`.
#[derive(Clone, Debug, PartialEq)]
pub struct ModLimits {
    /// one file (the client loader's limits: a bigger pack would be refused there)
    pub max_file_bytes: u64,
    /// all files of one version
    pub max_pack_bytes: u64,
    /// files per version
    pub max_files: usize,
    /// packs in the index
    pub max_packs: usize,
    /// older versions drop out of the index (their files stay on VPS 2 until cleaned)
    pub max_versions_kept: usize,
    /// name / author
    pub max_text: usize,
    pub max_description: usize,
}

pub const MOD_LIMITS: ModLimits = ModLimits {
    max_file_bytes: 16 * 1024 * 1024,
    max_pack_bytes: 128 * 1024 * 1024,
    max_files: 512,
    max_packs: 2000,
    max_versions_kept: 5,
    max_text: 120,
    max_description: 1000,
};

impl Default for ModLimits {
    fn default() -> Self {
        MOD_LIMITS
    }
}

/// `MOD_PACK_KINDS = loadPackKinds(FALLBACK_PACK_KINDS)`, resolved once.
pub fn mod_pack_kinds() -> &'static [String] {
    static KINDS: OnceLock<Vec<String>> = OnceLock::new();
    KINDS.get_or_init(|| crate::pack_kinds::load_pack_kinds(FALLBACK_PACK_KINDS))
}

struct Patterns {
    id: Regex,
    version: Regex,
    sha256: Regex,
    file_path: Regex,
    override_id: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let kind_group = mod_pack_kinds().join("|");
        Patterns {
            id: Regex::new(MOD_PACK_ID_PATTERN).unwrap(),
            version: Regex::new(MOD_VERSION_PATTERN).unwrap(),
            sha256: Regex::new(r"^[a-f0-9]{64}$").unwrap(),
            file_path: Regex::new(&format!(
                r"^(?:pack\.json|(?:{kind_group})/{SEGMENT}\.json|assets/(?:{SEGMENT}/){{0,6}}{SEGMENT}\.(?:bbmodel|glb|png|ogg))$"
            ))
            .unwrap(),
            // "<kind>/<id>"; only ambience ids may contain ":" (e.g. "ambience/room:chapel"), never at either end
            override_id: Regex::new(&format!(
                r"^(?:(?:{kind_group})/[A-Za-z0-9_]{{1,64}}|ambience/[A-Za-z0-9_](?:[A-Za-z0-9_:]{{0,62}}[A-Za-z0-9_])?)$"
            ))
            .unwrap(),
        }
    })
}

/// `MOD_PACK_ID_PATTERN.test(id)`.
pub fn is_pack_id(id: &str) -> bool {
    patterns().id.is_match(id)
}

// ---------------------------------------------------------------------------------------------------------------
// JavaScript string helpers shared by this module, `pack_kinds` and `city_maps`.

/// `a < b ? -1 : a > b ? 1 : 0` on JavaScript strings (UTF-16 code unit order).
pub fn js_str_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// An approximation of `a.localeCompare(b)` (ICU root collation, as Node uses with no locale): punctuation and
/// symbols sort before digits, digits before letters; letters compare case- and accent-insensitively first, then by
/// accent, then lower case before upper case. Exact for ASCII; common Latin-1 accented letters are decomposed.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let (pa, sa, ta) = collation_keys(a);
    let (pb, sb, tb) = collation_keys(b);
    pa.cmp(&pb).then_with(|| sa.cmp(&sb)).then_with(|| ta.cmp(&tb))
}

const PUNCTUATION_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

fn mark_weight(c: char) -> u32 {
    match c as u32 {
        0x301 => 1,
        0x300 => 2,
        0x306 => 3,
        0x302 => 4,
        0x30C => 5,
        0x30A => 6,
        0x308 => 7,
        0x30B => 8,
        0x303 => 9,
        0x307 => 10,
        0x327 => 12,
        cp => 20 + (cp - 0x300),
    }
}

/// Latin-1 precomposed letters -> (base letters, secondary weight).
fn decompose_latin1(c: char) -> Option<(&'static str, u32)> {
    let acute = 1;
    let grave = 2;
    let circumflex = 4;
    let ring = 6;
    let diaeresis = 7;
    let tilde = 9;
    let cedilla = 12;
    let stroke = 13;
    Some(match c {
        'À' => ("A", grave),
        'Á' => ("A", acute),
        'Â' => ("A", circumflex),
        'Ã' => ("A", tilde),
        'Ä' => ("A", diaeresis),
        'Å' => ("A", ring),
        'Æ' => ("AE", 0),
        'Ç' => ("C", cedilla),
        'È' => ("E", grave),
        'É' => ("E", acute),
        'Ê' => ("E", circumflex),
        'Ë' => ("E", diaeresis),
        'Ì' => ("I", grave),
        'Í' => ("I", acute),
        'Î' => ("I", circumflex),
        'Ï' => ("I", diaeresis),
        'Ñ' => ("N", tilde),
        'Ò' => ("O", grave),
        'Ó' => ("O", acute),
        'Ô' => ("O", circumflex),
        'Õ' => ("O", tilde),
        'Ö' => ("O", diaeresis),
        'Ø' => ("O", stroke),
        'Ù' => ("U", grave),
        'Ú' => ("U", acute),
        'Û' => ("U", circumflex),
        'Ü' => ("U", diaeresis),
        'Ý' => ("Y", acute),
        'ß' => ("ss", 0),
        'à' => ("a", grave),
        'á' => ("a", acute),
        'â' => ("a", circumflex),
        'ã' => ("a", tilde),
        'ä' => ("a", diaeresis),
        'å' => ("a", ring),
        'æ' => ("ae", 0),
        'ç' => ("c", cedilla),
        'è' => ("e", grave),
        'é' => ("e", acute),
        'ê' => ("e", circumflex),
        'ë' => ("e", diaeresis),
        'ì' => ("i", grave),
        'í' => ("i", acute),
        'î' => ("i", circumflex),
        'ï' => ("i", diaeresis),
        'ñ' => ("n", tilde),
        'ò' => ("o", grave),
        'ó' => ("o", acute),
        'ô' => ("o", circumflex),
        'õ' => ("o", tilde),
        'ö' => ("o", diaeresis),
        'ø' => ("o", stroke),
        'ù' => ("u", grave),
        'ú' => ("u", acute),
        'û' => ("u", circumflex),
        'ü' => ("u", diaeresis),
        'ý' => ("y", acute),
        'ÿ' => ("y", diaeresis),
        _ => return None,
    })
}

fn collation_keys(text: &str) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut primary = Vec::new();
    let mut secondary = Vec::new();
    let mut tertiary = Vec::new();
    let push_base = |c: char, accent: u32, primary: &mut Vec<u32>, secondary: &mut Vec<u32>, tertiary: &mut Vec<u32>| {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let upper = c != lower;
        let weight = if let Some(index) = PUNCTUATION_ORDER.find(lower) {
            1 + index as u32
        } else if lower.is_ascii_digit() {
            100 + (lower as u32 - '0' as u32)
        } else if lower.is_ascii_lowercase() {
            200 + (lower as u32 - 'a' as u32)
        } else if lower.is_whitespace() {
            1
        } else {
            0x10000 + lower as u32
        };
        primary.push(weight);
        secondary.push(0);
        if accent > 0 {
            secondary.push(accent);
        }
        tertiary.push(u32::from(upper));
    };
    for c in text.chars() {
        let cp = c as u32;
        if cp < 0x20 || (0x7f..0xa0).contains(&cp) {
            continue; // control characters are ignorable
        }
        if (0x300..0x370).contains(&cp) {
            secondary.push(mark_weight(c));
            continue;
        }
        if let Some((base, accent)) = decompose_latin1(c) {
            let count = base.chars().count();
            for (i, b) in base.chars().enumerate() {
                push_base(b, if i + 1 == count { accent } else { 0 }, &mut primary, &mut secondary, &mut tertiary);
            }
            continue;
        }
        push_base(c, 0, &mut primary, &mut secondary, &mut tertiary);
    }
    (primary, secondary, tertiary)
}

/// `Object.keys(object)` order: array-index keys ascending first, then the others in insertion order.
pub fn js_object_keys(map: &Map<String, Value>) -> Vec<&String> {
    let index_of = |key: &str| -> Option<u64> {
        if key.is_empty() || key.len() > 10 || !key.bytes().all(|b| b.is_ascii_digit()) || (key.len() > 1 && key.starts_with('0')) {
            return None;
        }
        key.parse::<u64>().ok().filter(|n| *n < 4_294_967_295)
    };
    let mut indexed: Vec<(u64, &String)> = map.keys().filter_map(|k| index_of(k).map(|n| (n, k))).collect();
    indexed.sort_by_key(|(n, _)| *n);
    let mut keys: Vec<&String> = indexed.into_iter().map(|(_, k)| k).collect();
    keys.extend(map.keys().filter(|k| index_of(k).is_none()));
    keys
}

/// `${value}` in a template literal (undefined -> "undefined").
fn template(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(v) => util::js_string_of(v),
    }
}

/// `text(value, max)`: `String(value ?? "")`, control characters to spaces, trimmed, cut to `max` UTF-16 units.
fn text(value: Option<&Value>, max: usize) -> String {
    let raw = util::str_nullish(value);
    let replaced: String = raw.chars().map(|c| if (c as u32) < 0x20 || c as u32 == 0x7f { ' ' } else { c }).collect();
    util::js_slice(util::js_trim(&replaced), max)
}

/// `packIdentity(files)`: sha256 of `"<file sha256> <path>"` lines, sorted by path, joined by "\n", over every file
/// including pack.json (the client's cache / room handshake key).
pub fn pack_identity(files: &[Value]) -> String {
    let mut sorted: Vec<&Value> = files.iter().collect();
    sorted.sort_by(|a, b| js_str_cmp(&template(util::field(a, "path")), &template(util::field(b, "path"))));
    let lines: Vec<String> = sorted
        .iter()
        .map(|file| format!("{} {}", template(util::field(file, "sha256")), template(util::field(file, "path"))))
        .collect();
    util::sha256_hex(lines.join("\n"))
}

fn pack_identity_of(files: Option<&Value>) -> String {
    match files {
        Some(Value::Array(items)) => pack_identity(items),
        _ => pack_identity(&[]),
    }
}

/// A refused manifest: `{ error, path? }`.
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestError {
    pub error: &'static str,
    pub path: Option<String>,
}

fn refuse(error: &'static str) -> Result<Value, ManifestError> {
    Err(ManifestError { error, path: None })
}

fn refuse_at(error: &'static str, path: &str) -> Result<Value, ManifestError> {
    Err(ManifestError { error, path: Some(path.to_string()) })
}

/// `Number.isSafeInteger(Number(x))` on an already coerced number.
fn is_safe_integer_f64(f: f64) -> bool {
    f.is_finite() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0
}

/// `validatePackManifest(input, limits)`: Ok(pack) with keys id, version, name, author, description, gameVersion,
/// overrides, files, totalBytes, sha256 -- or the error code (and offending path).
pub fn validate_pack_manifest(input: &Value, limits: &ModLimits) -> Result<Value, ManifestError> {
    let empty = Map::new();
    let manifest = match input {
        Value::Object(map) => map,
        _ => &empty, // arrays are objects in JS too, but they have no "schema"
    };
    let p = patterns();
    if manifest.get("schema") != Some(&Value::String(MOD_PACK_SCHEMA.into())) {
        return refuse("INVALID_PACK_SCHEMA");
    }
    if let Some(unknown) = js_object_keys(manifest).into_iter().find(|key| !MANIFEST_KEYS.contains(&key.as_str())) {
        return refuse_at("UNKNOWN_PACK_FIELD", unknown);
    }
    if let Some(overrides) = manifest.get("overrides") {
        let valid = match overrides {
            Value::Array(entries) => entries.iter().all(|entry| p.override_id.is_match(&util::js_string_of(entry))),
            _ => false,
        };
        if !valid {
            return refuse("INVALID_PACK_OVERRIDES");
        }
    }
    let id = util::str_or_empty(manifest.get("id"));
    let version = util::str_or_empty(manifest.get("version"));
    if !p.id.is_match(&id) {
        return refuse("INVALID_PACK_ID");
    }
    if !p.version.is_match(&version) || version.contains("..") {
        return refuse("INVALID_PACK_VERSION");
    }
    let name = text(manifest.get("name"), limits.max_text);
    let author = text(manifest.get("author"), limits.max_text);
    if name.is_empty() {
        return refuse("PACK_NAME_REQUIRED");
    }
    if author.is_empty() {
        return refuse("PACK_AUTHOR_REQUIRED");
    }
    let entries = match manifest.get("files") {
        Some(Value::Array(entries)) if !entries.is_empty() => entries,
        _ => return refuse("PACK_FILES_REQUIRED"),
    };
    if entries.len() > limits.max_files {
        return refuse("TOO_MANY_FILES");
    }
    let mut seen: Vec<String> = Vec::new();
    let mut files: Vec<(String, String, f64)> = Vec::new();
    let mut total = 0f64;
    for entry in entries {
        let path = util::str_or_empty(util::field(entry, "path"));
        let sha256 = util::str_or_empty(util::field(entry, "sha256")).to_lowercase();
        let size = util::js_number(util::field(entry, "size"));
        if !p.file_path.is_match(&path) || path.split('/').any(|part| part == "." || part == "..") {
            return refuse_at("INVALID_FILE_PATH", &path);
        }
        let lower = path.to_lowercase();
        if seen.contains(&lower) {
            return refuse_at("DUPLICATE_FILE_PATH", &path);
        }
        seen.push(lower);
        if !p.sha256.is_match(&sha256) {
            return refuse_at("INVALID_FILE_SHA256", &path);
        }
        if !is_safe_integer_f64(size) || size < 0.0 {
            return refuse_at("INVALID_FILE_SIZE", &path);
        }
        if size > limits.max_file_bytes as f64 {
            return refuse_at("FILE_TOO_LARGE", &path);
        }
        total += size;
        files.push((path, sha256, size));
    }
    if total > limits.max_pack_bytes as f64 {
        return refuse("PACK_TOO_LARGE");
    }
    if !seen.iter().any(|path| path == "pack.json") {
        return refuse("PACK_JSON_MISSING");
    }
    files.sort_by(|a, b| js_str_cmp(&a.0, &b.0));
    let files: Vec<Value> =
        files.into_iter().map(|(path, sha256, size)| json!({ "path": path, "sha256": sha256, "size": util::num(size) })).collect();
    let identity = pack_identity(&files);
    let game_version = match manifest.get("game_version") {
        Some(v) if !v.is_null() => Some(v),
        _ => manifest.get("gameVersion"),
    };
    let overrides: Vec<Value> = match manifest.get("overrides") {
        Some(Value::Array(entries)) => entries.iter().map(|entry| Value::String(util::js_string_of(entry))).collect(),
        _ => Vec::new(),
    };
    Ok(json!({
        "id": id,
        "version": version,
        "name": name,
        "author": author,
        "description": text(manifest.get("description"), limits.max_description),
        "gameVersion": text(game_version, 32),
        "overrides": overrides,
        "files": files,
        "totalBytes": util::num(total),
        "sha256": identity,
    }))
}

/// `createModPacks({...})` options.
pub struct ModPacksOptions {
    /// `mod_packs.json`; None keeps the index in memory only.
    pub store_path: Option<PathBuf>,
    /// public URL of the VPS 2 mod folder (`LASTROUTE_MOD_DOWNLOAD_BASE`); "" = not configured
    pub download_base: String,
    pub limits: ModLimits,
    pub now: Clock,
}

impl Default for ModPacksOptions {
    fn default() -> Self {
        ModPacksOptions { store_path: None, download_base: String::new(), limits: MOD_LIMITS, now: util::system_clock() }
    }
}

/// The mod pack index (`createModPacks`).
pub struct ModPacks {
    store_path: Option<PathBuf>,
    base: String,
    limits: ModLimits,
    now: Clock,
    /// id -> { id, blocked, blockedReason, blockedBy, blockedAt, versions: [newest first] }, in insertion order
    packs: Map<String, Value>,
}

fn not_found() -> Reply {
    (404, json!({ "ok": false, "code": "PACK_NOT_FOUND", "error": "No such mod pack" }))
}

impl ModPacks {
    pub fn new(options: ModPacksOptions) -> ModPacks {
        let base = options.download_base.trim_end_matches('/').to_string();
        let mut packs = Map::new();
        if let Some(path) = &options.store_path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_str::<Value>(&String::from_utf8_lossy(&bytes)) {
                    Ok(stored) => {
                        if let Some(Value::Array(values)) = util::field(&stored, "packs") {
                            for value in values {
                                let id = util::str_or_empty(util::field(value, "id"));
                                if value.is_object()
                                    && is_pack_id(&id)
                                    && matches!(util::field(value, "versions"), Some(Value::Array(_)))
                                {
                                    packs.insert(id, value.clone());
                                }
                            }
                        }
                    }
                    Err(error) => util::log_error(&format!("Mod pack store read failed: {error}")),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => util::log_error(&format!("Mod pack store read failed: {error}")),
            }
        }
        ModPacks { store_path: options.store_path, base, limits: options.limits, now: options.now, packs }
    }

    fn persist(&self) {
        let Some(path) = &self.store_path else { return };
        let body = json!({ "version": 1, "packs": self.packs.values().cloned().collect::<Vec<_>>() });
        if let Err(error) = util::write_atomic(path, util::stringify(&body).as_bytes()) {
            util::log_error(&format!("Mod pack store write failed: {error}"));
        }
    }

    fn public_view(&self, record: &Value, version: Option<&str>) -> Option<Value> {
        let versions: &[Value] = match util::field(record, "versions") {
            Some(Value::Array(items)) => items,
            _ => &[],
        };
        let chosen = match version {
            Some(v) => versions.iter().find(|entry| util::field(entry, "version") == Some(&Value::String(v.to_string()))),
            None => versions.first(),
        }?;
        if !util::truthy(Some(chosen)) {
            return None;
        }
        let mut view = Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                view.insert(key.to_string(), value);
            }
        };
        let get = |key: &str| util::field(chosen, key).cloned();
        put("id", util::field(record, "id").cloned());
        put("name", get("name"));
        put("version", get("version"));
        put("author", get("author"));
        put("description", get("description"));
        put("gameVersion", get("gameVersion"));
        put(
            "overrides",
            Some(if util::truthy(util::field(chosen, "overrides")) { get("overrides").unwrap() } else { json!([]) }),
        );
        put(
            "sha256",
            Some(if util::truthy(util::field(chosen, "sha256")) {
                get("sha256").unwrap()
            } else {
                Value::String(pack_identity_of(util::field(chosen, "files")))
            }),
        );
        let blocked = util::truthy(util::field(record, "blocked"));
        put("blocked", Some(Value::Bool(blocked)));
        let reason = util::field(record, "blockedReason");
        put(
            "blockedReason",
            Some(if blocked && util::truthy(reason) { reason.cloned().unwrap() } else { Value::String(String::new()) }),
        );
        put("files", get("files"));
        put("totalBytes", get("totalBytes"));
        put(
            "downloadBase",
            Some(Value::String(if self.base.is_empty() {
                String::new()
            } else {
                format!("{}/{}/{}/", self.base, template(util::field(record, "id")), template(util::field(chosen, "version")))
            })),
        );
        put("registeredAt", get("registeredAt"));
        put(
            "versions",
            Some(Value::Array(
                versions.iter().map(|entry| util::field(entry, "version").cloned().unwrap_or(Value::Null)).collect(),
            )),
        );
        Some(Value::Object(view))
    }

    /// `GET /api/mods/packs`: `[200, {ok, packs, downloadConfigured}]`, sorted by name then id.
    pub fn list(&self) -> Reply {
        let mut items: Vec<Value> = self.packs.values().filter_map(|record| self.public_view(record, None)).collect();
        let key = |view: &Value, field: &str| util::str_nullish(util::field(view, field));
        items.sort_by(|a, b| locale_compare(&key(a, "name"), &key(b, "name")).then_with(|| locale_compare(&key(a, "id"), &key(b, "id"))));
        (200, json!({ "ok": true, "packs": items, "downloadConfigured": !self.base.is_empty() }))
    }

    /// `GET /api/mods/packs/<id>[?version=]`: the newest version, or the named one (an empty version = newest).
    pub fn get(&self, id: &str, version: Option<&str>) -> Reply {
        let version = version.filter(|v| !v.is_empty());
        match self.packs.get(id).and_then(|record| self.public_view(record, version)) {
            Some(view) => (200, json!({ "ok": true, "pack": view })),
            None => not_found(),
        }
    }

    /// `POST /api/admin/mods/register {pack, replace?}` (owner already checked by the router).
    pub fn register(&mut self, body: &Value, owner: &str) -> Reply {
        let pack = match validate_pack_manifest(util::field(body, "pack").unwrap_or(&Value::Null), &self.limits) {
            Ok(pack) => pack,
            Err(refused) => {
                let mut reply = Map::new();
                reply.insert("ok".into(), Value::Bool(false));
                reply.insert("code".into(), Value::String(refused.error.into()));
                reply.insert("error".into(), Value::String(refused.error.into()));
                if let Some(path) = refused.path {
                    reply.insert("path".into(), Value::String(path));
                }
                return (400, Value::Object(reply));
            }
        };
        let id = pack["id"].as_str().unwrap_or_default().to_string();
        let version = pack["version"].clone();
        let mut record = match self.packs.get(&id) {
            Some(record) => record.clone(),
            None => {
                if self.packs.len() >= self.limits.max_packs {
                    return (507, json!({ "ok": false, "code": "INDEX_FULL", "error": "Mod pack index is full" }));
                }
                json!({ "id": id, "blocked": false, "versions": [] })
            }
        };
        let existing: Vec<Value> = match util::field(&record, "versions") {
            Some(Value::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        let same = |entry: &Value| util::field(entry, "version") == Some(&version);
        if existing.iter().any(same) && !util::truthy(util::field(body, "replace")) {
            return (
                409,
                json!({ "ok": false, "code": "VERSION_EXISTS", "error": "That version is already registered; bump the version" }),
            );
        }
        let mut entry = pack.as_object().cloned().unwrap_or_default();
        entry.insert("registeredAt".into(), Value::String(util::iso_time((self.now)())));
        entry.insert("registeredBy".into(), Value::String(owner.to_string()));
        let mut versions = vec![Value::Object(entry)];
        versions.extend(existing.into_iter().filter(|entry| !same(entry)));
        versions.truncate(self.limits.max_versions_kept);
        if let Value::Object(map) = &mut record {
            map.insert("versions".into(), Value::Array(versions));
        }
        self.packs.insert(id, record.clone());
        self.persist();
        (200, json!({ "ok": true, "pack": self.public_view(&record, None) }))
    }

    fn set_blocked(&mut self, body: &Value, owner: &str, blocked: bool) -> Reply {
        let id = util::str_or_empty(util::field(body, "id"));
        let now = (self.now)();
        let Some(Value::Object(record)) = self.packs.get_mut(&id) else { return not_found() };
        record.insert("blocked".into(), Value::Bool(blocked));
        record.insert(
            "blockedReason".into(),
            Value::String(if blocked { text(util::field(body, "reason"), 200) } else { String::new() }),
        );
        record.insert("blockedBy".into(), Value::String(if blocked { owner.to_string() } else { String::new() }));
        record.insert("blockedAt".into(), Value::String(if blocked { util::iso_time(now) } else { String::new() }));
        let record = Value::Object(record.clone());
        self.persist();
        (200, json!({ "ok": true, "pack": self.public_view(&record, None) }))
    }

    /// `POST /api/admin/mods/block {id, reason?}`.
    pub fn block(&mut self, body: &Value, owner: &str) -> Reply {
        self.set_blocked(body, owner, true)
    }

    /// `POST /api/admin/mods/unblock {id}`.
    pub fn unblock(&mut self, body: &Value, owner: &str) -> Reply {
        self.set_blocked(body, owner, false)
    }

    /// `modPacks.size`.
    #[cfg(test)]
    pub fn size(&self) -> usize {
        self.packs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(text: &str) -> String {
        util::sha256_hex(text)
    }

    fn manifest(extra: Value) -> Value {
        let mut base = json!({
            "schema": "lr.pack/1", "id": "neon_halls", "name": "Neon Halls", "version": "1.0.0", "author": "KT",
            "files": [
                { "path": "pack.json", "sha256": sha("{}"), "size": 2 },
                { "path": "props/hall.json", "sha256": sha("hall"), "size": 4 },
            ],
        });
        if let (Value::Object(map), Value::Object(extra)) = (&mut base, extra) {
            for (k, v) in extra {
                map.insert(k, v);
            }
        }
        base
    }

    fn error_of(value: Value) -> &'static str {
        validate_pack_manifest(&value, &MOD_LIMITS).err().map(|e| e.error).unwrap_or("OK")
    }

    fn ok(value: Value) -> bool {
        validate_pack_manifest(&value, &MOD_LIMITS).is_ok()
    }

    #[test]
    fn manifest_validation_follows_the_client_loader_and_refuses_traversal_bad_ids_and_oversized_packs() {
        assert!(ok(manifest(json!({}))));
        assert_eq!(error_of(manifest(json!({ "id": "../etc" }))), "INVALID_PACK_ID");
        assert_eq!(error_of(manifest(json!({ "id": "neon-halls" }))), "INVALID_PACK_ID", "no hyphen: content ids are <pack_id>_<name>");
        assert_eq!(error_of(manifest(json!({ "id": "a".repeat(33) }))), "INVALID_PACK_ID", "32 characters at most");
        assert_eq!(error_of(manifest(json!({ "schema": "lr.pack/2" }))), "INVALID_PACK_SCHEMA");
        assert_eq!(error_of(manifest(json!({ "scripts": ["x.gd"] }))), "UNKNOWN_PACK_FIELD");
        assert!(ok(manifest(json!({ "overrides": ["props/officePrinter"], "game_version": "0.9", "description": "d" }))));
        assert_eq!(error_of(manifest(json!({ "overrides": ["../x"] }))), "INVALID_PACK_OVERRIDES");
        for kind in mod_pack_kinds() {
            let good = manifest(json!({ "overrides": [format!("{kind}/x")],
                "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 2 }, { "path": format!("{kind}/a.json"), "sha256": sha("a"), "size": 1 }] }));
            assert!(ok(good), "{kind}/*.json is a pack data folder");
        }
        // Node: assert.deepEqual(MOD_PACK_KINDS, readPackKindsFromRepo()) -- the real checkout has no
        // src/config/content_registry.gd, so here (no pack_kinds.json beside the test binary, no repo) the index
        // uses its fallback list.
        let expected = crate::pack_kinds::load_pack_kinds(FALLBACK_PACK_KINDS);
        assert_eq!(mod_pack_kinds(), expected.as_slice(), "the index takes its pack folders from the game");
        // only ambience override ids may contain ":" (not at either end)
        assert!(ok(manifest(json!({ "overrides": ["ambience/room:chapel"] }))));
        for bad in ["props/room:chapel", "ambience/:chapel", "ambience/room:", "ceilings/a b", "basements/x"] {
            assert_eq!(error_of(manifest(json!({ "overrides": [bad] }))), "INVALID_PACK_OVERRIDES", "{bad}");
        }
        for path in ["props/sub/x.json", "assets/a.GLB", "assets/a.BBMODEL", "assets/a.bbmodel.json", "assets/a.bbmodel/", "maps/x.json", "assets/a.gd", "props/x.png"] {
            let bad = manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 2 }, { "path": path, "sha256": sha("x"), "size": 1 }] }));
            assert_eq!(error_of(bad), "INVALID_FILE_PATH", "{path}");
        }
        // models are Blockbench .bbmodel (preferred) or legacy .glb; textures .png, sounds .ogg
        for path in ["assets/lamp.bbmodel", "assets/props/lamp.bbmodel", "assets/props/lamp.glb", "assets/props/lamp.png", "assets/a/b/c/d/e/f/lamp.bbmodel", "assets/s.ogg"] {
            let good = manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 2 }, { "path": path, "sha256": sha("x"), "size": 1 }] }));
            assert!(ok(good), "{path}");
        }
        assert_eq!(error_of(manifest(json!({ "version": "1/../2" }))), "INVALID_PACK_VERSION");
        for path in ["../x.gd", "/abs.gd", "a/../b.gd", "a\\b.gd", ".hidden", "a//b"] {
            let bad = manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 2 }, { "path": path, "sha256": sha("x"), "size": 1 }] }));
            assert_eq!(error_of(bad), "INVALID_FILE_PATH", "{path}");
        }
        assert_eq!(error_of(manifest(json!({ "files": [{ "path": "props/a.json", "sha256": sha("a"), "size": 1 }] }))), "PACK_JSON_MISSING");
        assert_eq!(error_of(manifest(json!({ "files": [{ "path": "pack.json", "sha256": "nothex", "size": 2 }] }))), "INVALID_FILE_SHA256");
        let dupe = manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 2 },
            { "path": "props/a.json", "sha256": sha("a"), "size": 1 }, { "path": "props/A.json", "sha256": sha("a"), "size": 1 }] }));
        assert_eq!(error_of(dupe), "DUPLICATE_FILE_PATH");
        let big = manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": MOD_LIMITS.max_file_bytes + 1 }] }));
        assert_eq!(error_of(big), "FILE_TOO_LARGE");
        let mut files = vec![json!({ "path": "pack.json", "sha256": sha("{}"), "size": 2 })];
        files.extend((0..9).map(|i| json!({ "path": format!("assets/f{i}.png"), "sha256": sha(&i.to_string()), "size": MOD_LIMITS.max_file_bytes })));
        assert_eq!(error_of(manifest(json!({ "files": files }))), "PACK_TOO_LARGE");
    }

    #[test]
    fn manifest_errors_carry_the_offending_path_and_sizes_are_coerced_like_number() {
        let refused = validate_pack_manifest(&manifest(json!({ "scripts": 1, "zz": 2 })), &MOD_LIMITS).unwrap_err();
        assert_eq!(refused, ManifestError { error: "UNKNOWN_PACK_FIELD", path: Some("scripts".into()) });
        let refused = validate_pack_manifest(&manifest(json!({ "zz": 2, "7": 1 })), &MOD_LIMITS).unwrap_err();
        assert_eq!(refused.path.as_deref(), Some("7"), "integer keys come first in Object.keys");
        let coerced = validate_pack_manifest(
            &manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}").to_uppercase(), "size": "2" }] })),
            &MOD_LIMITS,
        )
        .unwrap();
        assert_eq!(coerced["files"][0]["size"], json!(2));
        assert_eq!(coerced["files"][0]["sha256"], json!(sha("{}")));
        assert_eq!(
            util::stringify(&validate_pack_manifest(&manifest(json!({ "name": " \u{1}Neon\tHalls " })), &MOD_LIMITS).unwrap()["name"]),
            "\"Neon Halls\""
        );
        assert_eq!(error_of(json!(null)), "INVALID_PACK_SCHEMA");
        assert_eq!(error_of(manifest(json!({ "overrides": null }))), "INVALID_PACK_OVERRIDES");
        assert_eq!(error_of(manifest(json!({ "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 1.5 }] }))), "INVALID_FILE_SIZE");
    }

    #[test]
    fn pack_identity_sha256_of_sorted_lines_over_every_file_including_pack_json() {
        let files = vec![
            json!({ "path": "props/hall.json", "sha256": sha("hall"), "size": 4 }),
            json!({ "path": "pack.json", "sha256": sha("{}"), "size": 2 }),
        ];
        let expected = sha(&format!("{} pack.json\n{} props/hall.json", sha("{}"), sha("hall")));
        assert_eq!(pack_identity(&files), expected);
        assert_eq!(validate_pack_manifest(&manifest(json!({})), &MOD_LIMITS).unwrap()["sha256"], json!(expected));
    }

    fn temp_store(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("{label}-{}-{}", std::process::id(), util::hex(&util::random_bytes(6))))
            .join("mod_packs.json")
    }

    fn index(store: &PathBuf, clock: Clock) -> ModPacks {
        ModPacks::new(ModPacksOptions {
            store_path: Some(store.clone()),
            download_base: "https://files.example.test/lr3d/mods/".into(),
            now: clock,
            ..Default::default()
        })
    }

    // Node's version of this test goes through server.mjs (owner/player 403s are the router's); this covers the
    // module's part: register, version conflicts, listing, get, block/unblock and persistence across a restart.
    #[test]
    fn index_lists_and_reads_registers_blocks_and_unblocks_and_data_survives_a_restart() {
        let store = temp_store("mod-packs");
        let (clock, _) = util::manual_clock(1_700_000_000_000);
        let mut packs = index(&store, clock.clone());
        assert_eq!(packs.list(), (200, json!({ "ok": true, "packs": [], "downloadConfigured": true })));

        let registered = packs.register(&json!({ "pack": manifest(json!({})) }), "KYTTU");
        assert_eq!(registered.0, 200, "{}", registered.1);
        assert_eq!(registered.1["pack"]["downloadBase"], json!("https://files.example.test/lr3d/mods/neon_halls/1.0.0/"));

        let again = packs.register(&json!({ "pack": manifest(json!({})) }), "KYTTU");
        assert_eq!(again.0, 409);
        assert_eq!(again.1["code"], json!("VERSION_EXISTS"));
        packs.register(&json!({ "pack": manifest(json!({ "version": "1.1.0", "name": "Neon Halls+" })) }), "KYTTU");

        let listed = packs.list().1;
        assert_eq!(listed["packs"].as_array().unwrap().len(), 1);
        assert_eq!(listed["packs"][0]["version"], json!("1.1.0"), "the newest version is listed");
        assert_eq!(listed["packs"][0]["versions"], json!(["1.1.0", "1.0.0"]));
        let files = listed["packs"][0]["files"].as_array().unwrap();
        assert_eq!(files.iter().find(|f| f["path"] == "props/hall.json").unwrap()["sha256"], json!(sha("hall")));
        assert!(files.iter().any(|f| f["path"] == "pack.json"), "pack.json is in the index files list");
        assert!(Regex::new("^[a-f0-9]{64}$").unwrap().is_match(listed["packs"][0]["sha256"].as_str().unwrap()));
        assert_eq!(packs.get("neon_halls", Some("1.0.0")).1["pack"]["name"], json!("Neon Halls"));
        assert_eq!(packs.get("nope_pack", None).0, 404);
        assert_eq!(packs.get("..%2Fetc", None).0, 404);

        let blocked = packs.block(&json!({ "id": "neon_halls", "reason": "crashes hosts" }), "KYTTU");
        assert_eq!(blocked.1["pack"]["blocked"], json!(true));
        assert_eq!(blocked.1["pack"]["blockedReason"], json!("crashes hosts"));
        assert_eq!(packs.block(&json!({ "id": "nope_pack" }), "KYTTU").0, 404);

        let mut packs = index(&store, clock);
        let after = packs.get("neon_halls", None).1["pack"].clone();
        assert_eq!(after["blocked"], json!(true), "block survives a restart");
        assert_eq!(after["version"], json!("1.1.0"));
        let unblocked = packs.unblock(&json!({ "id": "neon_halls" }), "KYTTU");
        assert_eq!(unblocked.1["pack"]["blocked"], json!(false));
        assert_eq!(unblocked.1["pack"]["blockedReason"], json!(""));
        assert_eq!(packs.size(), 1);
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }

    #[test]
    fn the_store_format_and_views_match_the_node_server_byte_for_byte() {
        let store = temp_store("mod-packs-format");
        let (clock, _) = util::manual_clock(1_700_000_000_123);
        let mut packs = index(&store, clock);
        packs.register(&json!({ "pack": manifest(json!({ "overrides": ["props/officePrinter"] })) }), "KYTTU");
        packs.block(&json!({ "id": "neon_halls", "reason": "x" }), "KYTTU");
        let written = std::fs::read_to_string(&store).unwrap();
        // node: the same calls through createModPacks with now = () => 1700000000123
        let files = format!(
            r#"[{{"path":"pack.json","sha256":"{}","size":2}},{{"path":"props/hall.json","sha256":"{}","size":4}}]"#,
            sha("{}"),
            sha("hall")
        );
        let identity = sha(&format!("{} pack.json\n{} props/hall.json", sha("{}"), sha("hall")));
        assert_eq!(
            written,
            format!(
                r#"{{"version":1,"packs":[{{"id":"neon_halls","blocked":true,"versions":[{{"id":"neon_halls","version":"1.0.0","name":"Neon Halls","author":"KT","description":"","gameVersion":"","overrides":["props/officePrinter"],"files":{files},"totalBytes":6,"sha256":"{identity}","registeredAt":"2023-11-14T22:13:20.123Z","registeredBy":"KYTTU"}}],"blockedReason":"x","blockedBy":"KYTTU","blockedAt":"2023-11-14T22:13:20.123Z"}}]}}"#
            )
        );
        assert_eq!(
            util::stringify(&packs.get("neon_halls", None).1),
            format!(
                r#"{{"ok":true,"pack":{{"id":"neon_halls","name":"Neon Halls","version":"1.0.0","author":"KT","description":"","gameVersion":"","overrides":["props/officePrinter"],"sha256":"{identity}","blocked":true,"blockedReason":"x","files":{files},"totalBytes":6,"downloadBase":"https://files.example.test/lr3d/mods/neon_halls/1.0.0/","registeredAt":"2023-11-14T22:13:20.123Z","versions":["1.0.0"]}}}}"#
            )
        );
        let refused = packs.register(&json!({ "pack": manifest(json!({ "id": "x" })) }), "KYTTU");
        assert_eq!(util::stringify(&refused.1), r#"{"ok":false,"code":"INVALID_PACK_ID","error":"INVALID_PACK_ID"}"#);
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }

    #[test]
    fn replace_overwrites_a_version_and_old_versions_drop_out() {
        let limits = ModLimits { max_versions_kept: 2, max_packs: 1, ..MOD_LIMITS };
        let mut packs = ModPacks::new(ModPacksOptions { limits, ..Default::default() });
        for v in ["1.0.0", "1.1.0", "1.2.0"] {
            assert_eq!(packs.register(&json!({ "pack": manifest(json!({ "version": v })) }), "KYTTU").0, 200);
        }
        assert_eq!(packs.get("neon_halls", None).1["pack"]["versions"], json!(["1.2.0", "1.1.0"]));
        let replaced = packs.register(&json!({ "replace": true, "pack": manifest(json!({ "version": "1.1.0", "name": "Again" })) }), "KYTTU");
        assert_eq!(replaced.1["pack"]["versions"], json!(["1.1.0", "1.2.0"]));
        assert_eq!(replaced.1["pack"]["downloadBase"], json!(""));
        let full = packs.register(&json!({ "pack": manifest(json!({ "id": "other_pack" })) }), "KYTTU");
        assert_eq!(full.0, 507);
        assert_eq!(full.1["code"], json!("INDEX_FULL"));
    }

    #[test]
    fn locale_compare_matches_node_for_ascii_and_common_accents() {
        // node: [...].sort((a, b) => a.localeCompare(b))
        let mut words = vec!["a", "A", "b", "B", "ab", "Ab", "aB", "a b", "a-b", "ab2", "ab10", "é", "e", "f", "Zeta", "alpha",
            "neon_halls", "neon halls", "Neon Halls+", "Neon Halls"];
        words.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(words.join("|"), "a|A|a b|a-b|ab|aB|Ab|ab10|ab2|alpha|b|B|e|é|f|neon halls|Neon Halls|Neon Halls+|neon_halls|Zeta");
        let mut ascii: Vec<String> = (32u8..127).map(|b| (b as char).to_string()).collect();
        ascii.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(ascii.concat(), " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ");
        assert_eq!(locale_compare("résumé", "resume"), Ordering::Greater);
        assert_eq!(locale_compare("é", "e\u{301}"), Ordering::Equal);
    }
}
