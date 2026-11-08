//! Which server the client talks to: `backend_client.gd`'s origin policy, unchanged.
//!
//! * Every build uses the official online service: `PRIMARY_BASE_URL` on the owner's VPS.
//! * `--backend=URL` (`_apply_loopback_override`) may point a build at a LOCAL backend only:
//!   `http://127.0.0.1:PORT` or `http://localhost:PORT`. A debug build may also name the TEST
//!   instance in `TEST_BASE_URLS` (exact match). Anything else is ignored with a warning, so a
//!   shipped game can never be redirected to an arbitrary public HTTP origin.
//! * Accounts are only enabled for the primary origin, `https://` origins, the TEST instance in a
//!   debug build, and loopback with an exact host boundary (`localhost.example` is not loopback).
//!
//! The web build's `window.LR_API_BASE` forwarder and the `test_server` export feature have no
//! equivalent in this desktop port. The gameplay relay host/port are NOT configured here: the
//! directory returns them in every room answer (`room.address` / `room.port`, `transport:
//! "relay"`); the production server defaults them to `lastroute.kyttuslab.org` / UDP `1105`
//! (`LASTROUTE_RELAY_HOST` / `LASTROUTE_RELAY_PORT` in server.mjs).
//!
//! Constructor args ([`BackendConfig::loopback`], [`BackendConfig::from_args`]) replace Godot's
//! `use_test_base_url()`; they obey the same policy, so tests point at 127.0.0.1.

/// Default online directory: the same server as the web build (vps2, behind Caddy at /api).
pub const PRIMARY_BASE_URL: &str = "https://lastrouterust.games.kyttuslab.org";
/// Dev/test only: the separate TEST instance on the same VPS. Debug builds only, exact match.
pub const TEST_BASE_URLS: [&str; 1] = ["http://lastroute.kyttuslab.org:8205"];
/// Directory heartbeat for a hosted room (`HEARTBEAT_SECONDS`).
pub const HEARTBEAT_SECONDS: f64 = 20.0;
/// Server defaults, for documentation and tests (the client reads the real ones from the room).
pub const DEFAULT_RELAY_HOST: &str = "lastroute.kyttuslab.org";
pub const DEFAULT_RELAY_PORT: u16 = 1105;
pub const DEFAULT_HTTP_PORT: u16 = 8105;
/// `check_health()` waits this long (5 s; 25 s on the web build).
pub const HEALTH_TIMEOUT_SECS: u64 = 5;
/// `request_guest_name()` waits this long.
pub const GUEST_NAME_TIMEOUT_SECS: u64 = 8;
/// `release_host_handoff()` waits this long.
pub const RELEASE_HANDOFF_TIMEOUT_SECS: u64 = 6;
/// HTTP statuses that mean the server REJECTED a stored session (it is cleared); anything else
/// (no network, a restart, a 5xx) keeps the token.
pub const SESSION_REJECTED_CODES: [u16; 4] = [400, 401, 403, 404];
pub const OFFLINE_NOTE: &str = "OFFLINE MODE IS ON // THE SERVER IS NOT BEING CONTACTED. GO ONLINE TO PLAY WITH FRIENDS.";
pub const OFFLINE_JOIN_MESSAGE: &str = "OFFLINE MODE // GO ONLINE TO JOIN ROOMS";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendConfig {
    /// No trailing slash.
    pub base_url: String,
    /// True while a debug run talks to the TEST instance (menus show a TEST SERVER badge).
    pub test_server_active: bool,
    /// `--offline`: the server is never contacted.
    pub offline: bool,
    /// `--profile=NAME`: per-instance account file (`last_route_account_NAME.cfg`).
    pub profile: String,
}

impl Default for BackendConfig {
    fn default() -> Self {
        BackendConfig { base_url: default_base_url(), test_server_active: false, offline: false, profile: String::new() }
    }
}

/// The directory the build talks to. Native: the primary origin. The browser build has no fixed origin of its own: the
/// page serves the directory from the same origin (or `window.LR_API_BASE` when the page names one), so `/api/...` and
/// the relay at `/api/ws` are the page's own, as in the Godot web build.
#[cfg(not(target_arch = "wasm32"))]
fn default_base_url() -> String {
    PRIMARY_BASE_URL.into()
}

