//! Port of `backend/guest_names.mjs`.
//!
//! Guest names (KT 2026-10-28, the 2D game's short style): one letter, a dash, four digits, e.g. "R-4821". Letters skip
//! I and O and digits skip 0 and 1, so a name read aloud or typed from a screenshot is unambiguous. The server hands them
//! out unique: a name is leased to whoever asked for it (renewed every time they ask again with it) and is never handed
//! to anyone else while the lease runs.
//!
//!   POST /api/guest/name   {previous?, token?}   -> {ok, name, token}
//! `previous` is kept when it is a valid guest name that nobody holds, or when `token` proves this client holds it (a
//! renewal); otherwise a fresh name is issued. The token is a random secret only the holder has.
//!
//! Store-and-echo: <data>/guest_names.json (`{"leases":{name:{expiry,token}}}`), atomic writes. Registered accounts can
//! never clash: their names have no dash (account_service USERNAME_PATTERN).

use crate::account_service::OrderedMap;
use crate::util::{self, base64url, field, js_number, num, random_bytes, str_or_empty, stringify, Clock, Reply};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;

pub const GUEST_LETTERS: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ";
pub const GUEST_DIGITS: &str = "23456789";
/// The JavaScript `GUEST_NAME_PATTERN` source (use [`is_guest_name`] to test).
#[cfg(test)]
pub const GUEST_NAME_PATTERN: &str = r"^[A-HJ-NP-Z]-[2-9]{4}$";
pub const LEASE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const MAX_TRIES: usize = 64;

/// A `Math.random`-like source: uniform in [0, 1).
pub type RandomSource = Arc<dyn Fn() -> f64 + Send + Sync>;

/// `Math.random()` from the operating system's random source.
pub fn math_random() -> f64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&random_bytes(8));
    (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

pub fn default_random() -> RandomSource {
    Arc::new(math_random)
}

/// `GUEST_NAME_PATTERN.test(name)`.
pub fn is_guest_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 6
        && b[0].is_ascii_uppercase()
        && b[0] != b'I'
        && b[0] != b'O'
        && b[1] == b'-'
        && b[2..].iter().all(|d| (b'2'..=b'9').contains(d))
}

fn pick(alphabet: &str, random: &dyn Fn() -> f64) -> char {
    let chars: Vec<char> = alphabet.chars().collect();
    let index = (random() * chars.len() as f64).floor();
    let index = if index.is_finite() && index >= 0.0 { (index as usize).min(chars.len() - 1) } else { 0 };
    chars[index]
}

/// `randomGuestName(random)`.
pub fn random_guest_name(random: &dyn Fn() -> f64) -> String {
    let mut name = String::new();
    name.push(pick(GUEST_LETTERS, random));
    name.push('-');
    for _ in 0..4 {
        name.push(pick(GUEST_DIGITS, random));
    }
    name
}

pub struct GuestNamesOptions {
    pub store_path: Option<PathBuf>,
    pub now: Clock,
    pub random: RandomSource,
    pub lease_ms: i64,
}

impl Default for GuestNamesOptions {
    fn default() -> Self {
        GuestNamesOptions { store_path: None, now: util::system_clock(), random: default_random(), lease_ms: LEASE_MS }
    }
}

#[derive(Clone, Debug)]
struct Lease {
    expiry: f64,
    token: String,
}

pub struct GuestNames {
    store_path: Option<PathBuf>,
    now: Clock,
    random: RandomSource,
    lease_ms: i64,
    /// name -> {expiry (ms), token}
    leases: OrderedMap<Lease>,
}

