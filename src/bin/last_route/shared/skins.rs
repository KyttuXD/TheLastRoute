//! The skin list for the SKIN tab. Skins are 64x64 PNG files in `skins/` beside the settings file
//! (the user folder from `paths::user_dir`). A list id is the cleaned file stem. DEFAULT is the built-in
//! mannequin texture and has no file, so it is always first.
//!
//! On the wire a skin is known by its id instead: the lowercase hex SHA-256 of the PNG bytes ([`chosen_skin`]).
//! The website stores each uploaded skin under that id, and other clients fetch it from there into `skins-cache`.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::net::api;
use crate::net::config::BackendConfig;
use crate::net::http::{Method, Request};
use crate::net::NetError;
use crate::net::world_sync::{clean_skin, is_skin_hash, SKIN_DEFAULT};
use crate::shared::paths;
use crate::shared::skin_validate;

/// The side length every skin PNG must have.
pub const SKIN_SIZE: u32 = 64;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The folder skins are read from (`<user dir>/skins`), when the user folder is known.
pub fn skin_dir() -> Option<PathBuf> {
    paths::user_dir().map(|d| d.join("skins"))
}

/// True when the first bytes of a file are a PNG signature and an IHDR of 64x64 or the old 64x32.
/// Only the header is read; the pixels are not decoded (see [`validate_skin`] for the full check).
pub fn png_is_skin_size(head: &[u8]) -> bool {
    if head.len() < 24 || head[..8] != PNG_SIGNATURE || &head[12..16] != b"IHDR" {
        return false;
    }
    let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    width == SKIN_SIZE && (height == SKIN_SIZE || height == SKIN_SIZE / 2)
}

/// Reads the first 24 bytes of `path` and checks them with [`png_is_skin_size`].
fn file_is_skin(path: &Path) -> bool {
    let mut head = [0u8; 24];
    match File::open(path).and_then(|mut f| f.read_exact(&mut head)) {
        Ok(()) => png_is_skin_size(&head),
        Err(_) => false,
    }
}

/// Skin ids for a folder: DEFAULT first, then the valid `.png` stems in name order.
/// A missing folder gives just DEFAULT. Files that are not 64x64 PNGs are skipped.
pub fn list_skins(dir: Option<&Path>) -> Vec<String> {
    let mut ids = vec![SKIN_DEFAULT.to_string()];
    let Some(dir) = dir else { return ids };
    let Ok(entries) = std::fs::read_dir(dir) else { return ids };
    let mut found: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter(|p| file_is_skin(p))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(clean_skin))
        .filter(|id| id != SKIN_DEFAULT && !id.is_empty())
        .collect();
    found.sort();
    found.dedup();
    ids.extend(found);
    ids
}

/// The skin to use for a saved id: the id when the list has it, otherwise DEFAULT.
pub fn resolve_skin(saved: &str, ids: &[String]) -> String {
    let wanted = clean_skin(saved);
    if ids.iter().any(|id| *id == wanted) {
        wanted
    } else {
        SKIN_DEFAULT.to_string()
    }
}

/// The skin a runner sends to a room: the wire id (the SHA-256 of its PNG) of the saved skin when its file is still
/// valid, else DEFAULT. The chosen file is also kept in the cache, so this client draws its own runner the same way
/// the other clients will.
pub fn chosen_skin(saved: &str) -> String {
    let id = resolve_skin(saved, &list_skins(skin_dir().as_deref()));
    match skin_png(&id) {
        Some((hash, bytes)) => {
            if let Some(dir) = cache_dir() {
                if fetch_needed(&dir, &hash) {
                    let _ = write_skin_file(&dir, &hash, &bytes);
                }
            }
            hash
        }
        None => SKIN_DEFAULT.to_string(),
    }
}

/// The skin file a list id (a cleaned file stem) names in the skins folder, when it is a valid skin.
fn skin_file(id: &str) -> Option<PathBuf> {
    let dir = skin_dir()?;
    std::fs::read_dir(dir).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| {
        p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("png"))
            && p.file_stem().and_then(|s| s.to_str()).map(clean_skin).as_deref() == Some(id)
            && file_is_skin(p)
    })
}

/// The wire id and bytes of the skin a list id names. The id is the SHA-256 of the PNG. DEFAULT, unknown ids and
/// files over the website's size cap give None.
pub fn skin_png(id: &str) -> Option<(String, Vec<u8>)> {
    if id == SKIN_DEFAULT {
        return None;
    }
    let bytes = std::fs::read(skin_file(id)?).ok()?;
    // The canonical re-encoding is what is hashed, cached and uploaded, so every client agrees on the id.
    skin_from_png(bytes).ok()
}

/// The lowercase hex SHA-256 of `bytes`: the id a skin PNG is known by (64 hex characters).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// True when `bytes` is a 64x64 PNG whose SHA-256 is `id`. Both the disk cache and fetched skins are checked this way.
pub fn verify_skin(id: &str, bytes: &[u8]) -> bool {
    png_is_skin_size(bytes) && sha256_hex(bytes) == id
}

/// The folder skins fetched from the website are cached in (`<user dir>/skins-cache`).
pub fn cache_dir() -> Option<PathBuf> {
    paths::user_dir().map(|d| d.join("skins-cache"))
}

/// The bytes of a skin known by `id` from the user's own folder or the fetch cache, when they verify: the hash matches AND
/// the whole file passes the shared check ([`verify_download`]), so a hostile file that an older build cached, or one
/// dropped into the folder by hand, can never reach the image decoder (which panics on a broken body).
pub fn read_skin(id: &str) -> Option<Vec<u8>> {
    let file = format!("{id}.png");
    [skin_dir(), cache_dir()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(&file))
        .filter_map(|path| std::fs::read(path).ok())
        .find(|bytes| verify_download(id, bytes))
}

/// The path of a skin id inside a cache folder: `<dir>/<id>.png`.
pub fn cached_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.png"))
}

/// The bytes of a cache file, never more than the cap plus one byte, and only from a regular file (a FIFO or a device
/// named like a skin would block the reader).
fn read_cached_file(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(read_limit() as u64).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// True when the folder has no verified copy of skin `id`, so it has to be fetched. A cached skin is never fetched again.
pub fn fetch_needed(dir: &Path, id: &str) -> bool {
    !read_cached_file(&cached_path(dir, id)).is_some_and(|bytes| verify_download(id, &bytes))
}

/// The skin fetch settings from data/skins/config.json. Standard values match the Godot game: the size cap is
/// PlayerSkin.MAX_BYTES (game/player/player_skin.gd:16), and the backend route is `/api/skins/files/<hash>.png`
/// (game/autoload/skin_sync.gd:152).
#[derive(Clone, Debug, PartialEq)]
pub struct SkinConfig {
    /// The largest skin file accepted, in bytes.
    pub max_bytes: usize,
    /// The largest canonical PNG the website stores, in bytes (the server's SKIN_MAX_BYTES, 16 KB). A skin whose canonical
    /// re-encoding is bigger is refused before upload.
    pub upload_max_bytes: usize,
    /// The skins host base, tried first when fetching (no trailing slash).
    pub host_base: String,
    /// The path on the skins host, with `{id}` for the skin hash.
    pub host_path: String,
    /// The backend route of the legacy Godot backend (`skin_sync.gd`), with `{id}` for the skin hash. Relative to the
    /// backend base. Kept so a server that only has the old route still serves skins.
    pub backend_path: String,
    /// The backend route of `lr_server` (`GET /api/v1/skins/<id>`), with `{id}` for the skin hash. Relative to the backend base.
    pub v1_path: String,
    /// The timeout of one skin download, in seconds.
    pub timeout_secs: u64,
    /// How long a skin whose download (or upload) failed is left alone before it is tried again (Godot `RETRY_SECONDS`).
    pub retry_secs: u64,
    /// The most skin downloads and uploads running at once.
    pub max_concurrent: usize,
    /// The most downloaded skins kept in the cache folder; the oldest go first (Godot `PEER_CACHE_MAX`, 16).
    pub cache_max: usize,
}

/// The schema of data/skins/config.json.
pub const CONFIG_SCHEMA: &str = "lr.skin_config/1";

/// The fetch settings from their JSON text. None when the schema or any value is missing.
pub fn parse_config(text: &str) -> Option<SkinConfig> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v["schema"].as_str()? != CONFIG_SCHEMA {
        return None;
    }
    let fetch = &v["fetch"];
    Some(SkinConfig {
        max_bytes: v["max_bytes"].as_u64()? as usize,
        upload_max_bytes: v["upload_max_bytes"].as_u64()? as usize,
        host_base: fetch["host_base"].as_str()?.trim_end_matches('/').to_string(),
        host_path: fetch["host_path"].as_str()?.to_string(),
        backend_path: fetch["backend_path"].as_str()?.to_string(),
        v1_path: fetch["v1_path"].as_str()?.to_string(),
        timeout_secs: fetch["timeout_secs"].as_u64()?,
        retry_secs: fetch["retry_secs"].as_u64()?,
        max_concurrent: fetch["max_concurrent"].as_u64()? as usize,
        cache_max: fetch["cache_max"].as_u64()? as usize,
    })
}

/// The shipped fetch settings (data/skins/config.json), read once. None when the file is missing or broken.
pub fn config() -> Option<SkinConfig> {
    static CONFIG: std::sync::OnceLock<Option<SkinConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            crate::content::embedded::data_under("skins/config.json")
                .next()
                .and_then(|(_, text)| parse_config(text))
        })
        .clone()
}

/// The largest skin file accepted. 0 when the config is missing, which refuses every skin rather than guessing a cap.
pub fn max_bytes() -> usize {
    config().map_or(0, |c| c.max_bytes)
}

/// The largest canonical PNG the website accepts. 0 when the config is missing, which refuses every skin.
pub fn upload_max_bytes() -> usize {
    config().map_or(0, |c| c.upload_max_bytes)
}

/// The byte limit for a download: one more than the cap, so a file over the cap is seen as over it
/// (Godot's FETCH_LIMIT_BYTES = MAX_BYTES + 1, game/autoload/skin_sync.gd:23).
pub fn read_limit() -> usize {
    max_bytes() + 1
}

/// The URL a skin is fetched from on the skins host: `<host_base><host_path>` with the id filled in.
pub fn skin_host_url(cfg: &SkinConfig, id: &str) -> String {
    format!("{}{}", cfg.host_base, cfg.host_path.replace("{id}", id))
}