#[cfg(target_arch = "wasm32")]
fn default_base_url() -> String {
    super::web_bridge::api_base()
}

/// What `--backend=` (or a constructor) asked for and whether it was accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Override {
    Loopback(String),
    TestInstance(String),
    Ignored(String),
}

/// `_apply_loopback_override` for one candidate origin. `debug_build` is `cfg!(debug_assertions)`
/// in the game (Godot's `OS.is_debug_build()`).
pub fn classify_override(candidate: &str, debug_build: bool) -> Override {
    let candidate = candidate.trim_end_matches('/').to_string();
    let lowered = candidate.to_ascii_lowercase();
    if lowered.starts_with("http://127.0.0.1:") || lowered.starts_with("http://localhost:") {
        Override::Loopback(candidate)
    } else if debug_build && TEST_BASE_URLS.contains(&lowered.as_str()) {
        Override::TestInstance(lowered)
    } else {
        Override::Ignored(candidate)
    }
}

impl BackendConfig {
    /// The primary service (what every release build uses).
    pub fn primary() -> BackendConfig {
        BackendConfig::default()
    }

    /// A local backend on `127.0.0.1:port` (tests, a developer's own server).
    pub fn loopback(port: u16) -> BackendConfig {
        BackendConfig { base_url: format!("http://127.0.0.1:{port}"), ..BackendConfig::default() }
    }

    /// Applies an origin through the original's policy; a refused origin leaves the primary in place
    /// and returns the warning the original pushes.
    pub fn with_override(mut self, candidate: &str, debug_build: bool) -> (BackendConfig, Option<String>) {
        match classify_override(candidate, debug_build) {
            Override::Loopback(url) => {
                self.base_url = url;
                (self, None)
            }
            Override::TestInstance(url) => {
                self.base_url = url;
                self.test_server_active = true;
                (self, None)
            }
            Override::Ignored(_) => (
                self,
                Some("ignored --backend override: only loopback origins (or the test instance in a debug build) are allowed".into()),
            ),
        }
    }

    /// Reads `--backend=URL`, `--offline` and `--profile=NAME` from the command line, exactly as the
    /// original reads its user args.
    pub fn from_args<I: IntoIterator<Item = String>>(args: I, debug_build: bool) -> (BackendConfig, Vec<String>) {
        let mut config = BackendConfig::default();
        let mut warnings = Vec::new();
        for arg in args {
            if arg == "--offline" {
                config.offline = true;
            } else if let Some(v) = arg.strip_prefix("--backend=") {
                let (c, w) = config.clone().with_override(v, debug_build);
                config = c;
                warnings.extend(w);
            } else if let Some(v) = arg.strip_prefix("--profile=") {
                config.profile = v.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            }
        }
        (config, warnings)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn origin(&self) -> String {
        self.base_url.trim_end_matches('/').to_ascii_lowercase()
    }

    pub fn is_primary(&self) -> bool {
        self.origin() == PRIMARY_BASE_URL.to_ascii_lowercase()
    }

    /// `_account_transport_is_protected`.
    pub fn account_transport_is_protected(&self, debug_build: bool) -> bool {
        let origin = self.origin();
        if origin.starts_with("https://") {
            return true;
        }
        if self.test_server_active && debug_build {
            return true;
        }
        origin == "http://127.0.0.1"
            || origin.starts_with("http://127.0.0.1:")
            || origin == "http://localhost"
            || origin.starts_with("http://localhost:")
    }

    /// `accounts_available`.
    pub fn accounts_available(&self, debug_build: bool) -> bool {
        !self.offline && (self.account_transport_is_protected(debug_build) || self.is_primary())
    }

    /// TODO 1075: the origin may be used at all. `https://` always; plain `http://` only for loopback with an exact host
    /// boundary, or the TEST instance in a debug build. A release build refuses every other http origin.
    pub fn origin_transport_ok(&self, debug_build: bool) -> bool {
        let origin = self.origin();
        if origin.starts_with("https://") {
            return true;
        }
        let loopback = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
            .iter()
            .any(|p| origin == *p || origin.strip_prefix(p).is_some_and(|rest| rest.starts_with(':')));
        loopback || (debug_build && TEST_BASE_URLS.contains(&origin.as_str()))
    }

    /// `account_transport_warning`.
    pub fn account_transport_warning(&self, debug_build: bool) -> &'static str {
        if self.accounts_available(debug_build) {
            ""
        } else {
            "ACCOUNT SIGN-IN IS DISABLED FOR THIS BACKEND ORIGIN"
        }
    }

