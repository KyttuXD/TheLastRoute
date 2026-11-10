//! Night Watch's account client. It mirrors `last_route/net/api.rs` (`AccountReply`, `credentials_body`, the four
//! `/api/account/*` calls) and Last Route's `Backend` rules for sessions: a stored token is checked with
//! `/api/account/session` on start, a 400/401/403/404 answer drops it, any other failure keeps it.
//!
//! * Usernames are canonical UPPERCASE everywhere (`normalize_username`).
//! * The signed-in account lives in a process-wide slot read through [`current`]; the admin gate (`is_admin`) reads
//!   the server's `admin` flag from the reply, never the local name. Tests and `--check` may call
//!   [`set_force_admin`].
//! * Requests run on a worker thread ([`AccountClient`] polls once per frame, never blocks). The worker fills the
//!   slot itself, so [`current`] is right even when no screen is polling.
//! * There is no `/api/account/guest` route on `lr_server`: GUEST is local (the save's `GUEST_%08X` name), so
//!   "continue as guest" signs out and makes no request of its own.
//! * The browser build has no HTTP bridge for this executable: every request answers "ACCOUNTS NEED THE DESKTOP
//!   BUILD" there and play stays guest only.
#![allow(dead_code)]

use super::config::{self, BackendConfig};
use super::http::{Method, Request};
use super::{Json, NetError, Pending};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Shortest username and password the form accepts before asking the server (the Godot menu's rule).
pub const MIN_USERNAME: usize = 3;
pub const MAX_USERNAME: usize = 24;
pub const MIN_PASSWORD: usize = 6;

pub const TEXT_NEED_FIELDS: &str = "Enter a username (3-24 characters) and a password (6+).";
pub const TEXT_WORKING: &str = "Working...";
pub const TEXT_SIGNED_OUT: &str = "Signed out.";
pub const TEXT_NOTE: &str = "Usernames are CAPS only and shared with Last Route and KT: Systems.";
pub const TEXT_PLACEHOLDER_USER: &str = "3-24 CHARS: A-Z 0-9 _";
pub const TEXT_PLACEHOLDER_PASSWORD: &str = "6+ characters";
pub const TEXT_BROWSER: &str = "Accounts need the desktop build. You can play as a guest here.";

/// The signed-in account, as the admin gate and the save need it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    /// Canonical UPPERCASE.
    pub username: String,
    /// The server marked this account `admin`.
    pub admin: bool,
    pub admin_owner: bool,
    pub admin_permissions: Vec<String>,
    pub session_token: String,
}

static CURRENT: Mutex<Option<Account>> = Mutex::new(None);
static FORCE_ADMIN: AtomicBool = AtomicBool::new(false);

/// The signed-in account, or `None` for a guest.
pub fn current() -> Option<Account> {
    CURRENT.lock().ok().and_then(|g| g.clone())
}

pub fn set_current(account: Option<Account>) {
    if let Ok(mut g) = CURRENT.lock() {
        *g = account;
    }
}

/// True when the server marked the signed-in account `admin`, or `set_force_admin(true)` was called (tests, `--check`).
pub fn is_admin() -> bool {
    FORCE_ADMIN.load(Ordering::Relaxed) || current().map_or(false, |a| a.admin)
}

pub fn set_force_admin(on: bool) {
    FORCE_ADMIN.store(on, Ordering::Relaxed);
}

/// Trimmed and upper-cased (`credentials_body` in Last Route; the server stores names in capitals).
pub fn normalize_username(name: &str) -> String {
    name.trim().to_uppercase()
}

/// The Godot menu's check before any request: `Err` carries the line the status shows.
pub fn validate(username: &str, password: &str) -> Result<(String, String), &'static str> {
    let name = normalize_username(username);
    if name.chars().count() < MIN_USERNAME || name.chars().count() > MAX_USERNAME || password.chars().count() < MIN_PASSWORD {
        return Err(TEXT_NEED_FIELDS);
    }
    Ok((name, password.to_string()))
}

