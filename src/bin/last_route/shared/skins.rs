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
use crate::net::{NetError, Pending};
use crate::net::world_sync::{clean_skin, is_skin_hash, SKIN_DEFAULT};
use crate::shared::paths;
pub use crate::shared::skin_png::SKIN_SIZE;
use crate::shared::skin_png::{self, Limits, Shape};
use crate::shared::skin_validate::{self, PNG_SIGNATURE};

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
    let mut loaded = skins.and_then(|dir| load_skin_in(dir, &id));
    let mut id = id;
    // A skin adopted from the account at sign-in is saved as its hash and lives in the cache, not in the skins folder.
    if loaded.is_none() && is_skin_hash(saved) {
        loaded = read_skin_in(&[cache.map(Path::to_path_buf)], saved).and_then(|bytes| validate_skin(&bytes, max_bytes()).ok()).filter(|valid| valid.hash == saved);
        if loaded.is_some() {
            id = saved.to_string();
        }
    }
    if loaded.is_none() && skins.is_none() {
        // no skins folder (the browser): the applied custom skin lives in memory
        let wanted = clean_skin(saved);
        if let Some(valid) = load_memory_custom(&wanted) {
            loaded = Some(valid);
            id = wanted;
        }
    }
    match loaded {
        Some(valid) => {
            if let Some(dir) = cache {
                if fetch_needed(dir, &valid.hash) {
                    let _ = write_skin_file(dir, &valid.hash, &valid.png);
                }
            }
            // The cache is only a cache: pruning (or a wiped folder) must never take the runner's own skin away, so the
            // verified bytes are also kept in memory ([`read_skin`] finds them there) and the file is never evicted.
            remember_own(&valid.hash, &valid.png);
            mark_in_use(&valid.hash);
            // The saved `skin_slim` is the layout read when a file was picked and saved as CUSTOM (its canonical PNG has an
            // opaque base, so the layout cannot be read from it again). Any other file still has its layout, so it is
            // read from the file just validated, whatever an old or hand-edited settings file says.
            let layout_slim = if id == clean_skin(CUSTOM_STEM) { detected_slim } else { valid.slim };
            Worn { hash: valid.hash, slim: arms.resolve(layout_slim) }
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
    skin_dir().and_then(|dir| load_skin_in(&dir, id)).or_else(|| load_memory_custom(id))
}

/// The key of the browser's localStorage entry that keeps the applied custom skin between page loads (base64 of the PNG).
#[cfg(target_arch = "wasm32")]
const CUSTOM_STORAGE_KEY: &str = "last_route.custom_skin";

fn memory_custom() -> std::sync::MutexGuard<'static, Option<Vec<u8>>> {
    static CUSTOM: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);
    CUSTOM.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The applied custom skin kept in memory (no skins folder: the browser has no files, a desktop without a user folder
/// has none either). The browser also keeps it in localStorage and reads it back on the first use after a page load.
fn stored_custom() -> Option<Vec<u8>> {
    // only the browser build fills it in below
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
    let mut held = memory_custom();
    #[cfg(target_arch = "wasm32")]
    if held.is_none() {
        *held = crate::app::web_storage::get(CUSTOM_STORAGE_KEY).and_then(|text| crate::net::base64_decode(&text));
    }
    held.clone()
}

/// The validated custom skin held in memory, when `id` is the custom skin's list id.
fn load_memory_custom(id: &str) -> Option<ValidSkin> {
    if id != clean_skin(CUSTOM_STEM) {
        return None;
    }
    validate_skin(&stored_custom()?, max_bytes()).ok()
}

/// Keeps the custom skin in memory (and in the browser's localStorage) when there is no skins folder to write to.
fn keep_custom_in_memory(bytes: &[u8]) -> String {
    *memory_custom() = Some(bytes.to_vec());
    #[cfg(target_arch = "wasm32")]
    let _ = crate::app::web_storage::set(CUSTOM_STORAGE_KEY, &crate::net::base64_encode(bytes));
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    clean_skin(CUSTOM_STEM)
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
    if let Some(valid) = load_skin(id) {
        return Some((valid.hash, valid.png));
    }
    // A skin adopted from the account at sign-in is saved as its hash and lives in the cache (or in memory), not in the
    // skins folder: the same lookup the worn skin uses (worn_skin_in), so the SKIN tab shows the account's skin too.
    if is_skin_hash(id) {
        return read_skin(id).and_then(|bytes| validate_skin(&bytes, max_bytes()).ok()).filter(|v| v.hash == id).map(|v| (v.hash, v.png));
    }
    None
}

/// Counts one more saved skin, so a caller that resolved the worn skin once resolves it again even though its id did not
/// change (the account's skin was adopted at sign-in and its file has just become readable).
pub fn bump_generation() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// The lowercase hex SHA-256 of `bytes`: the id a skin PNG is known by (64 hex characters).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The limits of the shared PNG check, from data/skins/config.json (zero when the config is missing: everything is refused).
pub fn skin_limits() -> skin_validate::Limits {
    skin_validate::Limits { max_bytes: max_bytes(), max_chunks: config().map_or(0, |c| c.png_max_chunks) }
}

/// True when `bytes` is the skin `id` AND safe to draw: the SHA-256 matches and the whole file passes the one shared check
/// ([`skin_validate::check_skin_png_with`]: every chunk length and CRC, the chunk order, exactly 64x64, a real bounded
/// inflate of exactly the promised size, an opaque base layer). A header that merely looks right is not enough. Disk
/// copies, cache files and downloads are all checked this way, and only a skin that passes is ever decoded.
pub fn verify_skin(id: &str, bytes: &[u8]) -> bool {
    sha256_hex(bytes) == id && skin_validate::check_skin_png_with(bytes, skin_limits()).is_ok()
}

/// The 64x64 RGBA pixels a verified remote skin is drawn with: decoded by the shared validator's own bounded decoder (it
/// cannot panic), with the base layer made opaque (only the second layer may be transparent) as a last repair, like
/// Godot's `PlayerSkin.validate`. A file that passed [`verify_skin`] already is opaque. An `Err` means do not use it.
pub fn remote_skin_rgba(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let layout = layout().ok_or_else(|| "the skin rules are missing".to_string())?;
    let mut px = skin_validate::decode_skin_png_with(bytes, skin_limits())?;
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

/// The bytes of a skin known by `id` from the user's own folder or the fetch cache, when they verify: read with the size cap
/// (a huge or special file is never loaded), the hash matches AND the whole file passes the one shared check
/// ([`verify_skin`]), so a hostile file that an older build cached, or one dropped into the folder by hand, can never
/// reach the image decoder.
pub fn read_skin(id: &str) -> Option<Vec<u8>> {
    read_skin_in(&[skin_dir(), cache_dir()], id)
}

/// [`read_skin`] over the given folders; the runner's own worn skin is also found in memory ([`remember_own`]).
pub fn read_skin_in(dirs: &[Option<PathBuf>], id: &str) -> Option<Vec<u8>> {
    if !is_skin_hash(id) {
        return None;
    }
    let file = format!("{id}.png");
    dirs.iter()
        .flatten()
        .map(|dir| dir.join(&file))
        .filter_map(|path| read_capped(&path, max_bytes()).ok())
        .find(|bytes| verify_skin(id, bytes))
        .or_else(|| own_skin(id))
        .or_else(|| peer_skin(id))
}

/// The skins of other runners kept in memory when there is no cache folder to keep them in (the browser has no files).
/// Bounded like the GPU cache ([`peer_cache_max`]); the oldest goes first.
fn peer_skins() -> std::sync::MutexGuard<'static, Vec<(String, Vec<u8>)>> {
    static PEERS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, Vec<u8>)>>> = std::sync::OnceLock::new();
    PEERS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keeps a downloaded, already verified skin of another runner in memory under its hash.
fn remember_peer(id: &str, png: &[u8]) {
    keep_newest(&mut peer_skins(), peer_cache_max(), id, png);
}

/// Puts skin `id` last in `list` and drops the oldest ones beyond `max`.
fn keep_newest(list: &mut Vec<(String, Vec<u8>)>, max: usize, id: &str, png: &[u8]) {
    list.retain(|(h, _)| h != id);
    list.push((id.to_string(), png.to_vec()));
    let extra = list.len().saturating_sub(max);
    list.drain(..extra);
}

/// The remembered skin `id` of another runner, re-verified (hash and the full check) before it is handed out.
fn peer_skin(id: &str) -> Option<Vec<u8>> {
    let bytes = peer_skins().iter().find(|(h, _)| h == id).map(|(_, b)| b.clone())?;
    verify_skin(id, &bytes).then_some(bytes)
}

/// How many worn skins are kept in memory (one per run is normal; a few more cover a picker that is changed often).
const OWN_SKINS_MAX: usize = 4;

fn own_skins() -> std::sync::MutexGuard<'static, Vec<(String, Vec<u8>)>> {
    static OWN: std::sync::OnceLock<std::sync::Mutex<Vec<(String, Vec<u8>)>>> = std::sync::OnceLock::new();
    OWN.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keeps the verified PNG of the runner's own worn skin in memory under its hash, so a pruned or wiped cache folder cannot
/// take it away (the upload and the texture both read it through [`read_skin`]).
pub fn remember_own(hash: &str, png: &[u8]) {
    let mut own = own_skins();
    own.retain(|(h, _)| h != hash);
    own.push((hash.to_string(), png.to_vec()));
    let extra = own.len().saturating_sub(OWN_SKINS_MAX);
    own.drain(..extra);
}

/// The remembered worn skin `id`, re-verified (hash and the full check) before it is handed out.
fn own_skin(id: &str) -> Option<Vec<u8>> {
    let bytes = own_skins().iter().find(|(h, _)| h == id).map(|(_, b)| b.clone())?;
    verify_skin(id, &bytes).then_some(bytes)
}

fn in_use() -> std::sync::MutexGuard<'static, std::collections::HashSet<String>> {
    static IN_USE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    IN_USE.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Marks skin `id` as being drawn (a texture is on the GPU for it, or it is the runner's own), so [`prune_cache`] does not
/// delete its file while it is in use.
pub fn mark_in_use(id: &str) {
    in_use().insert(id.to_string());
}

/// Ends [`mark_in_use`] (the texture was dropped).
pub fn unmark_in_use(id: &str) {
    in_use().remove(id);
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
    /// How old a `<id>.tmp` file in the cache must be before the prune treats it as a leftover (a younger one is a download
    /// running on another thread).
    pub tmp_stale_secs: u64,
    /// The most downloaded skins kept in the cache folder and on the GPU; the oldest go first (Godot `PEER_CACHE_MAX`, 16).
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
        png_max_chunks: v["png"]["max_chunks"].as_u64()? as usize,
        upload_retry_secs: v["upload_retry_secs"].as_f64()?,
        host_base: fetch["host_base"].as_str()?.trim_end_matches('/').to_string(),
        host_path: fetch["host_path"].as_str()?.to_string(),
        backend_path: fetch["backend_path"].as_str()?.to_string(),
        v1_path: fetch["v1_path"].as_str()?.to_string(),
        timeout_secs: fetch["timeout_secs"].as_u64()?,
        retry_secs: fetch["retry_secs"].as_u64()?,
        max_concurrent: fetch["max_concurrent"].as_u64()? as usize,
        tmp_stale_secs: fetch["tmp_stale_secs"].as_u64()?,
        cache_max: (fetch["cache_max"].as_u64()? as usize).max(1),
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

/// Tries each URL in order with `get` and returns the first body that fully verifies as skin `id`
/// ([`verify_skin`]). A reply that fails or does not verify moves on to the next URL.
fn first_verified_skin(urls: &[String], id: &str, mut get: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    urls.iter().find_map(|url| get(url).filter(|bytes| verify_skin(id, bytes)))
}

/// The content types a skin picture may be served as. Plain servers and CDNs often send the generic ones.
const PNG_TYPES: [&str; 4] = ["image/png", "image/x-png", "application/octet-stream", "binary/octet-stream"];

/// Downloads one address. The backend (plain http, possibly on a LAN) is asked once. An https address (the skins host,
/// a player's link) may redirect up to the client's cap, each hop re-checked by the skin URL rules, and never
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
    retry_at: std::collections::HashMap<String, crate::shared::clock::Instant>,
    /// Failures in a row per key, and the words of the last one (shown to the player, never an address).
    fails: std::collections::HashMap<String, u32>,
    why: std::collections::HashMap<String, String>,
    /// When the recent downloads were started, newest last (see [`Book::fetch_allowed`]).
    fetches: std::collections::VecDeque<crate::shared::clock::Instant>,
}

/// The wait after the `fails`-th failure in a row: 5 s, 10 s, 20 s, ... never more than `cap` (the config's `retry_secs`,
/// Godot's 60 s). A skin the server was only late to hold is picked up in seconds instead of a minute.
fn backoff(fails: u32, cap: std::time::Duration) -> std::time::Duration {
    let step = std::time::Duration::from_secs(5u64.saturating_mul(1u64 << fails.saturating_sub(1).min(10)));
    step.min(cap)
}

/// How many failed download keys the book remembers at once.
const MAX_TRACKED_DOWNLOADS: usize = 64;
/// At most this many downloads start per [`FETCH_WINDOW`], whatever ids other runners announce.
const MAX_FETCHES_PER_WINDOW: usize = 8;
const FETCH_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

impl Book {
    /// True (and `key` is now running) when it is not running, not cooling down, and fewer than `max` jobs run.
    fn begin(&mut self, key: &str, now: crate::shared::clock::Instant, max: usize) -> bool {
        if self.running.contains(key) || self.running.len() >= max || self.retry_at.get(key).is_some_and(|at| now < *at) {
            return false;
        }
        self.running.insert(key.to_string());
        true
    }

    /// [`Book::begin`] for a download, also held back by the global start rate (a flood of announced ids cannot start a flood
    /// of requests, and a key that is only refused for the rate is not marked as failed).
    fn begin_download(&mut self, key: &str, now: crate::shared::clock::Instant, max: usize) -> bool {
        while self.fetches.front().is_some_and(|t| now.duration_since(*t) >= FETCH_WINDOW) {
            self.fetches.pop_front();
        }
        if self.fetches.len() >= MAX_FETCHES_PER_WINDOW || !self.begin(key, now, max) {
            return false;
        }
        self.fetches.push_back(now);
        true
    }

    /// Forgets everything about `key` unless it is running.
    fn forget(&mut self, key: &str) {
        if !self.running.contains(key) {
            self.retry_at.remove(key);
            self.fails.remove(key);
            self.why.remove(key);
        }
    }

    /// Ends a job: a failure cools `key` down for `retry` (with `ramp`, a short wait that doubles per failure in a row up
    /// to `retry`), a success clears it. `why` is what a failure says.
    fn finish_with(&mut self, key: &str, ok: bool, now: crate::shared::clock::Instant, retry: std::time::Duration, ramp: bool, why: &str) {
        self.running.remove(key);
        if ok {
            self.retry_at.remove(key);
            self.fails.remove(key);
            self.why.remove(key);
        } else {
            let n = self.fails.entry(key.to_string()).or_insert(0);
            *n += 1;
            let wait = if ramp { backoff(*n, retry) } else { retry };
            self.retry_at.insert(key.to_string(), now + wait);
            self.why.insert(key.to_string(), why.to_string());
            self.prune_downloads(now);
        }
    }

    /// Keeps at most [`MAX_TRACKED_DOWNLOADS`] failed download keys where it can (their ids are announced by other runners,
    /// so they are not to be trusted to stay few): the oldest whose cool-down is over go first. A key still cooling down is
    /// never forgotten (that would let it be asked for again at once; the start rate bounds how fast new ones appear), and
    /// neither is a running job or the runner's own upload.
    fn prune_downloads(&mut self, now: crate::shared::clock::Instant) {
        let tracked = self.retry_at.keys().filter(|k| k.starts_with("down:")).count();
        if tracked <= MAX_TRACKED_DOWNLOADS {
            return;
        }
        let mut over: Vec<(crate::shared::clock::Instant, String)> = self
            .retry_at
            .iter()
            .filter(|(k, at)| k.starts_with("down:") && !self.running.contains(*k) && **at <= now)
            .map(|(k, at)| (*at, k.clone()))
            .collect();
        over.sort();
        for (_, key) in over.into_iter().take(tracked - MAX_TRACKED_DOWNLOADS) {
            self.forget(&key);
        }
    }

    #[cfg(test)]
    fn finish(&mut self, key: &str, ok: bool, now: crate::shared::clock::Instant, retry: std::time::Duration) {
        self.finish_with(key, ok, now, retry, false, "");
    }
}

fn book() -> std::sync::MutexGuard<'static, Book> {
    static BOOK: std::sync::OnceLock<std::sync::Mutex<Book>> = std::sync::OnceLock::new();
    BOOK.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where this runner's own skin is on its way to the other runners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareState {
    /// DEFAULT is worn: nothing to share.
    Default,
    /// The website holds the skin: runners in a room fetch it by its id.
    Shared,
    /// A skin is worn but nobody is signed in to upload it: others see the default one.
    SignedOut,
    /// This game is offline: nothing is sent.
    Offline,
    /// The upload job is running right now.
    Uploading,
    /// Nothing has failed and nothing runs yet: the upload is offered every few seconds.
    Waiting,
    /// The last upload failed (the words say why); it is tried again on its own.
    Retrying(String),
}

impl ShareState {
    /// The line the SKIN tab shows; empty when there is nothing to say.
    pub fn text(&self) -> String {
        match self {
            ShareState::Default | ShareState::Shared => String::new(),
            ShareState::SignedOut => "sign in so other runners see it".to_string(),
            ShareState::Offline => "offline: other runners see the default skin".to_string(),
            ShareState::Uploading => "sharing with other runners...".to_string(),
            ShareState::Waiting => "waiting to share with other runners".to_string(),
            ShareState::Retrying(why) => format!("not shared yet: {why}"),
        }
    }
}

/// Where the worn skin `hash` stands: see [`ShareState`].
pub fn share_state(hash: &str, signed_in: bool, offline: bool, uploaded: &[String]) -> ShareState {
    if !is_skin_hash(hash) {
        return ShareState::Default;
    }
    if holds(uploaded, hash) {
        return ShareState::Shared;
    }
    if !signed_in {
        return ShareState::SignedOut;
    }
    if offline {
        return ShareState::Offline;
    }
    let b = book();
    let key = format!("up:{hash}");
    if b.running.contains(&key) {
        ShareState::Uploading
    } else if let Some(why) = b.why.get(&key) {
        ShareState::Retrying(why.clone())
    } else {
        ShareState::Waiting
    }
}

/// Downloads skin `id` from the first address that serves a fully verified copy and writes it into `dir`. Returns
/// whether the skin is now cached. Blocking (see [`fetch_skin_in_background`]).
pub fn fetch_skin_blocking(cfg: &SkinConfig, backend_base: &str, dir: &Path, id: &str) -> bool {
    fetch_skin_why(cfg, backend_base, dir, id).is_ok()
}

/// [`fetch_skin_blocking`] with the words of the failure (never an address): the first reason that is more than a plain
/// 404, since a 404 from one of the addresses is the ordinary "not there yet".
pub fn fetch_skin_why(cfg: &SkinConfig, backend_base: &str, dir: &Path, id: &str) -> Result<(), String> {
    let bytes = fetch_skin_bytes(cfg, backend_base, id)?;
    if write_skin_file(dir, id, &bytes).is_err() {
        return Err("the skin could not be saved".to_string());
    }
    prune_cache(dir, cfg.cache_max);
    Ok(())
}

/// The verified bytes of skin `id` from the first address that serves them, with the words of the failure as in
/// [`fetch_skin_why`]. Nothing is written: the caller keeps the skin in a folder or in memory ([`remember_peer`]).
pub fn fetch_skin_bytes(cfg: &SkinConfig, backend_base: &str, id: &str) -> Result<Vec<u8>, String> {
    let limit = read_limit();
    let mut why: Option<String> = None;
    let mut note = |text: String| {
        if why.as_ref().map_or(true, |old| old.ends_with("404")) {
            why = Some(text);
        }
    };
    for url in fetch_urls(cfg, backend_base, id) {
        match download(&url, cfg.timeout_secs, limit, &PNG_TYPES) {
            Ok(bytes) if verify_skin(id, &bytes) => return Ok(bytes),
            Ok(_) => note("the server sent a file that is not this skin".to_string()),
            Err(e) => note(download_error_text(&e)),
        }
    }
    Err(why.unwrap_or_else(|| "no server has this skin yet".to_string()))
}

/// Cleans the download cache with the configured temp-file age: see [`prune_cache_with`].
pub fn prune_cache(dir: &Path, keep: usize) -> usize {
    let stale = std::time::Duration::from_secs(config().map_or(60, |c| c.tmp_stale_secs));
    prune_cache_with(dir, keep, stale, &in_use().clone())
}

/// Cleans the download cache: stale leftovers of half-written files, any `<64 hex>.png` that fails the full check (a hostile
/// or broken file from an older build), and then the oldest skins beyond `keep`. Returns how many files were removed.
/// Only files named like a cached skin or its temp file are ever touched. A `<id>.tmp` younger than `tmp_stale` belongs to
/// a download running on another thread and is left alone (removing it would make that download's rename fail), and an id
/// in `protected` (drawn right now, or the runner's own skin) is never evicted by the age rule.
pub fn prune_cache_with(dir: &Path, keep: usize, tmp_stale: std::time::Duration, protected: &std::collections::HashSet<String>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let now = crate::shared::clock::SystemTime::now();
    let mut removed = 0;
    let mut kept: Vec<(crate::shared::clock::SystemTime, PathBuf, bool)> = Vec::new();
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let stem = name.strip_suffix(".png").or_else(|| name.strip_suffix(".tmp"));
        let Some(stem) = stem.filter(|s| is_skin_hash(s)) else { continue };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).map(crate::shared::clock::from_std).unwrap_or(crate::shared::clock::UNIX_EPOCH);
        if name.ends_with(".tmp") {
            // A temp file is a download in flight until it is old enough to be a leftover.
            if now.duration_since(modified).is_ok_and(|age| age >= tmp_stale) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
            continue;
        }
        let good = read_capped(&path, max_bytes()).is_ok_and(|bytes| verify_skin(stem, &bytes));
        if good {
            kept.push((modified, path, protected.contains(stem)));
        } else if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    kept.sort_by_key(|(age, _, _)| *age);
    let mut extra = kept.len().saturating_sub(keep);
    for (_, path, is_protected) in kept {
        if extra == 0 {
            break;
        }
        if !is_protected && std::fs::remove_file(path).is_ok() {
            removed += 1;
            extra -= 1;
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

/// How many skins of other runners are kept, on the GPU and in the disk cache (the config's `cache_max`).
pub fn peer_cache_max() -> usize {
    config().map_or(16, |c| c.cache_max)
}

/// The website the runners' skins are fetched from, set once at startup (see `app`). Empty means fetching is off.
static BACKEND_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Sets the base URL skins are fetched from (`GET <base>/api/v1/skins/<id>`, see [`fetch_urls`]). Only the first call counts.
pub fn set_backend_base(base: &str) {
    let _ = BACKEND_BASE.set(base.trim_end_matches('/').to_string());
}

/// Starts a background fetch of skin `id` into the cache folder, or into memory when there is no cache folder (the
/// browser has no files). It never blocks the frame; the skin shows up on a later check once it has been downloaded and
/// verified. The backend route is tried first, then the skins host and the legacy route. Does nothing when no backend is
/// set (fetching is off), the id is not a skin hash, or the skin is already held.
pub fn fetch_skin_in_background(id: &str) {
    let Some(base) = BACKEND_BASE.get().filter(|b| !b.is_empty()).cloned() else { return };
    let Some(cfg) = config() else { return };
    start_download(cfg, base, cache_dir(), id);
}

/// [`fetch_skin_in_background`] for a given config, backend and cache folder (None: keep the skin in memory). True when a
/// download was started; false when the skin is held already, a download of it runs or cools down after a failure, or
/// the start rate is used up.
pub(crate) fn start_download(cfg: SkinConfig, base: String, dir: Option<PathBuf>, id: &str) -> bool {
    // The id comes from another runner's profile: only 64 lowercase hex characters may reach a file path or the network.
    if !is_skin_hash(id) {
        return false;
    }
    let held = match &dir {
        Some(dir) => !fetch_needed(dir, id),
        None => peer_skin(id).is_some(),
    };
    if held {
        return false;
    }
    let key = format!("down:{id}");
    if !book().begin_download(&key, crate::shared::clock::Instant::now(), cfg.max_concurrent) {
        return false;
    }
    let id = id.to_string();
    crate::shared::task::detach(move || {
        // Once per run, before the first download: drop what an older build cached that fails the full check.
        static PURGED: std::sync::Once = std::sync::Once::new();
        if let Some(dir) = &dir {
            PURGED.call_once(|| {
                prune_cache(dir, cfg.cache_max);
            });
        }
        let result = match &dir {
            Some(dir) => fetch_skin_why(&cfg, &base, dir, &id),
            None => fetch_skin_bytes(&cfg, &base, &id).map(|bytes| remember_peer(&id, &bytes)),
        };
        let ok = result.is_ok();
        let why = result.err().unwrap_or_default();
        if !ok {
            eprintln!("skins: {id} could not be fetched ({why}); trying again shortly");
            note_download_failed(&id, &why);
        }
        book().finish_with(&key, ok, crate::shared::clock::Instant::now(), std::time::Duration::from_secs(cfg.retry_secs), true, &why);
    });
    true
}

/// The server origin the uploaded record is for: the backend skins are fetched from (empty when fetching is off).
fn server_origin() -> String {
    BACKEND_BASE.get().cloned().unwrap_or_default()
}

/// What this session knows about the recorded skins: true once the server was seen holding the hash (or it was uploaded
/// this session), false once it was found missing. An absent hash is not checked yet.
fn checked() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, bool>> {
    lock_checked(checked_map())
}

type Checked = std::sync::Mutex<std::collections::HashMap<String, bool>>;

fn checked_map() -> &'static Checked {
    static CHECKED: std::sync::OnceLock<Checked> = std::sync::OnceLock::new();
    CHECKED.get_or_init(Default::default)
}

fn lock_checked(map: &Checked) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, bool>> {
    map.lock().unwrap_or_else(|e| e.into_inner())
}

/// The ids recorded in `<user dir>/skins-uploaded.json` for the server `origin`. The file is `{"server": origin, "ids":
/// [...]}`; an old plain list `[id, ...]` (written before the origin was stored) is read as belonging to this server, and
/// a record of another server is ignored. A missing or unreadable file is empty.
fn read_record_at(path: &Path, origin: &str) -> Vec<String> {
    let Some(value) = std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()) else {
        return Vec::new();
    };
    let ids = match &value {
        serde_json::Value::Array(ids) => ids,
        serde_json::Value::Object(map) if map.get("server").and_then(|v| v.as_str()) == Some(origin) => match map.get("ids") {
            Some(serde_json::Value::Array(ids)) => ids,
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    ids.iter().filter_map(|v| v.as_str()).filter(|id| is_skin_hash(id)).map(str::to_string).collect()
}

/// Writes the record for `origin`; an empty list removes the file.
fn write_record_at(path: &Path, origin: &str, ids: &[String]) {
    if ids.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    let Ok(text) = serde_json::to_string(&serde_json::json!({"server": origin, "ids": ids})) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// The recorded ids that were checked against the server this session (or uploaded this session): the ids the website
/// holds for this account. The record alone is not trusted, since the server may have dropped the file since.
pub fn uploaded_ids() -> Vec<String> {
    uploaded_ids_for(uploaded_path().as_deref(), &server_origin(), checked_map())
}

/// [`uploaded_ids`] for the record file, if there is one. With no user folder (the browser) there is no record file, so
/// the skins the server was seen holding this session are all there is. Without this a runner in the browser never
/// published its skin, whatever the website held.
fn uploaded_ids_for(path: Option<&Path>, origin: &str, known: &Checked) -> Vec<String> {
    match path {
        Some(path) => uploaded_ids_in(path, origin, known),
        None => held_this_session(known),
    }
}

/// The ids this session uploaded or saw the server holding, for a runner with no record file.
fn held_this_session(known: &Checked) -> Vec<String> {
    let mut held: Vec<String> = lock_checked(known).iter().filter(|(id, held)| **held && is_skin_hash(id)).map(|(id, _)| id.clone()).collect();
    held.sort();
    held
}

/// [`uploaded_ids`] for a record file, server origin and checked map. With no origin there is no server to ask (see
/// [`verify_held_in`]), so the record is trusted as it is.
fn uploaded_ids_in(path: &Path, origin: &str, known: &Checked) -> Vec<String> {
    let recorded = read_record_at(path, origin);
    if origin.is_empty() {
        return recorded;
    }
    let known = lock_checked(known);
    recorded.into_iter().filter(|id| known.get(id) == Some(&true)).collect()
}

/// True when `uploaded` (the list [`uploaded_ids`] gives) holds the skin `id`. Only a skin hash can be held. A host
/// re-publishes its skin once the website holds it (see `net::world_sync::NetSession::set_skin`).
pub fn holds(uploaded: &[String], id: &str) -> bool {
    is_skin_hash(id) && uploaded.iter().any(|x| x == id)
}

fn uploaded_path() -> Option<PathBuf> {
    paths::user_dir().map(|d| d.join("skins-uploaded.json"))
}

/// Records the id the website now holds for this account: the list becomes just `[id]`. The website keeps one skin per
/// account, so an earlier id is no longer something to rely on (it is only kept for a grace period): picking it again
/// uploads it again, and the room's published skin never names a file the website has dropped.
fn remember_uploaded(id: &str) {
    remember_uploaded_in(uploaded_path().as_deref(), &server_origin(), checked_map(), id);
}

/// [`remember_uploaded`] for a record file (None: the browser, which keeps the list in memory only), origin and checked map.
fn remember_uploaded_in(path: Option<&Path>, origin: &str, known: &Checked, id: &str) {
    if let Some(path) = path {
        write_record_at(path, origin, &[id.to_string()]);
    }
    let mut known = lock_checked(known);
    if path.is_none() {
        // the session list is all there is without a record file: one skin per account here too
        known.retain(|_, held| !*held);
    }
    known.insert(id.to_string(), true);
}

/// Looks up whether the server holds the recorded skin `hash` (`GET <base>/api/v1/skins/<hash>`, which the server
/// re-validates). `Some(true)` when it serves a verified copy; `Some(false)` when it does not (a 404, or a file that
/// fails the check), and then the record drops the hash so the canonical PNG is uploaded again; `None` when the server
/// could not be asked (try again later). A hash that is not recorded for `origin` is `Some(false)` without asking.
fn verify_held_at(cfg: &SkinConfig, path: &Path, origin: &str, hash: &str) -> Option<bool> {
    let recorded = read_record_at(path, origin);
    if !recorded.iter().any(|id| id == hash) {
        return Some(false);
    }
    // The backend's own route first, then the Godot route (a server of the Node backend has only that one: asking one
    // route alone made it look like the server never held the skin, so every start uploaded it again).
    for url in [format!("{origin}/api/v1/skins/{hash}"), skin_backend_url(cfg, origin, hash)] {
        match download(&url, cfg.timeout_secs, read_limit(), &PNG_TYPES) {
            Ok(bytes) if verify_skin(hash, &bytes) => return Some(true),
            Ok(_) | Err(NetError::Http { status: 404, .. }) => {}
            Err(_) => return None,
        }
    }
    // The GET was slow: another skin may have been recorded meanwhile, so drop only this hash from a fresh read.
    let fresh = read_record_at(path, origin);
    if fresh.iter().any(|id| id == hash) {
        let rest: Vec<String> = fresh.into_iter().filter(|id| id != hash).collect();
        write_record_at(path, origin, &rest);
    }
    Some(false)
}

/// True once it is known whether the server holds the worn skin `hash`; until then a background check runs (once per
/// session per hash, retried after a failure) and this answers false. Only a settled hash is offered for upload, and
/// [`uploaded_ids`] lists it only when the server was seen holding it.
pub fn verify_held(hash: &str) -> bool {
    let (Some(path), Some(settings)) = (uploaded_path(), config()) else { return true };
    verify_held_in(hash, server_origin(), path, settings, checked_map())
}

/// [`verify_held`] for a record file, origin, config and checked map. With no origin there is no server to ask, so the
/// hash counts as settled and [`uploaded_ids_in`] trusts the record.
fn verify_held_in(hash: &str, origin: String, path: PathBuf, settings: SkinConfig, known: &'static Checked) -> bool {
    if !is_skin_hash(hash) || origin.is_empty() || lock_checked(known).contains_key(hash) {
        return true;
    }
    let key = format!("verify:{hash}");
    if !book().begin(&key, crate::shared::clock::Instant::now(), settings.max_concurrent) {
        return false;
    }
    let hash = hash.to_string();
    crate::shared::task::detach(move || {
        let answer = verify_held_at(&settings, &path, &origin, &hash);
        if let Some(held) = answer {
            lock_checked(known).insert(hash.clone(), held);
        }
        let retry = std::time::Duration::from_secs(settings.retry_secs);
        book().finish_with(&key, answer.is_some(), crate::shared::clock::Instant::now(), retry, true, "the server could not be asked");
    });
    false
}

/// A download of `id` ended with `why`. When that was the 404 of a skin this client recorded as uploaded (its own hash,
/// announced to the room), the server no longer holds it: check again.
fn note_download_failed(id: &str, why: &str) {
    if !why.ends_with("404") {
        return;
    }
    let mut known = checked();
    if known.get(id) == Some(&true) {
        known.remove(id);
        book().forget(&format!("verify:{id}"));
    }
}

/// When the worn skin is next offered to the website. A new skin is offered at once; one the website has not taken
/// (not signed in yet, a failed upload) is offered again after the retry delay (Godot skin_sync.gd RETRY_SECONDS).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UploadPlan {
    hash: String,
    next_at: f64,
    /// The ids the website holds, read once per hash (and again after an offer, which may have uploaded it).
    uploaded: Option<Vec<String>>,
}

impl UploadPlan {
    /// True when `hash` should be uploaded now: signed in, a real skin hash the website does not hold yet, and either
    /// a hash not offered before or past the retry time. Answering true starts the retry clock.
    /// `uploaded` lists the ids the website holds; it is only asked for when an upload could be due, because it reads a file.
    pub fn due(&mut self, now: f64, hash: &str, signed_in: bool, uploaded: impl FnOnce() -> Vec<String>, retry_secs: f64) -> bool {
        if self.hash != hash {
            if is_skin_hash(&self.hash) {
                book().forget(&format!("up:{}", self.hash));
            }
            self.hash = hash.to_string();
            self.next_at = 0.0;
            self.uploaded = None;
        }
        if !signed_in || !is_skin_hash(hash) || now < self.next_at {
            return false;
        }
        self.next_at = now + retry_secs;
        let ids = self.uploaded.get_or_insert_with(uploaded);
        if holds(ids, hash) {
            return false;
        }
        self.uploaded = None;
        true
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

/// What signing in does to the local skin: nothing, or wear the account's skin (its hash, and whether the slim arms show).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInSkin {
    Keep,
    Adopt(String, bool),
}

/// The slim arms an account skin is worn with: SLIM and CLASSIC force the account's ARMS setting, AUTO (or an unknown
/// setting) keeps the local value.
pub fn arms_slim(arms: &str, local_slim: bool) -> bool {
    match ArmsSetting::parse(arms) {
        Some(ArmsSetting::Slim) => true,
        Some(ArmsSetting::Classic) => false,
        Some(ArmsSetting::Auto) | None => local_slim,
    }
}

/// What signing in does with the account's website skin. `account_hash` is the hash the website holds (empty for none),
/// `arms` its ARMS setting; `local_skin` is the saved `settings.skin`, `local_is_default` whether no custom skin is
/// applied, `local_slim` the saved `settings.skin_slim` and `worn_hash` the hash the runner wears now.
/// An empty account skin changes nothing. A hash already applied (saved or worn) changes nothing. Otherwise the account's
/// skin is adopted: with no local custom skin, or over one that differs, since the account holds the last upload.
pub fn sign_in_skin(account_hash: &str, arms: &str, local_skin: &str, local_is_default: bool, local_slim: bool, worn_hash: &str) -> SignInSkin {
    let hash = account_hash.trim().to_ascii_lowercase();
    if !is_skin_hash(&hash) {
        return SignInSkin::Keep;
    }
    let applied = !local_is_default && (local_skin == hash || worn_hash == hash);
    if applied {
        return SignInSkin::Keep;
    }
    SignInSkin::Adopt(hash, arms_slim(arms, local_slim))
}

/// The local skin state a sign-in check is judged against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSkin {
    pub skin: String,
    pub slim: bool,
    pub worn_hash: String,
}

/// Whether the account's skin picture is in the cache (or, with no cache folder, in memory) and passes the check.
fn account_picture_held(dir: Option<&Path>, hash: &str) -> bool {
    match dir {
        Some(dir) => !fetch_needed(dir, hash),
        None => peer_skin(hash).is_some(),
    }
}

/// The full sign-in decision for an answer from the website: [`sign_in_skin`], and when the account's skin is adopted and
/// not held yet, its picture is downloaded, checked (hash and the whole PNG check) and kept in the cache. A picture that
/// fails the check is refused with its reason: nothing is written and the caller keeps the local skin.
pub fn sign_in_skin_fetching(info: &crate::net::api::AccountSkin, local: &LocalSkin, cfg: &SkinConfig, base: &str, dir: Option<&Path>) -> Result<SignInSkin, String> {
    let decision = sign_in_skin(&info.hash, &info.arms, &local.skin, local.skin == SKIN_DEFAULT, local.slim, &local.worn_hash);
    let SignInSkin::Adopt(hash, _) = &decision else { return Ok(decision) };
    if account_picture_held(dir, hash) {
        return Ok(decision);
    }
    match dir {
        Some(dir) => fetch_skin_why(cfg, base, dir, hash)?,
        None => {
            let bytes = fetch_skin_bytes(cfg, base, hash)?;
            remember_peer(hash, &bytes);
        }
    }
    Ok(decision)
}

/// A sign-in check running on a worker thread (one GET, and a download when the account's skin must be adopted). The
/// frame loop polls it once; the answer is applied only when the local skin has not changed since the check started.
pub struct SignInSync {
    pending: Pending<SignInSkin>,
    local_skin: String,
}

impl SignInSync {
    /// Starts the check for the signed-in `account`. None when offline or the skin settings are missing.
    pub fn start(cfg: BackendConfig, account: String, local: LocalSkin) -> Option<SignInSync> {
        if cfg.offline {
            return None;
        }
        let settings = config()?;
        let dir = cache_dir();
        let local_skin = local.skin.clone();
        let pending = Pending::spawn(move || {
            let info = api::account_skin(&cfg, &account)?;
            // a refused picture, or a failed download, leaves the local skin as it is
            match sign_in_skin_fetching(&info, &local, &settings, &cfg.base_url, dir.as_deref()) {
                Ok(decision) => Ok(decision),
                Err(why) => {
                    eprintln!("skins: the account skin could not be adopted ({why})");
                    Ok(SignInSkin::Keep)
                }
            }
        });
        Some(SignInSync { pending, local_skin })
    }

    /// The answer once the check has finished (None while it runs). An answer for a local skin that changed since the
    /// check started is dropped, and a failed check counts as Keep.
    pub fn poll(&mut self, current_skin: &str) -> Option<SignInSkin> {
        let result = self.pending.poll()?;
        let decision = match result {
            Ok(decision) => decision,
            Err(e) => {
                eprintln!("skins: the account skin could not be checked ({})", download_error_text(&e));
                SignInSkin::Keep
            }
        };
        Some(if current_skin == self.local_skin { decision } else { SignInSkin::Keep })
    }
}

/// Records that the website holds the account's skin `hash` (it was just adopted from there), so it is not uploaded back.
pub fn mark_account_holds(hash: &str) {
    if is_skin_hash(hash) {
        remember_uploaded(hash);
    }
}

/// What a server that answers 404 to both skin upload routes is told apart by.
const SERVER_NEEDS_UPDATE: &str = "server needs an update";

/// Uploads the skin known by `hash` (its verified copy in the skins folder or the cache) to the website in the
/// background, when signed in (`session_token` set) and not offline. Returns true when an upload was started; a skin the
/// website already has (see [`uploaded_ids`]) is not sent again. A failure is dropped here: the caller's [`UploadPlan`]
/// offers the skin again after its retry delay.
pub fn upload_in_background(cfg: BackendConfig, session_token: String, hash: String) -> bool {
    if session_token.is_empty() || cfg.offline || !is_skin_hash(&hash) || holds(&uploaded_ids(), &hash) {
        return false;
    }
    let Some(settings) = config() else { return false };
    let key = format!("up:{hash}");
    let Some(bytes) = read_skin(&hash) else {
        // The picture is gone from the skins folder and the cache: say so instead of looking like an upload on its way.
        let retry = std::time::Duration::from_secs(settings.retry_secs);
        book().finish_with(&key, false, crate::shared::clock::Instant::now(), retry, true, "the skin file could not be read");
        return false;
    };
    // One upload per skin at a time, and a failed one waits `retry_secs` before the next try (Godot `RETRY_SECONDS`), so a
    // caller may ask again every frame.
    if !book().begin(&key, crate::shared::clock::Instant::now(), settings.max_concurrent) {
        return false;
    }
    crate::shared::task::detach(move || {
        let result = api::upload_skin(&cfg, &session_token, &bytes);
        let ok = matches!(&result, Ok(server_id) if *server_id == hash);
        let why = match &result {
            Ok(_) => "the server answered with another skin".to_string(),
            Err(e) if api::is_unknown_endpoint(e) => SERVER_NEEDS_UPDATE.to_string(),
            Err(NetError::Http { status: 401 | 403, .. }) => "the server did not accept this sign-in".to_string(),
            Err(e) => download_error_text(e).replace("the download", "the upload"),
        };
        if ok {
            remember_uploaded(&hash);
        } else {
            eprintln!("skins: the upload of {hash} failed ({why}); trying again shortly");
        }
        // A server without the skin routes will not grow them in seconds: wait the full delay instead of ramping up.
        let ramp = why != SERVER_NEEDS_UPDATE;
        book().finish_with(&key, ok, crate::shared::clock::Instant::now(), std::time::Duration::from_secs(settings.retry_secs), ramp, &why);
    });
    true
}

/// Removes the account's stored skin from the website in the background (a player chose DEFAULT), and forgets that the
/// old skins were uploaded so choosing one again uploads it again. Does nothing offline or signed out.
pub fn clear_in_background(cfg: BackendConfig, session_token: String) {
    if session_token.is_empty() || cfg.offline {
        return;
    }
    let Some(settings) = config() else { return };
    let key = "clear".to_string();
    if !book().begin(&key, crate::shared::clock::Instant::now(), settings.max_concurrent) {
        return;
    }
    crate::shared::task::detach(move || {
        let result = api::clear_skin(&cfg, &session_token);
        // A server with no skin store of its own (SKINS_UNAVAILABLE) holds nothing the game could clear either.
        let ok = clear_succeeded(&result);
        if ok {
            forget_uploaded();
        }
        book().finish_with(&key, ok, crate::shared::clock::Instant::now(), std::time::Duration::from_secs(settings.retry_secs), false, "the website could not clear the skin");
    });
}

/// True when a clear request ended with the website holding no skin for the account: it said so, or it has no skin store at all.
fn clear_succeeded<T>(result: &Result<T, NetError>) -> bool {
    match result {
        Ok(_) => true,
        Err(NetError::Http { status: 503, code, .. }) => code == "SKINS_UNAVAILABLE",
        Err(_) => false,
    }
}

/// Empties the uploaded list: the website holds none of the skins any more.
fn forget_uploaded() {
    if let Some(path) = uploaded_path() {
        let _ = std::fs::remove_file(path);
    }
    checked().clear();
}

/// Whether the skin known by wire id `id` uses slim arms, read from its picture when it is in the skins folder or the
/// cache. None while the picture is not here yet (it is being fetched) or does not pass the full check.
pub fn skin_is_slim(id: &str) -> Option<bool> {
    if let Some(known) = slim_table().get(id) {
        return Some(*known);
    }
    // A picture that is not here yet, or fails the check, is looked for again only every two seconds: this is asked
    // for every remote runner on every frame.
    static MISSES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, crate::shared::clock::Instant>>> = std::sync::OnceLock::new();
    let misses = MISSES.get_or_init(Default::default);
    let now = crate::shared::clock::Instant::now();
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
/// The schema of the rules file (data/skins/rules.json): the URL checks, key repeat and layout for the SKIN tab.
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

/// The URL rules, key repeat and layout, read from data/skins/rules.json.
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
}

/// The rules from their JSON text. None when the schema or any value is missing.
pub fn parse_rules(text: &str) -> Option<SkinRules> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v["schema"].as_str()? != RULES_SCHEMA {
        return None;
    }
    let url = &v["url"];
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
/// (when not empty) narrows the host further.
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
    let limits = Limits { max_bytes: cap, ..skin_limits() };
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
    fetch_url_skin_via(url, &mut |url, hosts, types| fetch_https(url, rules, hosts, read_limit(), types))
}

/// What fetches one address for the FROM URL download: `(url, allowed hosts, accepted content types)` to the
/// body. The real one is [`fetch_https`]; a test supplies its own.
pub type Getter<'a> = &'a mut dyn FnMut(&str, &[&str], &[&str]) -> Result<Vec<u8>, String>;

/// [`fetch_url_skin`] over a given getter.
pub fn fetch_url_skin_via(url: &str, get: Getter) -> Result<ValidSkin, String> {
    let bytes = get(url, &[], &PNG_TYPES)?;
    validate_skin(&bytes, max_bytes())
}

/// The list id a picked skin is saved under: `custom.png` in the skins folder.
pub const CUSTOM_STEM: &str = "custom";

/// Counts the skins saved with [`save_custom_skin`], so a caller that resolved the worn skin once knows to resolve it
/// again after an APPLY even though the list id (always CUSTOM) did not change.
static GENERATION: crate::shared::atomic64::AtomicU64 = crate::shared::atomic64::AtomicU64::new(0);

/// The number of skins saved as the user's own so far this run.
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// Saves validated skin bytes as the user's own skin (`<skins folder>/custom.png`), replacing it in one step. Returns
/// the list id ([`clean_skin`] of the stem). Without a skins folder (the browser) the skin is kept in memory instead.
pub fn save_custom_skin(bytes: &[u8]) -> std::io::Result<String> {
    // the browser has no files: the skin is kept in memory (and localStorage) and worn from there
    let Some(dir) = skin_dir() else { return Ok(keep_custom_in_memory(bytes)) };
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

    #[test]
    fn skin_png_returns_a_remembered_account_skin_by_its_hash() {
        let valid = validate_skin(&fixture_png(false), max_bytes()).expect("the fixture is a valid skin");
        remember_own(&valid.hash, &valid.png);
        let (hash, png) = skin_png(&valid.hash).expect("the account's skin is found by its hash");
        assert_eq!(hash, valid.hash);
        assert_eq!(png, valid.png, "the canonical bytes are handed out");
    }

    #[test]
    fn skin_png_gives_nothing_for_an_unknown_hash_default_or_a_non_hash() {
        assert!(skin_png(&sha256_hex(b"no such skin anywhere")).is_none(), "a hash not held is nothing");
        assert!(skin_png(SKIN_DEFAULT).is_none(), "DEFAULT has no file");
        assert!(skin_png("NOT A HASH").is_none(), "a non-hash id is never read as a file");
    }

    #[test]
    fn bump_generation_counts_one_more_saved_skin() {
        let before = generation();
        bump_generation();
        assert!(generation() > before, "a bump is seen by callers that compare the generation");
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
        let dir = std::env::temp_dir().join(format!("lr_skins_test_{}", crate::shared::task::process_id()));
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
    fn the_servers_route_is_tried_first_then_the_host_then_the_legacy_route() {
        let id = sha256_hex(b"skin");
        let cfg = shipped_config();
        let urls = fetch_urls(&cfg, "http://host:8105/", &id);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], skin_v1_url(&cfg, "http://host:8105/", &id), "lr_server's route first");
        assert_eq!(urls[1], skin_host_url(&cfg, &id));
        assert_eq!(urls[2], skin_backend_url(&cfg, "http://host:8105/", &id));
        assert_eq!(urls[0], format!("http://host:8105/api/v1/skins/{id}"));
    }

    /// A canonical, fully valid skin and its id.
    fn good_skin() -> (String, Vec<u8>) {
        skin_from_png(fixture_png(false)).expect("the fixture is a valid skin")
    }

    /// A PNG with a good 64x64 header and no body: a header-only check accepts it, and decoding it panics.
    fn hostile_skin() -> (String, Vec<u8>) {
        let bytes = png_head(64, 64);
        (sha256_hex(&bytes), bytes)
    }

    #[test]
    fn the_games_own_encoder_output_passes_the_shared_check() {
        let (id, bytes) = good_skin();
        assert_eq!(skin_validate::check_skin_png(&bytes, max_bytes()), Ok(()));
        assert!(verify_skin(&id, &bytes));
        // A skin with a clear hat layer (the usual case) passes too.
        let mut px = fixture_rgba(false);
        px[px_at(40, 5, 64) + 3] = 0;
        let png = encode_png(&px, 64, 64);
        assert_eq!(skin_validate::check_skin_png(&png, max_bytes()), Ok(()));
    }

    #[test]
    fn a_header_only_download_matches_its_hash_but_is_not_a_verified_skin() {
        let (id, bytes) = hostile_skin();
        assert_eq!(sha256_hex(&bytes), id);
        assert!(!verify_skin(&id, &bytes), "the full check refuses it");
        assert!(png_is_skin_size(&bytes), "the header alone would have passed");
    }

    #[test]
    fn fetch_stops_at_the_first_route_that_answers_with_a_verified_skin() {
        let (id, bytes) = good_skin();
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
                // read the body of a POST too: closing with unread data can reset the connection before the client reads the answer
                let head_end = got.windows(4).position(|w| w == b"\r\n\r\n").map_or(got.len(), |i| i + 4);
                let want = String::from_utf8_lossy(&got[..head_end]).lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                while got.len() < head_end + want {
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
        let dir = std::env::temp_dir().join(format!("lr_skins_{name}_{}", crate::shared::task::process_id()));
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
        use crate::shared::clock::Instant;
use std::time::Duration;
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
    fn failed_jobs_are_retried_after_a_short_wait_that_grows_to_the_cap() {
        use crate::shared::clock::Instant;
use std::time::Duration;
        let (start, cap) = (Instant::now(), Duration::from_secs(60));
        let secs = |s| Duration::from_secs(s);
        assert_eq!([1, 2, 3, 4, 5, 6, 40].map(|n| backoff(n, cap)), [secs(5), secs(10), secs(20), secs(40), secs(60), secs(60), secs(60)]);
        let mut book = Book::default();
        assert!(book.begin("up:a", start, 2));
        book.finish_with("up:a", false, start, cap, true, "the server has no skin upload (it needs an update)");
        assert!(!book.begin("up:a", start + secs(4), 2));
        assert!(book.begin("up:a", start + secs(5), 2), "the first retry comes after 5 s");
        book.finish_with("up:a", false, start + secs(5), cap, true, "x");
        assert!(!book.begin("up:a", start + secs(14), 2));
        assert!(book.begin("up:a", start + secs(15), 2), "the second after 10 s more");
        book.finish_with("up:a", true, start + secs(15), cap, true, "");
        assert!(book.fails.is_empty() && book.why.is_empty(), "a success forgets the failures");
    }

    #[test]
    fn the_share_state_tells_the_player_where_the_worn_skin_stands() {
        let id = "ab".repeat(32);
        assert_eq!(share_state(SKIN_DEFAULT, true, false, &[]), ShareState::Default);
        assert_eq!(share_state(&id, true, false, &[id.clone()]), ShareState::Shared);
        assert_eq!(share_state(&id, false, false, &[]), ShareState::SignedOut);
        assert!(ShareState::SignedOut.text().contains("sign in"));
        assert_eq!(ShareState::Shared.text(), "");
        let unique = "cd".repeat(32);
        assert_eq!(share_state(&unique, true, true, &[]), ShareState::Offline);
        assert_eq!(share_state(&unique, true, false, &[]), ShareState::Waiting, "nothing runs and nothing failed: not 'sharing'");
        book().running.insert(format!("up:{unique}"));
        assert_eq!(share_state(&unique, true, false, &[]), ShareState::Uploading, "only while the job runs");
        book().running.remove(&format!("up:{unique}"));
        book().finish_with(&format!("up:{unique}"), false, crate::shared::clock::Instant::now(), std::time::Duration::from_secs(60), true, "the server did not accept this sign-in");
        let state = share_state(&unique, true, false, &[]);
        assert_eq!(state, ShareState::Retrying("the server did not accept this sign-in".into()));
        assert!(state.text().starts_with("not shared yet: "));
    }

    #[test]
    fn failed_download_keys_stay_bounded_once_cooled_and_keep_their_cooldown_before() {
        let (start, cap) = (crate::shared::clock::Instant::now(), std::time::Duration::from_secs(60));
        let mut book = Book::default();
        book.finish_with("up:own", false, start, cap, true, "x");
        for n in 0..(MAX_TRACKED_DOWNLOADS * 3) {
            book.finish_with(&format!("down:{n}"), false, start + std::time::Duration::from_millis(n as u64), cap, true, "no");
        }
        let downs = |b: &Book| b.retry_at.keys().filter(|k| k.starts_with("down:")).count();
        assert_eq!(downs(&book), MAX_TRACKED_DOWNLOADS * 3, "every key is still cooling down: none is forgotten");
        assert!(!book.begin("down:0", start + std::time::Duration::from_secs(1), 99), "the cool-down of an old key holds");
        // once the cool-downs are over the oldest go, the newest stay, the own upload is kept
        let later = start + std::time::Duration::from_secs(600);
        book.finish_with("down:late", false, later, cap, true, "no");
        assert_eq!(downs(&book), MAX_TRACKED_DOWNLOADS);
        assert!(book.why.contains_key("up:own") && !book.retry_at.contains_key("down:0") && book.retry_at.contains_key("down:late"));
    }

    #[test]
    fn downloads_start_at_a_bounded_rate_whatever_ids_are_announced() {
        let (start, cap) = (crate::shared::clock::Instant::now(), std::time::Duration::from_secs(60));
        let mut book = Book::default();
        let mut started = 0;
        for n in 0..100 {
            let key = format!("down:{n}");
            if book.begin_download(&key, start, 1000) {
                started += 1;
                book.finish_with(&key, false, start, cap, true, "no");
            }
        }
        assert_eq!(started, MAX_FETCHES_PER_WINDOW);
        assert!(!book.running.iter().any(|k| k == "down:99") && !book.retry_at.contains_key("down:99"), "a key refused for the rate is not marked failed");
        assert!(book.begin_download("down:99", start + FETCH_WINDOW, 1000), "the next window allows more");
    }

    #[test]
    fn a_failed_fetch_says_why_and_a_plain_404_stays_the_default_words() {
        use std::sync::atomic::Ordering;
        let (id, hostile) = hostile_skin();
        let cfg = offline_host_config();
        let dir = temp_cache("why");
        let (port, _seen, stop) = serve_routes(Vec::new());
        let base = format!("http://127.0.0.1:{port}");
        let why = fetch_skin_why(&cfg, &base, &dir, &id).unwrap_err();
        assert!(!why.is_empty() && !why.contains("127.0.0.1"), "{why}");
        stop.store(true, Ordering::SeqCst);
        let (port, _seen, stop) = serve_routes(vec![(format!("/api/v1/skins/{id}"), hostile, "image/png")]);
        let why = fetch_skin_why(&cfg, &format!("http://127.0.0.1:{port}"), &dir, &id).unwrap_err();
        stop.store(true, Ordering::SeqCst);
        assert_eq!(why, "the server sent a file that is not this skin", "the real reason, not a fixed sentence");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_worn_hash_forgets_the_old_upload_entries() {
        let (a, b) = ("d".repeat(64), "e".repeat(64));
        let mut plan = UploadPlan::default();
        assert!(plan.due(1.0, &a, true, Vec::new, 60.0));
        book().finish_with(&format!("up:{a}"), false, crate::shared::clock::Instant::now(), std::time::Duration::from_secs(60), true, "boom");
        assert!(matches!(share_state(&a, true, false, &[]), ShareState::Retrying(_)));
        assert!(plan.due(2.0, &b, true, Vec::new, 60.0));
        assert_eq!(share_state(&a, true, false, &[]), ShareState::Waiting, "the old skin's failure no longer lingers");
    }

    #[test]
    fn the_uploaded_ids_are_read_once_per_hash_while_the_website_holds_it() {
        let a = "f".repeat(64);
        let mut plan = UploadPlan::default();
        let mut reads = 0;
        for t in 0..5 {
            let due = plan.due(1.0 + 100.0 * t as f64, &a, true, || { reads += 1; vec![a.clone()] }, 60.0);
            assert!(!due);
        }
        assert_eq!(reads, 1);
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
    fn from_url_refuses_what_the_rules_refuse_before_any_connection() {
        let rules = rules().expect("rules load");
        for url in ["http://example.org/a.png", "https://localhost/a.png", "https://127.0.0.1/a.png", "https://user@example.org/a.png", "https://example.org:8443/a.png"] {
            assert!(fetch_url_skin(url, &rules).is_err(), "{url}");
        }
    }

    #[test]
    fn a_cached_skin_is_not_fetched_again() {
        let dir = std::env::temp_dir().join(format!("lr_skins_cache_test_{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (id, bytes) = good_skin();
        assert!(fetch_needed(&dir, &id), "nothing cached yet");
        write_skin_file(&dir, &id, &bytes).unwrap();
        assert!(!fetch_needed(&dir, &id), "a verified cached copy is kept");
        std::fs::write(cached_path(&dir, &id), b"damaged").unwrap();
        assert!(fetch_needed(&dir, &id), "a damaged copy is fetched again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn local(skin: &str, slim: bool, worn: &str) -> LocalSkin {
        LocalSkin { skin: skin.to_string(), slim, worn_hash: worn.to_string() }
    }

    #[test]
    fn sign_in_adopts_the_account_skin_over_no_custom_skin_or_a_different_one() {
        let (acct, other) = ("a".repeat(64), "b".repeat(64));
        assert_eq!(sign_in_skin(&acct, "auto", SKIN_DEFAULT, true, false, SKIN_DEFAULT), SignInSkin::Adopt(acct.clone(), false), "no custom skin here");
        assert_eq!(sign_in_skin(&acct, "auto", "CUSTOM", false, true, &other), SignInSkin::Adopt(acct.clone(), true), "a different custom skin is replaced");
        assert_eq!(sign_in_skin(&acct, "classic", SKIN_DEFAULT, true, true, SKIN_DEFAULT), SignInSkin::Adopt(acct.clone(), false), "the account's CLASSIC wins");
        assert_eq!(sign_in_skin(&acct, "slim", SKIN_DEFAULT, true, false, SKIN_DEFAULT), SignInSkin::Adopt(acct, true), "the account's SLIM wins");
    }

    #[test]
    fn sign_in_changes_nothing_for_an_empty_or_an_already_applied_account_skin() {
        let acct = "c".repeat(64);
        assert_eq!(sign_in_skin("", "slim", "CUSTOM", false, false, &acct), SignInSkin::Keep, "the website holds no skin");
        assert_eq!(sign_in_skin("NOT A HASH", "slim", SKIN_DEFAULT, true, false, SKIN_DEFAULT), SignInSkin::Keep, "a non-hash is nothing");
        assert_eq!(sign_in_skin(&acct, "slim", &acct, false, false, SKIN_DEFAULT), SignInSkin::Keep, "the saved hash is already applied");
        assert_eq!(sign_in_skin(&acct, "slim", "CUSTOM", false, false, &acct), SignInSkin::Keep, "the worn hash is already applied");
    }

    #[test]
    fn the_account_arms_decide_slim_and_auto_keeps_the_local_value() {
        assert!(arms_slim("slim", false));
        assert!(!arms_slim("classic", true));
        assert!(arms_slim("auto", true));
        assert!(!arms_slim("auto", false));
        assert!(arms_slim("", true), "an unknown setting keeps the local value");
    }

    #[test]
    fn the_account_skin_answer_is_read_with_its_hash_and_arms() {
        let p: api::Json = serde_json::from_str(r#"{"ok":true,"account":"KYTTU","hash":"ABCDEF","arms":"Slim","updatedAt":1,"versions":[]}"#).unwrap();
        assert_eq!(api::AccountSkin::from_json(&p), api::AccountSkin { hash: "abcdef".into(), arms: "slim".into() });
        let none: api::Json = serde_json::from_str(r#"{"ok":true,"account":"KYTTU","hash":"","arms":"auto"}"#).unwrap();
        assert_eq!(api::AccountSkin::from_json(&none).hash, "");
        assert!(api::account_skin(&BackendConfig::primary(), "A/B").is_err(), "a name that is not a path segment is never sent");
    }

    #[test]
    fn an_adopted_account_skin_is_fetched_checked_and_cached_once() {
        use std::sync::atomic::Ordering;
        let (id, bytes) = good_skin();
        let cfg = offline_host_config();
        let (port, seen, stop) = serve_routes(vec![(format!("/api/v1/skins/{id}"), bytes.clone(), "image/png")]);
        let dir = temp_cache("account_ok");
        let base = format!("http://127.0.0.1:{port}");
        let info = api::AccountSkin { hash: id.clone(), arms: "classic".into() };
        let first = sign_in_skin_fetching(&info, &local(SKIN_DEFAULT, true, SKIN_DEFAULT), &cfg, &base, Some(&dir));
        assert_eq!(first, Ok(SignInSkin::Adopt(id.clone(), false)));
        assert_eq!(std::fs::read(cached_path(&dir, &id)).unwrap(), bytes, "the verified picture is cached under its hash");
        let again = sign_in_skin_fetching(&info, &local(SKIN_DEFAULT, true, SKIN_DEFAULT), &cfg, &base, Some(&dir));
        assert_eq!(again, Ok(SignInSkin::Adopt(id.clone(), false)));
        stop.store(true, Ordering::SeqCst);
        assert_eq!(seen.lock().unwrap().len(), 1, "a held picture is not asked for again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_account_skin_picture_is_refused_and_changes_nothing() {
        use std::sync::atomic::Ordering;
        let (id, _good) = good_skin();
        let (_hid, hostile) = hostile_skin();
        let cfg = offline_host_config();
        let (port, _seen, stop) = serve_routes(vec![
            (format!("/api/v1/skins/{id}"), hostile.clone(), "image/png"),
            (format!("/api/skins/files/{id}.png"), hostile, "image/png"),
        ]);
        let dir = temp_cache("account_bad");
        let info = api::AccountSkin { hash: id.clone(), arms: "slim".into() };
        let local_state = local(SKIN_DEFAULT, false, SKIN_DEFAULT);
        let got = sign_in_skin_fetching(&info, &local_state, &cfg, &format!("http://127.0.0.1:{port}"), Some(&dir));
        stop.store(true, Ordering::SeqCst);
        assert!(got.is_err(), "a picture that is not the account's skin is refused");
        assert!(!cached_path(&dir, &id).exists(), "nothing reaches the cache");
        assert_eq!(local_state, local(SKIN_DEFAULT, false, SKIN_DEFAULT), "the local state is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_adopted_skin_kept_under_its_hash_is_worn_by_that_hash() {
        let skins = temp_dir("adopted_skins");
        let cache = temp_dir("adopted_cache");
        let (id, bytes) = good_skin();
        write_skin_file(&cache, &id, &bytes).unwrap();
        let w = worn_skin_in(Some(&skins), Some(&cache), &id, true, ArmsSetting::Auto);
        assert_eq!(w.hash, id, "the adopted hash is worn from the cache");
        assert!(!w.slim, "the layout of the picture is read from it");
        assert_eq!(worn_skin_in(Some(&skins), Some(&cache), &"e".repeat(64), false, ArmsSetting::Auto).hash, SKIN_DEFAULT, "a hash not held is DEFAULT");
        let _ = std::fs::remove_dir_all(&skins);
        let _ = std::fs::remove_dir_all(&cache);
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
        assert!(url_refusal("https://example.com/skin.png", &r, &["skins.example.net"]).is_some(), "a host list narrows it");
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
        assert_eq!(c.upload_max_bytes, 64 * 1024, "the website's SKIN_MAX_BYTES (lr_server skins.rs, skins.mjs)");
        assert_eq!(c.upload_max_bytes, skin_validate::SKIN_MAX_BYTES, "the client and the server share one cap");
        assert_eq!(c.png_max_chunks, skin_validate::DEFAULT_MAX_CHUNKS, "the client and the server share the chunk limit");
        assert_eq!(c.upload_retry_secs, 60.0, "RETRY_SECONDS (game/autoload/skin_sync.gd:18)");
        assert_eq!(c.max_bytes, 65536, "PlayerSkin.MAX_BYTES (game/player/player_skin.gd:16), not the old 16 KB");
        assert_eq!(read_limit(), 65537, "FETCH_LIMIT_BYTES = MAX_BYTES + 1 (game/autoload/skin_sync.gd:23)");
        assert_eq!(c.host_base, "https://skins.kyttuslab.org");
        assert_eq!(c.host_path, "/{id}.png");
        assert_eq!(c.backend_path, "/api/skins/files/{id}.png", "game/autoload/skin_sync.gd:152");
        assert_eq!(c.v1_path, "/api/v1/skins/{id}", "lr_server's own skin route");
        assert_eq!(c.retry_secs, 60, "skin_sync.gd RETRY_SECONDS");
        assert_eq!(c.max_concurrent, 4);
        assert_eq!(c.cache_max, 16, "Godot PEER_CACHE_MAX");
        assert_eq!(c.tmp_stale_secs, 60, "a download in flight is never pruned inside its timeout");
        assert!(c.tmp_stale_secs > c.timeout_secs, "the temp file of a running download is younger than the prune limit");
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
        let err = validate_skin_with(&noisy, 65536, 16 * 1024).unwrap_err();
        assert!(err.contains("too detailed"), "{err}");
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
        let dir = std::env::temp_dir().join(format!("lr_skins_{tag}_{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
        assert_eq!(download_error_text(&NetError::Timeout), "the download timed out");
        assert!(download_error_text(&NetError::Transport("tls handshake failed".into())).contains("secure connection"));
        assert!(download_error_text(&NetError::Transport("refused".into())).contains("could not connect"));
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

    #[test]
    fn from_url_validates_what_the_getter_returns_and_asks_for_a_picture() {
        let png = fixture_png(true);
        let mut types = Vec::new();
        let mut get = |url: &str, hosts: &[&str], accept: &[&str]| -> Result<Vec<u8>, String> {
            assert_eq!(url, "https://example.org/skin.png");
            assert!(hosts.is_empty(), "a player's link may be any public host");
            types = accept.iter().map(|t| t.to_string()).collect();
            Ok(png.clone())
        };
        let valid = fetch_url_skin_via("https://example.org/skin.png", &mut get).expect("a valid skin");
        assert!(valid.slim && skin_validate::check_skin_png(&valid.png, max_bytes()).is_ok());
        assert!(types.iter().any(|t| t == "image/png"));
        // a hostile body (header only) is refused, and a failed download keeps its words
        let mut hostile = |_: &str, _: &[&str], _: &[&str]| Ok(png_head(64, 64));
        assert!(fetch_url_skin_via("https://example.org/a.png", &mut hostile).is_err());
        let mut down = |_: &str, _: &[&str], _: &[&str]| Err("the download timed out".to_string());
        assert_eq!(fetch_url_skin_via("https://example.org/a.png", &mut down).unwrap_err(), "the download timed out");
    }

    #[test]
    fn the_cache_prune_leaves_a_download_in_flight_alone_and_removes_stale_temp_files() {
        let dir = temp_dir("prune_tmp");
        let (young, old) = (format!("{:064x}", 1), format!("{:064x}", 2));
        std::fs::write(dir.join(format!("{young}.tmp")), b"half").unwrap();
        let old_tmp = dir.join(format!("{old}.tmp"));
        std::fs::write(&old_tmp, b"half").unwrap();
        let long_ago = crate::shared::clock::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&old_tmp).unwrap().set_modified(long_ago).unwrap();
        let none = std::collections::HashSet::new();
        let removed = prune_cache_with(&dir, 16, std::time::Duration::from_secs(60), &none);
        assert_eq!(removed, 1);
        assert!(dir.join(format!("{young}.tmp")).exists(), "a running download's temp file survives a prune on another thread");
        assert!(!old_tmp.exists(), "a leftover older than the limit is removed");
        // the download that was running can still finish its rename
        let (id, bytes) = good_skin();
        std::fs::write(dir.join(format!("{id}.tmp")), &bytes).unwrap();
        prune_cache_with(&dir, 16, std::time::Duration::from_secs(60), &none);
        assert!(std::fs::rename(dir.join(format!("{id}.tmp")), cached_path(&dir, &id)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_skin_in_use_is_not_evicted_by_the_age_rule() {
        let dir = temp_dir("prune_use");
        let skins: Vec<(String, Vec<u8>)> = (0..4u8)
            .map(|i| {
                let mut px = fixture_rgba(false);
                px[0] = i;
                let png = encode_png(&px, 64, 64);
                (sha256_hex(&png), png)
            })
            .collect();
        for (i, (id, png)) in skins.iter().enumerate() {
            write_skin_file(&dir, id, png).unwrap();
            let when = crate::shared::clock::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i as u64);
            std::fs::File::options().write(true).open(cached_path(&dir, id)).unwrap().set_modified(when).unwrap();
        }
        let protected: std::collections::HashSet<String> = [skins[0].0.clone()].into();
        prune_cache_with(&dir, 2, std::time::Duration::from_secs(60), &protected);
        assert!(cached_path(&dir, &skins[0].0).exists(), "the oldest file is in use: kept");
        assert!(!cached_path(&dir, &skins[1].0).exists());
        assert!(!cached_path(&dir, &skins[2].0).exists());
        assert!(cached_path(&dir, &skins[3].0).exists(), "the newest stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_runners_own_skin_survives_a_pruned_cache_and_stays_readable_and_uploadable() {
        let skins_dir = temp_dir("own_skins");
        let cache = temp_dir("own_cache");
        let png = fixture_png(false);
        std::fs::write(skins_dir.join("mine.png"), &png).unwrap();
        let worn = worn_skin_in(Some(&skins_dir), Some(&cache), "MINE", false, ArmsSetting::Auto);
        assert!(is_skin_hash(&worn.hash));
        assert!(cached_path(&cache, &worn.hash).exists(), "the cache copy is written");
        // 20 peer skins arrive and the prune keeps 16: the cache copy of the own skin is the oldest
        std::fs::File::options().write(true).open(cached_path(&cache, &worn.hash)).unwrap().set_modified(crate::shared::clock::UNIX_EPOCH).unwrap();
        for i in 0..20u8 {
            let mut px = fixture_rgba(false);
            px[1] = i + 1;
            let peer = encode_png(&px, 64, 64);
            write_skin_file(&cache, &sha256_hex(&peer), &peer).unwrap();
        }
        prune_cache_with(&cache, 16, std::time::Duration::from_secs(60), &std::collections::HashSet::new());
        assert!(!cached_path(&cache, &worn.hash).exists(), "the disk copy was evicted");
        let found = read_skin_in(&[None, Some(cache.clone())], &worn.hash).expect("the own skin is still found, from memory");
        assert!(verify_skin(&worn.hash, &found));
        // it is not handed out for another id, and a damaged memory copy would not verify
        assert_eq!(read_skin_in(&[None, Some(cache.clone())], &sha256_hex(b"other")), None);
        let _ = std::fs::remove_dir_all(&skins_dir);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn the_worn_arm_style_comes_from_the_validated_file_except_for_the_custom_file() {
        let skins_dir = temp_dir("worn_slim");
        // a slim picture under a folder name, and the same picture saved as CUSTOM (whose layout settings remember)
        std::fs::write(skins_dir.join("alex.png"), fixture_png(true)).unwrap();
        std::fs::write(skins_dir.join("custom.png"), fixture_png(true)).unwrap();
        // settings.json says skin_slim=false (an old or hand-edited save): the file's own layout still wins for ALEX
        let alex = worn_skin_in(Some(&skins_dir), None, "ALEX", false, ArmsSetting::Auto);
        assert!(alex.slim, "a slim folder skin draws slim arms in AUTO whatever the saved flag says");
        let alex_forced = worn_skin_in(Some(&skins_dir), None, "ALEX", true, ArmsSetting::Classic);
        assert!(!alex_forced.slim, "CLASSIC still forces classic");
        // CUSTOM trusts the layout read when it was picked
        let custom_false = worn_skin_in(Some(&skins_dir), None, "CUSTOM", false, ArmsSetting::Auto);
        let custom_true = worn_skin_in(Some(&skins_dir), None, "CUSTOM", true, ArmsSetting::Auto);
        assert!(!custom_false.slim && custom_true.slim);
        // a classic folder skin is classic even when the saved flag says slim
        std::fs::write(skins_dir.join("steve.png"), fixture_png(false)).unwrap();
        assert!(!worn_skin_in(Some(&skins_dir), None, "STEVE", true, ArmsSetting::Auto).slim);
        let _ = std::fs::remove_dir_all(&skins_dir);
    }

    #[test]
    fn with_no_record_file_the_session_knows_what_the_website_took_so_a_web_skin_is_announced_and_uploaded_once() {
        // the browser build: no user folder, so no skins-uploaded.json
        let known: &'static Checked = Box::leak(Box::default());
        let (a, b) = (sha256_hex(b"web a"), sha256_hex(b"web b"));
        assert!(uploaded_ids_for(None, "https://site", known).is_empty(), "nothing taken yet: DEFAULT is announced");
        let mut plan = UploadPlan::default();
        let ids = || uploaded_ids_for(None, "https://site", known);
        assert!(plan.due(1.0, &a, true, ids, 60.0), "the first offer");
        remember_uploaded_in(None, "https://site", known, &a);
        assert_eq!(uploaded_ids_for(None, "https://site", known), vec![a.clone()], "taken: announced to the room");
        for minute in 2..30 {
            assert!(!plan.due(60.0 * minute as f64, &a, true, ids, 60.0), "never offered again after the website took it (minute {minute})");
        }
        assert_eq!(crate::net::world_sync::published_skin(&a, &ids()), a);
        // a second skin replaces the first, as on the website
        remember_uploaded_in(None, "https://site", known, &b);
        assert_eq!(uploaded_ids_for(None, "https://site", known), vec![b.clone()]);
        assert_eq!(crate::net::world_sync::published_skin(&a, &ids()), SKIN_DEFAULT, "the first is not relied on any more");
    }

    #[test]
    fn only_the_latest_upload_counts_as_held_because_the_website_keeps_one_skin_per_account() {
        let dir = temp_dir("uploaded_list");
        let path = dir.join("skins-uploaded.json");
        let (a, b) = (sha256_hex(b"a"), sha256_hex(b"b"));
        let held = || read_record_at(&path, "http://s");
        write_record_at(&path, "http://s", &[a.clone()]);
        assert!(holds(&held(), &a));
        write_record_at(&path, "http://s", &[b.clone()]);
        assert!(holds(&held(), &b));
        assert!(!holds(&held(), &a), "after A then B the website no longer relies on A, so choosing A again uploads it again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recorded_skin_the_server_dropped_is_reuploaded_and_announced_as_default_meanwhile() {
        use crate::net::world_sync::published_skin;
        use std::sync::atomic::Ordering;
        let (id, bytes) = good_skin();
        let dir = temp_dir("verify_held");
        let path = dir.join("skins-uploaded.json");
        let cfg = shipped_config();
        // 404: the server answers nothing for the hash
        let (port, seen, stop) = serve_routes(Vec::new());
        let origin = format!("http://127.0.0.1:{port}");
        write_record_at(&path, &origin, &[id.clone()]);
        let known: &'static Checked = Box::leak(Box::default());
        let ids = || uploaded_ids_in(&path, &origin, known);
        assert!(!verify_held_in(&id, origin.clone(), path.clone(), cfg.clone(), known), "unsettled while the check runs");
        let settled = |known: &Checked| { for _ in 0..200 { if lock_checked(known).contains_key(&id) { return true; } std::thread::sleep(std::time::Duration::from_millis(25)); } false };
        assert!(settled(known));
        assert_eq!(
            seen.lock().unwrap().clone(),
            vec![format!("GET /api/v1/skins/{id} HTTP/1.1"), format!("GET /api/skins/files/{id}.png HTTP/1.1")],
            "both routes are asked before the skin counts as missing"
        );
        assert!(read_record_at(&path, &origin).is_empty(), "the stale record is dropped");
        assert!(verify_held_in(&id, origin.clone(), path.clone(), cfg.clone(), known), "settled, so the skin is offered");
        let mut plan = UploadPlan::default();
        assert!(plan.due(1.0, &id, true, ids, 60.0), "the canonical PNG is offered again");
        assert_eq!(published_skin(&id, &ids()), SKIN_DEFAULT, "DEFAULT until the re-upload");
        write_record_at(&path, &origin, &[id.clone()]);
        lock_checked(known).insert(id.clone(), true);
        assert_eq!(published_skin(&id, &ids()), id, "announced after the re-upload");
        // no origin: nothing to ask, the record is trusted
        write_record_at(&path, "", &[id.clone()]);
        let fresh: &'static Checked = Box::leak(Box::default());
        assert!(verify_held_in(&id, String::new(), path.clone(), cfg.clone(), fresh));
        assert_eq!(uploaded_ids_in(&path, "", fresh), vec![id.clone()]);
        // another skin recorded during the slow GET survives the drop of the stale one
        let other = sha256_hex(b"other");
        write_record_at(&path, &origin, &[other.clone()]);
        assert_eq!(verify_held_at(&cfg, &path, &origin, &id), Some(false));
        assert_eq!(read_record_at(&path, &origin), vec![other.clone()]);
        stop.store(true, Ordering::SeqCst);
        // 200: the server still holds it, the record stays
        let (port, _seen, stop) = serve_routes(vec![(format!("/api/v1/skins/{id}"), bytes, "image/png")]);
        let origin = format!("http://127.0.0.1:{port}");
        write_record_at(&path, &origin, &[id.clone()]);
        assert_eq!(verify_held_at(&cfg, &path, &origin, &id), Some(true));
        assert_eq!(read_record_at(&path, &origin), vec![id.clone()]);
        stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_plain_list_still_loads_and_another_servers_record_is_ignored() {
        let dir = temp_dir("record_format");
        let path = dir.join("skins-uploaded.json");
        let a = sha256_hex(b"a");
        std::fs::write(&path, format!("[\"{a}\", \"not a hash\"]")).unwrap();
        assert_eq!(read_record_at(&path, "http://one"), vec![a.clone()], "the old format reads for any server");
        write_record_at(&path, "http://one", &[a.clone()]);
        assert_eq!(read_record_at(&path, "http://one"), vec![a.clone()]);
        assert!(read_record_at(&path, "http://two").is_empty(), "a record of another server is ignored");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clear_with_no_skin_store_on_the_server_still_forgets_the_upload() {
        use crate::net::NetError;
        let http = |status: u16, code: &str| NetError::Http { status, code: code.into(), error: String::new(), body: serde_json::Value::Null };
        assert!(clear_succeeded(&Ok::<(), NetError>(())));
        assert!(clear_succeeded::<()>(&Err(http(503, "SKINS_UNAVAILABLE"))), "nothing to clear there");
        assert!(!clear_succeeded::<()>(&Err(http(503, "UPSTREAM_UNAVAILABLE"))), "a website that is down is retried");
        assert!(!clear_succeeded::<()>(&Err(http(401, "NO_SESSION"))));
        assert!(!clear_succeeded::<()>(&Err(NetError::Timeout)));
    }

    #[test]
    fn a_cached_hostile_file_counts_as_not_cached_and_is_purged() {
        let (good_id, good) = good_skin();
        let (bad_id, bad) = hostile_skin();
        let dir = temp_cache("purge");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(cached_path(&dir, &good_id), &good).unwrap();
        std::fs::write(cached_path(&dir, &bad_id), &bad).unwrap();
        let leftover = dir.join(format!("{}.tmp", "a".repeat(64)));
        std::fs::write(&leftover, b"half").unwrap();
        std::fs::File::options().write(true).open(&leftover).unwrap().set_modified(crate::shared::clock::UNIX_EPOCH).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not ours").unwrap();
        assert!(!fetch_needed(&dir, &good_id));
        assert!(fetch_needed(&dir, &bad_id), "a header-only file with the right hash is not a cached skin");
        assert_eq!(prune_cache(&dir, 16), 2, "the hostile file and the stale half-written one go");
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
            file.set_modified(crate::shared::clock::UNIX_EPOCH + std::time::Duration::from_secs(1000 + n as u64)).unwrap();
            ids.push(id);
        }
        assert_eq!(prune_cache(&dir, 3), 2);
        assert!(!cached_path(&dir, &ids[0]).exists() && !cached_path(&dir, &ids[1]).exists(), "the oldest two are evicted");
        assert!(ids[2..].iter().all(|id| cached_path(&dir, id).exists()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_skins_folder_the_applied_skin_is_worn_from_memory() {
        let valid = validate_skin(&fixture_png(true), max_bytes()).expect("the fixture is a valid skin");
        let before = generation();
        assert_eq!(keep_custom_in_memory(&valid.png), "CUSTOM");
        assert!(generation() > before, "the worn skin is resolved again after an APPLY");
        let worn = worn_skin_in(None, None, "CUSTOM", true, ArmsSetting::Auto);
        assert_eq!(worn.hash, valid.hash, "the browser build has no folder: the memory copy is worn");
        assert!(worn.slim, "the arm layout read at APPLY is kept");
        assert_eq!(load_memory_custom("CUSTOM").map(|v| v.hash), Some(valid.hash.clone()));
        assert!(load_memory_custom("DEFAULT").is_none() && load_memory_custom("OTHER").is_none());
        assert_eq!(worn_skin_in(None, None, "DEFAULT", false, ArmsSetting::Auto).hash, SKIN_DEFAULT);
        *memory_custom() = None;
        assert_eq!(worn_skin_in(None, None, "CUSTOM", false, ArmsSetting::Auto).hash, SKIN_DEFAULT, "nothing kept: DEFAULT");
    }

    #[test]
    fn a_skin_that_was_validated_here_has_a_known_arm_style() {
        let (id, _) = good_skin();
        assert_eq!(skin_is_slim(&id), Some(false), "the classic fixture");
        note_slim(&id, true);
        assert_eq!(skin_is_slim(&id), Some(true));
        assert_eq!(skin_is_slim(&"e".repeat(64)), None, "an unknown picture has no style");
    }

    /// A valid skin that differs per `tag`: the download book and the memory store are process-wide, so tests that run side
    /// by side must not share an id.
    fn tagged_skin(tag: u8) -> (String, Vec<u8>) {
        let mut px = fixture_rgba(false);
        px[px_at(1, 1, 64)] = tag;
        skin_from_png(encode_png(&px, 64, 64)).expect("the fixture is a valid skin")
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..400 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out waiting for: {what}");
    }

    #[test]
    fn a_runner_with_no_cache_folder_fetches_another_runners_skin_into_memory() {
        use std::sync::atomic::Ordering;
        // The browser build has no folders: before this, nothing was ever fetched there and every remote skin stayed DEFAULT.
        let (id, bytes) = tagged_skin(11);
        let (port, seen, stop) = serve_routes(vec![(format!("/api/skins/files/{id}.png"), bytes.clone(), "image/png")]);
        let base = format!("http://127.0.0.1:{port}");
        assert!(read_skin_in(&[None, None], &id).is_none(), "not here yet");
        assert!(start_download(offline_host_config(), base.clone(), None, &id), "a download starts with no cache folder");
        wait_until("the skin is held in memory", || read_skin_in(&[None, None], &id).is_some());
        stop.store(true, Ordering::SeqCst);
        assert_eq!(read_skin_in(&[None, None], &id), Some(bytes.clone()));
        assert!(!start_download(offline_host_config(), base, None, &id), "a skin that is held is not fetched again");
        assert_eq!(skin_is_slim(&id), Some(false), "the arm style is read from the kept picture too");
        let asked = seen.lock().unwrap().clone();
        assert!(asked.iter().all(|line| line.starts_with("GET ")), "a read-only fetch: {asked:?}");
        assert!(remote_skin_rgba(&bytes).is_ok());
    }

    #[test]
    fn a_hostile_id_from_a_profile_never_reaches_a_file_path_or_the_network() {
        use std::sync::atomic::Ordering;
        let (port, seen, stop) = serve_routes(Vec::new());
        let base = format!("http://127.0.0.1:{port}");
        let dir = temp_dir("hostile_id");
        let upper = tagged_skin(51).0.to_ascii_uppercase();
        let hostile = [
            "../../etc/passwd".to_string(),
            "..\\..\\secret".to_string(),
            format!("{}/../x", "a".repeat(60)),
            "a".repeat(5000),
            "a".repeat(63),
            upper,
            String::new(),
        ];
        for id in &hostile {
            assert!(!start_download(offline_host_config(), base.clone(), Some(dir.clone()), id), "refused: {:.30}", id);
            assert!(!start_download(offline_host_config(), base.clone(), None, id), "refused in memory: {:.30}", id);
            assert!(!book().running.contains(&format!("down:{id}")) && !book().retry_at.contains_key(&format!("down:{id}")), "not even booked");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        stop.store(true, Ordering::SeqCst);
        assert!(seen.lock().unwrap().is_empty(), "nothing was asked of the server: {:?}", seen.lock().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_fetch_cools_down_then_retries_and_never_leaves_the_skin_stuck() {
        use std::sync::atomic::Ordering;
        let (id, bytes) = tagged_skin(12);
        let cfg = offline_host_config();
        let key = format!("down:{id}");
        // the server does not hold the skin yet
        let (empty_port, _seen, stop_empty) = serve_routes(Vec::new());
        let empty = format!("http://127.0.0.1:{empty_port}");
        assert!(start_download(cfg.clone(), empty.clone(), None, &id));
        wait_until("the failure is booked", || {
            let b = book();
            b.fails.get(&key) == Some(&1) && !b.running.contains(&key)
        });
        stop_empty.store(true, Ordering::SeqCst);
        assert!(read_skin_in(&[None], &id).is_none(), "nothing to draw: the default skin shows meanwhile");
        assert!(!start_download(cfg.clone(), empty, None, &id), "cooling down: not asked again at once");
        let wait = book().retry_at[&key].saturating_duration_since(crate::shared::clock::Instant::now());
        assert!(wait > std::time::Duration::from_secs(2) && wait <= std::time::Duration::from_secs(5), "the first wait is short: {wait:?}");
        assert!(book().why.get(&key).is_some_and(|why| !why.is_empty()), "the player can be told why");
        // the cool-down ends, and the server holds the skin by now
        book().retry_at.insert(key.clone(), crate::shared::clock::Instant::now());
        let (port, _seen, stop) = serve_routes(vec![(format!("/api/skins/files/{id}.png"), bytes.clone(), "image/png")]);
        assert!(start_download(cfg, format!("http://127.0.0.1:{port}"), None, &id), "the retry starts");
        wait_until("the retry brings the skin", || read_skin_in(&[None], &id).is_some());
        stop.store(true, Ordering::SeqCst);
        wait_until("the book forgets the failure", || {
            let b = book();
            !b.fails.contains_key(&key) && !b.running.contains(&key)
        });
        assert_eq!(read_skin_in(&[None], &id), Some(bytes));
    }

    #[test]
    fn the_skins_kept_in_memory_for_other_runners_are_bounded_and_the_newest_stay() {
        let mut list = Vec::new();
        for tag in 0..5u8 {
            keep_newest(&mut list, 3, &format!("id{tag}"), &[tag]);
        }
        assert_eq!(list.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec!["id2", "id3", "id4"]);
        keep_newest(&mut list, 3, "id2", &[9]);
        assert_eq!(list.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec!["id3", "id4", "id2"], "a skin seen again is the newest");
        assert_eq!(list[2].1, vec![9]);
    }

    #[test]
    fn without_a_record_file_the_skin_the_server_holds_this_session_is_published() {
        use crate::net::world_sync::published_skin;
        // The browser keeps no skins-uploaded.json: before this its uploaded list was always empty, so a signed-in runner in the
        // browser announced DEFAULT to the room whatever the website held.
        let (a, _) = tagged_skin(21);
        let (b, _) = tagged_skin(22);
        let known: Checked = Default::default();
        assert!(uploaded_ids_for(None, "http://s", &known).is_empty());
        assert_eq!(published_skin(&a, &uploaded_ids_for(None, "http://s", &known)), SKIN_DEFAULT, "DEFAULT until the upload is accepted");
        remember_uploaded_in(None, "http://s", &known, &a);
        assert_eq!(published_skin(&a, &uploaded_ids_for(None, "http://s", &known)), a, "announced once the website holds it");
        remember_uploaded_in(None, "http://s", &known, &b);
        let held = uploaded_ids_for(None, "http://s", &known);
        assert!(holds(&held, &b) && !holds(&held, &a), "one skin per account, as in the record file");
        let mut plan = UploadPlan::default();
        assert!(!plan.due(1.0, &b, true, || held.clone(), 60.0), "a skin the website holds is not uploaded again");
        lock_checked(&known).insert(b.clone(), false);
        assert!(uploaded_ids_for(None, "http://s", &known).is_empty(), "a skin found missing is not announced");
    }

    #[test]
    fn a_server_that_has_only_the_legacy_route_still_counts_as_holding_a_recorded_skin() {
        use std::sync::atomic::Ordering;
        let (id, bytes) = tagged_skin(31);
        let dir = temp_dir("verify_legacy");
        let path = dir.join("skins-uploaded.json");
        let (port, _seen, stop) = serve_routes(vec![(format!("/api/skins/files/{id}.png"), bytes, "image/png")]);
        let origin = format!("http://127.0.0.1:{port}");
        write_record_at(&path, &origin, &[id.clone()]);
        assert_eq!(verify_held_at(&shipped_config(), &path, &origin, &id), Some(true));
        assert_eq!(read_record_at(&path, &origin), vec![id], "the record stays: no upload on every start");
        stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_upload_to_the_node_backend_is_recognised_by_its_hash_answer() {
        use std::sync::atomic::Ordering;
        // The Node backend's POST /api/skins/upload answers {ok, hash} with no id (lr_server sends both). The client read only
        // "id", so every upload there looked like it stored another skin: it was never recorded and never announced.
        let (id, bytes) = tagged_skin(41);
        let answer = format!(r#"{{"ok":true,"hash":"{id}"}}"#).into_bytes();
        let (port, seen, stop) = serve_routes(vec![("/api/skins/upload".to_string(), answer, "application/json")]);
        let cfg = BackendConfig::loopback(port);
        assert_eq!(api::upload_skin(&cfg, "token", &bytes).as_deref(), Ok(id.as_str()));
        stop.store(true, Ordering::SeqCst);
        assert_eq!(seen.lock().unwrap().clone(), vec!["POST /api/v1/skins HTTP/1.1".to_string(), "POST /api/skins/upload HTTP/1.1".to_string()]);
    }
}
