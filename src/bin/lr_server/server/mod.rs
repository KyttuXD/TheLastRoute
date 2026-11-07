//! Port of `backend/server.mjs`: the HTTP API router, the ENet room-code directory, admin tickets, rate limits,
//! update manifests, static files, and the glue to the UDP gameplay relay (`enet_relay.rs`).
//!
//! [`LastRouteServer::handle`] turns a [`Request`] into a [`Response`] (used directly by the tests);
//! [`LastRouteServer::serve`] runs it on a tiny_http listener. One mutex guards the whole state, and the relay
//! thread locks the same mutex, so relay callbacks (`roomByCode`, `promoteRoomHost`, `onRoomClosed`) see exactly
//! the directory the HTTP routes see.
//!
//! Skins: the v1 routes and the Godot backend's (`/api/skins/upload`, `/mine`, `/clear`, `/remove`, `/files/<id>.png`)
//! all live in `skins/`. Not ported: `PUT /api/v1/skins/<name>` (503 after the size and moderation checks), city maps (`/api/city-maps/*`, a different game: 404 `CITY_MAPS_UNAVAILABLE`) and the
//! browser WebSocket relay (`ws_relay.mjs`; `transport: "ws"` rooms still register/resolve; health reports
//! `wsRelay.enabled: false`).

pub mod support;
mod routes;
#[cfg(test)]
pub(crate) use routes::body_limit;
#[cfg(test)]
mod tests;

pub use support::*;

use crate::account_service::{AccountService, AccountServiceOptions};
use crate::crash_reports::{CrashReports, CrashReportsOptions};
use crate::enet_relay::{self, EnetRelay, EnetRelayOptions, RelayDriver, RelayEvent, RelayRoom, RelayRooms};
use crate::game_saves::{GameSaves, GameSavesOptions};
use crate::skins::skins::{Skins, SkinsOptions};
use crate::guest_names::{GuestNames, GuestNamesOptions};
use crate::mod_packs::{ModPacks, ModPacksOptions};
use crate::moderation::{Moderation, ModerationHost, ModerationOptions};
use crate::public_route::{PublicRoute, PublicRouteOptions, PUBLIC_CODES};
use crate::regions::{Regions, RegionsOptions};
use crate::reserved_rooms::{ReservedRooms, ReservedRoomsOptions};
use crate::room_service::{RoomService, RoomServiceOptions};
use crate::upstream::Upstream;
use crate::util::{base64url, log_error, log_info, num, random_bytes, safe_equal, system_clock, Clock, SessionLookup};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MOD_REGISTER_MAX_BODY_BYTES: usize = 256 * 1024;
/// Raw bodies are never read past this (the Node server destroys the request at 1 MB).
pub const RAW_BODY_HARD_CAP: usize = 1024 * 1024;
pub const DEFAULT_ADMIN_TICKET_TTL_MS: i64 = 5 * 60 * 1000;
pub const DEFAULT_RATE_LIMIT_WINDOW_MS: i64 = 60 * 1000;
pub const DEFAULT_DEV_TOKEN_TTL_MS: i64 = 12 * 60 * 60 * 1000;
pub const PUBLIC_ENET_TTL_MS: i64 = 15_000;
/// How long after its expiry a public room's host may still HOST_REGISTER on the relay with its token.
pub const PUBLIC_HOST_REGISTER_GRACE_MS: i64 = 60_000;
/// How long an address that passed (or failed) the public-host moderation check on HTTP register stays vouched (refused).
pub const HOSTLESS_VOUCH_MS: i64 = 300_000;
/// A source address may make this many relay-created public entries per window.
pub const HOSTLESS_CREATE_LIMIT: usize = 5;
pub const HOSTLESS_CREATE_WINDOW_MS: i64 = 60_000;
const HOSTLESS_ADDRESS_CAP: usize = 1024;
pub const RESERVED_TAKEOVER_MS: i64 = 30_000;
pub const WS_PATH: &str = "/api/ws";
/// The biggest skin PNG: 64 KB, the one number of `skins/skins.rs` (and of the game's validator).
pub const SKIN_MAX_BYTES: usize = crate::skins::skins::SKIN_MAX_BYTES;
pub const ACCOUNT_RATE_LIMIT_PATHS: [&str; 18] = [
    "/api/guest/name",
    "/api/account/register",
    "/api/account/login",
    "/api/account/session",
    "/api/account/password",
    "/api/account/rename",
    "/api/admin/roster",
    "/api/admin/roster/set",
    "/api/admin/roster/remove",
    "/api/admin/account/reset-password",
    "/api/dev/login",
    "/api/dev/reload",
    "/api/enet/admin-tickets/issue",
    "/api/moderation/integrity",
    "/api/admin/flags",
    "/api/admin/flags/set",
    "/api/admin/builds/add",
    "/api/admin/builds/remove",
];

pub fn public_code(game: &str) -> &'static str {
    PUBLIC_CODES.iter().find(|(g, _)| *g == game).map(|(_, c)| *c).unwrap_or(PUBLIC_CODES[0].1)
}

pub fn is_public_code(code: &str) -> bool {
    PUBLIC_CODES.iter().any(|(_, c)| *c == code)
}

/// `publicGameId(value)`: "last_route" unless the value names another public route.
pub fn public_game_id(value: Option<&str>) -> String {
    let game = value.unwrap_or("").trim().to_lowercase();
    if PUBLIC_CODES.iter().any(|(g, _)| *g == game) {
        game
    } else {
        "last_route".into()
    }
}