fn s(map: &Json, key: &str) -> String {
    match map.get(key) {
        Some(Value::String(v)) => v.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn b(map: &Json, key: &str) -> bool {
    match map.get(key) {
        Some(Value::Bool(v)) => *v,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(t)) => !t.is_empty(),
        _ => false,
    }
}

/// The register / login / session answer (`accountResult` in account_service.mjs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountReply {
    pub username: String,
    pub session_token: String,
    pub admin: bool,
    pub has_admin_owner: bool,
    pub admin_owner: bool,
    pub has_admin_permissions: bool,
    pub admin_permissions: Vec<String>,
}

impl AccountReply {
    pub fn from_json(p: &Json) -> AccountReply {
        let perms = match p.get("adminPermissions") {
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        AccountReply {
            username: normalize_username(&s(p, "username")),
            session_token: s(p, "sessionToken"),
            admin: b(p, "admin"),
            has_admin_owner: p.contains_key("adminOwner"),
            admin_owner: b(p, "adminOwner"),
            has_admin_permissions: p.contains_key("adminPermissions"),
            admin_permissions: perms,
        }
    }

    /// The account this reply signs in. `fallback_token` is the token the request carried (a session check may not
    /// send a new one).
    pub fn into_account(self, fallback_token: &str) -> Account {
        Account {
            username: self.username,
            admin: self.admin,
            admin_owner: self.admin_owner,
            admin_permissions: self.admin_permissions,
            session_token: if self.session_token.is_empty() { fallback_token.to_string() } else { self.session_token },
        }
    }
}

/// `{username: UPPERCASE trimmed, password, adminProof?}`.
pub fn credentials_body(username: &str, password: &str, admin_proof: &str) -> Value {
    let mut body = json!({"username": normalize_username(username), "password": password});
    if !admin_proof.is_empty() {
        body["adminProof"] = Value::String(admin_proof.to_string());
    }
    body
}

/// `{sessionToken}` for `/api/account/session` and `/api/account/logout`.
pub fn token_body(token: &str) -> Value {
    json!({"sessionToken": token})
}

fn guard(cfg: &BackendConfig) -> Result<(), NetError> {
    if cfg.offline {
        return Err(NetError::Offline);
    }
    #[cfg(target_arch = "wasm32")]
    {
        return Err(NetError::Unsupported(TEXT_BROWSER.into()));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !cfg.accounts_available(cfg!(debug_assertions)) {
            return Err(NetError::Refused(cfg.account_transport_warning(cfg!(debug_assertions)).to_string()));
        }
        Ok(())
    }
}

/// Turns a response into the JSON object on 2xx, or [`NetError::Http`] with the server's `{code, error}`.
fn check(response: super::http::Response) -> Result<Json, NetError> {
    let payload = response.json();
    if response.ok() {
        Ok(payload)
    } else {
        Err(NetError::Http { status: response.status, code: s(&payload, "code"), error: s(&payload, "error") })
    }
}

/// POST `path` with a JSON body (`Accept` and `Content-Type: application/json`, as Last Route's `_headers()`).
pub fn post(cfg: &BackendConfig, path: &str, body: &Value) -> Result<Json, NetError> {
    guard(cfg)?;
    let request = Request::new(Method::Post, cfg.url(path))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json_body(body)
        .timeout(Duration::from_secs(15));
    check(request.send()?)
}

/// `POST /api/account/register`.
pub fn register(cfg: &BackendConfig, username: &str, password: &str) -> Result<AccountReply, NetError> {
    post(cfg, "/api/account/register", &credentials_body(username, password, "")).map(|p| AccountReply::from_json(&p))
}

/// `POST /api/account/login`.
pub fn login(cfg: &BackendConfig, username: &str, password: &str) -> Result<AccountReply, NetError> {
    post(cfg, "/api/account/login", &credentials_body(username, password, "")).map(|p| AccountReply::from_json(&p))
}

/// `POST /api/account/session {sessionToken}`: restores a stored login.
pub fn session(cfg: &BackendConfig, token: &str) -> Result<AccountReply, NetError> {
    post(cfg, "/api/account/session", &token_body(token)).map(|p| AccountReply::from_json(&p))
}

/// `POST /api/account/logout {sessionToken}` (always 200).
pub fn logout(cfg: &BackendConfig, token: &str) -> Result<(), NetError> {
    post(cfg, "/api/account/logout", &token_body(token)).map(|_| ())
}

/// Which request finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    SignIn,
    Register,
    Restore,
}