impl GuestNames {
    pub fn new(opts: GuestNamesOptions) -> GuestNames {
        let mut leases = OrderedMap::new();
        if let Some(path) = opts.store_path.as_ref().filter(|p| p.exists()) {
            let parsed = std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()));
            match parsed {
                Err(message) => util::log_error(&format!("[guest-names] unreadable {}: {message}", path.display())),
                Ok(Value::Null) => util::log_error(&format!(
                    "[guest-names] unreadable {}: Cannot read properties of null (reading 'leases')",
                    path.display()
                )),
                Ok(parsed) => {
                    let now = (opts.now)() as f64;
                    if let Some(Value::Object(entries)) = field(&parsed, "leases") {
                        for (name, lease) in entries {
                            let expiry = js_number(field(lease, "expiry"));
                            if let (true, Some(Value::String(token))) = (is_guest_name(name) && expiry > now, field(lease, "token")) {
                                leases.insert(name.clone(), Lease { expiry, token: token.clone() });
                            }
                        }
                    }
                }
            }
        }
        GuestNames { store_path: opts.store_path, now: opts.now, random: opts.random, lease_ms: opts.lease_ms, leases }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn persist(&mut self) {
        let Some(path) = self.store_path.clone() else { return };
        let now = self.now() as f64;
        self.leases.retain(|_, lease| lease.expiry > now);
        let mut out = Map::new();
        for (name, lease) in self.leases.iter() {
            out.insert(name.clone(), json!({ "expiry": num(lease.expiry), "token": lease.token }));
        }
        let text = stringify(&json!({ "leases": out }));
        if let Err(error) = util::write_atomic(&path, text.as_bytes()) {
            util::log_error(&format!("[guest-names] write failed {}: {error}", path.display()));
        }
    }

    /// `held(name)`: a live lease exists for the name.
    pub fn held(&self, name: &str) -> bool {
        self.leases.get(name).is_some_and(|lease| lease.expiry > self.now() as f64)
    }

    fn grant(&mut self, name: &str, token: Option<String>) -> String {
        let token = token.unwrap_or_else(|| base64url(&random_bytes(18)));
        let expiry = (self.now() + self.lease_ms) as f64;
        self.leases.insert(name.to_string(), Lease { expiry, token: token.clone() });
        self.persist();
        token
    }

    /// POST /api/guest/name `{previous?, token?}`.
    pub fn issue(&mut self, body: &Value) -> Reply {
        let previous = str_or_empty(field(body, "previous")).to_uppercase();
        let token = match field(body, "token") {
            Some(Value::String(t)) => t.clone(),
            _ => String::new(),
        };
        if is_guest_name(&previous) {
            let held = self.held(&previous);
            if held && !token.is_empty() && self.leases.get(&previous).is_some_and(|l| l.token == token) {
                let token = self.grant(&previous, Some(token));
                return (200, json!({ "ok": true, "name": previous, "token": token, "renewed": true }));
            }
            if !held {
                let token = self.grant(&previous, None);
                return (200, json!({ "ok": true, "name": previous, "token": token }));
            }
        }
        for _ in 0..MAX_TRIES {
            let name = random_guest_name(&*self.random);
            if self.held(&name) {
                continue; // a clash: try again
            }
            let token = self.grant(&name, None);
            return (200, json!({ "ok": true, "name": name, "token": token }));
        }
        (503, json!({ "ok": false, "code": "GUEST_NAMES_FULL", "error": "No free guest name right now; try again" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{hex, manual_clock};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn guest_names_are_one_letter_a_dash_and_four_digits_without_i_o_0_or_1() {
        for _ in 0..500 {
            let name = random_guest_name(&math_random);
            assert!(is_guest_name(&name), "{name}");
            assert!(!name.contains(['I', 'O', '0', '1']), "{name}");
        }
        for bad in ["R-482", "I-2345", "O-2345", "R-1234", "R-0234", "r-2345", "RR-2345", "R_2345", "FAR-LIGHT-658"] {
            assert!(!is_guest_name(bad), "{bad}");
        }
        // the hand-written matcher agrees with the original regex
        let re = regex::Regex::new(GUEST_NAME_PATTERN).unwrap();
        for letter in ('A'..='Z').chain(['a', '-', '1']) {
            for digits in ["2345", "9999", "1234", "234", "23456", "2a45"] {
                let name = format!("{letter}-{digits}");
                assert_eq!(is_guest_name(&name), re.is_match(&name), "{name}");
            }
        }
    }

    #[test]
    fn a_clash_is_retried_a_held_name_is_never_issued_to_someone_else() {
        // a random source that first lands on R-2222 twice, then on another name
        let sequence = [0.64, 0.0, 0.0, 0.0, 0.0, 0.64, 0.0, 0.0, 0.0, 0.0, 0.1, 0.5, 0.5, 0.5, 0.5];
        let index = Arc::new(AtomicUsize::new(0));
        let random: RandomSource = Arc::new(move || sequence[index.fetch_add(1, Ordering::SeqCst) % sequence.len()]);
        let mut names = GuestNames::new(GuestNamesOptions { random, ..Default::default() });
        let (_, first) = names.issue(&json!({}));
        let (_, second) = names.issue(&json!({}));
        assert_eq!(first["name"], "R-2222");
        assert_ne!(second["name"], first["name"], "the second guest got a clashing name");
        assert!(is_guest_name(second["name"].as_str().unwrap()));
    }

    #[test]
    fn a_guest_keeps_its_name_with_its_token_others_cannot_take_it_a_free_previous_name_is_reused() {
        let mut names = GuestNames::new(GuestNamesOptions::default());
        let (_, mine) = names.issue(&json!({}));
        let (_, again) = names.issue(&json!({ "previous": mine["name"], "token": mine["token"] }));
        assert_eq!(again["name"], mine["name"], "the holder lost its name on renewal");
        assert_eq!(again["renewed"], true);
        let (_, thief) = names.issue(&json!({ "previous": mine["name"], "token": "guess" }));
        assert_ne!(thief["name"], mine["name"], "another client took a held name");
        let (_, free) = names.issue(&json!({ "previous": "z-9876" }));
        assert_eq!(free["name"], "Z-9876", "a free previous name is kept, canonical uppercase");
        let (_, legacy) = names.issue(&json!({ "previous": "FAR-LIGHT-658" }));
        assert!(is_guest_name(legacy["name"].as_str().unwrap()), "an old-style name gets a new-style one");
        assert_eq!(mine["token"].as_str().unwrap().len(), 24);
    }

    #[test]
    fn leases_survive_a_restart_and_expire() {
        let root = std::env::temp_dir().join(format!("guests-{}", hex(&random_bytes(6))));
        std::fs::create_dir_all(&root).unwrap();
        let (clock, handle) = manual_clock(1_000_000);
        let path = root.join("guest_names.json");
        let open = || {
            GuestNames::new(GuestNamesOptions {
                store_path: Some(path.clone()),
                now: clock.clone(),
                lease_ms: 1000,
                ..Default::default()
            })
        };
        let mut first = open();
        let (_, issued) = first.issue(&json!({}));
        let name = issued["name"].as_str().unwrap().to_string();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, format!(r#"{{"leases":{{"{name}":{{"expiry":1001000,"token":"{}"}}}}}}"#, issued["token"].as_str().unwrap()));
        let reopened = open();
        assert!(reopened.held(&name), "a lease was lost on restart");
        handle.advance(5000);
        assert!(!reopened.held(&name), "an expired lease still blocks the name");
        let _ = std::fs::remove_dir_all(&root);
    }

    // Not ported here: "POST /api/guest/name issues unique names over HTTP" (server.mjs routing, router author's tests).
}