/// One registered ENet room (the Node `enetRooms` value).
#[derive(Debug, Clone)]
pub struct EnetRoom {
    pub code: String,
    pub token: String,
    pub address: String,
    pub port: i64,
    pub transport: String,
    pub host_user: String,
    pub instance: String,
    pub players: f64,
    pub session_active: bool,
    pub daytime: bool,
    pub game: String,
    pub expires_at: i64,
    pub last_seen: i64,
    pub protocol: String,
    pub dedicated: bool,
    pub region: String,
    pub reserved: bool,
}

/// `publicEnetRoom(room, timestamp, relayAddress, relayPort)`.
pub fn public_enet_room(room: &EnetRoom, timestamp: i64, relay_address: &str, relay_port: u16) -> Value {
    let expires_in = (((room.expires_at - timestamp) as f64) / 1000.0).ceil().max(0.0);
    let mut out = Map::new();
    out.insert("code".into(), json!(room.code));
    out.insert("game".into(), json!(if room.game.is_empty() { "last_route" } else { room.game.as_str() }));
    out.insert("players".into(), num(room.players));
    out.insert("sessionActive".into(), json!(room.session_active));
    out.insert("daytime".into(), json!(room.daytime));
    out.insert("protocol".into(), json!(room.protocol));
    out.insert("dedicated".into(), json!(room.dedicated));
    out.insert("region".into(), json!(room.region));
    out.insert("expiresIn".into(), num(expires_in));
    if room.transport == "ws" {
        out.insert("transport".into(), json!("ws"));
        out.insert("address".into(), json!(""));
        out.insert("port".into(), json!(0));
        out.insert("wsPath".into(), json!(WS_PATH));
        return Value::Object(out);
    }
    let relayed = !relay_address.is_empty() && relay_port != 0;
    out.insert("transport".into(), json!(if relayed { "relay" } else { "direct" }));
    out.insert("address".into(), json!(if relayed { relay_address.to_string() } else { room.address.clone() }));
    out.insert("port".into(), if relayed { json!(relay_port) } else { json!(room.port) });
    Value::Object(out)
}

#[derive(Debug, Clone)]
pub(crate) struct AdminTicket {
    pub code: String,
    pub host_token: String,
    pub expires_at: i64,
    pub username: String,
    pub permissions: Vec<Value>,
    pub owner: bool,
}

/// The part of the state the relay calls back into.
pub struct Directory {
    pub(crate) now: Clock,
    pub(crate) enet_room_ttl_ms: i64,
    pub enet_rooms: IndexMap<String, EnetRoom>,
    pub(crate) admin_tickets: IndexMap<String, AdminTicket>,
    pub(crate) public_routes: Vec<(String, PublicRoute)>,
    pub(crate) reserved: Option<ReservedRooms>,
    /// Addresses whose HTTP register passed `check_host_public`: the only ones the relay lets create a public entry.
    pub(crate) vouched: IndexMap<String, i64>,
    /// Addresses whose HTTP register was refused by moderation (banned or flagged).
    pub(crate) refused: IndexMap<String, i64>,
    /// Relay-created public entries: code -> the address of the relay host that made it.
    pub(crate) relay_made: IndexMap<String, String>,
    /// Recent relay-created entries per source address (the rate limit).
    pub(crate) hostless_hits: Vec<(String, i64)>,
}

/// An address in one canonical text form (IPv4-mapped and dual-stack forms become plain IPv4), so the HTTP side and the
/// relay side compare the same string. Anything that is not an IP stays as it is.
pub(crate) fn canonical_address(address: &str) -> String {
    address.trim().parse::<std::net::IpAddr>().map(|ip| ip.to_canonical().to_string()).unwrap_or_else(|_| address.to_string())
}

impl Directory {
    /// Whether `address` is the relay host that made the relay-made entry `code`.
    pub(crate) fn made_by(&self, code: &str, address: &str) -> bool {
        let address = canonical_address(address);
        self.relay_made.get(code).is_some_and(|made| *made == address)
    }

    /// Records the outcome of a public-host moderation check for `address` (an HTTP register).
    pub(crate) fn note_host_check(&mut self, address: &str, allowed: bool) {
        let address = canonical_address(address);
        let address = address.as_str();
        let now = (self.now)();
        self.vouched.retain(|_, until| *until > now);
        self.refused.retain(|_, until| *until > now);
        let (set, other) = if allowed { (&mut self.vouched, &mut self.refused) } else { (&mut self.refused, &mut self.vouched) };
        other.shift_remove(address);
        set.shift_remove(address);
        set.insert(address.to_string(), now + HOSTLESS_VOUCH_MS);
        while set.len() > HOSTLESS_ADDRESS_CAP {
            set.shift_remove_index(0);
        }
    }

    pub(crate) fn is_vouched(&self, address: &str) -> bool {
        self.vouched.get(&canonical_address(address)).is_some_and(|until| *until > (self.now)())
    }

    pub(crate) fn is_refused(&self, address: &str) -> bool {
        self.refused.get(&canonical_address(address)).is_some_and(|until| *until > (self.now)())
    }

    /// A vouched host's later HTTP traffic keeps its vouch alive (never creates one).
    pub(crate) fn refresh_vouch(&mut self, address: &str) {
        let now = (self.now)();
        if let Some(until) = self.vouched.get_mut(&canonical_address(address)).filter(|until| **until > now) {
            *until = now + HOSTLESS_VOUCH_MS;
        }
    }