/// The URL a skin is fetched from on the legacy Godot backend route: `<backend base><backend_path>`, e.g.
/// `GET <base>/api/skins/files/<id>.png`. `lr_server` answers it too, but its own route is [`skin_v1_url`].
pub fn skin_backend_url(cfg: &SkinConfig, backend_base: &str, id: &str) -> String {
    format!("{}{}", backend_base.trim_end_matches('/'), cfg.backend_path.replace("{id}", id))
}

/// The URL a skin is fetched from on `lr_server`: `<backend base><v1_path>`, i.e. `GET <base>/api/v1/skins/<id>`.
pub fn skin_v1_url(cfg: &SkinConfig, backend_base: &str, id: &str) -> String {
    format!("{}{}", backend_base.trim_end_matches('/'), cfg.v1_path.replace("{id}", id))
}

/// The URLs a skin is tried at, in order: the backend's own route ([`skin_v1_url`], the store the uploads go to), then
/// the skins host (immutable static copies behind Caddy), then the legacy Godot route ([`skin_backend_url`]).
pub fn fetch_urls(cfg: &SkinConfig, backend_base: &str, id: &str) -> Vec<String> {
    vec![skin_v1_url(cfg, backend_base, id), skin_host_url(cfg, id), skin_backend_url(cfg, backend_base, id)]
}

/// True when `bytes` are the skin `id` AND safe to draw: the hash matches ([`verify_skin`]) and the whole file passes the
/// shared check ([`skin_validate::check_skin_png`]: every chunk CRC, 64x64, a real inflate, an opaque base layer). A
/// header that merely looks right is not enough: decoding a broken body panics the image decoder.
pub fn verify_download(id: &str, bytes: &[u8]) -> bool {
    verify_skin(id, bytes) && skin_validate::check_skin_png(bytes, max_bytes()).is_ok()
}

/// Tries each URL in order with `get` and returns the first body that fully verifies as skin `id`
/// ([`verify_download`]). A reply that fails or does not verify moves on to the next URL.
fn first_verified_skin(urls: &[String], id: &str, mut get: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    urls.iter().find_map(|url| get(url).filter(|bytes| verify_download(id, bytes)))
}

/// The content types a skin picture may be served as. Plain servers and CDNs often send the generic ones.
const PNG_TYPES: [&str; 4] = ["image/png", "image/x-png", "application/octet-stream", "binary/octet-stream"];
/// The content type of Mojang's lookup answers.
const JSON_TYPES: [&str; 1] = ["application/json"];

/// Downloads one address. The backend (plain http, possibly on a LAN) is asked once. An https address (the skins host,
/// a player's link, Mojang) may redirect up to the client's cap, each hop re-checked by the skin URL rules, and never
/// connects to a non-public address. Only a 200 answer's body comes back.
fn download(url: &str, timeout_secs: u64, limit: usize, accept: &[&str]) -> Result<Vec<u8>, NetError> {
    let mut request = Request::new(Method::Get, url.trim().to_string())
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .limit(limit)
        .content_types(accept);
    if url.trim().to_ascii_lowercase().starts_with("https://") {
        request = request.public_only().redirects(crate::net::http::MAX_REDIRECTS).hop_guard(|hop| match rules() {
            Some(rules) => url_refusal(hop, &rules, &[]).map(str::to_string),
            None => Some("the skin rules are missing from this build".to_string()),
        });
    }
    let reply = request.send()?;
    if reply.status != 200 {
        return Err(NetError::Http { status: reply.status, code: String::new(), error: String::new(), body: serde_json::Value::Null });
    }
    Ok(reply.body)
}

/// The words a player sees for a failed download. Never an address.
pub fn download_error_text(error: &NetError) -> String {
    match error {
        NetError::Offline => "this game is offline".to_string(),
        NetError::Timeout => "the download timed out".to_string(),
        NetError::Refused(why) | NetError::Unsupported(why) => why.to_ascii_lowercase(),
        NetError::Transport(why) => {
            let why = why.to_ascii_lowercase();
            if why.contains("tls") || why.contains("certificate") || why.contains("handshake") || why.contains("alert") {
                "the secure connection could not be verified".to_string()
            } else {
                "the download failed: could not connect".to_string()
            }
        }
        NetError::Protocol(why) => match why.as_str() {
            "RESPONSE TOO LARGE" => "the file is too big".to_string(),
            "UNEXPECTED CONTENT TYPE" => "that address is not a PNG picture".to_string(),
            "TOO MANY REDIRECTS" => "the address redirects too many times".to_string(),
            _ => "the download failed: the server sent something unusable".to_string(),
        },
        NetError::Http { status, .. } => format!("the server answered {status}"),
    }
}

/// Which skin downloads and uploads are running or cooling down after a failure, so a skin nobody can serve is not
/// asked for every frame and a flood of unknown ids cannot start a flood of threads.
#[derive(Default)]
struct Book {
    running: std::collections::HashSet<String>,
    retry_at: std::collections::HashMap<String, std::time::Instant>,
}

impl Book {
    /// True (and `key` is now running) when it is not running, not cooling down, and fewer than `max` jobs run.
    fn begin(&mut self, key: &str, now: std::time::Instant, max: usize) -> bool {
        if self.running.contains(key) || self.running.len() >= max || self.retry_at.get(key).is_some_and(|at| now < *at) {
            return false;
        }
        self.running.insert(key.to_string());
        true
    }

    /// Ends a job: a failure cools `key` down for `retry`, a success clears it.
    fn finish(&mut self, key: &str, ok: bool, now: std::time::Instant, retry: std::time::Duration) {
        self.running.remove(key);
        if ok {
            self.retry_at.remove(key);
        } else {
            self.retry_at.insert(key.to_string(), now + retry);
        }
    }
}

fn book() -> std::sync::MutexGuard<'static, Book> {
    static BOOK: std::sync::OnceLock<std::sync::Mutex<Book>> = std::sync::OnceLock::new();
    BOOK.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Downloads skin `id` from the first address that serves a fully verified copy and writes it into `dir`. Returns
/// whether the skin is now cached. Blocking (see [`fetch_skin_in_background`]).
pub fn fetch_skin_blocking(cfg: &SkinConfig, backend_base: &str, dir: &Path, id: &str) -> bool {
    let urls = fetch_urls(cfg, backend_base, id);
    let limit = read_limit();
    let fetched = first_verified_skin(&urls, id, |url| download(url, cfg.timeout_secs, limit, &PNG_TYPES).ok());
    match fetched {
        Some(bytes) => {
            let wrote = write_skin_file(dir, id, &bytes).is_ok();
            if wrote {
                prune_cache(dir, cfg.cache_max);
            }
            wrote
        }
        None => false,
    }
}

/// Cleans the download cache: leftovers of half-written files, any `<64 hex>.png` that fails the full check (a hostile or
/// broken file from an older build), and then the oldest skins beyond `keep`. Returns how many files were removed. Only
/// files named like a cached skin or its temp file are ever touched.
pub fn prune_cache(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    let mut kept: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let stem = name.strip_suffix(".png").or_else(|| name.strip_suffix(".tmp"));
        let Some(stem) = stem.filter(|s| is_skin_hash(s)) else { continue };
        let good = name.ends_with(".png") && read_cached_file(&path).is_some_and(|bytes| verify_download(stem, &bytes));
        if good {
            let age = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            kept.push((age, path));
        } else if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    kept.sort();
    let extra = kept.len().saturating_sub(keep);
    for (_, path) in kept.into_iter().take(extra) {
        if std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Writes a skin into `dir` as `<id>.png`. The bytes go to a temporary name first so a half-written file is never read.
pub fn write_skin_file(dir: &Path, id: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{id}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, cached_path(dir, id))
}

/// The website the runners' skins are fetched from, set once at startup (see `app`). Empty means fetching is off.
static BACKEND_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Sets the base URL skins are fetched from (`GET <base>/api/v1/skins/<id>`, see [`fetch_urls`]). Only the first call counts.
pub fn set_backend_base(base: &str) {
    let _ = BACKEND_BASE.set(base.trim_end_matches('/').to_string());
}

/// Starts a background fetch of skin `id` into the cache folder. It never blocks the frame; the file shows up on a later
/// check once it has been downloaded and verified. The skins host is tried first, then the backend route. Does nothing
/// when no backend is set (fetching is off), the id is not a skin hash, or the skin is already cached.
pub fn fetch_skin_in_background(id: &str) {
    let Some(base) = BACKEND_BASE.get().filter(|b| !b.is_empty()).cloned() else { return };
    let Some(cfg) = config() else { return };
    let Some(dir) = cache_dir() else { return };
    if !is_skin_hash(id) || !fetch_needed(&dir, id) {
        return;
    }
    let key = format!("down:{id}");
    if !book().begin(&key, std::time::Instant::now(), cfg.max_concurrent) {
        return;
    }
    let id = id.to_string();
    std::thread::spawn(move || {
        // Once per run, before the first download: drop what an older build cached that fails the full check.
        static PURGED: std::sync::Once = std::sync::Once::new();
        PURGED.call_once(|| {
            prune_cache(&dir, cfg.cache_max);
        });
        let ok = fetch_skin_blocking(&cfg, &base, &dir, &id);
        book().finish(&key, ok, std::time::Instant::now(), std::time::Duration::from_secs(cfg.retry_secs));
    });
}

/// The ids the website has accepted, from `<user dir>/skins-uploaded.json`. A missing or unreadable file is empty.
pub fn uploaded_ids() -> Vec<String> {
    let Some(path) = uploaded_path() else { return Vec::new() };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|id| is_skin_hash(id))
        .collect()
}

/// True when `uploaded` (the list [`uploaded_ids`] gives) holds the skin `id`. Only a skin hash can be held. A host
/// re-publishes its skin once the website holds it (see `net::world_sync::NetSession::set_skin`).
pub fn holds(uploaded: &[String], id: &str) -> bool {
    is_skin_hash(id) && uploaded.iter().any(|x| x == id)
}

fn uploaded_path() -> Option<PathBuf> {
    paths::user_dir().map(|d| d.join("skins-uploaded.json"))
}