/// What a screen learns from [`AccountClient::poll`].
#[derive(Clone, Debug, PartialEq)]
pub enum AccountEvent {
    /// Signed in (or the stored session was confirmed). Store `account.username` and `account.session_token` in the save.
    SignedIn { op: Op, account: Account },
    /// The server rejected the stored session: forget it.
    Rejected,
    /// The request failed; the text is player facing. A failed restore keeps the stored token.
    Failed { op: Op, message: String },
}

impl AccountEvent {
    /// The status line the ACCOUNT screen shows for it.
    pub fn status_text(&self) -> String {
        match self {
            AccountEvent::SignedIn { account, .. } => format!("Signed in as {}.", account.username),
            AccountEvent::Rejected => "Your saved sign-in ended. Playing as a guest.".to_string(),
            AccountEvent::Failed { message, .. } => message.clone(),
        }
    }
}

/// The player facing text of a failed call (the server's own `error`, else `ACCOUNT REQUEST FAILED (HTTP n)`).
pub fn failure_text(e: &NetError) -> String {
    let fallback = format!("ACCOUNT REQUEST FAILED (HTTP {})", e.status().unwrap_or(0));
    e.message_or(&fallback)
}

/// True for the statuses that mean the server REJECTED a stored session.
pub fn session_rejected(e: &NetError) -> bool {
    e.status().map_or(false, |s| config::SESSION_REJECTED_CODES.contains(&s))
}

/// One account request at a time, polled from the frame loop.
pub struct AccountClient {
    cfg: BackendConfig,
    op: Option<(Op, Pending<Account>)>,
    logout: Option<Pending<()>>,
}

impl AccountClient {
    /// The backend comes from the same place as Last Route's: the primary origin, or `--backend=` / `--offline` from
    /// the command line, under the same origin policy.
    pub fn new() -> AccountClient {
        let (cfg, _warnings) = BackendConfig::from_args(std::env::args().skip(1), cfg!(debug_assertions));
        AccountClient::with_config(cfg)
    }

    pub fn with_config(cfg: BackendConfig) -> AccountClient {
        AccountClient { cfg, op: None, logout: None }
    }

    /// A request is in flight.
    pub fn busy(&self) -> bool {
        self.op.is_some()
    }

    /// Why accounts cannot be used here, or `None` when they can.
    pub fn unavailable_reason(&self) -> Option<String> {
        guard(&self.cfg).err().map(|e| e.message_or("Accounts are not available."))
    }

    fn start(&mut self, op: Op, work: impl FnOnce(BackendConfig) -> Result<Account, NetError> + Send + 'static) {
        if self.op.is_some() {
            return;
        }
        let pending = match guard(&self.cfg) {
            Err(e) => Pending::done(Err(e)),
            Ok(()) => {
                let cfg = self.cfg.clone();
                Pending::spawn(move || {
                    let account = work(cfg)?;
                    set_current(Some(account.clone()));
                    Ok(account)
                })
            }
        };
        self.op = Some((op, pending));
    }

    /// SIGN IN.
    pub fn sign_in(&mut self, username: &str, password: &str) {
        let (u, p) = (username.to_string(), password.to_string());
        self.start(Op::SignIn, move |cfg| login(&cfg, &u, &p).map(|r| r.into_account("")));
    }

    /// CREATE ACCOUNT.
    pub fn create_account(&mut self, username: &str, password: &str) {
        let (u, p) = (username.to_string(), password.to_string());
        self.start(Op::Register, move |cfg| register(&cfg, &u, &p).map(|r| r.into_account("")));
    }

    /// Start of the program: check the token stored with the save.
    pub fn restore(&mut self, token: &str) {
        if token.is_empty() {
            return;
        }
        let token = token.to_string();
        self.start(Op::Restore, move |cfg| session(&cfg, &token).map(|r| r.into_account(&token)));
    }

