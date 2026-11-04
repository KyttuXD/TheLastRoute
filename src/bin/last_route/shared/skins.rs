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
use crate::net::world_sync::{clean_skin, is_skin_hash, SKIN_DEFAULT};
use crate::shared::paths;

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

/// The bytes of a skin known by `id` from the user's own folder or the fetch cache, when they verify.
pub fn read_skin(id: &str) -> Option<Vec<u8>> {
    let file = format!("{id}.png");
    [skin_dir(), cache_dir()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(&file))
        .filter_map(|path| std::fs::read(path).ok())
        .find(|bytes| verify_skin(id, bytes))
}

/// The path of a skin id inside a cache folder: `<dir>/<id>.png`.
pub fn cached_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.png"))
}

/// True when the folder has no verified copy of skin `id`, so it has to be fetched. A cached skin is never fetched again.
pub fn fetch_needed(dir: &Path, id: &str) -> bool {
    !std::fs::read(cached_path(dir, id)).is_ok_and(|bytes| verify_skin(id, &bytes))
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
    /// The backend route, with `{id}` for the skin hash. Relative to the backend base.
    pub backend_path: String,
    /// The timeout of one skin download, in seconds.
    pub timeout_secs: u64,
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
        timeout_secs: fetch["timeout_secs"].as_u64()?,
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

/// The URL a skin is fetched from on the backend: `<backend base><backend_path>`, e.g. `GET <base>/api/skins/files/<id>.png`.
pub fn skin_backend_url(cfg: &SkinConfig, backend_base: &str, id: &str) -> String {
    format!("{}{}", backend_base.trim_end_matches('/'), cfg.backend_path.replace("{id}", id))
}

/// The URLs a skin is tried at, in order: the skins host first, then the backend route ([`skin_backend_url`]).
pub fn fetch_urls(cfg: &SkinConfig, backend_base: &str, id: &str) -> Vec<String> {
    vec![skin_host_url(cfg, id), skin_backend_url(cfg, backend_base, id)]
}

/// Tries each URL in order with `get` and returns the first body that verifies as skin `id`. A reply that fails or does
/// not verify moves on to the next URL.
fn first_verified_skin(urls: &[String], id: &str, mut get: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    urls.iter().find_map(|url| get(url).filter(|bytes| verify_skin(id, bytes)))
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

/// Sets the base URL skins are fetched from (`GET <base>/api/skins/files/<id>.png`). Only the first call counts.
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
    let id = id.to_string();
    let limit = read_limit();
    std::thread::spawn(move || {
        let urls = fetch_urls(&cfg, &base, &id);
        let fetched = first_verified_skin(&urls, &id, |url| {
            let reply = Request::new(Method::Get, url.to_string())
                .timeout(std::time::Duration::from_secs(cfg.timeout_secs))
                .limit(limit)
                .send()
                .ok()?;
            (reply.status == 200).then_some(reply.body)
        });
        if let Some(bytes) = fetched {
            let _ = write_skin_file(&dir, &id, &bytes);
        }
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
    std::thread::spawn(move || {
        if let Ok(server_id) = api::upload_skin(&cfg, &session_token, &bytes) {
            if server_id == hash {
                remember_uploaded(&hash);
            }
        }
    });
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

/// GET `url` over https with the rules' timeout and `limit` bytes. Only a 200 reply's body is returned.
pub fn fetch_https(url: &str, rules: &SkinRules, allowed_hosts: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    if let Some(why) = url_refusal(url, rules, allowed_hosts) {
        return Err(why.to_string());
    }
    let reply = Request::new(Method::Get, url.trim().to_string())
        .timeout(std::time::Duration::from_secs(rules.url_timeout_secs))
        .limit(limit)
        .send()
        .map_err(|_| "the download failed".to_string())?;
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
    validate_skin(&bytes, cap).map(|s| (s.hash, s.png))
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
    let bytes = fetch_https(url, rules, &[], read_limit())?;
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
    let lookup = fetch_https(&lookup_url, rules, &[rules.lookup_host.as_str()], read_limit())?;
    let found: serde_json::Value =
        serde_json::from_slice(&lookup).map_err(|_| "no Minecraft player was found".to_string())?;
    let uuid = found["id"].as_str().unwrap_or("");
    if uuid.len() != 32 || !uuid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("no Minecraft player was found".to_string());
    }
    let profile_url = rules.profile_url.replace("{uuid}", uuid);
    let body = fetch_https(&profile_url, rules, &[rules.profile_host.as_str()], read_limit())?;
    let profile: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "the profile could not be read".to_string())?;
    let url = profile_skin_url(&profile, rules).ok_or_else(|| "that player has no skin".to_string())?;
    let file = fetch_https(&url, rules, &[rules.textures_host.as_str()], read_limit())?;
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
            "the Godot route (game/autoload/skin_sync.gd:152), not the account JSON route"
        );
        assert!(!skin_backend_url(&cfg, "http://host:8105/", &id).contains("/api/v1/"));
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
    fn skins_host_is_tried_before_the_backend_route() {
        let id = sha256_hex(b"skin");
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105/", &id);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], skin_host_url(&cfg, &id));
        assert_eq!(urls[1], skin_backend_url(&cfg, "http://host:8105/", &id));
    }

    #[test]
    fn fetch_uses_the_host_when_it_answers_with_a_verified_skin() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            Some(bytes.clone())
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, vec![skin_host_url(&cfg, &id)], "the backend is not asked once the host answers");
    }

    #[test]
    fn a_failing_host_falls_back_to_the_backend() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            if url.starts_with(&cfg.host_base) {
                None
            } else {
                Some(bytes.clone())
            }
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, vec![skin_host_url(&cfg, &id), skin_backend_url(&cfg, "http://host:8105", &id)]);
    }

    #[test]
    fn a_host_reply_that_does_not_verify_falls_back_to_the_backend() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let got = first_verified_skin(&urls, &id, |url| {
            if url.starts_with(&cfg.host_base) {
                Some(png_head(32, 32))
            } else {
                Some(bytes.clone())
            }
        });
        assert_eq!(got, Some(bytes), "a wrong-hash or wrong-size host reply is not used");
    }

    #[test]
    fn nothing_verifies_when_both_routes_fail() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let urls = fetch_urls(&shipped_config(), "http://host:8105", &id);
        assert_eq!(first_verified_skin(&urls, &id, |_| None), None);
    }

    #[test]
    fn a_cached_skin_is_not_fetched_again() {
        let dir = std::env::temp_dir().join(format!("lr_skins_cache_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
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
        assert_eq!(c.upload_max_bytes, 16 * 1024, "the website's SKIN_MAX_BYTES (lr_server skins.rs)");
        assert_eq!(c.max_bytes, 65536, "PlayerSkin.MAX_BYTES (game/player/player_skin.gd:16), not the old 16 KB");
        assert_eq!(read_limit(), 65537, "FETCH_LIMIT_BYTES = MAX_BYTES + 1 (game/autoload/skin_sync.gd:23)");
        assert_eq!(c.host_base, "https://skins.kyttuslab.org");
        assert_eq!(c.host_path, "/{id}.png");
        assert_eq!(c.backend_path, "/api/skins/files/{id}.png", "game/autoload/skin_sync.gd:152");
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
        // Pure noise cannot compress: it is refused with a message, never uploaded over the cap.
        let mut state = 12345u32;
        let noise: Vec<u8> = (0..64 * 64 * 4)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let err = skin_from_png(encode_png(&noise, 64, 64)).unwrap_err();
        assert!(err.contains("too detailed"), "{err}");
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
}
