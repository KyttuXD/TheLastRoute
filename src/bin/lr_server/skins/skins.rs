//! Port of the skin routes of `backend/skins.mjs` (the website side lives in KyttusLab).
//!
//!   POST /api/v1/skins        body {sessionToken, png: base64 PNG}  -> {ok, id}
//!   GET  /api/v1/skins/<id>   -> image/png, or 404 SKIN_NOT_FOUND
//!
//! A skin is a 64x64 PNG of at most 16 KB. Its id is the lowercase hex SHA-256 of the PNG bytes, so an id names exactly
//! one picture. With `LASTROUTE_ACCOUNT_UPSTREAM` set, the website owns the accounts and the skins, so both routes are
//! forwarded there (the upload keeps only `sessionToken` and `png`). Otherwise skins are stored as
//! `<data dir>/skins/<id>.png` (temp file + rename) and the session is checked against the local accounts.

use crate::upstream::{Upstream, UpstreamError};
use crate::util::{base64, from_base64, log_error, log_warn, sha256_hex, write_atomic, Reply, SessionLookup};
use serde_json::{json, Value};
use std::path::PathBuf;

pub const SKIN_SIZE: u32 = 64;
pub const SKIN_MAX_BYTES: usize = 16 * 1024;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

pub struct SkinsOptions {
    /// Where skins are stored when there is no upstream (`<data dir>/skins`). None: the skin routes answer 503.
    pub directory: Option<PathBuf>,
    /// `LASTROUTE_ACCOUNT_UPSTREAM`: the website that stores the skins.
    pub upstream: Option<Upstream>,
}

#[derive(Clone)]
pub struct Skins {
    directory: Option<PathBuf>,
    upstream: Option<Upstream>,
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

/// `skinProblem(bytes)`: None when the bytes are a 64x64 PNG of at most 16 KB, else the reason.
pub fn skin_problem(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some("Skin PNG is empty".into());
    }
    if bytes.len() > SKIN_MAX_BYTES {
        return Some("Skin PNG must be 16 KB or smaller".into());
    }
    if bytes.len() < 33 || bytes[..8] != PNG_SIGNATURE {
        return Some("Skin must be a PNG file".into());
    }
    if &bytes[12..16] != b"IHDR" {
        return Some("Skin PNG is missing its IHDR chunk".into());
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width != SKIN_SIZE || height != SKIN_SIZE {
        return Some(format!("Skin PNG must be {SKIN_SIZE}x{SKIN_SIZE}"));
    }
    None
}

impl Skins {
    pub fn new(options: SkinsOptions) -> Skins {
        Skins { directory: options.directory, upstream: options.upstream }
    }

    /// POST /api/v1/skins.
    pub fn upload(&self, accounts: &dyn SessionLookup, body: &Value) -> Reply {
        if let Some(upstream) = &self.upstream {
            let forwarded = json!({
                "sessionToken": body.get("sessionToken").cloned().unwrap_or(Value::Null),
                "png": body.get("png").cloned().unwrap_or(Value::Null),
            });
            return upstream.call("/api/v1/skins", &forwarded);
        }
        let Some(directory) = &self.directory else { return unavailable() };
        let token = body.get("sessionToken").and_then(Value::as_str).unwrap_or("");
        if token.is_empty() || accounts.session_username(token).is_none() {
            return (401, json!({ "ok": false, "code": "NO_SESSION", "error": "Sign in to upload a skin" }));
        }
        let Some(bytes) = decode_skin_base64(body.get("png").and_then(Value::as_str).unwrap_or("")) else {
            return bad_skin("png must be base64 of a 64x64 PNG up to 16 KB");
        };
        if let Some(problem) = skin_problem(&bytes) {
            return bad_skin(&problem);
        }
        let id = skin_id_for(&bytes);
        if let Err(error) = write_atomic(&directory.join(format!("{id}.png")), &bytes) {
            log_error(&format!("[KYTTU SKIN] could not store skin {id}: {error}"));
            return (500, json!({ "ok": false, "code": "SKIN_STORE_FAILED", "error": "Could not store skin" }));
        }
        (200, json!({ "ok": true, "id": id }))
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
        let bytes = std::fs::read(directory.join(format!("{id}.png"))).map_err(|_| skin_not_found())?;
        // The file must still be the picture its name claims.
        if skin_problem(&bytes).is_some() || skin_id_for(&bytes) != id {
            return Err(skin_not_found());
        }
        Ok(bytes)
    }
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
    use crate::util::random_bytes;

