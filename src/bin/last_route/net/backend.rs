//! The stateful `Backend` autoload of `game/autoload/backend_client.gd`, for the macroquad menus: one object that
//! owns the saved session, the guest name lease, the hosted room (with its 20 s directory heartbeat) and the
//! requests in flight. Call [`Backend::update`] once per frame and handle the returned [`BackendEvent`]s.

use super::api::{self, AccountReply, Heartbeat, RegisterRoom, Resolution, RoomEndpoint};
use super::config::{self, BackendConfig};
use super::store::{self, AccountStore, GuestStore, StoredSession};
use super::{NetError, Pending};
use std::path::{Path, PathBuf};
use crate::shared::clock::Instant;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    /// Signed in (login, account creation or a restored session).
    SignedIn(AccountReply),
    /// The saved session was refused by the server (cleared locally) or the player signed out.
    SignedOut,
    /// An account action failed: the text the ACCOUNT panel shows.
    AccountError(String),
    /// The server's guest name lease was issued or renewed; the payload is the name this runner plays under
    /// (`GUEST_4829`), which the lease never changes.
    GuestName(String),
    /// CREATE ROOM finished: the room is listed and heartbeating; `token` authenticates the relay host.
    RoomCreated { code: String, token: String, room: RoomEndpoint },
    RoomCreateFailed(String),
    /// JOIN ROOM finished.
    JoinResolved { code: String, resolution: Resolution },
    /// The directory forgot the hosted room (404 / 403 on heartbeat).
    RoomLost(String),
    /// Password changed / account renamed.
    AccountUpdated(String),
}

struct HostedRoom {
    code: String,
    token: String,
    players: u32,
    session_active: bool,
    next_heartbeat: Instant,
}

enum Op {
    Account(Pending<AccountReply>, bool),
    Logout(Pending<()>),
    Guest(Pending<api::GuestName>),
    Create(Pending<api::Registered>),
    Join(String, Pending<Resolution>),
    Heartbeat(Pending<api::HeartbeatReply>),
    Unregister(Pending<bool>),
    Message(Pending<String>),
}

pub struct Backend {
    pub config: BackendConfig,
    pub dir: PathBuf,
    accounts: AccountStore,
    guests: GuestStore,
    /// The signed-in account (empty username = guest).
    pub account: StoredSession,
    pub account_info: Option<AccountReply>,
    /// The signed-in account's admin flag, from the server's reply (never from the local name).
    pub admin: bool,
    /// The name this runner plays under while it is not signed in: `GUEST_` and four digits, chosen once and kept
    /// ([`super::guest_name`]). Never empty.
    pub guest_name: String,
    /// The server's optional guest name lease (an `R-4823` style name) and its token; never shown.
    lease_name: String,
    guest_token: String,
    hosted: Option<HostedRoom>,
    ops: Vec<Op>,
}

impl Backend {
    /// `Backend._ready`: load the saved session and the guest lease from `dir` (the per-user folder).
    pub fn new(config: BackendConfig, dir: &Path) -> Backend {
        let accounts = AccountStore::new(dir, &config);
        let guests = GuestStore::new(dir, &config);
        let account = accounts.load(&config);
        let mut saved = guests.load();
        if saved.name.is_empty() {
            // the first run (or cleared storage): draw the number once and keep it
            saved.name = super::guest_name::random_guest_name();
            guests.save(&saved);
        }
        Backend {
            config,
            dir: dir.to_path_buf(),
            accounts,
            guests,
            account,
            account_info: None,
            admin: false,
            guest_name: saved.name,
            lease_name: saved.lease_name,
            guest_token: saved.lease_token,
            hosted: None,
            ops: Vec::new(),
        }
    }

    /// The primary server and the game's per-user folder.
    pub fn primary() -> Backend {
        Backend::new(BackendConfig::primary(), &store::default_dir())
    }

    pub fn signed_in(&self) -> bool {
        !self.account.session_token.is_empty() && self.account_info.is_some()
    }

    /// The name shown in lobbies: the account name, else the guest name.
    pub fn display_name(&self) -> String {
        if self.signed_in() {
            self.account.username.clone()
        } else {
            self.guest_name.clone()
        }
    }