/// Adds an id to the uploaded list (once) so it is never uploaded again.
fn remember_uploaded(id: &str) {
    let Some(path) = uploaded_path() else { return };
    let mut ids = uploaded_ids();
    if ids.iter().any(|x| x == id) {
        return;
    }
    ids.push(id.to_string());
    let Ok(text) = serde_json::to_string(&ids) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Uploads the skin `id` (a list id, not DEFAULT) to the website in the background, when signed in (`session_token`
/// set) and not offline. A skin the website already has (see [`uploaded_ids`]) is not sent again.
pub fn upload_in_background(cfg: BackendConfig, session_token: String, id: String) {
    if session_token.is_empty() || cfg.offline {
        return;
    }
    let Some((hash, bytes)) = skin_png(&id) else { return };
    if holds(&uploaded_ids(), &hash) {
        return;
    }
    let Some(settings) = config() else { return };
    // One upload per skin at a time, and a failed one waits `retry_secs` before the next try (Godot `RETRY_SECONDS`), so a
    // caller may ask again every frame.
    let key = format!("up:{hash}");
    if !book().begin(&key, std::time::Instant::now(), settings.max_concurrent) {
        return;
    }
    std::thread::spawn(move || {
        let ok = matches!(api::upload_skin(&cfg, &session_token, &bytes), Ok(server_id) if server_id == hash);
        if ok {
            remember_uploaded(&hash);
        }
        book().finish(&key, ok, std::time::Instant::now(), std::time::Duration::from_secs(settings.retry_secs));
    });
}

/// Removes the account's stored skin from the website in the background (a player chose DEFAULT), and forgets that the
/// old skins were uploaded so choosing one again uploads it again. Does nothing offline or signed out.
pub fn clear_in_background(cfg: BackendConfig, session_token: String) {
    if session_token.is_empty() || cfg.offline {
        return;
    }
    let Some(settings) = config() else { return };
    let key = "clear".to_string();
    if !book().begin(&key, std::time::Instant::now(), settings.max_concurrent) {
        return;
    }
    std::thread::spawn(move || {
        let ok = api::clear_skin(&cfg, &session_token).is_ok();
        if ok {
            forget_uploaded();
        }
        book().finish(&key, ok, std::time::Instant::now(), std::time::Duration::from_secs(settings.retry_secs));
    });
}

/// Empties the uploaded list: the website holds none of the skins any more.
fn forget_uploaded() {
    if let Some(path) = uploaded_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether the skin known by wire id `id` uses slim arms, read from its picture when it is in the skins folder or the
/// cache. None while the picture is not here yet (it is being fetched) or does not pass the full check.
pub fn skin_is_slim(id: &str) -> Option<bool> {
    if let Some(known) = slim_table().get(id) {
        return Some(*known);
    }
    // A picture that is not here yet, or fails the check, is looked for again only every two seconds: this is asked
    // for every remote runner on every frame.
    static MISSES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> = std::sync::OnceLock::new();
    let misses = MISSES.get_or_init(Default::default);
    let now = std::time::Instant::now();
    {
        let mut m = misses.lock().unwrap_or_else(|p| p.into_inner());
        if m.get(id).is_some_and(|at| now.duration_since(*at) < std::time::Duration::from_secs(2)) {
            return None;
        }
        if m.len() > 256 {
            m.clear();
        }
        m.insert(id.to_string(), now);
    }
    let bytes = read_skin(id)?;
    let slim = validate_skin(&bytes, max_bytes()).ok()?.slim;
    note_slim(id, slim);
    Some(slim)
}

/// The schema of a catalogue entry (data/skins/*.json), the Godot `lr.skin/1` format.
pub const CATALOG_SCHEMA: &str = "lr.skin/1";
/// The schema of the rules file (data/skins/rules.json): the URL and Minecraft-name checks for the SKIN tab.
pub const RULES_SCHEMA: &str = "lr.skin_rules/1";

/// One ready-made skin from the catalogue. `image` is a path under the embedded assets (e.g. `skins/ref_classic.png`).
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogSkin {
    pub id: String,
    pub name: String,
    /// The arm style a catalogue skin is worn with (auto, classic or slim). Read once the arm style is saved with a skin.
    #[allow(dead_code)]
    pub arms: String,
    pub image: String,
}

/// A catalogue entry from its JSON text. None unless it has the `lr.skin/1` schema, an id, a name, an image and an arm
/// style of auto, classic or slim (the same shape as Godot's data/skins/ref_classic.json).
pub fn parse_catalog_skin(text: &str) -> Option<CatalogSkin> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v["schema"].as_str()? != CATALOG_SCHEMA {
        return None;
    }
    let id = v["id"].as_str()?.trim();
    let name = v["name"].as_str()?.trim();
    let image = v["image"].as_str()?.trim();
    let arms = v["arms"].as_str().unwrap_or("auto");
    if id.is_empty() || name.is_empty() || image.is_empty() || !matches!(arms, "auto" | "classic" | "slim") {
        return None;
    }
    Some(CatalogSkin {
        id: id.to_string(),
        name: name.to_string(),
        arms: arms.to_string(),
        image: image.to_string(),
    })
}

/// The shipped catalogue (data/skins/*.json, embedded at build time), sorted by id. Files that are not `lr.skin/1`
/// entries (such as rules.json) are skipped.
pub fn catalog() -> Vec<CatalogSkin> {
    let mut out: Vec<CatalogSkin> = crate::content::embedded::data_under("skins/")
        .filter_map(|(_, text)| parse_catalog_skin(text))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The PNG bytes of a catalogue skin from the embedded assets. None when this build does not carry the picture.
pub fn catalog_png(skin: &CatalogSkin) -> Option<Vec<u8>> {
    crate::content::embedded::asset(&skin.image).map(<[u8]>::to_vec)
}

/// The URL and Minecraft-name rules, read from data/skins/rules.json.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinRules {
    pub scheme: String,
    pub max_url_length: usize,
    pub url_timeout_secs: u64,
    pub url_port: u16,
    pub blocked_hosts: Vec<String>,
    pub blocked_suffixes: Vec<String>,
    pub name_max: usize,
    pub name_chars: String,
    pub lookup_url: String,
    pub lookup_host: String,
    pub profile_url: String,
    pub profile_host: String,
    pub textures_host: String,
}

/// The rules from their JSON text. None when the schema or any value is missing.
pub fn parse_rules(text: &str) -> Option<SkinRules> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v["schema"].as_str()? != RULES_SCHEMA {
        return None;
    }
    let url = &v["url"];
    let mc = &v["minecraft"];
    let strings = |val: &serde_json::Value| -> Vec<String> {
        val.as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    Some(SkinRules {
        scheme: url["scheme"].as_str()?.to_string(),
        max_url_length: url["max_length"].as_u64()? as usize,
        url_timeout_secs: url["timeout_secs"].as_u64()?,
        url_port: url["port"].as_u64()? as u16,
        blocked_hosts: strings(&url["blocked_hosts"]),
        blocked_suffixes: strings(&url["blocked_suffixes"]),
        name_max: mc["name_max"].as_u64()? as usize,
        name_chars: mc["name_chars"].as_str()?.to_string(),
        lookup_url: mc["lookup_url"].as_str()?.to_string(),
        lookup_host: mc["lookup_host"].as_str()?.to_string(),
        profile_url: mc["profile_url"].as_str()?.to_string(),
        profile_host: mc["profile_host"].as_str()?.to_string(),
        textures_host: mc["textures_host"].as_str()?.to_string(),
    })
}

/// The shipped rules (data/skins/rules.json). None when the file is missing or broken.
pub fn rules() -> Option<SkinRules> {
    crate::content::embedded::data_under("skins/rules.json")
        .next()
        .and_then(|(_, text)| parse_rules(text))
}

/// Why a URL may not be fetched, or None when it may. Mirrors PlayerSkin.check_url in the Godot game: the scheme from
/// the rules, a real public host, no login, no loopback, LAN or IP literal, the standard port only. `allowed_hosts`
/// (when not empty) narrows the host further, as the Minecraft lookups do.
pub fn url_refusal(url: &str, rules: &SkinRules, allowed_hosts: &[&str]) -> Option<&'static str> {
    let text = url.trim();
    if text.is_empty() || text.len() > rules.max_url_length {
        return Some("that is not a usable address");
    }
    if !text.to_ascii_lowercase().starts_with(&rules.scheme.to_ascii_lowercase()) {
        return Some("only https:// addresses are allowed");
    }
    let rest = &text[rules.scheme.len()..];
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..host_end];
    if authority.contains('@') {
        return Some("addresses with a login are not allowed");
    }
    let mut host = authority.to_ascii_lowercase();
    if host.starts_with('[') {
        return Some("numeric addresses are not allowed");
    }
    let with_port = host.split_once(':').map(|(name, port)| (name.to_string(), port.to_string()));
    if let Some((name, port)) = with_port {
        if port != rules.url_port.to_string() {
            return Some("only the standard https port is allowed");
        }
        host = name;
    }
    if host.is_empty() || !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return Some("that is not a usable address");
    }
    if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
        return Some("that is not a usable address");
    }
    if rules.blocked_hosts.iter().any(|h| *h == host)
        || rules.blocked_suffixes.iter().any(|s| host.ends_with(s.as_str()))
    {
        return Some("local addresses are not allowed");
    }
    if host.split('.').all(|part| part.parse::<i64>().is_ok()) {
        return Some("numeric addresses are not allowed");
    }
    if !allowed_hosts.is_empty() && !allowed_hosts.contains(&host.as_str()) {
        return Some("that host is not allowed here");
    }
    None
}

/// GET `url` over https with the rules' timeout and `limit` bytes. The address is checked by the rules first, a redirect
/// is followed at most three times with every hop checked again, the connection is refused if the name resolves to a
/// non-public address, and an answer whose content type is not in `accept` is dropped unread. Only a 200 reply's body
/// is returned; a failure is described in words a player can read ([`download_error_text`]).
pub fn fetch_https(url: &str, rules: &SkinRules, allowed_hosts: &[&str], limit: usize, accept: &[&str]) -> Result<Vec<u8>, String> {
    if let Some(why) = url_refusal(url, rules, allowed_hosts) {
        return Err(why.to_string());
    }
    let (guard_rules, guard_hosts) = (rules.clone(), allowed_hosts.iter().map(|h| h.to_string()).collect::<Vec<_>>());
    let reply = Request::new(Method::Get, url.trim().to_string())
        .timeout(std::time::Duration::from_secs(rules.url_timeout_secs))
        .limit(limit)
        .content_types(accept)
        .public_only()
        .redirects(crate::net::http::MAX_REDIRECTS)
        .hop_guard(move |hop| {
            let hosts: Vec<&str> = guard_hosts.iter().map(String::as_str).collect();
            url_refusal(hop, &guard_rules, &hosts).map(str::to_string)
        })
        .send()
        .map_err(|e| download_error_text(&e))?;
    if reply.status != 200 {
        return Err(format!("the server answered {}", reply.status));
    }
    Ok(reply.body)
}

/// A downloaded or picked file as a skin: at most the website's size cap, a real PNG, 64x64 (or the old 64x32), with the
/// base layer made opaque. Returns the wire id and the canonical PNG bytes (see [`validate_skin`]).
pub fn skin_from_png(bytes: Vec<u8>) -> Result<(String, Vec<u8>), String> {
    skin_from_png_capped(bytes, max_bytes())
}

