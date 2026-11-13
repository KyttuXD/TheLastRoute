//! Tests for the hardening described in docs/SERVER_SECURITY.md: the limits a route counts itself (per account, per
//! room, per address), the bounded directories, the poisoned-lock recovery and the request-body handling over a real
//! socket (a late body, an oversized one and a `Content-Length` no machine could hold).

use super::*;
use crate::util::{manual_clock, ManualClock};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const PASSWORD: &str = "correct horse battery";

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lr_server_security_{name}_{}", crate::util::hex(&random_bytes(6))));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A server whose per-address request limits are out of the way (they have their own tests), on a settable clock.
fn app(f: impl FnOnce(&mut ServerOptions)) -> (LastRouteServer, ManualClock) {
    let (clock, time) = manual_clock(1_700_000_000_000);
    let mut options = ServerOptions::default();
    options.api_rate_limit_max = 100_000;
    options.account_rate_limit_max = 100_000;
    options.mod_rate_limit_max = 100_000;
    options.now = clock;
    f(&mut options);
    (LastRouteServer::new(options), time)
}

fn post_from(server: &LastRouteServer, remote: &str, path: &str, body: Value) -> (u16, Value) {
    let mut request = Request::json("POST", path, &body);
    request.remote = remote.into();
    let response = server.handle(&request);
    (response.status, response.json_body())
}

fn code_of_reply(reply: &(u16, Value)) -> (u16, Value) {
    (reply.0, reply.1["code"].clone())
}

// --- limits ------------------------------------------------------------------------------------------------------

