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

/// True when the first bytes of a file are a PNG signature and an IHDR of exactly 64x64.
/// Only the header is read; the pixels are not decoded.
pub fn png_is_skin_size(head: &[u8]) -> bool {
    if head.len() < 24 || head[..8] != PNG_SIGNATURE || &head[12..16] != b"IHDR" {
        return false;
    }
    let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    width == SKIN_SIZE && height == SKIN_SIZE
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
    if bytes.len() > api::SKIN_MAX_BYTES || !png_is_skin_size(&bytes) {
        return None;
    }
    let hash = sha256_hex(&bytes);
    Some((hash, bytes))
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

/// The URL a skin is fetched from: `GET <base>/api/v1/skins/<id>`.
pub fn skin_url(base: &str, id: &str) -> String {
    format!("{}/api/v1/skins/{id}", base.trim_end_matches('/'))
}

/// The skins host, tried first when fetching. Skin files are `<id>.png` and never change, so the host serves them forever.
pub const SKIN_HOST_BASE: &str = "https://skins.kyttuslab.org";

/// The URL a skin is fetched from on the skins host: `GET <SKIN_HOST_BASE>/<id>.png`.
pub fn skin_host_url(id: &str) -> String {
    format!("{}/{id}.png", SKIN_HOST_BASE.trim_end_matches('/'))
}

/// The URLs a skin is tried at, in order: the skins host first, then the backend route ([`skin_url`]).
pub fn fetch_urls(backend_base: &str, id: &str) -> [String; 2] {
    [skin_host_url(id), skin_url(backend_base, id)]
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

/// Sets the base URL skins are fetched from (`GET <base>/api/v1/skins/<id>`). Only the first call counts.
pub fn set_backend_base(base: &str) {
    let _ = BACKEND_BASE.set(base.trim_end_matches('/').to_string());
}

/// Starts a background fetch of skin `id` into the cache folder. It never blocks the frame; the file shows up on a later
/// check once it has been downloaded and verified. The skins host is tried first, then the backend route. Does nothing
/// when no backend is set (fetching is off), the id is not a skin hash, or the skin is already cached.
pub fn fetch_skin_in_background(id: &str) {
    let Some(base) = BACKEND_BASE.get().filter(|b| !b.is_empty()).cloned() else { return };
    let Some(dir) = cache_dir() else { return };
    if !is_skin_hash(id) || !fetch_needed(&dir, id) {
        return;
    }
    let id = id.to_string();
    std::thread::spawn(move || {
        let urls = fetch_urls(&base, &id);
        let fetched = first_verified_skin(&urls, &id, |url| {
            let reply = Request::new(Method::Get, url.to_string())
                .timeout(std::time::Duration::from_secs(8))
                .limit(64 * 1024)
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
    if uploaded_ids().contains(&hash) {
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

    #[test]
    fn fetch_url_and_cache_file_name_use_the_id() {
        let id = sha256_hex(b"skin");
        assert_eq!(skin_url("http://host:8105/", &id), format!("http://host:8105/api/v1/skins/{id}"));
        let dir = Path::new("cache");
        assert_eq!(cached_path(dir, &id), dir.join(format!("{id}.png")));
    }

    #[test]
    fn skins_host_url_and_file_name_use_the_id() {
        let id = sha256_hex(b"skin");
        assert_eq!(SKIN_HOST_BASE, "https://skins.kyttuslab.org");
        assert_eq!(skin_host_url(&id), format!("https://skins.kyttuslab.org/{id}.png"));
    }

    #[test]
    fn skins_host_is_tried_before_the_backend_route() {
        let id = sha256_hex(b"skin");
        let urls = fetch_urls("http://host:8105/", &id);
        assert_eq!(urls[0], skin_host_url(&id));
        assert_eq!(urls[1], skin_url("http://host:8105/", &id));
    }

    #[test]
    fn fetch_uses_the_host_when_it_answers_with_a_verified_skin() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let urls = fetch_urls("http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            Some(bytes.clone())
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, vec![skin_host_url(&id)], "the backend is not asked once the host answers");
    }

    #[test]
    fn a_failing_host_falls_back_to_the_backend() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let urls = fetch_urls("http://host:8105", &id);
        let mut asked = Vec::new();
        let got = first_verified_skin(&urls, &id, |url| {
            asked.push(url.to_string());
            if url.starts_with(SKIN_HOST_BASE) {
                None
            } else {
                Some(bytes.clone())
            }
        });
        assert_eq!(got, Some(bytes));
        assert_eq!(asked, vec![skin_host_url(&id), skin_url("http://host:8105", &id)]);
    }

    #[test]
    fn a_host_reply_that_does_not_verify_falls_back_to_the_backend() {
        let bytes = png_head(64, 64);
        let id = sha256_hex(&bytes);
        let urls = fetch_urls("http://host:8105", &id);
        let got = first_verified_skin(&urls, &id, |url| {
            if url.starts_with(SKIN_HOST_BASE) {
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
        let urls = fetch_urls("http://host:8105", &id);
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
}
