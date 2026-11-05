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
pub use crate::shared::skin_png::SKIN_SIZE;
use crate::shared::skin_png::{self, Limits, Shape, PNG_SIGNATURE};

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

/// Reads a regular file of at most `cap` bytes. A file that is not a regular file (a FIFO or a device would hang the
/// open) or is over the cap is an error, and a big file is never loaded whole: at most `cap + 1` bytes are read
/// (Godot `PlayerSkin.read_file`, which reads at most MAX_BYTES + 1).
pub fn read_capped(path: &Path, cap: usize) -> std::io::Result<Vec<u8>> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > cap as u64 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the file is too big"));
    }
    let mut out = Vec::new();
    File::open(path)?.take(cap as u64 + 1).read_to_end(&mut out)?;
    if out.len() > cap {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the file is too big"));
    }
    Ok(out)
}

/// Reads the first 24 bytes of a regular file and checks them with [`png_is_skin_size`].
fn file_is_skin(path: &Path) -> bool {
    let mut head = [0u8; 24];
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= max_bytes() as u64) {
        return false;
    }
    match File::open(path).and_then(|mut f| f.read_exact(&mut head)) {
        Ok(()) => png_is_skin_size(&head),
        Err(_) => false,
    }
}

/// One skin file in the skins folder: its list id (unique), the file name as the player typed it, and its path.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinFile {
    pub id: String,
    /// The file name without `.png`, as on disk (shown in the picker).
    pub name: String,
    pub path: PathBuf,
}

/// The longest list id (`world_sync::clean_skin` cuts a skin id to 24 characters).
const ID_MAX_CHARS: usize = 24;

/// The skin files of a folder in name order, each with a list id that no other file has: the cleaned stem (a-b.png and
/// ab.png both clean to AB), with `_2`, `_3`, ... on the later ones. DEFAULT is never an id. Files that are not 64x64 PNGs,
/// are not regular files, or are links that leave the folder are skipped.
pub fn list_skin_files(dir: &Path) -> Vec<SkinFile> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let root = dir.canonicalize().ok();
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter(|p| match (&root, p.canonicalize()) {
            (Some(root), Ok(real)) => real.starts_with(root),
            _ => false,
        })
        .filter(|p| file_is_skin(p))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(|s| (s.to_string(), p.clone())))
        .collect();
    files.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()).then_with(|| a.0.cmp(&b.0)));
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    taken.insert(SKIN_DEFAULT.to_string());
    let mut out = Vec::new();
    for (name, path) in files {
        let base = clean_skin(&name);
        if base.is_empty() {
            continue;
        }
        let mut id = base.clone();
        let mut n = 2;
        while taken.contains(&id) {
            let suffix = format!("_{n}");
            let keep: String = base.chars().take(ID_MAX_CHARS.saturating_sub(suffix.len())).collect();
            id = format!("{keep}{suffix}");
            n += 1;
        }
        taken.insert(id.clone());
        out.push(SkinFile { id, name, path });
    }
    out
}

/// Skin ids for a folder: DEFAULT first, then the valid `.png` files in name order, each with its own id.
/// A missing folder gives just DEFAULT. Files that are not 64x64 PNGs are skipped.
pub fn list_skins(dir: Option<&Path>) -> Vec<String> {
    let mut ids = vec![SKIN_DEFAULT.to_string()];
    if let Some(dir) = dir {
        ids.extend(list_skin_files(dir).into_iter().map(|f| f.id));
    }
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
    worn_skin(saved, false, ArmsSetting::Auto).hash
}

/// The skin the local runner wears, resolved once: the wire id and whether the arms are slim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worn {
    /// The SHA-256 of the canonical PNG, or DEFAULT when the saved skin is the default or its file is gone.
    pub hash: String,
    /// True when the model should draw slim arms (the arm setting applied to the layout read when the skin was applied).
    pub slim: bool,
}

/// Resolves the saved skin id (a list id) and arm style into what the first-person hands, the preview and the room use.
/// `detected_slim` is the layout read when the skin was applied (`Settings::skin_slim`): the canonical PNG has its base
/// layer made opaque, so the layout cannot be read from it again. The file is kept in the fetch cache under its hash,
/// where the texture cache finds it by that hash, the same way it finds every other runner's skin.
pub fn worn_skin(saved: &str, detected_slim: bool, arms: ArmsSetting) -> Worn {
    worn_skin_in(skin_dir().as_deref(), cache_dir().as_deref(), saved, detected_slim, arms)
}

/// [`worn_skin`] with the skins folder and the cache folder given.
pub fn worn_skin_in(skins: Option<&Path>, cache: Option<&Path>, saved: &str, detected_slim: bool, arms: ArmsSetting) -> Worn {
    let id = resolve_skin(saved, &list_skins(skins));
    match skins.and_then(|dir| load_skin_in(dir, &id)) {
        Some(valid) => {
            if let Some(dir) = cache {
                if fetch_needed(dir, &valid.hash) {
                    let _ = write_skin_file(dir, &valid.hash, &valid.png);
                }
            }
            Worn { hash: valid.hash, slim: arms.resolve(detected_slim) }
        }
        None => Worn { hash: SKIN_DEFAULT.to_string(), slim: arms.resolve(false) },
    }
}

