//! Port of `backend/account_service.mjs`: player accounts (scrypt password records), bearer sessions (stored by their
//! sha256), cloud progress, renames and the owner-managed admin roster.
//!
//! Persistence is the same `accounts.json` the Node server writes:
//! `{"version":2,"accounts":[{username,salt,hash,progress,savedAt,adminPermissions,...}],"sessions":[{tokenHash,username,expiresAt}]}`.
//!
//! All methods take `&self`: the state lives behind an internal mutex so the service can be used as a
//! `&dyn SessionLookup` (expired sessions are still dropped and persisted on lookup, like the original). The slow scrypt
//! derivations run outside the lock (Node's scrypt is asynchronous, so other requests proceed meanwhile).

use crate::util::{
    self, field, from_hex, hex, js_len, js_slice, js_string_of, js_trim, num, random_bytes, scrypt_derive, sha256_hex,
    str_or_empty, stringify, timing_safe_equal, truthy, Clock, Reply, SessionLookup,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

pub const ADMIN_OWNER_USERNAMES: [&str; 2] = ["KYTTU", "AYTTU"];
pub const ADMIN_PERMISSION_NAMES: [&str; 5] = ["inventory", "player", "world", "hunters", "visual"];
pub const MIN_PASSWORD_LENGTH: usize = 6;
pub const MAX_PASSWORD_LENGTH: usize = 200;
pub const MAX_PROGRESS_BYTES: usize = 32 * 1024;
const SCRYPT_KEY_LENGTH: usize = 64;
pub const DEFAULT_SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const DECOY_SALT: &str = "last-route-decoy-salt";

// ---------------------------------------------------------------------------------------------------------------
// A string-keyed map that iterates in insertion order (a JavaScript `Map`): re-inserting an existing key keeps its
// position, removing and re-adding moves it to the end. Shared with guest_names.rs and skins.rs.

#[derive(Clone, Debug)]
pub(crate) struct OrderedMap<V> {
    entries: HashMap<String, (u64, V)>,
    order: BTreeMap<u64, String>,
    next: u64,
}

impl<V> Default for OrderedMap<V> {
    fn default() -> Self {
        OrderedMap { entries: HashMap::new(), order: BTreeMap::new(), next: 0 }
    }
}

#[allow(dead_code)]
impl<V> OrderedMap<V> {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key).map(|(_, v)| v)
    }
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.entries.get_mut(key).map(|(_, v)| v)
    }
    /// `map.set(key, value)`.
    pub(crate) fn insert(&mut self, key: String, value: V) {
        if let Some(slot) = self.entries.get_mut(&key) {
            slot.1 = value;
            return;
        }
        let seq = self.next;
        self.next += 1;
        self.order.insert(seq, key.clone());
        self.entries.insert(key, (seq, value));
    }
    /// `map.delete(key)`.
    pub(crate) fn remove(&mut self, key: &str) -> Option<V> {
        let (seq, value) = self.entries.remove(key)?;
        self.order.remove(&seq);
        Some(value)
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.order.values().map(move |key| (key, &self.entries[key].1))
    }
    pub(crate) fn keys(&self) -> Vec<String> {
        self.order.values().cloned().collect()
    }
    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries.values_mut().map(|(_, v)| v)
    }
    /// Keep only the entries for which `keep` is true (order of the rest unchanged).
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&str, &V) -> bool) {
        let doomed: Vec<String> =
            self.iter().filter(|(k, v)| !keep(k.as_str(), v)).map(|(k, _)| k.clone()).collect();
        for key in doomed {
            self.remove(&key);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Helpers.

/// `String(value)` including `undefined` (None -> "undefined").
fn js_string_any(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(v) => js_string_of(v),
    }
}

fn derive(password: &str, salt: &str) -> Vec<u8> {
    scrypt_derive(password, salt, SCRYPT_KEY_LENGTH)
}

/// `{ salt, hash }` for a new password.
fn password_record(password: &str) -> (String, String) {
    let salt = hex(&random_bytes(16));
    let hash = hex(&derive(password, &salt));
    (salt, hash)
}

fn password_matches(password: &str, salt: Option<&Value>, hash: Option<&Value>) -> bool {
    let expected = from_hex(&str_or_empty(hash));
    let actual = derive(password, &str_or_empty(salt));
    expected.len() == actual.len() && timing_safe_equal(&expected, &actual)
}

/// `usernameOf(value)`: `String(value || "").trim().toUpperCase().slice(0, 24)`.
fn username_of(value: Option<&Value>) -> String {
    username_of_str(&str_or_empty(value))
}

fn username_of_str(text: &str) -> String {
    js_slice(&js_trim(text).to_uppercase(), 24)
}

/// `/^[A-Z0-9_]{3,24}$/`.
fn valid_username(name: &str) -> bool {
    (3..=24).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn is_admin_owner_username(name: &str) -> bool {
    let canonical = username_of_str(name);
    ADMIN_OWNER_USERNAMES.contains(&canonical.as_str())
}

/// `cleanAdminPermissions(value)`: known names only, trimmed + lower-cased, de-duplicated, sorted.
fn clean_admin_permissions(value: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(Value::Array(items)) = value {
        for item in items {
            let entry = js_trim(&str_or_empty(Some(item))).to_lowercase();
            if ADMIN_PERMISSION_NAMES.contains(&entry.as_str()) && !out.contains(&entry) {
                out.push(entry);
            }
        }
    }
    out.sort();
    out
}

fn session_hash(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

/// `String.prototype.localeCompare` (ICU root collation) for usernames: `_` < digits < letters.
fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    fn weight(c: char) -> (u8, u32) {
        match c {
            '_' => (0, 0),
            '0'..='9' => (1, c as u32),
            'A'..='Z' => (2, c.to_ascii_lowercase() as u32),
            'a'..='z' => (2, c as u32),
            _ => (3, c as u32),
        }
    }
    a.chars().map(weight).cmp(b.chars().map(weight))
}

fn fail(status: u16, code: &str, error: &str) -> Reply {
    (status, json!({ "ok": false, "code": code, "error": error }))
}

fn invalid_username() -> Reply {
    fail(400, "INVALID_USERNAME", "Username must be 3-24 characters: letters, numbers, underscore")
}

fn weak_password() -> Reply {
    fail(400, "WEAK_PASSWORD", "Password must be 6-200 characters")
}

fn owner_required() -> Reply {
    fail(403, "ADMIN_OWNER_REQUIRED", "Full admin account required")
}

// ---------------------------------------------------------------------------------------------------------------

pub struct AccountServiceOptions {
    /// `storePath` (null = in memory only).
    pub store_path: Option<PathBuf>,
    pub now: Clock,
    pub session_ttl_ms: i64,
}

impl Default for AccountServiceOptions {
    fn default() -> Self {
        AccountServiceOptions { store_path: None, now: util::system_clock(), session_ttl_ms: DEFAULT_SESSION_TTL_MS }
    }
}

#[derive(Clone, Debug)]
struct Session {
    username: String,
    expires_at: f64,
}

#[derive(Default)]
struct State {
    /// username -> the stored account object (kept as JSON so unknown fields and key order survive a round trip)
    accounts: OrderedMap<Map<String, Value>>,
    /// sha256(token) -> session
    sessions: OrderedMap<Session>,
}

pub struct AccountService {
    store_path: Option<PathBuf>,
    now: Clock,
    session_ttl_ms: i64,
    state: Mutex<State>,
    load_warning: Option<String>,
}

impl AccountService {
    pub fn new(opts: AccountServiceOptions) -> AccountService {
        let mut state = State::default();
        let mut load_warning = None;
        if let Some(path) = &opts.store_path {
            match std::fs::read_to_string(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => load_warning = Some(format!("Account store was not loaded: {error}")),
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Err(error) => load_warning = Some(format!("Account store was not loaded: {error}")),
                    Ok(parsed) => Self::load(&mut state, &parsed, (opts.now)()),
                },
            }
        }
        if let Some(message) = &load_warning {
            util::log_warn(message);
        }
        AccountService {
            store_path: opts.store_path,
            now: opts.now,
            session_ttl_ms: opts.session_ttl_ms,
            state: Mutex::new(state),
            load_warning,
        }
    }

    fn load(state: &mut State, parsed: &Value, now: i64) {
        if let Some(Value::Array(accounts)) = field(parsed, "accounts") {
            for value in accounts {
                let username = username_of(field(value, "username"));
                if valid_username(&username) && truthy(field(value, "salt")) && truthy(field(value, "hash")) {
                    if let Value::Object(map) = value {
                        let mut account = map.clone();
                        account.insert("username".into(), Value::String(username.clone()));
                        state.accounts.insert(username, account);
                    }
                }
            }
        }
        if let Some(Value::Array(sessions)) = field(parsed, "sessions") {
            for value in sessions {
                let token_hash = str_or_empty(field(value, "tokenHash"));
                let username = username_of(field(value, "username"));
                let expires_at = util::number_or(field(value, "expiresAt"), 0.0);
                let well_formed =
                    token_hash.len() == 64 && token_hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if well_formed && state.accounts.contains_key(&username) && expires_at > now as f64 {
                    state.sessions.insert(token_hash, Session { username, expires_at });
                }
            }
        }
    }

    /// The warning logged while loading the store (None when it loaded or did not exist). For tests and diagnostics.
    pub fn load_warning(&self) -> Option<String> {
        self.load_warning.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn persist(&self, state: &State) {
        let Some(path) = &self.store_path else { return };
        let accounts: Vec<Value> = state.accounts.iter().map(|(_, account)| Value::Object(account.clone())).collect();
        let sessions: Vec<Value> = state
            .sessions
            .iter()
            .map(|(token_hash, session)| {
                json!({ "tokenHash": token_hash, "username": session.username, "expiresAt": num(session.expires_at) })
            })
            .collect();
        let text = stringify(&json!({ "version": 2, "accounts": accounts, "sessions": sessions }));
        if let Err(error) = util::write_atomic(path, text.as_bytes()) {
            util::log_error(&format!("Account store write failed: {error}"));
        }
    }

    fn issue_session(&self, state: &mut State, username: &str) -> String {
        let token = hex(&random_bytes(24));
        let expires_at = (self.now() + self.session_ttl_ms) as f64;
        state.sessions.insert(session_hash(&token), Session { username: username.to_string(), expires_at });
        self.persist(state);
        token
    }

    fn valid_password(password: Option<&Value>) -> bool {
        matches!(password, Some(Value::String(p)) if (MIN_PASSWORD_LENGTH..=MAX_PASSWORD_LENGTH).contains(&js_len(p)))
    }

    /// `{ admin, adminOwner, adminPermissions }` for a username.
    fn admin_identity(state: &State, username_value: &str) -> Map<String, Value> {
        let username = username_of_str(username_value);
        let owner = is_admin_owner_username(&username);
        let permissions = if owner {
            let all: Vec<Value> = ADMIN_PERMISSION_NAMES.iter().map(|n| json!(n)).collect();
            clean_admin_permissions(Some(&Value::Array(all)))
        } else {
            clean_admin_permissions(state.accounts.get(&username).and_then(|a| a.get("adminPermissions")))
        };
        let mut out = Map::new();
        out.insert("admin".into(), Value::Bool(owner || !permissions.is_empty()));
        out.insert("adminOwner".into(), Value::Bool(owner));
        out.insert("adminPermissions".into(), json!(permissions));
        out
    }

    /// `{ username, ...adminIdentity(username) }`.
    fn roster_entry(state: &State, username: &str) -> Value {
        let mut out = Map::new();
        out.insert("username".into(), json!(username));
        out.extend(Self::admin_identity(state, username));
        Value::Object(out)
    }

    /// `{ ok: true, username, ...adminIdentity(username), ...extra }`.
    fn account_result(state: &State, username: &str, extra: Vec<(&str, Value)>) -> Value {
        let mut out = Map::new();
        out.insert("ok".into(), Value::Bool(true));
        out.insert("username".into(), json!(username));
        out.extend(Self::admin_identity(state, username));
        for (key, value) in extra {
            out.insert(key.into(), value);
        }
        Value::Object(out)
    }

    /// `progress` / `savedAt` of an account as extras (an absent field stays absent, like `undefined` in JSON).
    fn progress_extras(account: Option<&Map<String, Value>>) -> Vec<(&'static str, Value)> {
        let mut extra = Vec::new();
        if let Some(account) = account {
            if let Some(progress) = account.get("progress") {
                extra.push(("progress", progress.clone()));
            }
            if let Some(saved_at) = account.get("savedAt") {
                extra.push(("savedAt", saved_at.clone()));
            }
        }
        extra
    }

    fn session_user(&self, state: &mut State, token: &str) -> Option<String> {
        let key = session_hash(token);
        let session = state.sessions.get(&key)?.clone();
        if session.expires_at <= self.now() as f64 {
            state.sessions.remove(&key);
            self.persist(state);
            return None;
        }
        Some(session.username)
    }

    /// POST /api/account/register. `allow_developer_names` = server.mjs `developerRequest(body)`.
    pub fn register(&self, body: &Value, allow_developer_names: bool) -> Reply {
        let username = username_of(field(body, "username"));
        if !valid_username(&username) {
            return invalid_username();
        }
        if is_admin_owner_username(&username) && !allow_developer_names {
            return fail(403, "RESERVED_USERNAME", "That username is reserved");
        }
        let password = field(body, "password");
        if !Self::valid_password(password) {
            return weak_password();
        }
        let taken = || fail(409, "USERNAME_TAKEN", "That username is already taken");
        if self.lock().accounts.contains_key(&username) {
            return taken();
        }
        let (salt, hash) = password_record(&js_string_any(password));
        let mut state = self.lock();
        if state.accounts.contains_key(&username) {
            return taken();
        }
        let mut account = Map::new();
        account.insert("username".into(), json!(username));
        account.insert("salt".into(), json!(salt));
        account.insert("hash".into(), json!(hash));
        account.insert("progress".into(), Value::Null);
        account.insert("savedAt".into(), json!(0));
        account.insert("adminPermissions".into(), json!([]));
        state.accounts.insert(username.clone(), account);
        self.persist(&state);
        let token = self.issue_session(&mut state, &username);
        (200, Self::account_result(&state, &username, vec![("sessionToken", json!(token))]))
    }

    /// POST /api/account/login.
    pub fn login(&self, body: &Value) -> Reply {
        let username = username_of(field(body, "username"));
        let password = js_string_any(field(body, "password"));
        let account = self.lock().accounts.get(&username).cloned();
        let valid = match &account {
            Some(account) => password_matches(&password, account.get("salt"), account.get("hash")),
            None => {
                let _ = derive(&password, DECOY_SALT);
                false
            }
        };
        if !valid {
            return fail(401, "INVALID_CREDENTIALS", "Incorrect username or password");
        }
        let mut state = self.lock();
        let token = self.issue_session(&mut state, &username);
        let current = state.accounts.get(&username).cloned().or(account);
        let mut extra = vec![("sessionToken", json!(token))];
        extra.extend(Self::progress_extras(current.as_ref()));
        (200, Self::account_result(&state, &username, extra))
    }

    /// `sessionUsername(token)`: the canonical username of a live session (expired sessions are dropped).
    /// `token` is `String(token || "")` of whatever the caller received.
    pub fn session_username(&self, token: &str) -> Option<String> {
        let mut state = self.lock();
        self.session_user(&mut state, token)
    }

    /// `sessionUsername(body.sessionToken)` for a raw JSON value.
    pub fn session_username_of(&self, token: Option<&Value>) -> Option<String> {
        self.session_username(&str_or_empty(token))
    }

    /// POST /api/account/session.
    pub fn current_session(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let Some(username) = self.session_user(&mut state, &token) else {
            return fail(401, "NO_SESSION", "Session expired; sign in again");
        };
        let mut extra = vec![("sessionToken", json!(js_string_any(field(body, "sessionToken"))))];
        extra.extend(Self::progress_extras(state.accounts.get(&username)));
        (200, Self::account_result(&state, &username, extra))
    }

    /// POST /api/account/save. (`JSON.stringify` of a parsed JSON value cannot throw, so the original's 400
    /// INVALID_PROGRESS branch for cyclic/BigInt values has no Rust equivalent.)
    pub fn save(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let Some(username) = self.session_user(&mut state, &token) else {
            return fail(401, "NO_SESSION", "Sign in to save progress");
        };
        let progress = field(body, "progress").cloned().unwrap_or(Value::Null);
        if stringify(&progress).len() > MAX_PROGRESS_BYTES {
            return fail(413, "PROGRESS_TOO_LARGE", "Save data is too large");
        }
        let saved_at = self.now();
        let Some(account) = state.accounts.get_mut(&username) else {
            return fail(401, "NO_SESSION", "Sign in to save progress");
        };
        account.insert("progress".into(), progress.clone());
        account.insert("savedAt".into(), json!(saved_at));
        self.persist(&state);
        (200, json!({ "ok": true, "progress": progress, "savedAt": saved_at }))
    }

    /// POST /api/account/load.
    pub fn load_progress(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let Some(username) = self.session_user(&mut state, &token) else {
            return fail(401, "NO_SESSION", "Sign in to load progress");
        };
        let mut out = Map::new();
        out.insert("ok".into(), Value::Bool(true));
        for (key, value) in Self::progress_extras(state.accounts.get(&username)) {
            out.insert(key.into(), value);
        }
        (200, Value::Object(out))
    }

    /// POST /api/account/logout.
    pub fn logout(&self, body: &Value) -> Reply {
        let key = session_hash(&str_or_empty(field(body, "sessionToken")));
        let mut state = self.lock();
        if state.sessions.remove(&key).is_some() {
            self.persist(&state);
        }
        (200, json!({ "ok": true }))
    }

    /// POST /api/account/password. Drops every other session of the account, keeps the one that asked.
    pub fn change_password(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let current_session_hash = session_hash(&token);
        let Some(username) = self.session_username(&token) else {
            return fail(401, "NO_SESSION", "Sign in to change your password");
        };
        let password = field(body, "password");
        if !Self::valid_password(password) {
            return weak_password();
        }
        let (salt, hash) = password_record(&js_string_any(password));
        let mut state = self.lock();
        let Some(account) = state.accounts.get_mut(&username) else {
            return fail(401, "NO_SESSION", "Sign in to change your password");
        };
        account.insert("salt".into(), json!(salt));
        account.insert("hash".into(), json!(hash));
        // Password replacement is also account recovery: any other device holding a bearer token must lose access.
        // Keep the session which authorized this request so the player is not signed out of the client doing it.
        state.sessions.retain(|token_hash, session| !(session.username == username && token_hash != current_session_hash));
        self.persist(&state);
        (200, json!({ "ok": true, "username": username }))
    }

    /// POST /api/account/rename. `allow_developer_names` = server.mjs `developerRequest(body)`.
    pub fn rename(&self, body: &Value, allow_developer_names: bool) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let Some(current_username) = self.session_user(&mut state, &token) else {
            return fail(401, "NO_SESSION", "Sign in to rename your account");
        };
        let username = username_of(field(body, "username"));
        if !valid_username(&username) {
            return invalid_username();
        }
        if is_admin_owner_username(&current_username) && username != current_username {
            return fail(403, "OWNER_IMMUTABLE", "Full admin account usernames cannot be changed");
        }
        if is_admin_owner_username(&username) && username != current_username && !allow_developer_names {
            return fail(403, "RESERVED_USERNAME", "That username is reserved");
        }
        if username != current_username && state.accounts.contains_key(&username) {
            return fail(409, "USERNAME_TAKEN", "That username is already taken");
        }
        if username != current_username {
            if let Some(mut account) = state.accounts.remove(&current_username) {
                account.insert("username".into(), json!(username));
                state.accounts.insert(username.clone(), account);
            }
            for session in state.sessions.values_mut() {
                if session.username == current_username {
                    session.username = username.clone();
                }
            }
            self.persist(&state);
        }
        let echoed = js_string_any(field(body, "sessionToken"));
        (200, Self::account_result(&state, &username, vec![("sessionToken", json!(echoed))]))
    }

    /// `sessionAdminIdentity(token)`: `{username, admin, adminOwner, adminPermissions}` or None.
    pub fn session_admin_identity(&self, token: &str) -> Option<Value> {
        let mut state = self.lock();
        let username = self.session_user(&mut state, token)?;
        Some(Self::roster_entry(&state, &username))
    }

    /// POST /api/admin/roster.
    pub fn list_admin_roster(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let requester = self.session_user(&mut state, &token).unwrap_or_default();
        if !is_admin_owner_username(&requester) {
            return owner_required();
        }
        let mut owners: Vec<&str> = ADMIN_OWNER_USERNAMES.to_vec();
        owners.sort();
        let mut roster: Vec<Value> = owners.iter().map(|name| Self::roster_entry(&state, name)).collect();
        for (username, account) in state.accounts.iter() {
            if is_admin_owner_username(username) {
                continue;
            }
            if !clean_admin_permissions(account.get("adminPermissions")).is_empty() {
                roster.push(Self::roster_entry(&state, username));
            }
        }
        roster.sort_by(|a, b| {
            locale_compare(
                a.get("username").and_then(Value::as_str).unwrap_or(""),
                b.get("username").and_then(Value::as_str).unwrap_or(""),
            )
        });
        (200, json!({ "ok": true, "roster": roster, "availablePermissions": ADMIN_PERMISSION_NAMES }))
    }

    /// POST /api/admin/roster/set.
    pub fn set_admin_roster(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let requester = self.session_user(&mut state, &token).unwrap_or_default();
        if !is_admin_owner_username(&requester) {
            return owner_required();
        }
        let username = username_of(field(body, "username"));
        if !valid_username(&username) || !state.accounts.contains_key(&username) {
            return fail(404, "ACCOUNT_NOT_FOUND", "Admin target account was not found");
        }
        if is_admin_owner_username(&username) {
            return fail(403, "OWNER_IMMUTABLE", "Built-in owner permissions cannot be changed");
        }
        let permissions = clean_admin_permissions(field(body, "permissions"));
        if permissions.is_empty() {
            return fail(400, "NO_PERMISSIONS", "Choose at least one valid admin permission");
        }
        if let Some(account) = state.accounts.get_mut(&username) {
            account.insert("adminPermissions".into(), json!(permissions));
        }
        self.persist(&state);
        (200, json!({ "ok": true, "entry": Self::roster_entry(&state, &username) }))
    }

    /// POST /api/admin/roster/remove.
    pub fn remove_admin_roster(&self, body: &Value) -> Reply {
        let token = str_or_empty(field(body, "sessionToken"));
        let mut state = self.lock();
        let requester = self.session_user(&mut state, &token).unwrap_or_default();
        if !is_admin_owner_username(&requester) {
            return owner_required();
        }
        let username = username_of(field(body, "username"));
        if is_admin_owner_username(&username) {
            return fail(403, "OWNER_IMMUTABLE", "Built-in owners cannot be removed");
        }
        let Some(account) = state.accounts.get_mut(&username) else {
            return fail(404, "ACCOUNT_NOT_FOUND", "Admin target account was not found");
        };
        account.insert("adminPermissions".into(), json!([]));
        self.persist(&state);
        (200, json!({ "ok": true, "username": username, "removed": true }))
    }

    /// `isAdminOwner(name)`: one of the built-in full-admin usernames (whether or not the account exists).
    pub fn is_admin_owner(&self, name: &str) -> bool {
        is_admin_owner_username(name)
    }

    /// `accountExists(name)`.
    pub fn account_exists(&self, name: &str) -> bool {
        self.lock().accounts.contains_key(&username_of_str(name))
    }
}

impl SessionLookup for AccountService {
    fn session_username(&self, token: &str) -> Option<String> {
        AccountService::session_username(self, token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{manual_clock, ManualClock};
    use std::path::Path;

    const PASSWORD: &str = "correct horse battery";

    struct App {
        root: PathBuf,
        store_path: PathBuf,
        accounts: AccountService,
        clock: Clock,
        handle: ManualClock,
    }

    impl App {
        fn now(&self) -> i64 {
            self.handle.get()
        }
        fn reopen(&self) -> AccountService {
            AccountService::new(AccountServiceOptions {
                store_path: Some(self.store_path.clone()),
                now: self.clock.clone(),
                ..Default::default()
            })
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("{prefix}{}", hex(&random_bytes(6))));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn open_accounts() -> App {
        let root = temp_root("accounts-");
        let store_path = root.join("nested").join("accounts.json");
        let (clock, handle) = manual_clock(1_700_000_000_000);
        let accounts = AccountService::new(AccountServiceOptions {
            store_path: Some(store_path.clone()),
            now: clock.clone(),
            ..Default::default()
        });
        App { root, store_path, accounts, clock, handle }
    }

    fn is_hex_of_len(text: &str, len: Option<usize>) -> bool {
        len.map_or(!text.is_empty(), |n| text.len() == n) && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    fn token(body: &Value) -> String {
        body["sessionToken"].as_str().unwrap().to_string()
    }

    fn names(roster: &Value) -> Vec<String> {
        roster.as_array().unwrap().iter().map(|e| e["username"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn admin_permission_names_are_the_five_owner_assignable_keys() {
        assert_eq!(ADMIN_PERMISSION_NAMES, ["inventory", "player", "world", "hunters", "visual"]);
    }

    #[test]
    fn register_login_session_progress_logout_and_password_change_stay_on_the_session_that_asked() {
        let app = open_accounts();
        let a = &app.accounts;
        assert_eq!(a.register(&json!({ "username": "no", "password": PASSWORD }), false).0, 400);
        assert_eq!(a.register(&json!({ "username": "kyttu", "password": PASSWORD }), false).0, 403);
        assert!(!a.account_exists("KYTTU"));
        assert_eq!(a.register(&json!({ "username": "ada", "password": "short" }), false).0, 400);
        assert_eq!(a.register(&json!({ "username": "ada", "password": "x".repeat(201) }), false).0, 400);
        assert_eq!(a.register(&json!({ "username": "ada", "password": 123456 }), false).0, 400);

        let (status, created) = a.register(&json!({ "username": " ada ", "password": PASSWORD }), false);
        assert_eq!(status, 200);
        assert_eq!(created["username"], "ADA");
        assert_eq!(created["admin"], false);
        assert_eq!(created["adminOwner"], false);
        assert_eq!(created["adminPermissions"], json!([]));
        assert!(is_hex_of_len(&token(&created), Some(48)));
        let created_token = token(&created);

        assert_eq!(a.register(&json!({ "username": "ada", "password": PASSWORD }), false).0, 409);
        assert_eq!(a.login(&json!({ "username": "missing", "password": PASSWORD })).0, 401);
        assert_eq!(a.login(&json!({ "username": "ada", "password": "not-the-password" })).0, 401);

        let (login_status, logged_in) = a.login(&json!({ "username": "Ada", "password": PASSWORD }));
        assert_eq!(login_status, 200);
        assert_eq!(logged_in["username"], "ADA");
        assert_ne!(token(&logged_in), created_token);
        assert_eq!(logged_in["progress"], Value::Null);
        let logged_in_token = token(&logged_in);

        let saved = a.save(&json!({ "sessionToken": created_token, "progress": { "floor": 2, "note": "ok" } }));
        assert_eq!(saved.0, 200);
        assert_eq!(saved.1["savedAt"], json!(app.now()));
        assert_eq!(a.load_progress(&json!({ "sessionToken": created_token })).1["progress"], json!({ "floor": 2, "note": "ok" }));
        // (The original's cyclic-object and BigInt progress cases cannot be expressed as JSON values.)
        let too_big = "z".repeat(32 * 1024);
        assert_eq!(a.save(&json!({ "sessionToken": created_token, "progress": too_big })).0, 413);
        assert_eq!(a.load_progress(&json!({ "sessionToken": logged_in_token })).1["progress"], json!({ "floor": 2, "note": "ok" }));

        let current = a.current_session(&json!({ "sessionToken": created_token }));
        assert_eq!(current.0, 200);
        assert_eq!(current.1["sessionToken"], json!(created_token));
        assert_eq!(current.1["progress"]["floor"], 2);

        assert_eq!(a.logout(&json!({ "sessionToken": "not-a-session" })).0, 200);
        assert_eq!(a.logout(&json!({ "sessionToken": logged_in_token })).0, 200);
        assert_eq!(a.current_session(&json!({ "sessionToken": logged_in_token })).0, 401);
        assert_eq!(a.current_session(&json!({ "sessionToken": created_token })).0, 200);

        let (again_status, again) = a.login(&json!({ "username": "ADA", "password": PASSWORD }));
        assert_eq!(again_status, 200);
        assert_eq!(again["progress"]["floor"], 2);
        let changed = a.change_password(&json!({ "sessionToken": created_token, "password": "new-secret" }));
        assert_eq!(changed.0, 200);
        assert_eq!(a.current_session(&json!({ "sessionToken": created_token })).0, 200, "the caller keeps its session");
        assert_eq!(a.current_session(&json!({ "sessionToken": token(&again) })).0, 401, "other sessions are dropped");
        assert_eq!(a.login(&json!({ "username": "ADA", "password": PASSWORD })).0, 401);
        assert_eq!(a.login(&json!({ "username": "ADA", "password": "new-secret" })).0, 200);
        assert_eq!(a.change_password(&json!({ "sessionToken": created_token, "password": "short" })).0, 400);
    }

    #[test]
    fn long_usernames_are_clipped_to_24_so_a_different_25th_character_still_collides() {
        let app = open_accounts();
        let base = "A".repeat(24);
        let (status, created) = app.accounts.register(&json!({ "username": format!("{base}B"), "password": PASSWORD }), false);
        assert_eq!(status, 200);
        assert_eq!(created["username"], json!(base));
        let (taken, body) = app.accounts.register(&json!({ "username": format!("{base}C"), "password": PASSWORD }), false);
        assert_eq!(taken, 409);
        assert_eq!(body["code"], "USERNAME_TAKEN");
    }

    #[test]
    fn rename_keeps_the_session_and_progress_and_blocks_reserved_or_taken_names() {
        let app = open_accounts();
        let a = &app.accounts;
        let owner = token(&a.register(&json!({ "username": "KYTTU", "password": PASSWORD }), true).1);
        let player = token(&a.register(&json!({ "username": "PLAYER", "password": PASSWORD }), false).1);
        a.save(&json!({ "sessionToken": player, "progress": { "floor": 4 } }));

        let owner_rename = a.rename(&json!({ "sessionToken": owner, "username": "OTHER" }), false);
        assert_eq!(owner_rename.0, 403);
        assert_eq!(owner_rename.1["code"], "OWNER_IMMUTABLE");
        assert_eq!(a.session_username(&owner).as_deref(), Some("KYTTU"));

        assert_eq!(a.rename(&json!({ "sessionToken": player, "username": "kyttu" }), false).1["code"], "RESERVED_USERNAME");
        assert_eq!(a.rename(&json!({ "sessionToken": player, "username": "no" }), false).0, 400);
        let second = token(&a.register(&json!({ "username": "OTHER", "password": PASSWORD }), false).1);
        assert_eq!(a.rename(&json!({ "sessionToken": player, "username": "other" }), false).0, 409);

        let renamed = a.rename(&json!({ "sessionToken": player, "username": "player" }), false);
        assert_eq!(renamed.0, 200);
        assert_eq!(renamed.1["username"], "PLAYER");
        let next = a.rename(&json!({ "sessionToken": player, "username": "runner" }), false);
        assert_eq!(next.0, 200);
        assert_eq!(next.1["username"], "RUNNER");
        assert_eq!(next.1["sessionToken"], json!(player));
        assert_eq!(a.load_progress(&json!({ "sessionToken": player })).1["progress"]["floor"], 4);
        assert_eq!(a.login(&json!({ "username": "PLAYER", "password": PASSWORD })).0, 401);
        assert_eq!(a.login(&json!({ "username": "runner", "password": PASSWORD })).0, 200);
        assert_eq!(a.session_username(&second).as_deref(), Some("OTHER"));
        assert_eq!(a.rename(&json!({ "sessionToken": "missing" }), false).0, 401);
    }

    #[test]
    fn only_a_built_in_owner_can_grant_list_and_clear_admin_permissions() {
        let app = open_accounts();
        let a = &app.accounts;
        let owner = token(&a.register(&json!({ "username": "AYTTU", "password": PASSWORD }), true).1);
        a.register(&json!({ "username": "KYTTU", "password": PASSWORD }), true);
        let player = token(&a.register(&json!({ "username": "MOD", "password": PASSWORD }), false).1);
        assert!(a.is_admin_owner("ayttu"));
        assert!(!a.is_admin_owner("MOD"));
        assert_eq!(a.session_admin_identity(&player).unwrap()["admin"], false);
        assert_eq!(a.list_admin_roster(&json!({ "sessionToken": player })).0, 403);
        assert_eq!(a.list_admin_roster(&json!({})).0, 403);

        let listed = a.list_admin_roster(&json!({ "sessionToken": owner }));
        assert_eq!(listed.0, 200);
        assert_eq!(names(&listed.1["roster"]), ["AYTTU", "KYTTU"]);
        assert_eq!(listed.1["roster"][0]["adminOwner"], true);
        assert_eq!(listed.1["availablePermissions"], json!(ADMIN_PERMISSION_NAMES));

        let empty = a.set_admin_roster(&json!({ "sessionToken": owner, "username": "mod", "permissions": ["nope"] }));
        assert_eq!(empty.0, 400);
        assert_eq!(empty.1["code"], "NO_PERMISSIONS");
        assert_eq!(a.set_admin_roster(&json!({ "sessionToken": owner, "username": "missing", "permissions": ["world"] })).0, 404);
        assert_eq!(
            a.set_admin_roster(&json!({ "sessionToken": owner, "username": "KYTTU", "permissions": ["world"] })).1["code"],
            "OWNER_IMMUTABLE"
        );
        assert_eq!(a.set_admin_roster(&json!({ "sessionToken": player, "username": "MOD", "permissions": ["world"] })).0, 403);

        let granted = a.set_admin_roster(&json!({
            "sessionToken": owner, "username": " mod ", "permissions": [" Visual ", "player", "nope", "player"],
        }));
        assert_eq!(granted.0, 200);
        assert_eq!(granted.1["entry"]["adminPermissions"], json!(["player", "visual"]));
        assert_eq!(granted.1["entry"]["admin"], true);
        assert_eq!(granted.1["entry"]["adminOwner"], false);
        let identity = a.session_admin_identity(&player).unwrap();
        assert_eq!(identity["adminPermissions"], json!(["player", "visual"]));
        let roster = a.list_admin_roster(&json!({ "sessionToken": owner })).1;
        assert_eq!(names(&roster["roster"]), ["AYTTU", "KYTTU", "MOD"]);

        assert_eq!(a.remove_admin_roster(&json!({ "sessionToken": owner, "username": "AYTTU" })).1["code"], "OWNER_IMMUTABLE");
        assert_eq!(a.remove_admin_roster(&json!({ "sessionToken": owner, "username": "nobody" })).0, 404);
        assert_eq!(a.remove_admin_roster(&json!({ "sessionToken": player, "username": "MOD" })).0, 403);
        let removed = a.remove_admin_roster(&json!({ "sessionToken": owner, "username": "mod" }));
        assert_eq!(removed.0, 200);
        assert_eq!(removed.1["removed"], true);
        assert_eq!(a.session_admin_identity(&player).unwrap()["adminPermissions"], json!([]));
    }

    #[test]
    fn the_store_reloads_accounts_and_live_sessions_skips_junk_and_warns_on_a_corrupt_file() {
        let app = open_accounts();
        let created = token(&app.accounts.register(&json!({ "username": "ADA", "password": PASSWORD }), false).1);
        app.accounts.save(&json!({ "sessionToken": created, "progress": { "floor": 1 } }));
        let raw_text = std::fs::read_to_string(&app.store_path).unwrap();
        let raw: Value = serde_json::from_str(&raw_text).unwrap();
        assert_eq!(raw["version"], 2);
        assert_eq!(raw["accounts"][0]["username"], "ADA");
        assert!(is_hex_of_len(raw["accounts"][0]["hash"].as_str().unwrap(), None));
        assert!(is_hex_of_len(raw["sessions"][0]["tokenHash"].as_str().unwrap(), Some(64)));
        assert!(!raw_text.contains(PASSWORD));
        assert!(!raw_text.contains(&created));

        let reopened = app.reopen();
        assert_eq!(reopened.current_session(&json!({ "sessionToken": created })).0, 200);
        assert_eq!(reopened.load_progress(&json!({ "sessionToken": created })).1["progress"]["floor"], 1);

        app.handle.advance(31 * 24 * 60 * 60 * 1000);
        assert_eq!(app.accounts.current_session(&json!({ "sessionToken": created })).0, 401);
        let expired = app.reopen();
        assert_eq!(expired.current_session(&json!({ "sessionToken": created })).0, 401);
        assert!(expired.account_exists("ada"));

        std::fs::write(&app.store_path, "{").unwrap();
        let broken = AccountService::new(AccountServiceOptions { store_path: Some(app.store_path.clone()), ..Default::default() });
        assert!(!broken.account_exists("ADA"));
        assert!(broken.load_warning().unwrap().contains("not loaded"));

        let missing_path = app.root.join("missing").join("accounts.json");
        let missing = AccountService::new(AccountServiceOptions { store_path: Some(missing_path.clone()), ..Default::default() });
        assert_eq!(missing.load_warning(), None);
        assert!(!Path::new(&missing_path).exists());
    }

    #[test]
    fn persisted_file_keeps_the_node_layout() {
        // extra (unknown) account fields and key order survive a load + save round trip, like `{ ...value, username }`
        let app = open_accounts();
        let text = r#"{"version":2,"accounts":[{"username":"ada","salt":"s","hash":"00","progress":null,"savedAt":0,"adminPermissions":[],"extra":1}],"sessions":[]}"#;
        std::fs::create_dir_all(app.store_path.parent().unwrap()).unwrap();
        std::fs::write(&app.store_path, text).unwrap();
        let service = app.reopen();
        assert!(service.account_exists("ADA"));
        service.persist(&service.lock());
        let written = std::fs::read_to_string(&app.store_path).unwrap();
        assert_eq!(
            written,
            r#"{"version":2,"accounts":[{"username":"ADA","salt":"s","hash":"00","progress":null,"savedAt":0,"adminPermissions":[],"extra":1}],"sessions":[]}"#
        );
        assert_eq!(locale_compare("A_", "A1"), std::cmp::Ordering::Less);
        assert_eq!(locale_compare("A1", "AA"), std::cmp::Ordering::Less);
        assert_eq!(locale_compare("A", "A_"), std::cmp::Ordering::Less);
    }
}