    /// SIGN OUT: forgotten locally at once, revoked on the server in the background.
    pub fn sign_out(&mut self, token: &str) {
        set_current(None);
        self.logout = if token.is_empty() || guard(&self.cfg).is_err() {
            None
        } else {
            let cfg = self.cfg.clone();
            let token = token.to_string();
            Some(Pending::spawn(move || logout(&cfg, &token)))
        };
    }

    /// Call once per frame. `Some(event)` exactly once per finished request.
    pub fn poll(&mut self) -> Option<AccountEvent> {
        if let Some(l) = self.logout.as_mut() {
            if l.poll().is_some() {
                self.logout = None;
            }
        }
        let (op, pending) = self.op.as_mut()?;
        let op = *op;
        let result = pending.poll()?;
        self.op = None;
        Some(match result {
            Ok(account) => AccountEvent::SignedIn { op, account },
            Err(e) if op == Op::Restore && session_rejected(&e) => {
                set_current(None);
                AccountEvent::Rejected
            }
            Err(_) if op == Op::Restore => AccountEvent::Failed { op, message: "Could not check your saved sign-in. Playing as a guest for now.".to_string() },
            Err(e) => AccountEvent::Failed { op, message: failure_text(&e) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Save;

    #[test]
    fn credentials_are_uppercase_and_trimmed() {
        let body = credentials_body("  ada_1 ", "hunter22", "");
        assert_eq!(body["username"], "ADA_1");
        assert_eq!(body["password"], "hunter22");
        assert!(body.get("adminProof").is_none());
        assert_eq!(credentials_body("a", "b", "key")["adminProof"], "key");
        assert_eq!(token_body("tok"), json!({"sessionToken": "tok"}));
        assert_eq!(normalize_username(" Kyttu "), "KYTTU");
    }

    #[test]
    fn the_form_check_is_the_godot_one() {
        assert_eq!(validate("ab", "123456"), Err(TEXT_NEED_FIELDS));
        assert_eq!(validate("abc", "12345"), Err(TEXT_NEED_FIELDS));
        assert_eq!(validate(&"x".repeat(25), "123456"), Err(TEXT_NEED_FIELDS));
        assert_eq!(validate(" ada ", "123456"), Ok(("ADA".to_string(), "123456".to_string())));
    }

    #[test]
    fn replies_parse_with_admin_fields() {
        let p: Json = serde_json::from_str(
            "{\"ok\":true,\"username\":\"kyttu\",\"sessionToken\":\"abc\",\"admin\":true,\"adminOwner\":true,\"adminPermissions\":[\"rooms\",\"skins\"],\"progress\":{\"x\":1}}",
        )
        .unwrap();
        let r = AccountReply::from_json(&p);
        assert_eq!(r.username, "KYTTU");
        assert_eq!(r.session_token, "abc");
        assert!(r.admin && r.admin_owner && r.has_admin_owner && r.has_admin_permissions);
        assert_eq!(r.admin_permissions, vec!["rooms".to_string(), "skins".to_string()]);
        let a = r.into_account("old");
        assert_eq!(a.session_token, "abc");
        assert!(a.admin);

        let plain: Json = serde_json::from_str("{\"username\":\"ada\"}").unwrap();
        let r = AccountReply::from_json(&plain);
        assert!(!r.admin && !r.has_admin_owner && !r.has_admin_permissions && r.admin_permissions.is_empty());
        // a session check without a new token keeps the one it asked with
        assert_eq!(r.into_account("kept").session_token, "kept");
    }

    #[test]
    fn status_lines_match_the_godot_menu() {
        let account = Account { username: "ADA".into(), ..Account::default() };
        assert_eq!(AccountEvent::SignedIn { op: Op::SignIn, account }.status_text(), "Signed in as ADA.");
        assert_eq!(AccountEvent::Failed { op: Op::Register, message: "Username taken".into() }.status_text(), "Username taken");
        assert_eq!(TEXT_SIGNED_OUT, "Signed out.");
    }

    #[test]
    fn failures_read_like_last_route() {
        let e = NetError::Http { status: 401, code: "INVALID_CREDENTIALS".into(), error: "Incorrect username or password".into() };
        assert_eq!(failure_text(&e), "Incorrect username or password");
        assert!(session_rejected(&e));
        assert_eq!(failure_text(&NetError::Timeout), "ACCOUNT REQUEST FAILED (HTTP 0)");
        assert!(!session_rejected(&NetError::Http { status: 502, code: String::new(), error: String::new() }));
        assert!(!session_rejected(&NetError::Timeout));
    }

    #[test]
    fn the_session_token_round_trips_through_the_save() {
        let mut save = Save::default();
        save.debug_run = true;
        save.account_name = "ADA".to_string();
        save.session_token = "tok-123".to_string();
        let back = Save::from_json(&save.to_json());
        assert_eq!(back.account_name, "ADA");
        assert_eq!(back.session_token, "tok-123");
        // an old save without the keys loads as a guest
        let old = Save::from_json("{\"unlocked\": 2}");
        assert!(old.account_name.is_empty() && old.session_token.is_empty());
    }

    #[test]
    fn offline_and_closed_origins_are_refused_before_sending() {
        let mut cfg = BackendConfig::loopback(9);
        cfg.offline = true;
        assert_eq!(login(&cfg, "ada", "123456"), Err(NetError::Offline));
        let mut odd = BackendConfig::primary();
        odd.base_url = "http://10.0.0.5:8105".to_string();
        assert!(matches!(login(&odd, "ada", "123456"), Err(NetError::Refused(_))));
        let client = AccountClient::with_config(odd);
        assert!(client.unavailable_reason().is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn serve_once(reply: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = sock.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let want = text[..i]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if buf.len() >= i + 4 + want {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let _ = sock.write_all(reply.as_bytes());
            String::from_utf8_lossy(&buf).to_string()
        });
        (port, handle)
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn login_round_trip_against_a_loopback_server() {
        let body = "{\"ok\":true,\"username\":\"ada\",\"sessionToken\":\"tok-9\",\"admin\":false}";
        let reply: &'static str = Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_boxed_str());
        let (port, server) = serve_once(reply);
        let cfg = BackendConfig::loopback(port);
        let r = login(&cfg, " ada ", "secret1").unwrap();
        assert_eq!(r.username, "ADA");
        assert_eq!(r.session_token, "tok-9");
        let seen = server.join().unwrap();
        assert!(seen.starts_with("POST /api/account/login HTTP/1.1"));
        assert!(seen.contains("Content-Type: application/json"));
        assert!(seen.ends_with("{\"username\":\"ADA\",\"password\":\"secret1\"}"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn server_errors_carry_the_servers_text() {
        let body = "{\"ok\":false,\"code\":\"INVALID_CREDENTIALS\",\"error\":\"Incorrect username or password\"}";
        let reply: &'static str = Box::leak(format!("HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_boxed_str());
        let (port, server) = serve_once(reply);
        let e = login(&BackendConfig::loopback(port), "ada", "wrong12").unwrap_err();
        assert_eq!(e.status(), Some(401));
        assert_eq!(failure_text(&e), "Incorrect username or password");
        let _ = server.join();
    }

    /// The only test that touches the process-wide account slot and the force switch.
    #[test]
    fn the_slot_and_the_admin_gate() {
        set_current(None);
        set_force_admin(false);
        assert!(current().is_none() && !is_admin());
        set_current(Some(Account { username: "ADA".into(), admin: false, ..Account::default() }));
        assert_eq!(current().map(|a| a.username), Some("ADA".to_string()));
        assert!(!is_admin());
        set_current(Some(Account { username: "KYTTU".into(), admin: true, ..Account::default() }));
        assert!(is_admin());
        set_current(None);
        assert!(!is_admin());
        set_force_admin(true);
        assert!(is_admin());
        set_force_admin(false);
        let mut client = AccountClient::with_config(BackendConfig::loopback(9));
        client.sign_out("");
        assert!(current().is_none());
    }
}