    /// A PNG header of the given size. The check reads the signature and IHDR only, so the CRC is left zero.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
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

    /// Accepts one token, `good-token`, for the user ADA.
    struct OneSession;
    impl SessionLookup for OneSession {
        fn session_username(&self, token: &str) -> Option<String> {
            (token == "good-token").then(|| "ADA".to_string())
        }
    }

    fn local(directory: &PathBuf) -> Skins {
        Skins::new(SkinsOptions { directory: Some(directory.clone()), upstream: None })
    }

    #[test]
    fn the_id_is_the_sha256_of_the_png_and_the_validator_names_each_refusal() {
        let good = png_bytes(64, 64);
        assert_eq!(skin_id_for(&good), sha256_hex(&good));
        assert!(is_skin_id(&skin_id_for(&good)));
        assert!(!is_skin_id(&skin_id_for(&good).to_uppercase()));
        assert!(!is_skin_id("../etc/passwd"));
        assert_eq!(skin_problem(&good), None);
        assert_eq!(skin_problem(&png_bytes(32, 64)).as_deref(), Some("Skin PNG must be 64x64"));
        assert_eq!(skin_problem(b"GIF89a................................").as_deref(), Some("Skin must be a PNG file"));
        assert_eq!(skin_problem(&vec![0u8; SKIN_MAX_BYTES + 1]).as_deref(), Some("Skin PNG must be 16 KB or smaller"));
    }