    /// Whether `address` may create another relay-made entry now; counts the attempt when it may.
    fn allow_hostless_create(&mut self, address: &str) -> bool {
        let now = (self.now)();
        self.hostless_hits.retain(|(_, at)| now - at < HOSTLESS_CREATE_WINDOW_MS);
        if self.hostless_hits.iter().filter(|(a, _)| a == address).count() >= HOSTLESS_CREATE_LIMIT {
            return false;
        }
        self.hostless_hits.push((address.to_string(), now));
        true
    }

    /// `pruneEnetRooms()`.
    pub fn prune(&mut self) {
        let timestamp = (self.now)();
        for (game, route) in self.public_routes.iter_mut() {
            if route.authority_stale() {
                self.enet_rooms.shift_remove(public_code(game));
                route.reset_authority();
            }
        }
        self.enet_rooms.retain(|_, room| room.expires_at > timestamp);
        let rooms = &self.enet_rooms;
        self.relay_made.retain(|code, _| rooms.contains_key(code));
        if let Some(reserved) = self.reserved.as_mut() {
            let live: std::collections::HashSet<String> = self.enet_rooms.keys().cloned().collect();
            for code in reserved.sweep(&|code| live.contains(code)) {
                crate::util::log_error(&format!("Reserved rooms: deleted the idle room {code}"));
            }
        }
        let rooms = &self.enet_rooms;
        self.admin_tickets.retain(|_, proof| proof.expires_at > timestamp && rooms.contains_key(&proof.code));
    }

    /// `forgetEnetRoom(code)`.
    pub fn forget(&mut self, code: &str) {
        self.relay_made.shift_remove(code);
        let Some(room) = self.enet_rooms.shift_remove(code) else { return };
        self.admin_tickets.retain(|_, proof| !(proof.code == code && safe_equal(&proof.host_token, &room.token)));
    }

    /// `promoteEnetRoomHost(code)`.
    pub fn promote(&mut self, code: &str) -> Option<String> {
        let now = (self.now)();
        let ttl = self.enet_room_ttl_ms;
        let room = self.enet_rooms.get_mut(code)?;
        let old = std::mem::replace(&mut room.token, base64url(&random_bytes(24)));
        room.players = (room.players - 1.0).max(1.0);
        room.expires_at = now + ttl;
        let token = room.token.clone();
        self.admin_tickets.retain(|_, proof| !(proof.code == code && safe_equal(&proof.host_token, &old)));
        Some(token)
    }

    pub(crate) fn route_mut(&mut self, game: &str) -> &mut PublicRoute {
        let index = self.public_routes.iter().position(|(g, _)| g == game).unwrap_or(0);
        &mut self.public_routes[index].1
    }
}

impl Directory {
    /// A public code whose host the relay takes without a token (a reserved code keeps its token check).
    fn is_hostless_code(&self, code: &str) -> bool {
        is_public_code(code) && !self.reserved.as_ref().is_some_and(|r| r.is_reserved(code))
    }
}

impl RelayRooms for Directory {
    fn hostless_room(&mut self, code: &str, host: std::net::SocketAddr) -> Option<RelayRoom> {
        if !self.is_hostless_code(code) {
            return None;
        }
        self.prune();
        let now = (self.now)();
        let source = host.ip().to_canonical().to_string();
        if self.is_refused(&source) {
            return None;
        }
        if !self.enet_rooms.contains_key(code) {
            // no live entry (expired, or never registered over HTTP): only an address that passed the HTTP host check
            // (moderation) may recreate it, and only a few times a minute
            if !self.is_vouched(&source) || !self.allow_hostless_create(&source) {
                return None;
            }
            // the datagram's source is not a game address: the entry carries none, guests reach the room through the relay
            let room = EnetRoom {
                code: code.to_string(),
                token: base64url(&random_bytes(24)),
                address: String::new(),
                port: 0,
                transport: "enet".into(),
                host_user: String::new(),
                instance: base64url(&random_bytes(9)),
                players: 1.0,
                session_active: false,
                daytime: false,
                game: PUBLIC_CODES.iter().find(|(_, c)| *c == code).map(|(g, _)| g.to_string()).unwrap_or_default(),
                expires_at: now + PUBLIC_ENET_TTL_MS,
                last_seen: now,
                protocol: String::new(),
                dedicated: false,
                region: String::new(),
                reserved: false,
            };
            self.enet_rooms.insert(code.to_string(), room);
            self.relay_made.insert(code.to_string(), source);
        }
        self.enet_rooms.get(code).map(|room| RelayRoom { token: room.token.clone(), gated: false })
    }
    fn hostless_host_allowed(&mut self, code: &str, host: std::net::SocketAddr, token: &str) -> bool {
        let source = host.ip().to_canonical().to_string();
        if !self.is_hostless_code(code) || self.is_refused(&source) {
            return false;
        }
        // the registered host of the entry holds its token, whatever address string its HTTP register came from
        let tokened = self.enet_rooms.get(code).is_some_and(|room| !token.is_empty() && safe_equal(token, &room.token));
        tokened || self.is_vouched(&source) || self.made_by(code, &source)
    }
    fn hostless_host_alive(&mut self, code: &str) {
        // the relay host of a relay-made entry is live: the entry must not lapse under it
        if !self.relay_made.contains_key(code) {
            return;
        }
        let expires = (self.now)() + PUBLIC_ENET_TTL_MS;
        if let Some(room) = self.enet_rooms.get_mut(code) {
            room.expires_at = expires;
        }
    }
    fn is_hostless(&mut self, code: &str) -> bool {
        self.is_hostless_code(code)
    }
    fn room_for_host_register(&mut self, code: &str, token: &str) -> Option<RelayRoom> {
        // the public room's host registers on the relay right after its directory heartbeat; one that came a little
        // late still finds its entry. The lookup changes nothing: the expired clone is checked against the token, and
        // only a match puts the entry back (a wrong token leaves it expired).
        let now = (self.now)();
        let late = self
            .enet_rooms
            .get(code)
            .filter(|room| is_public_code(code) && room.expires_at <= now && room.expires_at + PUBLIC_HOST_REGISTER_GRACE_MS > now)
            .cloned();
        if late.as_ref().is_some_and(|room| !safe_equal(token, &room.token)) {
            // not the host: the lookup must not prune or revive anything
            return None;
        }
        let authority_was_stale = self.public_routes.iter().any(|(game, route)| public_code(game) == code && route.authority_stale());
        let found = self.room_by_code(code);
        if found.is_some() {
            return found;
        }
        let room = late?;
        // the prune above may have dropped the entry for good: a stale route authority, or a reserved room it swept
        let reserved_gone = room.reserved && !self.reserved.as_ref().is_some_and(|r| r.is_reserved(code));
        if authority_was_stale || reserved_gone {
            return None;
        }
        let relay_room = RelayRoom { token: room.token.clone(), gated: room.reserved };
        self.enet_rooms.insert(code.to_string(), room);
        Some(relay_room)
    }
    fn host_registered(&mut self, code: &str) {
        let now = (self.now)();
        if is_public_code(code) {
            if let Some(room) = self.enet_rooms.get_mut(code) {
                room.expires_at = now + PUBLIC_ENET_TTL_MS;
            }
        }
    }
    fn room_by_code(&mut self, code: &str) -> Option<RelayRoom> {
        self.prune();
        self.enet_rooms.get(code).map(|room| RelayRoom { token: room.token.clone(), gated: room.reserved })
    }
    fn promote_room_host(&mut self, code: &str) -> Option<String> {
        self.promote(code)
    }
    fn on_room_closed(&mut self, code: &str, _reason: &str) {
        self.forget(code);
    }
    fn admit_guest(&mut self, code: &str, nonce: &[u8]) -> bool {
        self.reserved.as_ref().map(|r| r.ticket_valid(code, nonce)).unwrap_or(false)
    }
}

