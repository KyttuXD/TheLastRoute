//! Integration tests against a running backend on loopback (the Node original or `lr_server`). They only run when
//! `LR_NET_TEST_HTTP_PORT` (and `LR_NET_TEST_RELAY_PORT` for the relay test) are set, e.g.
//! `LR_NET_TEST_HTTP_PORT=18105 LR_NET_TEST_RELAY_PORT=11105 cargo test net::tests`.

use super::api::{self, Resolution};
use super::backend::{Backend, BackendEvent};
use super::protocol::Message;
use super::relay::RelayLink;
use super::saves::{CloudOutcome, CloudSaves, Owed};
use super::session::{CoopSession, SessionEvent, HOST_PEER_ID};
use super::BackendConfig;
use std::time::{Duration, Instant};

fn ports() -> Option<(u16, u16)> {
    let http = std::env::var("LR_NET_TEST_HTTP_PORT").ok()?.parse().ok()?;
    let relay = std::env::var("LR_NET_TEST_RELAY_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((http, relay))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lr_net_it_{name}_{}_{}", std::process::id(), super::unix_now()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn unique_name(prefix: &str) -> String {
    let tail: String = super::random_bytes(4).iter().map(|b| format!("{b:02X}")).collect();
    format!("{prefix}_{tail}")
}

#[test]
fn accounts_guest_names_rooms_and_cloud_saves_against_a_live_server() {
    let Some((http, _)) = ports() else { return };
    let cfg = BackendConfig::loopback(http);
    let dir = scratch("backend");
    let mut backend = Backend::new(cfg.clone(), &dir);
    backend.ensure_guest_name();
    let events = backend.settle(Duration::from_secs(10));
    assert!(events.iter().any(|e| matches!(e, BackendEvent::GuestName(n) if api::valid_guest_name(n))), "{events:?}");

    let name = unique_name("RUST");
    backend.create_account(&name, "correct horse battery", "");
    let events = backend.settle(Duration::from_secs(20));
    assert!(events.iter().any(|e| matches!(e, BackendEvent::SignedIn(r) if r.username == name)), "{events:?}");
    assert!(backend.signed_in());
    // a restart restores the saved session from last_route_account.cfg
    let mut again = Backend::new(cfg.clone(), &dir);
    again.restore_session();
    let events = again.settle(Duration::from_secs(10));
    assert!(events.iter().any(|e| matches!(e, BackendEvent::SignedIn(_))), "{events:?}");
    // wrong password -> the server's own message
    let mut other = Backend::new(cfg.clone(), &scratch("other"));
    other.sign_in(&name, "wrong password!");
    let events = other.settle(Duration::from_secs(20));
    assert!(events.iter().any(|e| matches!(e, BackendEvent::AccountError(_))), "{events:?}");

    // create a room, join it by code
    backend.create_room(None, "");
    let events = backend.settle(Duration::from_secs(10));
    let code = events
        .iter()
        .find_map(|e| if let BackendEvent::RoomCreated { code, .. } = e { Some(code.clone()) } else { None })
        .unwrap_or_else(|| panic!("{events:?}"));
    other.join_room(&code.to_lowercase(), "");
    let events = other.settle(Duration::from_secs(10));
    match events.iter().find_map(|e| if let BackendEvent::JoinResolved { resolution, .. } = e { Some(resolution.clone()) } else { None }) {
        Some(Resolution::Found { room, .. }) => assert_eq!(room.code, code),
        other => panic!("join did not resolve: {other:?}"),
    }
    other.join_room("ZZZZZ", "");
    let events = other.settle(Duration::from_secs(10));
    assert!(events.iter().any(|e| matches!(e, BackendEvent::JoinResolved { resolution: Resolution::Failed { .. }, .. })), "{events:?}");
    backend.close_room();
    backend.settle(Duration::from_secs(5));
    assert!(matches!(api::lookup_room(&cfg, &code), Err(e) if e.status() == Some(404)));

    // cloud saves: push, newest wins, pull adopts a newer copy
    let token = backend.account.session_token.clone();
    let saves = CloudSaves::new(&dir.join("route_saves").join("run.json"), 1);
    let record = |t: i64| serde_json::json!({ "saved_at": t, "depth": t }).as_object().unwrap().clone();
    saves.mark(Owed::Push);
    assert!(matches!(saves.sync_blocking(&cfg, &token, Some(&record(100))), CloudOutcome::Pushed { updated_at: 100 }));
    assert_eq!(saves.pending(), Owed::None);
    saves.mark(Owed::Push);
    assert_eq!(saves.sync_blocking(&cfg, &token, Some(&record(50))), CloudOutcome::Adopt(record(100)), "409 STALE then the pull adopts");
    assert_eq!(saves.sync_blocking(&cfg, &token, Some(&record(100))), CloudOutcome::UpToDate);
    saves.mark(Owed::Clear);
    assert_eq!(saves.sync_blocking(&cfg, &token, None), CloudOutcome::Cleared);

    backend.sign_out();
    backend.settle(Duration::from_secs(5));
    assert!(matches!(api::session(&cfg, &token), Err(e) if e.status() == Some(401)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coop_session_over_the_live_relay() {
    let Some((http, relay_port)) = ports() else { return };
    if relay_port == 0 {
        return;
    }
    let cfg = BackendConfig::loopback(http);
    let code = api::make_room_code();
    let registered = api::register_room(&cfg, &api::RegisterRoom { code: code.clone(), ..Default::default() }).expect("register");
    let mut host = CoopSession::new(RelayLink::host(&code, &registered.token, "127.0.0.1", relay_port).unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !host.link.ready() {
        host.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(host.link.ready(), "the relay acknowledged the host");
    let mut guest = CoopSession::new(RelayLink::guest(&code, "127.0.0.1", relay_port, "").unwrap());
    let (mut hello, mut reply) = (false, false);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !reply {
        for e in guest.poll() {
            match e {
                SessionEvent::Ready => {
                    guest.send(HOST_PEER_ID, &Message::RegisterPeer { requested_name: "R-4821".into() });
                }
                SessionEvent::Message { message: Message::Daytime { daytime: true }, .. } => reply = true,
                _ => {}
            }
        }
        for e in host.poll() {
            if let SessionEvent::Message { from, message: Message::RegisterPeer { .. } } = e {
                hello = true;
                host.send(from, &Message::Daytime { daytime: true });
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(hello && reply, "a message went guest -> host -> guest through the relay");
    let _ = api::unregister_room(&cfg, &code, &registered.token);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut left = false;
    while Instant::now() < deadline && !left {
        left = guest.poll().iter().any(|e| matches!(e, SessionEvent::Relay(super::relay::RelayEvent::HostLeft(_))));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(left, "an explicit quit tells the guest HOST_LEFT");
    host.close();
}
