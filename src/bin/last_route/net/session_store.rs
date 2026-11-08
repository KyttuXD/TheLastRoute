//! The signed-in sessions the running game knows, one per server (TODO 224 sign-out, TODO 226 saved sessions per server).
//!
//! The files are `store::AccountStore`'s job (one `[account:<origin>]` section per server). This is the in-memory view over
//! them, keyed by the server's origin, so the primary server and a local test server never share a login, and
//! [`sign_out`] removes the saved session on disk AND in memory in one call. After a sign-out the name is empty (the
//! runner plays as a guest again).

use super::config::BackendConfig;
use super::store::{AccountStore, StoredSession};
use std::collections::BTreeMap;
use std::path::Path;

/// The key of one server: its origin, lower case, without a trailing slash.
pub fn server_key(config: &BackendConfig) -> String {
    config.base_url.trim_end_matches('/').to_ascii_lowercase()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionStore {
    by_server: BTreeMap<String, StoredSession>,
}

impl SessionStore {
    /// The session for `config`'s server; the signed-out state when none is known.
    pub fn get(&self, config: &BackendConfig) -> StoredSession {
        self.by_server.get(&server_key(config)).cloned().unwrap_or_default()
    }

    /// Remembers a session for this server only. An empty name or token is a sign-out.
    pub fn set(&mut self, config: &BackendConfig, session: StoredSession) {
        if session.username.is_empty() || session.session_token.is_empty() {
            self.by_server.remove(&server_key(config));
        } else {
            let canonical = StoredSession { username: session.username.trim().to_uppercase(), session_token: session.session_token };
            self.by_server.insert(server_key(config), canonical);
        }
    }

    /// Loads this server's session from the account file into memory.
    pub fn load_from(&mut self, dir: &Path, config: &BackendConfig) -> StoredSession {
        let session = AccountStore::new(dir, config).load(config);
        self.set(config, session.clone());
        session
    }

    /// Saves `session` for this server (file and memory); other servers are untouched.
    pub fn save_to(&mut self, dir: &Path, config: &BackendConfig, session: StoredSession) {
        AccountStore::new(dir, config).save(config, &session);
        self.set(config, session);
    }

    /// The servers with a remembered session.
    pub fn servers(&self) -> Vec<String> {
        self.by_server.keys().cloned().collect()
    }
}

/// TODO 224: sign-out clears the saved session on this PC (this server's section only) and in memory. Returns the signed-out
/// state: an empty name, so the game falls back to the guest name.
pub fn sign_out(store: &mut SessionStore, dir: &Path, config: &BackendConfig) -> StoredSession {
    AccountStore::new(dir, config).clear(config);
    store.set(config, StoredSession::default());
    StoredSession::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str, token: &str) -> StoredSession {
        StoredSession { username: name.into(), session_token: token.into() }
    }

    #[test]
    fn sessions_are_keyed_by_server_url() {
        let (a, b) = (BackendConfig::primary(), BackendConfig::loopback(18105));
        let mut s = SessionStore::default();
        s.set(&a, session("one", "t1"));
        s.set(&b, session("two", "t2"));
        assert_eq!(s.get(&a), session("ONE", "t1"));
        assert_eq!(s.get(&b), session("TWO", "t2"));
        assert_eq!(s.servers().len(), 2);
        let mut slash = b.clone();
        slash.base_url.push('/');
        assert_eq!(s.get(&slash), session("TWO", "t2"), "a trailing slash is the same server");
        s.set(&a, StoredSession::default());
        assert_eq!(s.get(&a), StoredSession::default());
        assert_eq!(s.get(&b).session_token, "t2", "clearing one server leaves the other");
    }

    #[test]
    fn sign_out_clears_the_saved_session_and_the_name() {
        let dir = std::env::temp_dir().join(format!("lr_net_session_store_{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (BackendConfig::primary(), BackendConfig::loopback(18105));
        let mut s = SessionStore::default();
        s.save_to(&dir, &a, session("ANNA", "ta"));
        s.save_to(&dir, &b, session("BEN", "tb"));
        let mut fresh = SessionStore::default();
        assert_eq!(fresh.load_from(&dir, &a).username, "ANNA");
        let after = sign_out(&mut s, &dir, &a);
        assert_eq!(after.username, "");
        assert_eq!(s.get(&a), StoredSession::default());
        assert_eq!(AccountStore::new(&dir, &a).load(&a), StoredSession::default(), "the file no longer holds the login");
        assert_eq!(AccountStore::new(&dir, &b).load(&b).username, "BEN", "the other server keeps its login");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