/// The skin file a list id names in a skins folder, when it is a valid skin.
fn skin_file_in(dir: &Path, id: &str) -> Option<PathBuf> {
    list_skin_files(dir).into_iter().find(|f| f.id == id).map(|f| f.path)
}

/// The validated skin a list id names in the skins folder: read with the size cap, checked and canonicalised. DEFAULT,
/// unknown ids and files that fail the check give None.
pub fn load_skin(id: &str) -> Option<ValidSkin> {
    load_skin_in(&skin_dir()?, id)
}

/// [`load_skin`] for a given skins folder.
pub fn load_skin_in(dir: &Path, id: &str) -> Option<ValidSkin> {
    if id == SKIN_DEFAULT {
        return None;
    }
    let bytes = read_capped(&skin_file_in(dir, id)?, max_bytes()).ok()?;
    validate_skin(&bytes, max_bytes()).ok()
}

/// The wire id and bytes of the skin a list id names. The id is the SHA-256 of the PNG. DEFAULT, unknown ids and
/// files over the size cap give None.
pub fn skin_png(id: &str) -> Option<(String, Vec<u8>)> {
    // The canonical re-encoding is what is hashed, cached and uploaded, so every client agrees on the id.
    load_skin(id).map(|v| (v.hash, v.png))
}

/// The lowercase hex SHA-256 of `bytes`: the id a skin PNG is known by (64 hex characters).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The limits of the strict PNG check, from data/skins/config.json (zero when the config is missing: everything is refused).
pub fn png_limits() -> Limits {
    Limits { max_bytes: max_bytes(), max_chunks: config().map_or(0, |c| c.png_max_chunks) }
}

/// True when `bytes` is a skin PNG whose SHA-256 is `id`: the hash matches and the whole file passes the strict check
/// (every chunk checksum, a 64x64 header, a legal format, pixel data of exactly the promised size). Both the disk
/// cache and fetched skins are checked this way, and only a skin that passes is ever decoded.
pub fn verify_skin(id: &str, bytes: &[u8]) -> bool {
    sha256_hex(bytes) == id && skin_png::check_png(bytes, png_limits(), Shape::Remote).is_ok()
}

/// The 64x64 RGBA pixels a verified remote skin is drawn with: decoded through the fallible decoder, with the base layer
/// made opaque (only the second layer may be transparent), exactly as the Godot game repairs a downloaded skin
/// (`PlayerSkin.validate`). An `Err` means the file must not be used.
pub fn remote_skin_rgba(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let layout = layout().ok_or_else(|| "the skin rules are missing".to_string())?;
    let (w, h, mut px) = skin_png::decode_rgba(bytes, png_limits(), Shape::Remote)?;
    if w != SKIN_SIZE || h != SKIN_SIZE {
        return Err("a skin must be 64x64 pixels".to_string());
    }
    force_opaque_base(&layout, &mut px);
    Ok(px)
}

/// Makes every pixel outside the second-layer rectangles opaque.
fn force_opaque_base(layout: &SkinLayout, px: &mut [u8]) {
    for y in 0..SKIN_SIZE {
        for x in 0..SKIN_SIZE {
            if !is_overlay_pixel(layout, x, y) {
                px[((y * SKIN_SIZE + x) * 4 + 3) as usize] = 255;
            }
        }
    }
}

/// The folder skins fetched from the website are cached in (`<user dir>/skins-cache`).
pub fn cache_dir() -> Option<PathBuf> {
    paths::user_dir().map(|d| d.join("skins-cache"))
}

/// The bytes of a skin known by `id` from the user's own folder or the fetch cache, when they verify. Files are read
/// with the size cap, so a huge or special file there is never loaded.
pub fn read_skin(id: &str) -> Option<Vec<u8>> {
    if !is_skin_hash(id) {
        return None;
    }
    let file = format!("{id}.png");
    [skin_dir(), cache_dir()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(&file))
        .filter_map(|path| read_capped(&path, max_bytes()).ok())
        .find(|bytes| verify_skin(id, bytes))
}

/// The path of a skin id inside a cache folder: `<dir>/<id>.png`.
pub fn cached_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.png"))
}

/// True when the folder has no verified copy of skin `id`, so it has to be fetched. A cached skin is never fetched again;
/// a copy that no longer passes the check (damaged, or cached by an older build that checked less) is fetched again.
pub fn fetch_needed(dir: &Path, id: &str) -> bool {
    !read_capped(&cached_path(dir, id), max_bytes()).is_ok_and(|bytes| verify_skin(id, &bytes))
}