    /// `_session_section`: sessions are stored per server.
    pub fn session_section(&self) -> String {
        format!("account:{}", self.origin())
    }

    /// `directory_label`.
    pub fn directory_label(&self) -> &'static str {
        if self.offline {
            "ROOM DIRECTORY DISABLED"
        } else {
            "PRIMARY ROOM DIRECTORY"
        }
    }

    /// `RunProfile.path`: `name.cfg` -> `name_PROFILE.cfg`.
    pub fn profile_file(&self, base: &str) -> String {
        if self.profile.is_empty() {
            base.to_string()
        } else {
            format!("{}_{}.cfg", base.trim_end_matches(".cfg"), self.profile)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_policy() {
        assert_eq!(classify_override("http://127.0.0.1:18105/", false), Override::Loopback("http://127.0.0.1:18105".into()));
        assert_eq!(classify_override("http://LOCALHOST:9", false), Override::Loopback("http://LOCALHOST:9".into()));
        assert!(matches!(classify_override("http://lastroute.kyttuslab.org:8205", false), Override::Ignored(_)));
        assert!(matches!(classify_override("http://lastroute.kyttuslab.org:8205", true), Override::TestInstance(_)));
        assert!(matches!(classify_override("http://evil.example.org:8105", true), Override::Ignored(_)));
        assert!(matches!(classify_override("http://localhost.example.org:1", true), Override::Ignored(_)));
        assert!(matches!(classify_override("http://127.0.0.1", true), Override::Ignored(_)), "needs an explicit port like the original");
    }

    #[test]
    fn accounts_gate() {
        let p = BackendConfig::primary();
        assert!(p.accounts_available(false));
        assert_eq!(p.session_section(), "account:https://lastrouterust.games.kyttuslab.org");
        let l = BackendConfig::loopback(18105);
        assert!(l.accounts_available(false));
        let mut odd = BackendConfig::primary();
        odd.base_url = "http://10.0.0.5:8105".into();
        assert!(!odd.accounts_available(true));
        assert_eq!(odd.account_transport_warning(true), "ACCOUNT SIGN-IN IS DISABLED FOR THIS BACKEND ORIGIN");
        let mut off = BackendConfig::primary();
        off.offline = true;
        assert!(!off.accounts_available(false));
    }

    #[test]
    fn args() {
        let args = ["--backend=http://evil.example:1", "--profile=a b!c", "--offline"].map(String::from);
        let (c, w) = BackendConfig::from_args(args, false);
        assert_eq!(c.base_url, PRIMARY_BASE_URL);
        assert_eq!(w.len(), 1);
        assert!(c.offline);
        assert_eq!(c.profile, "abc");
        assert_eq!(c.profile_file("last_route_account.cfg"), "last_route_account_abc.cfg");
    }
}

#[cfg(test)]
mod transport_policy_tests {
    use super::*;

    #[test]
    fn release_refuses_http_off_loopback() {
        let mut c = BackendConfig::primary();
        assert!(c.origin_transport_ok(false), "the primary origin is https");
        c.base_url = "http://10.0.0.5:8105".into();
        assert!(!c.origin_transport_ok(false));
        assert!(!c.origin_transport_ok(true));
        c.base_url = "http://localhost.example:1".into();
        assert!(!c.origin_transport_ok(true), "exact host boundary");
        assert!(BackendConfig::loopback(81).origin_transport_ok(false));
        c.base_url = TEST_BASE_URLS[0].into();
        assert!(!c.origin_transport_ok(false), "release refuses the http TEST instance");
        assert!(c.origin_transport_ok(true), "debug build exception");
    }
}
