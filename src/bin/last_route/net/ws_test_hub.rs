//! A tiny in-process WebSocket relay for tests: the server side of RFC 6455 (upgrade, masked frames in, plain frames
//! out, ping/pong, close codes) and the room routing of `backend/ws_relay.mjs`, on a real loopback socket. It lets the
//! native link ([`super::ws_native`]) and the dedicated room (`headless/dedicated.rs`) be tested over real TCP without
//! the Node server. The real Node hub is checked by the `node_hub_*` test in `ws_native` when `LR_WS_RELAY_MJS` names
//! the file (see that test).
//!
//! Not a copy of the Node server: no rate limit, no size cap beyond the parser's, rooms are added with
//! [`TestHub::add_room`], guest ids count up from 2.

use super::ws_native::{close_payload, encode_ws_frame, opcode, sha1, FrameParser, WsMessage, MAX_MESSAGE_BYTES};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone)]
struct Client {
    id: u32,
    out: Arc<Mutex<TcpStream>>,
}

impl Client {
    fn send(&self, op: u8, payload: &[u8]) {
        if let Ok(mut s) = self.out.lock() {
            let _ = s.write_all(&encode_ws_frame(op, payload, None));
        }
    }

    fn text(&self, v: Value) {
        self.send(opcode::TEXT, v.to_string().as_bytes());
    }

    fn close(&self, code: u16, reason: &str) {
        self.send(opcode::CLOSE, &close_payload(code, reason));
        if let Ok(s) = self.out.lock() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

#[derive(Default)]
struct Hub {
    host: Option<Client>,
    guests: HashMap<u32, Client>,
}

#[derive(Default)]
struct State {
    /// code -> host token
    rooms: HashMap<String, String>,
    hubs: HashMap<String, Hub>,
    next_guest: u32,
}

pub struct TestHub {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    /// Binary frames routed.
    pub routed: Arc<AtomicU64>,
    /// Pong frames received from clients.
    pub pongs: Arc<AtomicU64>,
    /// Text keepalives (`{"t":"ping"}`) received from clients.
    pub keepalives: Arc<AtomicU64>,
    accept: Option<JoinHandle<()>>,
}

impl TestHub {
    pub fn start() -> TestHub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the test hub");
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State { next_guest: 2, ..State::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let routed = Arc::new(AtomicU64::new(0));
        let pongs = Arc::new(AtomicU64::new(0));
        let keepalives = Arc::new(AtomicU64::new(0));
        let accept = {
            let (state, stop, routed, pongs, keepalives) = (state.clone(), stop.clone(), routed.clone(), pongs.clone(), keepalives.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (state, stop, routed, pongs, keepalives) = (state.clone(), stop.clone(), routed.clone(), pongs.clone(), keepalives.clone());
                            std::thread::spawn(move || serve(stream, state, stop, routed, pongs, keepalives));
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                        Err(_) => break,
                    }
                }
            })
        };
        TestHub { addr, state, stop, routed, pongs, keepalives, accept: Some(accept) }
    }

    /// The origin to hand a link: `ws://127.0.0.1:port`.
    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Lets room `code` exist with host token `token` (the directory's job in production).
    pub fn add_room(&self, code: &str, token: &str) {
        self.state.lock().unwrap().rooms.insert(code.to_uppercase(), token.to_string());
    }

    pub fn has_host(&self, code: &str) -> bool {
        self.state.lock().unwrap().hubs.get(code).is_some_and(|h| h.host.is_some())
    }

    pub fn guest_ids(&self, code: &str) -> Vec<u32> {
        let mut ids: Vec<u32> = self.state.lock().unwrap().hubs.get(code).map(|h| h.guests.keys().copied().collect()).unwrap_or_default();
        ids.sort_unstable();
        ids
    }

    /// Sends a ping frame to every client of `code`.
    pub fn ping_all(&self, code: &str) {
        let clients: Vec<Client> = self.state.lock().unwrap().hubs.get(code).map(|h| h.host.iter().chain(h.guests.values()).cloned().collect()).unwrap_or_default();
        for c in clients {
            c.send(opcode::PING, b"hub");
        }
    }

    /// The directory dropped the room: everyone is closed with 4004.
    pub fn close_room(&self, code: &str) {
        let hub = self.state.lock().unwrap().hubs.remove(code);
        if let Some(hub) = hub {
            for g in hub.guests.values() {
                g.close(4004, "room closed");
            }
            if let Some(h) = &hub.host {
                h.close(4004, "room closed");
            }
        }
    }
}