pub(crate) struct ModCtx<'a> {
    pub accounts: &'a AccountService,
    pub updates: Value,
}

impl ModerationHost for ModCtx<'_> {
    fn is_owner(&self, name: &str) -> bool {
        self.accounts.is_admin_owner(name)
    }
    fn account_exists(&self, name: &str) -> bool {
        self.accounts.account_exists(name)
    }
    fn updates(&self) -> Value {
        self.updates.clone()
    }
}

pub type UpdatesProvider = Arc<dyn Fn() -> Result<Value, String> + Send + Sync>;

/// `createLastRouteServer({...})` options (same defaults as the Node server).
pub struct ServerOptions {
    pub enet_room_ttl_ms: i64,
    pub admin_ticket_ttl_ms: i64,
    pub room_ttl_ms: i64,
    pub player_ttl_ms: i64,
    pub account_store_path: Option<PathBuf>,
    /// `LASTROUTE_ACCOUNT_UPSTREAM`: the website that owns the accounts and saves. When set, the local store is unused.
    pub account_upstream: Option<Upstream>,
    /// `LASTROUTE_SERVICE_TOKEN` (32+ characters, else None): the server-to-server proof sent with the account check.
    pub account_service_token: Option<String>,
    pub moderation_store_path: Option<PathBuf>,
    pub moderation_salt: String,
    pub public_directory: Option<PathBuf>,
    pub custom_map_directory: Option<PathBuf>,
    pub save_directory: Option<PathBuf>,
    /// Skins stored without an upstream (`<data dir>/skins`).
    pub skin_directory: Option<PathBuf>,
    /// How long a skin file that no account holds is kept before it is deleted, in milliseconds
    /// (`LASTROUTE_SKIN_GRACE_SECONDS`, 7 days by default).
    pub skin_grace_ms: i64,
    pub update_directory: Option<PathBuf>,
    /// When set, update files need this key in the `X-Download-Key` header (LASTROUTE_UPDATE_KEY).
    pub update_key: String,
    pub admin_password: String,
    pub updates: Value,
    pub updates_provider: Option<UpdatesProvider>,
    pub trust_proxy: bool,
    pub relay_address: String,
    pub relay_port: u16,
    pub public_route_store_path: Option<PathBuf>,
    pub public_route_seed: String,
    pub public_item_respawn_ms: Option<i64>,
    pub mod_pack_store_path: Option<PathBuf>,
    pub crash_report_directory: Option<PathBuf>,
    pub reserved_room_store_path: Option<PathBuf>,
    pub reserved_room_codes: Vec<String>,
    /// Idle time after which an admin-made reservation with no live session is deleted (0 never deletes).
    pub reserved_room_idle_ms: i64,
    pub agent_tokens: HashMap<String, String>,
    pub mod_download_base: String,
    pub rate_limit_window_ms: i64,
    pub api_rate_limit_max: u32,
    pub mod_rate_limit_max: u32,
    pub account_rate_limit_max: u32,
    pub dev_token_ttl_ms: i64,
    /// `LASTROUTE_INSTANCE` (shown in /api/health).
    pub instance: String,
    pub now: Clock,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            enet_room_ttl_ms: 60_000,
            admin_ticket_ttl_ms: DEFAULT_ADMIN_TICKET_TTL_MS,
            room_ttl_ms: 300_000,
            player_ttl_ms: 120_000,
            account_store_path: None,
            account_upstream: None,
            account_service_token: None,
            moderation_store_path: None,
            moderation_salt: String::new(),
            public_directory: None,
            custom_map_directory: None,
            save_directory: None,
            skin_directory: None,
            skin_grace_ms: crate::skins::skins::UNUSED_SKIN_GRACE_MS,
            update_directory: None,
            update_key: String::new(),
            admin_password: String::new(),
            updates: json!({}),
            updates_provider: None,
            trust_proxy: false,
            relay_address: String::new(),
            relay_port: 0,
            public_route_store_path: None,
            public_route_seed: "PUBLIC-ROUTE".into(),
            public_item_respawn_ms: None,
            mod_pack_store_path: None,
            crash_report_directory: None,
            reserved_room_store_path: None,
            reserved_room_codes: Vec::new(),
            reserved_room_idle_ms: 3_600_000,
            agent_tokens: HashMap::new(),
            mod_download_base: String::new(),
            rate_limit_window_ms: DEFAULT_RATE_LIMIT_WINDOW_MS,
            api_rate_limit_max: 600,
            mod_rate_limit_max: 120,
            account_rate_limit_max: 30,
            dev_token_ttl_ms: DEFAULT_DEV_TOKEN_TTL_MS,
            instance: "production".into(),
            now: system_clock(),
        }
    }
}