    /// The name this runner takes into a room: `username` (the account name the app keeps, empty when signed out),
    /// else the guest name. Never a guest name for a signed-in account.
    pub fn room_name(&self, username: &str) -> String {
        super::guest_name::local_name(username, &self.guest_name)
    }

    /// The room gave this guest another name (its own was taken by an earlier guest): play under it from now on, and
    /// keep it for the next run. Only a drawn guest name is taken. Returns whether the name changed.
    pub fn set_guest_name(&mut self, name: &str) -> bool {
        let name = name.trim().to_ascii_uppercase();
        if !super::guest_name::is_drawn_name(&name) || name == self.guest_name {
            return false;
        }
        self.guest_name = name;
        self.save_guest();
        true
    }

    fn save_guest(&self) {
        self.guests.save(&store::SavedGuest {
            name: self.guest_name.clone(),
            lease_name: self.lease_name.clone(),
            lease_token: self.guest_token.clone(),
        });
    }

    pub fn busy(&self) -> bool {
        !self.ops.is_empty()
    }

    /// A sign in, account creation or title session check is in flight.
    pub fn account_pending(&self) -> bool {
        self.ops.iter().any(|op| matches!(op, Op::Account(..)))
    }

    /// Account requests are only allowed on the primary origin and loopback (backend_client.gd's rule).
    pub fn accounts_available(&self) -> bool {
        self.config.accounts_available(cfg!(debug_assertions))
    }

    fn account_refusal(&self) -> Option<NetError> {
        if self.config.offline {
            return Some(NetError::Offline);
        }
        if !self.accounts_available() {
            return Some(NetError::Refused(self.config.account_transport_warning(cfg!(debug_assertions)).to_string()));
        }
        None
    }

    fn start_account(&mut self, work: impl FnOnce(BackendConfig) -> Result<AccountReply, NetError> + Send + 'static, restore: bool) {
        let pending = match self.account_refusal() {
            Some(e) => Pending::done(Err(e)),
            None => {
                let cfg = self.config.clone();
                Pending::spawn(move || work(cfg))
            }
        };
        self.ops.push(Op::Account(pending, restore));
    }

    /// Title start: check the saved session (`POST /api/account/session`).
    pub fn restore_session(&mut self) {
        if self.account.session_token.is_empty() {
            return;
        }
        let token = self.account.session_token.clone();
        self.start_account(move |cfg| api::session(&cfg, &token), true);
    }

    /// SIGN IN.
    pub fn sign_in(&mut self, username: &str, password: &str) {
        let (u, p) = (username.to_string(), password.to_string());
        self.start_account(move |cfg| api::login(&cfg, &u, &p), false);
    }

    /// CREATE ACCOUNT (`admin_setup_key` is the ADMIN SETUP KEY field, only for KYTTU / AYTTU).
    pub fn create_account(&mut self, username: &str, password: &str, admin_setup_key: &str) {
        let (u, p, k) = (username.to_string(), password.to_string(), admin_setup_key.to_string());
        self.start_account(move |cfg| api::register(&cfg, &u, &p, &k), false);
    }

    /// SIGN OUT: the session is forgotten locally at once and revoked on the server.
    pub fn sign_out(&mut self) {
        let token = std::mem::take(&mut self.account.session_token);
        self.account = StoredSession::default();
        self.account_info = None;
        self.admin = false;
        self.accounts.clear(&self.config);
        let cfg = self.config.clone();
        self.ops.push(Op::Logout(if token.is_empty() || cfg.offline {
            Pending::done(Ok(()))
        } else {
            Pending::spawn(move || api::logout(&cfg, &token))
        }));
    }

    pub fn change_password(&mut self, new_password: &str) {
        let (cfg, token, pw) = (self.config.clone(), self.account.session_token.clone(), new_password.to_string());
        self.ops.push(Op::Message(Pending::spawn(move || api::change_password(&cfg, &token, &pw))));
    }

    /// The server's guest name lease (`POST /api/guest/name {previous, token}`), kept beside the guest name. It is
    /// optional: the name shown is the client's `GUEST_####`, which exists before and without the lease, and an
    /// `R-4823` answer is only stored.
    pub fn ensure_guest_name(&mut self) {
        if self.config.offline {
            return;
        }
        let (cfg, previous, token) = (self.config.clone(), self.lease_name.clone(), self.guest_token.clone());
        self.ops.push(Op::Guest(Pending::spawn(move || api::guest_name(&cfg, &previous, &token))));
    }

