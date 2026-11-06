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
    /// The guest name lease was issued or renewed.
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
    pub guest_name: String,
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
        let (guest_name, guest_token) = guests.load();
        Backend {
            config,
            dir: dir.to_path_buf(),
            accounts,
            guests,
            account,
            account_info: None,
            admin: false,
            guest_name,
            guest_token,
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

    /// The guest name lease (`POST /api/guest/name {previous, token}`), kept in `last_route_guest.cfg`.
    pub fn ensure_guest_name(&mut self) {
        if self.config.offline {
            if self.guest_name.is_empty() {
                self.guest_name = api::random_guest_name();
            }
            return;
        }
        let (cfg, previous, token) = (self.config.clone(), self.guest_name.clone(), self.guest_token.clone());
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
                        if let Ok(g) = r {
                            self.guest_name = g.name.clone();
                            self.guest_token = g.token.clone();
                            self.guests.save(&g.name, &g.token);
                            events.push(BackendEvent::GuestName(g.name));
                        } else if self.guest_name.is_empty() {
                            self.guest_name = api::random_guest_name();
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