pub(crate) struct Config {
    pub enet_room_ttl_ms: i64,
    pub admin_ticket_ttl_ms: i64,
    pub player_ttl_ms: i64,
    pub public_directory: Option<PathBuf>,
    pub custom_map_directory: Option<PathBuf>,
    pub update_directory: Option<PathBuf>,
    /// When set, update files need this key in the `X-Download-Key` header (LASTROUTE_UPDATE_KEY).
    pub update_key: String,
    pub admin_password: String,
    pub updates: Value,
    pub updates_provider: Option<UpdatesProvider>,
    pub trust_proxy: bool,
    pub relay_address: String,
    pub relay_port: u16,
    pub rate_limit_window_ms: i64,
    pub api_rate_limit_max: u32,
    pub mod_rate_limit_max: u32,
    pub account_rate_limit_max: u32,
    pub dev_token_ttl_ms: i64,
    pub instance: String,
}

pub struct State {
    pub dir: Directory,
    pub relay: Option<EnetRelay>,
    pub(crate) room_service: RoomService,
    pub(crate) accounts: Arc<AccountService>,
    pub(crate) guest_names: GuestNames,
    pub(crate) moderation: Moderation,
    pub(crate) crash_reports: Option<CrashReports>,
    pub(crate) regions: Regions,
    pub(crate) spawn_tickets: HashMap<String, (String, i64)>,
    pub(crate) mod_packs: ModPacks,
    pub(crate) game_saves: Arc<Mutex<GameSaves>>,
    pub(crate) skins: Skins,
    pub(crate) dev_tokens: HashMap<String, i64>,
    pub(crate) request_rates: HashMap<String, (u32, i64)>,
    pub(crate) reload_id: i64,
}

pub struct LastRouteServer {
    pub(crate) now: Clock,
    pub(crate) config: Config,
    pub(crate) state: Arc<Mutex<State>>,
    relay_driver: Mutex<Option<RelayDriver>>,
    /// When the server was built: `uptimeSeconds` on /api/health.
    started: std::time::Instant,
}