    /// CREATE ROOM: register a fresh code (or `code`) in the directory; on success the room heartbeats every 20 s.
    pub fn create_room(&mut self, code: Option<&str>, room_password: &str) {
        let request = RegisterRoom {
            code: code.map(str::to_string).unwrap_or_else(api::make_room_code),
            session_token: self.account.session_token.clone(),
            device_id: store::device_id(&self.dir),
            room_password: room_password.to_string(),
            ..RegisterRoom::default()
        };
        let cfg = self.config.clone();
        self.ops.push(Op::Create(if cfg.offline {
            Pending::done(Err(NetError::Offline))
        } else {
            Pending::spawn(move || api::register_room(&cfg, &request))
        }));
    }

    /// JOIN ROOM: resolve a typed code (reserved rooms use the session and `room_password`).
    pub fn join_room(&mut self, code: &str, room_password: &str) {
        let (cfg, token, pw, code) = (self.config.clone(), self.account.session_token.clone(), room_password.to_string(), code.to_string());
        let mine = super::protocol::fingerprint();
        let c = code.clone();
        self.ops.push(Op::Join(code, Pending::spawn(move || Ok(api::resolve_room(&cfg, &c, &token, &pw, &mine, false)))));
    }

    /// The hosted room's player count / session flag for the next heartbeat.
    pub fn set_room_status(&mut self, players: u32, session_active: bool) {
        if let Some(room) = self.hosted.as_mut() {
            room.players = players;
            room.session_active = session_active;
        }
    }

    pub fn hosted_room(&self) -> Option<(&str, &str)> {
        self.hosted.as_ref().map(|r| (r.code.as_str(), r.token.as_str()))
    }

    /// Leave / close the hosted room (`/api/enet/rooms/unregister`).
    pub fn close_room(&mut self) {
        if let Some(room) = self.hosted.take() {
            let cfg = self.config.clone();
            self.ops.push(Op::Unregister(Pending::spawn(move || api::unregister_room(&cfg, &room.code, &room.token))));
        }
    }

    /// Polls everything in flight; call once per frame.
    pub fn update(&mut self) -> Vec<BackendEvent> {
        let mut events = Vec::new();
        if let Some(room) = self.hosted.as_mut() {
            if Instant::now() >= room.next_heartbeat {
                room.next_heartbeat = Instant::now() + Duration::from_secs_f64(config::HEARTBEAT_SECONDS);
                let beat = Heartbeat {
                    code: room.code.clone(),
                    token: room.token.clone(),
                    players: room.players,
                    session_active: room.session_active,
                    daytime: false,
                    command_since: 0,
                    command_ack: 0,
                };
                let cfg = self.config.clone();
                self.ops.push(Op::Heartbeat(Pending::spawn(move || api::heartbeat(&cfg, &beat))));
            }
        }
        let ops = std::mem::take(&mut self.ops);
        for mut op in ops {
            let done = match &mut op {
                Op::Account(p, restore) => p.poll().map(|r| self.on_account(r, *restore, &mut events)).is_some(),
                Op::Logout(p) => p.poll().map(|_| events.push(BackendEvent::SignedOut)).is_some(),
                Op::Guest(p) => p
                    .poll()
                    .map(|r| {
                        // a refused or failed lease changes nothing: the guest name does not depend on it
                        if let Ok(g) = r {
                            if api::valid_lease_name(&g.name.to_ascii_uppercase()) {
                                self.lease_name = g.name.to_ascii_uppercase();
                                self.guest_token = g.token.clone();
                                self.save_guest();
                            }
                            events.push(BackendEvent::GuestName(self.guest_name.clone()));
                        }
                    })
                    .is_some(),
                Op::Create(p) => p
                    .poll()
                    .map(|r| match r {
                        Ok(reg) => {
                            self.hosted = Some(HostedRoom {
                                code: reg.room.code.clone(),
                                token: reg.token.clone(),
                                players: 1,
                                session_active: false,
                                next_heartbeat: Instant::now() + Duration::from_secs_f64(config::HEARTBEAT_SECONDS),
                            });
                            events.push(BackendEvent::RoomCreated { code: reg.room.code.clone(), token: reg.token, room: reg.room });
                        }
                        Err(e) => events.push(BackendEvent::RoomCreateFailed(e.message_or("ROOM REGISTRATION FAILED"))),
                    })
                    .is_some(),
                Op::Join(code, p) => {
                    let code = code.clone();
                    p.poll()
                        .map(|r| {
                            let resolution = r.unwrap_or_else(|e| Resolution::Failed { message: e.message_or("ROOM NOT FOUND") });
                            events.push(BackendEvent::JoinResolved { code, resolution });
                        })
                        .is_some()
                }
                Op::Heartbeat(p) => p
                    .poll()
                    .map(|r| {
                        if let Err(e) = r {
                            if matches!(e.status(), Some(403) | Some(404)) {
                                self.hosted = None;
                                events.push(BackendEvent::RoomLost(e.message_or("ROOM NOT FOUND")));
                            }
                        }
                    })
                    .is_some(),
                Op::Unregister(p) => p.poll().is_some(),
                Op::Message(p) => p
                    .poll()
                    .map(|r| match r {
                        Ok(text) => events.push(BackendEvent::AccountUpdated(text)),
                        Err(e) => events.push(BackendEvent::AccountError(e.message_or("ACCOUNT UPDATE FAILED"))),
                    })
                    .is_some(),
            };
            if !done {
                self.ops.push(op);
            }
        }
        events
    }

