//! The public route end to end over the test hub (`net/ws_test_hub.rs`), with a directory stand-in that keeps the one rule the
//! real ones keep: a code has one live holder, and a second registration of it is refused (HTTP 409).
//!
//! What these tests do not cover: the real directories and relays (Node on vps2, `lr_server` on vps3), a browser, and
//! more than a handful of runners. See docs/SERVERS.md for what needs a live deployment.

use super::*;
use crate::net::public_reform::{PublicReform, ReformStep, Settled};
use crate::net::public_route::{self, Plan, Reform, Rules};
use crate::net::ws_native::NativeWsLink;
use crate::net::ws_test_hub::TestHub;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CODE: &str = "PUBLC";

/// The directory and relay of a test: one hub, one live holder of the public code at a time.
struct Dir {
    hub: TestHub,
    /// Registration is one step at a time, like the directory's single-threaded map.
    registering: Mutex<()>,
    /// The directory still lists a holder that is already gone (its entry lapses only after the TTL).
    ghost: AtomicBool,
    tokens: AtomicUsize,
    /// How many runners opened the room, ever.
    opened: AtomicUsize,
}

impl Dir {
    fn new() -> Arc<Dir> {
        Arc::new(Dir {
            hub: TestHub::start(),
            registering: Mutex::new(()),
            ghost: AtomicBool::new(false),
            tokens: AtomicUsize::new(0),
            opened: AtomicUsize::new(0),
        })
    }

    fn holder(&self) -> bool {
        self.hub.has_host(CODE)
    }
}

/// One runner's view of the directory.
struct Ops {
    dir: Arc<Dir>,
    name: String,
}

fn ops(dir: &Arc<Dir>, name: &str) -> Arc<Ops> {
    Arc::new(Ops { dir: dir.clone(), name: name.into() })
}

fn miss(status: u16, error: &str) -> NetError {
    NetError::Http { status, code: String::new(), error: error.into(), body: Value::Null }
}

impl PublicOps for Ops {
    fn lookup(&self) -> Result<api::RoomEndpoint, NetError> {
        if self.dir.holder() || self.dir.ghost.load(Ordering::SeqCst) {
            Ok(api::RoomEndpoint { code: CODE.into(), transport: "ws".into(), ..api::RoomEndpoint::default() })
        } else {
            Err(miss(404, "ROOM NOT FOUND"))
        }
    }

    fn open(&self) -> Result<HostOutcome, NetError> {
        let _one_at_a_time = self.dir.registering.lock().unwrap();
        if self.dir.holder() {
            return Err(miss(409, "ROOM CODE ALREADY REGISTERED"));
        }
        let token = format!("tok{}", self.dir.tokens.fetch_add(1, Ordering::SeqCst));
        self.dir.hub.add_room(CODE, &token);
        let link = NativeWsLink::host(CODE, &token, &self.dir.hub.url()).map_err(|e| NetError::Transport(e.to_string()))?;
        let mut session = NetSession::new(
            BackendConfig::default(),
            CODE.into(),
            Role::Host,
            CoopSession::new(link),
            token,
            "ROOM-PUBLC-AAAA".into(),
            self.name.clone(),
            SKIN_DEFAULT.into(),
        );
        session.directory = false;
        // the registration holds until the hub has the host, so the next runner's lookup sees it
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while !self.dir.holder() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.dir.opened.fetch_add(1, Ordering::SeqCst);
        Ok(HostOutcome::Hosting(session))
    }

    fn join(&self, _room: api::RoomEndpoint) -> Result<NetSession, NetError> {
        let link = NativeWsLink::guest(CODE, "", &self.dir.hub.url()).map_err(|e| NetError::Transport(e.to_string()))?;
        Ok(NetSession::guest_over(BackendConfig::default(), CODE.into(), Box::new(link), self.name.clone(), SKIN_DEFAULT.into()))
    }
}

/// A server process holding the public room, over the same hub. Returns once the hub has it, so the next lookup sees it.
fn server(dir: &Arc<Dir>) -> NetSession {
    dir.hub.add_room(CODE, "server-token");
    let link = NativeWsLink::host(CODE, "server-token", &dir.hub.url()).unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while !dir.holder() && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(2));
    }
    NetSession::dedicated_host(BackendConfig::default(), CODE.into(), Box::new(link), "server-token".into(), "ROOM-PUBLC-SRV".into(), "SERVER".into(), false)
}

fn enter(dir: &Arc<Dir>, name: &str, may_host: bool) -> Entry {
    enter_public(ops(dir, name).as_ref(), may_host).unwrap_or_else(|e| panic!("{name} could not enter: {e:?}"))
}