impl LastRouteServer {
    pub fn new(opts: ServerOptions) -> LastRouteServer {
        let now = opts.now.clone();
        let city_store = opts.public_route_store_path.as_ref().map(|p| {
            let text = p.to_string_lossy().to_string();
            let re = Regex::new(r"(\.[^./\\]+)?$").unwrap();
            PathBuf::from(re.replace(&text, "_city$1").to_string())
        });
        let mut public_routes = Vec::new();
        for (game, code) in PUBLIC_CODES {
            let (store, seed) = if game == "last_route" {
                (opts.public_route_store_path.clone(), opts.public_route_seed.clone())
            } else {
                (city_store.clone(), format!("{}-CITY", opts.public_route_seed))
            };
            let mut route_opts =
                PublicRouteOptions { now: now.clone(), store_path: store, initial_seed: seed, code: code.into(), ..PublicRouteOptions::default() };
            if let Some(ms) = opts.public_item_respawn_ms {
                route_opts.item_respawn_ms = ms;
            }
            let mut route = PublicRoute::new(route_opts);
            route.load();
            public_routes.push((game.to_string(), route));
        }
        let accounts =
            AccountService::new(AccountServiceOptions {
                store_path: opts.account_store_path.clone(),
                now: now.clone(),
                upstream: opts.account_upstream.clone(),
                service_token: opts.account_service_token.clone(),
                ..Default::default()
            });
        let guest_names = GuestNames::new(GuestNamesOptions {
            store_path: opts.account_store_path.as_ref().map(|p| p.parent().unwrap_or(Path::new(".")).join("guest_names.json")),
            now: now.clone(),
            ..Default::default()
        });
        let moderation =
            Moderation::new(ModerationOptions { store_path: opts.moderation_store_path.clone(), salt: opts.moderation_salt.clone(), now: now.clone() });
        let crash_reports =
            opts.crash_report_directory.clone().map(|directory| CrashReports::new(CrashReportsOptions { directory, now: now.clone() }));
        let reserved = opts.reserved_room_store_path.clone().map(|store_path| {
            ReservedRooms::new(ReservedRoomsOptions {
                store_path,
                codes: opts.reserved_room_codes.clone(),
                now: now.clone(),
                idle_ttl_ms: opts.reserved_room_idle_ms,
            })
        });
        let state = State {
            dir: Directory {
                now: now.clone(),
                enet_room_ttl_ms: opts.enet_room_ttl_ms,
                enet_rooms: IndexMap::new(),
                admin_tickets: IndexMap::new(),
                public_routes,
                reserved,
                vouched: IndexMap::new(),
                refused: IndexMap::new(),
                relay_made: IndexMap::new(),
                hostless_hits: Vec::new(),
            },
            relay: None,
            room_service: RoomService::new(RoomServiceOptions {
                now: now.clone(),
                room_ttl_ms: opts.room_ttl_ms,
                player_ttl_ms: opts.player_ttl_ms,
            }),
            accounts: Arc::new(accounts),
            guest_names,
            moderation,
            crash_reports,
            regions: Regions::new(RegionsOptions { tokens: opts.agent_tokens.clone(), now: now.clone() }),
            spawn_tickets: HashMap::new(),
            mod_packs: ModPacks::new(ModPacksOptions {
                store_path: opts.mod_pack_store_path.clone(),
                download_base: opts.mod_download_base.clone(),
                now: now.clone(),
                ..Default::default()
            }),
            game_saves: Arc::new(Mutex::new(GameSaves::new(GameSavesOptions {
                directory: opts.save_directory.clone(),
                now: now.clone(),
                upstream: opts.account_upstream.clone(),
                ..Default::default()
            }))),
            skins: Skins::new(SkinsOptions { directory: opts.skin_directory.clone(), upstream: opts.account_upstream.clone(), now: now.clone(), grace_ms: opts.skin_grace_ms }),
            dev_tokens: HashMap::new(),
            request_rates: HashMap::new(),
            reload_id: 0,
        };
        let instance: String =
            opts.instance.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(32).collect();
        LastRouteServer {
            now,
            started: std::time::Instant::now(),
            config: Config {
                enet_room_ttl_ms: opts.enet_room_ttl_ms,
                admin_ticket_ttl_ms: opts.admin_ticket_ttl_ms,
                player_ttl_ms: opts.player_ttl_ms,
                public_directory: opts.public_directory,
                custom_map_directory: opts.custom_map_directory,
                update_directory: opts.update_directory,
                update_key: opts.update_key,
                admin_password: opts.admin_password,
                updates: opts.updates,
                updates_provider: opts.updates_provider,
                trust_proxy: opts.trust_proxy,
                relay_address: opts.relay_address,
                relay_port: opts.relay_port,
                rate_limit_window_ms: opts.rate_limit_window_ms,
                api_rate_limit_max: opts.api_rate_limit_max,
                mod_rate_limit_max: opts.mod_rate_limit_max,
                account_rate_limit_max: opts.account_rate_limit_max,
                dev_token_ttl_ms: opts.dev_token_ttl_ms,
                instance: if instance.is_empty() { "production".into() } else { instance },
            },
            state: Arc::new(Mutex::new(state)),
            relay_driver: Mutex::new(None),
        }
    }

    pub(crate) fn now(&self) -> i64 {
        (self.now)()
    }

    pub(crate) fn current_updates(&self) -> Value {
        match &self.config.updates_provider {
            None => self.config.updates.clone(),
            Some(provider) => match provider() {
                Ok(value) if value.is_object() => value,
                Ok(_) => self.config.updates.clone(),
                Err(error) => {
                    log_error(&format!("Could not reload update manifest: {error}"));
                    self.config.updates.clone()
                }
            },
        }
    }

    /// `server.startEnetRelay({host, port})`: binds the UDP relay and starts its thread. Returns the bound port.
    pub fn start_enet_relay(&self, host: &str, port: u16) -> std::io::Result<u16> {
        let mut slot = self.relay_driver.lock().unwrap();
        if let Some(driver) = slot.as_ref() {
            return Ok(driver.local_port());
        }
        let options = EnetRelayOptions { now: self.now.clone(), ..EnetRelayOptions::default() };
        let (socket, buffers) = enet_relay::bind_relay_socket(host, port, options.socket_buffer_bytes)?;
        let bound = socket.local_addr()?.port();
        let mut relay = EnetRelay::new(options);
        relay.set_listening(bound, buffers);
        let tick = relay.prune_interval();
        self.state.lock().unwrap().relay = Some(relay);
        log_info(&format!("ENet gameplay relay: UDP {bound}"));
        let state = self.state.clone();
        let driver = RelayDriver::spawn(socket, tick, move |event| {
            let mut guard = state.lock().unwrap();
            let s = &mut *guard;
            let Some(relay) = s.relay.as_mut() else { return Vec::new() };
            match event {
                RelayEvent::Datagram(from, data) => relay.handle_datagram(from, data, &mut s.dir),
                RelayEvent::Tick => relay.prune(&mut s.dir),
                RelayEvent::SendError => relay.note_send_error(),
            }
            relay.take_outbox()
        })?;
        *slot = Some(driver);
        Ok(bound)
    }

    fn flush_relay(&self, outgoing: Vec<enet_relay::Outgoing>) {
        if outgoing.is_empty() {
            return;
        }
        let failed = match self.relay_driver.lock().unwrap().as_ref() {
            Some(driver) => enet_relay::send_outgoing(driver.sender(), outgoing),
            None => 0,
        };
        if failed > 0 {
            if let Some(relay) = self.state.lock().unwrap().relay.as_mut() {
                for _ in 0..failed {
                    relay.note_send_error();
                }
            }
        }
    }

