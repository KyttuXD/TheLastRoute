//! A runner's way back into the public route after the room's host is gone: the handover.
//!
//! The relay closes a room when its host goes (the UDP relay's `HOST_LEFT`, the WebSocket hub's 4001): that cannot be
//! changed from the client, and the room's world is the host's. So the guests do not wait for a promotion the relay does
//! not send; they open the room again themselves, in a fixed order, and keep playing meanwhile (their game is not torn
//! down; the hunters they last saw stand where they stood):
//!
//! 1. each guest knows its place in line from the host's last room message ([`super::public_route::plan`]): the lowest
//!    session id is first;
//! 2. the first in line looks the code up, finds none (the host unregistered, or the relay forgot it) and registers it:
//!    it is the new host. The others look the code up on their retry interval and join whoever holds it. A runner that
//!    finds none before its turn waits; one whose turn has come opens the room, so a first in line that never came costs
//!    one step, not the room;
//! 3. two runners that open at the same moment cannot both win: the directory accepts one registration of the code
//!    (HTTP 409 for the other, which then joins), and the relay answers a second public host `HOST_JOIN_AS_GUEST`.
//!
//! This is the same path as a fresh PUBLIC ROUTE join ([`super::world_sync::enter_public`]), so it runs on the UDP relay
//! and on the WebSocket relay, native and in the browser, with no change to either relay. When a server process
//! (`--dedicated-host PUBLC`) holds the route, nobody ever needs this except to rejoin it after a restart.
//!
//! What the new host knows: its own game (the world is the route's seed, which every runner builds itself) and the
//! hunters it last saw in the old host's frames. The hunters keep their place, facing, state, level, stun and disguise;
//! what the old host's brains knew beyond that (their paths, search memory and timers) is not on the wire, so a hunter
//! starts from a clean brain at that place. The sabotaged relay count rides in the old host's join snapshot and is
//! re-published from the new host's own game.

use super::config::BackendConfig;
use super::public_route::{Plan, Reform};
use super::world_sync::{enter_public, DirectoryOps, Entry, NetSession, PublicOps};
use super::Pending;
use crate::shared::clock::Instant;
use std::sync::Arc;

/// What the handover has for the app this frame.
pub enum ReformStep {
    /// Still trying (or waiting for the next try).
    Waiting,
    /// The runner is in the room again: as its new host, or as a guest of whoever holds it.
    Entered(Box<NetSession>),
    /// The time is up: go to the title.
    GaveUp,
}

/// What the app learns of a session it adopted from the handover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    /// The relay accepted the session: the handover is over.
    Linked,
    /// The session ended, or never linked in time: it is dropped and the next try is made.
    Retry,
    /// Still waiting for the relay.
    Waiting,
}

pub struct PublicReform {
    ops: Arc<dyn PublicOps>,
    state: Reform,
    attempt: Option<Pending<Entry>>,
    /// When the session adopted from the last try began to link.
    adopted: Option<Instant>,
}

impl PublicReform {
    /// Starts the handover of a runner whose host is gone.
    pub fn begin(cfg: BackendConfig, name: String, skin: String, plan: Plan, now: Instant) -> PublicReform {
        PublicReform::with_ops(Arc::new(DirectoryOps::new(cfg, name, skin)), Reform::new(plan, now))
    }

    pub(crate) fn with_ops(ops: Arc<dyn PublicOps>, state: Reform) -> PublicReform {
        PublicReform { ops, state, attempt: None, adopted: None }
    }

    pub fn plan(&self) -> Plan {
        self.state.plan()
    }

    pub fn tries(&self) -> u32 {
        self.state.tries()
    }

    /// Once a frame while the runner has no session: polls the running try, starts the next one when it is due.
    pub fn update(&mut self, now: Instant) -> ReformStep {
        if let Some(pending) = self.attempt.as_mut() {
            match pending.poll() {
                None => return ReformStep::Waiting,
                Some(Ok(Entry::Hosting(session) | Entry::Joined(session))) => {
                    self.attempt = None;
                    self.adopted = Some(now);
                    return ReformStep::Entered(Box::new(session));
                }
                // no room yet and not this runner's turn, or the directory could not be reached: try again
                Some(Ok(Entry::NoRoom) | Err(_)) => {
                    self.attempt = None;
                    self.state.tried(now);
                }
            }
        }
        if self.state.expired(now) {
            return ReformStep::GaveUp;
        }
        if self.state.due(now) {
            let (ops, may_host) = (self.ops.clone(), self.state.may_host(now));
            self.attempt = Some(Pending::spawn(move || enter_public(ops.as_ref(), may_host)));
        }
        ReformStep::Waiting
    }

    /// Once a frame while the adopted session has not yet settled: `linked` is whether the relay accepted it and `ended`
    /// whether it reported the room gone (or a problem) since. A session that does not link within the settle time is a
    /// room whose host is already gone (the directory still listed it): it is dropped and the next try is made.
    pub fn settle(&mut self, now: Instant, linked: bool, ended: bool) -> Settled {
        let Some(since) = self.adopted else { return Settled::Linked };
        if linked && !ended {
            self.adopted = None;
            return Settled::Linked;
        }
        if ended || now.duration_since(since) >= self.state.settle() {
            self.adopted = None;
            self.state.tried(now);
            return Settled::Retry;
        }
        Settled::Waiting
    }

    /// Whether the app should still be waiting for this handover (it has not given up).
    pub fn expired(&self, now: Instant) -> bool {
        self.state.expired(now)
    }
}