fn hosting(entry: Entry) -> NetSession {
    match entry {
        Entry::Hosting(s) => s,
        Entry::Joined(_) => panic!("expected to open the room, joined it"),
        Entry::NoRoom => panic!("expected to open the room, found no room"),
    }
}

fn joined(entry: Entry) -> NetSession {
    match entry {
        Entry::Joined(s) => s,
        Entry::Hosting(_) => panic!("expected to join the room, opened it"),
        Entry::NoRoom => panic!("expected to join the room, found no room"),
    }
}

/// Ticks `sessions` until `done` says so (20 s at most); returns the events each raised, in order.
fn pump(sessions: &mut [&mut NetSession], mut done: impl FnMut(&[&mut NetSession], &[Vec<NetEvent>]) -> bool) -> Vec<Vec<NetEvent>> {
    let mut seen: Vec<Vec<NetEvent>> = sessions.iter().map(|_| Vec::new()).collect();
    let end = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < end {
        for (i, s) in sessions.iter_mut().enumerate() {
            seen[i].extend(s.tick(None));
        }
        if done(sessions, &seen) {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    panic!("timed out; events so far {seen:?}");
}

/// Ticks for `time`; the events each raised.
fn pump_for(sessions: &mut [&mut NetSession], time: Duration) -> Vec<Vec<NetEvent>> {
    let end = std::time::Instant::now() + time;
    pump(sessions, |_, _| std::time::Instant::now() >= end)
}

fn host_left(events: &[NetEvent]) -> bool {
    events.iter().any(|e| matches!(e, NetEvent::HostLeft(_)))
}

/// The guests joined the room, the host knows them all, and the host's room message has listed them to each other.
fn registered(host: &mut NetSession, guests: &mut [&mut NetSession]) {
    let want = guests.len();
    let mut all: Vec<&mut NetSession> = vec![host];
    all.extend(guests.iter_mut().map(|g| &mut **g));
    pump(&mut all, |s, _| s[0].guest_count() == want && s[1..].iter().all(|g| g.player_peers.len() == want && g.linked()));
}

/// Fast timings for the handover, so a test runs in a second or two.
fn fast() -> Rules {
    Rules {
        handover_step_seconds: 0.4,
        handover_retry_seconds: 0.1,
        handover_grace_seconds: 12.0,
        handover_settle_seconds: 2.0,
        ..Rules::default()
    }
}

fn now() -> crate::shared::clock::Instant {
    crate::shared::clock::Instant::now()
}

fn reform_for(dir: &Arc<Dir>, name: &str, plan: Plan) -> PublicReform {
    PublicReform::with_ops(ops(dir, name), Reform::with_rules(plan, now(), fast()))
}

/// Runs the handovers of `reforms` until each has a session; the sessions in the order given.
fn hand_over(reforms: &mut [&mut PublicReform]) -> Vec<NetSession> {
    let mut got: Vec<Option<NetSession>> = reforms.iter().map(|_| None).collect();
    let end = std::time::Instant::now() + Duration::from_secs(20);
    while got.iter().any(Option::is_none) && std::time::Instant::now() < end {
        for (i, r) in reforms.iter_mut().enumerate() {
            if got[i].is_none() {
                match r.update(now()) {
                    ReformStep::Entered(s) => got[i] = Some(*s),
                    ReformStep::GaveUp => panic!("runner {i} gave up"),
                    ReformStep::Waiting => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    got.into_iter().map(|s| s.expect("every runner entered the room again in time")).collect()
}

// ---- who is the host ------------------------------------------------------------------------------------------------

#[test]
fn a_server_process_holds_the_room_and_no_runner_who_joins_it_becomes_its_host() {
    let dir = Dir::new();
    let mut srv = server(&dir);
    // the first runner to join finds the room held, whatever it is allowed to do when it finds none
    let mut a = joined(enter(&dir, "A", true));
    let mut b = joined(enter(&dir, "B", true));
    assert!(!a.is_host() && !b.is_host());
    assert_eq!(dir.opened.load(Ordering::SeqCst), 0, "no runner registered the room");
    registered(&mut srv, &mut [&mut a, &mut b]);
    assert!(a.is_public_route() && !a.host_controls());
    // the guests know the host is a server process, and it has no row in the PLAYERS list: it is no runner
    assert!(a.reform_plan().was_dedicated);
    assert!(a.roster().iter().all(|r| !r.host), "no HOST tag on the public route");
    assert!(a.roster().is_empty(), "the server has no row of its own, and B has sent no sample yet: {:?}", a.roster());
}

#[test]
fn a_promotion_the_relay_sends_a_guest_is_not_taken_a_guest_never_makes_itself_the_host() {
    let dir = Dir::new();
    let mut srv = server(&dir);
    let mut a = joined(enter(&dir, "A", true));
    registered(&mut srv, &mut [&mut a]);
    let mut out = Vec::new();
    for event in [
        RelayEvent::Promoted { token: "a-token".into(), epoch: 0 },
        RelayEvent::Promoted { token: "a-token".into(), epoch: 3 },
        RelayEvent::Migrating { epoch: 3, ready: true },
    ] {
        a.on_session_event(SessionEvent::Relay(event), Instant::now(), &mut out);
    }
    assert!(out.is_empty(), "nothing reaches the app: {out:?}");
    assert!(!a.is_host() && !a.host_controls(), "still a guest");
}

#[test]
fn the_first_runner_who_finds_no_room_holds_it_only_as_a_stand_in_with_no_host_controls() {
    let dir = Dir::new();
    let mut a = hosting(enter(&dir, "A", true));
    assert!(a.is_host(), "until a server process holds the route a runner has to");
    assert!(a.is_public_route());
    assert!(!a.host_controls(), "but it has none of a host's controls");
    let mut b = joined(enter(&dir, "B", true));
    let mut c = joined(enter(&dir, "C", true));
    registered(&mut a, &mut [&mut b, &mut c]);
    let peer = a.roster_peers()[0].0;
    assert!(!a.kick_guest(peer), "nobody is removed from the public route");
    assert_eq!(a.guest_count(), 2);
    // nobody sees a HOST tag, the stand-in's own row included
    assert!(b.roster().iter().all(|r| !r.host) && c.roster().iter().all(|r| !r.host));
    // and a private room's host keeps its controls
    dir.hub.add_room("ABCDE", "t");
    let mut private = NetSession::new(BackendConfig::default(), "ABCDE".into(), Role::Host, CoopSession::new(NativeWsLink::host("ABCDE", "t", &dir.hub.url()).unwrap()), "t".into(), String::new(), "H".into(), SKIN_DEFAULT.into());
    private.directory = false;
    assert!(private.host_controls() && !private.is_public_route());
}

#[test]
fn with_the_stand_in_turned_off_no_runner_opens_the_room_when_there_is_none() {
    let dir = Dir::new();
    assert!(matches!(enter(&dir, "A", false), Entry::NoRoom));
    assert_eq!(dir.opened.load(Ordering::SeqCst), 0);
    let rules = Rules { client_host_fallback: false, ..Rules::default() };
    let reform = Reform::with_rules(Plan { rank: Some(0), was_dedicated: false }, now(), rules);
    assert!(!reform.may_host(now() + Duration::from_secs(600)));
}

#[test]
fn an_owner_command_on_the_public_route_is_refused_by_the_room_and_never_reaches_the_app_or_the_directory() {
    let dir = Dir::new();
    let mut a = hosting(enter(&dir, "A", true));
    let mut b = joined(enter(&dir, "B", true));
    registered(&mut a, &mut [&mut b]);
    assert!(b.send_owner_command("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ", &super::super::hosted::OwnerAction::End));
    let seen = pump(&mut [&mut a, &mut b], |_, s| s[1].iter().any(|e| matches!(e, NetEvent::OwnerRefused(_))));
    assert!(seen[1].contains(&NetEvent::OwnerRefused(public_route::NO_OWNER_TEXT.into())), "{:?}", seen[1]);
    assert!(!seen[0].iter().any(|e| matches!(e, NetEvent::OwnerCommand { .. })), "the stand-in's app never sees a command: {:?}", seen[0]);
}

// ---- the host leaves ----------------------------------------------------------------------------------------------------

#[test]
fn reproduction_the_first_joiner_leaves_and_the_relay_closes_the_room_on_everyone() {
    // A joins first (and holds the room), B and C join. The relay ends the room for B and C the moment A goes: the
    // WebSocket hub closes every guest with 4001 when the host's socket closes. Without a handover this is the report:
    // "the first joining member kicks everyone out of a public room when they leave".
    let dir = Dir::new();
    let mut a = hosting(enter(&dir, "A", true));
    let mut b = joined(enter(&dir, "B", true));
    let mut c = joined(enter(&dir, "C", true));
    registered(&mut a, &mut [&mut b, &mut c]);
    a.leave();
    let seen = pump(&mut [&mut b, &mut c], |_, s| host_left(&s[0]) && host_left(&s[1]));
    assert!(host_left(&seen[0]) && host_left(&seen[1]), "B and C are both told the room is gone: {seen:?}");
    assert!(!dir.holder(), "and the room is gone from the directory too");
}

#[test]
fn when_the_first_joiner_leaves_the_runner_with_the_lowest_id_opens_the_room_and_the_rest_rejoin_it() {
    let dir = Dir::new();
    let mut a = hosting(enter(&dir, "A", true));
    let mut b = joined(enter(&dir, "B", true));
    registered(&mut a, &mut [&mut b]);
    let mut c = joined(enter(&dir, "C", true));
    registered(&mut a, &mut [&mut b, &mut c]);
    // the plans are made while the host is there: B joined first, so it is first in line
    let (plan_b, plan_c) = (b.reform_plan(), c.reform_plan());
    assert_eq!((plan_b.rank, plan_c.rank), (Some(0), Some(1)));
    a.leave();
    pump(&mut [&mut b, &mut c], |_, s| host_left(&s[0]) && host_left(&s[1]));
    drop((b, c));

    let (mut rb, mut rc) = (reform_for(&dir, "B", plan_b), reform_for(&dir, "C", plan_c));
    let mut entered = hand_over(&mut [&mut rb, &mut rc]);
    let (mut c2, mut b2) = (entered.pop().unwrap(), entered.pop().unwrap());
    // exactly one host: B, who was first in line; C is its guest
    assert!(b2.is_host() && !c2.is_host(), "one host only");
    assert_eq!(dir.opened.load(Ordering::SeqCst), 2, "A opened the room once and B opened it again, nobody else did");
    // the world goes on: the new host sends the seed the runners were playing, and the others are told to enter it
    b2.set_room_seed("ROOM-PUBLC-WORLD".into());
    b2.start_run("ROOM-PUBLC-WORLD", "STANDARD");
    let seen = pump(&mut [&mut b2, &mut c2], |s, seen| s[0].guest_count() == 1 && seen[1].iter().any(|e| matches!(e, NetEvent::Start { .. })));
    assert!(seen[1].contains(&NetEvent::Start { seed: "ROOM-PUBLC-WORLD".into(), difficulty: "STANDARD".into() }), "{:?}", seen[1]);
    assert!(!host_left(&seen[1]), "C is not kicked by the handover");
    assert_eq!(rb.settle(now(), b2.linked(), false), Settled::Linked);
    assert_eq!(rc.settle(now(), c2.linked(), false), Settled::Linked);
    // and it stays that way: no second handover, no further host change
    let later = pump_for(&mut [&mut b2, &mut c2], Duration::from_millis(800));
    assert!(later.iter().all(|e| !host_left(e)), "{later:?}");
    assert!(b2.is_host() && !c2.is_host());
    assert_eq!(dir.hub.guest_ids(CODE).len(), 1);
}

#[test]
fn a_public_room_with_one_runner_left_goes_on_and_takes_new_runners() {
    let dir = Dir::new();
    let mut a = hosting(enter(&dir, "A", true));
    let mut b = joined(enter(&dir, "B", true));
    registered(&mut a, &mut [&mut b]);
    let plan = b.reform_plan();
    a.leave();
    pump(&mut [&mut b], |_, s| host_left(&s[0]));
    drop(b);
    let mut r = reform_for(&dir, "B", plan);
    let mut b2 = hand_over(&mut [&mut r]).pop().unwrap();
    assert!(b2.is_host(), "alone, B holds the room");
    pump(&mut [&mut b2], |s, _| s[0].linked());
    // a later runner finds the room held and joins it
    let mut d = joined(enter(&dir, "D", true));
    registered(&mut b2, &mut [&mut d]);
    assert_eq!(b2.guest_count(), 1);
}

#[test]
fn when_nobody_remains_the_room_is_gone_and_the_next_runner_to_arrive_opens_a_fresh_one() {
    let dir = Dir::new();
    let a = hosting(enter(&dir, "A", true));
    a.leave();
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while dir.holder() && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!dir.holder(), "an empty public room closes");
    assert!(matches!(enter(&dir, "E", false), Entry::NoRoom));
    let e = hosting(enter(&dir, "E", true));
    assert!(e.is_host());
    assert_eq!(dir.opened.load(Ordering::SeqCst), 2);
}

#[test]
fn two_runners_who_open_the_room_at_the_same_moment_cannot_both_host_it() {
    // both think they are first in line (an older host listed no ids); the directory lets one registration through
    for round in 0..5 {
        let dir = Dir::new();
        let (d1, d2) = (dir.clone(), dir.clone());
        let t1 = std::thread::spawn(move || enter_public(ops(&d1, "X").as_ref(), true).map_err(|e| format!("{e:?}")));
        let t2 = std::thread::spawn(move || enter_public(ops(&d2, "Y").as_ref(), true).map_err(|e| format!("{e:?}")));
        let (x, y) = (t1.join().unwrap().unwrap(), t2.join().unwrap().unwrap());
        let hosts = [&x, &y].iter().filter(|e| matches!(e, Entry::Hosting(_))).count();
        let guests = [&x, &y].iter().filter(|e| matches!(e, Entry::Joined(_))).count();
        assert_eq!((hosts, guests), (1, 1), "round {round}: one host and one guest, never two hosts");
        assert_eq!(dir.opened.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn a_room_the_directory_still_lists_after_its_host_is_gone_is_dropped_and_tried_again() {
    let dir = Dir::new();
    // the host crashed: the directory lists it for a while, but the hub has nobody
    dir.ghost.store(true, Ordering::SeqCst);
    let mut r = reform_for(&dir, "B", Plan { rank: Some(0), was_dedicated: false });
    let mut ghost_session = hand_over(&mut [&mut r]).pop().unwrap();
    assert!(!ghost_session.is_host(), "the lookup found a listed room and joined it");
    // it never links (the hub has no host), so the handover drops it and tries again
    let mut verdict = Settled::Waiting;
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while verdict == Settled::Waiting && std::time::Instant::now() < end {
        let events = ghost_session.tick(None);
        verdict = r.settle(now(), ghost_session.linked(), host_left(&events));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(verdict, Settled::Retry);
    drop(ghost_session);
    // the listing lapses; the next try finds no room and, being first in line, opens it
    dir.ghost.store(false, Ordering::SeqCst);
    let again = hand_over(&mut [&mut r]).pop().unwrap();
    assert!(again.is_host());
}

#[test]
fn a_runner_that_is_not_first_in_line_waits_for_the_one_ahead_and_then_opens_the_room_itself() {
    let dir = Dir::new();
    // nobody ahead ever comes: the second in line opens the room after its step, not before
    let mut r = reform_for(&dir, "C", Plan { rank: Some(1), was_dedicated: false });
    let t0 = std::time::Instant::now();
    let c = hand_over(&mut [&mut r]).pop().unwrap();
    assert!(c.is_host());
    assert!(t0.elapsed() >= Duration::from_millis(350), "it waited its step first: {:?}", t0.elapsed());
}

#[test]
fn a_runner_whose_handover_runs_out_of_time_gives_up_and_a_client_that_may_not_host_never_opens_the_room() {
    let dir = Dir::new();
    let rules = Rules { client_host_fallback: false, handover_grace_seconds: 0.5, handover_retry_seconds: 0.05, ..fast() };
    let mut r = PublicReform::with_ops(ops(&dir, "C"), Reform::with_rules(Plan::join_only(), now(), rules));
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let mut gave_up = false;
    while !gave_up && std::time::Instant::now() < end {
        match r.update(now()) {
            ReformStep::GaveUp => gave_up = true,
            ReformStep::Entered(_) => panic!("there is no room to enter"),
            ReformStep::Waiting => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert!(gave_up, "the title is where it ends up");
    assert_eq!(dir.opened.load(Ordering::SeqCst), 0);
}

#[test]
fn when_the_server_process_leaves_the_runners_wait_for_it_and_none_opens_the_room_in_a_hurry() {
    let dir = Dir::new();
    let mut srv = server(&dir);
    let mut a = joined(enter(&dir, "A", true));
    registered(&mut srv, &mut [&mut a]);
    let plan = a.reform_plan();
    assert!(plan.was_dedicated);
    drop(srv);
    pump(&mut [&mut a], |_, s| host_left(&s[0]));
    drop(a);
    // with the data's timings nobody opens the room in the first seconds: a restarted server process gets the room
    let reform = Reform::with_rules(plan, now(), Rules::default());
    assert!(!reform.may_host(now() + Duration::from_secs(10)));
    // the server process comes back before that: the runner joins it instead of opening a room of its own
    let _srv2 = server(&dir);
    let mut r = PublicReform::with_ops(ops(&dir, "A"), Reform::with_rules(plan, now(), Rules { handover_dedicated_seconds: 30.0, ..fast() }));
    let a2 = hand_over(&mut [&mut r]).pop().unwrap();
    assert!(!a2.is_host());
    assert_eq!(dir.opened.load(Ordering::SeqCst), 0, "no runner opened the room");
}