    /// Stops the relay thread and flushes the public routes (the Node server "close").
    pub fn close(&self) {
        if let Some(mut driver) = self.relay_driver.lock().unwrap().take() {
            driver.stop();
        }
        let mut state = self.state.lock().unwrap();
        if let Some(relay) = state.relay.as_mut() {
            relay.close();
        }
        for (_, route) in state.dir.public_routes.iter_mut() {
            route.flush();
        }
    }

    /// The Node maintenance interval: prune rooms, clean web rooms, expire rate buckets (plus public-route ticks).
    pub fn maintenance(&self) {
        let timestamp = self.now();
        let mut state = self.state.lock().unwrap();
        state.dir.prune();
        state.room_service.cleanup();
        state.request_rates.retain(|_, (_, reset)| timestamp < *reset);
        for (_, route) in state.dir.public_routes.iter_mut() {
            route.tick();
        }
    }

    pub fn maintenance_interval_ms(&self) -> i64 {
        self.config.enet_room_ttl_ms.min(self.config.player_ttl_ms).min(15_000).max(1000)
    }

    /// Handles one HTTP request (the body of the Node `createServer` callback).
    pub fn handle(&self, request: &Request) -> Response {
        let response = self.route(request);
        let outgoing = self.state.lock().unwrap().relay.as_mut().map(|r| r.take_outbox()).unwrap_or_default();
        self.flush_relay(outgoing);
        response
    }

    /// Serves HTTP until `stop` is set: `workers` threads take requests; one more runs the maintenance timer.
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
        let length = response.header("content-length").map(str::to_string);
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
        let _ = length;
        let _ = raw.respond(out);
    }
}

impl SessionLookup for LastRouteServer {
    fn session_username(&self, token: &str) -> Option<String> {
        let accounts = self.state.lock().unwrap().accounts.clone();
        accounts.session_username(token)
    }
}

/// What `startLastRouteServerFromEnvironment()` derives from the environment (same variable names).
pub struct EnvironmentConfig {
    pub options: ServerOptions,
    pub host: String,
    pub http_port: u16,
    pub relay_port: u16,
    pub relay_enabled: bool,
    pub account_store_path: PathBuf,
}

/// `LASTROUTE_SERVICE_TOKEN` as the account check uses it: trimmed, and only when it is at least 32 characters (a shorter
/// value counts as unset). Read from the environment only; never logged.
pub fn service_token_from(env: &HashMap<String, String>) -> Option<String> {
    env.get("LASTROUTE_SERVICE_TOKEN").map(|value| value.trim()).filter(|value| value.chars().count() >= 32).map(str::to_string)
}

/// The data directory the server uses: `LASTROUTE_DATA_DIR`, else `KYTTU_DATA_DIR` (both relative to `cwd`), else `.data`.
pub fn data_dir_from(env: &HashMap<String, String>, cwd: &Path) -> PathBuf {
    let path_of = |name: &str| env.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()).map(|v| cwd.join(v));
    path_of("LASTROUTE_DATA_DIR").or_else(|| path_of("KYTTU_DATA_DIR")).unwrap_or_else(|| cwd.join(".data"))
}

/// Where the server keeps `accounts.json`: `LASTROUTE_ACCOUNT_STORE`, else `KYTTU_ACCOUNT_STORE_PATH`, else
/// `accounts.json` in the data directory. The offline password reset resolves the file the same way.
pub fn account_store_path_from(env: &HashMap<String, String>, cwd: &Path) -> PathBuf {
    let path_of = |name: &str| env.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()).map(|v| cwd.join(v));
    path_of("LASTROUTE_ACCOUNT_STORE")
        .or_else(|| path_of("KYTTU_ACCOUNT_STORE_PATH"))
        .unwrap_or_else(|| data_dir_from(env, cwd).join("accounts.json"))
}