/// [`skin_from_png`] with the size cap given (the shipped cap is data/skins/config.json `max_bytes`).
pub fn skin_from_png_capped(bytes: Vec<u8>, cap: usize) -> Result<(String, Vec<u8>), String> {
    validate_skin(&bytes, cap).map(|s| {
        note_slim(&s.hash, s.slim);
        (s.hash, s.png)
    })
}

/// The arm style of each skin this run has validated, by wire id, so the wire profile can say it and a remote runner's
/// arms can be chosen without reading the picture again.
fn slim_table() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, bool>> {
    static TABLE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> = std::sync::OnceLock::new();
    TABLE.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Remembers that skin `id` has slim (or classic) arms.
pub fn note_slim(id: &str, slim: bool) {
    let mut table = slim_table();
    if table.len() < 256 || table.contains_key(id) {
        table.insert(id.to_string(), slim);
    }
}

#[allow(dead_code)]
/// A skin that passed [`validate_skin`].
#[derive(Clone, Debug, PartialEq)]
pub struct ValidSkin {
    /// The canonical 64x64 PNG: the bytes the id, the cache and the website are keyed on.
    pub png: Vec<u8>,
    /// The lowercase hex SHA-256 of `png`.
    pub hash: String,
    /// True when the arms use the slim (Alex) layout. Read before the base layer is made opaque; an old 64x32 skin is never slim.
    pub slim: bool,
    /// The 64x64 RGBA pixels of `png` (16384 bytes).
    pub rgba: Vec<u8>,
}

/// The pixel rectangles of data/skins/rules.json `layout`: [x, y, w, h].
#[derive(Clone, Debug, PartialEq)]
pub struct SkinLayout {
    /// Second-layer rectangles, the only places a pixel may stay transparent.
    pub overlay_rects: Vec<[u32; 4]>,
    /// The arms' 4th texture column in both base layers: transparent there means slim arms.
    pub slim_probe_rects: Vec<[u32; 4]>,
    /// Old-format limb boxes copied mirrored, as (from, to) corners.
    pub legacy_mirror: Vec<([u32; 2], [u32; 2])>,
}

/// The layout from its rules JSON text. None when the schema or a rectangle is missing or malformed.
pub fn parse_layout(text: &str) -> Option<SkinLayout> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v["schema"].as_str()? != RULES_SCHEMA {
        return None;
    }
    let layout = &v["layout"];
    let rects = |val: &serde_json::Value| -> Option<Vec<[u32; 4]>> {
        val.as_array()?
            .iter()
            .map(|r| {
                let a = r.as_array()?;
                let n = |i: usize| a.get(i).and_then(|x| x.as_u64()).map(|x| x as u32);
                (a.len() == 4).then_some([n(0)?, n(1)?, n(2)?, n(3)?])
            })
            .collect()
    };
    let pair = |val: &serde_json::Value| -> Option<[u32; 2]> {
        let a = val.as_array()?;
        (a.len() == 2).then_some([a[0].as_u64()? as u32, a[1].as_u64()? as u32])
    };
    let mirror = layout["legacy_mirror"]
        .as_array()?
        .iter()
        .map(|m| Some((pair(&m["from"])?, pair(&m["to"])?)))
        .collect::<Option<Vec<_>>>()?;
    Some(SkinLayout {
        overlay_rects: rects(&layout["overlay_rects"])?,
        slim_probe_rects: rects(&layout["slim_probe_rects"])?,
        legacy_mirror: mirror,
    })
}

/// The shipped layout (data/skins/rules.json), read once. None when the file is missing or broken.
pub fn layout() -> Option<SkinLayout> {
    static LAYOUT: std::sync::OnceLock<Option<SkinLayout>> = std::sync::OnceLock::new();
    LAYOUT
        .get_or_init(|| {
            crate::content::embedded::data_under("skins/rules.json").next().and_then(|(_, text)| parse_layout(text))
        })
        .clone()
}

fn in_rect(r: &[u32; 4], x: u32, y: u32) -> bool {
    x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
}

/// True when pixel (x, y) is in a second-layer rectangle (player_skin.gd `is_overlay_pixel`).
pub fn is_overlay_pixel(layout: &SkinLayout, x: u32, y: u32) -> bool {
    layout.overlay_rects.iter().any(|r| in_rect(r, x, y))
}

/// True when the arms use the slim layout: the 4th arm column is fully transparent in both arm bases (player_skin.gd
/// `is_slim_layout`). `rgba` is w*h*4 bytes; a skin smaller than 64x64 is never slim.
#[allow(dead_code)]
pub fn is_slim_layout(rgba: &[u8], w: u32, h: u32) -> bool {
    let Some(layout) = layout() else { return false };
    slim_with(&layout, rgba, w, h)
}

fn slim_with(layout: &SkinLayout, rgba: &[u8], w: u32, h: u32) -> bool {
    if w < SKIN_SIZE || h < SKIN_SIZE || rgba.len() < (w * h * 4) as usize {
        return false;
    }
    layout.slim_probe_rects.iter().all(|r| {
        (r[1]..r[1] + r[3]).all(|y| (r[0]..r[0] + r[2]).all(|x| rgba[((y * w + x) * 4 + 3) as usize] == 0))
    })
}

/// Copies a limb box (4 wide, 4 deep, 12 tall) mirrored left to right: top, bottom, front and back flip; the two side
/// faces swap (player_skin.gd `_mirror_box`).
fn mirror_box(px: &mut [u8], from: [u32; 2], to: [u32; 2]) {
    let src = px.to_vec();
    let at = |x: u32, y: u32| ((y * SKIN_SIZE + x) * 4) as usize;
    let pairs = [
        ([from[0] + 4, from[1], 4, 4], [to[0] + 4, to[1]]),
        ([from[0] + 8, from[1], 4, 4], [to[0] + 8, to[1]]),
        ([from[0], from[1] + 4, 4, 12], [to[0] + 8, to[1] + 4]),
        ([from[0] + 4, from[1] + 4, 4, 12], [to[0] + 4, to[1] + 4]),
        ([from[0] + 8, from[1] + 4, 4, 12], [to[0], to[1] + 4]),
        ([from[0] + 12, from[1] + 4, 4, 12], [to[0] + 12, to[1] + 4]),
    ];
    for (s, d) in pairs {
        for y in 0..s[3] {
            for x in 0..s[2] {
                let (a, b) = (at(s[0] + x, s[1] + y), at(d[0] + (s[2] - 1 - x), d[1] + y));
                px[b..b + 4].copy_from_slice(&src[a..a + 4]);
            }
        }
    }
}

/// Validates and normalises a skin file, never trusting the bytes (player_skin.gd `validate`): size cap, PNG signature,
/// header dimensions (64x64 or the old 64x32, checked before decoding), a real decode, the old format upgraded to
/// 64x64, the slim layout read, the base layer forced opaque, and a canonical re-encoding that is hashed.
pub fn validate_skin(bytes: &[u8], cap: usize) -> Result<ValidSkin, String> {
    if bytes.is_empty() {
        return Err("the file is empty".to_string());
    }
    if bytes.len() > cap {
        return Err("the file is too big".to_string());
    }
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err("that is not a PNG file".to_string());
    }
    if !png_is_skin_size(bytes) {
        return Err("a skin must be 64x64 pixels".to_string());
    }
    let layout = layout().ok_or_else(|| "the skin rules are missing".to_string())?;
    let img = macroquad::prelude::Image::from_file_with_format(bytes, Some(macroquad::prelude::ImageFormat::Png))
        .map_err(|_| "the PNG could not be read".to_string())?;
    let (w, h) = (img.width as u32, img.height as u32);
    let legacy = w == SKIN_SIZE && h == SKIN_SIZE / 2;
    if w != SKIN_SIZE || (h != SKIN_SIZE && !legacy) || img.bytes.len() != (w * h * 4) as usize {
        return Err("a skin must be 64x64 pixels".to_string());
    }
    let mut px = vec![0u8; (SKIN_SIZE * SKIN_SIZE * 4) as usize];
    px[..img.bytes.len()].copy_from_slice(&img.bytes);
    if legacy {
        for (from, to) in &layout.legacy_mirror {
            mirror_box(&mut px, *from, *to);
        }
    }
    let slim = !legacy && slim_with(&layout, &px, SKIN_SIZE, SKIN_SIZE);
    for y in 0..SKIN_SIZE {
        for x in 0..SKIN_SIZE {
            if !is_overlay_pixel(&layout, x, y) {
                px[((y * SKIN_SIZE + x) * 4 + 3) as usize] = 255;
            }
        }
    }
    let png = encode_png(&px, SKIN_SIZE, SKIN_SIZE);
    // The canonical bytes are what is uploaded and hashed, and the website refuses more than its cap.
    if png.len() > upload_max_bytes() {
        return Err(format!("the skin is too detailed to upload (over {} KB once saved)", upload_max_bytes() / 1024));
    }
    Ok(ValidSkin { hash: sha256_hex(&png), png, slim, rgba: px })
}

/// The arm style setting of the SKIN tab (ids as `settings_panel.rs` ARMS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ArmsSetting {
    Auto,
    Classic,
    Slim,
}

#[allow(dead_code)]
impl ArmsSetting {
    /// The setting for a saved id ("auto", "classic", "slim"). None for anything else.
    pub fn parse(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "classic" => Some(Self::Classic),
            "slim" => Some(Self::Slim),
            _ => None,
        }
    }
    /// The saved id.
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Classic => "classic",
            Self::Slim => "slim",
        }
    }
    /// The button label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Classic => "CLASSIC",
            Self::Slim => "SLIM",
        }
    }
    /// The next setting when ARMS is pressed: auto, classic, slim, auto.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Classic,
            Self::Classic => Self::Slim,
            Self::Slim => Self::Auto,
        }
    }
    /// True when the model should draw slim arms: Auto follows the skin's layout, the others force it.
    pub fn resolve(self, is_slim: bool) -> bool {
        match self {
            Self::Auto => is_slim,
            Self::Classic => false,
            Self::Slim => true,
        }
    }
}

/// [`ArmsSetting::resolve`] as a function: slim arms or not for a setting and the skin's detected layout.
#[allow(dead_code)]
pub fn resolve_arms(setting: ArmsSetting, is_slim: bool) -> bool {
    setting.resolve(is_slim)
}

