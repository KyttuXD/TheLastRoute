//! Port of the skin routes of `backend/skins.mjs` (the website side lives in KyttusLab).
//!
//!   POST /api/v1/skins          body {sessionToken, png: base64 PNG}  -> {ok, id, hash}
//!   POST /api/skins/upload      raw PNG body, header x-session-token  -> {ok, id, hash}   (the Godot backend route)
//!   GET  /api/v1/skins/<id>     -> image/png, or 404 SKIN_NOT_FOUND
//!   GET  /api/skins/files/<id>.png                                    (the Godot backend route, same answer)
//!   POST /api/v1/skins/mine | /clear | /remove   (also /api/skins/mine | /clear | /remove, as on the Godot backend)
//!
//! A skin is a 64x64 PNG of at most 64 KB (`SKIN_MAX_BYTES`, the Godot `SKIN_MAX_BYTES`). Its id is the lowercase hex
//! SHA-256 of the PNG bytes, so an id names exactly one picture. Every upload is checked with the full validator the
//! game uses (`last_route_core::shared::skin_validate`: every chunk CRC, 64x64, a real inflate of exactly the promised
//! size, an opaque base layer), so a header that only looks right never reaches the store, and a skin that would crash
//! or hide a runner on another machine is refused here, before anyone can fetch it.
//!
//! With `LASTROUTE_ACCOUNT_UPSTREAM` set, the website owns the accounts and the skins, so upload and read are
//! forwarded there (the upload keeps only `sessionToken` and `png`, after the same check). Otherwise skins are stored as
//! `<data dir>/skins/<id>.png` (temp file + rename), the session is checked against the local accounts, and the store
//! keeps the rules of `skins.mjs`:
//!
//! * one skin per account (`index.json` maps account to id); replacing it deletes the old file unless another account
//!   still uses it, and `clear` removes it;
//! * at most 5 uploads per account in 10 minutes (separate from the per-address request limit);
//! * an owner can `remove` a skin by id or by account name: the id is blocked from ever being uploaded again;
//! * a stored picture is served only while it is still the file its name claims, with `nosniff`, a
//!   `default-src 'none'` policy and a year of immutable caching (see `routes.rs`).

use crate::account_service::AccountService;
use crate::upstream::{Upstream, UpstreamError};
use crate::util::{base64, from_base64, log_error, log_warn, sha256_hex, system_clock, write_atomic, Clock, Reply, SessionLookup};
use last_route_core::shared::skin_validate;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const SKIN_SIZE: u32 = skin_validate::SKIN_SIZE;
/// The largest skin PNG stored or served: 64 KB, the same number the game checks (`skins.mjs` `SKIN_MAX_BYTES`).
pub const SKIN_MAX_BYTES: usize = skin_validate::SKIN_MAX_BYTES;
/// The largest JSON body of `POST /api/v1/skins`: the base64 of a full-size skin plus room for the JSON around it.
pub const SKIN_JSON_BODY_BYTES: usize = (SKIN_MAX_BYTES + 2) / 3 * 4 + 1024;
/// Uploads one account may make per window (`skins.mjs` `uploadLimit`).
pub const UPLOAD_LIMIT: usize = 5;
/// The length of that window, in milliseconds (`skins.mjs` `uploadWindowMs`: 10 minutes).
pub const UPLOAD_WINDOW_MS: i64 = 10 * 60 * 1000;
/// The file the owners and the blocked ids are kept in, inside the skin directory.
const INDEX_FILE: &str = "index.json";

pub struct SkinsOptions {
    /// Where skins are stored when there is no upstream (`<data dir>/skins`). None: the skin routes answer 503.
    pub directory: Option<PathBuf>,
    /// `LASTROUTE_ACCOUNT_UPSTREAM`: the website that stores the skins.
    pub upstream: Option<Upstream>,
    /// The clock the upload limit runs on.
    pub now: Clock,
}

#[cfg(test)]
impl SkinsOptions {
    /// Options on the system clock.
    pub fn new(directory: Option<PathBuf>, upstream: Option<Upstream>) -> SkinsOptions {
        SkinsOptions { directory, upstream, now: system_clock() }
    }
}

/// What the local store remembers between requests (and, for owners and blocked ids, between restarts).
#[derive(Default)]
struct Book {
    /// Account name to the id of its skin.
    owners: HashMap<String, String>,
    /// Ids an owner removed: never stored again.
    blocked: HashSet<String>,
    /// Account name to the times of its recent uploads.
    uploads: HashMap<String, Vec<i64>>,
}

/// Who may call the account routes: a session check, and the full-admin check `remove` needs.
pub trait SkinAccounts: SessionLookup {
    fn is_owner(&self, username: &str) -> bool;
}

impl SkinAccounts for AccountService {
    fn is_owner(&self, username: &str) -> bool {
        self.is_admin_owner(username)
    }
}

/// The account routes: `mine`, `clear` and `remove`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinAction {
    Mine,
    Clear,
    Remove,
}