/// The skin fetch settings from data/skins/config.json. Standard values match the Godot game: the size cap is
/// PlayerSkin.MAX_BYTES (game/player/player_skin.gd:16), and the backend route is `/api/skins/files/<hash>.png`
/// (game/autoload/skin_sync.gd:152).
#[derive(Clone, Debug, PartialEq)]
pub struct SkinConfig {
    /// The largest skin file accepted, in bytes.
    pub max_bytes: usize,
    /// The largest canonical PNG the website stores, in bytes (the server's SKIN_MAX_BYTES, 64 KB like Godot's skins.mjs).
    /// A skin whose canonical re-encoding is bigger is refused before upload.
    pub upload_max_bytes: usize,
    /// The most chunks a skin PNG may hold (the strict check reads every chunk, so this bounds its work).
    pub png_max_chunks: usize,
    /// Seconds before a failed or unanswered upload is tried again (Godot skin_sync.gd RETRY_SECONDS).
    pub upload_retry_secs: f64,
    /// The most skins of other runners kept (on the GPU and in the disk cache; Godot PEER_CACHE_MAX, player_skin.gd).
    pub peer_cache_max: usize,
    /// The skins host base, tried first when fetching (no trailing slash).
    pub host_base: String,
    /// The path on the skins host, with `{id}` for the skin hash.
    pub host_path: String,
    /// The Godot backend route, with `{id}` for the skin hash. Relative to the backend base.
    pub backend_path: String,
    /// The lr_server route (`GET /api/v1/skins/<id>`), with `{id}` for the skin hash. Relative to the backend base.
    pub v1_path: String,
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
        png_max_chunks: v["png"]["max_chunks"].as_u64()? as usize,
        upload_retry_secs: v["upload_retry_secs"].as_f64()?,
        peer_cache_max: (v["peer_cache_max"].as_u64()? as usize).max(1),
        host_base: fetch["host_base"].as_str()?.trim_end_matches('/').to_string(),
        host_path: fetch["host_path"].as_str()?.to_string(),
        backend_path: fetch["backend_path"].as_str()?.to_string(),
        v1_path: fetch["v1_path"].as_str()?.to_string(),
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

/// The URL a skin is fetched from on lr_server: `GET <backend base>/api/v1/skins/<id>`.
pub fn skin_v1_url(cfg: &SkinConfig, backend_base: &str, id: &str) -> String {
    format!("{}{}", backend_base.trim_end_matches('/'), cfg.v1_path.replace("{id}", id))
}

/// The URLs a skin is tried at, in order: lr_server's own route (plain http to the backend the game talks to), then the
/// skins host, then the Godot backend route ([`skin_backend_url`]).
pub fn fetch_urls(cfg: &SkinConfig, backend_base: &str, id: &str) -> Vec<String> {
    vec![skin_v1_url(cfg, backend_base, id), skin_host_url(cfg, id), skin_backend_url(cfg, backend_base, id)]
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

/// How many skins of other runners are kept (the config's `peer_cache_max`).
pub fn peer_cache_max() -> usize {
    config().map_or(16, |c| c.peer_cache_max)
}

/// Keeps the `keep` newest skin files of the cache folder and deletes the rest (a runner's skin is fetched again when it
/// is next seen). Only `<64 hex>.png` files are touched.
pub fn prune_cache(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let stem = path.file_stem()?.to_str()?.to_string();
            let is_png = path.extension().is_some_and(|x| x == "png");
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            (is_png && is_skin_hash(&stem)).then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), path))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
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
            prune_cache(&dir, cfg.peer_cache_max);
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

/// When the worn skin is next offered to the website. A new skin is offered at once; one the website has not taken
/// (not signed in yet, a failed upload) is offered again after the retry delay (Godot skin_sync.gd RETRY_SECONDS).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UploadPlan {
    hash: String,
    next_at: f64,
}

impl UploadPlan {
    /// True when `hash` should be uploaded now: signed in, a real skin hash the website does not hold yet, and either
    /// a hash not offered before or past the retry time. Answering true starts the retry clock.
    /// `uploaded` lists the ids the website holds; it is only asked for when an upload could be due, because it reads a file.
    pub fn due(&mut self, now: f64, hash: &str, signed_in: bool, uploaded: impl FnOnce() -> Vec<String>, retry_secs: f64) -> bool {
        if self.hash != hash {
            self.hash = hash.to_string();
            self.next_at = 0.0;
        }
        if !signed_in || !is_skin_hash(hash) || now < self.next_at {
            return false;
        }
        self.next_at = now + retry_secs;
        !holds(&uploaded(), hash)
    }
}

/// Resolves the worn skin ([`worn_skin`]) once per change instead of once per frame. The key is what the result depends
/// on: the saved list id, the layout read at APPLY, the ARMS setting, and [`generation`] (an APPLY rewrites custom.png
/// under the same list id).
#[derive(Debug, Default)]
pub struct WornTracker {
    key: Option<(String, bool, String, u64)>,
    worn: Option<Worn>,
}