// A small PNG writer: Sub filter, LZ77 and one fixed-Huffman deflate block. Dependency-free and deterministic, so two
// clients that normalise the same pixels get the same bytes and the same id.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    /// Writes the low `n` bits of `v`, least significant first.
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    /// Writes a Huffman code, most significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        let mut rev = 0;
        for i in 0..len {
            rev |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(rev, len);
    }
    fn literal_or_length(&mut self, sym: u32) {
        match sym {
            0..=143 => self.code(0x30 + sym, 8),
            144..=255 => self.code(0x190 + sym - 144, 9),
            256..=279 => self.code(sym - 256, 7),
            _ => self.code(0xC0 + sym - 280, 8),
        }
    }
}

const LEN_BASE: [u32; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// A zlib stream (one fixed-Huffman block) for `data`.
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut w = Bits { out: vec![0x78, 0x01], acc: 0, n: 0 };
    w.put(1, 1);
    w.put(1, 2);
    let mut head: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let key = |i: usize| (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
    let mut i = 0;
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if i + 3 <= data.len() {
            if let Some(&cand) = head.get(&key(i)) {
                let dist = i - cand;
                if dist <= 32768 {
                    let mut len = 0;
                    while len < 258 && i + len < data.len() && data[cand + len] == data[i + len] {
                        len += 1;
                    }
                    if len >= 3 {
                        best = (len, dist);
                    }
                }
            }
        }
        let step = if best.0 >= 3 {
            let li = LEN_BASE.iter().rposition(|&b| b <= best.0 as u32).unwrap_or(0);
            w.literal_or_length(257 + li as u32);
            w.put(best.0 as u32 - LEN_BASE[li], LEN_EXTRA[li]);
            let di = DIST_BASE.iter().rposition(|&b| b <= best.1 as u32).unwrap_or(0);
            w.code(di as u32, 5);
            w.put(best.1 as u32 - DIST_BASE[di], DIST_EXTRA[di]);
            best.0
        } else {
            w.literal_or_length(data[i] as u32);
            1
        };
        for j in i..i + step {
            if j + 3 <= data.len() {
                head.insert(key(j), j);
            }
        }
        i += step;
    }
    w.literal_or_length(256);
    if w.n > 0 {
        w.put(0, 8 - w.n);
    }
    let mut out = w.out;
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut crc_in = kind.to_vec();
    crc_in.extend_from_slice(body);
    out.extend_from_slice(&crc_in);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

/// Encodes `rgba` (w*h*4 bytes) as an 8-bit RGBA PNG with the Sub filter on every row.
pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let stride = (w * 4) as usize;
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    for y in 0..h as usize {
        let row = &rgba[y * stride..(y + 1) * stride];
        raw.push(1);
        for (i, &b) in row.iter().enumerate() {
            raw.push(b.wrapping_sub(if i >= 4 { row[i - 4] } else { 0 }));
        }
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = PNG_SIGNATURE.to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// FROM URL: a 64x64 PNG downloaded from an https link. Returns the validated bytes.
pub fn fetch_url_skin(url: &str, rules: &SkinRules) -> Result<Vec<u8>, String> {
    let bytes = fetch_https(url, rules, &[], read_limit(), &PNG_TYPES)?;
    skin_from_png(bytes).map(|(_, b)| b)
}

/// True when `name` is a Minecraft username by the rules: 1 to `name_max` characters, all from `name_chars`.
pub fn valid_minecraft_name(name: &str, rules: &SkinRules) -> bool {
    !name.is_empty() && name.len() <= rules.name_max && name.chars().all(|c| rules.name_chars.contains(c))
}

/// The skin URL in a Mojang profile: the `textures` property (base64 JSON) with a SKIN url on the textures host.
pub fn profile_skin_url(profile: &serde_json::Value, rules: &SkinRules) -> Option<String> {
    let props = profile["properties"].as_array()?;
    let prop = props.iter().find(|p| p["name"].as_str() == Some("textures"))?;
    let raw = crate::content::embedded::base64_decode(prop["value"].as_str()?);
    let decoded: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let url = decoded["textures"]["SKIN"]["url"].as_str()?;
    url_refusal(url, rules, &[rules.textures_host.as_str()]).is_none().then(|| url.to_string())
}

/// FROM MC NAME: the name's skin through Mojang's APIs (name to uuid, uuid to profile, then the textures host). Returns
/// the validated PNG bytes.
pub fn fetch_minecraft_skin(name: &str, rules: &SkinRules) -> Result<Vec<u8>, String> {
    let name = name.trim();
    if !valid_minecraft_name(name, rules) {
        return Err("a Minecraft name is 1-16 letters, numbers or underscores".to_string());
    }
    let lookup_url = rules.lookup_url.replace("{name}", name);
    let lookup = fetch_https(&lookup_url, rules, &[rules.lookup_host.as_str()], read_limit(), &JSON_TYPES)?;
    let found: serde_json::Value =
        serde_json::from_slice(&lookup).map_err(|_| "no Minecraft player was found".to_string())?;
    let uuid = found["id"].as_str().unwrap_or("");
    if uuid.len() != 32 || !uuid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("no Minecraft player was found".to_string());
    }
    let profile_url = rules.profile_url.replace("{uuid}", uuid);
    let body = fetch_https(&profile_url, rules, &[rules.profile_host.as_str()], read_limit(), &JSON_TYPES)?;
    let profile: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "the profile could not be read".to_string())?;
    let url = profile_skin_url(&profile, rules).ok_or_else(|| "that player has no skin".to_string())?;
    let file = fetch_https(&url, rules, &[rules.textures_host.as_str()], read_limit(), &PNG_TYPES)?;
    skin_from_png(file).map(|(_, b)| b)
}

/// The list id a picked skin is saved under: `custom.png` in the skins folder.
pub const CUSTOM_STEM: &str = "custom";