    #[test]
    fn base64_is_strict_and_refuses_what_the_lenient_decoder_would_take() {
        let good = png_bytes(64, 64);
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
        let body = json!({ "png": encode(&png_bytes(64, 64)) });
        assert_eq!(skins.upload(&OneSession, &body).0, 401);
        let stale = json!({ "sessionToken": "stale", "png": encode(&png_bytes(64, 64)) });
        assert_eq!(skins.upload(&OneSession, &stale).1["code"], "NO_SESSION");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn upload_with_a_session_stores_the_png_under_its_id_and_get_serves_it() {
        let dir = temp_dir("store");
        let skins = local(&dir);
        let good = png_bytes(64, 64);
        let (status, reply) = skins.upload(&OneSession, &json!({ "sessionToken": "good-token", "png": encode(&good) }));
        let id = skin_id_for(&good);
        assert_eq!((status, reply), (200, json!({ "ok": true, "id": id })));
        assert_eq!(std::fs::read(dir.join(format!("{id}.png"))).unwrap(), good);
        assert_eq!(skins.read(&id), Ok(good));
    }

    #[test]
    fn a_bad_size_a_non_png_and_bad_base64_each_answer_400_bad_skin() {
        let dir = temp_dir("badsize");
        let skins = local(&dir);
        let small = skins.upload(&OneSession, &json!({ "sessionToken": "good-token", "png": encode(&png_bytes(32, 32)) }));
        assert_eq!((small.0, small.1["code"].clone()), (400, json!("BAD_SKIN")));
        assert_eq!(small.1["error"], "Skin PNG must be 64x64");
        let not_png = skins.upload(&OneSession, &json!({ "sessionToken": "good-token", "png": encode(&[1u8; 64]) }));
        assert_eq!(not_png.0, 400);
        let garbage = skins.upload(&OneSession, &json!({ "sessionToken": "good-token", "png": "@@@@" }));
        assert_eq!((garbage.0, garbage.1["code"].clone()), (400, json!("BAD_SKIN")));
        let mut huge = png_bytes(64, 64);
        huge.extend(vec![0u8; SKIN_MAX_BYTES]);
        let too_big = skins.upload(&OneSession, &json!({ "sessionToken": "good-token", "png": encode(&huge) }));
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
    fn a_stored_file_that_no_longer_matches_its_id_is_not_served() {
        let dir = temp_dir("tampered");
        let skins = local(&dir);
        let id = skin_id_for(&png_bytes(64, 64));
        std::fs::write(dir.join(format!("{id}.png")), png_bytes(16, 16)).unwrap();
        assert_eq!(skins.read(&id).unwrap_err().0, 404);
    }

    #[test]
    fn with_an_upstream_the_upload_is_forwarded_with_only_the_session_and_the_png() {
        let good = png_bytes(64, 64);
        let (upstream, seen) = fake_website(vec![r#"{"ok":true,"id":"stored-by-website"}"#]);
        let skins = Skins::new(SkinsOptions { directory: None, upstream: Some(upstream) });
        let body = json!({ "sessionToken": "good-token", "png": encode(&good), "extra": 1 });
        assert_eq!(skins.upload(&OneSession, &body), (200, json!({ "ok": true, "id": "stored-by-website" })));
        assert_eq!(
            seen.join().unwrap(),
            vec![("/api/v1/skins".to_string(), json!({ "sessionToken": "good-token", "png": encode(&good) }))]
        );
    }

    #[test]
    fn with_an_upstream_a_stored_skin_is_fetched_from_the_website_and_checked_against_its_id() {
        let good = png_bytes(64, 64);
        let id = skin_id_for(&good);
        let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", good.len()).into_bytes();
        reply.extend_from_slice(&good);
        let (upstream, path) = fake_website_bytes(reply);
        let skins = Skins::new(SkinsOptions { directory: None, upstream: Some(upstream) });
        assert_eq!(skins.read(&id), Ok(good));
        assert_eq!(path.join().unwrap(), format!("/api/v1/skins/{id}"));
    }

    #[test]
    fn with_an_upstream_a_website_404_passes_through_and_a_mismatched_picture_is_refused() {
        let id = skin_id_for(&png_bytes(64, 64));
        let not_found = r#"{"ok":false,"code":"SKIN_NOT_FOUND","error":"x"}"#;
        let (upstream, _) = fake_website_bytes(
            format!("HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{not_found}", not_found.len())
                .into_bytes(),
        );
        let skins = Skins::new(SkinsOptions { directory: None, upstream: Some(upstream) });
        let (status, body) = skins.read(&id).unwrap_err();
        assert_eq!((status, body["code"].clone()), (404, json!("SKIN_NOT_FOUND")));

        let other = png_bytes(16, 16);
        let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", other.len()).into_bytes();
        reply.extend_from_slice(&other);
        let (upstream, _) = fake_website_bytes(reply);
        let skins = Skins::new(SkinsOptions { directory: None, upstream: Some(upstream) });
        assert_eq!(skins.read(&id).unwrap_err().0, 502);
    }

    #[test]
    fn with_an_unreachable_upstream_both_routes_answer_503_upstream_unavailable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let upstream = Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let skins = Skins::new(SkinsOptions { directory: None, upstream: Some(upstream) });
        let uploaded = skins.upload(&OneSession, &json!({ "sessionToken": "good-token", "png": encode(&png_bytes(64, 64)) }));
        assert_eq!((uploaded.0, uploaded.1["code"].clone()), (503, json!("UPSTREAM_UNAVAILABLE")));
        let read = skins.read(&skin_id_for(&png_bytes(64, 64))).unwrap_err();
        assert_eq!((read.0, read.1["code"].clone()), (503, json!("UPSTREAM_UNAVAILABLE")));
    }

    #[test]
    fn the_router_serves_a_stored_skin_as_a_png_and_refuses_an_upload_without_a_session() {
        let dir = temp_dir("router");
        let good = png_bytes(64, 64);
        let id = skin_id_for(&good);
        std::fs::write(dir.join(format!("{id}.png")), &good).unwrap();
        let server = LastRouteServer::new(ServerOptions { skin_directory: Some(dir.clone()), ..Default::default() });

        let found = server.handle(&Request::new("GET", &format!("/api/v1/skins/{id}")));
        assert_eq!(found.status, 200);
        assert_eq!(found.header("content-type"), Some("image/png"));
        assert_eq!(found.body, good);

        let missing = server.handle(&Request::new("GET", &format!("/api/v1/skins/{}", "b".repeat(64))));
        assert_eq!((missing.status, missing.json_body()["code"].clone()), (404, json!("SKIN_NOT_FOUND")));

        let anonymous = server.handle(&Request::json("POST", "/api/v1/skins", &json!({ "png": encode(&good) })));
        assert_eq!((anonymous.status, anonymous.json_body()["code"].clone()), (401, json!("NO_SESSION")));
    }
}