pub fn options_from_environment(env: &HashMap<String, String>, cwd: &Path) -> Result<EnvironmentConfig, String> {
    let get = |name: &str| env.get(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let account_upstream = get("LASTROUTE_ACCOUNT_UPSTREAM")
        .map(|url| Upstream::parse(&url).map_err(|error| format!("LASTROUTE_ACCOUNT_UPSTREAM {error}")))
        .transpose()?;
    let account_service_token = service_token_from(env);
    if account_upstream.is_some() && account_service_token.is_none() {
        // The value itself is never printed.
        log_info("Account existence check is off: LASTROUTE_SERVICE_TOKEN is unset or shorter than 32 characters");
    }
    let path_of = |name: &str| get(name).map(|v| cwd.join(v));
    let (http_port, relay_port) = deployment_ports(env);
    let host = env.get("HOST").cloned().filter(|h| !h.is_empty()).unwrap_or_else(|| "0.0.0.0".into());
    let relay_enabled = !Regex::new(r"(?i)^(0|false|no|off)$")
        .unwrap()
        .is_match(env.get("LASTROUTE_RELAY_ENABLED").filter(|v| !v.is_empty()).map(String::as_str).unwrap_or("true"));
    let relay_address = get("LASTROUTE_RELAY_HOST").unwrap_or_else(|| "lastroute.kyttuslab.org".into());
    let data = data_dir_from(env, cwd);
    let accounts = account_store_path_from(env, cwd);
    let custom = path_of("LASTROUTE_CUSTOM_MAP_DIR").or_else(|| Some(cwd.join("assets/custom_maps")).filter(|p| p.exists()));
    let manifest = path_of("LASTROUTE_UPDATE_MANIFEST");
    let mut updates = json!({});
    if let Some(m) = manifest.as_ref().filter(|m| m.exists()) {
        match std::fs::read_to_string(m).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(value) => updates = value,
            Err(error) => log_error(&format!("Could not read update manifest {}: {error}", m.display())),
        }
    }
    let number = |name: &str, fallback: f64| -> f64 {
        let n = crate::util::js_number_from_str(env.get(name).map(String::as_str).unwrap_or("x"));
        if n == 0.0 || n.is_nan() {
            fallback
        } else {
            n
        }
    };
    let fallback_updates = updates.clone();
    let provider: Option<UpdatesProvider> = manifest.clone().map(|m| {
        Arc::new(move || -> Result<Value, String> {
            if !m.exists() {
                return Ok(fallback_updates.clone());
            }
            let text = std::fs::read_to_string(&m).map_err(|e| e.to_string())?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        }) as UpdatesProvider
    });
    let existing = |p: Option<PathBuf>| p.filter(|p| p.exists());
    let options = ServerOptions {
        enet_room_ttl_ms: (number("ENET_ROOM_TTL_SECONDS", 60.0).max(15.0) * 1000.0) as i64,
        reserved_room_idle_ms: (number("RESERVED_ROOM_IDLE_SECONDS", 3600.0).max(0.0) * 1000.0) as i64,
        account_store_path: Some(accounts.clone()),
        account_upstream,
        account_service_token,
        moderation_store_path: Some(
            path_of("LASTROUTE_MODERATION_STORE").unwrap_or_else(|| accounts.parent().unwrap_or(&data).join("moderation.json")),
        ),
        moderation_salt: env.get("LASTROUTE_MODERATION_SALT").cloned().unwrap_or_default(),
        save_directory: Some(path_of("LASTROUTE_SAVES_DIR").unwrap_or_else(|| data.join("saves"))),
        skin_directory: Some(data.join("skins")),
        skin_grace_ms: (number("LASTROUTE_SKIN_GRACE_SECONDS", (crate::skins::skins::UNUSED_SKIN_GRACE_MS / 1000) as f64).max(0.0) * 1000.0) as i64,
        mod_pack_store_path: Some(path_of("LASTROUTE_MOD_INDEX_STORE").unwrap_or_else(|| data.join("mod_packs.json"))),
        crash_report_directory: Some(path_of("LASTROUTE_CRASH_REPORTS_DIR").unwrap_or_else(|| data.join("crash-reports"))),
        reserved_room_store_path: Some(path_of("LASTROUTE_RESERVED_ROOMS_STORE").unwrap_or_else(|| data.join("reserved_rooms.json"))),
        agent_tokens: crate::regions::parse_agent_tokens(env.get("LASTROUTE_AGENT_TOKENS").map(String::as_str).unwrap_or("")),
        reserved_room_codes: env
            .get("LASTROUTE_RESERVED_ROOMS")
            .cloned()
            .unwrap_or_else(|| "KATHY,AKIRI".into())
            .split(',')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect(),
        mod_download_base: crate::mod_packs::download_base_setting(
            get("LASTROUTE_MODS_BASE").as_deref(),
            get("LASTROUTE_MOD_DOWNLOAD_BASE").as_deref(),
        ),
        public_directory: existing(path_of("LASTROUTE_PUBLIC_DIR")),
        custom_map_directory: existing(custom),
        update_directory: existing(path_of("LASTROUTE_UPDATE_DIR")),
        update_key: get("LASTROUTE_UPDATE_KEY").unwrap_or_default(),
        updates,
        updates_provider: provider,
        admin_password: env
            .get("LASTROUTE_ADMIN_PASSWORD")
            .filter(|v| !v.is_empty())
            .or_else(|| env.get("KYTTU_DEV_PASSWORD"))
            .cloned()
            .unwrap_or_default(),
        trust_proxy: Regex::new(r"(?i)^(1|true|yes|on)$").unwrap().is_match(env.get("TRUST_PROXY").map(String::as_str).unwrap_or("")),
        relay_address: if relay_enabled { relay_address } else { String::new() },
        relay_port: if relay_enabled { relay_port } else { 0 },
        public_route_store_path: Some(
            path_of("LASTROUTE_PUBLIC_ROUTE_STORE").unwrap_or_else(|| data.join("lastroute").join("godot_publicroom.json")),
        ),
        public_route_seed: env.get("LASTROUTE_PUBLIC_ROUTE_SEED").cloned().unwrap_or_else(|| "PUBLIC-ROUTE".into()).chars().take(64).collect(),
        rate_limit_window_ms: (number("LASTROUTE_RATE_LIMIT_WINDOW_SECONDS", 60.0).max(1.0) * 1000.0) as i64,
        api_rate_limit_max: number("LASTROUTE_API_RATE_LIMIT", 600.0).max(1.0) as u32,
        account_rate_limit_max: number("LASTROUTE_ACCOUNT_RATE_LIMIT", 30.0).max(1.0) as u32,
        mod_rate_limit_max: number("LASTROUTE_MOD_RATE_LIMIT", 120.0).max(1.0) as u32,
        instance: env.get("LASTROUTE_INSTANCE").cloned().unwrap_or_else(|| "production".into()),
        ..ServerOptions::default()
    };
    Ok(EnvironmentConfig { options, host, http_port, relay_port, relay_enabled, account_store_path: accounts })
}