#[test]
fn an_ipv6_caller_is_limited_by_its_network_not_by_each_address_it_picks() {
    let (server, _) = app(|o| o.auth_rate_limit_max = 2);
    let attempt = |remote: &str| post_from(&server, remote, "/api/account/login", json!({ "username": "nobody_here", "password": "wrong password" })).0;
    assert_eq!(attempt("2001:db8:1:2::1"), 401);
    assert_eq!(attempt("2001:db8:1:2::2"), 401);
    assert_eq!(attempt("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), 429, "a third address of the same /64 is the same caller");
    assert_eq!(attempt("2001:db8:1:3::1"), 401, "another network has its own count");
}

#[test]
fn wrong_sign_ins_are_counted_per_account_across_addresses() {
    let (server, time) = app(|o| o.login_failure_limit = 3);
    let sign_up = |name: &str| post_from(&server, "198.51.100.1", "/api/account/register", json!({ "username": name, "password": PASSWORD }));
    assert_eq!(sign_up("TARGET").0, 200);
    assert_eq!(sign_up("BYSTANDER").0, 200);
    let sign_in = |remote: &str, name: &str, password: &str| {
        code_of_reply(&post_from(&server, remote, "/api/account/login", json!({ "username": name, "password": password })))
    };
    for last in 1..=3 {
        assert_eq!(sign_in(&format!("203.0.113.{last}"), "target", "a wrong guess"), (401, json!("INVALID_CREDENTIALS")));
    }
    // the name used up its window: a fourth address is refused before the password is looked at, right or wrong
    let (status, refused) = post_from(&server, "203.0.113.4", "/api/account/login", json!({ "username": "Target", "password": "a wrong guess" }));
    assert_eq!((status, refused["code"].clone()), (429, json!("RATE_LIMITED")), "{refused}");
    assert!(refused["error"].as_str().unwrap().starts_with("TOO MANY SIGN-IN ATTEMPTS"), "{refused}");
    assert!(refused["retryAfterSeconds"].as_i64().unwrap() >= 1);
    assert_eq!(sign_in("203.0.113.5", "TARGET", PASSWORD).0, 429);
    // other accounts are not touched, and the window ends by itself
    assert_eq!(sign_in("203.0.113.5", "BYSTANDER", PASSWORD).0, 200);
    time.advance(LOGIN_FAILURE_WINDOW_MS + 1);
    assert_eq!(sign_in("203.0.113.5", "TARGET", PASSWORD).0, 200);
}

#[test]
fn reserved_room_password_attempts_are_limited_per_address_and_per_room() {
    let root = temp_root("room_password");
    let store = root.join("reserved_rooms.json");
    let (server, time) = app(move |o| {
        o.reserved_room_store_path = Some(store);
        o.reserved_room_codes = vec!["KTPRV".into()];
    });
    let guess = |remote: &str| code_of_reply(&post_from(&server, remote, "/api/rooms/reserved/resolve", json!({ "code": "KTPRV", "password": "a guess" })));
    let denied = (403, json!("ROOM_ACCESS_DENIED"));
    let limited = (429, json!("RATE_LIMITED"));
    for _ in 0..ROOM_PASSWORD_ATTEMPTS_PER_ADDRESS {
        assert_eq!(guess("203.0.113.1"), denied);
    }
    assert_eq!(guess("203.0.113.1"), limited, "one address ran out of attempts");
    assert_eq!(guess("203.0.113.2"), denied, "another address has its own");
    // many addresses against one room: the room's own count stops them together
    let mut counted = ROOM_PASSWORD_ATTEMPTS_PER_ADDRESS + 1;
    let mut last = 10u32;
    while counted < ROOM_PASSWORD_ATTEMPTS_PER_ROOM {
        last += 1;
        for _ in 0..ROOM_PASSWORD_ATTEMPTS_PER_ADDRESS.min(ROOM_PASSWORD_ATTEMPTS_PER_ROOM - counted) {
            assert_eq!(guess(&format!("203.0.113.{last}")), denied);
            counted += 1;
        }
    }
    assert_eq!(guess("198.51.100.7"), limited, "the room ran out of attempts for every address");
    // a request with no password is not a guess and is never counted
    let no_password = post_from(&server, "198.51.100.7", "/api/rooms/reserved/resolve", json!({ "code": "KTPRV" }));
    assert_eq!(code_of_reply(&no_password), denied);
    // the window ends; hosting with a room password is counted the same way
    time.advance(61_000);
    let host = |remote: &str| post_from(&server, remote, "/api/enet/rooms/register", json!({ "code": "KTPRV", "port": 7000, "roomPassword": "a guess" }));
    for _ in 0..ROOM_PASSWORD_ATTEMPTS_PER_ADDRESS {
        assert_eq!(code_of_reply(&host("203.0.113.1")), (403, json!("ROOM_RESERVED")));
    }
    assert_eq!(code_of_reply(&host("203.0.113.1")), limited);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_guest_names_are_limited_per_address_and_a_renewal_is_free() {
    let (server, time) = app(|_| {});
    let (status, first) = post_from(&server, "203.0.113.9", "/api/guest/name", json!({}));
    assert_eq!(status, 200, "{first}");
    for _ in 1..GUEST_NAMES_PER_ADDRESS {
        assert_eq!(post_from(&server, "203.0.113.9", "/api/guest/name", json!({})).0, 200);
    }
    let refused = post_from(&server, "203.0.113.9", "/api/guest/name", json!({}));
    assert_eq!(code_of_reply(&refused), (429, json!("RATE_LIMITED")));
    // the holder of a name keeps renewing it; a wrong token is a request for a new name
    let renewal = json!({ "previous": first["name"], "token": first["token"] });
    let (status, renewed) = post_from(&server, "203.0.113.9", "/api/guest/name", renewal);
    assert_eq!((status, renewed["name"].clone(), renewed["renewed"].clone()), (200, first["name"].clone(), json!(true)));
    let forged = json!({ "previous": first["name"], "token": "not-the-token" });
    assert_eq!(post_from(&server, "203.0.113.9", "/api/guest/name", forged).0, 429);
    assert_eq!(post_from(&server, "203.0.113.10", "/api/guest/name", json!({})).0, 200, "another address is not limited");
    time.advance(GUEST_NAME_WINDOW_MS + 1);
    assert_eq!(post_from(&server, "203.0.113.9", "/api/guest/name", json!({})).0, 200);
}

#[test]
fn one_address_holds_a_bounded_number_of_enet_room_codes() {
    let (server, _) = app(|_| {});
    let register = |remote: &str, code: &str| post_from(&server, remote, "/api/enet/rooms/register", json!({ "code": code, "port": 8421 }));
    let codes = |prefix: &str| -> Vec<String> {
        let mut out = Vec::new();
        for a in "ABCDEFGH".chars() {
            for b in "JKLMN".chars() {
                out.push(format!("{prefix}{a}{b}2"));
            }
        }
        out
    };
    let mine = codes("RM");
    assert!(mine.len() > ENET_ROOMS_PER_ADDRESS);
    for code in &mine[..ENET_ROOMS_PER_ADDRESS] {
        assert_eq!(register("203.0.113.9", code).0, 201, "{code}");
    }
    let refused = register("203.0.113.9", &mine[ENET_ROOMS_PER_ADDRESS]);
    assert_eq!(code_of_reply(&refused), (429, json!("TOO_MANY_ROOMS")));
    assert_eq!(register("203.0.113.10", &mine[ENET_ROOMS_PER_ADDRESS]).0, 201, "another address may still register");
    // a caller on this machine (a proxy, or local testing) is not counted per address
    for code in &codes("LP")[..ENET_ROOMS_PER_ADDRESS + 1] {
        assert_eq!(register("127.0.0.1", code).0, 201, "{code}");
    }
}

#[test]
fn one_address_opens_a_bounded_number_of_web_rooms_per_window() {
    let (server, time) = app(|_| {});
    let join = |remote: &str, room: &str, player: &str, join_only: bool| {
        // the player id doubles as the name, so two players of one room never clash
        post_from(&server, remote, "/api/rooms/join", json!({ "room": room, "name": player, "playerId": player, "joinOnly": join_only }))
    };
    for n in 0..WEB_ROOMS_PER_ADDRESS {
        let opened = join("203.0.113.9", &format!("WEBR{n:02}"), "player-1", false);
        assert_eq!(opened.0, 200, "{n}: {}", opened.1);
    }
    assert_eq!(code_of_reply(&join("203.0.113.9", "WEBRZZ", "player-1", false)), (429, json!("RATE_LIMITED")));
    // joining a room that exists, or asking for one that does not, opens nothing and is not counted
    let second = join("203.0.113.9", "WEBR00", "player-2", false);
    assert_eq!(second.0, 200, "{}", second.1);
    assert_eq!(join("203.0.113.9", "NOSUCH", "player-2", true).0, 404);
    assert_eq!(join("203.0.113.10", "WEBRZZ", "player-3", false).0, 200, "another address is not limited");
    time.advance(61_000);
    assert_eq!(join("203.0.113.9", "WEBRYY", "player-1", false).0, 200);
}

#[test]
fn an_oversized_region_report_is_refused_before_its_signature_is_checked() {
    let (server, _) = app(|_| {});
    let mut request = Request::new("POST", "/api/regions/report");
    request.body = vec![b'x'; MAX_BODY_BYTES + 1];
    assert_eq!(server.handle(&request).status, 413);
    request.body = b"{}".to_vec();
    assert_eq!(server.handle(&request).status, 403, "an unsigned report of a normal size is still refused as unsigned");
}

#[test]
fn every_reply_tells_the_browser_not_to_guess_its_type() {
    let (server, _) = app(|_| {});
    for response in [
        server.handle(&Request::new("GET", "/api/health")),
        server.handle(&Request::new("GET", "/no/such/page")),
        server.handle(&Request::json("POST", "/api/account/login", &json!({ "username": "nobody", "password": "wrong pass" }))),
    ] {
        assert_eq!(response.header("x-content-type-options"), Some("nosniff"), "{}", response.status);
    }
}

// --- a panic must not take the server down ------------------------------------------------------------------------

#[test]
fn a_poisoned_state_lock_does_not_stop_requests_maintenance_or_session_lookups() {
    let (server, _) = app(|_| {});
    let server = Arc::new(server);
    let poisoner = server.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.state.lock().unwrap();
        panic!("poison the state lock");
    })
    .join();
    assert!(server.state.is_poisoned());
    assert_eq!(server.handle(&Request::new("GET", "/api/health")).status, 200);
    server.maintenance();
    assert_eq!(SessionLookup::session_username(&*server, "no-such-token"), None);
    server.close();
}

// --- request bodies over a real socket ----------------------------------------------------------------------------

struct Live {
    server: Arc<LastRouteServer>,
    port: u16,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Live {
    /// The server on a real listener with ONE worker, so a worker that is held shows at once.
    fn start(body_read_timeout_ms: u64) -> Live {
        let (server, _) = app(|o| {
            o.body_read_timeout_ms = body_read_timeout_ms;
            o.now = crate::util::system_clock();
        });
        let server = Arc::new(server);
        let http = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = http.server_addr().to_ip().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let threads = server.serve(http, 1, stop.clone());
        Live { server, port, stop, threads }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream
    }

    /// The status line of `GET /api/health` on a new connection ("" when nothing came back in time).
    fn health(&self) -> String {
        let mut stream = self.connect();
        stream.write_all(b"GET /api/health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out.lines().next().unwrap_or_default().to_string()
    }

    /// Waits up to three seconds for `done`.
    fn eventually(&self, done: impl Fn(&LastRouteServer) -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            if done(&self.server) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn finish(self) {
        self.stop.store(true, Ordering::SeqCst);
        for thread in self.threads {
            thread.join().unwrap();
        }
    }
}

#[test]
fn a_body_that_never_arrives_does_not_hold_the_worker() {
    let live = Live::start(300);
    let mut stalled = live.connect();
    // 2000 bytes are announced, a few are sent, then nothing
    stalled.write_all(b"POST /api/account/login HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 2000\r\n\r\n{\"username\":").unwrap();
    assert!(live.eventually(|server| server.body_readers() == 1), "the late body is read on a helper thread");
    let asked = Instant::now();
    assert!(live.health().starts_with("HTTP/1.1 200"), "the one worker is free again after the body timeout");
    assert!(asked.elapsed() < Duration::from_secs(4), "{:?}", asked.elapsed());
    assert_eq!(live.server.body_readers(), 1, "only the helper thread still waits for the body");
    // the sender goes away: the helper ends
    drop(stalled);
    assert!(live.eventually(|server| server.body_readers() == 0));
    assert_eq!(live.server.abandoned_requests(), 0);
    live.finish();
}

#[test]
fn a_body_that_arrives_in_pieces_within_the_timeout_is_served() {
    let live = Live::start(5_000);
    let body = format!(r#"{{"code":"WXYZ2","port":8421,"padding":"{}"}}"#, "p".repeat(2000));
    let mut stream = live.connect();
    let head = format!("POST /api/enet/rooms/register HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).unwrap();
    let (first, second) = body.split_at(700);
    stream.write_all(first.as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    stream.write_all(second.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 201"), "{out}");
    assert!(live.eventually(|server| server.body_readers() == 0));
    live.finish();
}

#[test]
fn a_declared_body_over_the_cap_is_answered_413_without_holding_the_worker() {
    let live = Live::start(300);
    let mut big = live.connect();
    let head = format!("POST /api/account/login HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", RAW_BODY_HARD_CAP + 1000);
    big.write_all(head.as_bytes()).unwrap();
    let mut status = [0u8; 12];
    big.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"HTTP/1.1 413");
    // the rest of the announced body is waited for on a helper thread, not on the worker
    assert!(live.health().starts_with("HTTP/1.1 200"));
    assert_eq!(live.server.body_readers(), 1);
    drop(big);
    assert!(live.eventually(|server| server.body_readers() == 0));
    assert_eq!(live.server.abandoned_requests(), 0);
    live.finish();
}

#[test]
fn a_content_length_no_machine_could_hold_does_not_end_the_process_or_hold_the_worker() {
    let live = Live::start(300);
    let mut huge = live.connect();
    // before the fix, dropping this request made tiny_http allocate the whole declared size: the process aborted
    let head = format!("POST /api/account/login HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", usize::MAX / 2);
    huge.write_all(head.as_bytes()).unwrap();
    assert!(live.eventually(|server| server.abandoned_requests() == 1), "the request is given up on, never dropped");
    assert!(live.health().starts_with("HTTP/1.1 200"));
    drop(huge);
    assert!(live.health().starts_with("HTTP/1.1 200"), "the server still answers after the sender left");
    assert_eq!(live.server.body_readers(), 0);
    live.finish();
}
