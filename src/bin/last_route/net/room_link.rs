//! The transport seam under [`super::session::CoopSession`]: one relay connection, host or guest, whatever carries it.
//!
//! Two implementations exist:
//! * [`super::relay::RelayLink`]: the native UDP LRR1 relay (unchanged).
//! * [`super::ws_link::WsLink`]: the browser build's WebSocket relay (`/api/ws`, the same hub the Godot web build
//!   joins), reached through the page's WebSocket API in dist/web/index.html.
//!
//! Both report through [`RelayEvent`] and carry opaque session datagrams, so the co-op layer above does not care
//! which one it has.

use super::relay::{RelayEvent, Role};

/// `Send`: a session is built on a request thread (native) and then handed to the frame loop.
pub trait RoomLink: Send {
    /// Host or guest.
    fn role(&self) -> Role;
    /// True once the relay accepted this connection (HOST_ACK / welcome).
    fn ready(&self) -> bool;
    /// A guest's relay session (or welcome id); 0 for a host.
    fn session(&self) -> u32;
    /// The room code.
    fn code(&self) -> &str;
    /// Host: to guest `session`. Guest: to the host (`session` is ignored). False when it was not sent.
    fn send(&self, session: u32, payload: &[u8]) -> bool;
    /// Drains what arrived since the last call and keeps the connection alive.
    fn poll(&mut self) -> Vec<RelayEvent>;
    /// The host token changed (a heartbeat rotated it).
    fn update_host_token(&mut self, token: &str);
    /// Ends the connection.
    fn close(&mut self);
}

/// A boxed link is a link (the co-op layer holds one as `Box<dyn RoomLink>`).
impl<T: RoomLink + ?Sized> RoomLink for Box<T> {
    fn role(&self) -> Role {
        (**self).role()
    }
    fn ready(&self) -> bool {
        (**self).ready()
    }
    fn session(&self) -> u32 {
        (**self).session()
    }
    fn code(&self) -> &str {
        (**self).code()
    }
    fn send(&self, session: u32, payload: &[u8]) -> bool {
        (**self).send(session, payload)
    }
    fn poll(&mut self) -> Vec<RelayEvent> {
        (**self).poll()
    }
    fn update_host_token(&mut self, token: &str) {
        (**self).update_host_token(token)
    }
    fn close(&mut self) {
        (**self).close()
    }
}

impl RoomLink for super::relay::RelayLink {
    fn role(&self) -> Role {
        super::relay::RelayLink::role(self)
    }
    fn ready(&self) -> bool {
        super::relay::RelayLink::ready(self)
    }
    fn session(&self) -> u32 {
        super::relay::RelayLink::session(self)
    }
    fn code(&self) -> &str {
        super::relay::RelayLink::code(self)
    }
    fn send(&self, session: u32, payload: &[u8]) -> bool {
        super::relay::RelayLink::send(self, session, payload)
    }
    fn poll(&mut self) -> Vec<RelayEvent> {
        super::relay::RelayLink::poll(self)
    }
    fn update_host_token(&mut self, token: &str) {
        super::relay::RelayLink::update_host_token(self, token)
    }
    fn close(&mut self) {
        super::relay::RelayLink::close(self)
    }
}
