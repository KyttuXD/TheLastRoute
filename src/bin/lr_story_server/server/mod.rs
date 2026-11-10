//! The story room server: the HTTP API, accounts and guest names, the room directory and the UDP relay.
//!
//! Shaped like lr_server's `server/`: `support` holds the request plumbing, `routes` the dispatch table, and this file
//! the state, the relay wiring and the serving loop. The room layer is `rooms::room_service`.

pub mod support;
mod routes;
#[cfg(test)]
mod tests;

use crate::account_service::{AccountService, AccountServiceOptions, DEFAULT_SESSION_TTL_MS};
use crate::config::StoryConfig;
use crate::enet_relay::{self, EnetRelay, EnetRelayOptions, RelayDriver, RelayEvent};
use crate::guest_names::{default_random, GuestNames, GuestNamesOptions, LEASE_MS};
use crate::packs::PackRegistry;
use crate::rooms::room_service::{RoomService, RoomServiceOptions};
use crate::saves::PartySaves;
use crate::util::{log_error, log_info, system_clock, Clock};
pub use support::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// The biggest JSON body a POST route reads.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// The transport's hard cap: a longer body is refused with 413.
pub const RAW_BODY_HARD_CAP: usize = 256 * 1024;
/// How often the maintenance timer settles votes and drops idle rooms.
pub const MAINTENANCE_MS: i64 = 1000;
/// Routes with the smaller, account-style rate limit.
pub const ACCOUNT_RATE_LIMIT_PATHS: [&str; 6] = [
    "/api/guest/name",
    "/api/account/register",
    "/api/account/login",
    "/api/account/session",
    "/api/account/password",
    "/api/account/rename",
];

/// What the server needs to start: the folders, the clock, and the rate limits.
#[derive(Clone)]
pub struct ServerOptions {
    /// Accounts (`accounts.json`), guest names (`guest-names.json`) and the party saves (`parties/`).
    pub data_dir: PathBuf,
    /// One folder per story pack id.
    pub packs_dir: PathBuf,
    pub now: Clock,
    pub trust_proxy: bool,
    pub rate_limit_window_ms: i64,
    pub api_rate_limit_max: u32,
    pub account_rate_limit_max: u32,
    /// The account website (see `StoryConfig::account_upstream`); None keeps local accounts.
    pub account_upstream: Option<crate::upstream::Upstream>,
}

impl ServerOptions {
    pub fn from_config(config: &StoryConfig) -> ServerOptions {
        ServerOptions {
            data_dir: config.data_dir.clone(),
            packs_dir: config.packs_dir.clone(),
            now: system_clock(),
            trust_proxy: config.trust_proxy,
            rate_limit_window_ms: config.rate_limit_window_ms,
            api_rate_limit_max: config.api_rate_limit_max,
            account_rate_limit_max: config.account_rate_limit_max,
            account_upstream: config.account_upstream.clone(),
        }
    }
}

/// Everything the request handlers and the relay share, behind one lock.
pub struct State {
    pub guests: GuestNames,
    pub rooms: RoomService,
    pub relay: Option<EnetRelay>,
    /// rate key -> (requests in the current window, the window's reset time in ms)
    pub request_rates: HashMap<String, (u32, i64)>,
}

pub struct StoryServer {
    pub options: ServerOptions,
    now: Clock,
    /// Accounts synchronize themselves, so the handlers use them without the state lock.
    pub accounts: Arc<AccountService>,
    pub state: Arc<Mutex<State>>,
    relay_driver: Mutex<Option<RelayDriver>>,
}

impl StoryServer {
    pub fn new(options: ServerOptions) -> StoryServer {
        let now = options.now.clone();
        let accounts = AccountService::new(AccountServiceOptions {
            store_path: Some(options.data_dir.join("accounts.json")),
            now: now.clone(),
            session_ttl_ms: DEFAULT_SESSION_TTL_MS,
            upstream: options.account_upstream.clone(),
            service_token: None,
        });
        let guests = GuestNames::new(GuestNamesOptions {
            store_path: Some(options.data_dir.join("guest-names.json")),
            now: now.clone(),
            random: default_random(),
            lease_ms: LEASE_MS,
        });
        let rooms = RoomService::new(RoomServiceOptions {
            now: now.clone(),
            packs: PackRegistry::new(options.packs_dir.clone()),
            saves: PartySaves::new(options.data_dir.join("parties")),
        });
        let state = State { guests, rooms, relay: None, request_rates: HashMap::new() };
        StoryServer {
            options,
            now,
            accounts: Arc::new(accounts),
            state: Arc::new(Mutex::new(state)),
            relay_driver: Mutex::new(None),
        }
    }

    pub fn now(&self) -> i64 {
        (self.now)()
    }

    /// Locks the shared state. A panicking request must not take the other requests down, so a poisoned lock is reused.
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn relay_running(&self) -> bool {
        self.relay_driver.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some()
    }