impl SkinAction {
    /// The action a request path names, on either prefix (`/api/v1/skins/<x>` or the Godot `/api/skins/<x>`).
    pub fn from_path(path: &str) -> Option<SkinAction> {
        match path {
            "/api/v1/skins/mine" | "/api/skins/mine" => Some(SkinAction::Mine),
            "/api/v1/skins/clear" | "/api/skins/clear" => Some(SkinAction::Clear),
            "/api/v1/skins/remove" | "/api/skins/remove" => Some(SkinAction::Remove),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Skins {
    directory: Option<PathBuf>,
    upstream: Option<Upstream>,
    now: Clock,
    book: Arc<Mutex<Book>>,
}

fn unavailable() -> Reply {
    (503, json!({ "ok": false, "code": "SKINS_UNAVAILABLE", "error": "Skins are not available on this server" }))
}

fn bad_skin(reason: &str) -> Reply {
    (400, json!({ "ok": false, "code": "BAD_SKIN", "error": reason }))
}

fn skin_not_found() -> Reply {
    (404, json!({ "ok": false, "code": "SKIN_NOT_FOUND", "error": "Skin not found" }))
}

fn not_signed_in(error: &str) -> Reply {
    (401, json!({ "ok": false, "code": "NO_SESSION", "error": error }))
}

/// The website's reply for a failed forward, as `forwardFailure` maps it in the account routes.
fn upstream_failure(error: UpstreamError) -> Reply {
    match error {
        UpstreamError::Unavailable(detail) => {
            log_warn(&format!("skin upstream unavailable: {detail}"));
            (503, json!({ "ok": false, "code": "UPSTREAM_UNAVAILABLE", "error": "Accounts are unavailable right now; try again shortly" }))
        }
        UpstreamError::BadReply(detail) => {
            log_warn(&format!("skin upstream sent a bad reply: {detail}"));
            (502, json!({ "ok": false, "code": "UPSTREAM_BAD_REPLY", "error": "The account service sent an unusable reply" }))
        }
    }
}

/// `skinIdFor(bytes)`: the lowercase hex SHA-256 of the PNG bytes.
pub fn skin_id_for(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

/// `SKIN_ID_PATTERN`: exactly 64 lowercase hex characters.
pub fn is_skin_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Strict base64: standard alphabet, padding only at the end, a whole number of quads, and a round trip back to the same
/// text (the lenient decoder would otherwise accept malformed padding). Refused above the byte cap before decoding.
pub fn decode_skin_base64(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() % 4 != 0 || text.len() > (SKIN_MAX_BYTES + 2) / 3 * 4 {
        return None;
    }
    let data = text.trim_end_matches('=');
    if text.len() - data.len() > 2 || !data.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/') {
        return None;
    }
    let bytes = from_base64(text);
    if bytes.is_empty() || bytes.len() > SKIN_MAX_BYTES || base64(&bytes) != text {
        return None;
    }
    Some(bytes)
}

/// `skinProblem(bytes)`: None when the bytes are a skin the game would accept, else the reason. This is the same full
/// check the game runs on every download (`skin_validate::check_skin_png`), not just a look at the header.
pub fn skin_problem(bytes: &[u8]) -> Option<String> {
    let reason = skin_validate::check_skin_png(bytes, SKIN_MAX_BYTES).err()?;
    Some(match reason.as_str() {
        "the file is empty" => "Skin PNG is empty".to_string(),
        "the file is too big" => format!("Skin PNG must be {} KB or smaller", SKIN_MAX_BYTES / 1024),
        "that is not a PNG file" => "Skin must be a PNG file".to_string(),
        "a skin must be 64x64 pixels" => format!("Skin PNG must be {SKIN_SIZE}x{SKIN_SIZE}"),
        other => format!("Skin PNG refused: {other}"),
    })
}

fn too_many_uploads() -> Reply {
    (429, json!({ "ok": false, "code": "UPLOAD_RATE_LIMIT", "error": "Too many skin uploads; try again later" }))
}

impl Skins {
    pub fn new(options: SkinsOptions) -> Skins {
        let book = options.directory.as_deref().map(load_index).unwrap_or_default();
        Skins { directory: options.directory, upstream: options.upstream, now: options.now, book: Arc::new(Mutex::new(book)) }
    }

    /// True when the website owns the skins (uploads and reads are forwarded to it).
    pub fn is_upstream(&self) -> bool {
        self.upstream.is_some()
    }

    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn persist(&self, book: &Book) {
        let Some(directory) = &self.directory else { return };
        let mut owners: Vec<(&String, &String)> = book.owners.iter().collect();
        owners.sort();
        let mut blocked: Vec<&String> = book.blocked.iter().collect();
        blocked.sort();
        let owners: serde_json::Map<String, Value> = owners.into_iter().map(|(who, id)| (who.clone(), json!(id))).collect();
        let text = json!({ "owners": owners, "blocked": blocked }).to_string();
        if let Err(error) = write_atomic(&directory.join(INDEX_FILE), text.as_bytes()) {
            log_error(&format!("[KYTTU SKIN] could not save the skin index: {error}"));
        }
    }

    /// POST /api/v1/skins.
    pub fn upload(&self, accounts: &dyn SessionLookup, body: &Value) -> Reply {
        if let Some(upstream) = &self.upstream {
            // The website stores it, but a picture that fails the check is never forwarded.
            let Some(bytes) = decode_skin_base64(body.get("png").and_then(Value::as_str).unwrap_or("")) else {
                return bad_skin("png must be base64 of a 64x64 PNG up to 64 KB");
            };
            if let Some(problem) = skin_problem(&bytes) {
                return bad_skin(&problem);
            }
            let forwarded = json!({
                "sessionToken": body.get("sessionToken").cloned().unwrap_or(Value::Null),
                "png": body.get("png").cloned().unwrap_or(Value::Null),
            });
            return upstream.call("/api/v1/skins", &forwarded);
        }
        if self.directory.is_none() {
            return unavailable();
        }
        let token = body.get("sessionToken").and_then(Value::as_str).unwrap_or("");
        let Some(user) = (!token.is_empty()).then(|| accounts.session_username(token)).flatten() else {
            return not_signed_in("Sign in to upload a skin");
        };
        self.upload_as(&user, body.get("png").and_then(Value::as_str).unwrap_or(""))
    }

    /// The local store's upload for the account `user` (already signed in and moderation-checked): `png` is base64.
    pub fn upload_as(&self, user: &str, png: &str) -> Reply {
        let Some(bytes) = decode_skin_base64(png) else {
            return bad_skin("png must be base64 of a 64x64 PNG up to 64 KB");
        };
        self.store(user, &bytes)
    }

    /// `POST /api/skins/upload`: the same upload with the raw PNG as the body (the Godot backend route).
    pub fn upload_raw(&self, accounts: &dyn SessionLookup, token: &str, bytes: &[u8]) -> Reply {
        if self.directory.is_none() {
            return unavailable();
        }
        let Some(user) = (!token.is_empty()).then(|| accounts.session_username(token)).flatten() else {
            return not_signed_in("Sign in to share a skin");
        };
        self.upload_raw_as(&user, bytes)
    }

    /// The raw-body upload for the account `user` (already signed in and moderation-checked).
    pub fn upload_raw_as(&self, user: &str, bytes: &[u8]) -> Reply {
        if self.directory.is_none() {
            return unavailable();
        }
        self.store(user, bytes)
    }

    /// Checks and stores a skin for `user`: the rate limit, the full validation, the block list, then the file and the
    /// account's one slot.
    fn store(&self, user: &str, bytes: &[u8]) -> Reply {
        let Some(directory) = &self.directory else { return unavailable() };
        let now = (self.now)();
        let mut book = self.book();
        let recent = book.uploads.entry(user.to_string()).or_default();
        recent.retain(|at| now - *at < UPLOAD_WINDOW_MS);
        if recent.len() >= UPLOAD_LIMIT {
            return too_many_uploads();
        }
        if let Some(problem) = skin_problem(bytes) {
            return bad_skin(&problem);
        }
        let id = skin_id_for(bytes);
        if book.blocked.contains(&id) {
            return (403, json!({ "ok": false, "code": "SKIN_BLOCKED", "error": "That skin was removed by an owner" }));
        }
        let path = directory.join(format!("{id}.png"));
        if let Err(error) = write_atomic(&path, bytes) {
            log_error(&format!("[KYTTU SKIN] could not store skin {id}: {error}"));
            return (500, json!({ "ok": false, "code": "SKIN_STORE_FAILED", "error": "Could not store skin" }));
        }
        book.uploads.entry(user.to_string()).or_default().push(now);
        let previous = book.owners.insert(user.to_string(), id.clone());
        if let Some(previous) = previous.filter(|old| *old != id) {
            self.drop_file_if_unused(&book, &previous);
        }
        self.persist(&book);
        (200, json!({ "ok": true, "id": id, "hash": id }))
    }

    /// Deletes a skin file when no account holds it any more.
    fn drop_file_if_unused(&self, book: &Book, id: &str) {
        if book.owners.values().any(|owned| owned == id) {
            return;
        }
        if let Some(directory) = &self.directory {
            let _ = std::fs::remove_file(directory.join(format!("{id}.png")));
        }
    }

    /// `mine`, `clear` and `remove`. Only the local store has them: with an upstream the website owns the skins.
    pub fn account_action(&self, accounts: &dyn SkinAccounts, action: SkinAction, body: &Value) -> Reply {
        if self.upstream.is_some() || self.directory.is_none() {
            return unavailable();
        }
        let token = body.get("sessionToken").and_then(Value::as_str).unwrap_or("");
        let user = (!token.is_empty()).then(|| accounts.session_username(token)).flatten();
        match action {
            SkinAction::Mine => match user {
                None => not_signed_in("Sign in first"),
                Some(user) => {
                    let id = self.book().owners.get(&user).cloned().unwrap_or_default();
                    (200, json!({ "ok": true, "id": id, "hash": id }))
                }
            },
            SkinAction::Clear => match user {
                None => not_signed_in("Sign in first"),
                Some(user) => {
                    let mut book = self.book();
                    if let Some(previous) = book.owners.remove(&user) {
                        self.drop_file_if_unused(&book, &previous);
                    }
                    self.persist(&book);
                    (200, json!({ "ok": true }))
                }
            },
            SkinAction::Remove => {
                if !user.as_deref().is_some_and(|name| accounts.is_owner(name)) {
                    return (403, json!({ "ok": false, "code": "ADMIN_OWNER_REQUIRED", "error": "Full admin account required" }));
                }
                let mut book = self.book();
                let mut id = body.get("hash").or_else(|| body.get("id")).and_then(Value::as_str).unwrap_or("").to_lowercase();
                if id.is_empty() {
                    let who = body.get("username").and_then(Value::as_str).unwrap_or("").to_uppercase();
                    id = book.owners.get(&who).cloned().unwrap_or_default();
                }
                if !is_skin_id(&id) {
                    return skin_not_found();
                }
                book.owners.retain(|_, owned| *owned != id);
                book.blocked.insert(id.clone());
                if let Some(directory) = &self.directory {
                    let _ = std::fs::remove_file(directory.join(format!("{id}.png")));
                }
                self.persist(&book);
                (200, json!({ "ok": true, "id": id, "hash": id, "blocked": true }))
            }
        }
    }

    /// GET /api/v1/skins/<id>: the PNG bytes, or the reply to send instead (an error with its JSON).
    pub fn read(&self, id: &str) -> Result<Vec<u8>, Reply> {
        if !is_skin_id(id) {
            return Err((400, json!({ "ok": false, "code": "BAD_SKIN_ID", "error": "Skin id must be 64 lowercase hex characters" })));
        }
        if let Some(upstream) = &self.upstream {
            return read_upstream(upstream, id);
        }
        let Some(directory) = &self.directory else { return Err(unavailable()) };
        if self.book().blocked.contains(id) {
            return Err(skin_not_found());
        }
        let path = directory.join(format!("{id}.png"));
        // Never read more than the cap (a planted huge file must not be slurped), and only a regular file.
        let bytes = read_capped(&path).ok_or_else(skin_not_found)?;
        // The file must still be the picture its name claims.
        if skin_problem(&bytes).is_some() || skin_id_for(&bytes) != id {
            return Err(skin_not_found());
        }
        Ok(bytes)
    }
}

/// The bytes of a regular file of at most `SKIN_MAX_BYTES`; None for anything else.
fn read_capped(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(SKIN_MAX_BYTES as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= SKIN_MAX_BYTES).then_some(bytes)
}

/// The owners and blocked ids saved in `<dir>/index.json`; empty when there is none or it cannot be read.
fn load_index(directory: &std::path::Path) -> Book {
    let mut book = Book::default();
    let Ok(text) = std::fs::read_to_string(directory.join(INDEX_FILE)) else { return book };
    let Ok(saved) = serde_json::from_str::<Value>(&text) else {
        log_error("[KYTTU SKIN] the skin index is unreadable; starting empty");
        return book;
    };
    if let Some(owners) = saved.get("owners").and_then(Value::as_object) {
        book.owners = owners.iter().filter_map(|(who, id)| id.as_str().filter(|id| is_skin_id(id)).map(|id| (who.clone(), id.to_string()))).collect();
    }
    if let Some(blocked) = saved.get("blocked").and_then(Value::as_array) {
        book.blocked = blocked.iter().filter_map(|id| id.as_str().filter(|id| is_skin_id(id)).map(str::to_string)).collect();
    }
    book
}

fn read_upstream(upstream: &Upstream, id: &str) -> Result<Vec<u8>, Reply> {
    let (status, body) = upstream.get_raw(&format!("/api/v1/skins/{id}")).map_err(upstream_failure)?;
    if status == 200 {
        // Only a picture that matches its id is served to the game.
        if skin_problem(&body).is_none() && skin_id_for(&body) == id {
            return Ok(body);
        }
        return Err((502, json!({ "ok": false, "code": "UPSTREAM_BAD_REPLY", "error": "The account service sent an unusable reply" })));
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(json) => Err((status, json)),
        Err(_) => Err((502, json!({ "ok": false, "code": "UPSTREAM_BAD_REPLY", "error": "The account service sent an unusable reply" }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{LastRouteServer, Request, ServerOptions};
    use crate::upstream::test_website::{fake_website, fake_website_bytes};
    use crate::util::{manual_clock, random_bytes, ManualClock};
    use last_route_core::shared::skin_validate::{encode_png_stored, png_from_raw, push_chunk, zlib_stored};

    /// A well-formed, opaque 64x64 skin; `seed` makes a different picture (so a different id).
    fn skin(seed: u8) -> Vec<u8> {
        let mut rgba: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 251) as u8, (i / 64) as u8 * 4, seed, 255]).collect();
        rgba[0] = seed;
        encode_png_stored(&rgba, 64, 64)
    }

    /// A well-formed opaque PNG of another size.
    fn sized(width: u32, height: u32) -> Vec<u8> {
        encode_png_stored(&vec![255u8; (width * height * 4) as usize], width, height)
    }

    /// The hostile file of the audit: a good signature and IHDR (64x64) and nothing else.
    fn header_only() -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&[0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    fn encode(bytes: &[u8]) -> String {
        base64(bytes)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_skins_{name}_{}", crate::util::hex(&random_bytes(6))));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Accepts the token `good-token` for ADA and `other-token` for BOB; KYTTU (`owner-token`) is a full admin.
    struct Sessions;
    impl SessionLookup for Sessions {
        fn session_username(&self, token: &str) -> Option<String> {
            match token {
                "good-token" => Some("ADA".to_string()),
                "other-token" => Some("BOB".to_string()),
                "owner-token" => Some("KYTTU".to_string()),
                _ => None,
            }
        }
    }
    impl SkinAccounts for Sessions {
        fn is_owner(&self, username: &str) -> bool {
            username == "KYTTU"
        }
    }

    fn local(directory: &PathBuf) -> Skins {
        Skins::new(SkinsOptions::new(Some(directory.clone()), None))
    }

    fn timed(directory: &PathBuf) -> (Skins, ManualClock) {
        let (clock, handle) = manual_clock(1_000_000);
        (Skins::new(SkinsOptions { directory: Some(directory.clone()), upstream: None, now: clock }), handle)
    }

    fn upload_as(skins: &Skins, token: &str, png: &[u8]) -> Reply {
        skins.upload(&Sessions, &json!({ "sessionToken": token, "png": encode(png) }))
    }

    #[test]
    fn the_id_is_the_sha256_of_the_png_and_the_validator_names_each_refusal() {
        let good = skin(1);
        assert_eq!(skin_id_for(&good), sha256_hex(&good));
        assert!(is_skin_id(&skin_id_for(&good)));
        assert!(!is_skin_id(&skin_id_for(&good).to_uppercase()));
        assert!(!is_skin_id("../etc/passwd"));
        assert_eq!(skin_problem(&good), None);
        assert_eq!(skin_problem(&sized(32, 64)).as_deref(), Some("Skin PNG must be 64x64"));
        assert_eq!(skin_problem(b"GIF89a................................").as_deref(), Some("Skin must be a PNG file"));
        assert_eq!(skin_problem(&vec![0u8; SKIN_MAX_BYTES + 1]).as_deref(), Some("Skin PNG must be 64 KB or smaller"));
        assert_eq!(skin_problem(&[]).as_deref(), Some("Skin PNG is empty"));
    }

    #[test]
    fn the_cap_is_64_kb_like_the_game_and_the_json_body_limit_fits_its_base64() {
        assert_eq!(SKIN_MAX_BYTES, 64 * 1024);
        assert_eq!(SKIN_MAX_BYTES, crate::server::SKIN_MAX_BYTES);
        // A full-size skin as base64 inside its JSON must fit the route's body limit.
        let json_len = json!({ "sessionToken": "t".repeat(64), "png": base64(&vec![0u8; SKIN_MAX_BYTES]) }).to_string().len();
        assert!(json_len <= SKIN_JSON_BODY_BYTES, "{json_len} > {}", SKIN_JSON_BODY_BYTES);
        assert!(SKIN_JSON_BODY_BYTES > crate::server::MAX_BODY_BYTES, "the default 64 KiB limit would refuse a full-size skin");
    }

    #[test]
    fn a_hostile_png_that_only_has_a_good_header_is_refused() {
        let dir = temp_dir("hostile");
        let skins = local(&dir);
        let reply = upload_as(&skins, "good-token", &header_only());
        assert_eq!((reply.0, reply.1["code"].clone()), (400, json!("BAD_SKIN")));
        assert!(skin_problem(&header_only()).is_some());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing was stored");
    }

    #[test]
    fn upload_refuses_a_bad_crc_a_wrong_inflate_size_a_transparent_base_and_trailing_junk() {
        let dir = temp_dir("refusals");
        let skins = local(&dir);
        let refused = |bytes: &[u8]| {
            let r = upload_as(&skins, "good-token", bytes);
            assert_eq!((r.0, r.1["code"].clone()), (400, json!("BAD_SKIN")), "{}", r.1);
            r.1["error"].as_str().unwrap().to_string()
        };
        let mut bad_crc = skin(2);
        let at = bad_crc.len() - 20;
        bad_crc[at] ^= 0x40;
        assert!(refused(&bad_crc).contains("checksum"));
        // The IDAT promises 64x64 RGBA (64 rows of a filter byte and 256 bytes: 16448 bytes) but inflates to one byte less.
        let short = png_from_raw(64, 64, 8, 6, &zlib_stored(&vec![0u8; 16447]));
        assert!(refused(&short).contains("wrong size"));
        // One byte more, and a stream far longer than promised (a decompression bomb stops at the promise).
        let long = png_from_raw(64, 64, 8, 6, &zlib_stored(&vec![0u8; 16449]));
        assert!(refused(&long).contains("wrong size"));
        let bomb = png_from_raw(64, 64, 8, 6, &zlib_stored(&vec![0u8; 40_000]));
        assert!(refused(&bomb).contains("corrupt"));
        // Fully transparent: passes every structural check, hides the runner.
        assert!(refused(&encode_png_stored(&vec![0u8; 64 * 64 * 4], 64, 64)).contains("opaque"));
        let mut junk = skin(2);
        junk.extend_from_slice(b"trailing");
        assert!(refused(&junk).contains("after the end"));
        // An unknown critical chunk before IEND.
        let mut critical = skin(2);
        critical.truncate(critical.len() - 12);
        push_chunk(&mut critical, b"ZZZZ", b"x");
        push_chunk(&mut critical, b"IEND", &[]);
        assert!(refused(&critical).contains("cannot read"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn base64_is_strict_and_refuses_what_the_lenient_decoder_would_take() {
        let good = skin(3);
        assert_eq!(decode_skin_base64(&encode(&good)), Some(good.clone()));
        assert_eq!(decode_skin_base64("@@@@"), None);
        assert_eq!(decode_skin_base64("QQ=="), Some(b"A".to_vec()));
        assert_eq!(decode_skin_base64("QQ"), None, "not a whole quad");
        assert_eq!(decode_skin_base64("QUJD=="), None, "padding on a full quad");
        assert_eq!(decode_skin_base64("QQ=A"), None, "padding in the middle");
        assert_eq!(decode_skin_base64(&encode(&vec![0u8; SKIN_MAX_BYTES + 3])), None, "over the byte cap");
    }

    #[test]
    fn upload_without_a_session_answers_401_and_stores_nothing() {
        let dir = temp_dir("nosession");
        let skins = local(&dir);
        let body = json!({ "png": encode(&skin(1)) });
        assert_eq!(skins.upload(&Sessions, &body).0, 401);
        let stale = json!({ "sessionToken": "stale", "png": encode(&skin(1)) });
        assert_eq!(skins.upload(&Sessions, &stale).1["code"], "NO_SESSION");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn upload_with_a_session_stores_the_png_under_its_id_and_get_serves_it() {
        let dir = temp_dir("store");
        let skins = local(&dir);
        let good = skin(1);
        let (status, reply) = upload_as(&skins, "good-token", &good);
        let id = skin_id_for(&good);
        assert_eq!((status, reply), (200, json!({ "ok": true, "id": id, "hash": id })));
        assert_eq!(std::fs::read(dir.join(format!("{id}.png"))).unwrap(), good);
        assert_eq!(skins.read(&id), Ok(good));
    }

    #[test]
    fn a_bad_size_a_non_png_and_bad_base64_each_answer_400_bad_skin() {
        let dir = temp_dir("badsize");
        let skins = local(&dir);
        let small = upload_as(&skins, "good-token", &sized(32, 32));
        assert_eq!((small.0, small.1["code"].clone()), (400, json!("BAD_SKIN")));
        assert_eq!(small.1["error"], "Skin PNG must be 64x64");
        let not_png = upload_as(&skins, "good-token", &[1u8; 64]);
        assert_eq!(not_png.0, 400);
        let garbage = skins.upload(&Sessions, &json!({ "sessionToken": "good-token", "png": "@@@@" }));
        assert_eq!((garbage.0, garbage.1["code"].clone()), (400, json!("BAD_SKIN")));
        let mut huge = skin(1);
        huge.extend(vec![0u8; SKIN_MAX_BYTES]);
        let too_big = upload_as(&skins, "good-token", &huge);
        assert_eq!((too_big.0, too_big.1["code"].clone()), (400, json!("BAD_SKIN")));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn get_of_an_unknown_skin_is_404_and_a_malformed_id_is_400() {
        let dir = temp_dir("unknown");
        let skins = local(&dir);
        let unknown = skins.read(&"a".repeat(64));
        assert_eq!(unknown.unwrap_err().0, 404);
        let malformed = skins.read("../../secrets").unwrap_err();
        assert_eq!((malformed.0, malformed.1["code"].clone()), (400, json!("BAD_SKIN_ID")));
    }

    #[test]
    fn a_stored_file_that_no_longer_matches_its_id_or_is_hostile_is_not_served() {
        let dir = temp_dir("tampered");
        let skins = local(&dir);
        let id = skin_id_for(&skin(1));
        std::fs::write(dir.join(format!("{id}.png")), sized(16, 16)).unwrap();
        assert_eq!(skins.read(&id).unwrap_err().0, 404);
        // A hostile file planted under its own hash is refused too: the name matches, the body does not validate.
        let hostile = header_only();
        let hostile_id = skin_id_for(&hostile);
        std::fs::write(dir.join(format!("{hostile_id}.png")), &hostile).unwrap();
        assert_eq!(skins.read(&hostile_id).unwrap_err().0, 404);
    }

    #[test]
    fn the_per_account_limit_is_five_uploads_per_ten_minutes() {
        let dir = temp_dir("limit");
        let (skins, clock) = timed(&dir);
        for seed in 0..UPLOAD_LIMIT as u8 {
            assert_eq!(upload_as(&skins, "good-token", &skin(seed)).0, 200, "upload {seed}");
        }
        let sixth = upload_as(&skins, "good-token", &skin(9));
        assert_eq!((sixth.0, sixth.1["code"].clone()), (429, json!("UPLOAD_RATE_LIMIT")));
        assert_eq!(upload_as(&skins, "other-token", &skin(9)).0, 200, "the limit is per account");
        clock.advance(UPLOAD_WINDOW_MS - 1);
        assert_eq!(upload_as(&skins, "good-token", &skin(9)).0, 429, "still inside the window");
        clock.advance(1);
        assert_eq!(upload_as(&skins, "good-token", &skin(9)).0, 200, "the window has passed");
        // A refused picture does not use a slot.
        let (skins, _) = timed(&temp_dir("limit_refused"));
        for _ in 0..UPLOAD_LIMIT + 2 {
            assert_eq!(upload_as(&skins, "good-token", &header_only()).0, 400);
        }
        assert_eq!(upload_as(&skins, "good-token", &skin(1)).0, 200);
    }

    #[test]
    fn an_account_keeps_one_skin_and_replacing_it_deletes_the_old_file_unless_someone_else_has_it() {
        let dir = temp_dir("replace");
        let skins = local(&dir);
        let (first, second) = (skin(1), skin(2));
        let (first_id, second_id) = (skin_id_for(&first), skin_id_for(&second));
        upload_as(&skins, "good-token", &first);
        upload_as(&skins, "other-token", &first); // BOB holds the same picture
        upload_as(&skins, "good-token", &second);
        assert!(dir.join(format!("{first_id}.png")).exists(), "BOB still uses the first skin");
        assert!(dir.join(format!("{second_id}.png")).exists());
        upload_as(&skins, "other-token", &second);
        assert!(!dir.join(format!("{first_id}.png")).exists(), "nobody uses the first skin any more");
        let mine = skins.account_action(&Sessions, SkinAction::Mine, &json!({ "sessionToken": "good-token" }));
        assert_eq!(mine.1["id"], json!(second_id));
    }

    #[test]
    fn mine_and_clear_work_for_the_signed_in_account_only() {
        let dir = temp_dir("mine");
        let skins = local(&dir);
        let good = skin(1);
        let id = skin_id_for(&good);
        let anonymous = skins.account_action(&Sessions, SkinAction::Mine, &json!({}));
        assert_eq!((anonymous.0, anonymous.1["code"].clone()), (401, json!("NO_SESSION")));
        assert_eq!(skins.account_action(&Sessions, SkinAction::Mine, &json!({ "sessionToken": "good-token" })).1["id"], json!(""));
        upload_as(&skins, "good-token", &good);
        assert_eq!(skins.account_action(&Sessions, SkinAction::Mine, &json!({ "sessionToken": "good-token" })).1["id"], json!(id));
        assert_eq!(skins.account_action(&Sessions, SkinAction::Clear, &json!({})).0, 401);
        assert_eq!(skins.account_action(&Sessions, SkinAction::Clear, &json!({ "sessionToken": "good-token" })).0, 200);
        assert_eq!(skins.account_action(&Sessions, SkinAction::Mine, &json!({ "sessionToken": "good-token" })).1["id"], json!(""));
        assert_eq!(skins.read(&id).unwrap_err().0, 404, "clearing deletes the file");
    }

    #[test]
    fn owner_remove_blocks_the_hash_and_survives_a_restart() {
        let dir = temp_dir("remove");
        let skins = local(&dir);
        let bad = skin(7);
        let id = skin_id_for(&bad);
        upload_as(&skins, "good-token", &bad);
        let denied = skins.account_action(&Sessions, SkinAction::Remove, &json!({ "sessionToken": "good-token", "hash": id }));
        assert_eq!((denied.0, denied.1["code"].clone()), (403, json!("ADMIN_OWNER_REQUIRED")));
        assert_eq!(skins.account_action(&Sessions, SkinAction::Remove, &json!({ "hash": id })).0, 403);
        let by_name = skins.account_action(&Sessions, SkinAction::Remove, &json!({ "sessionToken": "owner-token", "username": "ada" }));
        assert_eq!((by_name.0, by_name.1["blocked"].clone()), (200, json!(true)));
        assert_eq!(skins.read(&id).unwrap_err().0, 404);
        assert!(!dir.join(format!("{id}.png")).exists());
        let again = upload_as(&skins, "good-token", &bad);
        assert_eq!((again.0, again.1["code"].clone()), (403, json!("SKIN_BLOCKED")));
        // A new store over the same directory still knows.
        let restarted = local(&dir);
        assert_eq!(upload_as(&restarted, "other-token", &bad).0, 403);
        assert_eq!(restarted.account_action(&Sessions, SkinAction::Remove, &json!({ "sessionToken": "owner-token", "hash": "nope" })).0, 404);
        let by_id = restarted.account_action(&Sessions, SkinAction::Remove, &json!({ "sessionToken": "owner-token", "hash": "a".repeat(64) }));
        assert_eq!(by_id.0, 200, "blocking an id that was never stored is fine");
    }

    #[test]
    fn owners_survive_a_restart() {
        let dir = temp_dir("restart");
        let first = skin(1);
        upload_as(&local(&dir), "good-token", &first);
        let restarted = local(&dir);
        let mine = restarted.account_action(&Sessions, SkinAction::Mine, &json!({ "sessionToken": "good-token" }));
        assert_eq!(mine.1["id"], json!(skin_id_for(&first)));
    }

    #[test]
    fn a_stored_file_over_the_cap_or_a_directory_named_like_a_skin_is_never_slurped() {
        let dir = temp_dir("planted");
        let skins = local(&dir);
        let id = "b".repeat(64);
        std::fs::create_dir(dir.join(format!("{id}.png"))).unwrap();
        assert_eq!(skins.read(&id).unwrap_err().0, 404);
        let big = "c".repeat(64);
        std::fs::write(dir.join(format!("{big}.png")), vec![0u8; SKIN_MAX_BYTES + 10]).unwrap();
        assert_eq!(skins.read(&big).unwrap_err().0, 404);
    }

    #[test]
    fn with_an_upstream_the_upload_is_forwarded_with_only_the_session_and_the_png() {
        let good = skin(1);
        let (upstream, seen) = fake_website(vec![r#"{"ok":true,"id":"stored-by-website"}"#]);
        let skins = Skins::new(SkinsOptions::new(None, Some(upstream)));
        let body = json!({ "sessionToken": "good-token", "png": encode(&good), "extra": 1 });
        assert_eq!(skins.upload(&Sessions, &body), (200, json!({ "ok": true, "id": "stored-by-website" })));
        assert_eq!(
            seen.join().unwrap(),
            vec![("/api/v1/skins".to_string(), json!({ "sessionToken": "good-token", "png": encode(&good) }))]
        );
    }

    #[test]
    fn with_an_upstream_a_hostile_upload_is_refused_here_and_never_forwarded() {
        // The website is never asked: a header-only file is refused before the forward (the fake would panic on accept).
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let upstream = Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let skins = Skins::new(SkinsOptions::new(None, Some(upstream)));
        let reply = skins.upload(&Sessions, &json!({ "sessionToken": "good-token", "png": encode(&header_only()) }));
        assert_eq!((reply.0, reply.1["code"].clone()), (400, json!("BAD_SKIN")));
    }

    #[test]
    fn with_an_upstream_a_stored_skin_is_fetched_from_the_website_and_checked_against_its_id() {
        let good = skin(1);
        let id = skin_id_for(&good);
        let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", good.len()).into_bytes();
        reply.extend_from_slice(&good);
        let (upstream, path) = fake_website_bytes(reply);
        let skins = Skins::new(SkinsOptions::new(None, Some(upstream)));
        assert_eq!(skins.read(&id), Ok(good));
        assert_eq!(path.join().unwrap(), format!("/api/v1/skins/{id}"));
    }

    #[test]
    fn with_an_upstream_a_website_404_passes_through_and_a_mismatched_or_hostile_picture_is_refused() {
        let id = skin_id_for(&skin(1));
        let not_found = r#"{"ok":false,"code":"SKIN_NOT_FOUND","error":"x"}"#;
        let (upstream, _) = fake_website_bytes(
            format!("HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{not_found}", not_found.len())
                .into_bytes(),
        );
        let skins = Skins::new(SkinsOptions::new(None, Some(upstream)));
        let (status, body) = skins.read(&id).unwrap_err();
        assert_eq!((status, body["code"].clone()), (404, json!("SKIN_NOT_FOUND")));

        for other in [sized(16, 16), header_only()] {
            let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", other.len()).into_bytes();
            reply.extend_from_slice(&other);
            let (upstream, _) = fake_website_bytes(reply);
            let skins = Skins::new(SkinsOptions::new(None, Some(upstream)));
            assert_eq!(skins.read(&id).unwrap_err().0, 502);
        }
        // Hostile bytes whose id is right are refused as well.
        let hostile = header_only();
        let hostile_id = skin_id_for(&hostile);
        let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", hostile.len()).into_bytes();
        reply.extend_from_slice(&hostile);
        let (upstream, _) = fake_website_bytes(reply);
        assert_eq!(Skins::new(SkinsOptions::new(None, Some(upstream))).read(&hostile_id).unwrap_err().0, 502);
    }

    #[test]
    fn with_an_unreachable_upstream_both_routes_answer_503_upstream_unavailable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let upstream = Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let skins = Skins::new(SkinsOptions::new(None, Some(upstream)));
        let uploaded = upload_as(&skins, "good-token", &skin(1));
        assert_eq!((uploaded.0, uploaded.1["code"].clone()), (503, json!("UPSTREAM_UNAVAILABLE")));
        let read = skins.read(&skin_id_for(&skin(1))).unwrap_err();
        assert_eq!((read.0, read.1["code"].clone()), (503, json!("UPSTREAM_UNAVAILABLE")));
        assert_eq!(skins.account_action(&Sessions, SkinAction::Mine, &json!({ "sessionToken": "good-token" })).0, 503);
    }

    // ---- through the router and a real socket -----------------------------------------------------------------

    fn register(server: &LastRouteServer, name: &str) -> String {
        let body = json!({ "username": name, "password": "correct horse battery" });
        let response = server.handle(&Request::json("POST", "/api/account/register", &body));
        assert_eq!(response.status, 200, "{}", response.json_body());
        response.json_body()["sessionToken"].as_str().unwrap().to_string()
    }

    fn router(directory: &PathBuf) -> LastRouteServer {
        let mut options = ServerOptions { skin_directory: Some(directory.clone()), ..Default::default() };
        options.api_rate_limit_max = 10_000;
        options.account_rate_limit_max = 10_000;
        LastRouteServer::new(options)
    }

    #[test]
    fn the_router_serves_a_stored_skin_as_a_safe_immutable_png_on_both_routes_and_refuses_an_upload_without_a_session() {
        let dir = temp_dir("router");
        let good = skin(1);
        let id = skin_id_for(&good);
        std::fs::write(dir.join(format!("{id}.png")), &good).unwrap();
        let server = LastRouteServer::new(ServerOptions { skin_directory: Some(dir.clone()), ..Default::default() });

        for path in [format!("/api/v1/skins/{id}"), format!("/api/skins/files/{id}.png")] {
            let found = server.handle(&Request::new("GET", &path));
            assert_eq!(found.status, 200, "{path}");
            assert_eq!(found.header("content-type"), Some("image/png"));
            assert_eq!(found.header("cache-control"), Some("public, max-age=31536000, immutable"));
            assert_eq!(found.header("x-content-type-options"), Some("nosniff"));
            assert_eq!(found.header("content-security-policy"), Some("default-src 'none'"));
            assert_eq!(found.body, good);
        }

        let missing = server.handle(&Request::new("GET", &format!("/api/v1/skins/{}", "b".repeat(64))));
        assert_eq!((missing.status, missing.json_body()["code"].clone()), (404, json!("SKIN_NOT_FOUND")));
        let legacy_missing = server.handle(&Request::new("GET", &format!("/api/skins/files/{}.png", "b".repeat(64))));
        assert_eq!((legacy_missing.status, legacy_missing.json_body()["code"].clone()), (404, json!("SKIN_NOT_FOUND")));
        let bad_legacy = server.handle(&Request::new("GET", "/api/skins/files/not-a-hash.png"));
        assert_eq!(bad_legacy.status, 400);

        let anonymous = server.handle(&Request::json("POST", "/api/v1/skins", &json!({ "png": encode(&good) })));
        assert_eq!((anonymous.status, anonymous.json_body()["code"].clone()), (401, json!("NO_SESSION")));
    }

    #[test]
    fn the_router_stores_an_upload_on_the_v1_json_route_and_the_legacy_raw_route() {
        let dir = temp_dir("router_upload");
        let server = router(&dir);
        let token = register(&server, "ADA");
        let (first, second) = (skin(1), skin(2));

        let v1 = server.handle(&Request::json("POST", "/api/v1/skins", &json!({ "sessionToken": token, "png": encode(&first) })));
        assert_eq!((v1.status, v1.json_body()["id"].clone()), (200, json!(skin_id_for(&first))));

        let mut raw = Request::new("POST", "/api/skins/upload").with_header("x-session-token", &token);
        raw.body = second.clone();
        let legacy = server.handle(&raw);
        assert_eq!((legacy.status, legacy.json_body()["hash"].clone()), (200, json!(skin_id_for(&second))));
        let mut anonymous = Request::new("POST", "/api/skins/upload");
        anonymous.body = second.clone();
        assert_eq!(server.handle(&anonymous).status, 401);
        let mut hostile = Request::new("POST", "/api/skins/upload").with_header("x-session-token", &token);
        hostile.body = header_only();
        assert_eq!(server.handle(&hostile).status, 400);

        let mine = server.handle(&Request::json("POST", "/api/skins/mine", &json!({ "sessionToken": token })));
        assert_eq!(mine.json_body()["hash"], json!(skin_id_for(&second)));
        let mine_v1 = server.handle(&Request::json("POST", "/api/v1/skins/mine", &json!({ "sessionToken": token })));
        assert_eq!(mine_v1.json_body()["id"], json!(skin_id_for(&second)));
        let cleared = server.handle(&Request::json("POST", "/api/v1/skins/clear", &json!({ "sessionToken": token })));
        assert_eq!(cleared.status, 200);
        let gone = server.handle(&Request::new("GET", &format!("/api/v1/skins/{}", skin_id_for(&second))));
        assert_eq!(gone.status, 404);
    }

    #[test]
    fn the_route_table_gives_the_skin_upload_its_own_body_limit() {
        use crate::server::{body_limit, MAX_BODY_BYTES, MOD_REGISTER_MAX_BODY_BYTES};
        assert_eq!(body_limit("/api/v1/skins"), SKIN_JSON_BODY_BYTES);
        assert_eq!(body_limit("/api/admin/mods/register"), MOD_REGISTER_MAX_BODY_BYTES);
        assert_eq!(body_limit("/api/account/login"), MAX_BODY_BYTES);
        assert_eq!(body_limit("/api/v1/skins/mine"), MAX_BODY_BYTES);
        // Through the router: a full-size skin passes the limit, the same bytes elsewhere are refused as too large.
        let dir = temp_dir("router_limit");
        let server = router(&dir);
        let token = register(&server, "ADA");
        let big = json!({ "sessionToken": token, "png": base64(&vec![0u8; SKIN_MAX_BYTES]) });
        let on_skins = server.handle(&Request::json("POST", "/api/v1/skins", &big));
        assert_eq!(on_skins.status, 400, "not 413: it reached the skin check and failed it as a picture");
        let elsewhere = server.handle(&Request::json("POST", "/api/account/login", &big));
        assert_eq!(elsewhere.status, 413);
        let mut oversized = json!({ "sessionToken": token });
        oversized["png"] = json!("A".repeat(SKIN_JSON_BODY_BYTES));
        assert_eq!(server.handle(&Request::json("POST", "/api/v1/skins", &oversized)).status, 413);
        // The raw route: over the cap is 413 before anything is read.
        let mut raw = Request::new("POST", "/api/skins/upload").with_header("x-session-token", &token);
        raw.body = vec![0u8; SKIN_MAX_BYTES + 1];
        assert_eq!(server.handle(&raw).status, 413);
    }

    /// A GET or POST over a real socket; returns the status, the headers text and the body.
    fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, String, Vec<u8>) {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, head, raw[split + 4..].to_vec())
    }

    #[test]
    fn upload_then_fetch_then_validate_against_a_local_server_over_loopback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = temp_dir("loopback");
        let server = Arc::new(router(&dir));
        let token = register(&server, "ADA");
        let listener = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = listener.server_addr().to_ip().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let threads = server.serve(listener, 2, stop.clone());

        // The body the game's `api::skin_upload_body` builds: {sessionToken, png: base64}.
        let skin = skin(5);
        let id = skin_id_for(&skin);
        let upload = json!({ "sessionToken": token, "png": base64(&skin) }).to_string();
        let (status, _, body) = http(port, "POST", "/api/v1/skins", &[("Content-Type", "application/json")], upload.as_bytes());
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["id"], json!(id));

        // Fetch by hash on the route the game asks first, and on the legacy route.
        for path in [format!("/api/v1/skins/{id}"), format!("/api/skins/files/{id}.png")] {
            let (status, head, fetched) = http(port, "GET", &path, &[], b"");
            assert_eq!(status, 200, "{path}");
            assert!(head.to_ascii_lowercase().contains("content-type: image/png"), "{head}");
            // Validate exactly as the client does with a download: the hash is the id and the whole file passes.
            assert_eq!(skin_id_for(&fetched), id);
            assert_eq!(skin_validate::check_skin_png(&fetched, SKIN_MAX_BYTES), Ok(()));
            assert_eq!(fetched, skin);
        }

        // A hostile upload is refused over the wire and cannot be fetched.
        let hostile = json!({ "sessionToken": token, "png": base64(&header_only()) }).to_string();
        let (status, _, _) = http(port, "POST", "/api/v1/skins", &[("Content-Type", "application/json")], hostile.as_bytes());
        assert_eq!(status, 400);
        let (status, _, _) = http(port, "GET", &format!("/api/v1/skins/{}", skin_id_for(&header_only())), &[], b"");
        assert_eq!(status, 404);

        stop.store(true, Ordering::SeqCst);
        for thread in threads {
            let _ = thread.join();
        }
    }
}