impl WornTracker {
    /// Makes sure the worn skin matches the settings; `resolve` runs only when the key changed. True when the worn hash or
    /// arm style differs from before (the first call is always a change).
    pub fn update(&mut self, skin: &str, skin_slim: bool, arms: &str, generation: u64, resolve: impl FnOnce() -> Worn) -> bool {
        let key = (skin.to_string(), skin_slim, arms.to_string(), generation);
        if self.key.as_ref() == Some(&key) {
            return false;
        }
        self.key = Some(key);
        let next = resolve();
        let changed = self.worn.as_ref() != Some(&next);
        self.worn = Some(next);
        changed
    }

    /// The worn skin; DEFAULT and classic until the first [`WornTracker::update`].
    pub fn get(&self) -> Worn {
        self.worn.clone().unwrap_or(Worn { hash: SKIN_DEFAULT.to_string(), slim: false })
    }
}

/// True while an upload thread runs, so a slow upload is never doubled.
static UPLOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Uploads the skin known by `hash` (its verified copy in the skins folder or the cache) to the website in the
/// background, when signed in (`session_token` set) and not offline. Returns true when an upload was started; a skin the
/// website already has (see [`uploaded_ids`]) is not sent again. A failure is dropped here: the caller's [`UploadPlan`]
/// offers the skin again after its retry delay.
pub fn upload_in_background(cfg: BackendConfig, session_token: String, hash: String) -> bool {
    use std::sync::atomic::Ordering;
    if session_token.is_empty() || cfg.offline || !is_skin_hash(&hash) || holds(&uploaded_ids(), &hash) {
        return false;
    }
    let Some(bytes) = read_skin(&hash) else { return false };
    if UPLOADING.swap(true, Ordering::SeqCst) {
        return false;
    }
    std::thread::spawn(move || {
        match api::upload_skin(&cfg, &session_token, &bytes) {
            Ok(server_id) if server_id == hash => remember_uploaded(&hash),
            Ok(_) => eprintln!("skins: the website stored a different id for {hash}"),
            Err(e) => eprintln!("skins: upload of {hash} failed: {e:?}"),
        }
        UPLOADING.store(false, Ordering::SeqCst);
    });
    true
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
    /// Seconds a key is held in a SKIN text box before it repeats.
    pub repeat_delay_secs: f64,
    /// How many times a second a held key repeats once it has started.
    pub repeat_per_sec: f64,
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
        repeat_delay_secs: v["input"]["repeat_delay_secs"].as_f64()?,
        repeat_per_sec: v["input"]["repeat_per_sec"].as_f64()?,
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

/// What the SKIN tab says when a download fails: the real reason, not a blanket "the download failed".
pub fn fetch_error_text(e: &crate::net::NetError) -> String {
    use crate::net::NetError;
    match e {
        NetError::Unsupported(_) => "this build cannot make https connections yet".to_string(),
        NetError::Timeout => "the server did not answer in time".to_string(),
        NetError::Offline => "the game is offline".to_string(),
        _ => "the download failed".to_string(),
    }
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
        .map_err(|e| fetch_error_text(&e))?;
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
    /// True when the file was the old 64x32 format, upgraded to 64x64 (such a skin has only classic arms).
    pub legacy: bool,
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
/// header dimensions (64x64 or the old 64x32, checked before decoding), the strict structure check (every chunk checksum,
/// a legal format, pixel data of exactly the promised size), a real decode through the fallible decoder, the old format
/// upgraded to 64x64, the slim layout read, the base layer forced opaque, and a canonical re-encoding that is hashed.
pub fn validate_skin(bytes: &[u8], cap: usize) -> Result<ValidSkin, String> {
    validate_skin_with(bytes, cap, upload_max_bytes())
}

/// [`validate_skin`] with the website's size cap for the canonical PNG given too.
pub fn validate_skin_with(bytes: &[u8], cap: usize, upload_cap: usize) -> Result<ValidSkin, String> {
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
    let limits = Limits { max_bytes: cap, ..png_limits() };
    let (w, h, decoded) = skin_png::decode_rgba(bytes, limits, Shape::Local)?;
    let legacy = w == SKIN_SIZE && h == SKIN_SIZE / 2;
    let mut px = vec![0u8; (SKIN_SIZE * SKIN_SIZE * 4) as usize];
    px[..decoded.len()].copy_from_slice(&decoded);
    if legacy {
        for (from, to) in &layout.legacy_mirror {
            mirror_box(&mut px, *from, *to);
        }
    }
    let slim = !legacy && slim_with(&layout, &px, SKIN_SIZE, SKIN_SIZE);
    force_opaque_base(&layout, &mut px);
    let png = encode_png(&px, SKIN_SIZE, SKIN_SIZE);
    // The canonical bytes are what is uploaded and hashed, and the website refuses more than its cap.
    if png.len() > upload_cap {
        return Err(format!("the skin is too detailed to upload (over {} KB once saved)", upload_cap / 1024));
    }
    Ok(ValidSkin { hash: sha256_hex(&png), png, slim, rgba: px, legacy })
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

use crate::shared::skin_png::crc32;

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

/// FROM URL: a 64x64 PNG downloaded from an https link. Returns the validated skin (canonical bytes and arm layout).
pub fn fetch_url_skin(url: &str, rules: &SkinRules) -> Result<ValidSkin, String> {
    let bytes = fetch_https(url, rules, &[], read_limit())?;
    validate_skin(&bytes, max_bytes())
}

/// True when `name` is a Minecraft username by the rules: 1 to `name_max` characters, all from `name_chars`.
pub fn valid_minecraft_name(name: &str, rules: &SkinRules) -> bool {
    !name.is_empty() && name.len() <= rules.name_max && name.chars().all(|c| rules.name_chars.contains(c))
}

/// The skin URL in a Mojang profile: the `textures` property (base64 JSON) with a SKIN url on the textures host.
pub fn profile_skin_url(profile: &serde_json::Value, rules: &SkinRules) -> Option<String> {
    profile_skin(profile, rules).map(|(url, _)| url)
}

/// The skin URL in a Mojang profile and whether the profile says the model is slim (Alex): the SKIN texture's
/// `metadata.model` (Godot `PlayerSkin` reads the same field and lets it win over the pixel detection).
pub fn profile_skin(profile: &serde_json::Value, rules: &SkinRules) -> Option<(String, bool)> {
    let props = profile["properties"].as_array()?;
    let prop = props.iter().find(|p| p["name"].as_str() == Some("textures"))?;
    let raw = crate::content::embedded::base64_decode(prop["value"].as_str()?);
    let decoded: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let skin = &decoded["textures"]["SKIN"];
    let url = skin["url"].as_str()?;
    let slim = skin["metadata"]["model"].as_str() == Some("slim");
    url_refusal(url, rules, &[rules.textures_host.as_str()]).is_none().then(|| (url.to_string(), slim))
}

/// FROM MC NAME: the name's skin through Mojang's APIs (name to uuid, uuid to profile, then the textures host). Returns
/// the validated skin; the profile's slim model wins over the pixel detection, except for an old 64x32 skin, which has
/// only classic arms.
pub fn fetch_minecraft_skin(name: &str, rules: &SkinRules) -> Result<ValidSkin, String> {
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
    let (url, slim_model) = profile_skin(&profile, rules).ok_or_else(|| "that player has no skin".to_string())?;
    let file = fetch_https(&url, rules, &[rules.textures_host.as_str()], read_limit())?;
    let mut valid = validate_skin(&file, max_bytes())?;
    valid.slim = (valid.slim || slim_model) && !valid.legacy;
    Ok(valid)
}

/// The list id a picked skin is saved under: `custom.png` in the skins folder.
pub const CUSTOM_STEM: &str = "custom";

/// Counts the skins saved with [`save_custom_skin`], so a caller that resolved the worn skin once knows to resolve it
/// again after an APPLY even though the list id (always CUSTOM) did not change.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The number of skins saved as the user's own so far this run.
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// Saves validated skin bytes as the user's own skin (`<skins folder>/custom.png`), replacing it in one step. Returns
/// the list id ([`clean_skin`] of the stem).
pub fn save_custom_skin(bytes: &[u8]) -> std::io::Result<String> {
    let dir = skin_dir().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no user folder"))?;
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{CUSTOM_STEM}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(format!("{CUSTOM_STEM}.png")))?;
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(clean_skin(CUSTOM_STEM))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_id_is_the_sha256_of_the_png_bytes() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let bytes = fixture_png(false);
        let id = sha256_hex(&bytes);
        assert_eq!(id.len(), 64);
        assert!(verify_skin(&id, &bytes));
        assert!(!verify_skin(&id, &png_head(32, 32)), "a wrong-size PNG never verifies");
        assert!(!verify_skin(&sha256_hex(b"other"), &bytes), "bytes must hash to the id");
        let header_only = png_head(64, 64);
        assert!(!verify_skin(&sha256_hex(&header_only), &header_only), "a header with the right hash is still no skin");
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
    fn the_servers_route_is_tried_first_then_the_host_then_the_legacy_route() {
        let id = sha256_hex(b"skin");
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105/", &id);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], skin_v1_url(&cfg, "http://host:8105/", &id), "lr_server's route first: it is plain http");
        assert_eq!(urls[1], skin_host_url(&cfg, &id));
        assert_eq!(urls[2], skin_backend_url(&cfg, "http://host:8105/", &id));
        assert_eq!(urls[0], format!("http://host:8105/api/v1/skins/{id}"));
    }

    #[test]
    fn fetch_stops_at_the_first_route_that_answers_with_a_verified_skin() {
        let bytes = fixture_png(false);
        let id = sha256_hex(&bytes);
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            Some(bytes.clone())
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, vec![skin_v1_url(&cfg, "http://host:8105", &id)], "nothing else is asked once a route answers");
    }

    #[test]
    fn a_failing_route_falls_through_to_the_next() {
        let bytes = fixture_png(false);
        let id = sha256_hex(&bytes);
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            if url.starts_with(&cfg.host_base) || url.contains("/api/v1/") {
                None
            } else {
                Some(bytes.clone())
            }
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, fetch_urls(&cfg, "http://host:8105", &id));
    }

    #[test]
    fn a_reply_that_does_not_verify_falls_through_to_the_next_route() {
        let bytes = fixture_png(false);
        let id = sha256_hex(&bytes);
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105", &id);
        let got = first_verified_skin(&urls, &id, |url| {
            if url.starts_with(&cfg.host_base) || url.contains("/api/v1/") {
                Some(png_head(64, 64))
            } else {
                Some(bytes.clone())
            }
        });
        assert_eq!(got, Some(bytes), "a header-only or wrong-hash reply is not used");
    }

    #[test]
    fn nothing_verifies_when_every_route_fails() {
        let bytes = fixture_png(false);
        let id = sha256_hex(&bytes);
        let urls = fetch_urls(&shipped_config(), "http://host:8105", &id);
        assert_eq!(first_verified_skin(&urls, &id, |_| None), None);
    }

    #[test]
    fn a_cached_skin_is_not_fetched_again() {
        let dir = std::env::temp_dir().join(format!("lr_skins_cache_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bytes = fixture_png(false);
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
        assert!(
            skin_from_png(png_head(64, 64)).unwrap_err().starts_with("the PNG could not be read"),
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
        assert_eq!(c.upload_max_bytes, 65536, "the website's SKIN_MAX_BYTES, 64 KB like Godot's skins.mjs:16");
        assert_eq!(c.png_max_chunks, 64);
        assert_eq!(c.upload_retry_secs, 60.0, "RETRY_SECONDS (game/autoload/skin_sync.gd:18)");
        assert_eq!(c.peer_cache_max, 16, "PEER_CACHE_MAX (game/player/player_skin.gd)");
        assert_eq!(c.v1_path, "/api/v1/skins/{id}", "lr_server routes.rs");
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
        // Pure noise cannot compress: it is 16 KB of pixels, and a 16 KB website cap would refuse it with a message.
        let mut state = 12345u32;
        let noise: Vec<u8> = (0..64 * 64 * 4)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let noisy = encode_png(&noise, 64, 64);
        let err = validate_skin_with(&noisy, 65536, 16 * 1024).unwrap_err();
        assert!(err.contains("too detailed"), "{err}");
        assert!(skin_from_png(noisy).is_ok(), "a detailed skin is fine under the 64 KB cap Godot uses");
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
        assert!(validate_skin(&broken, 65536).unwrap_err().starts_with("the PNG could not be read"), "pixels must really decode");
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

    // ---- hostile skins, bounded reads, collision-free ids, the worn skin, the catalogue ----

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_skins_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_peer_cache_keeps_only_the_newest_skins_and_only_skin_files() {
        let dir = temp_dir("prune");
        let ids: Vec<String> = (0..5).map(|i| format!("{:064x}", i + 1)).collect();
        for (i, id) in ids.iter().enumerate() {
            let path = dir.join(format!("{id}.png"));
            std::fs::write(&path, b"x").unwrap();
            let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i as u64);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
        std::fs::write(dir.join("notes.png"), b"x").unwrap();
        prune_cache(&dir, 2);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(dir.join(format!("{id}.png")).exists(), i >= 3, "skin {i}: only the two newest stay");
        }
        assert!(dir.join("notes.png").exists(), "a file that is not a skin hash is never touched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A PNG with the base layer fully transparent: every pixel clear.
    fn transparent_png() -> Vec<u8> {
        encode_png(&vec![0u8; 64 * 64 * 4], 64, 64)
    }

    #[test]
    fn a_header_only_skin_never_verifies_and_never_reaches_a_decoder() {
        let head = png_head(64, 64);
        let id = sha256_hex(&head);
        assert!(!verify_skin(&id, &head));
        let dir = temp_dir("hostile_cache");
        write_skin_file(&dir, &id, &head).unwrap();
        assert!(fetch_needed(&dir, &id), "a hostile cached file is fetched again, not trusted");
        assert!(remote_skin_rgba(&head).is_err(), "the texture path refuses it with an Err, not a panic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_hostile_remote_shape_is_an_error_not_a_panic() {
        let good = fixture_png(false);
        let mut bad_crc = good.clone();
        bad_crc[40] ^= 1;
        let mut truncated_idat = good.clone();
        truncated_idat.truncate(good.len() / 2);
        let wrong_size = encode_png(&vec![255u8; 32 * 32 * 4], 32, 32);
        let old_shape = encode_png(&fixture_rgba(false)[..64 * 32 * 4], 64, 32);
        for (what, bytes) in [
            ("bad crc", bad_crc),
            ("truncated", truncated_idat),
            ("32x32", wrong_size),
            ("64x32 is not stored", old_shape),
            ("header only", png_head(64, 64)),
            ("not a png", b"GIF89a not a png at all".to_vec()),
            ("empty", Vec::new()),
        ] {
            assert!(remote_skin_rgba(&bytes).is_err(), "{what}");
            assert!(!verify_skin(&sha256_hex(&bytes), &bytes), "{what}");
        }
        assert!(remote_skin_rgba(&good).is_ok());
    }

    #[test]
    fn a_transparent_remote_base_is_repaired_to_opaque_like_godot() {
        let px = remote_skin_rgba(&transparent_png()).expect("the structure is fine, the base is repaired");
        assert_eq!(px.len(), 64 * 64 * 4);
        let layout = shipped_layout();
        for y in 0..64 {
            for x in 0..64 {
                let alpha = px[px_at(x, y, 64) + 3];
                if is_overlay_pixel(&layout, x, y) {
                    assert_eq!(alpha, 0, "overlay pixel {x},{y} may stay clear");
                } else {
                    assert_eq!(alpha, 255, "base pixel {x},{y} is forced opaque, so the runner is never invisible");
                }
            }
        }
    }

    #[test]
    fn a_noisy_skin_under_64_kb_is_accepted_with_its_real_size() {
        let mut state = 7u32;
        let noise: Vec<u8> = (0..64 * 64 * 4)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let png = encode_png(&noise, 64, 64);
        assert!(png.len() > 16 * 1024 && png.len() <= 64 * 1024, "{} bytes", png.len());
        let (hash, canonical) = skin_from_png(png).expect("a detailed skin is valid (Godot MAX_BYTES 64 KB)");
        assert!(verify_skin(&hash, &canonical), "and the canonical bytes pass the strict remote check");
    }

    #[test]
    fn reads_never_load_more_than_the_cap() {
        let dir = temp_dir("capped_reads");
        let big = dir.join("huge.png");
        let mut head = png_head(64, 64);
        head.resize(max_bytes() + 1, 0);
        std::fs::write(&big, &head).unwrap();
        assert!(read_capped(&big, max_bytes()).is_err(), "one byte over the cap is refused");
        assert!(!file_is_skin(&big), "a file over the cap is not listed as a skin");
        assert!(list_skins(Some(&dir)).len() == 1, "only DEFAULT");
        let ok = dir.join("ok.png");
        std::fs::write(&ok, fixture_png(false)).unwrap();
        assert_eq!(read_capped(&ok, max_bytes()).unwrap(), fixture_png(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_named_png_is_ignored_and_never_opened() {
        let dir = temp_dir("fifo");
        let fifo = dir.join("x.png");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().map(|s| s.success()).unwrap_or(false);
        if made {
            assert_eq!(list_skins(Some(&dir)), vec![SKIN_DEFAULT.to_string()], "a FIFO is not a skin and does not hang the listing");
            assert!(read_capped(&fifo, max_bytes()).is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_leaves_the_folder_is_not_listed() {
        let dir = temp_dir("links");
        let outside = temp_dir("links_outside");
        std::fs::write(outside.join("secret.png"), fixture_png(false)).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.png"), dir.join("linked.png")).unwrap();
        std::fs::write(dir.join("mine.png"), fixture_png(false)).unwrap();
        assert_eq!(list_skins(Some(&dir)), vec!["DEFAULT".to_string(), "MINE".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn files_with_the_same_cleaned_id_stay_distinct_and_resolve_by_exact_file() {
        let dir = temp_dir("ids");
        let a = fixture_png(false);
        let b = fixture_png(true);
        std::fs::write(dir.join("a-b.png"), &a).unwrap();
        std::fs::write(dir.join("ab.png"), &b).unwrap();
        let files = list_skin_files(&dir);
        let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["AB", "AB_2"], "the later file gets a suffix, in name order, not directory order");
        assert_eq!(files[0].name, "a-b");
        assert_eq!(files[1].name, "ab");
        assert_eq!(load_skin_in(&dir, "AB").unwrap().hash, skin_from_png(a).unwrap().0, "AB is a-b.png");
        assert_eq!(load_skin_in(&dir, "AB_2").unwrap().hash, skin_from_png(b).unwrap().0, "AB_2 is ab.png");
        let long = "x".repeat(40);
        std::fs::write(dir.join(format!("{long}1.png")), fixture_png(false)).unwrap();
        std::fs::write(dir.join(format!("{long}2.png")), fixture_png(false)).unwrap();
        for f in list_skin_files(&dir) {
            assert!(f.id.chars().count() <= 24, "{} fits an id", f.id);
        }
        assert_eq!(list_skins(Some(&dir)).len(), 1 + 4, "all four are listed with distinct ids");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_worn_skin_resolves_hash_and_slim_once_and_follows_the_applied_file() {
        let skins = temp_dir("worn_skins");
        let cache = temp_dir("worn_cache");
        let first = fixture_png(true);
        std::fs::write(skins.join("custom.png"), &first).unwrap();
        let w = worn_skin_in(Some(&skins), Some(&cache), "CUSTOM", true, ArmsSetting::Auto);
        let (hash, canonical) = skin_from_png(first).unwrap();
        assert_eq!(w.hash, hash, "the worn id is the canonical hash, not the list id");
        assert!(w.slim, "AUTO follows the layout read when the skin was applied");
        assert_eq!(read_capped(&cached_path(&cache, &hash), max_bytes()).unwrap(), canonical, "the file is cached under its hash");
        assert!(worn_skin_in(Some(&skins), Some(&cache), "CUSTOM", true, ArmsSetting::Classic).slim == false, "CLASSIC forces classic");
        assert!(worn_skin_in(Some(&skins), Some(&cache), "CUSTOM", false, ArmsSetting::Slim).slim, "SLIM forces slim");
        // a second picture under the same list id is a different worn skin
        let mut other = fixture_rgba(false);
        other[px_at(10, 10, 64)] ^= 0xff; // a visible pixel of the head
        std::fs::write(skins.join("custom.png"), encode_png(&other, 64, 64)).unwrap();
        let w2 = worn_skin_in(Some(&skins), Some(&cache), "CUSTOM", false, ArmsSetting::Auto);
        assert_ne!(w2.hash, w.hash, "applying a second skin changes the hash even though the id stayed CUSTOM");
        assert!(!w2.slim);
        let gone = worn_skin_in(Some(&skins), Some(&cache), "MISSING", true, ArmsSetting::Auto);
        assert_eq!(gone.hash, SKIN_DEFAULT, "an unknown or deleted skin falls back to the default");
        assert_eq!(worn_skin_in(None, None, "CUSTOM", false, ArmsSetting::Auto).hash, SKIN_DEFAULT);
        let _ = std::fs::remove_dir_all(&skins);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_new_hash_uploads_at_once_and_a_failed_one_is_retried_after_the_delay() {
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let mut plan = UploadPlan::default();
        assert!(!plan.due(0.0, &a, false, Vec::new, 60.0), "not signed in: nothing is sent");
        assert!(plan.due(1.0, &a, true, Vec::new, 60.0), "signed in: the new skin goes at once");
        assert!(!plan.due(30.0, &a, true, Vec::new, 60.0), "not again before the retry delay");
        assert!(plan.due(61.5, &a, true, Vec::new, 60.0), "a skin the website still lacks is offered again");
        assert!(!plan.due(200.0, &a, true, || vec![a.clone()], 60.0), "a skin the website holds is never sent again");
        assert!(plan.due(201.0, &b, true, || vec![a.clone()], 60.0), "a second skin in the same session uploads too");
        assert!(!plan.due(202.0, SKIN_DEFAULT, true, Vec::new, 60.0), "the default skin is never uploaded");
    }

    #[test]
    fn the_worn_skin_is_resolved_once_per_change_not_per_frame() {
        let mut t = WornTracker::default();
        let mut calls = 0;
        let mut worn = |hash: &str, slim: bool| {
            calls += 1;
            Worn { hash: hash.into(), slim }
        };
        assert!(t.update("CUSTOM", true, "auto", 1, || worn("h1", true)), "the first call is a change");
        assert_eq!(t.get(), Worn { hash: "h1".into(), slim: true });
        for _ in 0..100 {
            assert!(!t.update("CUSTOM", true, "auto", 1, || panic!("resolved again with nothing changed")));
        }
        assert!(t.update("CUSTOM", true, "auto", 2, || worn("h2", true)), "an APPLY (a new generation) resolves again, same list id");
        assert!(t.update("CUSTOM", true, "classic", 2, || worn("h2", false)), "ARMS changes the arm style");
        assert!(!t.update("CUSTOM", false, "classic", 2, || worn("h2", false)), "nothing visible changed: not reported as a change");
        assert_eq!(calls, 4);
    }

    #[test]
    fn download_errors_say_what_went_wrong() {
        use crate::net::NetError;
        assert!(fetch_error_text(&NetError::Unsupported("x".into())).contains("https"));
        assert!(fetch_error_text(&NetError::Timeout).contains("in time"));
        assert_eq!(fetch_error_text(&NetError::Transport("x".into())), "the download failed");
    }

    #[test]
    fn every_catalogue_picture_is_in_the_build_and_is_a_valid_skin() {
        let list = catalog();
        assert!(list.len() >= 2);
        for skin in &list {
            let png = catalog_png(skin).unwrap_or_else(|| panic!("{} has no picture in the build", skin.image));
            let valid = validate_skin(&png, max_bytes()).unwrap_or_else(|e| panic!("{}: {e}", skin.id));
            assert_eq!(valid.slim, skin.arms == "slim", "{}: the arm style in the data matches the picture", skin.id);
        }
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
