//! A tiny in-process stand-in for the directory's room routes, on a real loopback socket, for tests that need the round
//! trip "a host registers a room, then a player looks it up and decides whether to join" (the BUILD MISMATCH check).
//!
//! It keeps the three behaviours of `backend/server.mjs` (and `lr_server`) that decide that check, and nothing else:
//! * `POST /api/enet/rooms/register` stores the `protocol` field only when it is 8 lower-case hex digits (anything else,
//!   upper case included, is stored as "", which the client reads as HOST UNVERSIONED), lists the room with `dedicated`
//!   set when the register carried the code's spawn ticket, and keeps a hosted code's owner;
//! * `GET /api/enet/rooms/CODE` answers the room's public view, else 404;
//! * `POST /api/rooms/hosted/create` records the body and hands out the code set with [`TestDirectory::hosted_code`].
//!
//! It is not the real directory: the real one is exercised by `net/tests.rs` (`LR_NET_TEST_HTTP_PORT`) and by
//! `lr_server`'s own tests. Every request is kept, so a test can assert what the client actually sent.

use super::config::BackendConfig;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// The account the stand-in names as the owner of a hosted room.
pub const OWNER: &str = "KYTTU";
/// The spawn ticket of a hosted code.
pub const TICKET: &str = "spawn-ticket-0123456789";

#[derive(Default)]
struct Inner {
    rooms: HashMap<String, Value>,
    hosted: Option<String>,
    log: Vec<(String, String, Value)>,
}

pub struct TestDirectory {
    port: u16,
    stop: Arc<AtomicBool>,
    inner: Arc<Mutex<Inner>>,
    handle: Option<JoinHandle<()>>,
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String, Value)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut got = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        got.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&got).to_string();
        if let Some(at) = text.find("\r\n\r\n") {
            let length = text[..at]
                .to_ascii_lowercase()
                .lines()
                .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                .unwrap_or(0);
            if got.len() >= at + 4 + length {
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&got).to_string();
    let mut first = text.lines().next()?.split_whitespace();
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let body = text.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(Value::Null);
    Some((method, path, body))
}

fn reply(stream: &mut TcpStream, status: &str, body: &Value) {
    let text = body.to_string();
    let _ = stream.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{text}", text.len()).as_bytes(),
    );
}

/// The `protocol` the real directory would store for a register body.
pub fn stored_protocol(sent: &str) -> String {
    if sent.len() == 8 && sent.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        sent.to_string()
    } else {
        String::new()
    }
}

fn handle(inner: &Mutex<Inner>, stream: &mut TcpStream) {
    let Some((method, path, body)) = read_request(stream) else { return };
    let mut state = inner.lock().unwrap();
    state.log.push((method.clone(), path.clone(), body.clone()));
    match (method.as_str(), path.as_str()) {
        ("POST", "/api/rooms/hosted/create") => match state.hosted.clone() {
            Some(code) => reply(stream, "202 Accepted", &json!({"ok": true, "code": code, "region": "test", "relay": {"address": "", "port": 0}, "owner": OWNER})),
            None => reply(stream, "503 Service Unavailable", &json!({"ok": false, "code": "NO_FREE_REGION", "error": "No server has a free room slot right now"})),
        },
        ("POST", "/api/enet/rooms/register") => {
            let code = body["code"].as_str().unwrap_or("").to_uppercase();
            let hosted = state.hosted.as_deref() == Some(code.as_str());
            let spawned = hosted && body["spawnTicket"].as_str() == Some(TICKET);
            if hosted && !spawned {
                return reply(stream, "403 Forbidden", &json!({"ok": false, "code": "HOSTED_ROOM_SPAWN_ONLY", "error": "This room is hosted by the server"}));
            }
            if state.rooms.contains_key(&code) {
                return reply(stream, "409 Conflict", &json!({"ok": false, "error": "ROOM CODE ALREADY REGISTERED"}));
            }
            let mut room = json!({
                "code": code, "game": "last_route", "players": body["players"].as_i64().unwrap_or(1), "sessionActive": false, "daytime": false,
                "protocol": stored_protocol(body["protocol"].as_str().unwrap_or("")), "dedicated": spawned,
                "region": if spawned { "test" } else { "" }, "expiresIn": 60,
            });
            if hosted {
                room["owner"] = json!(OWNER);
            }
            if body["transport"].as_str() == Some("ws") {
                room["transport"] = json!("ws");
                room["address"] = json!("");
                room["port"] = json!(0);
                room["wsPath"] = json!("/api/ws");
            } else {
                room["transport"] = json!("relay");
                room["address"] = json!("127.0.0.1");
                room["port"] = json!(11105);
            }
            state.rooms.insert(code, room.clone());
            reply(stream, "201 Created", &json!({"ok": true, "token": "host-token", "room": room}));
        }
        ("GET", p) if p.starts_with("/api/enet/rooms/") => {
            let code = p.trim_start_matches("/api/enet/rooms/").to_uppercase();
            match state.rooms.get(&code) {
                Some(room) => reply(stream, "200 OK", &json!({"ok": true, "room": room})),
                None => reply(stream, "404 Not Found", &json!({"ok": false, "error": "ROOM NOT FOUND"})),
            }
        }
        _ => reply(stream, "404 Not Found", &json!({"ok": false, "error": "Not found"})),
    }
}

impl TestDirectory {
    pub fn start() -> TestDirectory {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let (stop, inner) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Inner::default())));
        let (stop_t, inner_t) = (stop.clone(), inner.clone());
        let handle = std::thread::spawn(move || {
            while !stop_t.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        handle(&inner_t, &mut stream);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        TestDirectory { port, stop, inner, handle: Some(handle) }
    }

    pub fn config(&self) -> BackendConfig {
        BackendConfig::loopback(self.port)
    }

    /// Makes `code` a hosted code: `POST /api/rooms/hosted/create` hands it out, and only a register that carries
    /// [`TICKET`] may list it (the listing is then `dedicated`, with [`OWNER`] as its owner).
    pub fn hosted_code(&self, code: &str) {
        self.inner.lock().unwrap().hosted = Some(code.to_string());
    }

    /// Every request so far: method, path, JSON body (null when it had none).
    pub fn requests(&self) -> Vec<(String, String, Value)> {
        self.inner.lock().unwrap().log.clone()
    }

    /// The bodies posted to `path`.
    pub fn posted(&self, path: &str) -> Vec<Value> {
        self.requests().into_iter().filter(|(m, p, _)| m == "POST" && p == path).map(|(_, _, b)| b).collect()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