    fn on_account(&mut self, result: Result<AccountReply, NetError>, restore: bool, events: &mut Vec<BackendEvent>) {
        match result {
            Ok(reply) => {
                if !reply.session_token.is_empty() {
                    self.account.session_token = reply.session_token.clone();
                }
                self.account.username = reply.username.clone();
                self.accounts.save(&self.config, &self.account);
                self.admin = reply.admin;
                self.account_info = Some(reply.clone());
                events.push(BackendEvent::SignedIn(reply));
            }
            Err(e) => {
                if restore && e.status().map(|s| config::SESSION_REJECTED_CODES.contains(&s)).unwrap_or(false) {
                    self.account = StoredSession::default();
                    self.account_info = None;
                    self.admin = false;
                    self.accounts.clear(&self.config);
                    events.push(BackendEvent::SignedOut);
                } else {
                    let fallback = format!("ACCOUNT REQUEST FAILED (HTTP {})", e.status().unwrap_or(0));
                    events.push(BackendEvent::AccountError(e.message_or(&fallback)));
                }
            }
        }
    }

    /// Waits (blocking, for tests/CLI) until nothing is in flight or `limit` passes, collecting events.
    pub fn settle(&mut self, limit: Duration) -> Vec<BackendEvent> {
        let deadline = Instant::now() + limit;
        let mut all = Vec::new();
        while Instant::now() < deadline {
            all.extend(self.update());
            if self.ops.is_empty() {
                break;
            }
            crate::shared::task::pause(Duration::from_millis(10));
        }
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lr_backend_session_{name}_{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn http(status: u16) -> NetError {
        NetError::Http { status, code: String::new(), error: String::new(), body: serde_json::Value::Null }
    }

    fn signed(dir: &Path, cfg: &BackendConfig) -> Backend {
        AccountStore::new(dir, cfg).save(cfg, &StoredSession { username: "KYTTU".into(), session_token: "tok".into() });
        Backend::new(cfg.clone(), dir)
    }

    #[test]
    fn a_new_install_draws_one_guest_name_and_keeps_it_for_the_next_run() {
        let dir = scratch("guest_once");
        let cfg = BackendConfig::loopback(18105);
        let first = Backend::new(cfg.clone(), &dir);
        assert!(super::super::guest_name::is_drawn_name(&first.guest_name), "{}", first.guest_name);
        assert_eq!(first.room_name(""), first.guest_name, "a guest goes into a room under its guest name");
        // a reload / a new start: the same number, whatever the server says
        let again = Backend::new(cfg.clone(), &dir);
        assert_eq!(again.guest_name, first.guest_name);
        // cleared storage: a number is drawn again (and is a valid one)
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(super::super::guest_name::is_drawn_name(&Backend::new(cfg, &dir).guest_name));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_signed_in_account_never_goes_into_a_room_as_a_guest() {
        let dir = scratch("guest_account");
        let backend = Backend::new(BackendConfig::loopback(18105), &dir);
        assert_eq!(backend.room_name("ana"), "ANA");
        assert_eq!(backend.room_name(" Ana "), "ANA");
        assert_eq!(backend.room_name(""), backend.guest_name);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_room_may_change_a_guest_name_only_to_another_drawn_guest_name() {
        let dir = scratch("guest_rename");
        let cfg = BackendConfig::loopback(18105);
        let mut backend = Backend::new(cfg.clone(), &dir);
        let before = backend.guest_name.clone();
        for refused in ["R-4823", "ANA", "GUEST_0007", "", "GUEST_48290", before.as_str()] {
            assert!(!backend.set_guest_name(refused), "{refused:?}");
            assert_eq!(backend.guest_name, before);
        }
        let next = if before == "GUEST_4321" { "GUEST_1234" } else { "GUEST_4321" };
        assert!(backend.set_guest_name(&next.to_lowercase()));
        assert_eq!(backend.guest_name, next);
        assert_eq!(Backend::new(cfg, &dir).guest_name, next, "the new number is the one kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_lease_file_keeps_the_lease_and_the_guest_gets_a_guest_name() {
        let dir = scratch("guest_legacy");
        let cfg = BackendConfig::loopback(18105);
        std::fs::write(dir.join(store::GUEST_CONFIG_FILE), "[player]

guest_name=\"R-4823\"
guest_name_token=\"tok\"
").unwrap();
        let backend = Backend::new(cfg.clone(), &dir);
        assert!(super::super::guest_name::is_drawn_name(&backend.guest_name), "the R-4823 lease is not the name shown");
        let saved = GuestStore::new(&dir, &cfg).load();
        assert_eq!((saved.lease_name.as_str(), saved.lease_token.as_str()), ("R-4823", "tok"), "the lease is kept apart");
        assert_eq!(saved.name, backend.guest_name);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_session_is_loaded_but_not_trusted_until_the_server_answers() {
        let dir = scratch("load");
        let cfg = BackendConfig::loopback(18105);
        let backend = signed(&dir, &cfg);
        assert_eq!(backend.account.session_token, "tok");
        assert!(!backend.signed_in(), "signed in only after the server confirms the token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepted_session_signs_in_and_is_kept() {
        let dir = scratch("ok");
        let cfg = BackendConfig::loopback(18105);
        let mut backend = signed(&dir, &cfg);
        let mut events = Vec::new();
        let reply = AccountReply { username: "KYTTU".into(), ..AccountReply::default() };
        backend.on_account(Ok(reply), true, &mut events);
        assert!(backend.signed_in());
        assert!(matches!(events.as_slice(), [BackendEvent::SignedIn(_)]));
        assert_eq!(AccountStore::new(&dir, &cfg).load(&cfg).session_token, "tok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejected_session_is_cleared_quietly() {
        for status in config::SESSION_REJECTED_CODES {
            let dir = scratch(&format!("rej{status}"));
            let cfg = BackendConfig::loopback(18105);
            let mut backend = signed(&dir, &cfg);
            let mut events = Vec::new();
            backend.on_account(Err(http(status)), true, &mut events);
            assert_eq!(events, vec![BackendEvent::SignedOut], "no error text for a refused saved session");
            assert!(!backend.signed_in());
            assert_eq!(backend.account, StoredSession::default());
            assert_eq!(AccountStore::new(&dir, &cfg).load(&cfg), StoredSession::default(), "cleared in the store");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn unreachable_server_keeps_the_saved_session() {
        let dir = scratch("down");
        let cfg = BackendConfig::loopback(18105);
        let mut backend = signed(&dir, &cfg);
        let mut events = Vec::new();
        backend.on_account(Err(NetError::Timeout), true, &mut events);
        backend.on_account(Err(http(500)), true, &mut events);
        assert!(events.iter().all(|e| matches!(e, BackendEvent::AccountError(_))));
        assert_eq!(AccountStore::new(&dir, &cfg).load(&cfg).session_token, "tok", "a down server is not a logout");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_sign_out_clears_the_store() {
        let dir = scratch("out");
        let mut cfg = BackendConfig::loopback(18105);
        cfg.offline = true; // the revoke call is skipped; the local clear is what is checked
        let mut backend = signed(&dir, &cfg);
        backend.sign_out();
        assert_eq!(backend.account, StoredSession::default());
        assert_eq!(AccountStore::new(&dir, &cfg).load(&cfg), StoredSession::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