    /// Binds the UDP relay and starts its thread. Returns the bound port.
    pub fn start_relay(&self, host: &str, port: u16) -> std::io::Result<u16> {
        let mut slot = self.relay_driver.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(driver) = slot.as_ref() {
            return Ok(driver.local_port());
        }
        let options = EnetRelayOptions { now: self.now.clone(), ..EnetRelayOptions::default() };
        let (socket, buffers) = enet_relay::bind_relay_socket(host, port, options.socket_buffer_bytes)?;
        let bound = socket.local_addr()?.port();
        let mut relay = EnetRelay::new(options);
        relay.set_listening(bound, buffers);
        let tick = relay.prune_interval();
        self.lock().relay = Some(relay);
        let state = self.state.clone();
        let driver = RelayDriver::spawn(socket, tick, move |event| {
            let mut guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let s = &mut *guard;
            let Some(relay) = s.relay.as_mut() else { return Vec::new() };
            match event {
                RelayEvent::Datagram(from, data) => relay.handle_datagram(from, data, &mut s.rooms),
                RelayEvent::Tick => relay.prune(&mut s.rooms),
                RelayEvent::SendError => relay.note_send_error(),
            }
            relay.take_outbox()
        })?;
        *slot = Some(driver);
        log_info(&format!("LR story relay: UDP {bound}"));
        Ok(bound)
    }

    fn flush_relay(&self, outgoing: Vec<enet_relay::Outgoing>) {
        if outgoing.is_empty() {
            return;
        }
        let failed = match self.relay_driver.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
            Some(driver) => enet_relay::send_outgoing(driver.sender(), outgoing),
            None => 0,
        };
        if failed > 0 {
            let mut state = self.lock();
            if let Some(relay) = state.relay.as_mut() {
                for _ in 0..failed {
                    relay.note_send_error();
                }
            }
        }
    }

    /// Stops the relay thread and closes the relay's sessions.
    pub fn close(&self) {
        let driver = self.relay_driver.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(mut driver) = driver {
            driver.stop();
        }
        let mut state = self.lock();
        if let Some(relay) = state.relay.as_mut() {
            relay.close();
        }
    }

    /// The maintenance pass: settle due votes, drop idle rooms, and expire old rate windows.
    pub fn maintenance(&self) {
        let timestamp = self.now();
        let mut state = self.lock();
        state.rooms.tick();
        state.request_rates.retain(|_, (_, reset)| timestamp < *reset);
    }

    pub fn maintenance_interval_ms(&self) -> i64 {
        MAINTENANCE_MS
    }

    /// Handles one HTTP request and sends out whatever the relay queued for it.
    pub fn handle(&self, request: &Request) -> Response {
        let response = self.route(request);
        let outgoing = self.lock().relay.as_mut().map(|r| r.take_outbox()).unwrap_or_default();
        self.flush_relay(outgoing);
        response
    }

    /// Serves HTTP until `stop` is set: `workers` threads take requests, one more runs the maintenance timer.
    pub fn serve(self: &Arc<Self>, server: Arc<tiny_http::Server>, workers: usize, stop: Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
        let mut threads = Vec::new();
        for _ in 0..workers.max(1) {
            let (server, this, stop) = (server.clone(), self.clone(), stop.clone());
            let local_port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match server.recv_timeout(std::time::Duration::from_millis(200)) {
                        Ok(Some(request)) => this.serve_one(request, local_port),
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            }));
        }
        let (this, stop) = (self.clone(), stop.clone());
        threads.push(std::thread::spawn(move || {
            let interval = std::time::Duration::from_millis(this.maintenance_interval_ms() as u64);
            let mut last = std::time::Instant::now();
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if last.elapsed() >= interval.min(std::time::Duration::from_secs(1)) {
                    last = std::time::Instant::now();
                    this.maintenance();
                }
            }
        }));
        threads
    }

    fn serve_one(&self, mut raw: tiny_http::Request, local_port: u16) {
        use std::io::Read;
        let (path, query) = split_target(raw.url());
        let mut request = Request {
            method: raw.method().as_str().to_uppercase(),
            path,
            query,
            remote: raw.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default(),
            local_port,
            ..Request::default()
        };
        for header in raw.headers() {
            request.headers.insert(header.field.as_str().as_str().to_ascii_lowercase(), header.value.as_str().to_string());
        }
        let mut body = Vec::new();
        let _ = raw.as_reader().take(RAW_BODY_HARD_CAP as u64 + 1).read_to_end(&mut body);
        if body.len() > RAW_BODY_HARD_CAP {
            request.body_overflow = true;
            body.truncate(RAW_BODY_HARD_CAP);
        }
        request.body = body;
        let response = self.handle(&request);
        let head_only = request.method == "HEAD";
        let mut out = tiny_http::Response::from_data(if head_only { Vec::new() } else { response.body.clone() })
            .with_status_code(response.status);
        for (key, value) in &response.headers {
            if key == "content-length" {
                continue;
            }
            if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes()) {
                out.add_header(header);
            }
        }
        if let Err(error) = raw.respond(out) {
            log_error(&format!("[story] could not answer a request: {error}"));
        }
    }
}