impl Drop for TestHub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        let hubs: Vec<Hub> = self.state.lock().unwrap().hubs.drain().map(|(_, h)| h).collect();
        for hub in hubs {
            for c in hub.host.iter().chain(hub.guests.values()) {
                c.close(1001, "server closing");
            }
        }
    }
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>, routed: Arc<AtomicU64>, pongs: Arc<AtomicU64>, keepalives: Arc<AtomicU64>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
    // the upgrade
    let mut head = Vec::new();
    let mut chunk = [0u8; 2048];
    let split = loop {
        if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if stop.load(Ordering::Relaxed) || head.len() > 8192 {
            return;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return,
        }
    };
    let text = String::from_utf8_lossy(&head[..split]).into_owned();
    let key = text.lines().find_map(|l| l.split_once(':').filter(|(n, _)| n.trim().eq_ignore_ascii_case("sec-websocket-key")).map(|(_, v)| v.trim().to_string()));
    let (Some(key), true) = (key, text.starts_with("GET /api/ws ")) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
        return;
    };
    let accept = super::base64_encode(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    let _ = stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes());

    let me = Client { id: 0, out: Arc::new(Mutex::new(stream.try_clone().unwrap())) };
    let mut me = me;
    let mut parser = FrameParser::new(true, MAX_MESSAGE_BYTES);
    parser.push(&head[split + 4..]);
    let (mut code, mut role) = (String::new(), "");
    let mut helloed = false;
    'conn: loop {
        loop {
            match parser.next_message() {
                Ok(Some(WsMessage::Text(bytes))) if !helloed => {
                    helloed = true;
                    let hello: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                    match on_hello(&state, &hello, &me) {
                        Ok((c, r, id)) => {
                            code = c;
                            role = r;
                            me.id = id;
                        }
                        Err(message) => {
                            me.text(json!({"t": "error", "message": message}));
                            me.close(4002, &message);
                            break 'conn;
                        }
                    }
                }
                Ok(Some(WsMessage::Text(bytes))) => {
                    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                    match v["t"].as_str() {
                        Some("ping") => {
                            keepalives.fetch_add(1, Ordering::Relaxed);
                        }
                        Some("kick") if role == "host" => {
                            let target = v["id"].as_u64().unwrap_or(0) as u32;
                            let guest = state.lock().unwrap().hubs.get(&code).and_then(|h| h.guests.get(&target).cloned());
                            if let Some(g) = guest {
                                g.close(4005, "removed by host");
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Some(WsMessage::Binary(bytes))) if helloed && !role.is_empty() => {
                    if bytes.len() >= 6 {
                        routed.fetch_add(1, Ordering::Relaxed);
                        route(&state, &code, &me, &bytes);
                    }
                }
                Ok(Some(WsMessage::Binary(_))) => {}
                Ok(Some(WsMessage::Ping(p))) => me.send(opcode::PONG, &p),
                Ok(Some(WsMessage::Pong(_))) => {
                    pongs.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Some(WsMessage::Close { .. })) => {
                    me.send(opcode::CLOSE, &close_payload(1000, "bye"));
                    break 'conn;
                }
                Ok(None) => break,
                Err(e) => {
                    me.close(e.code, e.why);
                    break 'conn;
                }
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => parser.push(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => break,
        }
    }
    detach(&state, &code, role, &me);
    let _ = stream.shutdown(Shutdown::Both);
}

/// The hello of ws_relay.mjs: a known room, the host's token, a host already connected for a guest.
fn on_hello(state: &Arc<Mutex<State>>, hello: &Value, me: &Client) -> Result<(String, &'static str, u32), String> {
    if hello["t"] != "hello" {
        return Err("EXPECTED HELLO".into());
    }
    let code = hello["code"].as_str().unwrap_or_default().trim().to_uppercase();
    let mut st = state.lock().unwrap();
    let Some(token) = st.rooms.get(&code).cloned() else { return Err("ROOM NOT FOUND".into()) };
    match hello["role"].as_str() {
        Some("host") => {
            if hello["token"].as_str() != Some(token.as_str()) {
                return Err("INVALID HOST TOKEN".into());
            }
            let old = st.hubs.insert(code.clone(), Hub { host: Some(Client { id: 1, out: me.out.clone() }), guests: HashMap::new() });
            if let Some(old) = old {
                if let Some(h) = old.host {
                    h.close(4000, "replaced by a new host connection");
                }
            }
            me.text(json!({"t": "welcome", "id": 1, "role": "host"}));
            Ok((code, "host", 1))
        }
        Some("guest") => {
            let id = st.next_guest;
            let Some(hub) = st.hubs.get_mut(&code).filter(|h| h.host.is_some()) else { return Err("HOST IS NOT CONNECTED YET".into()) };
            hub.guests.insert(id, Client { id, out: me.out.clone() });
            hub.host.as_ref().unwrap().text(json!({"t": "peer", "id": id, "on": true}));
            me.text(json!({"t": "welcome", "id": id, "role": "guest"}));
            st.next_guest += 1;
            Ok((code, "guest", id))
        }
        _ => Err("UNKNOWN ROLE".into()),
    }
}

/// ws_relay.mjs `route`: the first four bytes are the target (1 host, N guest, 0 everyone else) and become the sender.
fn route(state: &Arc<Mutex<State>>, code: &str, from: &Client, bytes: &[u8]) {
    let target = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let mut out = bytes.to_vec();
    out[..4].copy_from_slice(&(from.id as i32).to_le_bytes());
    let st = state.lock().unwrap();
    let Some(hub) = st.hubs.get(code) else { return };
    let everyone = || hub.host.iter().chain(hub.guests.values());
    let send = |c: &Client| c.send(opcode::BINARY, &out);
    match target {
        0 => everyone().filter(|c| c.id != from.id).for_each(send),
        t if t < 0 => everyone().filter(|c| c.id != from.id && c.id as i32 != -t).for_each(send),
        1 => {
            if from.id != 1 {
                if let Some(h) = &hub.host {
                    send(h);
                }
            }
        }
        t => {
            if let Some(g) = hub.guests.get(&(t as u32)).filter(|g| g.id != from.id) {
                send(g);
            }
        }
    }
}

/// ws_relay.mjs `detach`: a host leaving closes every guest with 4001, a guest leaving tells the host.
fn detach(state: &Arc<Mutex<State>>, code: &str, role: &str, me: &Client) {
    let id = me.id;
    let mut st = state.lock().unwrap();
    match role {
        "host" => {
            let Some(hub) = st.hubs.get(code) else { return };
            // a replaced host is no longer the hub's host: leave the new one alone
            if !hub.host.as_ref().is_some_and(|h| Arc::ptr_eq(&h.out, &me.out)) {
                return;
            }
            let hub = st.hubs.remove(code).unwrap();
            drop(st);
            for g in hub.guests.values() {
                g.text(json!({"t": "host", "on": false}));
                g.close(4001, "host left");
            }
        }
        "guest" => {
            if let Some(hub) = st.hubs.get_mut(code) {
                if hub.guests.remove(&id).is_some() {
                    if let Some(h) = &hub.host {
                        h.text(json!({"t": "peer", "id": id, "on": false}));
                    }
                }
            }
        }
        _ => {}
    }
}