/// Saves validated skin bytes as the user's own skin (`<skins folder>/custom.png`), replacing it in one step. Returns
/// the list id ([`clean_skin`] of the stem).
pub fn save_custom_skin(bytes: &[u8]) -> std::io::Result<String> {
    let dir = skin_dir().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no user folder"))?;
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{CUSTOM_STEM}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(format!("{CUSTOM_STEM}.png")))?;
    Ok(clean_skin(CUSTOM_STEM))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_id_is_the_sha256_of_the_png_bytes() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        assert_eq!(id.len(), 64);
        assert!(verify_skin(&id, &bytes));
        assert!(!verify_skin(&id, &png_head(32, 32)), "a wrong-size PNG never verifies");
        assert!(!verify_skin(&sha256_hex(b"other"), &bytes), "bytes must hash to the id");
    }

    fn png_head(w: u32, h: u32) -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    #[test]
    fn accepts_only_64_by_64_png_headers() {
        assert!(png_is_skin_size(&png_head(64, 64)));
        assert!(!png_is_skin_size(&png_head(32, 64)));
        assert!(!png_is_skin_size(&png_head(64, 128)));
        assert!(!png_is_skin_size(b"GIF89a..............................."));
        assert!(!png_is_skin_size(&png_head(64, 64)[..20]));
    }

    #[test]
    fn the_website_holds_only_a_listed_skin_hash() {
        let hash = "d".repeat(64);
        assert!(holds(&[hash.clone()], &hash), "a listed hash is held");
        assert!(!holds(&[], &hash), "an empty list holds nothing");
        assert!(!holds(&[hash.clone()], &"e".repeat(64)), "another hash is not held");
        assert!(!holds(&["NIGHTOWL".into()], "NIGHTOWL"), "a non-hash id is never held");
        assert!(!holds(&[hash.clone()], SKIN_DEFAULT), "DEFAULT is never held");
    }

    #[test]
    fn default_is_first_and_missing_folder_gives_only_default() {
        assert_eq!(list_skins(None), vec![SKIN_DEFAULT.to_string()]);
        let missing = std::env::temp_dir().join("lr_skins_missing_folder_for_test");
        assert_eq!(list_skins(Some(&missing)), vec![SKIN_DEFAULT.to_string()]);
    }

    #[test]
    fn lists_valid_skins_and_rejects_wrong_size_or_non_png() {
        let dir = std::env::temp_dir().join(format!("lr_skins_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("night.png"), png_head(64, 64)).unwrap();
        std::fs::write(dir.join("small.png"), png_head(32, 32)).unwrap();
        std::fs::write(dir.join("fake.png"), b"not a png at all, just text padding here").unwrap();
        std::fs::write(dir.join("notes.txt"), png_head(64, 64)).unwrap();
        let ids = list_skins(Some(&dir));
        assert_eq!(ids, vec!["DEFAULT".to_string(), "NIGHT".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_saved_skin_falls_back_to_default() {
        let ids = vec!["DEFAULT".to_string(), "NIGHT".to_string()];
        assert_eq!(resolve_skin("night", &ids), "NIGHT");
        assert_eq!(resolve_skin("GONE", &ids), "DEFAULT");
        assert_eq!(resolve_skin("", &ids), "DEFAULT");
    }

    fn shipped_config() -> SkinConfig {
        config().expect("data/skins/config.json loads")
    }

    #[test]
    fn fetch_url_and_cache_file_name_use_the_id() {
        let id = sha256_hex(b"skin");
        let cfg = shipped_config();
        assert_eq!(
            skin_backend_url(&cfg, "http://host:8105/", &id),
            format!("http://host:8105/api/skins/files/{id}.png"),
            "the legacy Godot route (game/autoload/skin_sync.gd:152) still answers on a Godot backend"
        );
        assert_eq!(
            skin_v1_url(&cfg, "http://host:8105/", &id),
            format!("http://host:8105/api/v1/skins/{id}"),
            "lr_server's own route (src/bin/lr_server/skins/skins.rs)"
        );
        let dir = Path::new("cache");
        assert_eq!(cached_path(dir, &id), dir.join(format!("{id}.png")));
    }

    #[test]
    fn skins_host_url_and_file_name_use_the_id() {
        let id = sha256_hex(b"skin");
        let cfg = shipped_config();
        assert_eq!(cfg.host_base, "https://skins.kyttuslab.org");
        assert_eq!(skin_host_url(&cfg, &id), format!("https://skins.kyttuslab.org/{id}.png"));
    }

    #[test]
    fn fetch_urls_lists_v1_then_host_then_legacy() {
        let id = sha256_hex(b"skin");
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105/", &id);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], skin_v1_url(&cfg, "http://host:8105/", &id));
        assert_eq!(urls[1], skin_host_url(&cfg, &id));
        assert_eq!(urls[2], skin_backend_url(&cfg, "http://host:8105/", &id));
    }

    /// A canonical, fully valid skin and its id.
    fn good_skin() -> (String, Vec<u8>) {
        skin_from_png(fixture_png(false)).expect("the fixture is a valid skin")
    }

    /// A PNG with a good 64x64 header and no body: the old check accepted it, and decoding it panics.
    fn hostile_skin() -> (String, Vec<u8>) {
        let bytes = png_head(64, 64);
        (sha256_hex(&bytes), bytes)
    }

    #[test]
    fn the_games_own_encoder_output_passes_the_shared_check() {
        let (id, bytes) = good_skin();
        assert_eq!(skin_validate::check_skin_png(&bytes, max_bytes()), Ok(()));
        assert!(verify_download(&id, &bytes));
        assert!(verify_skin(&id, &bytes));
        // A skin with a clear hat layer (the usual case) passes too.
        let mut px = fixture_rgba(false);
        px[px_at(40, 5, 64) + 3] = 0;
        let png = encode_png(&px, 64, 64);
        assert_eq!(skin_validate::check_skin_png(&png, max_bytes()), Ok(()));
    }

    #[test]
    fn a_header_only_download_matches_its_hash_but_is_not_a_verified_download() {
        let (id, bytes) = hostile_skin();
        assert!(verify_skin(&id, &bytes), "the old header check is fooled");
        assert!(!verify_download(&id, &bytes), "the full check is not");
    }

    #[test]
    fn fetch_uses_the_first_route_that_answers_with_a_verified_skin() {
        let (id, bytes) = good_skin();
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            Some(bytes.clone())
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, vec![skin_v1_url(&cfg, "http://host:8105", &id)], "nothing else is asked once the backend answers");
    }

    #[test]
    fn a_failing_route_falls_through_to_the_next_one() {
        let (id, bytes) = good_skin();
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            (url == skin_backend_url(&cfg, "http://host:8105", &id)).then(|| bytes.clone())
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, urls, "all three routes were tried in order");
    }

    #[test]
    fn a_reply_that_does_not_fully_verify_falls_through_to_the_next_route() {
        let (id, bytes) = good_skin();
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let got = first_verified_skin(&urls, &id, |url| {
            if url.starts_with(&cfg.host_base) {
                Some(bytes.clone())
            } else {
                Some(png_head(32, 32))
            }
        });
        assert_eq!(got, Some(bytes), "a wrong-hash or wrong-size reply is not used");
        // A hostile file whose hash is right is not used either, even from the only route that answers.
        let (hostile_id, hostile) = hostile_skin();
        let urls = fetch_urls(&cfg, "http://host:8105", &hostile_id);
        assert_eq!(first_verified_skin(&urls, &hostile_id, |_| Some(hostile.clone())), None);
    }

    #[test]
    fn nothing_verifies_when_every_route_fails() {
        let (id, _) = good_skin();
        let urls = fetch_urls(&shipped_config(), "http://host:8105", &id);
        assert_eq!(first_verified_skin(&urls, &id, |_| None), None);
    }

    /// A tiny http server for the download tests: answers `GET <path>` from `routes` (404 otherwise) and records every
    /// request line. Stops when the returned flag is set.
    fn serve_routes(routes: Vec<(String, Vec<u8>, &'static str)>) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let (seen2, stop2) = (seen.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                let Ok((mut conn, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                conn.set_nonblocking(false).unwrap();
                conn.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut got = Vec::new();
                let mut b = [0u8; 2048];
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match conn.read(&mut b) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&b[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&got).lines().next().unwrap_or("").to_string();
                seen2.lock().unwrap().push(line.clone());
                let path = line.split(' ').nth(1).unwrap_or("").to_string();
                let reply = match routes.iter().find(|(p, _, _)| *p == path) {
                    Some((_, body, kind)) => [format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(), body.clone()].concat(),
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
                };
                let _ = conn.write_all(&reply);
            }
        });
        (port, seen, stop)
    }

    /// The shipped config with the skins host pointed at a name that never resolves, so a test never touches the internet.
    fn offline_host_config() -> SkinConfig {
        SkinConfig { host_base: "https://skins.invalid".to_string(), ..shipped_config() }
    }

    fn temp_cache(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_skins_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_download_walks_the_routes_checks_the_skin_and_caches_it() {
        use std::sync::atomic::Ordering;
        let (id, bytes) = good_skin();
        let cfg = offline_host_config();
        // lr_server has no copy (404 on its v1 route); the legacy route serves it. The skins host is unreachable.
        let (port, seen, stop) = serve_routes(vec![(format!("/api/skins/files/{id}.png"), bytes.clone(), "image/png")]);
        let dir = temp_cache("walk");
        assert!(fetch_skin_blocking(&cfg, &format!("http://127.0.0.1:{port}"), &dir, &id));
        stop.store(true, Ordering::SeqCst);
        assert_eq!(std::fs::read(cached_path(&dir, &id)).unwrap(), bytes);
        assert!(!fetch_needed(&dir, &id));
        let asked = seen.lock().unwrap().clone();
        assert_eq!(asked, vec![format!("GET /api/v1/skins/{id} HTTP/1.1"), format!("GET /api/skins/files/{id}.png HTTP/1.1")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hostile_download_with_the_right_hash_is_never_written_to_the_cache() {
        use std::sync::atomic::Ordering;
        let (id, hostile) = hostile_skin();
        let cfg = offline_host_config();
        let (port, _seen, stop) = serve_routes(vec![
            (format!("/api/v1/skins/{id}"), hostile.clone(), "image/png"),
            (format!("/api/skins/files/{id}.png"), hostile, "image/png"),
        ]);
        let dir = temp_cache("hostile");
        assert!(!fetch_skin_blocking(&cfg, &format!("http://127.0.0.1:{port}"), &dir, &id));
        stop.store(true, Ordering::SeqCst);
        assert!(!cached_path(&dir, &id).exists(), "a skin that would crash the decoder never reaches the cache");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_that_is_not_a_picture_by_its_content_type_is_dropped() {
        use std::sync::atomic::Ordering;
        let (id, bytes) = good_skin();
        let cfg = offline_host_config();
        let (port, _seen, stop) = serve_routes(vec![
            (format!("/api/v1/skins/{id}"), bytes.clone(), "text/html"),
            (format!("/api/skins/files/{id}.png"), bytes, "text/html"),
        ]);
        let dir = temp_cache("html");
        assert!(!fetch_skin_blocking(&cfg, &format!("http://127.0.0.1:{port}"), &dir, &id));
        stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_download_is_not_retried_before_the_cool_down_and_the_number_of_jobs_is_capped() {
        use std::time::{Duration, Instant};
        let (start, retry) = (Instant::now(), Duration::from_secs(60));
        let mut book = Book::default();
        assert!(book.begin("down:a", start, 2));
        assert!(!book.begin("down:a", start, 2), "already running");
        assert!(book.begin("down:b", start, 2));
        assert!(!book.begin("down:c", start, 2), "two jobs already run");
        book.finish("down:a", false, start, retry);
        assert!(!book.begin("down:a", start + Duration::from_secs(59), 2), "cooling down");
        assert!(book.begin("down:a", start + retry, 2), "tried again after the cool-down");
        book.finish("down:a", true, start + retry, retry);
        assert!(book.begin("down:a", start + retry, 3), "a success clears the cool-down");
    }

    #[test]
    fn download_failures_are_described_without_an_address() {
        let said = |e: NetError| download_error_text(&e);
        assert_eq!(said(NetError::Timeout), "the download timed out");
        assert_eq!(said(NetError::Protocol("RESPONSE TOO LARGE".into())), "the file is too big");
        assert_eq!(said(NetError::Protocol("UNEXPECTED CONTENT TYPE".into())), "that address is not a PNG picture");
        assert_eq!(said(NetError::Transport("invalid peer certificate: UnknownIssuer".into())), "the secure connection could not be verified");
        assert_eq!(said(NetError::Transport("connection refused (os error 61)".into())), "the download failed: could not connect");
        assert_eq!(said(NetError::Refused("THAT ADDRESS IS NOT PUBLIC".into())), "that address is not public");
        assert_eq!(said(NetError::Http { status: 404, code: String::new(), error: String::new(), body: serde_json::Value::Null }), "the server answered 404");
        assert!(!said(NetError::Transport("tcp connect error to http://secret.example:81".into())).contains("secret"));
    }

    #[test]
    fn from_url_and_from_mc_name_refuse_what_the_rules_refuse_before_any_connection() {
        let rules = rules().expect("rules load");
        for url in ["http://example.org/a.png", "https://localhost/a.png", "https://127.0.0.1/a.png", "https://user@example.org/a.png", "https://example.org:8443/a.png"] {
            assert!(fetch_url_skin(url, &rules).is_err(), "{url}");
        }
        assert!(fetch_minecraft_skin("not a name!", &rules).is_err());
        assert!(fetch_minecraft_skin("", &rules).is_err());
    }

    #[test]
    fn a_cached_skin_is_not_fetched_again() {
        let dir = std::env::temp_dir().join(format!("lr_skins_cache_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (id, bytes) = good_skin();
        assert!(fetch_needed(&dir, &id), "nothing cached yet");
        write_skin_file(&dir, &id, &bytes).unwrap();
        assert!(!fetch_needed(&dir, &id), "a verified cached copy is kept");
        std::fs::write(cached_path(&dir, &id), b"damaged").unwrap();
        assert!(fetch_needed(&dir, &id), "a damaged copy is fetched again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn shipped_rules() -> SkinRules {
        rules().expect("data/skins/rules.json loads")
    }

    #[test]
    fn catalogue_loads_from_data() {
        let list = catalog();
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ref_classic", "ref_slim"], "the shipped catalogue, sorted by id");
        let classic = &list[0];
        assert_eq!(classic.name, "Reference (classic)");
        assert_eq!(classic.arms, "classic");
        assert_eq!(classic.image, "skins/ref_classic.png");
        assert_eq!(list[1].name, "Reference (slim)");
        assert_eq!(list[1].arms, "slim");
    }

    #[test]
    fn catalogue_entries_follow_the_godot_format() {
        let good = r#"{"schema":"lr.skin/1","id":"x","name":"X","image":"skins/x.png","arms":"slim"}"#;
        assert!(parse_catalog_skin(good).is_some());
        assert!(parse_catalog_skin(&good.replace("lr.skin/1", "lr.skin/2")).is_none(), "another schema is skipped");
        assert!(parse_catalog_skin(&good.replace("slim", "wide")).is_none(), "an unknown arm style is refused");
        assert!(parse_catalog_skin(&good.replace("\"name\":\"X\"", "\"name\":\"\"")).is_none(), "a nameless skin is refused");
    }

    #[test]
    fn rules_hold_the_godot_standard_values() {
        let r = shipped_rules();
        assert_eq!(r.scheme, "https://");
        assert_eq!(r.max_url_length, 512, "PlayerSkin.MAX_URL_LENGTH (game/player/player_skin.gd:108)");
        assert_eq!(r.url_timeout_secs, 5, "URL_TIMEOUT_SECONDS (game/autoload/skin_sync.gd:302)");
        assert_eq!(r.url_port, 443);
        assert_eq!(r.name_max, 16, "valid_minecraft_name (game/player/player_skin.gd:173)");
        assert_eq!(r.lookup_host, "api.mojang.com");
        assert_eq!(r.profile_host, "sessionserver.mojang.com");
        assert_eq!(r.textures_host, "textures.minecraft.net", "MC_HOSTS (game/player/player_skin.gd:107)");
        assert!(r.blocked_suffixes.iter().any(|s| s == ".local"));
    }

    #[test]
    fn url_rules_refuse_what_godot_refuses() {
        let r = shipped_rules();
        assert_eq!(url_refusal("https://example.com/skin.png", &r, &[]), None);
        assert!(url_refusal("http://example.com/skin.png", &r, &[]).is_some(), "https only");
        assert!(url_refusal("https://user:pw@example.com/skin.png", &r, &[]).is_some(), "no login");
        assert!(url_refusal("https://localhost/skin.png", &r, &[]).is_some(), "no loopback");
        assert!(url_refusal("https://pc.local/skin.png", &r, &[]).is_some(), "no LAN names");
        assert!(url_refusal("https://192.168.1.5/skin.png", &r, &[]).is_some(), "no IP literals");
        assert!(url_refusal("https://example.com:8443/skin.png", &r, &[]).is_some(), "standard port only");
        assert!(url_refusal("https://example.com:443/skin.png", &r, &[]).is_none(), "the standard port is fine");
        assert!(url_refusal("https://example.com/skin.png", &r, &["textures.minecraft.net"]).is_some(), "a host list narrows it");
    }

    #[test]
    fn the_url_rules_come_from_data() {
        let text = include_str!("../data/skins/rules.json").replace("\"max_length\": 512", "\"max_length\": 20");
        let edited = parse_rules(&text).expect("the edited rules still parse");
        let long = "https://example.com/a-long-path/skin.png";
        assert!(url_refusal(long, &edited, &[]).is_some(), "a shorter limit in data refuses it");
        assert_eq!(url_refusal(long, &shipped_rules(), &[]), None, "the shipped limit allows it");
    }

    #[test]
    fn from_url_rejects_non_64_by_64_and_non_png() {
        assert!(skin_from_png(fixture_png(false)).is_ok());
        assert_eq!(
            skin_from_png(png_head(64, 64)).unwrap_err(),
            "the PNG could not be read",
            "a bare header is not a skin: the pixels must decode"
        );
        assert_eq!(skin_from_png(png_head(32, 32)).unwrap_err(), "a skin must be 64x64 pixels");
        assert_eq!(
            skin_from_png(b"GIF89a..............................".to_vec()).unwrap_err(),
            "that is not a PNG file"
        );
        assert!(skin_from_png(vec![0u8; max_bytes() + 1]).is_err(), "over the cap in data/skins/config.json");
    }

    #[test]
    fn config_holds_the_godot_standard_values() {
        let c = shipped_config();
        assert_eq!(c.upload_max_bytes, 64 * 1024, "the website's SKIN_MAX_BYTES (lr_server skins.rs, skins.mjs)");
        assert_eq!(c.upload_max_bytes, skin_validate::SKIN_MAX_BYTES, "the client and the server share one cap");
        assert_eq!(c.max_bytes, 65536, "PlayerSkin.MAX_BYTES (game/player/player_skin.gd:16), not the old 16 KB");
        assert_eq!(read_limit(), 65537, "FETCH_LIMIT_BYTES = MAX_BYTES + 1 (game/autoload/skin_sync.gd:23)");
        assert_eq!(c.host_base, "https://skins.kyttuslab.org");
        assert_eq!(c.host_path, "/{id}.png");
        assert_eq!(c.backend_path, "/api/skins/files/{id}.png", "game/autoload/skin_sync.gd:152");
        assert_eq!(c.v1_path, "/api/v1/skins/{id}", "lr_server's own skin route");
        assert_eq!(c.retry_secs, 60, "skin_sync.gd RETRY_SECONDS");
        assert_eq!(c.max_concurrent, 4);
        assert_eq!(c.cache_max, 16, "Godot PEER_CACHE_MAX");
        assert_eq!(c.timeout_secs, 8, "the timeout this client has always used for a skin download");
    }

    #[test]
    fn the_fetch_settings_come_from_data() {
        let id = sha256_hex(b"skin");
        let text = include_str!("../data/skins/config.json")
            .replace("\"max_bytes\": 65536", "\"max_bytes\": 100")
            .replace("\"host_base\": \"https://skins.kyttuslab.org\"", "\"host_base\": \"https://mirror.example.org/\"")
            .replace("\"backend_path\": \"/api/skins/files/{id}.png\"", "\"backend_path\": \"/x/{id}\"")
            .replace("\"timeout_secs\": 8", "\"timeout_secs\": 3");
        let edited = parse_config(&text).expect("the edited config still parses");
        assert_eq!(edited.max_bytes, 100);
        assert_eq!(edited.host_base, "https://mirror.example.org", "a trailing slash is dropped");
        assert_eq!(edited.timeout_secs, 3);
        assert_eq!(skin_host_url(&edited, &id), format!("https://mirror.example.org/{id}.png"));
        assert_eq!(skin_backend_url(&edited, "http://h/", &id), format!("http://h/x/{id}"));
        assert_eq!(shipped_config().timeout_secs, 8, "the shipped value is unchanged");
        let png = fixture_png(false);
        let over = [png.clone(), vec![0u8; edited.max_bytes]].concat();
        assert!(skin_from_png_capped(over, edited.max_bytes).is_err(), "a lower cap in data refuses it");
        assert!(skin_from_png_capped(png.clone(), png.len()).is_ok(), "a cap that fits accepts it");
        assert!(skin_from_png_capped(png.clone(), png.len() - 1).is_err(), "one byte over the cap is refused");
    }

    #[test]
    fn a_config_with_the_wrong_schema_or_a_missing_value_is_refused() {
        let text = include_str!("../data/skins/config.json");
        assert!(parse_config(text).is_some());
        assert!(parse_config(&text.replace("lr.skin_config/1", "lr.skin_config/2")).is_none());
        assert!(parse_config(&text.replace("\"timeout_secs\": 8", "\"timeout_secs\": \"8\"")).is_none());
    }
    #[test]
    fn minecraft_names_and_profiles_follow_the_rules() {
        let r = shipped_rules();
        assert!(valid_minecraft_name("Notch_1", &r));
        assert!(!valid_minecraft_name("", &r));
        assert!(!valid_minecraft_name("seventeen_chars_x", &r), "16 characters at most");
        assert!(!valid_minecraft_name("bad-name", &r), "letters, numbers and underscores only");
        let good: serde_json::Value = serde_json::from_str(
            r#"{"properties":[{"name":"textures","value":"eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYzEyMyJ9fX0="}]}"#,
        )
        .unwrap();
        assert_eq!(
            profile_skin_url(&good, &r).as_deref(),
            Some("https://textures.minecraft.net/texture/abc123")
        );
        let other: serde_json::Value = serde_json::from_str(
            r#"{"properties":[{"name":"textures","value":"eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHBzOi8vZXZpbC5leGFtcGxlLmNvbS94LnBuZyJ9fX0="}]}"#,
        )
        .unwrap();
        assert_eq!(profile_skin_url(&other, &r), None, "a skin URL off the textures host is ignored");
    }

    // ---- full validation (Godot player_skin.gd validate / upgrade_legacy / is_slim_layout) ----

    fn shipped_layout() -> SkinLayout {
        layout().expect("data/skins/rules.json layout loads")
    }

    fn px_at(x: u32, y: u32, w: u32) -> usize {
        ((y * w + x) * 4) as usize
    }

    /// A 64x64 RGBA skin, every pixel opaque. A slim one clears the 4th arm column in both arm bases.
    fn fixture_rgba(slim: bool) -> Vec<u8> {
        let mut px = vec![255u8; 64 * 64 * 4];
        for y in 0..64u32 {
            for x in 0..64u32 {
                let i = px_at(x, y, 64);
                px[i] = (x * 4) as u8;
                px[i + 1] = (y * 4) as u8;
                px[i + 2] = 90;
            }
        }
        if slim {
            for r in &shipped_layout().slim_probe_rects {
                for y in r[1]..r[1] + r[3] {
                    for x in r[0]..r[0] + r[2] {
                        px[px_at(x, y, 64) + 3] = 0;
                    }
                }
            }
        }
        px
    }

    fn fixture_png(slim: bool) -> Vec<u8> {
        encode_png(&fixture_rgba(slim), 64, 64)
    }

    #[test]
    fn the_encoder_round_trips_through_the_decoder_and_fits_the_website_cap() {
        let mut px = fixture_rgba(false);
        px[px_at(40, 5, 64) + 3] = 0;
        let png = encode_png(&px, 64, 64);
        assert!(png_is_skin_size(&png));
        assert!(png.len() <= 16 * 1024, "a canonical skin fits the website's 16 KB cap ({} bytes)", png.len());
        let img = macroquad::prelude::Image::from_file_with_format(&png, Some(macroquad::prelude::ImageFormat::Png))
            .expect("the PNG decodes");
        assert_eq!((img.width, img.height), (64, 64));
        assert_eq!(img.bytes, px, "decoding gives back the pixels that were encoded");
        assert_eq!(encode_png(&px, 64, 64), png, "encoding is deterministic");
    }

    /// What the upload sends and the website checks: canonical bytes under its cap, hashed as received.
    fn assert_uploadable(png: &[u8], what: &str) {
        let (hash, canonical) = skin_from_png(png.to_vec()).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(canonical.len() <= upload_max_bytes(), "{what}: {} bytes is over the website cap", canonical.len());
        assert!(png_is_skin_size(&canonical), "{what}: the website wants exactly 64x64");
        assert_eq!(hash, sha256_hex(&canonical), "{what}: the announced id is the hash of the bytes the website receives");
        let again = skin_from_png(canonical.clone()).expect("canonical bytes validate again");
        assert_eq!(again, (hash, canonical), "{what}: canonical bytes are a fixed point");
    }

    #[test]
    fn canonical_skins_fit_the_website_cap_and_the_announced_hash_is_of_the_canonical_bytes() {
        assert_uploadable(&fixture_png(false), "classic fixture");
        assert_uploadable(&fixture_png(true), "slim fixture");
        for skin in catalog() {
            if let Some(png) = catalog_png(&skin) {
                assert_uploadable(&png, &skin.id);
            }
        }
        // A 64x32 skin is upgraded to 64x64 before it is hashed or uploaded.
        let old = encode_png(&fixture_rgba(false)[..64 * 32 * 4], 64, 32);
        let (_, canonical) = skin_from_png(old).expect("an old skin upgrades");
        assert!(png_is_skin_size(&canonical));
        // Pure noise cannot compress (about 17 KB with this encoder, 16 KB raw): a detailed skin like that was refused
        // under the old 16 KB cap, but the Godot and server cap is 64 KB, so it is accepted and passes the shared check.
        let mut state = 12345u32;
        let noise: Vec<u8> = (0..64 * 64 * 4)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let noisy = encode_png(&noise, 64, 64);
        assert!(noisy.len() > 16 * 1024, "the old cap would have refused it ({} bytes)", noisy.len());
        assert_uploadable(&noisy, "noise");
        let (_, canonical) = skin_from_png(noisy).expect("a noisy skin under 64 KB is accepted");
        assert_eq!(skin_validate::check_skin_png(&canonical, max_bytes()), Ok(()));
    }

    #[test]
    fn an_old_64x32_skin_is_upgraded_to_64x64_with_mirrored_limbs() {
        let mut px = vec![255u8; 64 * 32 * 4];
        // a marker on the right leg's front face (4..8, 20..32); the left leg front is (20..24, 52..64) mirrored
        let marker = px_at(4, 20, 64);
        px[marker..marker + 4].copy_from_slice(&[200, 10, 20, 255]);
        let v = validate_skin(&encode_png(&px, 64, 32), 65536).expect("a legacy skin is accepted");
        assert_eq!(v.rgba.len(), 64 * 64 * 4, "the result is 64x64");
        assert!(!v.slim, "an old skin has only classic arms");
        let img = macroquad::prelude::Image::from_file_with_format(&v.png, Some(macroquad::prelude::ImageFormat::Png))
            .unwrap();
        assert_eq!((img.width, img.height), (64, 64));
        let right_edge = px_at(16 + 4 + 3, 52, 64);
        assert_eq!(&v.rgba[right_edge..right_edge + 3], &[200, 10, 20], "the left leg is the right one mirrored");
        assert_eq!(v.rgba[right_edge + 3], 255);
    }

    #[test]
    fn a_transparent_body_pixel_becomes_opaque_but_a_transparent_hat_pixel_stays() {
        let mut px = fixture_rgba(false);
        px[px_at(10, 10, 64) + 3] = 0; // head, base layer
        px[px_at(40, 5, 64) + 3] = 0; // hat, overlay
        px[px_at(20, 40, 64) + 3] = 0; // jacket, overlay
        let v = validate_skin(&encode_png(&px, 64, 64), 65536).unwrap();
        assert_eq!(v.rgba[px_at(10, 10, 64) + 3], 255, "base pixels are forced opaque");
        assert_eq!(v.rgba[px_at(40, 5, 64) + 3], 0, "a hat pixel may stay transparent");
        assert_eq!(v.rgba[px_at(20, 40, 64) + 3], 0, "a jacket pixel may stay transparent");
    }

    #[test]
    fn skins_that_differ_only_in_invisible_base_alpha_hash_the_same() {
        let a = fixture_rgba(false);
        let mut b = a.clone();
        b[px_at(10, 10, 64) + 3] = 0;
        b[px_at(30, 12, 64) + 3] = 7;
        let (ha, pa) = skin_from_png(encode_png(&a, 64, 64)).unwrap();
        let (hb, pb) = skin_from_png(encode_png(&b, 64, 64)).unwrap();
        assert_eq!(ha, hb);
        assert_eq!(pa, pb, "the canonical bytes are identical");
        assert_eq!(ha, sha256_hex(&pa), "the id is the SHA-256 of the canonical PNG");
        let mut c = a.clone();
        c[px_at(40, 5, 64) + 3] = 0;
        assert_ne!(skin_from_png(encode_png(&c, 64, 64)).unwrap().0, ha, "a visible overlay difference changes the id");
    }

    #[test]
    fn slim_and_classic_layouts_are_detected() {
        assert!(is_slim_layout(&fixture_rgba(true), 64, 64));
        assert!(!is_slim_layout(&fixture_rgba(false), 64, 64));
        assert!(!is_slim_layout(&fixture_rgba(true)[..64 * 32 * 4], 64, 32), "a short image is never slim");
        let mut one_pixel = fixture_rgba(true);
        let r = shipped_layout().slim_probe_rects[1];
        one_pixel[px_at(r[0], r[1], 64) + 3] = 255;
        assert!(!is_slim_layout(&one_pixel, 64, 64), "one painted pixel in the 4th column means classic arms");
        assert!(validate_skin(&fixture_png(true), 65536).unwrap().slim, "slim is read before the base is made opaque");
        assert!(!validate_skin(&fixture_png(false), 65536).unwrap().slim);
    }

    #[test]
    fn a_bad_size_or_a_non_png_is_refused_with_a_reason() {
        let wrong = encode_png(&vec![255u8; 63 * 64 * 4], 63, 64);
        assert_eq!(validate_skin(&wrong, 65536).unwrap_err(), "a skin must be 64x64 pixels");
        let tall = encode_png(&vec![255u8; 64 * 128 * 4], 64, 128);
        assert_eq!(validate_skin(&tall, 65536).unwrap_err(), "a skin must be 64x64 pixels");
        assert_eq!(validate_skin(b"hello, not a png at all", 65536).unwrap_err(), "that is not a PNG file");
        assert_eq!(validate_skin(b"", 65536).unwrap_err(), "the file is empty");
        assert_eq!(validate_skin(&fixture_png(false), 100).unwrap_err(), "the file is too big");
        let mut broken = fixture_png(false);
        broken.truncate(60);
        assert_eq!(validate_skin(&broken, 65536).unwrap_err(), "the PNG could not be read", "pixels must really decode");
    }

    #[test]
    fn the_overlay_rects_come_from_data() {
        let text = include_str!("../data/skins/rules.json");
        let shipped = parse_layout(text).expect("shipped layout parses");
        assert_eq!(shipped.overlay_rects.len(), 6, "hat, jacket, two sleeves, two pants (player_skin.gd OVERLAY_RECTS)");
        assert!(is_overlay_pixel(&shipped, 40, 5), "the hat is an overlay");
        assert!(!is_overlay_pixel(&shipped, 10, 10), "the head is not");
        assert_eq!(shipped.overlay_rects[0], [32, 0, 32, 16]);
        let edited = parse_layout(&text.replace("[[32, 0, 32, 16], ", "[")).expect("edited layout parses");
        assert!(!is_overlay_pixel(&edited, 40, 5), "a changed rect in data changes what is overlay");
        assert!(parse_layout(&text.replace("\"overlay_rects\"", "\"nope\"")).is_none(), "a missing list is refused");
    }

    #[test]
    fn arms_settings_resolve_and_round_trip() {
        for id in ["auto", "classic", "slim"] {
            assert_eq!(ArmsSetting::parse(id).unwrap().id(), id);
        }
        assert_eq!(ArmsSetting::parse("wide"), None);
        assert_eq!(ArmsSetting::parse(" SLIM "), Some(ArmsSetting::Slim));
        assert_eq!(ArmsSetting::Auto.next(), ArmsSetting::Classic);
        assert_eq!(ArmsSetting::Slim.next(), ArmsSetting::Auto);
        assert!(resolve_arms(ArmsSetting::Auto, true), "auto follows the skin");
        assert!(!resolve_arms(ArmsSetting::Auto, false));
        assert!(!resolve_arms(ArmsSetting::Classic, true), "classic forces classic");
        assert!(resolve_arms(ArmsSetting::Slim, false), "slim forces slim");
        assert_eq!(ArmsSetting::Slim.label(), "SLIM");
    }

    /// Needs the internet: `cargo test --bin last_route live_ -- --ignored`. The whole FROM MC NAME chain (name to id,
    /// id to profile, textures host) over real https, then the validation: proves the download, the roots and the checks agree.
    #[test]
    #[ignore]
    fn live_from_mc_name_fetches_and_validates_a_real_skin() {
        let rules = rules().expect("rules load");
        let bytes = fetch_minecraft_skin("Notch", &rules).expect("Mojang's chain works over https");
        assert_eq!(skin_validate::check_skin_png(&bytes, max_bytes()), Ok(()), "the canonical result passes the shared check");
        assert!(verify_download(&sha256_hex(&bytes), &bytes));
    }

    #[test]
    fn a_cached_hostile_file_counts_as_not_cached_and_is_purged() {
        let (good_id, good) = good_skin();
        let (bad_id, bad) = hostile_skin();
        let dir = temp_cache("purge");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(cached_path(&dir, &good_id), &good).unwrap();
        std::fs::write(cached_path(&dir, &bad_id), &bad).unwrap();
        std::fs::write(dir.join(format!("{}.tmp", "a".repeat(64))), b"half").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not ours").unwrap();
        assert!(!fetch_needed(&dir, &good_id));
        assert!(fetch_needed(&dir, &bad_id), "a header-only file with the right hash is not a cached skin");
        assert_eq!(prune_cache(&dir, 16), 2, "the hostile file and the half-written one go");
        assert!(cached_path(&dir, &good_id).exists());
        assert!(!cached_path(&dir, &bad_id).exists());
        assert!(dir.join("notes.txt").exists(), "files that are not cache entries are never touched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cache_keeps_only_the_newest_skins() {
        let dir = temp_cache("cap");
        std::fs::create_dir_all(&dir).unwrap();
        let mut ids = Vec::new();
        for n in 0..5u8 {
            let mut px = fixture_rgba(false);
            px[px_at(1, 1, 64)] = n.wrapping_mul(40);
            let (id, bytes) = skin_from_png(encode_png(&px, 64, 64)).unwrap();
            std::fs::write(cached_path(&dir, &id), bytes).unwrap();
            let file = std::fs::File::options().write(true).open(cached_path(&dir, &id)).unwrap();
            file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000 + n as u64)).unwrap();
            ids.push(id);
        }
        assert_eq!(prune_cache(&dir, 3), 2);
        assert!(!cached_path(&dir, &ids[0]).exists() && !cached_path(&dir, &ids[1]).exists(), "the oldest two are evicted");
        assert!(ids[2..].iter().all(|id| cached_path(&dir, id).exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_skin_that_was_validated_here_has_a_known_arm_style() {
        let (id, _) = good_skin();
        assert_eq!(skin_is_slim(&id), Some(false), "the classic fixture");
        note_slim(&id, true);
        assert_eq!(skin_is_slim(&id), Some(true));
        assert_eq!(skin_is_slim(&"e".repeat(64)), None, "an unknown picture has no style");
    }
}
